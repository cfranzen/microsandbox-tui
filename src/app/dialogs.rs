//! State for the create-sandbox dialog, its sub-dialogs, the directory
//! picker, and the volumes management view.
//!
//! Nothing in this module drives the terminal directly — key handling lives
//! in [`super::keys`] and rendering lives in `crate::ui`. This module only
//! owns the plain-data state and small helper methods each dialog needs.

use crate::config::AppConfig;
use crate::sandbox::{
    NetRuleAction, NetRuleDestGroup, NetRuleDestKind, NetRuleDirection, NetRuleProtocol,
    NetworkRule, SecretConfig, VolumeInfo, VolumeMountConfig,
};

/// Which tab of the "create new sandbox" dialog is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DialogTab {
    #[default]
    Basic,
    GuestOs,
    Network,
    Secrets,
}

impl DialogTab {
    const ALL: [DialogTab; 4] = [
        DialogTab::Basic,
        DialogTab::GuestOs,
        DialogTab::Network,
        DialogTab::Secrets,
    ];

    pub fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        let idx = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(idx + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    pub fn title(self) -> &'static str {
        match self {
            DialogTab::Basic => "Basic",
            DialogTab::GuestOs => "Guest OS",
            DialogTab::Network => "Network",
            DialogTab::Secrets => "Secrets",
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

/// Mode for the Volumes view's list/add sub-states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubDialogMode {
    #[default]
    List,
    Add,
}

/// Popup dialog for adding one port mapping (host:guest). Entries already
/// added are listed inline in the Network tab; this popup only handles
/// adding a new one.
#[derive(Debug, Clone, Default)]
pub struct PortAddDialog {
    pub visible: bool,
    /// Host-port input buffer.
    pub host_input: String,
    /// Guest-port input buffer.
    pub guest_input: String,
    /// Focused input index: 0 = host, 1 = guest.
    pub add_field: usize,
    pub error: Option<String>,
}

impl PortAddDialog {
    pub fn open() -> Self {
        Self {
            visible: true,
            ..Default::default()
        }
    }
}

/// Popup dialog for adding one environment variable (KEY=VALUE). Entries
/// already added are listed inline in the Guest OS tab; this popup only
/// handles adding a new one.
#[derive(Debug, Clone, Default)]
pub struct EnvVarAddDialog {
    pub visible: bool,
    /// Key input buffer.
    pub key_input: String,
    /// Value input buffer.
    pub value_input: String,
    /// Focused input index: 0 = key, 1 = value.
    pub add_field: usize,
    pub error: Option<String>,
}

impl EnvVarAddDialog {
    pub fn open() -> Self {
        Self {
            visible: true,
            ..Default::default()
        }
    }
}

/// Popup dialog for adding one network policy rule. Entries already added
/// are listed inline in the Network tab; this popup only handles adding a
/// new one, and exposes the full expressiveness of the SDK's
/// `RuleBuilder`: direction, action, destination kind/value/group, protocol
/// filter, and an optional guest-side port or port range.
#[derive(Debug, Clone)]
pub struct NetRuleAddDialog {
    pub visible: bool,
    pub direction: NetRuleDirection,
    pub action: NetRuleAction,
    pub dest_kind: NetRuleDestKind,
    /// Free-text destination value (IP / CIDR / Domain / Domain Suffix).
    pub dest_input: String,
    pub dest_group: NetRuleDestGroup,
    /// Which protocols are currently toggled on.
    pub protocols: Vec<NetRuleProtocol>,
    /// Index into [`NetRuleProtocol::ALL`] currently highlighted for toggling.
    pub protocol_cursor: usize,
    /// Port or port-range input buffer, e.g. `"8080"` or `"1000-2000"`.
    pub ports_input: String,
    /// Focused field: 0 direction, 1 action, 2 dest kind, 3 dest value/group,
    /// 4 protocols, 5 ports.
    pub add_field: usize,
    pub error: Option<String>,
}

impl Default for NetRuleAddDialog {
    fn default() -> Self {
        Self {
            visible: false,
            direction: NetRuleDirection::default(),
            action: NetRuleAction::default(),
            dest_kind: NetRuleDestKind::default(),
            dest_input: String::new(),
            dest_group: NetRuleDestGroup::default(),
            protocols: Vec::new(),
            protocol_cursor: 0,
            ports_input: String::new(),
            add_field: 0,
            error: None,
        }
    }
}

impl NetRuleAddDialog {
    /// Total number of navigable fields in the Add popup.
    pub const FIELD_COUNT: usize = 6;

    pub fn open() -> Self {
        Self {
            visible: true,
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

/// Popup dialog for adding one volume mount. Entries already added are
/// listed inline in the Guest OS tab; this popup only handles adding a new
/// one.
#[derive(Debug, Clone, Default)]
pub struct MountAddDialog {
    pub visible: bool,
    /// Guest path input buffer.
    pub guest_input: String,
    /// Host path (Bind) or volume name (Named) input buffer.
    pub source_input: String,
    /// Which mount source kind is being configured.
    pub kind: MountKindChoice,
    /// Focused input index: 0 = guest path, 1 = source.
    pub add_field: usize,
    pub error: Option<String>,
}

impl MountAddDialog {
    pub fn open() -> Self {
        Self {
            visible: true,
            ..Default::default()
        }
    }
}

/// Popup dialog for adding one secret. Entries already added are listed
/// inline in the Secrets tab; this popup only handles adding a new one.
///
/// Mirrors the SDK's `SecretBuilder`: an env var + value pair, one or more
/// allowed hosts (comma-separated; `*.suffix` for a wildcard, a bare `*`
/// for "any host, dangerous"), and the four injection scopes plus the
/// TLS-identity requirement, each defaulted to match `SecretBuilder::new()`.
#[derive(Debug, Clone)]
pub struct SecretAddDialog {
    pub visible: bool,
    pub env_input: String,
    /// Real secret value. Rendered masked (`******`) in the UI.
    pub value_input: String,
    /// Comma-separated allowed hosts.
    pub hosts_input: String,
    pub inject_headers: bool,
    pub inject_basic_auth: bool,
    pub inject_query: bool,
    pub inject_body: bool,
    pub require_tls_identity: bool,
    /// Focused field: 0 env, 1 value, 2 hosts, 3 headers, 4 basic auth,
    /// 5 query, 6 body, 7 require TLS identity.
    pub add_field: usize,
    pub error: Option<String>,
}

impl Default for SecretAddDialog {
    fn default() -> Self {
        Self {
            visible: false,
            env_input: String::new(),
            value_input: String::new(),
            hosts_input: String::new(),
            inject_headers: true,
            inject_basic_auth: true,
            inject_query: false,
            inject_body: false,
            require_tls_identity: true,
            add_field: 0,
            error: None,
        }
    }
}

impl SecretAddDialog {
    /// Total number of navigable fields in the Add popup.
    pub const FIELD_COUNT: usize = 8;

    pub fn open() -> Self {
        Self {
            visible: true,
            ..Default::default()
        }
    }
}

/// Identifies which inline list is currently focused within a
/// [`CreateDialog`] tab, so key handling can route Up/Down/`a`/`d` to the
/// right list instead of the outer field navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListField {
    EnvVars,
    Mounts,
    Ports,
    NetworkRules,
    Secrets,
}

/// State of the "create new sandbox" modal dialog.
#[derive(Debug, Clone, Default)]
pub struct CreateDialog {
    pub visible: bool,
    pub tab: DialogTab,
    pub field: usize,

    // ── Basic tab (fields 0-6) ───────────────────────────────────────────
    pub name: String,
    pub image: String,
    pub cpus: String,
    pub max_cpus: String,
    pub memory: String,
    pub max_memory: String,
    pub workdir: String,

    // ── Guest OS tab (fields 0-4) ────────────────────────────────────────
    pub hostname: String,
    pub user: String,
    pub shell: String,
    /// Environment variables (key, value), listed inline in the tab.
    pub env_vars: Vec<(String, String)>,
    pub env_vars_selected: usize,
    /// Volume mounts, listed inline in the tab. Applied at creation time
    /// only — existing sandboxes cannot have their mounts changed.
    pub mounts: Vec<VolumeMountConfig>,
    pub mounts_selected: usize,

    // ── Network tab (fields 0-2) ─────────────────────────────────────────
    pub disable_network: bool,
    /// Port mappings (host, guest), listed inline in the tab.
    pub ports: Vec<(u16, u16)>,
    pub ports_selected: usize,
    /// Network policy rules, listed inline in the tab. Applied at creation
    /// time only — the SDK has no API for changing policy afterwards.
    pub network_rules: Vec<NetworkRule>,
    pub network_rules_selected: usize,

    // ── Secrets tab (field 0) ────────────────────────────────────────────
    /// Secrets injected via the TLS proxy, listed inline in the tab.
    pub secrets: Vec<SecretConfig>,
    pub secrets_selected: usize,

    pub error: Option<String>,

    /// Inline directory browser for picking the workdir path.
    pub dir_picker: DirPicker,
    /// Popup for adding one port mapping.
    pub port_add: PortAddDialog,
    /// Popup for adding one environment variable.
    pub env_var_add: EnvVarAddDialog,
    /// Popup for adding one network policy rule.
    pub net_rule_add: NetRuleAddDialog,
    /// Popup for adding one volume mount.
    pub mount_add: MountAddDialog,
    /// Popup for adding one secret.
    pub secret_add: SecretAddDialog,
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

    /// Number of navigable form fields (excluding the Create button) on the
    /// currently active tab.
    pub fn form_field_count(&self) -> usize {
        match self.tab {
            DialogTab::Basic => 7, // name image cpus max_cpus memory max_memory workdir
            DialogTab::GuestOs => 5, // hostname user shell env_vars mounts
            DialogTab::Network => 3, // no_net ports net_rules
            DialogTab::Secrets => 1, // secrets
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
    /// or `None` when the focused field is a non-text widget (toggle, list,
    /// or sub-dialog).
    pub fn current_field_mut(&mut self) -> Option<&mut String> {
        if self.is_create_focused() {
            return None;
        }
        match self.tab {
            DialogTab::Basic => match self.field {
                0 => Some(&mut self.name),
                1 => Some(&mut self.image),
                2 => Some(&mut self.cpus),
                3 => Some(&mut self.max_cpus),
                4 => Some(&mut self.memory),
                5 => Some(&mut self.max_memory),
                6 => None, // workdir — managed via dir picker
                _ => None,
            },
            DialogTab::GuestOs => match self.field {
                0 => Some(&mut self.hostname),
                1 => Some(&mut self.user),
                2 => Some(&mut self.shell),
                _ => None, // env_vars / mounts — inline lists
            },
            DialogTab::Network | DialogTab::Secrets => None,
        }
    }

    /// True when the focused field only accepts ASCII digits.
    pub fn is_numeric_field(&self) -> bool {
        if self.is_create_focused() {
            return false;
        }
        self.tab == DialogTab::Basic && matches!(self.field, 2 | 3 | 4 | 5)
    }

    /// True when the focused field is a boolean toggle activated by Space.
    pub fn is_toggle_field(&self) -> bool {
        !self.is_create_focused() && self.tab == DialogTab::Network && self.field == 0
    }

    /// Returns the inline list identified by the currently focused field,
    /// if any, so key handling can route Up/Down/`a`/`d` to it.
    pub fn focused_list(&self) -> Option<ListField> {
        if self.is_create_focused() {
            return None;
        }
        match (self.tab, self.field) {
            (DialogTab::GuestOs, 3) => Some(ListField::EnvVars),
            (DialogTab::GuestOs, 4) => Some(ListField::Mounts),
            (DialogTab::Network, 1) => Some(ListField::Ports),
            (DialogTab::Network, 2) => Some(ListField::NetworkRules),
            (DialogTab::Secrets, 0) => Some(ListField::Secrets),
            _ => None,
        }
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

/// The command line prefilled in the "Exec" dialog: empty by default,
/// requiring the user to type a command.
pub const DEFAULT_EXEC_COMMAND: &str = "";

/// State of the "Exec" dialog: prompts for a command line to run inside a
/// running sandbox, then opens a new terminal window on the host that runs
/// it there natively via the `microsandbox` SDK (see
/// [`crate::terminal_launcher`]).
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
    /// Open the dialog for the given sandbox, prefilled with
    /// [`DEFAULT_EXEC_COMMAND`] (empty).
    pub fn open(sandbox_name: impl Into<String>) -> Self {
        Self {
            visible: true,
            sandbox_name: sandbox_name.into(),
            command: DEFAULT_EXEC_COMMAND.to_owned(),
            error: None,
        }
    }
}
