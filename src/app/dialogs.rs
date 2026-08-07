//! State for the create-sandbox dialog, its sub-dialogs, the directory
//! picker, and the volumes management view.
//!
//! Nothing in this module drives the terminal directly — key handling lives
//! in [`super::keys`] and rendering lives in `crate::ui`. This module only
//! owns the plain-data state and small helper methods each dialog needs.

use crate::config::AppConfig;
use crate::sandbox::{NetRuleAction, NetRuleDirection, NetworkRule, VolumeInfo, VolumeMountConfig};

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

    pub fn title(self) -> &'static str {
        match self {
            DialogTab::Basic => "Basic",
            DialogTab::Advanced => "Advanced",
        }
    }
}

/// Sentinel entry that opens the drive-selection view.
pub const DRIVES_ENTRY: &str = "⊞ [Switch Drive]";

/// The number of picker entries that fit in the visible list area.
pub const PICKER_VISIBLE_ROWS: usize = 10;

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

/// The command line prefilled in the "Exec" dialog: opens a plain shell in
/// the sandbox by default.
pub const DEFAULT_EXEC_COMMAND: &str = "sh";

/// State of the "Exec" dialog: prompts for a command line to run inside a
/// running sandbox, then opens a new terminal window on the host that runs
/// it there via the `msb` CLI's `exec` subcommand.
#[derive(Debug, Clone, Default)]
pub struct ExecDialog {
    pub visible: bool,
    /// Name of the sandbox the command will be executed in.
    pub sandbox_name: String,
    /// Command line typed by the user, defaults to [`DEFAULT_EXEC_COMMAND`].
    pub command: String,
    pub error: Option<String>,
}

impl ExecDialog {
    /// Open the dialog for the given sandbox, prefilled with the default
    /// shell command.
    pub fn open(sandbox_name: impl Into<String>) -> Self {
        Self {
            visible: true,
            sandbox_name: sandbox_name.into(),
            command: DEFAULT_EXEC_COMMAND.to_owned(),
            error: None,
        }
    }
}
