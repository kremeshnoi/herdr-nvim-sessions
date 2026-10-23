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
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";
const EMPTY_MESSAGE: &str = " no nvim sessions";
const ROWS_PER_SESSION: usize = 2;
const RIGHT_PADDING: usize = 2;
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
    let content_width = width.saturating_sub(RIGHT_PADDING);
    let mut line = format!("{RESET}{background}");
    let mut used = 0;
    for (text, style) in parts {
        let text = truncate(text, content_width.saturating_sub(used));
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
    line.push_str(RESET);
    line
}

struct Sidebar {
    palette: Palette,
    sessions: Vec<Session>,
    selected_pane: Option<String>,
    focused_pane: Option<String>,
    row_targets: Vec<usize>,
    scroll: usize,
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

    fn update_sessions(&mut self, sessions: Vec<Session>) {
        let focused = sessions
            .iter()
            .find(|session| session.focused)
            .map(|session| session.pane_id.clone());
        if focused.is_some() && focused != self.focused_pane {
            self.selected_pane.clone_from(&focused);
            self.focused_pane = focused;
        }
        self.sessions = sessions;
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
        let _ = rpc("pane.focus", json!({ "pane_id": target }));
        self.selected_pane = Some(target.clone());
        self.focused_pane = Some(target);
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

    fn update_scroll(&mut self, visible_rows: usize) {
        let visible = (visible_rows / ROWS_PER_SESSION).max(1);
        let selected = self.selected_index();
        let max_scroll = self.sessions.len().saturating_sub(visible);
        self.scroll = self
            .scroll
            .min(selected)
            .max((selected + 1).saturating_sub(visible))
            .min(max_scroll);
    }

    fn session_lines(&self, session: &Session, highlighted: bool, width: usize) -> [String; 2] {
        let palette = &self.palette;
        let (row_bg, name_style, secondary) = if highlighted {
            (
                palette.active_row_bg,
                palette.text.fg() + BOLD,
                palette.mauve.fg(),
            )
        } else {
            (
                palette.sidebar_bg,
                palette.subtext0.fg(),
                palette.overlay0.fg(),
            )
        };
        let (icon, icon_color) = status_icon(session.state, palette);
        let title = session.title();
        let mut header = vec![
            (" ", String::new()),
            (icon, icon_color.fg()),
            (" ", String::new()),
            (session.workspace_label.as_str(), name_style),
        ];
        if session.tab_visible && !session.tab_label.is_empty() {
            header.push((" · ", palette.overlay0.fg()));
            header.push((session.tab_label.as_str(), secondary.clone()));
        }
        [
            render_row(&header, width, row_bg),
            render_row(
                &[("   ", String::new()), (title.as_str(), secondary)],
                width,
                row_bg,
            ),
        ]
    }

    fn draw(&mut self, (width, height): (usize, usize), out: &mut impl Write) -> io::Result<()> {
        self.update_scroll(height);
        let palette = &self.palette;
        let selected = self.selected_index();
        let mut lines = Vec::new();
        self.row_targets.clear();

        if self.sessions.is_empty() {
            lines.push(render_row(
                &[(EMPTY_MESSAGE, palette.overlay0.fg() + DIM)],
                width,
                palette.sidebar_bg,
            ));
        }
        for (index, session) in self.sessions.iter().enumerate().skip(self.scroll) {
            if lines.len() + ROWS_PER_SESSION > height {
                break;
            }
            lines.extend(self.session_lines(session, index == selected, width));
            self.row_targets.extend([index; ROWS_PER_SESSION]);
        }

        lines.truncate(height);
        let blank = render_row(&[], width, palette.sidebar_bg);
        lines.resize(height, blank);

        let frame: String = lines
            .iter()
            .enumerate()
            .map(|(row, line)| format!("\x1b[{};1H{line}", row + 1))
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
                self.update_sessions(sessions);
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
        focused_pane: None,
        row_targets: Vec::new(),
        scroll: 0,
        last_frame: String::new(),
        refresh_tx,
    };

    let terminal = Terminal::enter()?;
    sidebar.event_loop(&sessions_rx, &terminal)
}
