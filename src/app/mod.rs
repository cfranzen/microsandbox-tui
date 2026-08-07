//! Application state and event loop.
//!
//! This module is intentionally kept small: it owns the central [`App`]
//! state struct, its core (non-input, non-dialog-specific) methods, and the
//! top-level `run` loop. Related concerns are split into sibling modules:
//!
//! - [`dialogs`] — state for the create-sandbox dialog and its sub-dialogs,
//!   the directory picker, and the volumes view.
//! - [`messages`] — [`AppMessage`] and the logic that applies background
//!   messages to app state.
//! - [`actions`] — sandbox actions, pending-confirmation handling, and the
//!   other non-key-dispatch helpers used while responding to input.
//! - [`keys`] — translates terminal key/mouse events into state changes.

mod actions;
mod dialogs;
mod keys;
mod messages;

#[cfg(test)]
mod tests;

pub use actions::{PendingAction, SandboxAction};
pub use dialogs::{
    CreateDialog, DialogTab, DirPicker, EnvVarsDialog, ExecDialog, MountKindChoice, MountsDialog,
    NetworkRulesDialog, PortsDialog, SubDialogMode, VolumesView, DRIVES_ENTRY,
};
pub(crate) use keys::handle_event;
pub use messages::AppMessage;

use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::EventStream;
use futures::StreamExt;
use microsandbox::sandbox::LogEntry;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;
use tokio::sync::mpsc;

use crate::config::AppConfig;
use crate::sandbox::{FsEntry, MetricsSnapshot, SandboxInfo, SandboxStatus as Status};
use crate::theme::Theme;
use crate::ui;

use actions::sandbox_matches_filter;
pub use dialogs::PICKER_VISIBLE_ROWS;

//--------------------------------------------------------------------------------------------------
// Constants
//--------------------------------------------------------------------------------------------------

/// How often background tasks refresh sandbox list and metrics.
pub(crate) const REFRESH_INTERVAL: Duration = Duration::from_secs(3);
/// Maximum log lines kept in memory per sandbox.
pub(crate) const MAX_LOG_LINES: usize = 500;
/// Number of samples kept per sandbox for the metrics history sparklines.
pub(crate) const METRICS_HISTORY_LEN: usize = 60;

/// Which panel has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    SandboxList,
    Detail,
}

/// Tabs available in the detail panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailTab {
    Info,
    Metrics,
    Logs,
    Filesystem,
}

impl DetailTab {
    pub fn title(self) -> &'static str {
        match self {
            DetailTab::Info => "Info",
            DetailTab::Metrics => "Metrics",
            DetailTab::Logs => "Logs",
            DetailTab::Filesystem => "Filesystem",
        }
    }

    pub fn all() -> &'static [DetailTab] {
        &[
            DetailTab::Info,
            DetailTab::Metrics,
            DetailTab::Logs,
            DetailTab::Filesystem,
        ]
    }
}

/// Screen regions recorded during the most recent render, used to translate
/// mouse clicks/scrolls into the equivalent keyboard actions.
#[derive(Debug, Clone, Default)]
pub struct MouseRegions {
    /// Bounding rect of the sandbox list panel.
    pub list_area: Rect,
    /// Bounding rect of the detail panel.
    pub detail_area: Rect,
    /// Rects for each rendered sandbox card, in display order. `None` marks
    /// the "New Sandbox" placeholder card; `Some(i)` is an index into
    /// `App::sandboxes`.
    pub card_rects: Vec<(Rect, Option<usize>)>,
    /// Rects for each tab label in the detail panel's tab bar.
    pub tab_rects: Vec<(Rect, DetailTab)>,
}

/// A status notification shown briefly in the footer.
#[derive(Debug, Clone)]
pub struct Notification {
    pub message: String,
    pub is_error: bool,
    pub expires: Instant,
}

