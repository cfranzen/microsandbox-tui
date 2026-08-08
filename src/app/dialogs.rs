//! State for the create-sandbox dialog, its sub-dialogs, the directory
//! picker, and the volumes management view.
//!
//! Nothing in this module drives the terminal directly — key handling lives
//! in [`super::keys`] and rendering lives in `crate::ui`. This module only
//! owns the plain-data state and small helper methods each dialog needs.

use crate::config::AppConfig;
use crate::sandbox::{
    NetRuleAction, NetRuleDestGroup, NetRuleDestKind, NetRuleDirection, NetRuleProtocol,
    NetworkRule, SecretConfig, ViolationActionChoice, VolumeInfo, VolumeMountConfig,
};

/// Which tab of the "create new sandbox" dialog is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DialogTab {
    #[default]
    Basic,
    GuestOs,
    Network,
    Dns,
    Tls,
    Security,
}

impl DialogTab {
    const ALL: [DialogTab; 6] = [
        DialogTab::Basic,
        DialogTab::GuestOs,
        DialogTab::Network,
        DialogTab::Dns,
        DialogTab::Tls,
        DialogTab::Security,
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
            DialogTab::Dns => "DNS",
            DialogTab::Tls => "TLS",
            DialogTab::Security => "Security",
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
    pub editing_index: Option<usize>,
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

    pub fn open_for_edit(index: usize, existing: (u16, u16)) -> Self {
        Self {
            visible: true,
            editing_index: Some(index),
            host_input: existing.0.to_string(),
            guest_input: existing.1.to_string(),
            add_field: 0,
            error: None,
        }
    }
}

/// Popup dialog for adding one environment variable (KEY=VALUE). Entries
/// already added are listed inline in the Guest OS tab; this popup only
/// handles adding a new one.
#[derive(Debug, Clone, Default)]
pub struct EnvVarAddDialog {
    pub visible: bool,
    pub editing_index: Option<usize>,
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

