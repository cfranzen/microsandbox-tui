//! Binary entry point — terminal setup, panic hook, and run loop.
//!
//! All application logic lives in the `microsandbox_tui` library crate.

use std::io;
use std::panic;
use std::process::ExitCode;

use anyhow::Result;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use microsandbox_tui::terminal_launcher::EXEC_TERMINAL_FLAG;
use microsandbox_tui::{app, exec_session};
use ratatui::{backend::CrosstermBackend, Terminal};

#[tokio::main]
async fn main() -> Result<ExitCode> {
    // Hidden mode: when launched by `terminal_launcher::open_exec_terminal`
    // in a fresh terminal window, connect to a sandbox and forward this
    // process's stdio to it instead of starting the TUI.
    let args: Vec<String> = std::env::args().collect();
    if let [_, flag, sandbox_name, command] = args.as_slice() {
        if flag == EXEC_TERMINAL_FLAG {
            let code = exec_session::run(sandbox_name, command).await?;
            return Ok(ExitCode::from(code.clamp(0, 255) as u8));
        }
    }

    // Restore the terminal before printing any panic message so the shell is
    // not left in raw mode if the application crashes.
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        default_hook(info);
    }));

    let mut terminal = setup_terminal()?;
    let result = app::run(&mut terminal).await;
    restore_terminal()?;
    result.map(|()| ExitCode::SUCCESS)
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal() -> Result<()> {
    disable_raw_mode()?;
    execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
    Ok(())
}
