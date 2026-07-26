# microsandbox-tui

A terminal user interface for managing [MicroSandboxes](https://microsandbox.dev/) —
lightweight microVM sandboxes — built with [ratatui](https://ratatui.rs/) and the
official [microsandbox Rust SDK](https://crates.io/crates/microsandbox).

## Features

- **Sandbox list** — colour-coded status cards (🟢 running · 🟡 stopped · 🔴 crashed)
  with image, CPU/memory config, and age
- **Lifecycle management** — create, start, stop, kill, and remove sandboxes from the
  keyboard
- **Logs tab** — scrollable, colour-coded log output by source (stdout / stderr / pty /
  system); live-tails new output for running sandboxes via the SDK's log-streaming API
  (falls back to a one-shot read for stopped sandboxes)
- **Metrics tab** — live CPU and memory gauges, disk I/O counters, network rx/tx, uptime,
  plus rolling sparkline history (last 60 samples) for CPU % and memory usage
- **Filesystem tab** — browse the sandbox filesystem; navigate into directories with
  `Enter`, go up with `Backspace`
- **Info tab** — full sandbox configuration (image, CPUs, memory, timestamps)
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
| `↑` / `k` | Move up in list / scroll up in detail panel |
| `↓` / `j` | Move down in list / scroll down in detail panel |
| `Tab` | Switch focus between sandbox list and detail panel |
| `Esc` | Return focus to sandbox list |
| `←` / `h` | Previous detail tab *(detail panel focus only)* |
| `→` / `l` | Next detail tab *(detail panel focus only)* |
| `Enter` | Open "New Sandbox" dialog (when placeholder is selected) or switch focus to detail panel |
| `n` | Open "New Sandbox" dialog |
| `s` | Start selected sandbox *(list focus only)* |
| `S` | Stop selected sandbox *(list focus only)* |
| `K` | Kill selected sandbox (SIGKILL) *(list focus only)* |
| `d` | Remove selected sandbox *(list focus only)* |
| `v` | Open Volumes view |
| `r` | Force refresh |
| `Backspace` | Navigate up one directory *(Filesystem tab only)* |

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

## Project Structure

```
src/
├── main.rs             # Terminal setup/teardown, entry point
├── lib.rs              # Library crate root (exposes modules for integration tests)
├── app.rs              # Application state, event loop, input handling
├── sandbox/
│   └── mod.rs          # Async SDK wrappers (list, create, start, stop, …)
└── ui/
    ├── mod.rs           # Top-level render dispatcher
    ├── layout.rs        # Header / footer / two-column split
    ├── sandbox_list.rs  # Left panel: sandbox cards
    ├── detail.rs        # Right panel: tab bar
    ├── logs.rs          # Logs tab
    ├── metrics.rs       # Metrics tab
    ├── filesystem.rs    # Filesystem tab
    ├── info.rs          # Info tab
    └── create_dialog.rs # New sandbox modal + directory picker
tests/
└── ui_rendering.rs     # Integration tests for UI rendering
```

## Contributing

Contributions are welcome! Please read [CONTRIBUTING.md](CONTRIBUTING.md) first.

## License

MIT — see [LICENSE](LICENSE).
