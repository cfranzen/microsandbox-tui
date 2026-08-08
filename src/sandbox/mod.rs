//! Sandbox management: wraps the microsandbox SDK into async operations
//! that feed the TUI's state machine.

use anyhow::Result;
use chrono::{DateTime, Utc};
use futures::Stream;
use microsandbox::logs::{LogStreamOptions, LogStreamStart};
use microsandbox::sandbox::{FsEntryKind, LogEntry, LogOptions, LogSource, MAX_SANDBOX_LIST_LIMIT};
use microsandbox::{MicrosandboxError, NetworkPolicy, Sandbox, SandboxMetrics, Volume, VolumeKind};
use microsandbox_network::policy::DestinationGroup as SdkDestGroup;
use microsandbox_types::VolumeMount;

// Re-export for use in other modules
pub use microsandbox::sandbox::SandboxStatus;

//--------------------------------------------------------------------------------------------------
// Types
//--------------------------------------------------------------------------------------------------

/// Local snapshot of a sandbox's state used to drive the TUI.
#[derive(Debug, Clone)]
pub struct SandboxInfo {
    pub name: String,
    pub status: SandboxStatus,
    pub image: String,
    pub cpus: u8,
    pub memory_mib: u32,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    /// The sandbox's working directory, when known.
    pub workdir: Option<String>,
}

/// A point-in-time metrics snapshot for a sandbox.
#[derive(Debug, Clone, Default)]
pub struct MetricsSnapshot {
    pub cpu_percent: f64,
    pub memory_bytes: u64,
    pub disk_read_bytes: u64,
    pub disk_write_bytes: u64,
    pub net_rx_bytes: u64,
    pub net_tx_bytes: u64,
    /// Guest-visible OCI upper (writable overlay) filesystem used bytes, when reported.
    pub disk_used_bytes: Option<u64>,
    /// Guest-visible OCI upper (writable overlay) filesystem free bytes, when reported.
    pub disk_free_bytes: Option<u64>,
    pub uptime_secs: u64,
}

/// A filesystem entry inside a running sandbox.
#[derive(Debug, Clone)]
pub struct FsEntry {
    pub path: String,
    pub kind: LocalFsEntryKind,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalFsEntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

/// Whether a [`NetworkRule`] permits or blocks matching traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NetRuleAction {
    #[default]
    Allow,
    Deny,
}

impl NetRuleAction {
    pub fn label(self) -> &'static str {
        match self {
            NetRuleAction::Allow => "ALLOW",
            NetRuleAction::Deny => "DENY",
        }
    }

    /// Cycle to the next value, wrapping around.
    pub fn cycle(self) -> Self {
        match self {
            NetRuleAction::Allow => NetRuleAction::Deny,
            NetRuleAction::Deny => NetRuleAction::Allow,
        }
    }
}

/// Traffic direction a [`NetworkRule`] applies to.
///
/// `Any` applies the rule in both the egress and ingress evaluators (see
/// the SDK's `RuleBuilder::any()`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NetRuleDirection {
    #[default]
    Egress,
    Ingress,
    Any,
}

impl NetRuleDirection {
    pub fn label(self) -> &'static str {
        match self {
            NetRuleDirection::Egress => "EGRESS",
            NetRuleDirection::Ingress => "INGRESS",
            NetRuleDirection::Any => "ANY",
        }
    }

    /// Cycle to the next value, wrapping around.
    pub fn cycle(self) -> Self {
        match self {
            NetRuleDirection::Egress => NetRuleDirection::Ingress,
            NetRuleDirection::Ingress => NetRuleDirection::Any,
            NetRuleDirection::Any => NetRuleDirection::Egress,
        }
    }
}

/// A transport/network-layer protocol filter for a [`NetworkRule`].
///
/// ICMP protocols are egress-only in the SDK: attaching one to an
/// `Ingress`-direction rule fails policy validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetRuleProtocol {
    Tcp,
    Udp,
    Icmpv4,
    Icmpv6,
}

impl NetRuleProtocol {
    pub const ALL: [NetRuleProtocol; 4] = [
        NetRuleProtocol::Tcp,
        NetRuleProtocol::Udp,
        NetRuleProtocol::Icmpv4,
        NetRuleProtocol::Icmpv6,
    ];

    pub fn label(self) -> &'static str {
        match self {
            NetRuleProtocol::Tcp => "TCP",
            NetRuleProtocol::Udp => "UDP",
            NetRuleProtocol::Icmpv4 => "ICMPv4",
            NetRuleProtocol::Icmpv6 => "ICMPv6",
        }
    }

    /// True for the two ICMP variants, which the SDK only allows on
    /// egress (or "any"-direction, egress side) rules.
    pub fn is_icmp(self) -> bool {
        matches!(self, NetRuleProtocol::Icmpv4 | NetRuleProtocol::Icmpv6)
    }
}

/// One of the SDK's pre-defined destination groups (see
/// `microsandbox_network::policy::DestinationGroup`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NetRuleDestGroup {
    #[default]
    Public,
    Loopback,
    Private,
    LinkLocal,
    Metadata,
    Multicast,
    Host,
}

