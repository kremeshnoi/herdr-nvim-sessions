use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::herdr::{Result, home_dir, rpc};

const QUERY_TIMEOUT: Duration = Duration::from_millis(700);
const INFO_EXPR: &str = "json_encode({\
'file': expand('%:~:.'),\
'modified': len(filter(getbufinfo({'buflisted': 1}), 'v:val.changed')),\
'cwd': fnamemodify(getcwd(), ':~')\
})";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Working,
    Blocked,
    Unknown,
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            State::Idle => "idle",
            State::Working => "working",
            State::Blocked => "blocked",
            State::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Session {
    pub pane_id: String,
    pub workspace_label: String,
    pub workspace_number: u64,
    pub tab_label: String,
    pub tab_number: u64,
    pub tab_visible: bool,
    pub cwd: String,
    pub focused: bool,
    pub file: String,
    pub modified: u64,
    pub state: State,
}

impl Session {
    pub fn title(&self) -> String {
        let name = if !self.file.is_empty() {
            self.file.clone()
        } else {
            Path::new(&self.cwd)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "[No Name]".into())
        };
        if self.modified > 0 {
            format!("{name} [+{}]", self.modified)
        } else {
            name
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "pane_id": self.pane_id,
            "workspace": self.workspace_label,
            "tab": self.tab_label,
            "cwd": self.cwd,
            "focused": self.focused,
            "file": self.file,
            "modified": self.modified,
            "state": self.state.name(),
        })
    }
}

fn shorten_home(path: &str) -> String {
    let home = home_dir();
    let home = home.to_string_lossy();
    match path.strip_prefix(home.as_ref()) {
        Some(rest) if !home.is_empty() => format!("~{rest}"),
        _ => path.to_owned(),
    }
}

fn nvim_pids(pane_id: &str) -> Vec<u32> {
    let Ok(result) = rpc("pane.process_info", json!({ "pane_id": pane_id })) else {
        return Vec::new();
    };
    result["process_info"]["foreground_processes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|process| process["name"] == "nvim")
        .filter(|process| {
            !process["argv"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|arg| arg == "--headless")
        })
        .filter_map(|process| process["pid"].as_u64().map(|pid| pid as u32))
        .collect()
}

fn child_pids_by_parent() -> HashMap<u32, Vec<u32>> {
    let output = Command::new("ps")
        .args(["-A", "-o", "pid=,ppid="])
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default();
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for line in output.lines() {
        let mut fields = line
            .split_whitespace()
            .filter_map(|field| field.parse().ok());
        if let (Some(pid), Some(ppid)) = (fields.next(), fields.next()) {
            children.entry(ppid).or_default().push(pid);
        }
    }
    children
}

fn socket_roots() -> Vec<PathBuf> {
    let user = env::var("USER").unwrap_or_default();
    let tmpdir = env::var_os("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/tmp".into());
    vec![
        tmpdir.join(format!("nvim.{user}")),
        PathBuf::from(format!("/tmp/nvim.{user}")),
    ]
}

fn find_server(pids: &[u32], children: &HashMap<u32, Vec<u32>>) -> Option<PathBuf> {
    let candidates = pids.iter().chain(
        pids.iter()
            .flat_map(|pid| children.get(pid).into_iter().flatten()),
    );
    let runtime_dir = env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    let session_dirs: Vec<PathBuf> = socket_roots()
        .iter()
        .filter_map(|root| fs::read_dir(root).ok())
        .flatten()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect();
    for pid in candidates {
        let socket_name = format!("nvim.{pid}.0");
        let found = session_dirs
            .iter()
            .chain(runtime_dir.iter())
            .map(|dir| dir.join(&socket_name))
            .find(|path| path.exists());
        if found.is_some() {
            return found;
        }
    }
    None
}

fn query(session: &mut Session, server: &Path) {
    let Ok(mut child) = Command::new("nvim")
        .arg("--clean")
        .arg("--headless")
        .arg("--server")
        .arg(server)
        .args(["--remote-expr", INFO_EXPR])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };
    let deadline = Instant::now() + QUERY_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                session.state = State::Blocked;
                return;
            }
        }
    }
    let mut output = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_string(&mut output);
    }
    let Ok(info) = serde_json::from_str::<Value>(&output) else {
        return;
    };
    session.file = info["file"].as_str().unwrap_or_default().to_owned();
    session.modified = info["modified"].as_u64().unwrap_or_default();
    if let Some(cwd) = info["cwd"].as_str() {
        session.cwd = cwd.to_owned();
    }
    session.state = if session.modified > 0 {
        State::Working
    } else {
        State::Idle
    };
}

fn labels_by_id(items: &Value, id_key: &str) -> HashMap<String, (String, u64)> {
    items
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let id = item[id_key].as_str()?.to_owned();
            let label = item["label"].as_str().unwrap_or(&id).to_owned();
            Some((id, (label, item["number"].as_u64().unwrap_or_default())))
        })
        .collect()
}

pub fn collect(own_pane_id: Option<&str>) -> Result<Vec<Session>> {
    let result = rpc("session.snapshot", json!({}))?;
    let snapshot = &result["snapshot"];
    let workspaces = labels_by_id(&snapshot["workspaces"], "workspace_id");
    let tabs = labels_by_id(&snapshot["tabs"], "tab_id");
    let focused_pane = snapshot["focused_pane_id"].as_str();
    let mut tab_counts: HashMap<&str, usize> = HashMap::new();
    for tab in snapshot["tabs"].as_array().into_iter().flatten() {
        if let Some(workspace_id) = tab["workspace_id"].as_str() {
            *tab_counts.entry(workspace_id).or_default() += 1;
        }
    }
    let children = child_pids_by_parent();

    let mut found: Vec<(Session, Option<PathBuf>)> = Vec::new();
    for pane in snapshot["panes"].as_array().into_iter().flatten() {
        let Some(pane_id) = pane["pane_id"].as_str() else {
            continue;
        };
        if Some(pane_id) == own_pane_id {
            continue;
        }
        let pids = nvim_pids(pane_id);
        if pids.is_empty() {
            continue;
        }
        let workspace_id = pane["workspace_id"].as_str().unwrap_or_default();
        let (workspace_label, workspace_number) = workspaces
            .get(workspace_id)
            .cloned()
            .unwrap_or_else(|| (workspace_id.to_owned(), 0));
        let (tab_label, tab_number) = pane["tab_id"]
            .as_str()
            .and_then(|tab_id| tabs.get(tab_id).cloned())
            .unwrap_or_default();
        let cwd = pane["foreground_cwd"]
            .as_str()
            .or(pane["cwd"].as_str())
            .unwrap_or_default();
        let session = Session {
            pane_id: pane_id.to_owned(),
            workspace_label,
            workspace_number,
            tab_label,
            tab_number,
            tab_visible: tab_counts.get(workspace_id).copied().unwrap_or_default() > 1,
            cwd: shorten_home(cwd),
            focused: Some(pane_id) == focused_pane,
            file: String::new(),
            modified: 0,
            state: State::Unknown,
        };
        found.push((session, find_server(&pids, &children)));
    }

    thread::scope(|scope| {
        for (session, server) in found.iter_mut() {
            if let Some(server) = server {
                scope.spawn(|| query(session, server));
            }
        }
    });

    let mut sessions: Vec<Session> = found.into_iter().map(|(session, _)| session).collect();
    sessions.sort_by(|a, b| {
        (a.workspace_number, a.tab_number, &a.pane_id).cmp(&(
            b.workspace_number,
            b.tab_number,
            &b.pane_id,
        ))
    });
    Ok(sessions)
}
