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
- Multi-select bulk operations: press `Space` on a sandbox in the list to
  mark/unmark it (shown with a `☑` prefix and a yellow card border). When one
  or more sandboxes are marked, `s`/`S`/`K`/`d` apply Start/Stop/Kill/Remove
  to all marked sandboxes concurrently (filtered to the ones eligible for
  that action, e.g. only stopped sandboxes are started) instead of the
  highlighted sandbox, and a single summary notification reports how many
  succeeded. Marks are cleared after the bulk action runs and are
  automatically pruned if a marked sandbox disappears from a refreshed list.

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