impl NetRuleDestGroup {
    pub const ALL: [NetRuleDestGroup; 7] = [
        NetRuleDestGroup::Public,
        NetRuleDestGroup::Loopback,
        NetRuleDestGroup::Private,
        NetRuleDestGroup::LinkLocal,
        NetRuleDestGroup::Metadata,
        NetRuleDestGroup::Multicast,
        NetRuleDestGroup::Host,
    ];

    pub fn label(self) -> &'static str {
        match self {
            NetRuleDestGroup::Public => "Public",
            NetRuleDestGroup::Loopback => "Loopback",
            NetRuleDestGroup::Private => "Private",
            NetRuleDestGroup::LinkLocal => "Link-Local",
            NetRuleDestGroup::Metadata => "Metadata",
            NetRuleDestGroup::Multicast => "Multicast",
            NetRuleDestGroup::Host => "Host",
        }
    }

    /// Cycle to the next value, wrapping around.
    pub fn cycle(self) -> Self {
        let idx = Self::ALL.iter().position(|g| *g == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    fn to_sdk(self) -> SdkDestGroup {
        match self {
            NetRuleDestGroup::Public => SdkDestGroup::Public,
            NetRuleDestGroup::Loopback => SdkDestGroup::Loopback,
            NetRuleDestGroup::Private => SdkDestGroup::Private,
            NetRuleDestGroup::LinkLocal => SdkDestGroup::LinkLocal,
            NetRuleDestGroup::Metadata => SdkDestGroup::Metadata,
            NetRuleDestGroup::Multicast => SdkDestGroup::Multicast,
            NetRuleDestGroup::Host => SdkDestGroup::Host,
        }
    }
}

/// What kind of destination a [`NetworkRule`] matches against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NetRuleDestKind {
    /// Matches any destination.
    #[default]
    Any,
    /// A single IP address.
    Ip,
    /// A CIDR block.
    Cidr,
    /// An exact domain name (matched via the DNS-resolved-hostname cache).
    Domain,
    /// A domain and all of its subdomains.
    DomainSuffix,
    /// A pre-defined destination group (see [`NetRuleDestGroup`]).
    Group,
}

impl NetRuleDestKind {
    pub const ALL: [NetRuleDestKind; 6] = [
        NetRuleDestKind::Any,
        NetRuleDestKind::Ip,
        NetRuleDestKind::Cidr,
        NetRuleDestKind::Domain,
        NetRuleDestKind::DomainSuffix,
        NetRuleDestKind::Group,
    ];

    pub fn label(self) -> &'static str {
        match self {
            NetRuleDestKind::Any => "Any",
            NetRuleDestKind::Ip => "IP",
            NetRuleDestKind::Cidr => "CIDR",
            NetRuleDestKind::Domain => "Domain",
            NetRuleDestKind::DomainSuffix => "Domain Suffix",
            NetRuleDestKind::Group => "Group",
        }
    }

    /// Cycle to the next value, wrapping around.
    pub fn cycle(self) -> Self {
        let idx = Self::ALL.iter().position(|k| *k == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    /// True when this kind needs a free-text value (IP/CIDR/domain name).
    pub fn needs_text_value(self) -> bool {
        matches!(
            self,
            NetRuleDestKind::Ip
                | NetRuleDestKind::Cidr
                | NetRuleDestKind::Domain
                | NetRuleDestKind::DomainSuffix
        )
    }
}

/// A single network policy rule configured at sandbox-creation time (the
/// SDK does not support modifying network policy on an already created
/// sandbox — see [`create_sandbox`]'s use of this type).
///
/// Mirrors the full expressiveness of the SDK's `RuleBuilder`: any
/// direction, any destination kind (including pre-defined groups, exact
/// IPs/CIDRs, and domain/domain-suffix matches), an optional protocol
/// filter, and an optional guest-side port or port range.
#[derive(Debug, Clone, PartialEq)]
pub struct NetworkRule {
    pub direction: NetRuleDirection,
    pub action: NetRuleAction,
    pub dest_kind: NetRuleDestKind,
    /// Free-text destination value, used when `dest_kind` is `Ip`, `Cidr`,
    /// `Domain`, or `DomainSuffix`.
    pub dest_value: String,
    /// Destination group, used when `dest_kind` is `Group`.
    pub dest_group: NetRuleDestGroup,
    /// Protocol filter; empty means "any protocol".
    pub protocols: Vec<NetRuleProtocol>,
    /// Guest-side port or port range filter; `None` means "any port".
    pub port_range: Option<(u16, u16)>,
}

impl NetworkRule {
    /// One-line human-readable summary shown in the create-dialog's
    /// network-rules list.
    pub fn summary(&self) -> String {
        let dest = match self.dest_kind {
            NetRuleDestKind::Any => "any".to_owned(),
            NetRuleDestKind::Group => self.dest_group.label().to_owned(),
            _ => self.dest_value.clone(),
        };
        let proto = if self.protocols.is_empty() {
            "any proto".to_owned()
        } else {
            self.protocols
                .iter()
                .map(|p| p.label())
                .collect::<Vec<_>>()
                .join("+")
        };
        let ports = match self.port_range {
            None => "any port".to_owned(),
            Some((lo, hi)) if lo == hi => format!("port {lo}"),
            Some((lo, hi)) => format!("ports {lo}-{hi}"),
        };
        format!(
            "{} {} {} [{proto}, {ports}]",
            self.direction.label(),
            self.action.label(),
            dest
        )
    }
}

/// Where a volume mount's data comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MountSource {
    /// Bind-mount a host directory.
    Bind(String),
    /// Mount a pre-existing named volume (see [`Volume`]).
    Named(String),
}

