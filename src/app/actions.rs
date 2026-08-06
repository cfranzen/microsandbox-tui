//! Sandbox actions, pending-confirmation handling, and the input-adjacent
//! helpers (filtering, scrolling, tab bookkeeping, dialog submission) used
//! while responding to key events in [`super::keys`].

use std::time::Duration;

use crossterm::event::KeyCode;

use crate::sandbox::{SandboxInfo, SandboxStatus as Status};

use super::{App, AppMessage, DetailTab};

/// A destructive action awaiting user confirmation via the "Are you sure?"
/// dialog, e.g. stopping/terminating a sandbox, or removing a sandbox/volume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingAction {
    StopSandbox(String),
    TerminateSandbox(String),
    RemoveSandbox(String),
    RemoveVolume(String),
}

impl PendingAction {
    /// Human-readable confirmation prompt shown in the dialog.
    pub fn confirm_message(&self) -> String {
        match self {
            PendingAction::StopSandbox(name) => format!("Stop sandbox '{name}'?"),
            PendingAction::TerminateSandbox(name) => {
                format!("Terminate sandbox '{name}'? This forcefully stops it immediately.")
            }
            PendingAction::RemoveSandbox(name) => {
                format!("Remove sandbox '{name}'? This deletes all of its state.")
            }
            PendingAction::RemoveVolume(name) => {
                format!("Remove volume '{name}'? This deletes all of its data.")
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum SandboxAction {
    Start,
    Stop,
    Terminate,
    Remove,
}

pub(crate) fn submit_create_dialog(app: &mut App) {
    let dlg = &app.create_dialog;

    let name = dlg.name.trim().to_owned();
    if name.is_empty() {
        app.create_dialog.error = Some("Name is required".into());
        return;
    }

    let image = dlg.image.trim().to_owned();
    if image.is_empty() {
        app.create_dialog.error = Some("Image is required".into());
        return;
    }

    let cpus: u8 = match dlg.cpus.trim().parse::<u8>() {
        Ok(v) if (1..=32).contains(&v) => v,
        Ok(_) => {
            app.create_dialog.error = Some("CPUs must be between 1 and 32".into());
            return;
        }
        Err(_) => {
            app.create_dialog.error = Some("CPUs must be a number (1–32)".into());
            return;
        }
    };

    let memory: u32 = match dlg.memory.trim().parse::<u32>() {
        Ok(v) if v >= 64 => v,
        Ok(_) => {
            app.create_dialog.error = Some("Memory must be at least 64 MiB".into());
            return;
        }
        Err(_) => {
            app.create_dialog.error = Some("Memory must be a number in MiB (min 64)".into());
            return;
        }
    };

    let ports = dlg.ports.clone();
    let env_vars = dlg.env_vars.clone();

    let hostname = dlg.hostname.trim().to_owned();
    let hostname = if hostname.is_empty() {
        None
    } else {
        Some(hostname)
    };

    let workdir = dlg.workdir.trim().to_owned();
    let workdir = if workdir.is_empty() {
        None
    } else {
        Some(workdir)
    };

    let user = dlg.user.trim().to_owned();
    let user = if user.is_empty() { None } else { Some(user) };

    let shell_val = dlg.shell.trim().to_owned();
    let shell = if shell_val.is_empty() || shell_val == "/bin/sh" {
        None
    } else {
        Some(shell_val)
    };

    let max_cpus: Option<u8> = if dlg.max_cpus.trim().is_empty() {
        None
    } else {
        match dlg.max_cpus.trim().parse::<u8>() {
            Ok(v) if (1..=32).contains(&v) => Some(v),
            _ => {
                app.create_dialog.error = Some("Max CPUs must be between 1 and 32".into());
                return;
            }
        }
    };

    let max_memory: Option<u32> = if dlg.max_memory.trim().is_empty() {
        None
    } else {
        match dlg.max_memory.trim().parse::<u32>() {
            Ok(v) if v >= 64 => Some(v),
            _ => {
                app.create_dialog.error = Some("Max Memory must be at least 64 MiB".into());
                return;
            }
        }
    };

    let disable_network = dlg.disable_network;
    let network_rules = dlg.network_rules.clone();
    let mounts = dlg.mounts.clone();

    app.create_dialog = Default::default();

    let tx = app.msg_tx.clone();
    tokio::spawn(async move {
        let cfg = crate::sandbox::CreateConfig {
            name: name.clone(),
            image,
            cpus,
            memory_mib: memory,
            ports,
            env_vars,
            hostname,
            workdir,
            user,
            shell,
            max_cpus,
            max_memory_mib: max_memory,
            disable_network,
            network_rules,
            mounts,
        };
        let result = crate::sandbox::create_sandbox(&cfg).await;
        let (msg, is_err) = match result {
            Ok(()) => (format!("Created sandbox '{name}'"), false),
            Err(e) => (format!("Create failed: {e}"), true),
        };
        let _ = tx.send(AppMessage::Notification(msg, is_err));
        tokio::time::sleep(Duration::from_millis(300)).await;
        if let Ok(list) = crate::sandbox::list_sandboxes().await {
            let _ = tx.send(AppMessage::SandboxList(Ok(list)));
        }
    });
}

/// Handle key input while the search/filter box is active.
pub(crate) fn handle_search_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc => {
            app.search_active = false;
            app.filter.clear();
            clamp_selection_to_filter(app);
        }
        KeyCode::Enter => {
            app.search_active = false;
            clamp_selection_to_filter(app);
        }
        KeyCode::Backspace => {
            app.filter.pop();
            clamp_selection_to_filter(app);
        }
        KeyCode::Char(c) => {
            app.filter.push(c);
            clamp_selection_to_filter(app);
        }
        _ => {}
    }
}

/// Ensure the current selection still points at a visible sandbox after the
/// filter changes; snaps to the first match, or the "New Sandbox" slot when
/// no filter is active and the list is empty.
pub(crate) fn clamp_selection_to_filter(app: &mut App) {
    let visible = app.visible_indices();
    if visible.is_empty() {
        app.selected = if app.filter.trim().is_empty() {
            app.sandboxes.len()
        } else {
            0
        };
        return;
    }
    if !visible.contains(&app.selected) {
        app.selected = visible[0];
    }
}

pub(crate) fn on_sandbox_selected(app: &mut App) {
    app.log_scroll = 0;
    app.fs_scroll = 0;
    app.fs_path = "/".into();
    if let Some(sb) = app.selected_sandbox().cloned() {
        match app.tab {
            DetailTab::Logs => app.sync_log_stream(),
            DetailTab::Filesystem => {
                let path = app.fs_path.clone();
                app.request_fs(&sb.name, &path);
            }
            DetailTab::Info => app.request_metrics(&sb.name),
        }
    }
}

pub(crate) fn on_tab_switched(app: &mut App) {
    if let Some(sb) = app.selected_sandbox().cloned() {
        match app.tab {
            DetailTab::Logs => app.sync_log_stream(),
            DetailTab::Filesystem => {
                let path = app.fs_path.clone();
                app.request_fs(&sb.name, &path);
            }
            DetailTab::Info => app.request_metrics(&sb.name),
        }
    }
}

pub(crate) fn scroll_up(app: &mut App) {
    match app.tab {
        DetailTab::Logs => app.log_scroll = app.log_scroll.saturating_sub(3),
        DetailTab::Filesystem => app.fs_scroll = app.fs_scroll.saturating_sub(1),
        _ => {}
    }
}

pub(crate) fn scroll_down(app: &mut App) {
    match app.tab {
        DetailTab::Logs => app.log_scroll = app.log_scroll.saturating_add(3),
        DetailTab::Filesystem => app.fs_scroll = app.fs_scroll.saturating_add(1),
        _ => {}
    }
}

pub(crate) fn nav_fs_up(app: &mut App) {
    let path = std::path::Path::new(&app.fs_path);
    if let Some(parent) = path.parent() {
        let parent_str = parent.to_string_lossy().into_owned();
        let parent_str = if parent_str.is_empty() {
            "/".into()
        } else {
            parent_str
        };
        app.fs_path = parent_str.clone();
        app.fs_scroll = 0;
        if let Some(sb) = app.selected_sandbox().cloned() {
            app.request_fs(&sb.name, &parent_str);
        }
    }
}

/// Trigger a background refresh of the named-volumes list.
pub(crate) fn request_volume_refresh(app: &App) {
    let tx = app.msg_tx.clone();
    tokio::spawn(async move {
        let result = crate::sandbox::list_volumes().await;
        let _ = tx.send(AppMessage::VolumeList(result));
    });
}

/// Returns true if a sandbox matches every whitespace-separated token of the
/// filter string. A `status:<value>` token matches the sandbox's status
/// (case-insensitive, e.g. `status:running`); any other token is matched as
/// a case-insensitive substring of the sandbox's name.
pub(crate) fn sandbox_matches_filter(sb: &SandboxInfo, filter: &str) -> bool {
    filter.split_whitespace().all(|token| {
        if let Some(status) = token.strip_prefix("status:") {
            format!("{:?}", sb.status).eq_ignore_ascii_case(status)
        } else {
            sb.name.to_lowercase().contains(&token.to_lowercase())
        }
    })
}

/// Start a stopped sandbox immediately, or open the confirmation dialog to
/// stop a running one. Starting isn't destructive so it applies right away;
/// stopping is, so it always goes through the "Are you sure?" dialog.
pub(crate) fn action_toggle_start_stop(app: &mut App) {
    if let Some(sb) = app.selected_sandbox().cloned() {
        match sb.status {
            Status::Stopped => {
                app.run_action(SandboxAction::Start, &sb.name);
                app.notify(format!("Starting '{}'…", sb.name), false);
            }
            Status::Running => {
                app.confirm = Some(PendingAction::StopSandbox(sb.name));
            }
            _ => {
                app.notify("Sandbox can't be started or stopped right now", true);
            }
        }
    }
}

pub(crate) fn action_terminate(app: &mut App) {
    if let Some(sb) = app.selected_sandbox().cloned() {
        if sb.status == Status::Running {
            app.confirm = Some(PendingAction::TerminateSandbox(sb.name));
        } else {
            app.notify("Sandbox is not running", true);
        }
    }
}

pub(crate) fn action_remove(app: &mut App) {
    if let Some(sb) = app.selected_sandbox().cloned() {
        if sb.status != Status::Running {
            app.confirm = Some(PendingAction::RemoveSandbox(sb.name));
        } else {
            app.notify("Stop the sandbox before removing", true);
        }
    }
}

/// Open the "Exec" dialog for the selected sandbox, if it's running.
pub(crate) fn action_exec(app: &mut App) {
    if let Some(sb) = app.selected_sandbox().cloned() {
        if sb.status == Status::Running {
            app.exec_dialog = super::ExecDialog::open(sb.name);
        } else {
            app.notify("Sandbox is not running", true);
        }
    }
}

/// Handle a keypress while the "Are you sure?" confirmation dialog is open.
pub(crate) fn handle_confirm_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
            if let Some(action) = app.confirm.take() {
                execute_pending_action(app, action);
            }
        }
        KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
            app.confirm = None;
        }
        _ => {}
    }
}

/// Run a confirmed destructive action.
pub(crate) fn execute_pending_action(app: &mut App, action: PendingAction) {
    match action {
        PendingAction::StopSandbox(name) => {
            app.run_action(SandboxAction::Stop, &name);
            app.notify(format!("Stopping '{name}'…"), false);
        }
        PendingAction::TerminateSandbox(name) => {
            app.run_action(SandboxAction::Terminate, &name);
            app.notify(format!("Terminating '{name}'…"), false);
        }
        PendingAction::RemoveSandbox(name) => {
            app.run_action(SandboxAction::Remove, &name);
            app.notify(format!("Removing '{name}'…"), false);
        }
        PendingAction::RemoveVolume(name) => {
            let tx = app.msg_tx.clone();
            tokio::spawn(async move {
                let result = crate::sandbox::remove_volume(&name).await;
                let (msg, is_err) = match result {
                    Ok(()) => (format!("Removed volume '{name}'"), false),
                    Err(e) => (format!("Remove volume failed: {e}"), true),
                };
                let _ = tx.send(AppMessage::Notification(msg, is_err));
                if let Ok(list) = crate::sandbox::list_volumes().await {
                    let _ = tx.send(AppMessage::VolumeList(Ok(list)));
                }
            });
        }
    }
}
