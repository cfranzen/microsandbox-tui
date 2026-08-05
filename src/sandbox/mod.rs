//! Sandbox management: wraps the microsandbox SDK into async operations
//! that feed the TUI's state machine.

use anyhow::Result;
use chrono::{DateTime, Utc};
use futures::Stream;
use microsandbox::logs::{LogStreamOptions, LogStreamStart};
use microsandbox::sandbox::{FsEntryKind, LogEntry, LogOptions, LogSource};
use microsandbox::{MicrosandboxError, NetworkPolicy, Sandbox, SandboxMetrics, Volume, VolumeKind};

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
}

/// Traffic direction a [`NetworkRule`] applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NetRuleDirection {
    #[default]
    Egress,
    Ingress,
}

impl NetRuleDirection {
    pub fn label(self) -> &'static str {
        match self {
            NetRuleDirection::Egress => "EGRESS",
            NetRuleDirection::Ingress => "INGRESS",
        }
    }
}

/// A single CIDR-based network policy rule configured at sandbox-creation
/// time (the SDK does not support modifying network policy on an already
/// created sandbox — see [`create_sandbox`]'s use of this type).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkRule {
    pub cidr: String,
    pub action: NetRuleAction,
    pub direction: NetRuleDirection,
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

/// Summary of a named volume, as shown in the Volumes view.
#[derive(Debug, Clone, PartialEq)]
pub struct VolumeInfo {
    pub name: String,
    pub kind: VolumeKind,
    pub quota_mib: Option<u32>,
    pub used_bytes: u64,
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
    /// CIDR-based network policy rules applied at creation time. Ignored
    /// (with `disable_network` taking precedence) when empty.
    pub network_rules: Vec<NetworkRule>,
    /// Volume mounts applied at creation time. Existing sandboxes cannot
    /// have their mounts changed post-creation per the current SDK.
    pub mounts: Vec<VolumeMountConfig>,
}

//--------------------------------------------------------------------------------------------------
// List
//--------------------------------------------------------------------------------------------------

/// Retrieve all sandboxes from the local backend.
pub async fn list_sandboxes() -> Result<Vec<SandboxInfo>> {
    let handles = Sandbox::list().await?;
    let mut infos = Vec::with_capacity(handles.len());
    for h in handles {
        let (image, cpus, memory_mib) = if let Ok(cfg) = h.config() {
            let image = cfg
                .spec
                .image
                .oci_reference()
                .unwrap_or("(bind/disk)")
                .to_owned();
            (
                image,
                cfg.spec.resources.cpus,
                cfg.spec.resources.memory_mib,
            )
        } else {
            ("—".into(), 1, 512)
        };

        infos.push(SandboxInfo {
            name: h.name().to_owned(),
            status: h.status_snapshot(),
            image,
            cpus,
            memory_mib,
            created_at: h.created_at(),
            updated_at: h.updated_at(),
        });
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

/// Build a [`NetworkPolicy`] from a list of user-configured CIDR rules.
///
/// Starts from an allow-all default (matching the sandbox's normal
/// networking behaviour) and layers explicit egress/ingress allow/deny
/// rules on top, evaluated in order. Falls back to [`NetworkPolicy::allow_all`]
/// if any rule fails to parse (this shouldn't happen — the create dialog
/// validates CIDR syntax before an entry is added).
fn build_network_policy(rules: &[NetworkRule]) -> NetworkPolicy {
    let mut builder = NetworkPolicy::builder().default_allow();
    for rule in rules {
        let cidr = rule.cidr.clone();
        let direction = rule.direction;
        let action = rule.action;
        builder = builder.rule(move |r| {
            let r = match direction {
                NetRuleDirection::Egress => r.egress(),
                NetRuleDirection::Ingress => r.ingress(),
            };
            let dest = match action {
                NetRuleAction::Allow => r.allow(),
                NetRuleAction::Deny => r.deny(),
            };
            dest.cidr(cidr)
        });
    }
    builder
        .build()
        .unwrap_or_else(|_| NetworkPolicy::allow_all())
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
        builder = builder.network(|n| n.policy(build_network_policy(&cfg.network_rules)));
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

    // ── SandboxInfo ──────────────────────────────────────────────────────────

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

    #[test]
    fn test_net_rule_action_label() {
        assert_eq!(NetRuleAction::Allow.label(), "ALLOW");
        assert_eq!(NetRuleAction::Deny.label(), "DENY");
    }

    #[test]
    fn test_net_rule_direction_label() {
        assert_eq!(NetRuleDirection::Egress.label(), "EGRESS");
        assert_eq!(NetRuleDirection::Ingress.label(), "INGRESS");
    }

    #[test]
    fn test_build_network_policy_empty_is_allow_all() {
        let policy = build_network_policy(&[]);
        let allow_all = NetworkPolicy::allow_all();
        assert_eq!(policy.default_egress, allow_all.default_egress);
        assert_eq!(policy.default_ingress, allow_all.default_ingress);
        assert!(policy.rules.is_empty());
    }

    #[test]
    fn test_build_network_policy_with_rules() {
        let rules = vec![
            NetworkRule {
                cidr: "10.0.0.0/8".into(),
                action: NetRuleAction::Deny,
                direction: NetRuleDirection::Egress,
            },
            NetworkRule {
                cidr: "192.168.0.0/16".into(),
                action: NetRuleAction::Allow,
                direction: NetRuleDirection::Ingress,
            },
        ];
        let policy = build_network_policy(&rules);
        assert_eq!(policy.rules.len(), 2);
    }
}
