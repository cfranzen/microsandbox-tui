//! `microsandbox-tui` — terminal UI for managing MicroSandbox microVM sandboxes.
//!
//! The library crate owns all application logic.  The `main.rs` binary is a
//! thin wrapper that sets up the terminal and hands control to [`app::run`].

pub mod app;
pub mod config;
pub mod sandbox;
pub mod theme;
pub mod ui;
