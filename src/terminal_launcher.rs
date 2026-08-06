//! Opens a new terminal window on the host to run an interactive command
//! inside a sandbox, via the `msb` CLI's `exec` subcommand.
//!
//! Spawning a *new terminal window* (as opposed to running the command
//! in-process) is inherently platform-specific — there is no cross-platform
//! API for it. Each OS branch below uses the mechanism that platform's
//! terminals/shells expose.

use std::process::Command;

use anyhow::{Result};

/// Open a new terminal window on the host running
/// `msb exec <sandbox_name> -- sh -c <command>`.
///
/// The command is always wrapped in `sh -c` inside the sandbox so the user
/// can type arbitrary shell syntax (pipes, quoting, etc.) exactly as they
/// would at a shell prompt; it's passed as a single argument, so no host-side
/// shell quoting is needed for the sandbox side.
pub fn open_exec_terminal(sandbox_name: &str, command: &str) -> Result<()> {
    let msb_args = [
        "exec".to_owned(),
        sandbox_name.to_owned(),
        "--".to_owned(),
        "sh".to_owned(),
        "-c".to_owned(),
        command.to_owned(),
    ];

    #[cfg(target_os = "windows")]
    {
        spawn_windows_terminal(&msb_args)
    }
    #[cfg(target_os = "macos")]
    {
        spawn_macos_terminal(&msb_args)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        spawn_linux_terminal(&msb_args)
    }
}

/// Try a series of common Linux terminal emulators until one launches
/// successfully. Most accept a direct argv (no shell involved), so the
/// `msb` command and its arguments are passed through untouched.
#[cfg(all(unix, not(target_os = "macos")))]
fn spawn_linux_terminal(msb_args: &[String]) -> Result<()> {
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
            .arg("msb")
            .args(msb_args)
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
fn spawn_macos_terminal(msb_args: &[String]) -> Result<()> {
    let full_command =
        shell_join(std::iter::once("msb".to_owned()).chain(msb_args.iter().cloned()));
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
fn spawn_windows_terminal(msb_args: &[String]) -> Result<()> {
    if Command::new("wt.exe")
        .arg("msb")
        .args(msb_args)
        .spawn()
        .is_ok()
    {
        return Ok(());
    }

    // The empty "" argument is a required placeholder title for `start`
    // when the command itself is not quoted.
    Command::new("cmd")
        .args(["/C", "start", "", "msb"])
        .args(msb_args)
        .spawn()?;
    Ok(())
}
