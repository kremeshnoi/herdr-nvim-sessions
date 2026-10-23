use std::env;
use std::io::{self, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use serde_json::json;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::herdr::{rpc, subscribe};
use crate::nvim::{self, Session, State};
use crate::terminal::{Input, Terminal};
use crate::theme::{Color, Palette};

const POLL_INTERVAL: Duration = Duration::from_millis(1500);
const INPUT_POLL: Duration = Duration::from_millis(30);
const RESUBSCRIBE_DELAY: Duration = Duration::from_secs(1);
const BOLD: &str = "\x1b[1m";
const HINT: &str = " j/k  enter  r  q";
const EVENT_TYPES: &[&str] = &[
    "pane.created",
    "pane.closed",
    "pane.focused",
    "pane.moved",
    "pane.exited",
    "pane.updated",
    "tab.created",
    "tab.closed",
    "tab.focused",
    "tab.renamed",
    "tab.moved",
    "workspace.created",
    "workspace.closed",
    "workspace.focused",
    "workspace.renamed",
    "workspace.moved",
];

fn spawn_refresher(
    own_pane_id: Option<String>,
    sessions_tx: Sender<Vec<Session>>,
    refresh_rx: Receiver<()>,
) {
    thread::spawn(move || {
        loop {
            if let Ok(sessions) = nvim::collect(own_pane_id.as_deref())
                && sessions_tx.send(sessions).is_err()
            {
                return;
            }
            if let Err(RecvTimeoutError::Disconnected) = refresh_rx.recv_timeout(POLL_INTERVAL) {
                return;
            }
            while refresh_rx.try_recv().is_ok() {}
        }
    });
}

fn spawn_event_listener(refresh_tx: Sender<()>) {
    thread::spawn(move || {
        loop {
            let _ = subscribe(EVENT_TYPES, || {
                let _ = refresh_tx.send(());
            });
            thread::sleep(RESUBSCRIBE_DELAY);
        }
    });
}

fn status_icon(state: State, palette: &Palette) -> (&'static str, Color) {
    match state {
        State::Idle => ("○", palette.green),
        State::Working => ("●", palette.yellow),
        State::Blocked => ("●", palette.red),
        State::Unknown => ("·", palette.overlay0),
    }
}

fn truncate(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut result = String::new();
    let mut used = 0;
    for char in text.chars() {
        let char_width = char.width().unwrap_or(0);
        if used + char_width > width - 1 {
            break;
        }
        result.push(char);
        used += char_width;
    }
    result.push('…');
    result
}

fn render_row(parts: &[(&str, String)], width: usize, row_bg: Color) -> String {
    let background = row_bg.bg();
    let mut line = background.clone();
    let mut used = 0;
    for (text, style) in parts {
        let text = truncate(text, width.saturating_sub(used));
        if text.is_empty() {
            break;
        }
        line.push_str(style);
        line.push_str(&text);
        line.push_str("\x1b[39;22m");
        line.push_str(&background);
        used += text.width();
    }
    line.push_str(&" ".repeat(width.saturating_sub(used)));
    line.push_str("\x1b[0m");
    line
}

struct Sidebar {
    palette: Palette,
    sessions: Vec<Session>,
    selected_pane: Option<String>,
    row_targets: Vec<usize>,
    last_frame: String,
    refresh_tx: Sender<()>,
}

impl Sidebar {
    fn selected_index(&self) -> usize {
        self.selected_pane
            .as_ref()
            .and_then(|pane_id| {
                self.sessions
                    .iter()
                    .position(|session| &session.pane_id == pane_id)
            })
            .unwrap_or(0)
    }

    fn move_selection(&mut self, delta: isize) {
        if self.sessions.is_empty() {
            return;
        }
        let count = self.sessions.len() as isize;
        let index = (self.selected_index() as isize + delta).rem_euclid(count) as usize;
        self.selected_pane = Some(self.sessions[index].pane_id.clone());
    }

    fn activate(&mut self, index: Option<usize>) {
        let index = index.unwrap_or_else(|| self.selected_index());
        let Some(target) = self
            .sessions
            .get(index)
            .map(|session| session.pane_id.clone())
        else {
            return;
        };
        for session in &mut self.sessions {
            session.focused = session.pane_id == target;
        }
        let _ = rpc("pane.focus", json!({ "pane_id": target }));
        self.selected_pane = Some(target);
        let _ = self.refresh_tx.send(());
    }

    fn handle(&mut self, input: Input) -> bool {
        match input {
            Input::Quit => return false,
            Input::Down => self.move_selection(1),
            Input::Up => self.move_selection(-1),
            Input::Enter => self.activate(None),
            Input::Click { row } => {
                if let Some(&index) = self.row_targets.get(row) {
                    self.activate(Some(index));
                }
            }
            Input::Refresh => {
                let _ = self.refresh_tx.send(());
            }
        }
        true
    }

    fn draw(&mut self, (width, height): (usize, usize), out: &mut impl Write) -> io::Result<()> {
        let palette = &self.palette;
        let secondary = palette.overlay0.fg();
        let selected = self.selected_index();
        let mut lines = Vec::new();
        self.row_targets.clear();

        for (index, session) in self.sessions.iter().enumerate() {
            let (row_bg, name_color) = if session.focused {
                (palette.active_row_bg, palette.text)
            } else if index == selected {
                (palette.selection_bg, palette.subtext0)
            } else {
                (Color::Reset, palette.subtext0)
            };
            let marker = if index == selected && row_bg.is_reset() {
                "▌"
            } else {
                " "
            };
            let (icon, icon_color) = status_icon(session.state, palette);
            let title = session.title();

            let mut header = vec![
                (marker, palette.text.fg()),
                (icon, icon_color.fg()),
                (" ", String::new()),
                (session.workspace_label.as_str(), name_color.fg() + BOLD),
            ];
            if !session.tab_label.is_empty() {
                header.push((" · ", secondary.clone()));
                header.push((session.tab_label.as_str(), secondary.clone()));
            }
            lines.push(render_row(&header, width, row_bg));
            lines.push(render_row(
                &[("   ", String::new()), (title.as_str(), secondary.clone())],
                width,
                row_bg,
            ));
            self.row_targets.extend([index, index]);
        }

        lines.truncate(height.saturating_sub(1));
        lines.resize(height.saturating_sub(1), String::new());
        lines.push(render_row(&[(HINT, secondary)], width, Color::Reset));

        let frame: String = lines
            .iter()
            .enumerate()
            .map(|(row, line)| format!("\x1b[{};1H\x1b[2K{line}", row + 1))
            .collect();
        if frame != self.last_frame {
            out.write_all(frame.as_bytes())?;
            out.flush()?;
            self.last_frame = frame;
        }
        Ok(())
    }

    fn event_loop(
        &mut self,
        sessions_rx: &Receiver<Vec<Session>>,
        terminal: &Terminal,
    ) -> io::Result<()> {
        let mut out = io::stdout();
        let mut size = terminal.size();
        loop {
            while let Ok(sessions) = sessions_rx.try_recv() {
                self.sessions = sessions;
            }
            let current_size = terminal.size();
            if current_size != size {
                size = current_size;
                self.last_frame.clear();
            }
            self.draw(size, &mut out)?;
            for input in terminal.read_input(INPUT_POLL)? {
                if !self.handle(input) {
                    return Ok(());
                }
            }
        }
    }
}

pub fn run() -> io::Result<()> {
    let own_pane_id = env::var("HERDR_PANE_ID").ok();
    let (sessions_tx, sessions_rx) = mpsc::channel();
    let (refresh_tx, refresh_rx) = mpsc::channel();
    spawn_refresher(own_pane_id, sessions_tx, refresh_rx);
    spawn_event_listener(refresh_tx.clone());

    let mut sidebar = Sidebar {
        palette: Palette::load(),
        sessions: Vec::new(),
        selected_pane: None,
        row_targets: Vec::new(),
        last_frame: String::new(),
        refresh_tx,
    };

    let terminal = Terminal::enter()?;
    sidebar.event_loop(&sessions_rx, &terminal)
}