/// Central application state.
pub struct App {
    /// Full list of known sandboxes.
    pub sandboxes: Vec<SandboxInfo>,
    /// Index of the selected sandbox in the list.
    pub selected: usize,
    /// Active search/filter string (substring match on name, or `status:`
    /// tokens). Empty means no filter is applied.
    pub filter: String,
    /// True while the user is actively typing in the search/filter input.
    pub search_active: bool,
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
    /// Rolling history of recent metrics samples per sandbox, used to draw
    /// the CPU/memory sparklines. Capped at [`METRICS_HISTORY_LEN`] entries.
    pub metrics_history:
        std::collections::HashMap<String, std::collections::VecDeque<MetricsSnapshot>>,
    /// Cached filesystem listing keyed by (name, path).
    pub fs_entries: std::collections::HashMap<(String, String), Vec<FsEntry>>,
    /// Current filesystem path being browsed.
    pub fs_path: String,
    /// Create sandbox dialog state.
    pub create_dialog: CreateDialog,
    /// Named-volumes management view state.
    pub volumes_view: VolumesView,
    /// "Exec" dialog state: prompts for a command line, then opens a new
    /// host terminal running it inside the selected sandbox.
    pub exec_dialog: ExecDialog,
    /// Transient notification shown at the bottom.
    pub notification: Option<Notification>,
    /// A destructive action awaiting user confirmation, if any. While set,
    /// the confirmation dialog steals all keyboard/mouse input.
    pub confirm: Option<PendingAction>,
    /// True when the user has requested quit.
    pub should_quit: bool,
    /// Last full refresh timestamp.
    pub last_refresh: Option<Instant>,
    /// Channel for background tasks to push results.
    pub msg_tx: mpsc::UnboundedSender<AppMessage>,
    /// Handle of the currently running live log-stream task, if any.
    log_stream_task: Option<tokio::task::JoinHandle<()>>,
    /// Name of the sandbox the live log-stream task is currently following.
    log_stream_name: Option<String>,
    /// Default sandbox parameters loaded from the user's config file, used
    /// to prefill the "New Sandbox" dialog.
    pub config: AppConfig,
    /// Screen regions from the most recent render, for mouse hit-testing.
    pub mouse: MouseRegions,
    /// Active color/style palette. Every view reads colors and border
    /// styles from this instead of hardcoding them, so toggling it
    /// re-skins the whole app.
    pub theme: Theme,
}

//----------------------------------------------------------------------
// Methods
//----------------------------------------------------------------------

impl App {
    pub fn new(msg_tx: mpsc::UnboundedSender<AppMessage>) -> Self {
        Self {
            sandboxes: Vec::new(),
            selected: 0,
            filter: String::new(),
            search_active: false,
            focus: Focus::SandboxList,
            tab: DetailTab::Info,
            log_scroll: 0,
            fs_scroll: 0,
            logs: Default::default(),
            metrics: Default::default(),
            metrics_history: Default::default(),
            fs_entries: Default::default(),
            fs_path: "/".into(),
            create_dialog: Default::default(),
            volumes_view: Default::default(),
            exec_dialog: Default::default(),
            notification: None,
            confirm: None,
            should_quit: false,
            last_refresh: None,
            msg_tx,
            log_stream_task: None,
            log_stream_name: None,
            config: AppConfig::load(),
            mouse: MouseRegions::default(),
            theme: Theme::default(),
        }
    }

    /// Return the currently selected sandbox, if any.
    pub fn selected_sandbox(&self) -> Option<&SandboxInfo> {
        self.sandboxes.get(self.selected)
    }

    /// Switch between the dark and bright palettes.
    pub fn toggle_theme(&mut self) {
        self.theme = Theme::for_mode(self.theme.mode.toggled());
    }

    /// Push a notification that disappears after a few seconds.
    pub fn notify(&mut self, msg: impl Into<String>, is_error: bool) {
        self.notification = Some(Notification {
            message: msg.into(),
            is_error,
            expires: Instant::now() + Duration::from_secs(4),
        });
    }

    /// Return the indices into `sandboxes` that match the current filter.
    /// An empty filter matches every sandbox.
    pub fn visible_indices(&self) -> Vec<usize> {
        if self.filter.trim().is_empty() {
            return (0..self.sandboxes.len()).collect();
        }
        self.sandboxes
            .iter()
            .enumerate()
            .filter(|(_, sb)| sandbox_matches_filter(sb, &self.filter))
            .map(|(i, _)| i)
            .collect()
    }

    /// Select the next sandbox in the list.
    pub fn select_next(&mut self) {
        if self.filter.trim().is_empty() {
            if self.sandboxes.is_empty() {
                return;
            }
            let next = (self.selected + 1).min(self.sandboxes.len());
            // +1 for the "New Sandbox" entry at the bottom
            if next <= self.sandboxes.len() {
                self.selected = next;
            }
            return;
        }
        let visible = self.visible_indices();
        if visible.is_empty() {
            return;
        }
        match visible.iter().position(|&i| i == self.selected) {
            Some(pos) if pos + 1 < visible.len() => self.selected = visible[pos + 1],
            None => self.selected = visible[0],
            _ => {}
        }
    }

