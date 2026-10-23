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
const SIDEBAR_WIDTH_RATIO: f64 = 0.22;

fn sidebar_pane_file() -> PathBuf {
    env::var_os("HERDR_PLUGIN_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".local/state/herdr-nvim-sessions"))
        .join("sidebar_pane")
}

fn remember_sidebar_pane() {
    let Ok(pane_id) = env::var("HERDR_PANE_ID") else {
        return;
    };
    let path = sidebar_pane_file();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&path, &pane_id);
    let _ = rpc(
        "pane.resize",
        json!({ "pane_id": pane_id, "direction": "right", "amount": 0.5 - SIDEBAR_WIDTH_RATIO }),
    );
}

fn existing_sidebar_pane() -> Option<String> {
    let pane_id = fs::read_to_string(sidebar_pane_file())
        .ok()?
        .trim()
        .to_owned();
    rpc("pane.get", json!({ "pane_id": pane_id })).ok()?;
    Some(pane_id)
}

fn toggle() -> Result<()> {
    if let Some(pane_id) = existing_sidebar_pane() {
        return rpc("pane.close", json!({ "pane_id": pane_id })).map(drop);
    }
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

fn list() -> Result<()> {
    for session in nvim::collect(env::var("HERDR_PANE_ID").ok().as_deref())? {
        println!("{}", session.to_json());
    }
    Ok(())
}

fn sidebar() -> Result<()> {
    remember_sidebar_pane();
    ui::run().map_err(|error| error.to_string())
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
