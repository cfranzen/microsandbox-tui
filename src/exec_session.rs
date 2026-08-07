//! Runs an interactive command inside a sandbox over the microsandbox SDK's
//! guest exec channel — no `msb` CLI subprocess required.
//!
//! This is entered as a hidden mode of this same binary (see
//! [`crate::terminal_launcher::EXEC_TERMINAL_FLAG`]): the launcher spawns a
//! *new terminal window* that re-execs us with
//! `--exec-terminal <sandbox> <command>`. Running the session in a fresh
//! process — rather than in-process inside the TUI — is what lets it own a
//! brand new terminal window's stdio.

use anyhow::{bail, Context, Result};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use microsandbox::sandbox::exec::ExecSink;
use microsandbox::{ExecEvent, ExecHandle, Sandbox};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// How often the local terminal size is polled and, if changed, forwarded
/// to the guest PTY. Cheap enough to poll rather than needing a
/// platform-specific resize-signal handler on every OS.
const RESIZE_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(300);

/// Connect to `sandbox_name`, run `sh -c <command>` with an allocated PTY,
/// and forward the local terminal's stdin/stdout and size changes to it
/// until the command exits.
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

    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
    let mut handle = sandbox
        .exec_stream_with("sh", |o| o.arg("-c").arg(command).stdin_pipe().tty(true))
        .await
        .context("start exec session")?;
    let _ = handle.resize(rows, cols).await;

    let stdin_sink = handle.take_stdin();

    enable_raw_mode().context("enable raw mode")?;
    let result = forward(&mut handle, stdin_sink).await;
    let _ = disable_raw_mode();

    result
}

/// Pump data between the local terminal and the guest PTY until the guest
/// process exits, periodically syncing the PTY size to the local terminal.
async fn forward(handle: &mut ExecHandle, stdin_sink: Option<ExecSink>) -> Result<i32> {
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut input_buf = [0u8; 4096];
    let mut last_size = crossterm::terminal::size().unwrap_or((80, 24));
    let mut resize_tick = tokio::time::interval(RESIZE_POLL_INTERVAL);

    loop {
        tokio::select! {
            n = stdin.read(&mut input_buf) => {
                let n = n.context("read local stdin")?;
                if n == 0 {
                    if let Some(sink) = &stdin_sink {
                        let _ = sink.close().await;
                    }
                    continue;
                }
                if let Some(sink) = &stdin_sink {
                    let _ = sink.write(&input_buf[..n]).await;
                }
            }
            event = handle.recv() => {
                match event {
                    Some(ExecEvent::Stdout(data)) | Some(ExecEvent::Stderr(data)) => {
                        stdout.write_all(&data).await?;
                        stdout.flush().await?;
                    }
                    Some(ExecEvent::Exited { code }) => return Ok(code),
                    Some(ExecEvent::Failed(payload)) => {
                        bail!("command failed to start: {payload:?}");
                    }
                    Some(ExecEvent::Started { .. }) | Some(ExecEvent::StdinError(_)) => {}
                    None => return Ok(0),
                }
            }
            _ = resize_tick.tick() => {
                if let Ok(size) = crossterm::terminal::size() {
                    if size != last_size {
                        last_size = size;
                        let _ = handle.resize(size.1, size.0).await;
                    }
                }
            }
        }
    }
}
