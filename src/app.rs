//! Application state and event loop.

use std::time::{Duration, Instant};

use anyhow::Result;

use crossterm::event::{
    Event, EventStream, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use futures::StreamExt;
use microsandbox::sandbox::LogEntry;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;
use tokio::sync::mpsc;

use crate::config::AppConfig;
use crate::sandbox::{
    FsEntry, MetricsSnapshot, MountSource, NetRuleAction, NetRuleDirection, NetworkRule,
    SandboxInfo, SandboxStatus as Status, VolumeInfo, VolumeMountConfig,
};
use crate::ui;

//--------------------------------------------------------------------------------------------------
// Constants
//--------------------------------------------------------------------------------------------------

/// How often background tasks refresh sandbox list and metrics.
const REFRESH_INTERVAL: Duration = Duration::from_secs(3);
/// Maximum log lines kept in memory per sandbox.
const MAX_LOG_LINES: usize = 500;
/// Number of samples kept per sandbox for the metrics history sparklines.
const METRICS_HISTORY_LEN: usize = 60;

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
    Filesystem,
    Info,
}

impl DetailTab {
    pub fn title(self) -> &'static str {
        match self {
            DetailTab::Logs => "Logs",
            DetailTab::Filesystem => "Filesystem",
            DetailTab::Info => "Info",
        }
    }

    pub fn all() -> &'static [DetailTab] {
        &[DetailTab::Logs, DetailTab::Filesystem, DetailTab::Info]
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

/// Returns true if the point `(x, y)` falls within `rect`.
fn point_in_rect(x: u16, y: u16, rect: Rect) -> bool {
    rect.x <= x && x < rect.x + rect.width && rect.y <= y && y < rect.y + rect.height
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
        Self {
            visible: true,
            path,
            entries,
            selected: 0,
            scroll_offset: 0,
            showing_drives: false,
        }
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

/// Mode for the ports / env-vars sub-dialogs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubDialogMode {
    #[default]
    List,
    Add,
}

/// Sub-dialog for managing port mappings (host:guest).
#[derive(Debug, Clone, Default)]
pub struct PortsDialog {
    pub visible: bool,
    /// Confirmed mappings (host_port, guest_port).
    pub entries: Vec<(u16, u16)>,
    /// Selected entry index (List mode).
    pub selected: usize,
    pub mode: SubDialogMode,
    /// Host-port input buffer (Add mode).
    pub host_input: String,
    /// Guest-port input buffer (Add mode).
    pub guest_input: String,
    /// Focused input index in Add mode: 0 = host, 1 = guest.
    pub add_field: usize,
    pub error: Option<String>,
}

impl PortsDialog {
    pub fn open(entries: Vec<(u16, u16)>) -> Self {
        let selected = entries.len().saturating_sub(1);
        Self {
            visible: true,
            entries,
            selected,
            ..Default::default()
        }
    }
}

/// Sub-dialog for managing environment variables (KEY=VALUE).
#[derive(Debug, Clone, Default)]
pub struct EnvVarsDialog {
    pub visible: bool,
    /// Confirmed variables (key, value).
    pub entries: Vec<(String, String)>,
    /// Selected entry index (List mode).
    pub selected: usize,
    pub mode: SubDialogMode,
    /// Key input buffer (Add mode).
    pub key_input: String,
    /// Value input buffer (Add mode).
    pub value_input: String,
    /// Focused input index in Add mode: 0 = key, 1 = value.
    pub add_field: usize,
    pub error: Option<String>,
}

impl EnvVarsDialog {
    pub fn open(entries: Vec<(String, String)>) -> Self {
        let selected = entries.len().saturating_sub(1);
        Self {
            visible: true,
            entries,
            selected,
            ..Default::default()
        }
    }
}

/// Sub-dialog for managing CIDR-based network policy rules.
///
/// Network policy can only be configured at sandbox-creation time (the SDK's
/// `SandboxModificationBuilder` has no field for it), so this dialog is only
/// reachable from the create-sandbox dialog's Advanced tab.
#[derive(Debug, Clone, Default)]
pub struct NetworkRulesDialog {
    pub visible: bool,
    /// Confirmed rules.
    pub entries: Vec<NetworkRule>,
    /// Selected entry index (List mode).
    pub selected: usize,
    pub mode: SubDialogMode,
    /// CIDR input buffer (Add mode).
    pub cidr_input: String,
    /// Action for the rule being added.
    pub action: NetRuleAction,
    /// Direction for the rule being added.
    pub direction: NetRuleDirection,
    pub error: Option<String>,
}

impl NetworkRulesDialog {
    pub fn open(entries: Vec<NetworkRule>) -> Self {
        let selected = entries.len().saturating_sub(1);
        Self {
            visible: true,
            entries,
            selected,
            ..Default::default()
        }
    }
}

/// Which source kind is focused while adding a mount entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MountKindChoice {
    #[default]
    Bind,
    Named,
}

/// Sub-dialog for managing volume mounts on the create-sandbox dialog.
///
/// Mounts can only be configured at sandbox-creation time (the SDK's
/// `SandboxModificationBuilder` has no field for it), so this dialog is only
/// reachable from the create-sandbox dialog's Basic tab.
#[derive(Debug, Clone, Default)]
pub struct MountsDialog {
    pub visible: bool,
    /// Confirmed mounts.
    pub entries: Vec<VolumeMountConfig>,
    /// Selected entry index (List mode).
    pub selected: usize,
    pub mode: SubDialogMode,
    /// Guest path input buffer (Add mode).
    pub guest_input: String,
    /// Host path (Bind) or volume name (Named) input buffer (Add mode).
    pub source_input: String,
    /// Which mount source kind is being configured (Add mode).
    pub kind: MountKindChoice,
    /// Focused input index in Add mode: 0 = guest path, 1 = source.
    pub add_field: usize,
    pub error: Option<String>,
}

impl MountsDialog {
    pub fn open(entries: Vec<VolumeMountConfig>) -> Self {
        let selected = entries.len().saturating_sub(1);
        Self {
            visible: true,
            entries,
            selected,
            ..Default::default()
        }
    }
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
    /// Port mappings (host, guest) managed via [`PortsDialog`].
    pub ports: Vec<(u16, u16)>,
    /// Environment variables (key, value) managed via [`EnvVarsDialog`].
    pub env_vars: Vec<(String, String)>,
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
    /// Sub-dialog for managing port mappings.
    pub ports_dialog: PortsDialog,
    /// Sub-dialog for managing environment variables.
    pub env_vars_dialog: EnvVarsDialog,
    /// CIDR-based network policy rules, applied at creation time only.
    pub network_rules: Vec<NetworkRule>,
    /// Sub-dialog for managing network policy rules.
    pub network_rules_dialog: NetworkRulesDialog,
    /// Volume mounts, applied at creation time only.
    pub mounts: Vec<VolumeMountConfig>,
    /// Sub-dialog for managing volume mounts.
    pub mounts_dialog: MountsDialog,
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

    /// Open the dialog, prefilling fields from the user's config file where
    /// present, falling back to the built-in defaults from [`Self::open`]
    /// for any field the config file doesn't specify.
    pub fn open_with_config(cfg: &AppConfig) -> Self {
        let mut dlg = Self::open();
        if let Some(v) = &cfg.image {
            dlg.image = v.clone();
        }
        if let Some(v) = cfg.cpus {
            dlg.cpus = v.to_string();
        }
        if let Some(v) = cfg.memory_mib {
            dlg.memory = v.to_string();
        }
        if let Some(v) = &cfg.hostname {
            dlg.hostname = v.clone();
        }
        if let Some(v) = &cfg.workdir {
            dlg.workdir = v.clone();
        }
        if let Some(v) = &cfg.user {
            dlg.user = v.clone();
        }
        if let Some(v) = &cfg.shell {
            dlg.shell = v.clone();
        }
        dlg
    }

    pub fn form_field_count(&self) -> usize {
        match self.tab {
            DialogTab::Basic => 8, // name image cpus memory ports env_vars workdir mounts
            DialogTab::Advanced => 7, // hostname user shell max_cpus max_memory no_net net_rules
        }
    }

    /// Total navigable positions: form fields + the Create button.
    pub fn field_count(&self) -> usize {
        self.form_field_count() + 1
    }

