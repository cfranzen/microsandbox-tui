# microsandbox-tui — Implementation TODO

A Rust TUI application for managing MicroSandboxes, built with [ratatui](https://ratatui.rs/).

---

## Project Setup

- [x] Initialize Cargo project (`Cargo.toml`, dependencies)
- [x] Install Rust toolchain + C build tools in sandbox
- [x] Add dependencies: `microsandbox`, `ratatui`, `crossterm`, `tokio`, `futures`, `anyhow`, `chrono`, `humantime`, `serde`, `serde_json`

---

## Source Files

### Core

| File | Status | Description |
|------|--------|-------------|
| `src/main.rs` | ✅ done | Entry point: terminal setup, panic hook, run loop, cleanup |
| `src/app.rs` | ✅ done | App state, event loop, input handling, background message passing |
| `src/sandbox/mod.rs` | ✅ done | Async wrappers around the microsandbox SDK (list, start, stop, kill, remove, create, logs, metrics, filesystem) |

### UI Modules

| File | Status | Description |
|------|--------|-------------|
| `src/ui/mod.rs` | ✅ done | Top-level `render()` dispatcher, re-exports |
| `src/ui/layout.rs` | ✅ done | Split left/right panels, header bar, footer/keybind bar |
| `src/ui/sandbox_list.rs` | ✅ done | Left panel: sandbox cards with status indicator, image, resources, action hints |
| `src/ui/detail.rs` | ✅ done | Right panel: tab bar + dispatch to tab-specific renderers |
| `src/ui/logs.rs` | ✅ done | Logs tab: coloured log lines, source badges (OUT/ERR/PTY/SYS), scroll |
| `src/ui/filesystem.rs` | ✅ done | Filesystem tab: directory listing, kind icons, size column, navigate up with Backspace |
| `src/ui/info.rs` | ✅ done | Info tab: sandbox config dump (image, CPUs, memory, created/updated timestamps) + live metrics (CPU/memory/disk gauges with sparkline history, disk I/O, network I/O, uptime) |
| `src/ui/create_dialog.rs` | ✅ done | "New Sandbox" modal: name, image, CPUs, memory fields; validation error display |

---

## Features

### Sandbox List (left panel)

- [x] Async list of all sandboxes via `Sandbox::list()`
- [x] Preserve selection across refreshes
- [x] "New Sandbox" placeholder entry at the bottom
- [x] Keyboard navigation (↑/↓, j/k)
- [x] Auto-refresh every 3 seconds
- [x] Sandbox cards with coloured status dot (green=running, yellow=stopped, red=crashed)
- [x] Show sandbox name, image, CPU/memory config
- [x] Show age / uptime from `created_at`

### Sandbox Lifecycle

- [x] **Start** stopped sandbox (`s` key) — detached mode so it survives the TUI
- [x] **Stop** running sandbox (`S` key) — graceful shutdown
- [x] **Kill** running sandbox (`K` key) — immediate SIGKILL
- [x] **Remove** stopped sandbox (`d` key) — deletes all state on disk
- [x] **Create** new sandbox via dialog (`n` or Enter on "New Sandbox")
- [x] Transient status notifications (4 s auto-expire)
- [x] Background refresh after every action

### Logs Tab

- [x] Read logs via `SandboxHandle::logs()` (works for running and stopped)
- [x] Includes all sources: stdout, stderr, pty output, system markers
- [x] Capped at 500 lines in memory
- [x] Scroll support (↑/↓ when detail panel is focused)
- [x] Render with timestamp, source badge, message text
- [x] Colour-code by source (stdout=green, stderr=red, system=grey)
- [x] Auto-scroll to bottom on new entries

### Filesystem Tab

- [x] Directory listing via `sb.fs().list(path)` through `connect()`
- [x] Navigate up with Backspace
- [x] Cached per (sandbox, path) pair
- [x] Render as a table: kind icon, path, size
- [x] Navigate into directories with Enter
- [x] Show current path in tab header

### Info Tab

- [x] Parse `SandboxHandle::config()` and display structured config
- [x] Show image, CPUs, memory
- [x] Show `created_at` and `updated_at` timestamps
- [x] Live metrics merged in: point-in-time fetch via `Sandbox::metrics()` through
      `connect()`, cached per sandbox name, auto-refreshed every 3 seconds while the
      tab is active
- [x] CPU usage gauge / bar with rolling sparkline history (last 60 samples)
- [x] Memory usage gauge with MiB/MiB display and rolling sparkline history
- [x] Disk usage gauge (writable overlay used/free bytes), when reported by the SDK
- [x] Disk read/write byte counters
- [x] Network rx/tx byte counters
- [x] Uptime display

### Create Dialog

- [x] Fields: name, image, CPUs, memory
- [x] Tab/Shift-Tab field navigation
- [x] Input validation (non-empty name/image, numeric CPU/memory)
- [x] Error message display
- [x] Submit calls `Sandbox::builder().detached(true).create()`
- [x] Render as a centred modal with border and field labels
- [x] Highlight active field with cursor

### Layout & Chrome

- [x] Header bar: app title + version
- [x] Two-column layout: sandbox list (left, ~27%) + detail panel (right, ~73%)
- [x] Footer: context-sensitive keybind hints
- [x] Notification bar shown above footer when a message is active
- [x] Focus ring: highlight active panel border

---

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `q` / `Ctrl-c` | Quit |
| `↑` / `k` | Select previous sandbox (list focus) / Scroll up (detail focus) |
| `↓` / `j` | Select next sandbox (list focus) / Scroll down (detail focus) |
| `Tab` | Switch focus between list and detail panel |
| `←` / `h` | Previous detail tab (detail focus) |
| `→` / `l` | Next detail tab (detail focus) |
| `Enter` | Open detail panel / Open create dialog (on "New Sandbox") |
| `n` | Open create dialog |
| `s` | Start selected sandbox |
| `S` | Stop selected sandbox |
| `K` | Kill selected sandbox |
| `d` | Remove selected sandbox |
| `r` | Force refresh |
| `Backspace` | Navigate up one directory (filesystem tab) |
| `Esc` | Close dialog |

---

## Build & Verify

- [x] `cargo build` — compiles without errors or warnings
- [ ] `cargo clippy` — passes cleanly
- [ ] Manual smoke-test against a live microsandbox runtime

---

## Future Enhancements

- [x] Live streaming logs (continuous tail, not one-shot read)
- [x] Metrics history sparkline / time series chart
- [x] Network policy editor (add/remove CIDR rules)
- [x] Volume/mount management
- [x] Search/filter sandboxes by name or status
- [x] Config file for default sandbox parameters
- [x] Mouse support
- [x] ~~Multiple sandbox selection for bulk operations~~ (removed again — replaced
      with simpler single-sandbox actions plus confirmation dialogs; see
      "Navigation simplification" below)
- [x] Navigation simplification: `←`/`→`/`Tab` switch focus between the sandbox
      list and detail panel, a single `s` key toggles start/stop based on
      sandbox state, and destructive actions (stop, kill, remove sandbox,
      remove volume) show an "Are you sure?" confirmation dialog
