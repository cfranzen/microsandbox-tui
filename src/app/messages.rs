//! Background message type and the logic that applies each message to
//! [`App`] state.
//!
//! Background tasks (list/log/metrics/filesystem polling, sandbox actions)
//! run on separate tokio tasks and report their results back to the main
//! loop as an [`AppMessage`], picked up by [`App::handle_message`].

use std::time::Instant;

use anyhow::Result;
use microsandbox::sandbox::LogEntry;

use crate::sandbox::{FsEntry, MetricsSnapshot, SandboxInfo, VolumeInfo};

use super::{App, MAX_LOG_LINES, METRICS_HISTORY_LEN};

/// Background messages sent from async tasks to the main loop.
pub enum AppMessage {
    SandboxList(Result<Vec<SandboxInfo>>),
    LogEntries(String, Result<Vec<LogEntry>>),
    /// A single log entry pushed live by a running log-stream task.
    LogStreamEntry(String, LogEntry),
    Metrics(String, Result<Option<MetricsSnapshot>>),
    FsEntries(String, String, Result<Option<Vec<FsEntry>>>),
    Notification(String, bool),
    /// Result of refreshing the named-volumes list.
    VolumeList(Result<Vec<VolumeInfo>>),
}

impl App {
    /// Handle an incoming background message.
    pub fn handle_message(&mut self, msg: AppMessage) {
        match msg {
            AppMessage::SandboxList(Ok(list)) => {
                // Preserve selection when list changes
                let selected_name = self.sandboxes.get(self.selected).map(|s| s.name.clone());
                self.sandboxes = list;
                if let Some(name) = selected_name {
                    if let Some(idx) = self.sandboxes.iter().position(|s| s.name == name) {
                        self.selected = idx;
                    } else {
                        self.selected = self.selected.min(self.sandboxes.len());
                    }
                }
                self.last_refresh = Some(Instant::now());
                self.sync_log_stream();
            }
            AppMessage::SandboxList(Err(e)) => {
                self.notify(format!("List error: {e}"), true);
            }
            AppMessage::LogEntries(name, Ok(entries)) => {
                self.logs.insert(name, entries);
            }
            AppMessage::LogEntries(name, Err(e)) => {
                self.notify(format!("Log error for {name}: {e}"), true);
            }
            AppMessage::LogStreamEntry(name, entry) => {
                let entries = self.logs.entry(name).or_default();
                entries.push(entry);
                if entries.len() > MAX_LOG_LINES {
                    let excess = entries.len() - MAX_LOG_LINES;
                    entries.drain(0..excess);
                }
            }
            AppMessage::Metrics(name, Ok(Some(m))) => {
                let history = self.metrics_history.entry(name.clone()).or_default();
                history.push_back(m.clone());
                if history.len() > METRICS_HISTORY_LEN {
                    history.pop_front();
                }
                self.metrics.insert(name, m);
            }
            AppMessage::Metrics(_, Ok(None)) => {}
            AppMessage::Metrics(name, Err(e)) => {
                self.notify(format!("Metrics error for {name}: {e}"), true);
            }
            AppMessage::FsEntries(name, path, Ok(Some(entries))) => {
                self.fs_entries.insert((name, path), entries);
            }
            AppMessage::FsEntries(_, _, Ok(None)) => {}
            AppMessage::FsEntries(name, path, Err(e)) => {
                self.notify(format!("FS error for {name}:{path}: {e}"), true);
            }
            AppMessage::Notification(msg, is_err) => {
                self.notify(msg, is_err);
            }
            AppMessage::VolumeList(Ok(list)) => {
                self.volumes_view.volumes = list;
                if self.volumes_view.selected >= self.volumes_view.volumes.len() {
                    self.volumes_view.selected = self.volumes_view.volumes.len().saturating_sub(1);
                }
            }
            AppMessage::VolumeList(Err(e)) => {
                self.notify(format!("Volume list error: {e}"), true);
            }
        }
    }
}
