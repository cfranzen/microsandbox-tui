# microsandbox-tui

A terminal user interface for managing [MicroSandboxes](https://microsandbox.dev/) —
lightweight microVM sandboxes — built with [ratatui](https://ratatui.rs/) and the
official [microsandbox Rust SDK](https://crates.io/crates/microsandbox).

## Features

- **Sandbox list** — colour-coded status cards (🟢 running · 🟡 stopped · 🔴 crashed)
  with image, CPU/memory config, and age
- **Lifecycle management** — create, start, stop, terminate, and remove sandboxes from
  the keyboard; a single `s` key toggles start/stop depending on the sandbox's current
  state, and destructive actions (stop, terminate, remove) show an "Are you sure?"
  confirmation dialog before running
- **Search/filter** — press `/` to search sandboxes live by substring on name, or use
  `status:running` / `status:stopped` / `status:crashed` tokens to filter by status; the
  active filter is shown in the panel title and stays applied until cleared with `Esc`
- **Config file** — optional TOML config at the platform config directory prefills the
  create-dialog's defaults (image, CPUs, memory, hostname, workdir, user, shell)
- **Mouse support** — click a sandbox card to select it, click a detail tab to switch to
  it, and scroll the wheel over the list or detail panel to navigate/scroll
- **Logs tab** — scrollable, colour-coded log output by source (stdout / stderr / pty /
  system); live-tails new output for running sandboxes via the SDK's log-streaming API
  (falls back to a one-shot read for stopped sandboxes)
- **Filesystem tab** — browse the sandbox filesystem; navigate into directories with
  `Enter`, go up with `Backspace`
- **Info tab** — full sandbox configuration (image, CPUs, memory, timestamps) together
  with live metrics: CPU and memory gauges with rolling sparkline history (last 60
  samples), a writable-overlay disk usage gauge (when reported by the sandbox), disk I/O
  counters, network rx/tx, and uptime
- **Create dialog** — two-tab modal covering all
  [SandboxConfig](https://docs.microsandbox.dev/sdk/rust/sandbox#sandboxconfig) options:
  - **Basic tab**: Name, Image, CPUs, Memory, Port mappings, Environment variables,
    Working directory (with interactive directory picker), Volume mounts (bind-mount a
    host directory or attach a named volume — applied only when the sandbox is created;
    the SDK does not support changing mounts on an already-running sandbox)
  - **Advanced tab**: Hostname, User, Shell, Max CPUs, Max Memory, Disable network toggle,
    Network policy rules (add/remove CIDR-based allow/deny rules for egress/ingress
    traffic — applied only when the sandbox is created; the SDK does not support
    changing network policy on an already-running sandbox)
- **Volumes view** (`v`) — list, create, and remove named persistent volumes directly
  via the SDK, independent of any particular sandbox
- **Auto-refresh** — sandbox list and detail data refresh automatically every 3 seconds

## Keyboard Shortcuts

### Main view

| Key | Action |
|-----|--------|
| `q` / `Q` / `Ctrl-c` | Quit |
| `↑` | Move up in list / scroll up in detail panel |
| `↓` | Move down in list / scroll down in detail panel |
| `Tab` / `→` | Switch focus to detail panel *(list focus)*, or go to the next detail tab *(detail focus)* |
| `Shift-Tab` / `←` | Go to the previous detail tab *(detail focus)*, or return focus to the sandbox list once the leftmost tab is reached |
| `Esc` | Return focus to sandbox list |
| `Enter` | Open "New Sandbox" dialog (when placeholder is selected) or switch focus to detail panel |
| `n` | Open "New Sandbox" dialog |
| `s` | Start selected sandbox if stopped, or stop it (with confirmation) if running *(list focus only)* |
| `t` | Terminate selected sandbox (SIGKILL), with confirmation *(list focus only, running sandboxes only)* |
| `d` | Remove selected sandbox, with confirmation *(list focus only)* |
| `v` | Open Volumes view |
| `/` | Enter search/filter mode *(list focus only)* |
| `r` | Force refresh |
| `T` | Toggle between the dark and bright themes |
| `Backspace` | Navigate up one directory *(Filesystem tab only)* |

#### Confirmation dialog

Destructive actions — stopping or terminating a running sandbox, removing a sandbox, and
removing a volume — open an "Are you sure?" modal before running. Press `y` or `Enter`
to confirm, or `n`/`Esc` to cancel. No other input is processed while the dialog is
open.

#### Search / filter

Press `/` to open the search box (shown in the panel title). Type to filter the list
live by substring match on sandbox name; add a `status:running`, `status:stopped`, or
`status:crashed` token to also filter by status (multiple tokens are combined with AND,
e.g. `web status:running`). Press `Enter` to confirm the filter and return keyboard
focus to the list (the filter stays active), or `Esc` to clear the filter and exit
search mode. While a filter is active, the "New Sandbox" placeholder is hidden and the
panel title shows the current filter text.

#### Mouse support

- Click a sandbox card to select it.
- Click a detail tab label (Logs / Filesystem / Info) to switch to it and
  focus the detail panel.
- Click anywhere else in the list or detail panel to focus that panel.
- Scroll the wheel over the sandbox list to move the selection up/down.
- Scroll the wheel over the detail panel to scroll the Logs or Filesystem view.

Mouse input is ignored while the create-sandbox dialog, Volumes view, or search box is
active, so it never interferes with those modal flows.

### Create dialog

| Key | Action |
|-----|--------|
| `Tab` / `↑` / `↓` | Move between fields |
| `◄` / `►` | Switch between Basic and Advanced tabs |
| `Space` | Toggle boolean fields (e.g. Disable Network) |
| `Ctrl-F` | Open directory picker (Workdir field) |
| `Enter` | Create sandbox (or open the focused field's manage dialog: Ports, Env Vars, Mounts, Net Rules) |
| `Esc` | Close dialog |

### Network Rules dialog

Reached from the Advanced tab's "Net Rules" field. Lets you build a CIDR-based network
policy applied at sandbox-creation time (existing sandboxes cannot have their network
policy changed post-creation — this is a current limitation of the microsandbox SDK).

| Key | Action |
|-----|--------|
| `↑` / `↓` | Select a rule |
| `a` | Add a new rule |
| `d` / `Delete` | Delete the selected rule |
| `e` / `i` | Choose Egress / Ingress direction (Add mode) |
| `Space` | Toggle Allow / Deny action (Add mode) |
| `Enter` | Confirm the CIDR and add the rule (Add mode) / close dialog (List mode) |
| `Esc` | Cancel add / close dialog |

### Volume Mounts dialog

Reached from the Basic tab's "Mounts" field. Lets you add bind mounts (host directory)
or named-volume mounts, applied at sandbox-creation time (existing sandboxes cannot have
their mounts changed post-creation — this is a current limitation of the microsandbox
SDK).

| Key | Action |
|-----|--------|
| `↑` / `↓` | Select a mount |
| `a` | Add a new mount |
| `d` / `Delete` | Delete the selected mount |
| `Tab` / `↑` / `↓` | Move between the guest-path and source fields (Add mode) |
| `b` / `n` | Choose bind mount / named volume source kind (Add mode) |
| `Enter` | Confirm the mount (Add mode) / close dialog (List mode) |
| `Esc` | Cancel add / close dialog |

### Volumes view

Opened with `v` from the main view. Manages named, persistent volumes directly via the
SDK (independent of any particular sandbox).

| Key | Action |
|-----|--------|
| `↑` / `↓` | Select a volume |
| `n` | Create a new volume |
| `d` / `Delete` | Remove the selected volume |
| `r` | Refresh the volume list |
| `Space` | Toggle Directory / Disk kind (Add mode) |
| `Enter` | Confirm the volume name and create it (Add mode) |
| `Esc` | Cancel add / close view |

### Directory picker

| Key | Action |
|-----|--------|
| `↑` / `↓` | Navigate entries |
| `Enter` | Descend into directory |
| `Space` | Confirm current directory as workdir |
| `/` | Open drive / root selector |
| `~` | Jump to home directory |
| `Esc` | Close picker (returns to create dialog) |

## Installation

### Prerequisites

- A recent stable Rust toolchain — install via [rustup](https://rustup.rs/)
- The [`msb` CLI](https://docs.microsandbox.dev/cli/overview) installed and your host configured with KVM (Linux) or hardware virtualisation (macOS/Windows)
- Building may also require `libcap-ng` on Linux:
  ```bash
  sudo apt install libcap-ng-dev    # Debian / Ubuntu
  sudo dnf install libcap-ng-devel  # Fedora / RHEL
  ```

### Build from source

```bash
git clone https://github.com/cfranzen/microsandbox-tui
cd microsandbox-tui
cargo build --release
./target/release/microsandbox-tui
```

## Usage

```bash
# Launch the TUI
microsandbox-tui
```

The TUI connects to the local microsandbox runtime automatically (no server process needed).

## Configuration

Default parameters for the "New Sandbox" dialog can be set in a TOML config file at:

- Linux: `~/.config/microsandbox-tui/config.toml`
- macOS: `~/Library/Application Support/microsandbox-tui/config.toml`
- Windows: `%APPDATA%\microsandbox-tui\config.toml`

The file is optional — if it's missing, the built-in defaults (image `alpine`, 1 CPU,
512 MiB memory, shell `/bin/sh`) are used. Any field you omit falls back to those
defaults. Example:

```toml
image = "ubuntu:22.04"
cpus = 4
memory_mib = 2048
hostname = "dev-box"
workdir = "/workspace"
user = "dev"
shell = "/bin/bash"
```

## Project Structure

```
src/
├── main.rs             # Terminal setup/teardown, entry point
├── lib.rs              # Library crate root (exposes modules for integration tests)
├── app/                # Application state, event loop, input handling
├── config.rs           # TOML config file for default sandbox parameters
├── theme.rs            # Centralized design tokens (colors, borders, style helpers);
│                        # dark/bright palettes live here
├── sandbox/
│   └── mod.rs          # Async SDK wrappers (list, create, start, stop, …)
└── ui/
    ├── mod.rs           # Top-level render dispatcher
    ├── layout.rs        # Header / footer / two-column split
    ├── sandbox_list.rs  # Left panel: sandbox cards
    ├── detail.rs        # Right panel: tab bar
    ├── logs.rs          # Logs tab
    ├── filesystem.rs    # Filesystem tab
    ├── info.rs          # Info tab (config + timestamps + live metrics)
    ├── create_dialog.rs # New sandbox modal + directory picker
    ├── confirm_dialog.rs# "Are you sure?" confirmation modal
    └── volumes.rs       # Volumes management view
tests/
└── ui_rendering.rs     # Integration tests for UI rendering
```

Every color, border style, and reusable text-style "recipe" used by the UI is
defined once on [`Theme`](src/theme.rs) — views never hardcode a `Color` or
`BorderType`, they call a method like `theme.accent()` or
`theme.border_style(focused)` instead. Press `T` at runtime to switch between
the built-in dark and bright palettes (`Theme::dark()` / `Theme::light()`).

## Contributing

Contributions are welcome! Please read [CONTRIBUTING.md](CONTRIBUTING.md) first.

## License

MIT — see [LICENSE](LICENSE).
