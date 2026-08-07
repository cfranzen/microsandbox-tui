# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0](https://github.com/cfranzen/microsandbox-tui/compare/microsandbox-tui-v0.1.0...microsandbox-tui-v0.2.0) (2026-08-07)


### Added

* config file for default sandbox parameters ([1bc5d51](https://github.com/cfranzen/microsandbox-tui/commit/1bc5d51fa6d2e097b7d2320de954e61b09df4651))
* initial microsandbox TUI implementation ([c40e7c7](https://github.com/cfranzen/microsandbox-tui/commit/c40e7c76337dab2273c85dd34ea22bde1393e8ba))
* live streaming logs for running sandboxes ([9abe371](https://github.com/cfranzen/microsandbox-tui/commit/9abe37165bb6ac607bbbbb77ea9b6a5ea11f8910))
* manage ports and env vars via sub-dialogs ([67c1991](https://github.com/cfranzen/microsandbox-tui/commit/67c199154e233dc37c155fbaf00e9c9f03426bec))
* merge Metrics tab into Info tab, add disk usage gauge ([bc24440](https://github.com/cfranzen/microsandbox-tui/commit/bc244400935ac760029ec9e2f10ec6b3eaebcd0f))
* metrics history sparklines for CPU and memory ([0e74b57](https://github.com/cfranzen/microsandbox-tui/commit/0e74b5727f8bec9e1c86e0cd201925b63c6dc585))
* mouse support for list selection, tab switching, and scrolling ([cbae17a](https://github.com/cfranzen/microsandbox-tui/commit/cbae17a6f4eead1464e72d2f477e6d77fb3c763a))
* multi-select bulk operations for start/stop/kill/remove ([3b29a58](https://github.com/cfranzen/microsandbox-tui/commit/3b29a5863679c15d2581ae6f62eba1557d38b608))
* network policy editor for CIDR-based egress/ingress rules ([e0e5731](https://github.com/cfranzen/microsandbox-tui/commit/e0e57317e9b5a201110b7ddd31083ab35f0ca84a))
* replace Ctrl+F workdir shortcut and Enter-to-create with explicit UI ([4b6142f](https://github.com/cfranzen/microsandbox-tui/commit/4b6142f492cb040f0df41b02a5c410cb2c0c1d89))
* search/filter sandboxes by name or status ([df960c0](https://github.com/cfranzen/microsandbox-tui/commit/df960c041e25938efa406a418c2d14d2894a356e))
* two-tab create dialog with workdir directory picker ([aa70587](https://github.com/cfranzen/microsandbox-tui/commit/aa70587f7352a046acfa8a0e35987a5752d6b27e))
* volume/mount management (Volumes view + create-dialog mounts) ([958a298](https://github.com/cfranzen/microsandbox-tui/commit/958a298ad5e490194def5a66fb02e26e069ff690))


### Fixed

* add separator line at top of card ([3676ff4](https://github.com/cfranzen/microsandbox-tui/commit/3676ff47071a4a807a97e700965e2efc37fbd96d))
* Add title to sandbox metrics and fixed spacing. ([427d8d1](https://github.com/cfranzen/microsandbox-tui/commit/427d8d12ee5550cbb0e43340547b17f23af0cccd))
* added missing import ([e558fbb](https://github.com/cfranzen/microsandbox-tui/commit/e558fbbe2ebc3b18d64deb5b3fcc46ec177f960d))
* adopt implementation to microsandbox API changes ([b637db5](https://github.com/cfranzen/microsandbox-tui/commit/b637db548f275e79b734af802485b43296aac145))
* correct ESC key handling in dialog and main view ([74ebb36](https://github.com/cfranzen/microsandbox-tui/commit/74ebb3686bc58ac1b97ec65534d8d3a06513032d))
* ensure version shown in UI is taken from release build ([8b4a258](https://github.com/cfranzen/microsandbox-tui/commit/8b4a258d6538325872e5b1be6db3b167ad8e6955))
* fixed some style bugs ([1981f3a](https://github.com/cfranzen/microsandbox-tui/commit/1981f3a99fbca9a411816f16518d57ede4a7a2d4))
* minor visual fixes ([4e443bb](https://github.com/cfranzen/microsandbox-tui/commit/4e443bbd490bb9059cd8fe401fad967d2f8ae336))


### Changed

* adopt lib.rs/main.rs split per Rust conventions ([fc6ec50](https://github.com/cfranzen/microsandbox-tui/commit/fc6ec50d354c9fa20dd052733e0e8b0054e19984))
* split app.rs god-file into focused submodules ([499f9f9](https://github.com/cfranzen/microsandbox-tui/commit/499f9f9db45ea25ff36d5de22eeff7fe249349ea))

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
