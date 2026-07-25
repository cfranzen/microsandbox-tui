//! Application state and event loop.

use std::time::{Duration, Instant};

use anyhow::Result;

use crossterm::event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use futures::StreamExt;
use microsandbox::sandbox::LogEntry;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DialogTab {
    #[default]
    Basic,
    Advanced,
}

impl DialogTab {
    pub fn next(self) -> Self {
        match self {
            DialogTab::Basic => DialogTab::Advanced,
            DialogTab::Advanced => DialogTab::Basic,
        }
    }

    pub fn prev(self) -> Self {
        // With only two tabs, prev == next
        self.next()
    }
}

/// Sentinel entry that opens the drive-selection view.
pub const DRIVES_ENTRY: &str = "⊞ [Switch Drive]";

/// State of the inline directory picker used to select a workdir.
#[derive(Debug, Clone, Default)]
pub struct DirPicker {
    pub visible: bool,
    /// Absolute path currently being listed. Empty string means "drives view".
    pub path: String,
    /// Subdirectory entries (first is always `DRIVES_ENTRY`, second `".."`).
    pub entries: Vec<String>,
    pub selected: usize,
    pub scroll_offset: usize,
    /// True when the picker is showing the list of available drives/roots.
    pub showing_drives: bool,
}

impl DirPicker {
    /// Open the picker starting at `initial_path`.
    /// Falls back to the first available drive root on Windows, or `/` on Unix.
    pub fn open(initial_path: &str) -> Self {
        let path = if std::path::Path::new(initial_path).is_dir() {
            initial_path.to_owned()
        } else {
            default_root()
        };
        let entries = load_dir_entries(&path);
        Self { visible: true, path, entries, selected: 0, scroll_offset: 0, showing_drives: false }
    }

    /// Navigate to `new_path` and refresh the entry list.
    pub fn navigate_to(&mut self, new_path: String) {
        self.entries = load_dir_entries(&new_path);
        self.path = new_path;
        self.selected = 0;
        self.scroll_offset = 0;
        self.showing_drives = false;
    }

    /// Switch to the drive-selection view.
    pub fn show_drives(&mut self) {
        let drives = list_drives();
        self.entries = drives;
        self.selected = 0;
        self.scroll_offset = 0;
        self.showing_drives = true;
    }
}

/// Returns the default starting root: first available drive on Windows, `/` elsewhere.
fn default_root() -> String {
    let drives = list_drives();
    drives.into_iter().next().unwrap_or_else(|| "/".into())
}

/// Returns all available filesystem roots (drive letters on Windows, `/` on Unix/macOS).
pub fn list_drives() -> Vec<String> {
    #[cfg(target_os = "windows")]
    {
        (b'A'..=b'Z')
            .filter_map(|letter| {
                let path = format!("{}:\\", letter as char);
                if std::path::Path::new(&path).exists() {
                    Some(path)
                } else {
                    None
                }
            })
            .collect()
    }
    #[cfg(not(target_os = "windows"))]
    {
        // On Unix, expose `/` as the only root; if MSYS2/Git Bash mounts are
        // present they'll be visible as subdirectories of `/`.
        vec!["/".to_owned()]
    }
}

/// Read the immediate subdirectories of `path`, sorted alphabetically.
/// The list always begins with `DRIVES_ENTRY` and `".."`.
pub fn load_dir_entries(path: &str) -> Vec<String> {
    let mut entries = vec![DRIVES_ENTRY.to_owned(), "..".to_owned()];
    if let Ok(read_dir) = std::fs::read_dir(path) {
        let mut subdirs: Vec<String> = read_dir
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with('.'))
            .collect();
        subdirs.sort();
        entries.extend(subdirs);
    }
    entries
}

