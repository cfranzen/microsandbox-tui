//! Runs an interactive command inside a sandbox over the microsandbox SDK's
//! guest exec channel — no `msb` CLI subprocess required.
//!
//! This is entered as a hidden mode of this same binary (see
//! [`crate::terminal_launcher::EXEC_TERMINAL_FLAG`]): the launcher spawns a
//! *new terminal window* that re-execs us with
//! `--exec-terminal <sandbox> <command>`. Running the session in a fresh
//! process — rather than in-process inside the TUI — is what lets it own a
//! brand new terminal window's stdio.

use anyhow::{Context, Result};
use microsandbox::Sandbox;

/// Connect to `sandbox_name` and attach an interactive `sh -c <command>`
/// session to it, using the SDK's purpose-built interactive `attach` path —
/// the same one `msb exec` uses — instead of hand-rolling PTY forwarding
/// over the general-purpose streaming-exec API. `attach` reads the host
/// terminal via a low-level, non-blocking fd (not the buffered
/// `tokio::io::stdin()` wrapper) and forwards raw bytes verbatim, which is
/// what makes multi-byte sequences like arrow-key history navigation work
/// reliably; a hand-rolled forward loop over `exec_stream_with` was found to
/// not forward those reliably.
///
/// Returns the guest process's exit code, which the caller should use as
/// this process's own exit code so the host terminal reflects success or
/// failure the same way `msb exec` did.
pub async fn run(sandbox_name: &str, command: &str) -> Result<i32> {
    let sandbox = Sandbox::get(sandbox_name)
        .await
        .with_context(|| format!("look up sandbox '{sandbox_name}'"))?
        .connect()
        .await
        .with_context(|| format!("connect to sandbox '{sandbox_name}'"))?;

    // Forward the host's TERM (falling back to a sane default) so guest
    // readline/ncurses-based programs can look up terminal capabilities.
    let term = std::env::var("TERM").unwrap_or_else(|_| "xterm-256color".to_owned());

    let exit_code = sandbox
        .attach_with("sh", |a| a.arg("-c").arg(command).env("TERM", term))
        .await
        .context("attach exec session")?;

    Ok(exit_code)
}