    pub fn open_for_edit(index: usize, existing: &(String, String)) -> Self {
        Self {
            visible: true,
            editing_index: Some(index),
            key_input: existing.0.clone(),
            value_input: existing.1.clone(),
            add_field: 0,
            error: None,
        }
    }
}

/// Popup dialog for adding one network policy rule. Entries already added
/// are listed inline in the Network tab; this popup only handles adding a
/// new one, and exposes the full expressiveness of the SDK's
/// `RuleBuilder`: direction, action, destination kind/value/group, protocol
/// filter, and an optional comma-separated list of guest-side ports/ranges.
#[derive(Debug, Clone)]
pub struct NetRuleAddDialog {
    pub visible: bool,
    pub editing_index: Option<usize>,
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
    /// "Apply to ports" input buffer: a comma-separated list of ports
    /// and/or port ranges, e.g. `"80,443,8000-9000"`.
    pub ports_input: String,
    /// Focused field: 0 action, 1 direction, 2 dest kind, 3 dest value/group,
    /// 4 protocols, 5 ports.
    pub add_field: usize,
    pub error: Option<String>,
}

impl Default for NetRuleAddDialog {
    fn default() -> Self {
        Self {
            visible: false,
            editing_index: None,
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

    pub fn open_for_edit(index: usize, existing: &NetworkRule) -> Self {
        Self {
            visible: true,
            editing_index: Some(index),
            direction: existing.direction,
            action: existing.action,
            dest_kind: existing.dest_kind,
            dest_input: existing.dest_value.clone(),
            dest_group: existing.dest_group,
            protocols: existing.protocols.clone(),
            protocol_cursor: 0,
            ports_input: existing
                .port_ranges
                .iter()
                .map(|(lo, hi)| if lo == hi { lo.to_string() } else { format!("{lo}-{hi}") })
                .collect::<Vec<_>>()
                .join(","),
            add_field: 0,
            error: None,
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
    pub editing_index: Option<usize>,
    /// Guest path input buffer.
    pub guest_input: String,
    /// Host path (Bind) or volume name (Named) input buffer.
    pub source_input: String,
    /// Which mount source kind is being configured.
    pub kind: MountKindChoice,
    /// Focused input index: 0 = kind, 1 = guest path, 2 = source.
    pub add_field: usize,
    pub dir_picker: DirPicker,
    pub available_volumes: Vec<VolumeInfo>,
    pub selected_volume: usize,
    pub new_volume_mode: bool,
    pub new_volume_name: String,
    pub new_volume_disk: bool,
    pub error: Option<String>,
}

impl MountAddDialog {
    pub fn open() -> Self {
        Self {
            visible: true,
            ..Default::default()
        }
    }

    pub fn open_for_edit(index: usize, existing: &VolumeMountConfig) -> Self {
        let (kind, source_input) = match &existing.source {
            crate::sandbox::MountSource::Bind(path) => (MountKindChoice::Bind, path.clone()),
            crate::sandbox::MountSource::Named(name) => (MountKindChoice::Named, name.clone()),
        };
        Self {
            visible: true,
            editing_index: Some(index),
            guest_input: existing.guest_path.clone(),
            source_input,
            kind,
            add_field: 0,
            selected_volume: 0,
            error: None,
            ..Default::default()
        }
    }

    pub fn sync_selected_volume_from_source(&mut self) {
        if self.available_volumes.is_empty() {
            self.selected_volume = 0;
            return;
        }
        if let Some(idx) = self
            .available_volumes
            .iter()
            .position(|vol| vol.name == self.source_input)
        {
            self.selected_volume = idx;
        } else if self.selected_volume >= self.available_volumes.len() {
            self.selected_volume = self.available_volumes.len() - 1;
        }
        if let Some(vol) = self.available_volumes.get(self.selected_volume) {
            self.source_input = vol.name.clone();
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
    pub editing_index: Option<usize>,
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
            editing_index: None,
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

    pub fn open_for_edit(index: usize, existing: &SecretConfig) -> Self {
        let hosts_input = existing
            .allowed_hosts
            .iter()
            .map(|host| match host.kind {
                crate::sandbox::SecretHostKind::Any => "*".to_owned(),
                _ => host.value.clone(),
            })
            .collect::<Vec<_>>()
            .join(", ");
        Self {
            visible: true,
            editing_index: Some(index),
            env_input: existing.env_var.clone(),
            value_input: existing.value.clone(),
            hosts_input,
            inject_headers: existing.inject_headers,
            inject_basic_auth: existing.inject_basic_auth,
            inject_query: existing.inject_query,
            inject_body: existing.inject_body,
            require_tls_identity: existing.require_tls_identity,
            add_field: 0,
            error: None,
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
    pub list_edit_mode: bool,

    // ── Basic tab (fields 0-6) ───────────────────────────────────────────
    pub name: String,
    pub image: String,
    pub cpus: String,
    pub max_cpus: String,
    pub memory: String,
    pub max_memory: String,
    pub workdir: String,

    // ── Guest OS tab (fields 0-3) ────────────────────────────────────────
    pub hostname: String,
    pub shell: String,
    /// Environment variables (key, value), listed inline in the tab.
    pub env_vars: Vec<(String, String)>,
    pub env_vars_selected: usize,
    /// Volume mounts, listed inline in the tab. Applied at creation time
    /// only — existing sandboxes cannot have their mounts changed.
    pub mounts: Vec<VolumeMountConfig>,
    pub mounts_selected: usize,

    // ── Network tab ───────────────────────────────────────────────────────
    pub disable_network: bool,
    pub default_ingress_action: NetRuleAction,
    pub default_egress_action: NetRuleAction,
    /// Port mappings (host, guest), listed inline in the tab.
    pub ports: Vec<(u16, u16)>,
    pub ports_selected: usize,
    /// Network policy rules, listed inline in the tab. Applied at creation
    /// time only — the SDK has no API for changing policy afterwards.
    pub network_rules: Vec<NetworkRule>,
    pub network_rules_selected: usize,
    pub dns_nameservers: String,
    pub dns_query_timeout_ms: String,
    pub dns_rebind_protection: bool,

    // ── Security tab ──────────────────────────────────────────────────────
    pub user: String,
    pub tls_enabled: bool,
    pub tls_bypass_patterns: String,
    pub tls_intercepted_ports: String,
    pub tls_verify_upstream: bool,
    pub tls_block_quic: bool,
    pub violation_action: ViolationActionChoice,
    pub violation_passthrough_hosts: String,
    pub violation_passthrough_patterns: String,
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
    const NETWORK_FIELDS: [usize; 5] = [0, 1, 2, 3, 4];
    const DNS_FIELDS: [usize; 3] = [0, 1, 2];
    const TLS_FIELDS: [usize; 5] = [0, 1, 2, 3, 4];

    pub fn open() -> Self {
        Self {
            visible: true,
            image: "alpine".into(),
            cpus: "1".into(),
            memory: "512".into(),
            shell: "/bin/sh".into(),
            default_ingress_action: NetRuleAction::Deny,
            default_egress_action: NetRuleAction::Deny,
            dns_query_timeout_ms: "5000".into(),
            dns_rebind_protection: true,
            tls_intercepted_ports: "443".into(),
            tls_verify_upstream: true,
            tls_block_quic: true,
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
            DialogTab::GuestOs => 4,
            DialogTab::Network => 5,
            DialogTab::Dns => 3,
            DialogTab::Tls => 5,
            DialogTab::Security => 5,
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
        self.field = self.next_enabled_field_index(self.field, true);
        self.list_edit_mode = false;
    }

    pub fn prev_field(&mut self) {
        self.field = self.next_enabled_field_index(self.field, false);
        self.list_edit_mode = false;
    }

    pub fn switch_tab(&mut self, tab: DialogTab) {
        self.tab = tab;
        self.field = 0;
        self.list_edit_mode = false;
        self.error = None;
    }

    /// True when `tab` cannot currently be selected — the DNS and TLS tabs
    /// are both meaningless (and gated off) while network access itself is
    /// disabled.
    pub fn is_tab_disabled(&self, tab: DialogTab) -> bool {
        self.disable_network && matches!(tab, DialogTab::Dns | DialogTab::Tls)
    }

    /// Returns the next tab in the given direction that isn't disabled via
    /// [`Self::is_tab_disabled`], skipping over any that are.
    pub fn next_enabled_tab(&self, forward: bool) -> DialogTab {
        let mut tab = self.tab;
        for _ in 0..DialogTab::ALL.len() {
            tab = if forward { tab.next() } else { tab.prev() };
            if !self.is_tab_disabled(tab) {
                return tab;
            }
        }
        self.tab
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
                1 => Some(&mut self.shell),
                _ => None,
            },
            DialogTab::Network => match self.field {
                _ => None,
            },
            DialogTab::Dns => match self.field {
                0 => Some(&mut self.dns_nameservers),
                1 => Some(&mut self.dns_query_timeout_ms),
                _ => None,
            },
            DialogTab::Tls => match self.field {
                1 => Some(&mut self.tls_bypass_patterns),
                2 => Some(&mut self.tls_intercepted_ports),
                _ => None,
            },
            DialogTab::Security => match self.field {
                0 => Some(&mut self.user),
                2 => Some(&mut self.violation_passthrough_hosts),
                3 => Some(&mut self.violation_passthrough_patterns),
                _ => None,
            },
        }
    }

    /// True when the focused field only accepts ASCII digits.
    pub fn is_numeric_field(&self) -> bool {
        if self.is_create_focused() {
            return false;
        }
        (self.tab == DialogTab::Basic && matches!(self.field, 2 | 3 | 4 | 5))
            || (self.tab == DialogTab::Dns && self.field == 1)
    }

    /// True when the focused field is a boolean toggle or a fixed-choice
    /// cycle field, both activated by Space.
    pub fn is_toggle_field(&self) -> bool {
        !self.is_create_focused()
            && matches!(
                (self.tab, self.field),
                (DialogTab::Network, 0 | 1 | 2)
                    | (DialogTab::Dns, 2)
                    | (DialogTab::Tls, 0 | 3 | 4)
                    | (DialogTab::Security, 1)
            )
    }

    /// Returns the inline list identified by the currently focused field,
    /// if any, so key handling can route Up/Down/`a`/`d` to it.
    pub fn focused_list(&self) -> Option<ListField> {
        if self.is_create_focused() {
            return None;
        }
        match (self.tab, self.field) {
            (DialogTab::GuestOs, 2) => Some(ListField::EnvVars),
            (DialogTab::GuestOs, 3) => Some(ListField::Mounts),
            (DialogTab::Network, 3) => Some(ListField::Ports),
            (DialogTab::Network, 4) => Some(ListField::NetworkRules),
            (DialogTab::Security, 4) => Some(ListField::Secrets),
            _ => None,
        }
    }

    pub fn field_disabled_by_network(&self, tab: DialogTab, field_idx: usize) -> bool {
        if !self.disable_network {
            return false;
        }
        match tab {
            DialogTab::Network => Self::NETWORK_FIELDS.contains(&field_idx) && field_idx != 0,
            DialogTab::Dns => Self::DNS_FIELDS.contains(&field_idx),
            DialogTab::Tls => Self::TLS_FIELDS.contains(&field_idx),
            _ => false,
        }
    }

    pub fn next_enabled_field_index(&self, current: usize, forward: bool) -> usize {
        let count = self.field_count();
        let mut idx = current;
        for _ in 0..count {
            idx = if forward {
                (idx + 1) % count
            } else if idx == 0 {
                count - 1
            } else {
                idx - 1
            };
            if idx == self.form_field_count() || !self.field_disabled_by_network(self.tab, idx) {
                return idx;
            }
        }
        current
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