    /// True when the Create button is focused (last navigable position).
    pub fn is_create_focused(&self) -> bool {
        self.field == self.form_field_count()
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
    /// or `None` when the focused field is a non-text widget (toggle or sub-dialog).
    pub fn current_field_mut(&mut self) -> Option<&mut String> {
        match self.tab {
            DialogTab::Basic => match self.field {
                0 => Some(&mut self.name),
                1 => Some(&mut self.image),
                2 => Some(&mut self.cpus),
                3 => Some(&mut self.memory),
                4 => None, // ports — managed via sub-dialog
                5 => None, // env_vars — managed via sub-dialog
                6 => None, // workdir — managed via dir picker
                _ => None,
            },
            DialogTab::Advanced => match self.field {
                0 => Some(&mut self.hostname),
                1 => Some(&mut self.user),
                2 => Some(&mut self.shell),
                3 => Some(&mut self.max_cpus),
                4 => Some(&mut self.max_memory),
                5 => None, // disable_network toggle
                6 => None, // network rules — managed via sub-dialog
                _ => None,
            },
        }
    }

    /// True when the focused field only accepts ASCII digits.
    pub fn is_numeric_field(&self) -> bool {
        if self.is_create_focused() {
            return false;
        }
        match self.tab {
            DialogTab::Basic => matches!(self.field, 2 | 3),
            DialogTab::Advanced => matches!(self.field, 3 | 4),
        }
    }

    /// True when the focused field is a boolean toggle activated by Space.
    pub fn is_toggle_field(&self) -> bool {
        !self.is_create_focused() && self.tab == DialogTab::Advanced && self.field == 5
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
    /// A single log entry pushed live by a running log-stream task.
    LogStreamEntry(String, LogEntry),
    Metrics(String, Result<Option<MetricsSnapshot>>),
    FsEntries(String, String, Result<Option<Vec<FsEntry>>>),
    Notification(String, bool),
    /// Result of refreshing the named-volumes list.
    VolumeList(Result<Vec<VolumeInfo>>),
}

/// State of the top-level "Volumes" management view.
///
/// Volumes are managed directly against the SDK (not tied to any particular
/// sandbox), reached via the `v` key from the main view.
#[derive(Debug, Clone, Default)]
pub struct VolumesView {
    pub visible: bool,
    pub volumes: Vec<VolumeInfo>,
    pub selected: usize,
    pub mode: SubDialogMode,
    /// Name input buffer (Add mode).
    pub name_input: String,
    /// Whether the volume being created is disk-backed (vs. directory).
    pub disk: bool,
    pub error: Option<String>,
}

impl VolumesView {
    pub fn open() -> Self {
        Self {
            visible: true,
            ..Default::default()
        }
    }
}

/// Central application state.
pub struct App {
    /// Full list of known sandboxes.
    pub sandboxes: Vec<SandboxInfo>,
    /// Index of the selected sandbox in the list.
    pub selected: usize,
    /// Names of sandboxes marked for bulk operations. When non-empty, the
    /// start/stop/kill/remove keys apply to every marked sandbox instead of
    /// only the highlighted one.
    pub marked: std::collections::HashSet<String>,
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
    /// Transient notification shown at the bottom.
    pub notification: Option<Notification>,
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
}

//--------------------------------------------------------------------------------------------------
// Methods
//--------------------------------------------------------------------------------------------------

impl App {
    pub fn new(msg_tx: mpsc::UnboundedSender<AppMessage>) -> Self {
        Self {
            sandboxes: Vec::new(),
            selected: 0,
            marked: Default::default(),
            filter: String::new(),
            search_active: false,
            focus: Focus::SandboxList,
            tab: DetailTab::Logs,
            log_scroll: 0,
            fs_scroll: 0,
            logs: Default::default(),
            metrics: Default::default(),
            metrics_history: Default::default(),
            fs_entries: Default::default(),
            fs_path: "/".into(),
            create_dialog: Default::default(),
            volumes_view: Default::default(),
            notification: None,
            should_quit: false,
            last_refresh: None,
            msg_tx,
            log_stream_task: None,
            log_stream_name: None,
            config: AppConfig::load(),
            mouse: MouseRegions::default(),
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

    /// Perform a sandbox action against every sandbox in `names` concurrently,
    /// then report how many succeeded/failed in a single summary notification.
    pub fn run_bulk_action(&self, action: SandboxAction, names: Vec<String>) {
        let tx = self.msg_tx.clone();
        let total = names.len();
        tokio::spawn(async move {
            let futures = names.into_iter().map(|name| async move {
                match action {
                    SandboxAction::Start => crate::sandbox::start_sandbox(&name).await,
                    SandboxAction::Stop => crate::sandbox::stop_sandbox(&name).await,
                    SandboxAction::Kill => crate::sandbox::kill_sandbox(&name).await,
                    SandboxAction::Remove => crate::sandbox::remove_sandbox(&name).await,
                }
            });
            let results = futures::future::join_all(futures).await;
            let failed = results.iter().filter(|r| r.is_err()).count();
            let succeeded = total - failed;
            let msg = format!("{action:?}: {succeeded}/{total} succeeded");
            let _ = tx.send(AppMessage::Notification(msg, failed > 0));
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
                // Drop marks for sandboxes that no longer exist.
                let existing: std::collections::HashSet<&str> =
                    self.sandboxes.iter().map(|s| s.name.as_str()).collect();
                self.marked.retain(|name| existing.contains(name.as_str()));
                self.last_refresh = Some(Instant::now());
                self.sync_log_stream();
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
            AppMessage::LogStreamEntry(name, entry) => {
                let entries = self.logs.entry(name).or_default();
                entries.push(entry);
                if entries.len() > MAX_LOG_LINES {
                    let excess = entries.len() - MAX_LOG_LINES;
                    entries.drain(0..excess);
                }
            }
            AppMessage::Metrics(name, Ok(Some(m))) => {
                let history = self.metrics_history.entry(name.clone()).or_default();
                history.push_back(m.clone());
                if history.len() > METRICS_HISTORY_LEN {
                    history.pop_front();
                }
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
            AppMessage::VolumeList(Ok(list)) => {
                self.volumes_view.volumes = list;
                if self.volumes_view.selected >= self.volumes_view.volumes.len() {
                    self.volumes_view.selected = self.volumes_view.volumes.len().saturating_sub(1);
                }
            }
            AppMessage::VolumeList(Err(e)) => {
                self.notify(format!("Volume list error: {e}"), true);
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
                        DetailTab::Logs => app.sync_log_stream(),
                        DetailTab::Filesystem => {
                            let path = app.fs_path.clone();
                            app.request_fs(&sb.name, &path);
                        }
                        DetailTab::Info => app.request_metrics(&sb.name),
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
    let key = match event {
        Event::Key(key) => key,
        Event::Mouse(mouse) => {
            handle_mouse_event(app, mouse);
            return;
        }
        _ => return,
    };

    // Only act on key presses or repeats; ignore all release events, except
    // for Esc — some terminals (notably on Windows) only emit a release
    // event for the Esc key, so treating it like every other release would
    // make Esc silently do nothing on those terminals.
    // This ensures every physical keypress is handled exactly once,
    // regardless of how many events the terminal emits per keystroke.
    if key.kind == KeyEventKind::Release && key.code != KeyCode::Esc {
        return;
    }

    // Modal dialog steals all input; the dir picker overlays the dialog.
    if app.create_dialog.visible {
        if app.create_dialog.dir_picker.visible {
            handle_picker_key(app, key.code, key.modifiers);
        } else if app.create_dialog.ports_dialog.visible {
            handle_ports_dialog_key(app, key.code, key.modifiers);
        } else if app.create_dialog.env_vars_dialog.visible {
            handle_env_vars_dialog_key(app, key.code, key.modifiers);
        } else if app.create_dialog.network_rules_dialog.visible {
            handle_network_rules_dialog_key(app, key.code, key.modifiers);
        } else if app.create_dialog.mounts_dialog.visible {
            handle_mounts_dialog_key(app, key.code, key.modifiers);
        } else {
            handle_dialog_key(app, key.code, key.modifiers);
        }
        return;
    }

    // The Volumes view is a separate top-level modal.
    if app.volumes_view.visible {
        handle_volumes_view_key(app, key.code, key.modifiers);
        return;
    }

    // Search/filter input steals key input while active.
    if app.search_active {
        handle_search_key(app, key.code);
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
        KeyCode::Char('/') if app.focus == Focus::SandboxList => {
            app.search_active = true;
        }
        KeyCode::Char(' ') if app.focus == Focus::SandboxList => {
            toggle_mark(app);
        }
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
                app.create_dialog = CreateDialog::open_with_config(&app.config);
            } else if app.focus == Focus::SandboxList {
                app.focus = Focus::Detail;
            }
        }
        KeyCode::Char('n') => {
            app.create_dialog = CreateDialog::open_with_config(&app.config);
        }
        KeyCode::Char('v') => {
            app.volumes_view = VolumesView::open();
            request_volume_refresh(app);
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

/// Translate a mouse event into the equivalent list/detail-panel action.
/// Ignored while a modal dialog or the search box is active, to keep scope
/// limited to the main view (list selection, tab switching, scrolling).
fn handle_mouse_event(app: &mut App, mouse: MouseEvent) {
    if app.create_dialog.visible || app.volumes_view.visible || app.search_active {
        return;
    }

    let (x, y) = (mouse.column, mouse.row);
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some(tab) = tab_at(app, x, y) {
                app.tab = tab;
                app.focus = Focus::Detail;
                on_tab_switched(app);
            } else if let Some(target) = card_at(app, x, y) {
                app.selected = target.unwrap_or(app.sandboxes.len());
                app.focus = Focus::SandboxList;
                on_sandbox_selected(app);
            } else if point_in_rect(x, y, app.mouse.detail_area) {
                app.focus = Focus::Detail;
            } else if point_in_rect(x, y, app.mouse.list_area) {
                app.focus = Focus::SandboxList;
            }
        }
        MouseEventKind::ScrollUp => {
            if point_in_rect(x, y, app.mouse.list_area) {
                app.select_prev();
                on_sandbox_selected(app);
            } else if point_in_rect(x, y, app.mouse.detail_area) {
                scroll_up(app);
            }
        }
        MouseEventKind::ScrollDown => {
            if point_in_rect(x, y, app.mouse.list_area) {
                app.select_next();
                on_sandbox_selected(app);
            } else if point_in_rect(x, y, app.mouse.detail_area) {
                scroll_down(app);
            }
        }
        _ => {}
    }
}

/// Returns the detail tab whose rendered rect contains `(x, y)`, if any.
fn tab_at(app: &App, x: u16, y: u16) -> Option<DetailTab> {
    app.mouse
        .tab_rects
        .iter()
        .find(|(rect, _)| point_in_rect(x, y, *rect))
        .map(|(_, tab)| *tab)
}

/// Returns the sandbox card whose rendered rect contains `(x, y)`, if any.
/// `Some(None)` is the "New Sandbox" placeholder card.
fn card_at(app: &App, x: u16, y: u16) -> Option<Option<usize>> {
    app.mouse
        .card_rects
        .iter()
        .find(|(rect, _)| point_in_rect(x, y, *rect))
        .map(|(_, idx)| *idx)
}

fn handle_dialog_key(app: &mut App, code: KeyCode, _mods: KeyModifiers) {
    match code {
        KeyCode::Esc => {
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
            let dlg = &app.create_dialog;
            if dlg.is_create_focused() {
                submit_create_dialog(app);
            } else if dlg.tab == DialogTab::Basic && dlg.field == 4 {
                let entries = app.create_dialog.ports.clone();
                app.create_dialog.ports_dialog = PortsDialog::open(entries);
            } else if dlg.tab == DialogTab::Basic && dlg.field == 5 {
                let entries = app.create_dialog.env_vars.clone();
                app.create_dialog.env_vars_dialog = EnvVarsDialog::open(entries);
            } else if dlg.tab == DialogTab::Basic && dlg.field == 6 {
                let initial = app.create_dialog.workdir.trim().to_owned();
                let start = if initial.is_empty() { "/" } else { &initial };
                app.create_dialog.dir_picker = DirPicker::open(start);
            } else if dlg.tab == DialogTab::Advanced && dlg.field == 6 {
                let entries = app.create_dialog.network_rules.clone();
                app.create_dialog.network_rules_dialog = NetworkRulesDialog::open(entries);
            } else if dlg.tab == DialogTab::Basic && dlg.field == 7 {
                let entries = app.create_dialog.mounts.clone();
                app.create_dialog.mounts_dialog = MountsDialog::open(entries);
            }
            // Enter on plain text fields moves to next field.
            else {
                app.create_dialog.next_field();
            }
        }
        KeyCode::Backspace => {
            if let Some(field) = app.create_dialog.current_field_mut() {
                field.pop();
            }
            app.create_dialog.error = None;
        }
        KeyCode::Char(c) => {
            if app.create_dialog.is_toggle_field() || app.create_dialog.is_create_focused() {
                return;
            }
            // Managed fields (ports, env vars, workdir) don't accept direct text input.
            if app.create_dialog.tab == DialogTab::Basic && matches!(app.create_dialog.field, 4..=7)
            {
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
    if code == KeyCode::Esc {
        app.create_dialog.dir_picker.visible = false;
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
            let entry = picker
                .entries
                .get(picker.selected)
                .cloned()
                .unwrap_or_default();
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

fn handle_ports_dialog_key(app: &mut App, code: KeyCode, _mods: KeyModifiers) {
    match app.create_dialog.ports_dialog.mode {
        SubDialogMode::List => {
            match code {
                KeyCode::Esc | KeyCode::Enter => {
                    // Sync confirmed entries back to the parent field and close.
                    let entries = app.create_dialog.ports_dialog.entries.clone();
                    app.create_dialog.ports = entries;
                    app.create_dialog.ports_dialog.visible = false;
                }
                KeyCode::Up => {
                    if app.create_dialog.ports_dialog.selected > 0 {
                        app.create_dialog.ports_dialog.selected -= 1;
                    }
                }
                KeyCode::Down => {
                    let len = app.create_dialog.ports_dialog.entries.len();
                    if len > 0 && app.create_dialog.ports_dialog.selected + 1 < len {
                        app.create_dialog.ports_dialog.selected += 1;
                    }
                }
                KeyCode::Char('a') | KeyCode::Char('A') => {
                    app.create_dialog.ports_dialog.mode = SubDialogMode::Add;
                    app.create_dialog.ports_dialog.host_input.clear();
                    app.create_dialog.ports_dialog.guest_input.clear();
                    app.create_dialog.ports_dialog.add_field = 0;
                    app.create_dialog.ports_dialog.error = None;
                }
                KeyCode::Char('d') | KeyCode::Delete => {
                    let dialog = &mut app.create_dialog.ports_dialog;
                    if !dialog.entries.is_empty() {
                        dialog.entries.remove(dialog.selected);
                        if dialog.selected >= dialog.entries.len() && dialog.selected > 0 {
                            dialog.selected -= 1;
                        }
                        dialog.error = None;
                    }
                }
                _ => {}
            }
        }
        SubDialogMode::Add => match code {
            KeyCode::Esc => {
                app.create_dialog.ports_dialog.mode = SubDialogMode::List;
                app.create_dialog.ports_dialog.error = None;
            }
            KeyCode::Tab | KeyCode::Down => {
                let f = app.create_dialog.ports_dialog.add_field;
                app.create_dialog.ports_dialog.add_field = (f + 1) % 2;
            }
            KeyCode::BackTab | KeyCode::Up => {
                let f = app.create_dialog.ports_dialog.add_field;
                app.create_dialog.ports_dialog.add_field = (f + 1) % 2;
            }
            KeyCode::Backspace => {
                let dialog = &mut app.create_dialog.ports_dialog;
                if dialog.add_field == 0 {
                    dialog.host_input.pop();
                } else {
                    dialog.guest_input.pop();
                }
                dialog.error = None;
            }
            KeyCode::Enter => {
                let dialog = &mut app.create_dialog.ports_dialog;
                if dialog.add_field == 0 {
                    // Move focus to the guest port field.
                    dialog.add_field = 1;
                } else {
                    let host = dialog.host_input.trim().parse::<u16>();
                    let guest = dialog.guest_input.trim().parse::<u16>();
                    match (host, guest) {
                        (Ok(h), Ok(g)) => {
                            dialog.entries.push((h, g));
                            dialog.selected = dialog.entries.len().saturating_sub(1);
                            dialog.mode = SubDialogMode::List;
                            dialog.error = None;
                        }
                        (Err(_), _) => {
                            dialog.error = Some("Invalid host port (0–65535)".into());
                        }
                        (_, Err(_)) => {
                            dialog.error = Some("Invalid guest port (0–65535)".into());
                        }
                    }
                }
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                let dialog = &mut app.create_dialog.ports_dialog;
                if dialog.add_field == 0 {
                    dialog.host_input.push(c);
                } else {
                    dialog.guest_input.push(c);
                }
                dialog.error = None;
            }
            _ => {}
        },
    }
}

fn handle_env_vars_dialog_key(app: &mut App, code: KeyCode, _mods: KeyModifiers) {
    match app.create_dialog.env_vars_dialog.mode {
        SubDialogMode::List => {
            match code {
                KeyCode::Esc | KeyCode::Enter => {
                    // Sync confirmed entries back to the parent field and close.
                    let entries = app.create_dialog.env_vars_dialog.entries.clone();
                    app.create_dialog.env_vars = entries;
                    app.create_dialog.env_vars_dialog.visible = false;
                }
                KeyCode::Up => {
                    if app.create_dialog.env_vars_dialog.selected > 0 {
                        app.create_dialog.env_vars_dialog.selected -= 1;
                    }
                }
                KeyCode::Down => {
                    let len = app.create_dialog.env_vars_dialog.entries.len();
                    if len > 0 && app.create_dialog.env_vars_dialog.selected + 1 < len {
                        app.create_dialog.env_vars_dialog.selected += 1;
                    }
                }
                KeyCode::Char('a') | KeyCode::Char('A') => {
                    app.create_dialog.env_vars_dialog.mode = SubDialogMode::Add;
                    app.create_dialog.env_vars_dialog.key_input.clear();
                    app.create_dialog.env_vars_dialog.value_input.clear();
                    app.create_dialog.env_vars_dialog.add_field = 0;
                    app.create_dialog.env_vars_dialog.error = None;
                }
                KeyCode::Char('d') | KeyCode::Delete => {
                    let dialog = &mut app.create_dialog.env_vars_dialog;
                    if !dialog.entries.is_empty() {
                        dialog.entries.remove(dialog.selected);
                        if dialog.selected >= dialog.entries.len() && dialog.selected > 0 {
                            dialog.selected -= 1;
                        }
                        dialog.error = None;
                    }
                }
                _ => {}
            }
        }
        SubDialogMode::Add => match code {
            KeyCode::Esc => {
                app.create_dialog.env_vars_dialog.mode = SubDialogMode::List;
                app.create_dialog.env_vars_dialog.error = None;
            }
            KeyCode::Tab | KeyCode::Down => {
                let f = app.create_dialog.env_vars_dialog.add_field;
                app.create_dialog.env_vars_dialog.add_field = (f + 1) % 2;
            }
            KeyCode::BackTab | KeyCode::Up => {
                let f = app.create_dialog.env_vars_dialog.add_field;
                app.create_dialog.env_vars_dialog.add_field = (f + 1) % 2;
            }
            KeyCode::Backspace => {
                let dialog = &mut app.create_dialog.env_vars_dialog;
                if dialog.add_field == 0 {
                    dialog.key_input.pop();
                } else {
                    dialog.value_input.pop();
                }
                dialog.error = None;
            }
            KeyCode::Enter => {
                let dialog = &mut app.create_dialog.env_vars_dialog;
                if dialog.add_field == 0 {
                    // Move focus to the value field.
                    dialog.add_field = 1;
                } else {
                    let key = dialog.key_input.trim().to_owned();
                    let value = dialog.value_input.clone();
                    if key.is_empty() {
                        dialog.error = Some("Key cannot be empty".into());
                    } else if key.contains('=') {
                        dialog.error = Some("Key must not contain '='".into());
                    } else {
                        dialog.entries.push((key, value));
                        dialog.selected = dialog.entries.len().saturating_sub(1);
                        dialog.mode = SubDialogMode::List;
                        dialog.error = None;
                    }
                }
            }
            KeyCode::Char(c) => {
                let dialog = &mut app.create_dialog.env_vars_dialog;
                // Disallow '=' in the key field.
                if dialog.add_field == 0 && c == '=' {
                    dialog.error = Some("Key must not contain '='".into());
                    return;
                }
                if dialog.add_field == 0 {
                    dialog.key_input.push(c);
                } else {
                    dialog.value_input.push(c);
                }
                dialog.error = None;
            }
            _ => {}
        },
    }
}

/// Parse and lightly validate a CIDR string (`a.b.c.d/prefix` shape check).
/// Full semantic validation happens in the SDK's policy builder; this just
/// catches obviously malformed input before it's added to the rule list.
fn validate_cidr(input: &str) -> Result<(), &'static str> {
    let (addr, prefix) = input.split_once('/').ok_or("CIDR must be `addr/prefix`")?;
    if addr.is_empty() {
        return Err("CIDR address cannot be empty");
    }
    let prefix_len: u8 = prefix.parse().map_err(|_| "CIDR prefix must be a number")?;
    if addr.contains(':') {
        if prefix_len > 128 {
            return Err("IPv6 prefix must be 0–128");
        }
    } else {
        if !addr.split('.').all(|o| o.parse::<u8>().is_ok()) || addr.split('.').count() != 4 {
            return Err("Invalid IPv4 address");
        }
        if prefix_len > 32 {
            return Err("IPv4 prefix must be 0–32");
        }
    }
    Ok(())
}

fn handle_network_rules_dialog_key(app: &mut App, code: KeyCode, _mods: KeyModifiers) {
    match app.create_dialog.network_rules_dialog.mode {
        SubDialogMode::List => match code {
            KeyCode::Esc | KeyCode::Enter => {
                // Sync confirmed entries back to the parent field and close.
                let entries = app.create_dialog.network_rules_dialog.entries.clone();
                app.create_dialog.network_rules = entries;
                app.create_dialog.network_rules_dialog.visible = false;
            }
            KeyCode::Up => {
                if app.create_dialog.network_rules_dialog.selected > 0 {
                    app.create_dialog.network_rules_dialog.selected -= 1;
                }
            }
            KeyCode::Down => {
                let len = app.create_dialog.network_rules_dialog.entries.len();
                if len > 0 && app.create_dialog.network_rules_dialog.selected + 1 < len {
                    app.create_dialog.network_rules_dialog.selected += 1;
                }
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                let dialog = &mut app.create_dialog.network_rules_dialog;
                dialog.mode = SubDialogMode::Add;
                dialog.cidr_input.clear();
                dialog.action = NetRuleAction::Allow;
                dialog.direction = NetRuleDirection::Egress;
                dialog.error = None;
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                let dialog = &mut app.create_dialog.network_rules_dialog;
                if !dialog.entries.is_empty() {
                    dialog.entries.remove(dialog.selected);
                    if dialog.selected >= dialog.entries.len() && dialog.selected > 0 {
                        dialog.selected -= 1;
                    }
                    dialog.error = None;
                }
            }
            _ => {}
        },
        SubDialogMode::Add => match code {
            KeyCode::Esc => {
                app.create_dialog.network_rules_dialog.mode = SubDialogMode::List;
                app.create_dialog.network_rules_dialog.error = None;
            }
            KeyCode::Char('e') | KeyCode::Char('E') => {
                app.create_dialog.network_rules_dialog.direction = NetRuleDirection::Egress;
            }
            KeyCode::Char('i') | KeyCode::Char('I') => {
                app.create_dialog.network_rules_dialog.direction = NetRuleDirection::Ingress;
            }
            KeyCode::Char(' ') => {
                let dialog = &mut app.create_dialog.network_rules_dialog;
                dialog.action = match dialog.action {
                    NetRuleAction::Allow => NetRuleAction::Deny,
                    NetRuleAction::Deny => NetRuleAction::Allow,
                };
            }
            KeyCode::Backspace => {
                app.create_dialog.network_rules_dialog.cidr_input.pop();
                app.create_dialog.network_rules_dialog.error = None;
            }
            KeyCode::Enter => {
                let dialog = &mut app.create_dialog.network_rules_dialog;
                let cidr = dialog.cidr_input.trim().to_owned();
                match validate_cidr(&cidr) {
                    Ok(()) => {
                        dialog.entries.push(NetworkRule {
                            cidr,
                            action: dialog.action,
                            direction: dialog.direction,
                        });
                        dialog.selected = dialog.entries.len().saturating_sub(1);
                        dialog.mode = SubDialogMode::List;
                        dialog.error = None;
                    }
                    Err(e) => dialog.error = Some(e.to_string()),
                }
            }
            KeyCode::Char(c) => {
                app.create_dialog.network_rules_dialog.cidr_input.push(c);
                app.create_dialog.network_rules_dialog.error = None;
            }
            _ => {}
        },
    }
}

fn handle_mounts_dialog_key(app: &mut App, code: KeyCode, _mods: KeyModifiers) {
    match app.create_dialog.mounts_dialog.mode {
        SubDialogMode::List => match code {
            KeyCode::Esc | KeyCode::Enter => {
                // Sync confirmed entries back to the parent field and close.
                let entries = app.create_dialog.mounts_dialog.entries.clone();
                app.create_dialog.mounts = entries;
                app.create_dialog.mounts_dialog.visible = false;
            }
            KeyCode::Up => {
                if app.create_dialog.mounts_dialog.selected > 0 {
                    app.create_dialog.mounts_dialog.selected -= 1;
                }
            }
            KeyCode::Down => {
                let len = app.create_dialog.mounts_dialog.entries.len();
                if len > 0 && app.create_dialog.mounts_dialog.selected + 1 < len {
                    app.create_dialog.mounts_dialog.selected += 1;
                }
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                let dialog = &mut app.create_dialog.mounts_dialog;
                dialog.mode = SubDialogMode::Add;
                dialog.guest_input.clear();
                dialog.source_input.clear();
                dialog.kind = MountKindChoice::Bind;
                dialog.add_field = 0;
                dialog.error = None;
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                let dialog = &mut app.create_dialog.mounts_dialog;
                if !dialog.entries.is_empty() {
                    dialog.entries.remove(dialog.selected);
                    if dialog.selected >= dialog.entries.len() && dialog.selected > 0 {
                        dialog.selected -= 1;
                    }
                    dialog.error = None;
                }
            }
            _ => {}
        },
        SubDialogMode::Add => match code {
            KeyCode::Esc => {
                app.create_dialog.mounts_dialog.mode = SubDialogMode::List;
                app.create_dialog.mounts_dialog.error = None;
            }
            KeyCode::Tab | KeyCode::Down => {
                let f = app.create_dialog.mounts_dialog.add_field;
                app.create_dialog.mounts_dialog.add_field = (f + 1) % 2;
            }
            KeyCode::BackTab | KeyCode::Up => {
                let f = app.create_dialog.mounts_dialog.add_field;
                app.create_dialog.mounts_dialog.add_field = (f + 1) % 2;
            }
            KeyCode::Char('b') | KeyCode::Char('B')
                if app.create_dialog.mounts_dialog.add_field == 1 =>
            {
                app.create_dialog.mounts_dialog.kind = MountKindChoice::Bind;
            }
            KeyCode::Char('n') | KeyCode::Char('N')
                if app.create_dialog.mounts_dialog.add_field == 1 =>
            {
                app.create_dialog.mounts_dialog.kind = MountKindChoice::Named;
            }
            KeyCode::Backspace => {
                let dialog = &mut app.create_dialog.mounts_dialog;
                if dialog.add_field == 0 {
                    dialog.guest_input.pop();
                } else {
                    dialog.source_input.pop();
                }
                dialog.error = None;
            }
            KeyCode::Enter => {
                let dialog = &mut app.create_dialog.mounts_dialog;
                if dialog.add_field == 0 {
                    dialog.add_field = 1;
                } else {
                    let guest = dialog.guest_input.trim().to_owned();
                    let source_val = dialog.source_input.trim().to_owned();
                    if guest.is_empty() {
                        dialog.error = Some("Guest path cannot be empty".into());
                    } else if source_val.is_empty() {
                        dialog.error = Some("Host path / volume name cannot be empty".into());
                    } else {
                        let source = match dialog.kind {
                            MountKindChoice::Bind => MountSource::Bind(source_val),
                            MountKindChoice::Named => MountSource::Named(source_val),
                        };
                        dialog.entries.push(VolumeMountConfig {
                            guest_path: guest,
                            source,
                        });
                        dialog.selected = dialog.entries.len().saturating_sub(1);
                        dialog.mode = SubDialogMode::List;
                        dialog.error = None;
                    }
                }
            }
            KeyCode::Char(c) => {
                let dialog = &mut app.create_dialog.mounts_dialog;
                if dialog.add_field == 0 {
                    dialog.guest_input.push(c);
                } else {
                    dialog.source_input.push(c);
                }
                dialog.error = None;
            }
            _ => {}
        },
    }
}

fn handle_volumes_view_key(app: &mut App, code: KeyCode, _mods: KeyModifiers) {
    match app.volumes_view.mode {
        SubDialogMode::List => match code {
            KeyCode::Esc => {
                app.volumes_view.visible = false;
            }
            KeyCode::Up => {
                if app.volumes_view.selected > 0 {
                    app.volumes_view.selected -= 1;
                }
            }
            KeyCode::Down => {
                let len = app.volumes_view.volumes.len();
                if len > 0 && app.volumes_view.selected + 1 < len {
                    app.volumes_view.selected += 1;
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                app.volumes_view.mode = SubDialogMode::Add;
                app.volumes_view.name_input.clear();
                app.volumes_view.disk = false;
                app.volumes_view.error = None;
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                if let Some(vol) = app
                    .volumes_view
                    .volumes
                    .get(app.volumes_view.selected)
                    .cloned()
                {
                    let tx = app.msg_tx.clone();
                    tokio::spawn(async move {
                        let result = crate::sandbox::remove_volume(&vol.name).await;
                        let (msg, is_err) = match result {
                            Ok(()) => (format!("Removed volume '{}'", vol.name), false),
                            Err(e) => (format!("Remove volume failed: {e}"), true),
                        };
                        let _ = tx.send(AppMessage::Notification(msg, is_err));
                        if let Ok(list) = crate::sandbox::list_volumes().await {
                            let _ = tx.send(AppMessage::VolumeList(Ok(list)));
                        }
                    });
                }
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                request_volume_refresh(app);
            }
            _ => {}
        },
        SubDialogMode::Add => match code {
            KeyCode::Esc => {
                app.volumes_view.mode = SubDialogMode::List;
                app.volumes_view.error = None;
            }
            KeyCode::Char(' ') => {
                app.volumes_view.disk = !app.volumes_view.disk;
            }
            KeyCode::Backspace => {
                app.volumes_view.name_input.pop();
                app.volumes_view.error = None;
            }
            KeyCode::Enter => {
                let name = app.volumes_view.name_input.trim().to_owned();
                if name.is_empty() {
                    app.volumes_view.error = Some("Name cannot be empty".into());
                } else {
                    let disk = app.volumes_view.disk;
                    app.volumes_view.mode = SubDialogMode::List;
                    app.volumes_view.error = None;
                    let tx = app.msg_tx.clone();
                    tokio::spawn(async move {
                        let result = crate::sandbox::create_volume(&name, disk, None).await;
                        let (msg, is_err) = match result {
                            Ok(()) => (format!("Created volume '{name}'"), false),
                            Err(e) => (format!("Create volume failed: {e}"), true),
                        };
                        let _ = tx.send(AppMessage::Notification(msg, is_err));
                        if let Ok(list) = crate::sandbox::list_volumes().await {
                            let _ = tx.send(AppMessage::VolumeList(Ok(list)));
                        }
                    });
                }
            }
            KeyCode::Char(c) => {
                app.volumes_view.name_input.push(c);
                app.volumes_view.error = None;
            }
            _ => {}
        },
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

    let ports = dlg.ports.clone();
    let env_vars = dlg.env_vars.clone();

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
    let network_rules = dlg.network_rules.clone();
    let mounts = dlg.mounts.clone();

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
            network_rules,
            mounts,
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

//--------------------------------------------------------------------------------------------------
// Helpers
//--------------------------------------------------------------------------------------------------

/// Handle key input while the search/filter box is active.
fn handle_search_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc => {
            app.search_active = false;
            app.filter.clear();
            clamp_selection_to_filter(app);
        }
        KeyCode::Enter => {
            app.search_active = false;
            clamp_selection_to_filter(app);
        }
        KeyCode::Backspace => {
            app.filter.pop();
            clamp_selection_to_filter(app);
        }
        KeyCode::Char(c) => {
            app.filter.push(c);
            clamp_selection_to_filter(app);
        }
        _ => {}
    }
}

/// Ensure the current selection still points at a visible sandbox after the
/// filter changes; snaps to the first match, or the "New Sandbox" slot when
/// no filter is active and the list is empty.
fn clamp_selection_to_filter(app: &mut App) {
    let visible = app.visible_indices();
    if visible.is_empty() {
        app.selected = if app.filter.trim().is_empty() {
            app.sandboxes.len()
        } else {
            0
        };
        return;
    }
    if !visible.contains(&app.selected) {
        app.selected = visible[0];
    }
}

fn on_sandbox_selected(app: &mut App) {
    app.log_scroll = 0;
    app.fs_scroll = 0;
    app.fs_path = "/".into();
    if let Some(sb) = app.selected_sandbox().cloned() {
        match app.tab {
            DetailTab::Logs => app.sync_log_stream(),
            DetailTab::Filesystem => {
                let path = app.fs_path.clone();
                app.request_fs(&sb.name, &path);
            }
            DetailTab::Info => app.request_metrics(&sb.name),
        }
    }
}

fn on_tab_switched(app: &mut App) {
    if let Some(sb) = app.selected_sandbox().cloned() {
        match app.tab {
            DetailTab::Logs => app.sync_log_stream(),
            DetailTab::Filesystem => {
                let path = app.fs_path.clone();
                app.request_fs(&sb.name, &path);
            }
            DetailTab::Info => app.request_metrics(&sb.name),
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

/// Trigger a background refresh of the named-volumes list.
fn request_volume_refresh(app: &App) {
    let tx = app.msg_tx.clone();
    tokio::spawn(async move {
        let result = crate::sandbox::list_volumes().await;
        let _ = tx.send(AppMessage::VolumeList(result));
    });
}

/// Returns true if a sandbox matches every whitespace-separated token of the
/// filter string. A `status:<value>` token matches the sandbox's status
/// (case-insensitive, e.g. `status:running`); any other token is matched as
/// a case-insensitive substring of the sandbox's name.
fn sandbox_matches_filter(sb: &SandboxInfo, filter: &str) -> bool {
    filter.split_whitespace().all(|token| {
        if let Some(status) = token.strip_prefix("status:") {
            format!("{:?}", sb.status).eq_ignore_ascii_case(status)
        } else {
            sb.name.to_lowercase().contains(&token.to_lowercase())
        }
    })
}

fn action_start(app: &mut App) {
    if !app.marked.is_empty() {
        let names: Vec<String> = app
            .sandboxes
            .iter()
            .filter(|sb| app.marked.contains(&sb.name) && sb.status == Status::Stopped)
            .map(|sb| sb.name.clone())
            .collect();
        app.marked.clear();
        if names.is_empty() {
            app.notify("No marked sandboxes are stopped", true);
        } else {
            app.notify(format!("Starting {} sandboxes…", names.len()), false);
            app.run_bulk_action(SandboxAction::Start, names);
        }
        return;
    }
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
    if !app.marked.is_empty() {
        let names: Vec<String> = app
            .sandboxes
            .iter()
            .filter(|sb| app.marked.contains(&sb.name) && sb.status == Status::Running)
            .map(|sb| sb.name.clone())
            .collect();
        app.marked.clear();
        if names.is_empty() {
            app.notify("No marked sandboxes are running", true);
        } else {
            app.notify(format!("Stopping {} sandboxes…", names.len()), false);
            app.run_bulk_action(SandboxAction::Stop, names);
        }
        return;
    }
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
    if !app.marked.is_empty() {
        let names: Vec<String> = app
            .sandboxes
            .iter()
            .filter(|sb| app.marked.contains(&sb.name) && sb.status == Status::Running)
            .map(|sb| sb.name.clone())
            .collect();
        app.marked.clear();
        if names.is_empty() {
            app.notify("No marked sandboxes are running", true);
        } else {
            app.notify(format!("Killing {} sandboxes…", names.len()), false);
            app.run_bulk_action(SandboxAction::Kill, names);
        }
        return;
    }
    if let Some(sb) = app.selected_sandbox().cloned() {
        if sb.status == Status::Running {
            app.run_action(SandboxAction::Kill, &sb.name);
            app.notify(format!("Killing '{}'…", sb.name), false);
        }
    }
}

fn action_remove(app: &mut App) {
    if !app.marked.is_empty() {
        let names: Vec<String> = app
            .sandboxes
            .iter()
            .filter(|sb| app.marked.contains(&sb.name) && sb.status != Status::Running)
            .map(|sb| sb.name.clone())
            .collect();
        app.marked.clear();
        if names.is_empty() {
            app.notify("No marked sandboxes can be removed (stop them first)", true);
        } else {
            app.notify(format!("Removing {} sandboxes…", names.len()), false);
            app.run_bulk_action(SandboxAction::Remove, names);
        }
        return;
    }
    if let Some(sb) = app.selected_sandbox().cloned() {
        if sb.status != Status::Running {
            app.run_action(SandboxAction::Remove, &sb.name);
            app.notify(format!("Removing '{}'…", sb.name), false);
        } else {
            app.notify("Stop the sandbox before removing", true);
        }
    }
}

/// Toggle the mark on the currently highlighted sandbox for bulk operations.
fn toggle_mark(app: &mut App) {
    if app.new_sandbox_selected() {
        return;
    }
    if let Some(sb) = app.selected_sandbox() {
        let name = sb.name.clone();
        if !app.marked.remove(&name) {
            app.marked.insert(name);
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

    /// Build a mouse Event at the given column/row.
    fn mouse_event(kind: MouseEventKind, column: u16, row: u16) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
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

    // ── log_stream_target ────────────────────────────────────────────────────

    #[test]
    fn test_log_stream_target_none_when_not_on_logs_tab() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("alpha", Status::Running));
        app.tab = DetailTab::Info;
        assert!(app.log_stream_target().is_none());
    }

    #[test]
    fn test_log_stream_target_none_when_sandbox_stopped() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("alpha", Status::Stopped));
        app.tab = DetailTab::Logs;
        assert!(app.log_stream_target().is_none());
    }

    #[test]
    fn test_log_stream_target_some_when_running_on_logs_tab() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("alpha", Status::Running));
        app.tab = DetailTab::Logs;
        assert_eq!(app.log_stream_target().as_deref(), Some("alpha"));
    }

    #[test]
    fn test_log_stream_target_none_when_no_sandbox_selected() {
        let app = make_app();
        assert!(app.log_stream_target().is_none());
    }

    // ── LogStreamEntry message handling ──────────────────────────────────────

    #[test]
    fn test_handle_message_log_stream_entry_appends() {
        use microsandbox::logs::LogCursor;
        use microsandbox::sandbox::{LogEntry, LogSource};

        let mut app = make_app();
        let entry = LogEntry {
            timestamp: chrono::Utc::now(),
            source: LogSource::Stdout,
            session_id: None,
            data: b"hello".as_slice().into(),
            cursor: LogCursor::empty(),
        };
        app.handle_message(AppMessage::LogStreamEntry("alpha".into(), entry));
        assert_eq!(app.logs.get("alpha").map(|v| v.len()), Some(1));
    }

    #[test]
    fn test_handle_message_log_stream_entry_caps_at_max_lines() {
        use microsandbox::logs::LogCursor;
        use microsandbox::sandbox::{LogEntry, LogSource};

        let mut app = make_app();
        for _ in 0..(MAX_LOG_LINES + 10) {
            let entry = LogEntry {
                timestamp: chrono::Utc::now(),
                source: LogSource::Stdout,
                session_id: None,
                data: b"x".as_slice().into(),
                cursor: LogCursor::empty(),
            };
            app.handle_message(AppMessage::LogStreamEntry("alpha".into(), entry));
        }
        assert_eq!(app.logs.get("alpha").map(|v| v.len()), Some(MAX_LOG_LINES));
    }

    // ── metrics history ───────────────────────────────────────────────────────

    #[test]
    fn test_handle_message_metrics_appends_history() {
        let mut app = make_app();
        for i in 0..3 {
            let m = MetricsSnapshot {
                cpu_percent: i as f64,
                ..Default::default()
            };
            app.handle_message(AppMessage::Metrics("alpha".into(), Ok(Some(m))));
        }
        let history = app.metrics_history.get("alpha").unwrap();
        assert_eq!(history.len(), 3);
        assert_eq!(history.back().unwrap().cpu_percent, 2.0);
    }

    #[test]
    fn test_handle_message_metrics_history_caps_at_limit() {
        let mut app = make_app();
        for _ in 0..(METRICS_HISTORY_LEN + 15) {
            let m = MetricsSnapshot::default();
            app.handle_message(AppMessage::Metrics("alpha".into(), Ok(Some(m))));
        }
        let history = app.metrics_history.get("alpha").unwrap();
        assert_eq!(history.len(), METRICS_HISTORY_LEN);
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
        assert_eq!(dlg.field, 7);
        dlg.next_field();
        assert_eq!(dlg.field, 8); // Create button
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
        dlg.current_field_mut().unwrap().push('4');
        assert_eq!(dlg.cpus, "14");
        dlg.field = 3;
        dlg.current_field_mut().unwrap().push_str("1024");
        assert_eq!(dlg.memory, "5121024");
        // Fields 4 (ports), 5 (env vars), and 6 (workdir) are managed by
        // sub-dialogs / the directory picker rather than direct text input.
        dlg.field = 4;
        assert!(dlg.current_field_mut().is_none());
        dlg.field = 5;
        assert!(dlg.current_field_mut().is_none());
        dlg.field = 6;
        assert!(dlg.current_field_mut().is_none());
    }

    // ── DetailTab helpers ────────────────────────────────────────────────────

    #[test]
    fn test_detail_tab_titles() {
        assert_eq!(DetailTab::Logs.title(), "Logs");
        assert_eq!(DetailTab::Filesystem.title(), "Filesystem");
        assert_eq!(DetailTab::Info.title(), "Info");
    }

    #[test]
    fn test_detail_tab_all_has_three_entries() {
        assert_eq!(DetailTab::all().len(), 3);
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
        assert_eq!(app.create_dialog.field, 8); // Create button
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

        app.create_dialog.field = app.create_dialog.form_field_count(); // Create button
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

        app.create_dialog.field = app.create_dialog.form_field_count(); // Create button
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
    fn test_dialog_ports_sub_dialog_add_entry() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.field = 4;
        handle_event(&mut app, key_press(KeyCode::Enter));
        assert!(app.create_dialog.ports_dialog.visible);
        handle_event(&mut app, key_press(KeyCode::Char('a')));
        for ch in "8080".chars() {
            handle_event(&mut app, key_press(KeyCode::Char(ch)));
        }
        handle_event(&mut app, key_press(KeyCode::Enter));
        for ch in "80".chars() {
            handle_event(&mut app, key_press(KeyCode::Char(ch)));
        }
        handle_event(&mut app, key_press(KeyCode::Enter));
        handle_event(&mut app, key_press(KeyCode::Esc));
        assert_eq!(app.create_dialog.ports, vec![(8080, 80)]);
    }

    #[test]
    fn test_dialog_env_vars_sub_dialog_add_entry() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.field = 5;
        handle_event(&mut app, key_press(KeyCode::Enter));
        assert!(app.create_dialog.env_vars_dialog.visible);
        handle_event(&mut app, key_press(KeyCode::Char('a')));
        for ch in "FOO".chars() {
            handle_event(&mut app, key_press(KeyCode::Char(ch)));
        }
        handle_event(&mut app, key_press(KeyCode::Enter));
        for ch in "bar".chars() {
            handle_event(&mut app, key_press(KeyCode::Char(ch)));
        }
        handle_event(&mut app, key_press(KeyCode::Enter));
        handle_event(&mut app, key_press(KeyCode::Esc));
        assert_eq!(
            app.create_dialog.env_vars,
            vec![("FOO".to_string(), "bar".to_string())]
        );
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

    #[test]
    fn test_dialog_network_rules_sub_dialog_add_entry() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.switch_tab(DialogTab::Advanced);
        app.create_dialog.field = 6;
        handle_event(&mut app, key_press(KeyCode::Enter));
        assert!(app.create_dialog.network_rules_dialog.visible);
        handle_event(&mut app, key_press(KeyCode::Char('a')));
        handle_event(&mut app, key_press(KeyCode::Char('i')));
        handle_event(&mut app, key_press(KeyCode::Char(' ')));
        for ch in "10.0.0.0/8".chars() {
            handle_event(&mut app, key_press(KeyCode::Char(ch)));
        }
        handle_event(&mut app, key_press(KeyCode::Enter));
        handle_event(&mut app, key_press(KeyCode::Esc));
        assert_eq!(
            app.create_dialog.network_rules,
            vec![NetworkRule {
                cidr: "10.0.0.0/8".into(),
                action: NetRuleAction::Deny,
                direction: NetRuleDirection::Ingress,
            }]
        );
    }

    #[test]
    fn test_dialog_network_rules_invalid_cidr_shows_error() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.switch_tab(DialogTab::Advanced);
        app.create_dialog.field = 6;
        handle_event(&mut app, key_press(KeyCode::Enter));
        handle_event(&mut app, key_press(KeyCode::Char('a')));
        for ch in "not-a-cidr".chars() {
            handle_event(&mut app, key_press(KeyCode::Char(ch)));
        }
        handle_event(&mut app, key_press(KeyCode::Enter));
        assert!(app.create_dialog.network_rules_dialog.error.is_some());
        assert!(app.create_dialog.network_rules_dialog.entries.is_empty());
    }

    #[test]
    fn test_dialog_network_rules_delete_entry() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.network_rules_dialog = NetworkRulesDialog::open(vec![NetworkRule {
            cidr: "1.2.3.0/24".into(),
            action: NetRuleAction::Allow,
            direction: NetRuleDirection::Egress,
        }]);
        handle_event(&mut app, key_press(KeyCode::Char('d')));
        assert!(app.create_dialog.network_rules_dialog.entries.is_empty());
    }

    #[test]
    fn test_validate_cidr_accepts_valid_ipv4() {
        assert!(validate_cidr("10.0.0.0/8").is_ok());
        assert!(validate_cidr("192.168.1.1/32").is_ok());
    }

    #[test]
    fn test_validate_cidr_rejects_malformed() {
        assert!(validate_cidr("not-a-cidr").is_err());
        assert!(validate_cidr("10.0.0.0/99").is_err());
        assert!(validate_cidr("999.0.0.0/8").is_err());
        assert!(validate_cidr("10.0.0.0").is_err());
    }

    #[test]
    fn test_dialog_mounts_sub_dialog_add_bind_entry() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.field = 7;
        handle_event(&mut app, key_press(KeyCode::Enter));
        assert!(app.create_dialog.mounts_dialog.visible);
        handle_event(&mut app, key_press(KeyCode::Char('a')));
        for ch in "/data".chars() {
            handle_event(&mut app, key_press(KeyCode::Char(ch)));
        }
        handle_event(&mut app, key_press(KeyCode::Enter));
        for ch in "/host/data".chars() {
            handle_event(&mut app, key_press(KeyCode::Char(ch)));
        }
        handle_event(&mut app, key_press(KeyCode::Enter));
        handle_event(&mut app, key_press(KeyCode::Esc));
        assert_eq!(
            app.create_dialog.mounts,
            vec![VolumeMountConfig {
                guest_path: "/data".into(),
                source: MountSource::Bind("/host/data".into()),
            }]
        );
    }

    #[test]
    fn test_dialog_mounts_sub_dialog_add_named_entry() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.mounts_dialog = MountsDialog::open(vec![]);
        handle_event(&mut app, key_press(KeyCode::Char('a')));
        for ch in "/cache".chars() {
            handle_event(&mut app, key_press(KeyCode::Char(ch)));
        }
        handle_event(&mut app, key_press(KeyCode::Enter)); // move to source field
        handle_event(&mut app, key_press(KeyCode::Char('n'))); // choose Named kind
        for ch in "my-cache".chars() {
            handle_event(&mut app, key_press(KeyCode::Char(ch)));
        }
        handle_event(&mut app, key_press(KeyCode::Enter));
        assert_eq!(
            app.create_dialog.mounts_dialog.entries,
            vec![VolumeMountConfig {
                guest_path: "/cache".into(),
                source: MountSource::Named("my-cache".into()),
            }]
        );
    }

    #[test]
    fn test_dialog_mounts_delete_entry() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.mounts_dialog = MountsDialog::open(vec![VolumeMountConfig {
            guest_path: "/data".into(),
            source: MountSource::Bind("/host".into()),
        }]);
        handle_event(&mut app, key_press(KeyCode::Char('d')));
        assert!(app.create_dialog.mounts_dialog.entries.is_empty());
    }

    #[tokio::test]
    async fn test_v_key_opens_volumes_view() {
        let mut app = make_app();
        handle_event(&mut app, key_press(KeyCode::Char('v')));
        assert!(app.volumes_view.visible);
    }

    #[test]
    fn test_volumes_view_esc_closes() {
        let mut app = make_app();
        app.volumes_view = VolumesView::open();
        handle_event(&mut app, key_press(KeyCode::Esc));
        assert!(!app.volumes_view.visible);
    }

    #[test]
    fn test_volumes_view_navigation() {
        let mut app = make_app();
        app.volumes_view = VolumesView::open();
        app.volumes_view.volumes = vec![
            VolumeInfo {
                name: "a".into(),
                kind: microsandbox::VolumeKind::Directory,
                quota_mib: None,
                used_bytes: 0,
            },
            VolumeInfo {
                name: "b".into(),
                kind: microsandbox::VolumeKind::Directory,
                quota_mib: None,
                used_bytes: 0,
            },
        ];
        handle_event(&mut app, key_press(KeyCode::Down));
        assert_eq!(app.volumes_view.selected, 1);
        handle_event(&mut app, key_press(KeyCode::Up));
        assert_eq!(app.volumes_view.selected, 0);
    }

    #[test]
    fn test_volumes_view_add_mode_toggle_and_validation() {
        let mut app = make_app();
        app.volumes_view = VolumesView::open();
        handle_event(&mut app, key_press(KeyCode::Char('n')));
        assert_eq!(app.volumes_view.mode, SubDialogMode::Add);
        handle_event(&mut app, key_press(KeyCode::Char(' ')));
        assert!(app.volumes_view.disk);
        handle_event(&mut app, key_press(KeyCode::Enter));
        assert!(app.volumes_view.error.is_some());
    }

    #[test]
    fn test_handle_message_volume_list_updates_state() {
        let mut app = make_app();
        let volumes = vec![VolumeInfo {
            name: "vol1".into(),
            kind: microsandbox::VolumeKind::Disk,
            quota_mib: Some(1024),
            used_bytes: 512,
        }];
        app.handle_message(AppMessage::VolumeList(Ok(volumes.clone())));
        assert_eq!(app.volumes_view.volumes, volumes);
    }

    #[tokio::test]
    async fn test_submit_parses_ports_correctly() {
        let mut app = make_app();
        app.create_dialog = CreateDialog::open();
        app.create_dialog.name = "mybox".into();
        app.create_dialog.ports = vec![(8080, 80), (443, 443)];

        app.create_dialog.field = app.create_dialog.form_field_count(); // Create button
        handle_event(&mut app, key_press(KeyCode::Enter));
        assert!(!app.create_dialog.visible);
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
        assert_eq!(app.tab, DetailTab::Filesystem);
    }

    #[tokio::test]
    async fn test_l_advances_tab_in_detail_focus() {
        let mut app = make_app();
        app.focus = Focus::Detail;
        handle_event(&mut app, key_press(KeyCode::Char('l')));
        assert_eq!(app.tab, DetailTab::Filesystem);
    }

    #[tokio::test]
    async fn test_left_goes_back_tab_in_detail_focus() {
        let mut app = make_app();
        app.focus = Focus::Detail;
        app.tab = DetailTab::Filesystem;
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

    #[test]
    fn test_new_sandbox_dialog_prefilled_from_config() {
        let mut app = make_app();
        app.config = AppConfig::parse(
            r#"
                image = "ubuntu:22.04"
                cpus = 4
                memory_mib = 2048
                hostname = "dev-box"
                workdir = "/workspace"
                user = "dev"
                shell = "/bin/bash"
            "#,
        )
        .unwrap();
        handle_event(&mut app, key_press(KeyCode::Char('n')));
        assert_eq!(app.create_dialog.image, "ubuntu:22.04");
        assert_eq!(app.create_dialog.cpus, "4");
        assert_eq!(app.create_dialog.memory, "2048");
        assert_eq!(app.create_dialog.hostname, "dev-box");
        assert_eq!(app.create_dialog.workdir, "/workspace");
        assert_eq!(app.create_dialog.user, "dev");
        assert_eq!(app.create_dialog.shell, "/bin/bash");
    }

    #[test]
    fn test_new_sandbox_dialog_uses_builtin_defaults_when_config_empty() {
        let mut app = make_app();
        assert_eq!(app.config, AppConfig::default());
        handle_event(&mut app, key_press(KeyCode::Char('n')));
        assert_eq!(app.create_dialog.image, "alpine");
        assert_eq!(app.create_dialog.cpus, "1");
        assert_eq!(app.create_dialog.memory, "512");
        assert_eq!(app.create_dialog.shell, "/bin/sh");
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

    // ── handle_event: multi-select mark & bulk actions ───────────────────────

    #[test]
    fn test_space_toggles_mark_on_selected_sandbox() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("box1", Status::Stopped));
        app.selected = 0;
        handle_event(&mut app, key_press(KeyCode::Char(' ')));
        assert!(app.marked.contains("box1"));
        handle_event(&mut app, key_press(KeyCode::Char(' ')));
        assert!(!app.marked.contains("box1"));
    }

    #[test]
    fn test_space_on_new_sandbox_slot_does_nothing() {
        let mut app = make_app();
        // selected == 0 == len() → new sandbox slot, no real sandbox to mark
        handle_event(&mut app, key_press(KeyCode::Char(' ')));
        assert!(app.marked.is_empty());
    }

    #[tokio::test]
    async fn test_bulk_start_uses_marked_sandboxes_and_clears_marks() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("box1", Status::Stopped));
        app.sandboxes.push(make_sandbox("box2", Status::Stopped));
        app.marked.insert("box1".to_string());
        app.marked.insert("box2".to_string());
        handle_event(&mut app, key_press(KeyCode::Char('s')));
        assert!(app.marked.is_empty());
        let n = app.notification.as_ref().unwrap();
        assert!(!n.is_error);
        assert!(n.message.contains("2"));
    }

    #[tokio::test]
    async fn test_bulk_start_ignores_marked_sandboxes_not_stopped() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("box1", Status::Running));
        app.marked.insert("box1".to_string());
        handle_event(&mut app, key_press(KeyCode::Char('s')));
        assert!(app.marked.is_empty());
        assert!(app.notification.as_ref().unwrap().is_error);
    }

    #[tokio::test]
    async fn test_bulk_stop_filters_to_running_marked_sandboxes() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("box1", Status::Running));
        app.sandboxes.push(make_sandbox("box2", Status::Stopped));
        app.marked.insert("box1".to_string());
        app.marked.insert("box2".to_string());
        handle_event(&mut app, key_press(KeyCode::Char('S')));
        assert!(app.marked.is_empty());
        let n = app.notification.as_ref().unwrap();
        assert!(!n.is_error);
        assert!(n.message.contains('1'));
    }