    /// Select the previous sandbox in the list.
    pub fn select_prev(&mut self) {
        if self.filter.trim().is_empty() {
            self.selected = self.selected.saturating_sub(1);
            return;
        }
        let visible = self.visible_indices();
        if visible.is_empty() {
            return;
        }
        match visible.iter().position(|&i| i == self.selected) {
            Some(pos) if pos > 0 => self.selected = visible[pos - 1],
            None => self.selected = visible[0],
            _ => {}
        }
    }

    /// Returns true if the "New Sandbox" placeholder is selected. The
    /// placeholder is hidden while a filter is active.
    pub fn new_sandbox_selected(&self) -> bool {
        self.filter.trim().is_empty() && self.selected == self.sandboxes.len()
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

    /// Compute which sandbox (if any) the live log stream should currently
    /// be following: the selected sandbox, but only while the Logs tab is
    /// active and the sandbox is running.
    fn log_stream_target(&self) -> Option<String> {
        if self.tab != DetailTab::Logs {
            return None;
        }
        self.selected_sandbox()
            .filter(|s| s.status == Status::Running)
            .map(|s| s.name.clone())
    }

    /// Start (or stop) the live log-stream background task so it follows
    /// whatever [`log_stream_target`](Self::log_stream_target) currently
    /// resolves to. Safe to call after every selection/tab/list change —
    /// it's a no-op when the target hasn't changed.
    pub fn sync_log_stream(&mut self) {
        // Guard against being invoked outside a Tokio runtime (e.g. plain
        // `#[test]` functions that exercise `handle_message` directly).
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        let target = self.log_stream_target();
        if target != self.log_stream_name {
            self.stop_log_stream();
            if let Some(name) = target {
                // Backfill via one-shot read, then start following new entries.
                self.request_logs(&name);
                self.start_log_stream(&name);
                return;
            }
        }
        // No live stream is active (tab isn't Logs, or the sandbox is
        // stopped) — fall back to a plain one-shot fetch so a stopped
        // sandbox's Logs tab still shows its captured history.
        if self.tab == DetailTab::Logs {
            if let Some(sb) = self.selected_sandbox() {
                if sb.status != Status::Running {
                    let name = sb.name.clone();
                    self.request_logs(&name);
                }
            }
        }
    }

    /// Spawn a background task that consumes the live log stream for
    /// `name` and forwards each entry to the main loop.
    fn start_log_stream(&mut self, name: &str) {
        let tx = self.msg_tx.clone();
        let name_owned = name.to_owned();
        let task_name = name_owned.clone();
        let handle = tokio::spawn(async move {
            if let Ok(Some(mut stream)) = crate::sandbox::open_log_stream(&task_name).await {
                while let Some(item) = stream.next().await {
                    match item {
                        Ok(entry) => {
                            let msg = AppMessage::LogStreamEntry(task_name.clone(), entry);
                            if tx.send(msg).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            }
        });
        self.log_stream_task = Some(handle);
        self.log_stream_name = Some(name_owned);
    }

    /// Cancel the currently running live log-stream task, if any.
    fn stop_log_stream(&mut self) {
        if let Some(handle) = self.log_stream_task.take() {
            handle.abort();
        }
        self.log_stream_name = None;
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

    /// Perform a sandbox action (start/stop/terminate/remove) in the background.
    pub fn run_action(&self, action: SandboxAction, name: &str) {
        let tx = self.msg_tx.clone();
        let name = name.to_owned();
        tokio::spawn(async move {
            let result: Result<()> = match action {
                SandboxAction::Start => crate::sandbox::start_sandbox(&name).await,
                SandboxAction::Stop => crate::sandbox::stop_sandbox(&name).await,
                SandboxAction::Terminate => crate::sandbox::terminate_sandbox(&name).await,
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
}

//----------------------------------------------------------------------
// Event loop
//----------------------------------------------------------------------

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
                        DetailTab::Logs => app.sync_log_stream(),
                        DetailTab::Filesystem => {
                            let path = app.fs_path.clone();
                            app.request_fs(&sb.name, &path);
                        }
                        DetailTab::Metrics | DetailTab::Info => app.request_metrics(&sb.name),
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
