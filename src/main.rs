mod herdr;
mod nvim;
mod terminal;
mod theme;
mod ui;

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::json;

use crate::herdr::{Result, home_dir, rpc};

const DEFAULT_PLUGIN_ID: &str = "kremeshnoi.nvim-sessions";
const DEFAULT_SIDEBAR_WIDTH: u64 = 36;

fn sidebar_pane_file() -> PathBuf {
    env::var_os("HERDR_PLUGIN_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".local/state/herdr-nvim-sessions"))
        .join("sidebar_pane")
}

struct SidebarPane {
    pane_id: String,
    terminal_id: String,
}

fn pane_info(pane_id: &str) -> Option<serde_json::Value> {
    rpc("pane.get", json!({ "pane_id": pane_id }))
        .ok()
        .map(|mut result| result["pane"].take())
}

fn sidebar_width() -> u64 {
    env::var_os("HERDR_PLUGIN_CONFIG_DIR")
        .map(|dir| PathBuf::from(dir).join("config.toml"))
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|content| content.parse::<toml::Table>().ok())
        .and_then(|config| config.get("width").and_then(toml::Value::as_integer))
        .and_then(|width| u64::try_from(width).ok())
        .filter(|&width| width > 0)
        .unwrap_or(DEFAULT_SIDEBAR_WIDTH)
}

fn fit_sidebar_width(pane_id: &str) {
    let Ok(mut result) = rpc("pane.layout", json!({ "pane_id": pane_id })) else {
        return;
    };
    let layout = result["layout"].take();
    let rect = |value: &serde_json::Value| {
        let field = |key: &str| value[key].as_u64().unwrap_or_default();
        (field("x"), field("width"))
    };
    let Some((pane_x, pane_width)) = layout["panes"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|pane| pane["pane_id"] == pane_id)
        .map(|pane| rect(&pane["rect"]))
    else {
        return;
    };
    let parent = layout["splits"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|split| split["direction"] == "right")
        .filter_map(|split| {
            let (x, width) = rect(&split["rect"]);
            let ratio = split["ratio"].as_f64()?;
            (x < pane_x && x + width == pane_x + pane_width).then_some((width, ratio))
        })
        .min_by_key(|&(width, _)| width);
    let Some((split_width, ratio)) = parent else {
        return;
    };
    let target = 1.0 - sidebar_width().min(split_width) as f64 / split_width as f64;
    let delta = target - ratio;
    let direction = if delta < 0.0 { "left" } else { "right" };
    let _ = rpc(
        "pane.resize",
        json!({ "pane_id": pane_id, "direction": direction, "amount": delta.abs() }),
    );
}

fn remember_sidebar_pane() {
    let Ok(pane_id) = env::var("HERDR_PANE_ID") else {
        return;
    };
    let terminal_id = pane_info(&pane_id)
        .and_then(|pane| pane["terminal_id"].as_str().map(str::to_owned))
        .unwrap_or_default();
    let path = sidebar_pane_file();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&path, format!("{pane_id}\n{terminal_id}\n"));
    fit_sidebar_width(&pane_id);
}

fn forget_sidebar_pane() {
    let Ok(pane_id) = env::var("HERDR_PANE_ID") else {
        return;
    };
    let path = sidebar_pane_file();
    let stored = fs::read_to_string(&path).unwrap_or_default();
    if stored.lines().next() == Some(pane_id.as_str()) {
        let _ = fs::remove_file(path);
    }
}

fn existing_sidebar_pane() -> Option<(SidebarPane, String)> {
    let stored = fs::read_to_string(sidebar_pane_file()).ok()?;
    let mut lines = stored.lines().map(str::trim);
    let sidebar = SidebarPane {
        pane_id: lines.next().filter(|id| !id.is_empty())?.to_owned(),
        terminal_id: lines.next().unwrap_or_default().to_owned(),
    };
    let pane = pane_info(&sidebar.pane_id)?;
    if pane["terminal_id"].as_str() != Some(sidebar.terminal_id.as_str()) {
        return None;
    }
    let tab_id = pane["tab_id"].as_str().unwrap_or_default().to_owned();
    Some((sidebar, tab_id))
}

fn open_sidebar() -> Result<()> {
    let mut params = json!({
        "plugin_id": env::var("HERDR_PLUGIN_ID").unwrap_or_else(|_| DEFAULT_PLUGIN_ID.into()),
        "entrypoint": "sidebar",
        "placement": "split",
        "direction": "right",
        "focus": false,
    });
    if let Ok(pane_id) = env::var("HERDR_PANE_ID") {
        params["target_pane_id"] = pane_id.into();
    }
    rpc("plugin.pane.open", params).map(drop)
}

fn toggle() -> Result<()> {
    let Some((sidebar, tab_id)) = existing_sidebar_pane() else {
        return open_sidebar();
    };
    rpc("plugin.pane.close", json!({ "pane_id": sidebar.pane_id }))?;
    match env::var("HERDR_TAB_ID") {
        Ok(current_tab) if current_tab != tab_id => open_sidebar(),
        _ => Ok(()),
    }
}

fn list() -> Result<()> {
    for session in nvim::collect(env::var("HERDR_PANE_ID").ok().as_deref())? {
        println!("{}", session.to_json());
    }
    Ok(())
}

fn sidebar() -> Result<()> {
    remember_sidebar_pane();
    let result = ui::run().map_err(|error| error.to_string());
    forget_sidebar_pane();
    result
}

fn main() -> ExitCode {
    let command = env::args().nth(1).unwrap_or_else(|| "sidebar".into());
    let result = match command.as_str() {
        "sidebar" => sidebar(),
        "toggle" => toggle(),
        "list" => list(),
        other => Err(format!("unknown command: {other}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("herdr-nvim-sessions: {error}");
            ExitCode::FAILURE
        }
    }
}