    #[tokio::test]
    async fn test_bulk_remove_filters_to_non_running_marked_sandboxes() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("box1", Status::Stopped));
        app.sandboxes.push(make_sandbox("box2", Status::Running));
        app.marked.insert("box1".to_string());
        app.marked.insert("box2".to_string());
        handle_event(&mut app, key_press(KeyCode::Char('d')));
        assert!(app.marked.is_empty());
        let n = app.notification.as_ref().unwrap();
        assert!(!n.is_error);
        assert!(n.message.contains('1'));
    }

    #[test]
    fn test_sandbox_list_refresh_prunes_stale_marks() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("box1", Status::Stopped));
        app.marked.insert("box1".to_string());
        app.marked.insert("gone".to_string());
        app.handle_message(AppMessage::SandboxList(Ok(vec![make_sandbox(
            "box1",
            Status::Stopped,
        )])));
        assert!(app.marked.contains("box1"));
        assert!(!app.marked.contains("gone"));
    }

    // ── handle_event: search/filter ──────────────────────────────────────────

    #[test]
    fn test_slash_activates_search_mode() {
        let mut app = make_app();
        handle_event(&mut app, key_press(KeyCode::Char('/')));
        assert!(app.search_active);
    }

    #[test]
    fn test_typing_in_search_mode_updates_filter_live() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("alpha", Status::Running));
        app.sandboxes.push(make_sandbox("beta", Status::Stopped));
        handle_event(&mut app, key_press(KeyCode::Char('/')));
        handle_event(&mut app, key_press(KeyCode::Char('a')));
        handle_event(&mut app, key_press(KeyCode::Char('l')));
        assert_eq!(app.filter, "al");
        assert_eq!(app.visible_indices(), vec![0]);
    }

    #[test]
    fn test_backspace_in_search_mode_edits_filter() {
        let mut app = make_app();
        handle_event(&mut app, key_press(KeyCode::Char('/')));
        handle_event(&mut app, key_press(KeyCode::Char('a')));
        handle_event(&mut app, key_press(KeyCode::Char('b')));
        handle_event(&mut app, key_press(KeyCode::Backspace));
        assert_eq!(app.filter, "a");
    }

    #[test]
    fn test_esc_in_search_mode_clears_filter_and_exits() {
        let mut app = make_app();
        handle_event(&mut app, key_press(KeyCode::Char('/')));
        handle_event(&mut app, key_press(KeyCode::Char('x')));
        handle_event(&mut app, key_press(KeyCode::Esc));
        assert!(!app.search_active);
        assert!(app.filter.is_empty());
    }

    #[test]
    fn test_enter_in_search_mode_keeps_filter_and_exits_typing() {
        let mut app = make_app();
        handle_event(&mut app, key_press(KeyCode::Char('/')));
        handle_event(&mut app, key_press(KeyCode::Char('x')));
        handle_event(&mut app, key_press(KeyCode::Enter));
        assert!(!app.search_active);
        assert_eq!(app.filter, "x");
    }

    #[test]
    fn test_status_token_filters_by_status() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("alpha", Status::Running));
        app.sandboxes.push(make_sandbox("beta", Status::Stopped));
        app.filter = "status:running".to_string();
        assert_eq!(app.visible_indices(), vec![0]);
    }

    #[test]
    fn test_new_sandbox_slot_hidden_while_filter_active() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("alpha", Status::Running));
        app.selected = 1; // "new sandbox" slot when no filter
        assert!(app.new_sandbox_selected());
        app.filter = "alpha".to_string();
        assert!(!app.new_sandbox_selected());
    }

    // ── handle_event: mouse support ──────────────────────────────────────────

    #[test]
    fn test_mouse_click_on_card_selects_it() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("box0", Status::Stopped));
        app.sandboxes.push(make_sandbox("box1", Status::Stopped));
        app.mouse.card_rects = vec![
            (Rect::new(0, 0, 10, 6), Some(0)),
            (Rect::new(0, 6, 10, 6), Some(1)),
        ];
        app.selected = 0;
        handle_event(
            &mut app,
            mouse_event(MouseEventKind::Down(MouseButton::Left), 3, 7),
        );
        assert_eq!(app.selected, 1);
        assert_eq!(app.focus, Focus::SandboxList);
    }

    #[tokio::test]
    async fn test_mouse_click_on_tab_switches_tab_and_focus() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("box0", Status::Running));
        app.mouse.tab_rects = vec![
            (Rect::new(0, 0, 8, 1), DetailTab::Logs),
            (Rect::new(8, 0, 10, 1), DetailTab::Filesystem),
        ];
        app.tab = DetailTab::Logs;
        handle_event(
            &mut app,
            mouse_event(MouseEventKind::Down(MouseButton::Left), 9, 0),
        );
        assert_eq!(app.tab, DetailTab::Filesystem);
        assert_eq!(app.focus, Focus::Detail);
    }

    #[test]
    fn test_mouse_scroll_over_list_moves_selection() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("box0", Status::Stopped));
        app.sandboxes.push(make_sandbox("box1", Status::Stopped));
        app.mouse.list_area = Rect::new(0, 0, 20, 20);
        app.selected = 0;
        handle_event(&mut app, mouse_event(MouseEventKind::ScrollDown, 2, 2));
        assert_eq!(app.selected, 1);
        handle_event(&mut app, mouse_event(MouseEventKind::ScrollUp, 2, 2));
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn test_mouse_scroll_over_detail_scrolls_logs() {
        let mut app = make_app();
        app.mouse.detail_area = Rect::new(20, 0, 20, 20);
        app.tab = DetailTab::Logs;
        app.log_scroll = 5;
        handle_event(&mut app, mouse_event(MouseEventKind::ScrollDown, 21, 2));
        assert_eq!(app.log_scroll, 8);
        handle_event(&mut app, mouse_event(MouseEventKind::ScrollUp, 21, 2));
        assert_eq!(app.log_scroll, 5);
    }

    #[test]
    fn test_mouse_ignored_while_dialog_open() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("box0", Status::Stopped));
        app.create_dialog.visible = true;
        app.mouse.card_rects = vec![(Rect::new(0, 0, 10, 6), Some(0))];
        app.selected = 0;
        handle_event(
            &mut app,
            mouse_event(MouseEventKind::Down(MouseButton::Left), 3, 3),
        );
        // Selection must not change while the modal dialog steals input.
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn test_mouse_click_on_new_sandbox_card_selects_placeholder() {
        let mut app = make_app();
        app.sandboxes.push(make_sandbox("box0", Status::Stopped));
        app.mouse.card_rects = vec![
            (Rect::new(0, 0, 10, 6), Some(0)),
            (Rect::new(0, 6, 10, 3), None),
        ];
        handle_event(
            &mut app,
            mouse_event(MouseEventKind::Down(MouseButton::Left), 3, 7),
        );
        assert!(app.new_sandbox_selected());
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
        app.create_dialog.field = 8; // Create button (last position)
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
        assert_eq!(app.create_dialog.field, 8); // Create button
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

        app.create_dialog.field = app.create_dialog.form_field_count(); // Create button
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

        app.create_dialog.field = app.create_dialog.form_field_count(); // Create button
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

        app.create_dialog.field = app.create_dialog.form_field_count(); // Create button
        handle_event(&mut app, key_press(KeyCode::Enter));
        assert!(app.create_dialog.visible);
        let err = app.create_dialog.error.as_deref().unwrap_or("");
        assert!(err.contains("64"), "error must mention minimum memory");
    }
}