/// A single guest-path mount configured at sandbox-creation time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeMountConfig {
    pub guest_path: String,
    pub source: MountSource,
}

/// Convert a host filesystem path into the corresponding guest path used
/// when bind-mounting it into a sandbox at "the same" location.
///
/// POSIX-style host paths (Linux/macOS) are already valid guest paths and
/// are returned unchanged (aside from stripping a trailing slash). Windows
/// drive-letter paths (`D:\foo\bar` or `D:/foo/bar`) have no guest-side
/// equivalent, so they're rewritten to the POSIX-style form Git-Bash/WSL
/// conventions use: the drive letter becomes a lowercase top-level
/// directory, e.g. `D:\foo\bar` -> `/d/foo/bar`.
pub fn host_path_to_guest_path(host_path: &str) -> String {
    let normalized = host_path.replace('\\', "/");
    let bytes = normalized.as_bytes();

    // Windows drive-letter path, e.g. `D:\foo\bar` or `D:/foo/bar`.
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        let drive = (bytes[0] as char).to_ascii_lowercase();
        let rest = normalized[2..]
            .trim_start_matches('/')
            .trim_end_matches('/');
        return if rest.is_empty() {
            format!("/{drive}")
        } else {
            format!("/{drive}/{rest}")
        };
    }

    // Already POSIX-style; strip a trailing slash, but keep a bare root.
    if normalized.len() > 1 {
        normalized.trim_end_matches('/').to_string()
    } else {
        normalized
    }
}

/// Given a sandbox's guest workdir and its configured mounts, find the host
/// path that's bind-mounted at (or as an ancestor of) that guest path, so the
/// TUI can display the working directory as it appears on the host rather
/// than inside the guest.
///
/// Falls back to the raw guest path unchanged when no bind mount covers it
/// (e.g. the workdir lives on the root OCI filesystem with no host backing).
fn resolve_workdir_host_path(workdir: &str, mounts: &[VolumeMount]) -> String {
    let normalized_workdir = workdir.trim_end_matches('/');

    for mount in mounts {
        if let VolumeMount::Bind { host, guest, .. } = mount {
            let normalized_guest = guest.trim_end_matches('/');
            if normalized_workdir == normalized_guest {
                return host.display().to_string();
            }
            // The workdir is a subdirectory of a bind-mounted host directory.
            if let Some(rest) = normalized_workdir.strip_prefix(normalized_guest) {
                if let Some(rest) = rest.strip_prefix('/') {
                    let mut host_path = host.display().to_string();
                    host_path.push('/');
                    host_path.push_str(rest);
                    return host_path;
                }
            }
        }
    }

    workdir.to_owned()
}

/// Summary of a named volume, as shown in the Volumes view.
#[derive(Debug, Clone, PartialEq)]
pub struct VolumeInfo {
    pub name: String,
    pub kind: VolumeKind,
    pub quota_mib: Option<u32>,
    pub used_bytes: u64,
}

/// Which kind of host pattern an allowed/passthrough host entry uses (see
/// `microsandbox_network::secrets::config::HostPattern`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SecretHostKind {
    /// Exact hostname match.
    #[default]
    Exact,
    /// Wildcard suffix match, e.g. `*.example.com`.
    Wildcard,
    /// Matches every host. Dangerous — disables host-based protection.
    Any,
}

/// One allowed-host entry for a [`SecretConfig`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretHostPattern {
    pub kind: SecretHostKind,
    /// Hostname or wildcard pattern (e.g. `api.openai.com` or
    /// `*.googleapis.com`). Empty when `kind` is `Any`.
    pub value: String,
}

/// A single secret entry configured at sandbox-creation time, mirroring the
/// SDK's `SecretBuilder`: an environment variable that exposes a
/// placeholder inside the guest, the real value it's substituted for, the
/// hosts allowed to receive that value, and the injection scopes the TLS
/// proxy substitutes it in.
///
/// Adding any secret automatically enables TLS interception for the
/// sandbox (matching `SandboxBuilder::secret`'s behaviour).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretConfig {
    /// Environment variable name the guest sees the placeholder as.
    pub env_var: String,
    /// The real secret value, only ever revealed to allowed hosts via the
    /// TLS proxy — never exposed to the guest.
    pub value: String,
    /// Hosts allowed to receive the real value. At least one is required.
    pub allowed_hosts: Vec<SecretHostPattern>,
    /// Substitute in HTTP headers (default: true).
    pub inject_headers: bool,
    /// Substitute inside decoded HTTP Basic Auth credentials (default: true).
    pub inject_basic_auth: bool,
    /// Substitute in URL query parameters (default: false).
    pub inject_query: bool,
    /// Substitute in HTTP/1 request bodies (default: false).
    pub inject_body: bool,
    /// Require a verified TLS identity before substituting (default: true).
    pub require_tls_identity: bool,
}

impl SecretConfig {
    /// One-line human-readable summary shown in the create-dialog's
    /// secrets list.
    pub fn summary(&self) -> String {
        let hosts = self
            .allowed_hosts
            .iter()
            .map(|h| match h.kind {
                SecretHostKind::Any => "*".to_owned(),
                _ => h.value.clone(),
            })
            .collect::<Vec<_>>()
            .join(",");
        format!("{}=****** -> {hosts}", self.env_var)
    }
}

