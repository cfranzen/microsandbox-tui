//! Application state and event loop.

use std::time::{Duration, Instant};

use anyhow::Result;

use crossterm::event::{Event, EventStream, KeyCode, KeyModifiers};
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

fn handle_event(app: &mut App, event: Event) {
    let Event::Key(key) = event else { return };

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
