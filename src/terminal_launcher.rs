//! Opens a new terminal window on the host to run an interactive command
//! inside a sandbox, natively via the `microsandbox` SDK — no `msb` CLI
//! binary required.
//!
//! Spawning a *new terminal window* (as opposed to running the command
//! in-process) is inherently platform-specific — there is no cross-platform
//! API for it. Each OS branch below uses the mechanism that platform's
//! terminals/shells expose. What actually runs inside that new window is
//! this same executable, re-invoked with a hidden flag (see
//! [`EXEC_TERMINAL_FLAG`]) that makes it connect to the sandbox and forward
//! the new terminal's stdio, instead of starting the TUI (see
//! [`crate::exec_session`]).

use std::env;
use std::process::Command;

use anyhow::{bail, Context, Result};

/// Hidden CLI flag recognised by `main` to enter exec-session mode: when
/// `argv[1] == EXEC_TERMINAL_FLAG`, `argv[2]` is the sandbox name and
/// `argv[3]` is the command line to run, instead of starting the TUI.
pub const EXEC_TERMINAL_FLAG: &str = "--exec-terminal";

/// Open a new terminal window on the host that connects to `sandbox_name`
/// and runs `sh -c <command>` there, with a real PTY and full stdio
/// forwarding.
///
/// The command is always wrapped in `sh -c` inside the sandbox so the user
/// can type arbitrary shell syntax (pipes, quoting, etc.) exactly as they
/// would at a shell prompt; it's passed as a single argument, so no host-side
/// shell quoting is needed for the sandbox side.
pub fn open_exec_terminal(sandbox_name: &str, command: &str) -> Result<()> {
    let exe = env::current_exe().context("locate current executable")?;
    let self_args = [
        EXEC_TERMINAL_FLAG.to_owned(),
        sandbox_name.to_owned(),
        command.to_owned(),
    ];

    #[cfg(target_os = "windows")]
    {
        spawn_windows_terminal(&exe, &self_args)
    }
    #[cfg(target_os = "macos")]
    {
        spawn_macos_terminal(&exe, &self_args)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        spawn_linux_terminal(&exe, &self_args)
    }
}

/// Try a series of common Linux terminal emulators until one launches
/// successfully. Most accept a direct argv (no shell involved), so `exe`
/// and its arguments are passed through untouched.
#[cfg(all(unix, not(target_os = "macos")))]
fn spawn_linux_terminal(exe: &std::path::Path, self_args: &[String]) -> Result<()> {
    // (binary name, flag that introduces the command to run, if any)
    const CANDIDATES: &[(&str, &[&str])] = &[
        ("x-terminal-emulator", &["-e"]),
        ("gnome-terminal", &["--"]),
        ("konsole", &["-e"]),
        ("xfce4-terminal", &["-e"]),
        ("alacritty", &["-e"]),
        ("kitty", &[]),
        ("xterm", &["-e"]),
    ];

    for (bin, flag) in CANDIDATES {
        if Command::new(bin)
            .args(*flag)
            .arg(exe)
            .args(self_args)
            .spawn()
            .is_ok()
        {
            return Ok(());
        }
    }

    bail!("No supported terminal emulator found (tried gnome-terminal, konsole, xterm, and others)")
}

/// Open a new Terminal.app window via AppleScript's `do script`, which is
/// the standard way to launch a command in a fresh terminal window on macOS.
#[cfg(target_os = "macos")]
fn spawn_macos_terminal(exe: &std::path::Path, self_args: &[String]) -> Result<()> {
    let full_command = shell_join(
        std::iter::once(exe.to_string_lossy().into_owned()).chain(self_args.iter().cloned()),
    );
    let script = format!(
        "tell application \"Terminal\" to do script \"{}\"",
        escape_applescript(&full_command)
    );
    Command::new("osascript").arg("-e").arg(script).spawn()?;
    Ok(())
}

/// Join argv into a single POSIX shell command line, quoting each argument
/// that needs it. Used to build the AppleScript `do script` payload, since
/// Terminal.app only accepts a command line, not an argv array.
#[cfg(target_os = "macos")]
fn shell_join(args: impl Iterator<Item = String>) -> String {
    args.map(|a| shell_quote(&a)).collect::<Vec<_>>().join(" ")
}

#[cfg(target_os = "macos")]
fn shell_quote(s: &str) -> String {
    let is_safe = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:@%,+=".contains(c));
    if is_safe {
        s.to_owned()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

#[cfg(target_os = "macos")]
fn escape_applescript(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Prefer Windows Terminal (`wt.exe`) when installed; fall back to a
/// classic `cmd.exe` console window via `start` otherwise.
#[cfg(target_os = "windows")]
fn spawn_windows_terminal(exe: &std::path::Path, self_args: &[String]) -> Result<()> {
    if Command::new("wt.exe")
        .arg(exe)
        .args(self_args)
        .spawn()
        .is_ok()
    {
        return Ok(());
    }

    // The empty "" argument is a required placeholder title for `start`
    // when the command itself is not quoted.
    Command::new("cmd")
        .args(["/C", "start", ""])
        .arg(exe)
        .args(self_args)
        .spawn()?;
    Ok(())
}