/// All parameters for creating a new sandbox via the TUI dialog.
#[derive(Debug, Clone)]
pub struct CreateConfig {
    pub name: String,
    pub image: String,
    pub cpus: u8,
    pub memory_mib: u32,
    pub ports: Vec<(u16, u16)>,
    pub env_vars: Vec<(String, String)>,
    pub hostname: Option<String>,
    pub workdir: Option<String>,
    pub user: Option<String>,
    pub shell: Option<String>,
    pub max_cpus: Option<u8>,
    pub max_memory_mib: Option<u32>,
    pub disable_network: bool,
    /// Network policy rules applied at creation time. Ignored (with
    /// `disable_network` taking precedence) when empty.
    pub network_rules: Vec<NetworkRule>,
    /// Volume mounts applied at creation time. Existing sandboxes cannot
    /// have their mounts changed post-creation per the current SDK.
    pub mounts: Vec<VolumeMountConfig>,
    /// Secrets injected at creation time via the TLS proxy.
    pub secrets: Vec<SecretConfig>,
}

//--------------------------------------------------------------------------------------------------
// List
//--------------------------------------------------------------------------------------------------

/// Retrieve all sandboxes from the local backend.
///
/// `Sandbox::list()` returns a single (possibly partial) [`SandboxPage`], so
/// we page through with `list_with` (using the max page size) until the
/// backend stops returning a `next_cursor`, collecting every sandbox.
pub async fn list_sandboxes() -> Result<Vec<SandboxInfo>> {
    let mut infos = Vec::new();
    let mut cursor: Option<String> = None;

    loop {
        let page = Sandbox::list_with(|list| {
            let list = list.limit(MAX_SANDBOX_LIST_LIMIT);
            match cursor.take() {
                Some(c) => list.cursor(c),
                None => list,
            }
        })
        .await?;

        for h in page.sandboxes {
            let (image, cpus, memory_mib, workdir) = if let Ok(cfg) = h.config() {
                let image = cfg
                    .spec
                    .image
                    .oci_reference()
                    .unwrap_or("(bind/disk)")
                    .to_owned();
                let workdir = cfg
                    .spec
                    .runtime
                    .workdir
                    .as_deref()
                    .map(|w| resolve_workdir_host_path(w, &cfg.spec.mounts));
                (
                    image,
                    cfg.spec.resources.cpus,
                    cfg.spec.resources.memory_mib,
                    workdir,
                )
            } else {
                ("—".into(), 1, 512, None)
            };

            infos.push(SandboxInfo {
                name: h.name().to_owned(),
                status: h.status_snapshot(),
                image,
                cpus,
                memory_mib,
                created_at: h.created_at(),
                updated_at: h.updated_at(),
                workdir,
            });
        }

        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }

    Ok(infos)
}

//--------------------------------------------------------------------------------------------------
// Lifecycle
//--------------------------------------------------------------------------------------------------

/// Start a stopped sandbox in detached mode so it outlives the TUI.
pub async fn start_sandbox(name: &str) -> Result<()> {
    let handle = Sandbox::get(name).await?;
    let sb = handle.start_detached().await?;
    sb.detach().await;
    Ok(())
}

/// Gracefully stop a running sandbox.
pub async fn stop_sandbox(name: &str) -> Result<()> {
    let handle = Sandbox::get(name).await?;
    handle.stop().await?;
    Ok(())
}

/// Terminate a sandbox immediately (forceful stop).
pub async fn terminate_sandbox(name: &str) -> Result<()> {
    let handle = Sandbox::get(name).await?;
    handle.kill().await?;
    Ok(())
}

/// Remove a stopped sandbox and all its state.
pub async fn remove_sandbox(name: &str) -> Result<()> {
    let handle = Sandbox::get(name).await?;
    handle.remove().await?;
    Ok(())
}

/// Build a [`NetworkPolicy`] from the user-configured rule list.
///
/// Starts from an allow-all default (matching the sandbox's normal
/// networking behaviour) and layers explicit rules on top, each evaluated
/// first-match-wins, exactly mirroring the SDK's `NetworkPolicyBuilder`.
/// Propagates the first [`microsandbox_network::policy::BuildError`]
/// encountered (e.g. an ICMP protocol on an ingress-direction rule) as an
/// `anyhow` error rather than silently discarding the user's rules.
fn build_network_policy(rules: &[NetworkRule]) -> Result<NetworkPolicy> {
    let mut builder = NetworkPolicy::builder().default_allow();
    for rule in rules {
        let rule = rule.clone();
        builder = builder.rule(move |r| {
            match rule.direction {
                NetRuleDirection::Egress => {
                    r.egress();
                }
                NetRuleDirection::Ingress => {
                    r.ingress();
                }
                NetRuleDirection::Any => {
                    r.any();
                }
            }
            for proto in &rule.protocols {
                match proto {
                    NetRuleProtocol::Tcp => {
                        r.tcp();
                    }
                    NetRuleProtocol::Udp => {
                        r.udp();
                    }
                    NetRuleProtocol::Icmpv4 => {
                        r.icmpv4();
                    }
                    NetRuleProtocol::Icmpv6 => {
                        r.icmpv6();
                    }
                }
            }
            if let Some((lo, hi)) = rule.port_range {
                if lo == hi {
                    r.port(lo);
                } else {
                    r.port_range(lo, hi);
                }
            }
            let dest = match rule.action {
                NetRuleAction::Allow => r.allow(),
                NetRuleAction::Deny => r.deny(),
            };
            match rule.dest_kind {
                NetRuleDestKind::Any => dest.any(),
                NetRuleDestKind::Ip => dest.ip(rule.dest_value.clone()),
                NetRuleDestKind::Cidr => dest.cidr(rule.dest_value.clone()),
                NetRuleDestKind::Domain => dest.domain(rule.dest_value.clone()),
                NetRuleDestKind::DomainSuffix => dest.domain_suffix(rule.dest_value.clone()),
                NetRuleDestKind::Group => dest.group(rule.dest_group.to_sdk()),
            }
        });
    }
    builder
        .build()
        .map_err(|e| anyhow::anyhow!("network rule error: {e}"))
}

