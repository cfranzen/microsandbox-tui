# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Live streaming logs: the Logs tab now follows a running sandbox's output in
  real time via the SDK's `log_stream` API, instead of re-polling with a
  one-shot read every refresh. Stopped sandboxes still use the one-shot
  `logs()` read. The stream task is started/stopped automatically as the
  selection or active tab changes, so no background tasks leak across
  refreshes.
- Metrics history sparklines: the Metrics tab keeps a rolling history (last
  60 samples) of CPU % and memory usage per sandbox and renders it as a
  `Sparkline` below each gauge.
- Network policy editor: the create-sandbox dialog's Advanced tab gained a
  "Net Rules" field that opens a sub-dialog for adding/removing CIDR-based
  egress/ingress allow/deny rules, applied via the SDK's
  `NetworkPolicy`/`NetworkPolicyBuilder` when the sandbox is created. Network
  policy remains fixed for the lifetime of the sandbox — the SDK has no API
  for changing it post-creation, so this is documented as a creation-time-only
  feature in the UI and README.
- Volume/mount management: a new top-level "Volumes" view (`v`) lists named
  volumes and lets you create/remove them directly via the SDK's
  `Volume`/`VolumeHandle` API. The create-sandbox dialog's Basic tab gained a
  "Mounts" field for adding bind mounts (host directory) or named-volume
  mounts, applied via the sandbox builder's `.volume(...)` at creation time.
  As with network policy, existing sandboxes' mounts cannot be changed after
  creation — documented as a known SDK limitation.
- Search/filter: press `/` to open a live search box in the sandbox list.
  Typing filters sandboxes by substring match on name; `status:running`,
  `status:stopped`, and `status:crashed` tokens filter by status and can be
  combined with a name substring. `Enter` confirms the filter and returns
  focus to the list (filter stays active); `Esc` clears it. The active
  filter is shown in the panel title, and the "New Sandbox" placeholder is
  hidden while a filter is applied.
- Config file for default sandbox parameters: an optional TOML file at the
  platform config directory (e.g. `~/.config/microsandbox-tui/config.toml`
  on Linux) can set defaults for image, CPUs, memory, hostname, workdir,
  user, and shell, used to prefill the "New Sandbox" dialog. A missing file
  falls back to the existing built-in defaults without erroring. Adds the
  `toml` and `dirs` crates as dependencies.
- Mouse support: click a sandbox card to select it, click a detail tab
  (Logs/Metrics/Filesystem/Info) to switch to it, and scroll the wheel over
  the list or detail panel to move the selection or scroll content. Mouse
  capture is enabled/disabled alongside raw mode in the terminal setup, and
  mouse input is ignored while the create dialog, Volumes view, or search
  box is active.

### Changed
- Merged the Metrics tab into the Info tab: the Info tab now shows sandbox
  configuration/timestamps together with live CPU/memory gauges (with
  sparkline history), disk and network I/O counters, and uptime, all in one
  place. The detail panel is down to three tabs: Logs, Filesystem, Info.
- Added a disk usage gauge to the merged Info tab, showing the writable
  overlay's used/free bytes (`SandboxMetrics::upper_used_bytes` /
  `upper_free_bytes`) when the SDK reports them for a sandbox; falls back to
  a "not reported" hint otherwise.
- Simplified navigation: `Tab` switches focus between the sandbox list and
  detail panel; `→` also moves focus from the list into the detail panel
  and, once the detail panel is focused, `←`/`→` cycle its tabs instead. The
  vi-style `j`/`k`/`h`/`l` keys have been removed — only the arrow keys and
  `Tab` are used for navigation now. The separate `s` (start) and `S` (stop)
  shortcuts were merged into a single `s` key that starts a stopped sandbox
  or stops a running one, since a sandbox can only ever be started or
  stopped, never both.
- Destructive actions now require confirmation: stopping or killing a
  running sandbox, removing a sandbox, and removing a volume all open an
  "Are you sure?" modal (confirm with `y`/`Enter`, cancel with `n`/`Esc`)
  before running.

### Removed
- Multi-select bulk operations (marking sandboxes with `Space` and applying
  Start/Stop/Kill/Remove to all marked sandboxes at once). Replaced with
  single-sandbox actions plus confirmation dialogs for destructive ones,
  per simplified navigation above.

## [0.1.0] - 2026-07-25

### Added
- Two-panel TUI layout: sandbox list (left) and detail panel (right)
- Sandbox list with colour-coded status cards (running/stopped/crashed)
- Sandbox lifecycle management: create, start, stop, kill, remove
- Detail tabs: Logs, Metrics, Filesystem, Info
- Logs tab with colour-coded output by source (stdout/stderr/pty/system)
- Metrics tab with CPU/memory gauges and disk/network I/O counters
- Filesystem tab with directory listing and keyboard navigation
- Info tab with full sandbox configuration display
- "New Sandbox" modal dialog with form validation
- Auto-refresh every 3 seconds via background async tasks
- Transient status notifications in the footer bar
- Context-sensitive keybind hints in the footer
- Full keyboard navigation (vi-style `j`/`k` supported)

[Unreleased]: https://github.com/cfranzen/microsandbox-tui/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/cfranzen/microsandbox-tui/releases/tag/v0.1.0