/// State of the "create new sandbox" modal dialog.
#[derive(Debug, Clone, Default)]
pub struct CreateDialog {
    pub visible: bool,
    pub tab: DialogTab,
    pub field: usize,
    // Basic tab (fields 0-6)
    pub name: String,
    pub image: String,
    pub cpus: String,
    pub memory: String,
    pub ports: String,
    pub env_vars: String,
    pub workdir: String,
    // Advanced tab (fields 0-5)
    pub hostname: String,
    pub user: String,
    pub shell: String,
    pub max_cpus: String,
    pub max_memory: String,
    pub disable_network: bool,
    pub error: Option<String>,
    /// Inline directory browser for picking the workdir path.
    pub dir_picker: DirPicker,
    /// Absorb the next ESC event in the dialog (set when the picker was just
    /// closed by ESC so the release doesn't also close the dialog).
    pub absorb_esc: bool,
}

impl CreateDialog {
    pub fn open() -> Self {
        Self {
            visible: true,
            image: "alpine".into(),
            cpus: "1".into(),
            memory: "512".into(),
            shell: "/bin/sh".into(),
            ..Default::default()
        }
    }

    pub fn field_count(&self) -> usize {
        match self.tab {
            DialogTab::Basic => 7,    // name image cpus memory ports env_vars workdir
            DialogTab::Advanced => 6, // hostname user shell max_cpus max_memory no_net
        }
    }

    pub fn next_field(&mut self) {
        self.field = (self.field + 1) % self.field_count();
    }

    pub fn prev_field(&mut self) {
        let count = self.field_count();
        self.field = if self.field == 0 {
            count - 1
        } else {
            self.field - 1
        };
    }

    pub fn switch_tab(&mut self, tab: DialogTab) {
        self.tab = tab;
        self.field = 0;
        self.error = None;
    }

    /// Returns a mutable reference to the text value of the focused field,
    /// or `None` when the focused field is a non-text widget (e.g. a toggle).
    pub fn current_field_mut(&mut self) -> Option<&mut String> {
        match self.tab {
            DialogTab::Basic => match self.field {
                0 => Some(&mut self.name),
                1 => Some(&mut self.image),
                2 => Some(&mut self.cpus),
                3 => Some(&mut self.memory),
                4 => Some(&mut self.ports),
                5 => Some(&mut self.env_vars),
                6 => Some(&mut self.workdir),
                _ => None,
            },
            DialogTab::Advanced => match self.field {
                0 => Some(&mut self.hostname),
                1 => Some(&mut self.user),
                2 => Some(&mut self.shell),
                3 => Some(&mut self.max_cpus),
                4 => Some(&mut self.max_memory),
                5 => None, // disable_network toggle
                _ => None,
            },
        }
    }

    /// True when the focused field only accepts ASCII digits.
    pub fn is_numeric_field(&self) -> bool {
        match self.tab {
            DialogTab::Basic => matches!(self.field, 2 | 3),
            DialogTab::Advanced => matches!(self.field, 3 | 4),
        }
    }

