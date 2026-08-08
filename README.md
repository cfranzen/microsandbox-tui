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
- **Exec** — press `e` on a running sandbox to run a command inside it; opens a new
  terminal window on the host running `msb exec <name> -- sh -c <command>`, defaulting
  to a plain shell
- **Shell** — press `h` on a running sandbox to open a new host terminal attached
  directly to the sandbox's configured shell (set via `--shell` at creation time,
  defaulting to `/bin/sh`), without prompting for a command
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
- **Create dialog** — four-tab modal covering all
  [SandboxConfig](https://docs.microsandbox.dev/sdk/rust/sandbox#sandboxconfig) options:
  - **Basic tab**: Name, Image, CPUs / Max CPUs (side by side), Memory / Max Memory
    (side by side), Working directory (with interactive directory picker — the picked
    host directory is automatically bind-mounted into the sandbox at the equivalent
    guest path, so the guest sees it at "the same" location; Windows drive paths like
    `D:\foo\bar` are translated to `/d/foo/bar`)
  - **Guest OS tab**: Hostname, User, Shell, Environment variables (inline list, `a` to
    add via a popup, `d` to delete the selected entry), Volume mounts (inline list, bind
    a host directory or attach a named volume — applied only when the sandbox is
    created; the SDK does not support changing mounts on an already-running sandbox)
  - **Network tab**: Disable network toggle, Port mappings (inline list), Network policy
    rules (inline list) — supports the full range of the SDK's
    [network policy](https://docs.microsandbox.dev/sdk/rust/networking) options: Egress /
    Ingress / Any direction, Allow / Deny action, and Any / IP / CIDR / Domain / Domain
    suffix / Group destination kinds, with optional protocol (TCP/UDP/ICMP) and port
    range filters — applied only when the sandbox is created; the SDK does not support
    changing network policy on an already-running sandbox
  - **Secrets tab**: Injected secrets (inline list) — each secret maps an environment
    variable name to a value, a set of allowed hosts, and which
    [injection surfaces](https://docs.microsandbox.dev/sdk/rust/secrets) (HTTP headers,
    HTTP basic auth, query parameters, request body) it may be injected into, plus an
    optional "require TLS + verified peer identity" flag — applied only when the sandbox
    is created; the SDK does not support changing secrets on an already-running sandbox
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
| `e` | Open the "Exec" dialog to run a command in a new host terminal *(list focus only, running sandboxes only)* |
| `h` | Open a new host terminal attached to the sandbox's configured shell *(list focus only, running sandboxes only)* |
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
| `◄` / `►` | Switch between Basic / Guest OS / Network / Secrets tabs |
| `Space` | Toggle boolean fields (e.g. Disable Network, injection toggles) |
| `Ctrl-F` | Open directory picker (Workdir field) |
| `↑` / `↓` | Move selection within a focused inline list (Env Vars, Mounts, Ports, Net Rules, Secrets) |
| `a` | Add a new entry to the focused inline list (opens an Add popup) |
| `d` / `Delete` | Delete the selected entry from the focused inline list |
| `Enter` | Create sandbox, or (when a list is focused) open its Add popup, or (inside an Add popup) advance to the next field / submit on the last field |
| `Esc` | Close dialog / cancel the open Add popup |

### Network Rules add dialog

Reached by pressing `a` on the Network tab's "Net Rules" list. Lets you build a network
policy rule applied at sandbox-creation time (existing sandboxes cannot have their
network policy changed post-creation — this is a current limitation of the microsandbox
SDK). See the SDK's [networking docs](https://docs.microsandbox.dev/sdk/rust/networking)
for the full semantics.

| Key | Action |
|-----|--------|
| `◄` / `►` / `Space` | Cycle Direction (Egress / Ingress / Any) |
| `Space` | Cycle Action (Allow / Deny) or Destination kind (Any / IP / CIDR / Domain / Domain suffix / Group), or toggle the focused protocol checkbox (TCP / UDP / ICMP) |
| `◄` / `►` | Move the cursor between protocol checkboxes |
| `Tab` / `↑` / `↓` | Move between fields |
| `Enter` | Advance to the next field, or add the rule when on the last field (Ports) |
| `Esc` | Cancel and close the popup |

### Volume Mounts add dialog

Reached by pressing `a` on the Guest OS tab's "Mounts" list. Lets you add bind mounts
(host directory) or named-volume mounts, applied at sandbox-creation time (existing
sandboxes cannot have their mounts changed post-creation — this is a current limitation
of the microsandbox SDK).

| Key | Action |
|-----|--------|
| `Tab` / `↑` / `↓` | Move between the guest-path and source fields |
| `b` / `n` | Choose bind mount / named volume source kind |
| `Enter` | Advance to the next field, or add the mount when on the last field |
| `Esc` | Cancel and close the popup |

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
./target/release/msbui
```

## Usage

```bash
# Launch the TUI
msbui
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
