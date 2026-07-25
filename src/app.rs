//! Application state and event loop.

use std::time::{Duration, Instant};

use anyhow::Result;

use crossterm::event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use futures::StreamExt;
use microsandbox::sandbox::LogEntry;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;

use crate::sandbox::{FsEntry, MetricsSnapshot, SandboxInfo, SandboxStatus as Status};
use crate::ui;

//--------------------------------------------------------------------------------------------------
// Constants
//--------------------------------------------------------------------------------------------------

/// How often background tasks refresh sandbox list and metrics.
const REFRESH_INTERVAL: Duration = Duration::from_secs(3);
/// Maximum log lines kept in memory per sandbox.
const MAX_LOG_LINES: usize = 500;

//--------------------------------------------------------------------------------------------------
// Types
//--------------------------------------------------------------------------------------------------

/// Which panel has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    SandboxList,
    Detail,
}

/// Tabs available in the detail panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailTab {
    Logs,
    Metrics,
    Filesystem,
    Info,
}

impl DetailTab {
    pub fn title(self) -> &'static str {
        match self {
            DetailTab::Logs => "Logs",
            DetailTab::Metrics => "Metrics",
            DetailTab::Filesystem => "Filesystem",
            DetailTab::Info => "Info",
        }
    }

    pub fn all() -> &'static [DetailTab] {
        &[
            DetailTab::Logs,
            DetailTab::Metrics,
            DetailTab::Filesystem,
            DetailTab::Info,
        ]
    }
}

/// State of the "create new sandbox" modal dialog.
#[derive(Debug, Clone, Default)]
pub struct CreateDialog {
    pub visible: bool,
    /// Which input field is focused (0=name, 1=image, 2=cpus, 3=memory).
    pub field: usize,
    pub name: String,
    pub image: String,
    pub cpus: String,
    pub memory: String,
    /// Error message shown beneath the form.
    pub error: Option<String>,
}

impl CreateDialog {
    pub fn open() -> Self {
        Self {
            visible: true,
            image: "alpine".into(),
            cpus: "1".into(),
            memory: "512".into(),
            ..Default::default()
        }
    }

    pub fn current_field_mut(&mut self) -> &mut String {
        match self.field {
            0 => &mut self.name,
            1 => &mut self.image,
            2 => &mut self.cpus,
            3 => &mut self.memory,
            _ => &mut self.name,
        }
    }

    pub fn next_field(&mut self) {
        self.field = (self.field + 1) % 4;
    }
}

/// A status notification shown briefly in the footer.
#[derive(Debug, Clone)]
pub struct Notification {
    pub message: String,
    pub is_error: bool,
    pub expires: Instant,
}

/// Background messages sent from async tasks to the main loop.
pub enum AppMessage {
    SandboxList(Result<Vec<SandboxInfo>>),
    LogEntries(String, Result<Vec<LogEntry>>),
    Metrics(String, Result<Option<MetricsSnapshot>>),
    FsEntries(String, String, Result<Option<Vec<FsEntry>>>),
    Notification(String, bool),
}

/// Central application state.
pub struct App {
    /// Full list of known sandboxes.
    pub sandboxes: Vec<SandboxInfo>,
    /// Index of the selected sandbox in the list.
    pub selected: usize,
    /// Which panel has keyboard focus.
    pub focus: Focus,
    /// Which detail tab is active.
    pub tab: DetailTab,
    /// Scroll offset for the log view.
    pub log_scroll: usize,
    /// Scroll offset for the filesystem view.
    pub fs_scroll: usize,
    /// Cached log entries keyed by sandbox name.
    pub logs: std::collections::HashMap<String, Vec<LogEntry>>,
    /// Cached metrics keyed by sandbox name.
    pub metrics: std::collections::HashMap<String, MetricsSnapshot>,
    /// Cached filesystem listing keyed by (name, path).
    pub fs_entries: std::collections::HashMap<(String, String), Vec<FsEntry>>,
    /// Current filesystem path being browsed.
    pub fs_path: String,
    /// Create sandbox dialog state.
    pub create_dialog: CreateDialog,
    /// Transient notification shown at the bottom.
    pub notification: Option<Notification>,
    /// True when the user has requested quit.
    pub should_quit: bool,
    /// Last full refresh timestamp.
    pub last_refresh: Option<Instant>,
    /// Channel for background tasks to push results.
    pub msg_tx: mpsc::UnboundedSender<AppMessage>,
}

//--------------------------------------------------------------------------------------------------
// Methods
//--------------------------------------------------------------------------------------------------

impl App {
    pub fn new(msg_tx: mpsc::UnboundedSender<AppMessage>) -> Self {
        Self {
            sandboxes: Vec::new(),
            selected: 0,
            focus: Focus::SandboxList,
            tab: DetailTab::Logs,
            log_scroll: 0,
            fs_scroll: 0,
            logs: Default::default(),
            metrics: Default::default(),
            fs_entries: Default::default(),
            fs_path: "/".into(),
            create_dialog: Default::default(),
            notification: None,
            should_quit: false,
            last_refresh: None,
            msg_tx,
        }
    }

    /// Return the currently selected sandbox, if any.
    pub fn selected_sandbox(&self) -> Option<&SandboxInfo> {
        self.sandboxes.get(self.selected)
    }

    /// Push a notification that disappears after a few seconds.
    pub fn notify(&mut self, msg: impl Into<String>, is_error: bool) {
        self.notification = Some(Notification {
            message: msg.into(),
            is_error,
            expires: Instant::now() + Duration::from_secs(4),
        });
    }

    /// Select the next sandbox in the list.
    pub fn select_next(&mut self) {
        if self.sandboxes.is_empty() {
            return;
        }
        let next = (self.selected + 1).min(self.sandboxes.len());
        // +1 for the "New Sandbox" entry at the bottom
        if next <= self.sandboxes.len() {
            self.selected = next;
        }
    }