/// Create and immediately detach a new sandbox using the given configuration.
pub async fn create_sandbox(cfg: &CreateConfig) -> Result<()> {
    let mut builder = Sandbox::builder(&cfg.name)
        .image(cfg.image.as_str())
        .cpus(cfg.cpus)
        .memory(cfg.memory_mib)
        .detached(true);

    for &(host_port, guest_port) in &cfg.ports {
        builder = builder.port(host_port, guest_port);
    }
    for (key, value) in &cfg.env_vars {
        builder = builder.env(key.as_str(), value.as_str());
    }
    if let Some(ref v) = cfg.hostname {
        builder = builder.hostname(v.as_str());
    }
    if let Some(ref v) = cfg.workdir {
        builder = builder.workdir(v.as_str());
    }
    if let Some(ref v) = cfg.user {
        builder = builder.user(v.as_str());
    }
    if let Some(ref v) = cfg.shell {
        builder = builder.shell(v.as_str());
    }
    if let Some(v) = cfg.max_cpus {
        builder = builder.max_cpus(v);
    }
    if let Some(v) = cfg.max_memory_mib {
        builder = builder.max_memory(v);
    }
    if cfg.disable_network {
        builder = builder.disable_network();
    } else if !cfg.network_rules.is_empty() {
        let policy = build_network_policy(&cfg.network_rules)?;
        builder = builder.network(|n| n.policy(policy));
    }

    for mount in &cfg.mounts {
        let guest_path = mount.guest_path.clone();
        builder = match &mount.source {
            MountSource::Bind(host) => {
                let host = host.clone();
                builder.volume(guest_path, |m| m.bind(host))
            }
            MountSource::Named(name) => {
                let name = name.clone();
                builder.volume(guest_path, |m| m.named(name))
            }
        };
    }

    for secret in &cfg.secrets {
        let secret = secret.clone();
        builder = builder.secret(move |s| {
            let mut s = s.env(secret.env_var).value(secret.value);
            for host in &secret.allowed_hosts {
                s = match host.kind {
                    SecretHostKind::Exact => s.allow_host(host.value.clone()),
                    SecretHostKind::Wildcard => s.allow_host_pattern(host.value.clone()),
                    SecretHostKind::Any => s.allow_any_host_dangerous(true),
                };
            }
            s.inject_headers(secret.inject_headers)
                .inject_basic_auth(secret.inject_basic_auth)
                .inject_query(secret.inject_query)
                .inject_body(secret.inject_body)
                .require_tls_identity(secret.require_tls_identity)
        });
    }

    let sb = builder.create().await?;
    sb.detach().await;
    Ok(())
}

//--------------------------------------------------------------------------------------------------
// Logs
//--------------------------------------------------------------------------------------------------

/// Read recent log entries for a sandbox (works for running and stopped).
pub async fn read_logs(name: &str, tail: Option<usize>) -> Result<Vec<LogEntry>> {
    let handle = Sandbox::get(name).await?;
    let entries = handle
        .logs(&LogOptions {
            tail,
            sources: vec![
                LogSource::Stdout,
                LogSource::Stderr,
                LogSource::Output,
                LogSource::System,
            ],
            ..Default::default()
        })
        .await?;
    Ok(entries)
}

/// Open a live, continuously-following log stream for a running sandbox.
///
/// The stream starts from "now" (it does not replay history — callers should
/// pair this with an initial [`read_logs`] call for backfill) and yields new
/// entries as they are written. Returns `None` when the sandbox is stopped
/// or unreachable, in which case callers should fall back to [`read_logs`].
pub async fn open_log_stream(
    name: &str,
) -> Result<Option<impl Stream<Item = Result<LogEntry, MicrosandboxError>> + Send + 'static>> {
    let handle = match Sandbox::get(name).await {
        Ok(h) => h,
        Err(_) => return Ok(None),
    };

    if handle.status_snapshot() != SandboxStatus::Running {
        return Ok(None);
    }

    let stream = handle
        .log_stream(&LogStreamOptions {
            sources: vec![
                LogSource::Stdout,
                LogSource::Stderr,
                LogSource::Output,
                LogSource::System,
            ],
            start: LogStreamStart::Since(Utc::now()),
            until: None,
            follow: true,
        })
        .await?;

    Ok(Some(stream))
}