    /// True when the focused field is a boolean toggle activated by Space.
    pub fn is_toggle_field(&self) -> bool {
        self.tab == DialogTab::Advanced && self.field == 5
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

    // Only act on key presses or repeats; skip releases from enhanced terminals.
    // Exception: Esc is also accepted on release so that terminals which report
    // it only as a release event (kitty keyboard protocol, etc.) still close the
    // dialog reliably.
    let is_esc = key.code == KeyCode::Esc;
    if key.kind == KeyEventKind::Release && !is_esc {
        return;
    }

    // Modal dialog steals all input; the dir picker overlays the dialog.
    if app.create_dialog.visible {
        if app.create_dialog.dir_picker.visible {
            handle_picker_key(app, key.code, key.modifiers);
        } else {
            handle_dialog_key(app, key.code, key.modifiers);
        }
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

fn handle_dialog_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    match code {
        KeyCode::Esc => {
            // A preceding ESC that closed the dir picker sets absorb_esc so that
            // the ESC Release event (sent by standard terminals after the Press)
            // doesn't also close the create dialog.
            if app.create_dialog.absorb_esc {
                app.create_dialog.absorb_esc = false;
                return;
            }
            app.create_dialog = Default::default();
        }
        KeyCode::Tab | KeyCode::Down => app.create_dialog.next_field(),
        KeyCode::BackTab | KeyCode::Up => app.create_dialog.prev_field(),
        KeyCode::Left => {
            let prev = app.create_dialog.tab.prev();
            app.create_dialog.switch_tab(prev);
        }
        KeyCode::Right => {
            let next = app.create_dialog.tab.next();
            app.create_dialog.switch_tab(next);
        }
        KeyCode::Char(' ') if app.create_dialog.is_toggle_field() => {
            app.create_dialog.disable_network = !app.create_dialog.disable_network;
            app.create_dialog.error = None;
        }
        KeyCode::Enter => {
            submit_create_dialog(app);
        }
        KeyCode::Backspace => {
            if let Some(field) = app.create_dialog.current_field_mut() {
                field.pop();
            }
            app.create_dialog.error = None;
        }
        // Ctrl+F on the workdir field opens the directory picker.
        KeyCode::Char('f') | KeyCode::Char('F')
            if mods.contains(KeyModifiers::CONTROL)
                && app.create_dialog.tab == DialogTab::Basic
                && app.create_dialog.field == 6 =>
        {
            let initial = app.create_dialog.workdir.trim().to_owned();
            let start = if initial.is_empty() { "/" } else { initial.as_str() };
            app.create_dialog.dir_picker = DirPicker::open(start);
        }
        KeyCode::Char(c) => {
            if app.create_dialog.is_toggle_field() {
                return;
            }
            if app.create_dialog.is_numeric_field() && !c.is_ascii_digit() {
                app.create_dialog.error = Some("Only digits allowed here".into());
                return;
            }
            app.create_dialog.error = None;
            if let Some(field) = app.create_dialog.current_field_mut() {
                field.push(c);
            }
        }
        _ => {}
    }
}

/// The number of picker entries that fit in the visible list area.
pub const PICKER_VISIBLE_ROWS: usize = 10;

fn handle_picker_key(app: &mut App, code: KeyCode, _mods: KeyModifiers) {
    // Handle ESC before taking a borrow on dir_picker so we can also set
    // absorb_esc on the parent dialog to prevent the ESC Release event
    // (which standard terminals send after the Press) from also closing
    // the create dialog.
    if code == KeyCode::Esc {
        app.create_dialog.dir_picker.visible = false;
        app.create_dialog.absorb_esc = true;
        return;
    }

    let picker = &mut app.create_dialog.dir_picker;
    match code {
        KeyCode::Up => {
            if picker.selected > 0 {
                picker.selected -= 1;
                if picker.selected < picker.scroll_offset {
                    picker.scroll_offset = picker.selected;
                }
            }
        }
        KeyCode::Down => {
            if picker.selected + 1 < picker.entries.len() {
                picker.selected += 1;
                if picker.selected >= picker.scroll_offset + PICKER_VISIBLE_ROWS {
                    picker.scroll_offset = picker.selected + 1 - PICKER_VISIBLE_ROWS;
                }
            }
        }
        KeyCode::Enter => {
            let entry = picker.entries.get(picker.selected).cloned().unwrap_or_default();
            if entry == DRIVES_ENTRY {
                picker.show_drives();
            } else if picker.showing_drives {
                // Entry is a drive root — navigate into it.
                picker.navigate_to(entry);
            } else if entry == ".." {
                let parent = std::path::Path::new(&picker.path)
                    .parent()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|| picker.path.clone());
                if parent == picker.path {
                    // Already at a drive root — go to drive selection.
                    picker.show_drives();
                } else {
                    picker.navigate_to(parent);
                }
            } else {
                // Use Path::join so separators are correct on both Windows and Unix.
                let new_path = std::path::Path::new(&picker.path)
                    .join(&entry)
                    .to_string_lossy()
                    .into_owned();
                if std::path::Path::new(&new_path).is_dir() {
                    picker.navigate_to(new_path);
                }
            }
        }
        KeyCode::Char(' ') => {
            // Space confirms the current directory without descending.
            if !picker.showing_drives {
                let chosen = picker.path.clone();
                app.create_dialog.workdir = chosen;
            }
            app.create_dialog.dir_picker.visible = false;
        }
        // Jump to filesystem root / drive list.
        KeyCode::Char('/') => {
            picker.show_drives();
        }
        // Jump to home directory.
        KeyCode::Char('~') => {
            if let Some(home) = std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(|v| v.to_string_lossy().into_owned())
            {
                picker.navigate_to(home);
            }
        }
        _ => {}
    }
}

fn submit_create_dialog(app: &mut App) {
    let dlg = &app.create_dialog;

    let name = dlg.name.trim().to_owned();
    if name.is_empty() {
        app.create_dialog.error = Some("Name is required".into());
        return;
    }

    let image = dlg.image.trim().to_owned();
    if image.is_empty() {
        app.create_dialog.error = Some("Image is required".into());
        return;
    }

    let cpus: u8 = match dlg.cpus.trim().parse::<u8>() {
        Ok(v) if (1..=32).contains(&v) => v,
        Ok(_) => {
            app.create_dialog.error = Some("CPUs must be between 1 and 32".into());
            return;
        }
        Err(_) => {
            app.create_dialog.error = Some("CPUs must be a number (1–32)".into());
            return;
        }
    };

    let memory: u32 = match dlg.memory.trim().parse::<u32>() {
        Ok(v) if v >= 64 => v,
        Ok(_) => {
            app.create_dialog.error = Some("Memory must be at least 64 MiB".into());
            return;
        }
        Err(_) => {
            app.create_dialog.error = Some("Memory must be a number in MiB (min 64)".into());
            return;
        }
    };

    let mut ports: Vec<(u16, u16)> = Vec::new();
    for token in dlg
        .ports
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
    {
        match parse_port_mapping(token) {
            Ok(p) => ports.push(p),
            Err(e) => {
                app.create_dialog.error = Some(format!("Port '{token}': {e}"));
                return;
            }
        }
    }

    let mut env_vars: Vec<(String, String)> = Vec::new();
    for token in dlg
        .env_vars
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
    {
        match parse_env_var(token) {
            Ok(e) => env_vars.push(e),
            Err(e) => {
                app.create_dialog.error = Some(format!("Env '{token}': {e}"));
                return;
            }
        }
    }

    let hostname = dlg.hostname.trim().to_owned();
    let hostname = if hostname.is_empty() {
        None
    } else {
        Some(hostname)
    };

    let workdir = dlg.workdir.trim().to_owned();
    let workdir = if workdir.is_empty() {
        None
    } else {
        Some(workdir)
    };

    let user = dlg.user.trim().to_owned();
    let user = if user.is_empty() { None } else { Some(user) };

    let shell_val = dlg.shell.trim().to_owned();
    let shell = if shell_val.is_empty() || shell_val == "/bin/sh" {
        None
    } else {
        Some(shell_val)
    };

    let max_cpus: Option<u8> = if dlg.max_cpus.trim().is_empty() {
        None
    } else {
        match dlg.max_cpus.trim().parse::<u8>() {
            Ok(v) if (1..=32).contains(&v) => Some(v),
            _ => {
                app.create_dialog.error = Some("Max CPUs must be between 1 and 32".into());
                return;
            }
        }
    };

    let max_memory: Option<u32> = if dlg.max_memory.trim().is_empty() {
        None
    } else {
        match dlg.max_memory.trim().parse::<u32>() {
            Ok(v) if v >= 64 => Some(v),
            _ => {
                app.create_dialog.error = Some("Max Memory must be at least 64 MiB".into());
                return;
            }
        }
    };

    let disable_network = dlg.disable_network;

    app.create_dialog = Default::default();

    let tx = app.msg_tx.clone();
    tokio::spawn(async move {
        let cfg = crate::sandbox::CreateConfig {
            name: name.clone(),
            image,
            cpus,
            memory_mib: memory,
            ports,
            env_vars,
            hostname,
            workdir,
            user,
            shell,
            max_cpus,
            max_memory_mib: max_memory,
            disable_network,
        };
        let result = crate::sandbox::create_sandbox(&cfg).await;
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

fn parse_port_mapping(s: &str) -> std::result::Result<(u16, u16), String> {
    let parts: Vec<&str> = s.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err("expected host:guest format".into());
    }

    let host = parts[0]
        .parse::<u16>()
        .map_err(|_| "invalid host port".to_owned())?;
    let guest = parts[1]
        .parse::<u16>()
        .map_err(|_| "invalid guest port".to_owned())?;
    Ok((host, guest))
}

fn parse_env_var(s: &str) -> std::result::Result<(String, String), String> {
    match s.find('=') {
        Some(pos) if pos > 0 => Ok((s[..pos].to_owned(), s[pos + 1..].to_owned())),
        Some(_) => Err("key cannot be empty".into()),
        None => Err("expected KEY=VALUE format".into()),
    }
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
        let parent_str = if parent_str.is_empty() {
            "/".into()
        } else {
            parent_str
        };
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
        assert_eq!(dlg.tab, DialogTab::Basic);
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
        assert_eq!(dlg.shell, "/bin/sh");
        assert!(dlg.name.is_empty());
        assert!(dlg.error.is_none());
    }

    #[test]
    fn test_create_dialog_next_field_wraps() {
        let mut dlg = CreateDialog::open();
        dlg.next_field();
        assert_eq!(dlg.field, 1);
        dlg.next_field();
        assert_eq!(dlg.field, 2);
        dlg.next_field();
        assert_eq!(dlg.field, 3);
        dlg.next_field();
        assert_eq!(dlg.field, 4);
        dlg.next_field();
        assert_eq!(dlg.field, 5);
        dlg.next_field();
        assert_eq!(dlg.field, 6);
        dlg.next_field();
        assert_eq!(dlg.field, 0);
    }

    #[test]
    fn test_create_dialog_current_field_mut() {
        let mut dlg = CreateDialog::open();
        dlg.field = 0;
        dlg.current_field_mut().unwrap().push_str("mybox");
        assert_eq!(dlg.name, "mybox");
        dlg.field = 1;
        dlg.current_field_mut().unwrap().push_str("ubuntu");
        assert_eq!(dlg.image, "alpineubuntu");
        dlg.field = 2;
        dlg.current_field_mut().unwrap().push_str("4");
        assert_eq!(dlg.cpus, "14");
        dlg.field = 3;
        dlg.current_field_mut().unwrap().push_str("1024");
        assert_eq!(dlg.memory, "5121024");
        dlg.field = 5;
        dlg.current_field_mut().unwrap().push_str("FOO=bar");
        assert_eq!(dlg.env_vars, "FOO=bar");
        dlg.field = 6;
        dlg.current_field_mut().unwrap().push_str("/workspace");
        assert_eq!(dlg.workdir, "/workspace");
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
        app.handle_message(AppMessage::SandboxList(Err(anyhow::anyhow!(
            "conn refused"
        ))));
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
        app.handle_message(AppMessage::LogEntries(
            "mybox".into(),
            Err(anyhow::anyhow!("err")),
        ));
        assert!(app.notification.as_ref().unwrap().is_error);
    }

    #[test]
    fn test_handle_message_metrics_stored() {
        let mut app = make_app();
        let m = MetricsSnapshot {
            cpu_percent: 42.0,
            memory_bytes: 1024,
            ..Default::default()
        };
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
        app.handle_message(AppMessage::Metrics(
            "mybox".into(),
            Err(anyhow::anyhow!("x")),
        ));
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
        app.handle_message(AppMessage::FsEntries(
            "mybox".into(),
            "/".into(),
            Ok(Some(entries)),
        ));
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
        app.handle_message(AppMessage::FsEntries(
            "mybox".into(),
            "/".into(),
            Err(anyhow::anyhow!("x")),
        ));
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
        handle_event(
            &mut app,
            key_press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL),
        );
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
        assert_eq!(app.create_dialog.field, 6);
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

    #[test]
    fn test_dialog_left_right_switches_tab() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.field = 3;
        handle_event(&mut app, key_press(KeyCode::Right));
        assert_eq!(app.create_dialog.tab, DialogTab::Advanced);
        assert_eq!(app.create_dialog.field, 0);

        app.create_dialog.field = 4;
        handle_event(&mut app, key_press(KeyCode::Left));
        assert_eq!(app.create_dialog.tab, DialogTab::Basic);
        assert_eq!(app.create_dialog.field, 0);
    }

    #[test]
    fn test_dialog_space_toggles_disable_network() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.switch_tab(DialogTab::Advanced);
        app.create_dialog.field = 5;
        handle_event(&mut app, key_press(KeyCode::Char(' ')));
        assert!(app.create_dialog.disable_network);
    }