    /// Select the previous sandbox in the list.
    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Returns true if the "New Sandbox" placeholder is selected.
    pub fn new_sandbox_selected(&self) -> bool {
        self.selected == self.sandboxes.len()
    }

    /// Switch to the next detail tab.
    pub fn next_tab(&mut self) {
        let tabs = DetailTab::all();
        let idx = tabs.iter().position(|&t| t == self.tab).unwrap_or(0);
        self.tab = tabs[(idx + 1) % tabs.len()];
        self.reset_detail_scroll();
    }

    /// Switch to the previous detail tab.
    pub fn prev_tab(&mut self) {
        let tabs = DetailTab::all();
        let idx = tabs.iter().position(|&t| t == self.tab).unwrap_or(0);
        self.tab = tabs[(idx + tabs.len() - 1) % tabs.len()];
        self.reset_detail_scroll();
    }

    fn reset_detail_scroll(&mut self) {
        self.log_scroll = 0;
        self.fs_scroll = 0;
    }

    /// Trigger a background refresh of the sandbox list.
    pub fn request_refresh(&self) {
        let tx = self.msg_tx.clone();
        tokio::spawn(async move {
            let result = crate::sandbox::list_sandboxes().await;
            let _ = tx.send(AppMessage::SandboxList(result));
        });
    }

    /// Trigger a background log fetch for the selected sandbox.
    pub fn request_logs(&self, name: &str) {
        let tx = self.msg_tx.clone();
        let name = name.to_owned();
        tokio::spawn(async move {
            let result = crate::sandbox::read_logs(&name, Some(MAX_LOG_LINES)).await;
            let _ = tx.send(AppMessage::LogEntries(name, result));
        });
    }

    /// Trigger a background metrics fetch for the selected sandbox.
    pub fn request_metrics(&self, name: &str) {
        let tx = self.msg_tx.clone();
        let name = name.to_owned();
        tokio::spawn(async move {
            let result = crate::sandbox::fetch_metrics(&name).await;
            let _ = tx.send(AppMessage::Metrics(name, result));
        });
    }

    /// Trigger a background filesystem listing.
    pub fn request_fs(&self, name: &str, path: &str) {
        let tx = self.msg_tx.clone();
        let name = name.to_owned();
        let path = path.to_owned();
        tokio::spawn(async move {
            let result = crate::sandbox::list_fs(&name, &path).await;
            let _ = tx.send(AppMessage::FsEntries(name, path, result));
        });
    }

    /// Perform a sandbox action (start/stop/kill/remove) in the background.
    pub fn run_action(&self, action: SandboxAction, name: &str) {
        let tx = self.msg_tx.clone();
        let name = name.to_owned();
        tokio::spawn(async move {
            let result: Result<()> = match action {
                SandboxAction::Start => crate::sandbox::start_sandbox(&name).await,
                SandboxAction::Stop => crate::sandbox::stop_sandbox(&name).await,
                SandboxAction::Kill => crate::sandbox::kill_sandbox(&name).await,
                SandboxAction::Remove => crate::sandbox::remove_sandbox(&name).await,
            };
            let (msg, is_err) = match result {
                Ok(()) => (format!("{action:?} '{name}' OK"), false),
                Err(e) => (format!("{action:?} '{name}' failed: {e}"), true),
            };
            let _ = tx.send(AppMessage::Notification(msg, is_err));
            // Trigger a refresh after the action
            tokio::time::sleep(Duration::from_millis(500)).await;
            if let Ok(list) = crate::sandbox::list_sandboxes().await {
                let _ = tx.send(AppMessage::SandboxList(Ok(list)));
            }
        });
    }

    /// Handle an incoming background message.
    pub fn handle_message(&mut self, msg: AppMessage) {
        match msg {
            AppMessage::SandboxList(Ok(list)) => {
                // Preserve selection when list changes
                let selected_name = self.sandboxes.get(self.selected).map(|s| s.name.clone());
                self.sandboxes = list;
                if let Some(name) = selected_name {
                    if let Some(idx) = self.sandboxes.iter().position(|s| s.name == name) {
                        self.selected = idx;
                    } else {
                        self.selected = self.selected.min(self.sandboxes.len());
                    }
                }
                self.last_refresh = Some(Instant::now());
            }
            AppMessage::SandboxList(Err(e)) => {
                self.notify(format!("List error: {e}"), true);
            }
            AppMessage::LogEntries(name, Ok(entries)) => {
                self.logs.insert(name, entries);
            }
            AppMessage::LogEntries(name, Err(e)) => {
                self.notify(format!("Log error for {name}: {e}"), true);
            }
            AppMessage::Metrics(name, Ok(Some(m))) => {
                self.metrics.insert(name, m);
            }
            AppMessage::Metrics(_, Ok(None)) => {}
            AppMessage::Metrics(name, Err(e)) => {
                self.notify(format!("Metrics error for {name}: {e}"), true);
            }
            AppMessage::FsEntries(name, path, Ok(Some(entries))) => {
                self.fs_entries.insert((name, path), entries);
            }
            AppMessage::FsEntries(_, _, Ok(None)) => {}
            AppMessage::FsEntries(name, path, Err(e)) => {
                self.notify(format!("FS error for {name}:{path}: {e}"), true);
            }
            AppMessage::Notification(msg, is_err) => {
                self.notify(msg, is_err);
            }
        }
    }
}

//--------------------------------------------------------------------------------------------------
// Action enum
//--------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub enum SandboxAction {
    Start,
    Stop,
    Kill,
    Remove,
}

//--------------------------------------------------------------------------------------------------
// Event loop
//--------------------------------------------------------------------------------------------------