//--------------------------------------------------------------------------------------------------
// Metrics
//--------------------------------------------------------------------------------------------------

/// Fetch a single metrics snapshot for a running sandbox.
/// Returns `None` when the sandbox is stopped or unreachable.
pub async fn fetch_metrics(name: &str) -> Result<Option<MetricsSnapshot>> {
    let handle = match Sandbox::get(name).await {
        Ok(h) => h,
        Err(_) => return Ok(None),
    };

    if handle.status_snapshot() != SandboxStatus::Running {
        return Ok(None);
    }

    let m: SandboxMetrics = match handle.metrics().await {
        Ok(m) => m,
        Err(_) => return Ok(None),
    };

    Ok(Some(MetricsSnapshot {
        cpu_percent: m.cpu_percent as f64,
        memory_bytes: m.memory_bytes,
        disk_read_bytes: m.disk_read_bytes,
        disk_write_bytes: m.disk_write_bytes,
        net_rx_bytes: m.net_rx_bytes,
        net_tx_bytes: m.net_tx_bytes,
        disk_used_bytes: m.upper_used_bytes,
        disk_free_bytes: m.upper_free_bytes,
        uptime_secs: m.uptime.as_secs(),
    }))
}

//--------------------------------------------------------------------------------------------------
// Filesystem
//--------------------------------------------------------------------------------------------------

/// List the contents of a directory inside a running sandbox.
/// Returns `None` when the sandbox is stopped or unreachable.
pub async fn list_fs(name: &str, path: &str) -> Result<Option<Vec<FsEntry>>> {
    let handle = match Sandbox::get(name).await {
        Ok(h) => h,
        Err(_) => return Ok(None),
    };

    if handle.status_snapshot() != SandboxStatus::Running {
        return Ok(None);
    }

    let sb = match handle.connect().await {
        Ok(sb) => sb,
        Err(_) => return Ok(None),
    };

    let entries = match sb.fs().list(path).await {
        Ok(e) => e,
        Err(_) => return Ok(None),
    };

    let result = entries
        .into_iter()
        .map(|e| FsEntry {
            path: e.path,
            kind: match e.kind {
                FsEntryKind::File => LocalFsEntryKind::File,
                FsEntryKind::Directory => LocalFsEntryKind::Directory,
                FsEntryKind::Symlink => LocalFsEntryKind::Symlink,
                _ => LocalFsEntryKind::Other,
            },
            size: e.size,
        })
        .collect();

    Ok(Some(result))
}

//--------------------------------------------------------------------------------------------------
// Volumes
//--------------------------------------------------------------------------------------------------

/// List all named volumes known to the local backend.
pub async fn list_volumes() -> Result<Vec<VolumeInfo>> {
    let handles = Volume::list().await?;
    Ok(handles
        .into_iter()
        .map(|h| VolumeInfo {
            name: h.name().to_owned(),
            kind: h.kind(),
            quota_mib: h.quota_mib(),
            used_bytes: h.used_bytes(),
        })
        .collect())
}

/// Create a new named volume.
pub async fn create_volume(name: &str, disk: bool, quota_mib: Option<u32>) -> Result<()> {
    let mut builder = Volume::builder(name);
    builder = if disk {
        builder.disk()
    } else {
        builder.directory()
    };
    if let Some(q) = quota_mib {
        builder = builder.quota(q);
    }
    builder.create().await?;
    Ok(())
}

/// Remove a named volume by name.
pub async fn remove_volume(name: &str) -> Result<()> {
    Volume::remove(name).await?;
    Ok(())
}