    #[test]
    fn test_dialog_ports_text_field() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.field = 4;
        for ch in "8080:80".chars() {
            handle_event(&mut app, key_press(KeyCode::Char(ch)));
        }
        assert_eq!(app.create_dialog.ports, "8080:80");
    }

    #[test]
    fn test_dialog_env_vars_text_field() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.field = 5;
        for ch in "FOO=bar".chars() {
            handle_event(&mut app, key_press(KeyCode::Char(ch)));
        }
        assert_eq!(app.create_dialog.env_vars, "FOO=bar");
    }

    #[test]
    fn test_dialog_numeric_filter_on_advanced_max_cpus() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.switch_tab(DialogTab::Advanced);
        app.create_dialog.field = 3; // max_cpus
        handle_event(&mut app, key_press(KeyCode::Char('x')));
        assert!(app.create_dialog.error.is_some());
        assert!(app.create_dialog.max_cpus.is_empty());
    }

    #[test]
    fn test_dialog_toggle_field_ignores_char_input() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.switch_tab(DialogTab::Advanced);
        app.create_dialog.field = 5;
        handle_event(&mut app, key_press(KeyCode::Char('x')));
        assert!(!app.create_dialog.disable_network);
        assert!(app.create_dialog.error.is_none());
    }

    #[tokio::test]
    async fn test_submit_parses_ports_correctly() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.name = "mybox".into();
        app.create_dialog.ports = "8080:80 443:443".into();
        handle_event(&mut app, key_press(KeyCode::Enter));
        assert!(!app.create_dialog.visible);
    }

    #[test]
    fn test_submit_rejects_invalid_port_format() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.name = "mybox".into();
        app.create_dialog.ports = "invalid".into();
        handle_event(&mut app, key_press(KeyCode::Enter));
        assert!(app.create_dialog.visible);
        assert!(app
            .create_dialog
            .error
            .as_deref()
            .unwrap_or("")
            .contains("Port 'invalid'"));
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

    // ── dialog: ESC on key-release closes the dialog (kitty keyboard fix) ───

    #[test]
    fn test_esc_release_closes_dialog() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        // Simulate a terminal that emits ESC on key-release rather than press.
        handle_event(&mut app, key_release(KeyCode::Esc));
        assert!(
            !app.create_dialog.visible,
            "Esc release must close the dialog"
        );
    }

    #[test]
    fn test_other_release_events_still_ignored_when_dialog_open() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        // Releasing 'a' must not append to the name field.
        handle_event(&mut app, key_release(KeyCode::Char('a')));
        assert_eq!(
            app.create_dialog.name, "",
            "key-release must not modify dialog fields"
        );
    }

    // ── dialog: Up/Down arrow navigation between fields ──────────────────────

    #[test]
    fn test_dialog_down_advances_field() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        assert_eq!(app.create_dialog.field, 0);
        handle_event(&mut app, key_press(KeyCode::Down));
        assert_eq!(app.create_dialog.field, 1);
    }

    #[test]
    fn test_dialog_down_wraps_from_last_to_first() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.field = 6;
        handle_event(&mut app, key_press(KeyCode::Down));
        assert_eq!(app.create_dialog.field, 0);
    }

    #[test]
    fn test_dialog_up_goes_to_previous_field() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.field = 2;
        handle_event(&mut app, key_press(KeyCode::Up));
        assert_eq!(app.create_dialog.field, 1);
    }

    #[test]
    fn test_dialog_up_wraps_from_first_to_last() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.field = 0;
        handle_event(&mut app, key_press(KeyCode::Up));
        assert_eq!(app.create_dialog.field, 6);
    }

    // ── dialog: digit-only filtering for CPUs and Memory ────────────────────

    #[test]
    fn test_dialog_cpus_accepts_digits() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.field = 2; // CPUs
        app.create_dialog.cpus.clear();
        handle_event(&mut app, key_press(KeyCode::Char('4')));
        assert_eq!(app.create_dialog.cpus, "4");
        assert!(app.create_dialog.error.is_none());
    }

    #[test]
    fn test_dialog_cpus_rejects_letters() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.field = 2; // CPUs
        handle_event(&mut app, key_press(KeyCode::Char('x')));
        // Field must not change
        assert_eq!(app.create_dialog.cpus, "1");
        // An inline error must be shown
        assert!(app.create_dialog.error.is_some());
    }

    #[test]
    fn test_dialog_memory_accepts_digits() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.field = 3; // Memory
        app.create_dialog.memory.clear();
        handle_event(&mut app, key_press(KeyCode::Char('2')));
        assert_eq!(app.create_dialog.memory, "2");
        assert!(app.create_dialog.error.is_none());
    }

    #[test]
    fn test_dialog_memory_rejects_letters() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.field = 3; // Memory
        handle_event(&mut app, key_press(KeyCode::Char('m')));
        assert_eq!(app.create_dialog.memory, "512");
        assert!(app.create_dialog.error.is_some());
    }

    #[test]
    fn test_dialog_backspace_clears_error() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.field = 2; // CPUs
                                     // Trigger an error first
        handle_event(&mut app, key_press(KeyCode::Char('x')));
        assert!(app.create_dialog.error.is_some());
        // Backspace must clear it
        handle_event(&mut app, key_press(KeyCode::Backspace));
        assert!(app.create_dialog.error.is_none());
    }

    // ── dialog: submit validation for CPUs and Memory ────────────────────────

    #[test]
    fn test_dialog_submit_zero_cpus_sets_error() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.name = "mybox".into();
        app.create_dialog.cpus = "0".into();
        handle_event(&mut app, key_press(KeyCode::Enter));
        assert!(
            app.create_dialog.visible,
            "dialog stays open on invalid input"
        );
        let err = app.create_dialog.error.as_deref().unwrap_or("");
        assert!(
            err.contains("1") && err.contains("32"),
            "error must state valid range"
        );
    }

    #[test]
    fn test_dialog_submit_cpus_above_max_sets_error() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.name = "mybox".into();
        app.create_dialog.cpus = "33".into();
        handle_event(&mut app, key_press(KeyCode::Enter));
        assert!(app.create_dialog.visible);
        assert!(app.create_dialog.error.is_some());
    }

    #[test]
    fn test_dialog_submit_memory_below_min_sets_error() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.name = "mybox".into();
        app.create_dialog.memory = "32".into();
        handle_event(&mut app, key_press(KeyCode::Enter));
        assert!(app.create_dialog.visible);
        let err = app.create_dialog.error.as_deref().unwrap_or("");
        assert!(err.contains("64"), "error must mention minimum memory");
    }
}