/// Run the full TUI application until the user quits.
pub async fn run(terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>) -> Result<()> {
    let (msg_tx, mut msg_rx) = mpsc::unbounded_channel::<AppMessage>();
    let mut app = App::new(msg_tx);

    // Initial data fetch
    app.request_refresh();

    let mut event_stream = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let mut refresh_tick = tokio::time::interval(REFRESH_INTERVAL);

    loop {
        // Expire stale notification
        if let Some(ref n) = app.notification {
            if Instant::now() >= n.expires {
                app.notification = None;
            }
        }

        terminal.draw(|f| ui::render(f, &mut app))?;

        tokio::select! {
            _ = tick.tick() => {
                // just redraw
            }

            _ = refresh_tick.tick() => {
                app.request_refresh();
                // Also refresh logs/metrics for the selected sandbox
                if let Some(sb) = app.selected_sandbox().cloned() {
                    match app.tab {
                        DetailTab::Logs => app.request_logs(&sb.name),
                        DetailTab::Metrics => app.request_metrics(&sb.name),
                        DetailTab::Filesystem => {
                            let path = app.fs_path.clone();
                            app.request_fs(&sb.name, &path);
                        }
                        DetailTab::Info => {}
                    }
                }
            }

            Some(msg) = msg_rx.recv() => {
                app.handle_message(msg);
            }

            maybe_event = event_stream.next() => {
                if let Some(Ok(event)) = maybe_event {
                    handle_event(&mut app, event);
                }
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

//--------------------------------------------------------------------------------------------------
// Input handling
//--------------------------------------------------------------------------------------------------

pub(crate) fn handle_event(app: &mut App, event: Event) {
    let Event::Key(key) = event else { return };

    // Only act on key presses (ignore repeat and release events from enhanced terminals)
    if key.kind != KeyEventKind::Press {
        return;
    }

    // Modal dialog steals all input
    if app.create_dialog.visible {
        handle_dialog_key(app, key.code, key.modifiers);
        return;
    }

    match key.code {
        // Global quit
        KeyCode::Char('q') | KeyCode::Char('Q') => app.should_quit = true,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.should_quit = true
        }

        // Esc: move focus back to the sandbox list from the detail panel
        KeyCode::Esc => {
            app.focus = Focus::SandboxList;
        }

        // Focus switching
        KeyCode::Tab => {
            app.focus = match app.focus {
                Focus::SandboxList => Focus::Detail,
                Focus::Detail => Focus::SandboxList,
            };
        }

        // Navigation depends on focus
        KeyCode::Up | KeyCode::Char('k') => {
            if app.focus == Focus::SandboxList {
                app.select_prev();
                on_sandbox_selected(app);
            } else {
                scroll_up(app);
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.focus == Focus::SandboxList {
                app.select_next();
                on_sandbox_selected(app);
            } else {
                scroll_down(app);
            }
        }

        // Detail panel tab switching
        KeyCode::Left | KeyCode::Char('h') if app.focus == Focus::Detail => {
            app.prev_tab();
            on_tab_switched(app);
        }
        KeyCode::Right | KeyCode::Char('l') if app.focus == Focus::Detail => {
            app.next_tab();
            on_tab_switched(app);
        }

        // Sandbox actions (only when focus is on the list)
        KeyCode::Char('s') if app.focus == Focus::SandboxList => {
            action_start(app);
        }
        KeyCode::Char('S') if app.focus == Focus::SandboxList => {
            action_stop(app);
        }
        KeyCode::Char('K') if app.focus == Focus::SandboxList => {
            action_kill(app);
        }
        KeyCode::Char('d') if app.focus == Focus::SandboxList => {
            action_remove(app);
        }
        KeyCode::Enter => {
            if app.new_sandbox_selected() {
                app.create_dialog = CreateDialog::open();
            } else if app.focus == Focus::SandboxList {
                app.focus = Focus::Detail;
            }
        }
        KeyCode::Char('n') => {
            app.create_dialog = CreateDialog::open();
        }
        KeyCode::Char('r') => {
            app.request_refresh();
            app.notify("Refreshing…", false);
        }

        // Filesystem navigation
        KeyCode::Backspace if app.focus == Focus::Detail && app.tab == DetailTab::Filesystem => {
            nav_fs_up(app);
        }

        _ => {}
    }
}

fn handle_dialog_key(app: &mut App, code: KeyCode, _mods: KeyModifiers) {
    match code {
        KeyCode::Esc => {
            app.create_dialog = Default::default();
        }
        KeyCode::Tab => app.create_dialog.next_field(),
        KeyCode::BackTab => {
            let f = app.create_dialog.field;
            app.create_dialog.field = if f == 0 { 3 } else { f - 1 };
        }
        KeyCode::Enter => {
            submit_create_dialog(app);
        }
        KeyCode::Backspace => {
            app.create_dialog.current_field_mut().pop();
        }
        KeyCode::Char(c) => {
            app.create_dialog.current_field_mut().push(c);
        }
        _ => {}
    }
}

fn submit_create_dialog(app: &mut App) {
    let name = app.create_dialog.name.trim().to_owned();
    let image = app.create_dialog.image.trim().to_owned();
    let cpus: u8 = app
        .create_dialog
        .cpus
        .trim()
        .parse()
        .unwrap_or(1)
        .max(1)
        .min(32);
    let memory: u32 = app
        .create_dialog
        .memory
        .trim()
        .parse()
        .unwrap_or(512)
        .max(64);

    if name.is_empty() {
        app.create_dialog.error = Some("Name is required".into());
        return;
    }
    if image.is_empty() {
        app.create_dialog.error = Some("Image is required".into());
        return;
    }

    app.create_dialog = Default::default();

    let tx = app.msg_tx.clone();
    tokio::spawn(async move {
        let result = crate::sandbox::create_sandbox(&name, &image, cpus, memory).await;
        let (msg, is_err) = match result {
            Ok(()) => (format!("Created sandbox '{name}'"), false),
            Err(e) => (format!("Create failed: {e}"), true),
        };
        let _ = tx.send(AppMessage::Notification(msg, is_err));
        tokio::time::sleep(Duration::from_millis(300)).await;
        if let Ok(list) = crate::sandbox::list_sandboxes().await {
            let _ = tx.send(AppMessage::SandboxList(Ok(list)));
        }
    });
}

//--------------------------------------------------------------------------------------------------
// Helpers
//--------------------------------------------------------------------------------------------------

fn on_sandbox_selected(app: &mut App) {
    app.log_scroll = 0;
    app.fs_scroll = 0;
    app.fs_path = "/".into();
    if let Some(sb) = app.selected_sandbox().cloned() {
        match app.tab {
            DetailTab::Logs => app.request_logs(&sb.name),
            DetailTab::Metrics => app.request_metrics(&sb.name),
            DetailTab::Filesystem => {
                let path = app.fs_path.clone();
                app.request_fs(&sb.name, &path);
            }
            DetailTab::Info => {}
        }
    }
}

fn on_tab_switched(app: &mut App) {
    if let Some(sb) = app.selected_sandbox().cloned() {
        match app.tab {
            DetailTab::Logs => app.request_logs(&sb.name),
            DetailTab::Metrics => app.request_metrics(&sb.name),
            DetailTab::Filesystem => {
                let path = app.fs_path.clone();
                app.request_fs(&sb.name, &path);
            }
            DetailTab::Info => {}
        }
    }
}

fn scroll_up(app: &mut App) {
    match app.tab {
        DetailTab::Logs => app.log_scroll = app.log_scroll.saturating_sub(3),
        DetailTab::Filesystem => app.fs_scroll = app.fs_scroll.saturating_sub(1),
        _ => {}
    }
}

fn scroll_down(app: &mut App) {
    match app.tab {
        DetailTab::Logs => app.log_scroll = app.log_scroll.saturating_add(3),
        DetailTab::Filesystem => app.fs_scroll = app.fs_scroll.saturating_add(1),
        _ => {}
    }
}

fn nav_fs_up(app: &mut App) {
    let path = std::path::Path::new(&app.fs_path);
    if let Some(parent) = path.parent() {
        let parent_str = parent.to_string_lossy().into_owned();
        let parent_str = if parent_str.is_empty() { "/".into() } else { parent_str };
        app.fs_path = parent_str.clone();
        app.fs_scroll = 0;
        if let Some(sb) = app.selected_sandbox().cloned() {
            app.request_fs(&sb.name, &parent_str);
        }
    }
}

fn action_start(app: &mut App) {
    if let Some(sb) = app.selected_sandbox().cloned() {
        if sb.status == Status::Stopped {
            app.run_action(SandboxAction::Start, &sb.name);
            app.notify(format!("Starting '{}'…", sb.name), false);
        } else {
            app.notify("Sandbox is not stopped", true);
        }
    }
}

fn action_stop(app: &mut App) {
    if let Some(sb) = app.selected_sandbox().cloned() {
        if sb.status == Status::Running {
            app.run_action(SandboxAction::Stop, &sb.name);
            app.notify(format!("Stopping '{}'…", sb.name), false);
        } else {
            app.notify("Sandbox is not running", true);
        }
    }
}

fn action_kill(app: &mut App) {
    if let Some(sb) = app.selected_sandbox().cloned() {
        if sb.status == Status::Running {
            app.run_action(SandboxAction::Kill, &sb.name);
            app.notify(format!("Killing '{}'…", sb.name), false);
        }
    }
}

fn action_remove(app: &mut App) {
    if let Some(sb) = app.selected_sandbox().cloned() {
        if sb.status != Status::Running {
            app.run_action(SandboxAction::Remove, &sb.name);
            app.notify(format!("Removing '{}'…", sb.name), false);
        } else {
            app.notify("Stop the sandbox before removing", true);
        }
    }
}

//--------------------------------------------------------------------------------------------------
// Tests
//--------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyEventState};

    // ── Helpers ─────────────────────────────────────────────────────────────

    /// Build an App with a disconnected message channel (the sender is kept alive
    /// so the channel stays open; the receiver is dropped and never polled).
    fn make_app() -> App {
        let (tx, _rx) = mpsc::unbounded_channel();
        App::new(tx)
    }

    /// Construct a sandbox info with the given name and status.
    fn make_sandbox(name: &str, status: Status) -> SandboxInfo {
        SandboxInfo {
            name: name.into(),
            status,
            image: "alpine:latest".into(),
            cpus: 1,
            memory_mib: 512,
            created_at: None,
            updated_at: None,
        }
    }

    /// Build a key-press Event.
    fn key_press(code: KeyCode) -> Event {
        Event::Key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    /// Build a key-press Event with modifiers.
    fn key_press_mod(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    /// Build a key-release Event (should be ignored by handle_event).
    fn key_release(code: KeyCode) -> Event {
        Event::Key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Release,
            state: KeyEventState::NONE,
        })
    }

    // ── App initial state ────────────────────────────────────────────────────

    #[test]
    fn test_initial_state() {
        let app = make_app();
        assert!(app.sandboxes.is_empty());
        assert_eq!(app.selected, 0);
        assert_eq!(app.focus, Focus::SandboxList);
        assert_eq!(app.tab, DetailTab::Logs);
        assert!(!app.should_quit);
        assert!(!app.create_dialog.visible);
        assert!(app.notification.is_none());
        assert_eq!(app.fs_path, "/");
    }

    // ── selected_sandbox ────────────────────────────────────────────────────

    #[test]
    fn test_selected_sandbox_empty() {
        let app = make_app();
        assert!(app.selected_sandbox().is_none());
    }

    #[test]
    fn test_selected_sandbox_first() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("alpha", Status::Running));
        assert_eq!(app.selected_sandbox().unwrap().name, "alpha");
    }

    #[test]
    fn test_selected_sandbox_second() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("alpha", Status::Running));
        app.sandboxes.push(make_sandbox("beta", Status::Stopped));
        app.selected = 1;
        assert_eq!(app.selected_sandbox().unwrap().name, "beta");
    }

    // ── new_sandbox_selected ─────────────────────────────────────────────────

    #[test]
    fn test_new_sandbox_selected_empty_list() {
        // With no sandboxes selected==0==len() → new sandbox is selected
        let app = make_app();
        assert!(app.new_sandbox_selected());
    }

    #[test]
    fn test_new_sandbox_selected_false_when_sandbox_focused() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("alpha", Status::Running));
        assert!(!app.new_sandbox_selected());
    }

    #[test]
    fn test_new_sandbox_selected_true_at_end() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("alpha", Status::Running));
        app.selected = 1; // == len()
        assert!(app.new_sandbox_selected());
    }

    // ── select_next / select_prev ────────────────────────────────────────────

    #[test]
    fn test_select_next_empty_does_nothing() {
        let mut app = make_app();
        app.select_next();
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn test_select_next_advances() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("a", Status::Running));
        app.sandboxes.push(make_sandbox("b", Status::Running));
        app.select_next();
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn test_select_next_can_reach_new_sandbox_slot() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("a", Status::Running));
        app.select_next(); // selected = 1 == len() → "New Sandbox"
        assert!(app.new_sandbox_selected());
    }

    #[test]
    fn test_select_next_stops_at_new_sandbox_slot() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("a", Status::Running));
        app.selected = 1; // already at "New Sandbox"
        app.select_next();
        assert_eq!(app.selected, 1); // stays
    }

    #[test]
    fn test_select_prev_stops_at_zero() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("a", Status::Running));
        app.select_prev(); // already at 0
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn test_select_prev_moves_back() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("a", Status::Running));
        app.sandboxes.push(make_sandbox("b", Status::Running));
        app.selected = 1;
        app.select_prev();
        assert_eq!(app.selected, 0);
    }

    // ── next_tab / prev_tab ──────────────────────────────────────────────────

    #[test]
    fn test_next_tab_cycles_forward() {
        let mut app = make_app();
        assert_eq!(app.tab, DetailTab::Logs);
        app.next_tab();
        assert_eq!(app.tab, DetailTab::Metrics);
        app.next_tab();
        assert_eq!(app.tab, DetailTab::Filesystem);
        app.next_tab();
        assert_eq!(app.tab, DetailTab::Info);
        app.next_tab();
        assert_eq!(app.tab, DetailTab::Logs); // wraps
    }

    #[test]
    fn test_prev_tab_cycles_backward() {
        let mut app = make_app();
        app.prev_tab();
        assert_eq!(app.tab, DetailTab::Info); // wraps
        app.prev_tab();
        assert_eq!(app.tab, DetailTab::Filesystem);
        app.prev_tab();
        assert_eq!(app.tab, DetailTab::Metrics);
        app.prev_tab();
        assert_eq!(app.tab, DetailTab::Logs);
    }

    #[test]
    fn test_tab_switch_resets_scroll() {
        let mut app = make_app();
        app.log_scroll = 10;
        app.fs_scroll = 5;
        app.next_tab();
        assert_eq!(app.log_scroll, 0);
        assert_eq!(app.fs_scroll, 0);
    }

    // ── notify ───────────────────────────────────────────────────────────────

    #[test]
    fn test_notify_sets_message() {
        let mut app = make_app();
        app.notify("hello", false);
        let n = app.notification.as_ref().unwrap();
        assert_eq!(n.message, "hello");
        assert!(!n.is_error);
    }

    #[test]
    fn test_notify_error_flag() {
        let mut app = make_app();
        app.notify("boom", true);
        assert!(app.notification.as_ref().unwrap().is_error);
    }

    #[test]
    fn test_notify_replaces_previous() {
        let mut app = make_app();
        app.notify("first", false);
        app.notify("second", true);
        assert_eq!(app.notification.as_ref().unwrap().message, "second");
    }

    // ── CreateDialog ─────────────────────────────────────────────────────────

    #[test]
    fn test_create_dialog_default_is_closed() {
        let dlg = CreateDialog::default();
        assert!(!dlg.visible);
        assert_eq!(dlg.field, 0);
        assert!(dlg.name.is_empty());
    }

    #[test]
    fn test_create_dialog_open_has_defaults() {
        let dlg = CreateDialog::open();
        assert!(dlg.visible);
        assert_eq!(dlg.field, 0);
        assert_eq!(dlg.image, "alpine");
        assert_eq!(dlg.cpus, "1");
        assert_eq!(dlg.memory, "512");
        assert!(dlg.name.is_empty());
        assert!(dlg.error.is_none());
    }

    #[test]
    fn test_create_dialog_next_field_wraps() {
        let mut dlg = CreateDialog::open();
        dlg.next_field(); assert_eq!(dlg.field, 1);
        dlg.next_field(); assert_eq!(dlg.field, 2);
        dlg.next_field(); assert_eq!(dlg.field, 3);
        dlg.next_field(); assert_eq!(dlg.field, 0); // wraps
    }

    #[test]
    fn test_create_dialog_current_field_mut() {
        let mut dlg = CreateDialog::open();
        dlg.field = 0; dlg.current_field_mut().push_str("mybox"); assert_eq!(dlg.name, "mybox");
        dlg.field = 1; dlg.current_field_mut().push_str("ubuntu"); assert_eq!(dlg.image, "alpineubuntu");
        dlg.field = 2; dlg.current_field_mut().push_str("4"); assert_eq!(dlg.cpus, "14");
        dlg.field = 3; dlg.current_field_mut().push_str("1024"); assert_eq!(dlg.memory, "5121024");
    }

    // ── DetailTab helpers ────────────────────────────────────────────────────

    #[test]
    fn test_detail_tab_titles() {
        assert_eq!(DetailTab::Logs.title(), "Logs");
        assert_eq!(DetailTab::Metrics.title(), "Metrics");
        assert_eq!(DetailTab::Filesystem.title(), "Filesystem");
        assert_eq!(DetailTab::Info.title(), "Info");
    }

    #[test]
    fn test_detail_tab_all_has_four_entries() {
        assert_eq!(DetailTab::all().len(), 4);
    }

    // ── handle_message ───────────────────────────────────────────────────────

    #[test]
    fn test_handle_message_sandbox_list_populates() {
        let mut app = make_app();
        let list = vec![make_sandbox("alpha", Status::Running)];
        app.handle_message(AppMessage::SandboxList(Ok(list)));
        assert_eq!(app.sandboxes.len(), 1);
        assert_eq!(app.sandboxes[0].name, "alpha");
        assert!(app.last_refresh.is_some());
    }

    #[test]
    fn test_handle_message_sandbox_list_error_notifies() {
        let mut app = make_app();
        app.handle_message(AppMessage::SandboxList(Err(anyhow::anyhow!("conn refused"))));
        let n = app.notification.as_ref().unwrap();
        assert!(n.is_error);
        assert!(n.message.contains("conn refused"));
    }

    #[test]
    fn test_handle_message_preserves_selection_by_name() {
        let mut app = make_app();
        app.sandboxes = vec![
            make_sandbox("alpha", Status::Running),
            make_sandbox("beta", Status::Stopped),
        ];
        app.selected = 1; // "beta"

        // Refresh with beta still present but in a new position (alpha removed)
        let new_list = vec![make_sandbox("beta", Status::Running)];
        app.handle_message(AppMessage::SandboxList(Ok(new_list)));
        assert_eq!(app.selected, 0); // beta moved to index 0
        assert_eq!(app.sandboxes[app.selected].name, "beta");
    }

    #[test]
    fn test_handle_message_selection_clamps_when_sandbox_removed() {
        let mut app = make_app();
        app.sandboxes = vec![
            make_sandbox("alpha", Status::Running),
            make_sandbox("beta", Status::Stopped),
        ];
        app.selected = 1; // "beta" — will be removed

        let new_list = vec![make_sandbox("alpha", Status::Running)];
        app.handle_message(AppMessage::SandboxList(Ok(new_list)));
        // beta gone, selected should clamp to len() == 1 (new sandbox slot)
        assert!(app.selected <= app.sandboxes.len());
    }

    #[test]
    fn test_handle_message_log_entries_stored() {
        let mut app = make_app();
        app.handle_message(AppMessage::LogEntries("mybox".into(), Ok(vec![])));
        assert!(app.logs.contains_key("mybox"));
    }

    #[test]
    fn test_handle_message_log_error_notifies() {
        let mut app = make_app();
        app.handle_message(AppMessage::LogEntries("mybox".into(), Err(anyhow::anyhow!("err"))));
        assert!(app.notification.as_ref().unwrap().is_error);
    }

    #[test]
    fn test_handle_message_metrics_stored() {
        let mut app = make_app();
        let m = MetricsSnapshot { cpu_percent: 42.0, memory_bytes: 1024, ..Default::default() };
        app.handle_message(AppMessage::Metrics("mybox".into(), Ok(Some(m))));
        assert_eq!(app.metrics["mybox"].cpu_percent, 42.0);
    }

    #[test]
    fn test_handle_message_metrics_none_no_panic() {
        let mut app = make_app();
        app.handle_message(AppMessage::Metrics("mybox".into(), Ok(None)));
        assert!(!app.metrics.contains_key("mybox"));
    }

    #[test]
    fn test_handle_message_metrics_error_notifies() {
        let mut app = make_app();
        app.handle_message(AppMessage::Metrics("mybox".into(), Err(anyhow::anyhow!("x"))));
        assert!(app.notification.as_ref().unwrap().is_error);
    }

    #[test]
    fn test_handle_message_fs_entries_stored() {
        let mut app = make_app();
        let entries = vec![crate::sandbox::FsEntry {
            path: "/etc".into(),
            kind: crate::sandbox::LocalFsEntryKind::Directory,
            size: 0,
        }];
        app.handle_message(AppMessage::FsEntries("mybox".into(), "/".into(), Ok(Some(entries))));
        assert!(app.fs_entries.contains_key(&("mybox".into(), "/".into())));
    }

    #[test]
    fn test_handle_message_fs_none_no_panic() {
        let mut app = make_app();
        app.handle_message(AppMessage::FsEntries("mybox".into(), "/".into(), Ok(None)));
        assert!(app.fs_entries.is_empty());
    }

    #[test]
    fn test_handle_message_fs_error_notifies() {
        let mut app = make_app();
        app.handle_message(AppMessage::FsEntries("mybox".into(), "/".into(), Err(anyhow::anyhow!("x"))));
        assert!(app.notification.as_ref().unwrap().is_error);
    }

    #[test]
    fn test_handle_message_notification() {
        let mut app = make_app();
        app.handle_message(AppMessage::Notification("done".into(), false));
        let n = app.notification.as_ref().unwrap();
        assert_eq!(n.message, "done");
        assert!(!n.is_error);
    }

    // ── handle_event: key-event filtering ───────────────────────────────────

    #[test]
    fn test_release_events_are_ignored() {
        let mut app = make_app();
        // Releasing 'q' must not quit
        handle_event(&mut app, key_release(KeyCode::Char('q')));
        assert!(!app.should_quit);
    }

    #[test]
    fn test_non_key_events_are_ignored() {
        let mut app = make_app();
        handle_event(&mut app, Event::FocusGained);
        assert!(!app.should_quit);
    }

    // ── handle_event: global keys ────────────────────────────────────────────

    #[test]
    fn test_q_quits() {
        let mut app = make_app();
        handle_event(&mut app, key_press(KeyCode::Char('q')));
        assert!(app.should_quit);
    }

    #[test]
    fn test_shift_q_quits() {
        let mut app = make_app();
        handle_event(&mut app, key_press(KeyCode::Char('Q')));
        assert!(app.should_quit);
    }

    #[test]
    fn test_ctrl_c_quits() {
        let mut app = make_app();
        handle_event(&mut app, key_press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.should_quit);
    }

    // ── handle_event: Esc behaviour ──────────────────────────────────────────

    #[test]
    fn test_esc_closes_dialog() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        assert!(app.create_dialog.visible);
        handle_event(&mut app, key_press(KeyCode::Esc));
        assert!(!app.create_dialog.visible);
    }

    #[test]
    fn test_esc_does_not_quit() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        handle_event(&mut app, key_press(KeyCode::Esc));
        assert!(!app.should_quit);
    }

    #[test]
    fn test_esc_outside_dialog_moves_focus_to_list() {
        let mut app = make_app();
        app.focus = Focus::Detail;
        handle_event(&mut app, key_press(KeyCode::Esc));
        assert_eq!(app.focus, Focus::SandboxList);
    }

    // ── handle_event: dialog input ───────────────────────────────────────────

    #[test]
    fn test_dialog_tab_advances_field() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        handle_event(&mut app, key_press(KeyCode::Tab));
        assert_eq!(app.create_dialog.field, 1);
    }

    #[test]
    fn test_dialog_backtab_goes_back() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.field = 2;
        handle_event(&mut app, key_press(KeyCode::BackTab));
        assert_eq!(app.create_dialog.field, 1);
    }

    #[test]
    fn test_dialog_backtab_from_first_wraps_to_last() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        // field == 0
        handle_event(&mut app, key_press(KeyCode::BackTab));
        assert_eq!(app.create_dialog.field, 3);
    }

    #[test]
    fn test_dialog_char_appends_to_field() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.field = 0; // name
        handle_event(&mut app, key_press(KeyCode::Char('x')));
        assert_eq!(app.create_dialog.name, "x");
    }

    #[test]
    fn test_dialog_backspace_removes_char() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.name = "ab".into();
        handle_event(&mut app, key_press(KeyCode::Backspace));
        assert_eq!(app.create_dialog.name, "a");
    }

    #[test]
    fn test_dialog_enter_with_empty_name_sets_error() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.name = "".into();
        handle_event(&mut app, key_press(KeyCode::Enter));
        assert!(app.create_dialog.error.is_some());
        // Dialog stays open on validation error
        assert!(app.create_dialog.visible);
    }

    #[test]
    fn test_dialog_enter_with_empty_image_sets_error() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.name = "mybox".into();
        app.create_dialog.image = "".into();
        handle_event(&mut app, key_press(KeyCode::Enter));
        assert!(app.create_dialog.error.is_some());
        assert!(app.create_dialog.visible);
    }

    // ── handle_event: focus & navigation ────────────────────────────────────

    #[test]
    fn test_tab_switches_focus_list_to_detail() {
        let mut app = make_app();
        assert_eq!(app.focus, Focus::SandboxList);
        handle_event(&mut app, key_press(KeyCode::Tab));
        assert_eq!(app.focus, Focus::Detail);
    }

    #[test]
    fn test_tab_switches_focus_detail_to_list() {
        let mut app = make_app();
        app.focus = Focus::Detail;
        handle_event(&mut app, key_press(KeyCode::Tab));
        assert_eq!(app.focus, Focus::SandboxList);
    }

    #[test]
    fn test_n_opens_dialog() {
        let mut app = make_app();
        handle_event(&mut app, key_press(KeyCode::Char('n')));
        assert!(app.create_dialog.visible);
    }

    #[tokio::test]
    async fn test_down_moves_selection_in_list() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("a", Status::Running));
        app.sandboxes.push(make_sandbox("b", Status::Running));
        handle_event(&mut app, key_press(KeyCode::Down));
        assert_eq!(app.selected, 1);
    }

    #[tokio::test]
    async fn test_j_moves_selection_in_list() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("a", Status::Running));
        app.sandboxes.push(make_sandbox("b", Status::Running));
        handle_event(&mut app, key_press(KeyCode::Char('j')));
        assert_eq!(app.selected, 1);
    }

    #[tokio::test]
    async fn test_up_moves_selection_in_list() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("a", Status::Running));
        app.sandboxes.push(make_sandbox("b", Status::Running));
        app.selected = 1;
        handle_event(&mut app, key_press(KeyCode::Up));
        assert_eq!(app.selected, 0);
    }

    #[tokio::test]
    async fn test_k_moves_selection_in_list() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("a", Status::Running));
        app.sandboxes.push(make_sandbox("b", Status::Running));
        app.selected = 1;
        handle_event(&mut app, key_press(KeyCode::Char('k')));
        assert_eq!(app.selected, 0);
    }

    #[tokio::test]
    async fn test_right_advances_tab_in_detail_focus() {
        let mut app = make_app();
        app.focus = Focus::Detail;
        handle_event(&mut app, key_press(KeyCode::Right));
        assert_eq!(app.tab, DetailTab::Metrics);
    }

    #[tokio::test]
    async fn test_l_advances_tab_in_detail_focus() {
        let mut app = make_app();
        app.focus = Focus::Detail;
        handle_event(&mut app, key_press(KeyCode::Char('l')));
        assert_eq!(app.tab, DetailTab::Metrics);
    }

    #[tokio::test]
    async fn test_left_goes_back_tab_in_detail_focus() {
        let mut app = make_app();
        app.focus = Focus::Detail;
        app.tab = DetailTab::Metrics;
        handle_event(&mut app, key_press(KeyCode::Left));
        assert_eq!(app.tab, DetailTab::Logs);
    }

    #[test]
    fn test_right_does_nothing_in_list_focus() {
        let mut app = make_app();
        assert_eq!(app.focus, Focus::SandboxList);
        handle_event(&mut app, key_press(KeyCode::Right));
        assert_eq!(app.tab, DetailTab::Logs); // unchanged
    }

    #[tokio::test]
    async fn test_enter_on_sandbox_moves_focus_to_detail() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("a", Status::Running));
        handle_event(&mut app, key_press(KeyCode::Enter));
        assert_eq!(app.focus, Focus::Detail);
    }

    #[test]
    fn test_enter_on_new_sandbox_opens_dialog() {
        let mut app = make_app();
        // selected == 0 == len() → new sandbox slot
        handle_event(&mut app, key_press(KeyCode::Enter));
        assert!(app.create_dialog.visible);
    }

    // ── handle_event: sandbox actions ────────────────────────────────────────

    #[tokio::test]
    async fn test_s_start_notifies_when_sandbox_stopped() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("box1", Status::Stopped));
        handle_event(&mut app, key_press(KeyCode::Char('s')));
        let n = app.notification.as_ref().unwrap();
        assert!(!n.is_error);
        assert!(n.message.contains("box1"));
    }

    #[tokio::test]
    async fn test_s_start_notifies_error_when_sandbox_running() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("box1", Status::Running));
        handle_event(&mut app, key_press(KeyCode::Char('s')));
        let n = app.notification.as_ref().unwrap();
        assert!(n.is_error);
    }

    #[tokio::test]
    async fn test_shift_s_stop_notifies_when_sandbox_running() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("box1", Status::Running));
        handle_event(&mut app, key_press(KeyCode::Char('S')));
        let n = app.notification.as_ref().unwrap();
        assert!(!n.is_error);
        assert!(n.message.contains("box1"));
    }

    #[tokio::test]
    async fn test_shift_s_stop_error_when_sandbox_stopped() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("box1", Status::Stopped));
        handle_event(&mut app, key_press(KeyCode::Char('S')));
        assert!(app.notification.as_ref().unwrap().is_error);
    }

    #[tokio::test]
    async fn test_d_remove_notifies_when_sandbox_stopped() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("box1", Status::Stopped));
        handle_event(&mut app, key_press(KeyCode::Char('d')));
        let n = app.notification.as_ref().unwrap();
        assert!(!n.is_error);
        assert!(n.message.contains("box1"));
    }

    #[tokio::test]
    async fn test_d_remove_error_when_sandbox_running() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("box1", Status::Running));
        handle_event(&mut app, key_press(KeyCode::Char('d')));
        assert!(app.notification.as_ref().unwrap().is_error);
    }

    #[tokio::test]
    async fn test_actions_ignored_in_detail_focus() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("box1", Status::Stopped));
        app.focus = Focus::Detail;
        // 's' in detail focus should not produce a notification
        handle_event(&mut app, key_press(KeyCode::Char('s')));
        assert!(app.notification.is_none());
    }

    // ── scroll helpers ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_down_scrolls_logs_in_detail_focus() {
        let mut app = make_app();
        app.focus = Focus::Detail;
        app.tab = DetailTab::Logs;
        handle_event(&mut app, key_press(KeyCode::Down));
        assert!(app.log_scroll > 0);
    }

    #[tokio::test]
    async fn test_up_scrolls_logs_in_detail_focus() {
        let mut app = make_app();
        app.focus = Focus::Detail;
        app.tab = DetailTab::Logs;
        app.log_scroll = 6;
        handle_event(&mut app, key_press(KeyCode::Up));
        assert_eq!(app.log_scroll, 3);
    }

    #[tokio::test]
    async fn test_up_scroll_does_not_underflow() {
        let mut app = make_app();
        app.focus = Focus::Detail;
        app.tab = DetailTab::Logs;
        app.log_scroll = 0;
        handle_event(&mut app, key_press(KeyCode::Up));
        assert_eq!(app.log_scroll, 0);
    }

    #[tokio::test]
    async fn test_backspace_navigates_fs_up() {
        let mut app = make_app();
        app.focus = Focus::Detail;
        app.tab = DetailTab::Filesystem;
        app.fs_path = "/usr/local".into();
        handle_event(&mut app, key_press(KeyCode::Backspace));
        assert_eq!(app.fs_path, "/usr");
    }

    #[tokio::test]
    async fn test_backspace_at_root_stays_at_root() {
        let mut app = make_app();
        app.focus = Focus::Detail;
        app.tab = DetailTab::Filesystem;
        app.fs_path = "/".into();
        handle_event(&mut app, key_press(KeyCode::Backspace));
        // parent of "/" is "" which becomes "/"
        assert_eq!(app.fs_path, "/");
    }

    // ── r refresh key ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_r_shows_refreshing_notification() {
        let mut app = make_app();
        handle_event(&mut app, key_press(KeyCode::Char('r')));
        let n = app.notification.as_ref().unwrap();
        assert!(!n.is_error);
        assert!(n.message.contains("efresh"));
    }
}
