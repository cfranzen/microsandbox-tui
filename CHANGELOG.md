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