//--------------------------------------------------------------------------------------------------
// Tests
//--------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── resolve_workdir_host_path ────────────────────────────────────────────

    fn bind_mount(host: &str, guest: &str) -> VolumeMount {
        VolumeMount::Bind {
            host: host.into(),
            guest: guest.to_owned(),
            options: Default::default(),
            stat_virtualization: microsandbox_types::StatVirtualization::Relaxed,
            host_permissions: microsandbox_types::HostPermissions::Private,
            follow_root_symlinks: false,
            quota_mib: None,
        }
    }

    #[test]
    fn test_resolve_workdir_host_path_exact_match() {
        let mounts = vec![bind_mount("/home/user/project", "/workspace")];
        assert_eq!(
            resolve_workdir_host_path("/workspace", &mounts),
            "/home/user/project"
        );
    }

    #[test]
    fn test_resolve_workdir_host_path_subdirectory() {
        let mounts = vec![bind_mount("/home/user/project", "/workspace")];
        assert_eq!(
            resolve_workdir_host_path("/workspace/src", &mounts),
            "/home/user/project/src"
        );
    }

    #[test]
    fn test_resolve_workdir_host_path_no_matching_mount_falls_back_to_guest() {
        let mounts = vec![bind_mount("/home/user/project", "/other")];
        assert_eq!(
            resolve_workdir_host_path("/workspace", &mounts),
            "/workspace"
        );
    }

    #[test]
    fn test_resolve_workdir_host_path_no_mounts_falls_back_to_guest() {
        assert_eq!(resolve_workdir_host_path("/workspace", &[]), "/workspace");
    }

    // ── SandboxInfo ──────────────────────────────────────────────────────────

    #[test]
    fn test_host_path_to_guest_path_windows_backslash() {
        assert_eq!(
            host_path_to_guest_path(r"d:\my-directory\my-subdir\"),
            "/d/my-directory/my-subdir"
        );
    }

    #[test]
    fn test_host_path_to_guest_path_windows_forward_slash() {
        assert_eq!(
            host_path_to_guest_path("D:/my-directory/my-subdir"),
            "/d/my-directory/my-subdir"
        );
    }

    #[test]
    fn test_host_path_to_guest_path_windows_uppercase_drive() {
        assert_eq!(host_path_to_guest_path(r"C:\Users\me"), "/c/Users/me");
    }

    #[test]
    fn test_host_path_to_guest_path_windows_drive_root() {
        assert_eq!(host_path_to_guest_path(r"E:\"), "/e");
        assert_eq!(host_path_to_guest_path("E:"), "/e");
    }

    #[test]
    fn test_host_path_to_guest_path_unix_passthrough() {
        assert_eq!(
            host_path_to_guest_path("/home/user/project"),
            "/home/user/project"
        );
    }

    #[test]
    fn test_host_path_to_guest_path_unix_trailing_slash() {
        assert_eq!(
            host_path_to_guest_path("/home/user/project/"),
            "/home/user/project"
        );
    }

    #[test]
    fn test_host_path_to_guest_path_unix_root() {
        assert_eq!(host_path_to_guest_path("/"), "/");
    }

    #[test]
    fn test_sandbox_info_construction() {
        let info = SandboxInfo {
            name: "mybox".into(),
            status: SandboxStatus::Running,
            image: "alpine:latest".into(),
            cpus: 2,
            memory_mib: 1024,
            created_at: None,
            updated_at: None,
            workdir: None,
        };
        assert_eq!(info.name, "mybox");
        assert_eq!(info.status, SandboxStatus::Running);
        assert_eq!(info.cpus, 2);
        assert_eq!(info.memory_mib, 1024);
        assert!(info.created_at.is_none());
    }

    #[test]
    fn test_sandbox_info_clone() {
        let info = SandboxInfo {
            name: "box".into(),
            status: SandboxStatus::Stopped,
            image: "debian".into(),
            cpus: 1,
            memory_mib: 512,
            created_at: None,
            updated_at: None,
            workdir: None,
        };
        let cloned = info.clone();
        assert_eq!(cloned.name, info.name);
        assert_eq!(cloned.status, info.status);
    }

    // ── MetricsSnapshot ──────────────────────────────────────────────────────

    #[test]
    fn test_metrics_snapshot_default_is_zero() {
        let m = MetricsSnapshot::default();
        assert_eq!(m.cpu_percent, 0.0);
        assert_eq!(m.memory_bytes, 0);
        assert_eq!(m.disk_read_bytes, 0);
        assert_eq!(m.disk_write_bytes, 0);
        assert_eq!(m.net_rx_bytes, 0);
        assert_eq!(m.net_tx_bytes, 0);
        assert_eq!(m.disk_used_bytes, None);
        assert_eq!(m.disk_free_bytes, None);
        assert_eq!(m.uptime_secs, 0);
    }

    #[test]
    fn test_metrics_snapshot_construction() {
        let m = MetricsSnapshot {
            cpu_percent: 75.5,
            memory_bytes: 256 * 1024 * 1024,
            disk_read_bytes: 1_000_000,
            disk_write_bytes: 500_000,
            net_rx_bytes: 4096,
            net_tx_bytes: 2048,
            disk_used_bytes: Some(128 * 1024 * 1024),
            disk_free_bytes: Some(896 * 1024 * 1024),
            uptime_secs: 3661,
        };
        assert!((m.cpu_percent - 75.5).abs() < f64::EPSILON);
        assert_eq!(m.memory_bytes, 256 * 1024 * 1024);
        assert_eq!(m.disk_used_bytes, Some(128 * 1024 * 1024));
        assert_eq!(m.disk_free_bytes, Some(896 * 1024 * 1024));
        assert_eq!(m.uptime_secs, 3661);
    }

    #[test]
    fn test_metrics_snapshot_clone() {
        let m = MetricsSnapshot {
            cpu_percent: 10.0,
            ..Default::default()
        };
        let c = m.clone();
        assert_eq!(c.cpu_percent, 10.0);
    }

    // ── FsEntry ──────────────────────────────────────────────────────────────

    #[test]
    fn test_fs_entry_construction() {
        let e = FsEntry {
            path: "/etc/passwd".into(),
            kind: LocalFsEntryKind::File,
            size: 1234,
        };
        assert_eq!(e.path, "/etc/passwd");
        assert_eq!(e.kind, LocalFsEntryKind::File);
        assert_eq!(e.size, 1234);
    }

    #[test]
    fn test_fs_entry_clone() {
        let e = FsEntry {
            path: "/tmp".into(),
            kind: LocalFsEntryKind::Directory,
            size: 0,
        };
        let c = e.clone();
        assert_eq!(c.path, e.path);
        assert_eq!(c.kind, e.kind);
    }

    // ── LocalFsEntryKind ─────────────────────────────────────────────────────

    #[test]
    fn test_local_fs_entry_kind_equality() {
        assert_eq!(LocalFsEntryKind::File, LocalFsEntryKind::File);
        assert_ne!(LocalFsEntryKind::File, LocalFsEntryKind::Directory);
        assert_ne!(LocalFsEntryKind::Symlink, LocalFsEntryKind::Other);
    }

    #[test]
    fn test_all_local_fs_entry_kind_variants() {
        // Ensure all variants exist and are distinct
        let variants = [
            LocalFsEntryKind::File,
            LocalFsEntryKind::Directory,
            LocalFsEntryKind::Symlink,
            LocalFsEntryKind::Other,
        ];
        for (i, a) in variants.iter().enumerate() {
            for (j, b) in variants.iter().enumerate() {
                if i == j {
                    assert_eq!(a, b);
                } else {
                    assert_ne!(a, b);
                }
            }
        }
    }

    // ── NetworkRule / build_network_policy ──────────────────────────────────

    fn cidr_rule(cidr: &str, action: NetRuleAction, direction: NetRuleDirection) -> NetworkRule {
        NetworkRule {
            direction,
            action,
            dest_kind: NetRuleDestKind::Cidr,
            dest_value: cidr.to_owned(),
            dest_group: NetRuleDestGroup::default(),
            protocols: Vec::new(),
            port_range: None,
        }
    }

    #[test]
    fn test_net_rule_action_label() {
        assert_eq!(NetRuleAction::Allow.label(), "ALLOW");
        assert_eq!(NetRuleAction::Deny.label(), "DENY");
    }

    #[test]
    fn test_net_rule_direction_label() {
        assert_eq!(NetRuleDirection::Egress.label(), "EGRESS");
        assert_eq!(NetRuleDirection::Ingress.label(), "INGRESS");
        assert_eq!(NetRuleDirection::Any.label(), "ANY");
    }

    #[test]
    fn test_net_rule_dest_kind_cycle_wraps() {
        let mut kind = NetRuleDestKind::Any;
        for _ in 0..NetRuleDestKind::ALL.len() {
            kind = kind.cycle();
        }
        assert_eq!(kind, NetRuleDestKind::Any);
    }

    #[test]
    fn test_net_rule_dest_group_cycle_wraps() {
        let mut group = NetRuleDestGroup::Public;
        for _ in 0..NetRuleDestGroup::ALL.len() {
            group = group.cycle();
        }
        assert_eq!(group, NetRuleDestGroup::Public);
    }

    #[test]
    fn test_net_rule_protocol_is_icmp() {
        assert!(NetRuleProtocol::Icmpv4.is_icmp());
        assert!(NetRuleProtocol::Icmpv6.is_icmp());
        assert!(!NetRuleProtocol::Tcp.is_icmp());
        assert!(!NetRuleProtocol::Udp.is_icmp());
    }

    #[test]
    fn test_build_network_policy_empty_is_allow_all() {
        let policy = build_network_policy(&[]).expect("build should succeed");
        let allow_all = NetworkPolicy::allow_all();
        assert_eq!(policy.default_egress, allow_all.default_egress);
        assert_eq!(policy.default_ingress, allow_all.default_ingress);
        assert!(policy.rules.is_empty());
    }

    #[test]
    fn test_build_network_policy_with_cidr_rules() {
        let rules = vec![
            cidr_rule("10.0.0.0/8", NetRuleAction::Deny, NetRuleDirection::Egress),
            cidr_rule(
                "192.168.0.0/16",
                NetRuleAction::Allow,
                NetRuleDirection::Ingress,
            ),
        ];
        let policy = build_network_policy(&rules).expect("build should succeed");
        assert_eq!(policy.rules.len(), 2);
    }

    #[test]
    fn test_build_network_policy_with_group_and_protocols() {
        let rules = vec![NetworkRule {
            direction: NetRuleDirection::Egress,
            action: NetRuleAction::Allow,
            dest_kind: NetRuleDestKind::Group,
            dest_value: String::new(),
            dest_group: NetRuleDestGroup::Public,
            protocols: vec![NetRuleProtocol::Tcp],
            port_range: Some((443, 443)),
        }];
        let policy = build_network_policy(&rules).expect("build should succeed");
        assert_eq!(policy.rules.len(), 1);
    }

    #[test]
    fn test_build_network_policy_rejects_icmp_on_ingress() {
        let rules = vec![NetworkRule {
            direction: NetRuleDirection::Ingress,
            action: NetRuleAction::Allow,
            dest_kind: NetRuleDestKind::Any,
            dest_value: String::new(),
            dest_group: NetRuleDestGroup::default(),
            protocols: vec![NetRuleProtocol::Icmpv4],
            port_range: None,
        }];
        assert!(build_network_policy(&rules).is_err());
    }

    // ── SecretConfig ─────────────────────────────────────────────────────────

    #[test]
    fn test_secret_config_summary_redacts_value() {
        let secret = SecretConfig {
            env_var: "OPENAI_API_KEY".into(),
            value: "sk-super-secret".into(),
            allowed_hosts: vec![SecretHostPattern {
                kind: SecretHostKind::Exact,
                value: "api.openai.com".into(),
            }],
            inject_headers: true,
            inject_basic_auth: true,
            inject_query: false,
            inject_body: false,
            require_tls_identity: true,
        };
        let summary = secret.summary();
        assert!(!summary.contains("sk-super-secret"));
        assert!(summary.contains("OPENAI_API_KEY"));
        assert!(summary.contains("api.openai.com"));
    }
}
