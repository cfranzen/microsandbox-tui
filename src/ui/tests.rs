//! UI rendering tests using ratatui's TestBackend.
//!
//! All tests are async (`#[tokio::test]`) so that render functions that
//! trigger background data fetches via `tokio::spawn` work correctly inside
//! tests.  Where possible, caches (logs, metrics, fs_entries) are
//! pre-populated so the render code does not need to spawn at all.

use ratatui::{Terminal, backend::TestBackend};
use tokio::sync::mpsc;

use crate::app::{App, AppMessage, CreateDialog, DetailTab, Focus};
use crate::sandbox::{FsEntry, LocalFsEntryKind, MetricsSnapshot, SandboxInfo, SandboxStatus};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_app() -> App {
    let (tx, _rx) = mpsc::unbounded_channel::<AppMessage>();
    App::new(tx)
}

fn make_sandbox(name: &str, status: SandboxStatus) -> SandboxInfo {
    SandboxInfo {
        name: name.into(),
        status,
        image: "alpine:latest".into(),
        cpus: 1,
        memory_mib: 512,
        created_at: None,
        updated_at: None,
    }
}

fn make_terminal() -> Terminal<TestBackend> {
    let backend = TestBackend::new(120, 40);
    Terminal::new(backend).expect("terminal")
}

fn render_to_string(terminal: &mut Terminal<TestBackend>, app: &mut App) -> String {
    terminal
        .draw(|f| crate::ui::render(f, app))
        .expect("draw");
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol().to_owned())
        .collect()
}

/// Pre-populate an empty log cache entry so the logs renderer does not spawn.
fn seed_logs(app: &mut App, name: &str) {
    app.logs.insert(name.into(), vec![]);
}

/// Pre-populate a metrics cache entry so the metrics renderer does not spawn.
fn seed_metrics(app: &mut App, name: &str) {
    app.metrics.insert(name.into(), MetricsSnapshot::default());
}

/// Pre-populate a filesystem cache entry so the fs renderer does not spawn.
fn seed_fs(app: &mut App, name: &str) {
    app.fs_entries.insert(
        (name.into(), "/".into()),
        vec![FsEntry { path: "/etc".into(), kind: LocalFsEntryKind::Directory, size: 0 }],
    );
}

// ── Full render smoke tests ───────────────────────────────────────────────────

#[tokio::test]
async fn test_render_empty_state_no_panic() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    render_to_string(&mut terminal, &mut app);
}

#[tokio::test]
async fn test_render_header_contains_app_name() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    let buf = render_to_string(&mut terminal, &mut app);
    assert!(buf.contains("microsandbox"), "header should show app name; got: {buf:?}");
}

#[tokio::test]
async fn test_render_footer_contains_quit_hint() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    let buf = render_to_string(&mut terminal, &mut app);
    assert!(buf.contains('q'), "footer should contain quit hint");
}

#[tokio::test]
async fn test_render_with_running_sandbox() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("runner", SandboxStatus::Running));
    seed_logs(&mut app, "runner");
    let buf = render_to_string(&mut terminal, &mut app);
    assert!(buf.contains("runner"), "sandbox name should appear");
}

#[tokio::test]
async fn test_render_with_stopped_sandbox() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("stopper", SandboxStatus::Stopped));
    seed_logs(&mut app, "stopper");
    let buf = render_to_string(&mut terminal, &mut app);
    assert!(buf.contains("stopper"));
}

#[tokio::test]
async fn test_render_with_multiple_sandboxes() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("alpha", SandboxStatus::Running));
    app.sandboxes.push(make_sandbox("beta", SandboxStatus::Stopped));
    seed_logs(&mut app, "alpha");
    let buf = render_to_string(&mut terminal, &mut app);
    assert!(buf.contains("alpha"));
    assert!(buf.contains("beta"));
}

// ── Logs tab ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_render_logs_tab_no_panic() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("box", SandboxStatus::Running));
    app.tab = DetailTab::Logs;
    seed_logs(&mut app, "box");
    render_to_string(&mut terminal, &mut app);
}

#[tokio::test]
async fn test_render_logs_tab_shows_tab_label() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("box", SandboxStatus::Running));
    app.tab = DetailTab::Logs;
    seed_logs(&mut app, "box");
    let buf = render_to_string(&mut terminal, &mut app);
    assert!(buf.contains("Logs"));
}

// ── Metrics tab ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_render_metrics_tab_no_panic() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("box", SandboxStatus::Running));
    app.tab = DetailTab::Metrics;
    seed_metrics(&mut app, "box");
    render_to_string(&mut terminal, &mut app);
}

#[tokio::test]
async fn test_render_metrics_tab_shows_label() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("box", SandboxStatus::Running));
    app.tab = DetailTab::Metrics;
    seed_metrics(&mut app, "box");
    let buf = render_to_string(&mut terminal, &mut app);
    assert!(buf.contains("Metrics"));
}

#[tokio::test]
async fn test_render_metrics_tab_with_data() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("box", SandboxStatus::Running));
    app.tab = DetailTab::Metrics;
    app.metrics.insert(
        "box".into(),
        MetricsSnapshot {
            cpu_percent: 55.0,
            memory_bytes: 128 * 1024 * 1024,
            uptime_secs: 120,
            ..Default::default()
        },
    );
    render_to_string(&mut terminal, &mut app);
}

// ── Filesystem tab ────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_render_filesystem_tab_no_panic() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("box", SandboxStatus::Running));
    app.tab = DetailTab::Filesystem;
    seed_fs(&mut app, "box");
    render_to_string(&mut terminal, &mut app);
}

#[tokio::test]
async fn test_render_filesystem_tab_shows_label() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("box", SandboxStatus::Running));
    app.tab = DetailTab::Filesystem;
    seed_fs(&mut app, "box");
    let buf = render_to_string(&mut terminal, &mut app);
    assert!(buf.contains("Filesystem"));
}

#[tokio::test]
async fn test_render_filesystem_tab_with_entries() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("box", SandboxStatus::Running));
    app.tab = DetailTab::Filesystem;
    app.fs_entries.insert(
        ("box".into(), "/".into()),
        vec![
            FsEntry { path: "/etc".into(), kind: LocalFsEntryKind::Directory, size: 0 },
            FsEntry { path: "/bin/sh".into(), kind: LocalFsEntryKind::File, size: 4096 },
        ],
    );
    let buf = render_to_string(&mut terminal, &mut app);
    assert!(buf.contains("etc") || buf.contains("/etc"));
}

// ── Info tab ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_render_info_tab_no_panic() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("infobox", SandboxStatus::Stopped));
    app.tab = DetailTab::Info;
    render_to_string(&mut terminal, &mut app);
}

#[tokio::test]
async fn test_render_info_tab_shows_sandbox_name() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("infobox", SandboxStatus::Stopped));
    app.tab = DetailTab::Info;
    let buf = render_to_string(&mut terminal, &mut app);
    assert!(buf.contains("infobox"));
}

// ── Create dialog ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_render_create_dialog_no_panic() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    render_to_string(&mut terminal, &mut app);
}

#[tokio::test]
async fn test_render_create_dialog_shows_title() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    let buf = render_to_string(&mut terminal, &mut app);
    assert!(buf.contains("New Sandbox"));
}

#[tokio::test]
async fn test_render_create_dialog_shows_field_labels() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    let buf = render_to_string(&mut terminal, &mut app);
    assert!(buf.contains("Name") || buf.contains("Image"));
}

#[tokio::test]
async fn test_render_create_dialog_is_closed_by_default() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    // dialog.visible == false by default
    assert!(!app.create_dialog.visible);
    render_to_string(&mut terminal, &mut app); // must not panic
}

#[tokio::test]
async fn test_render_create_dialog_shows_error() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.error = Some("Name is required".into());
    let buf = render_to_string(&mut terminal, &mut app);
    assert!(buf.contains("Name is required"));
}

// ── Focus rendering ───────────────────────────────────────────────────────────

#[tokio::test]
async fn test_render_with_detail_focus_no_panic() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("box", SandboxStatus::Running));
    app.focus = Focus::Detail;
    seed_logs(&mut app, "box");
    render_to_string(&mut terminal, &mut app);
}

#[tokio::test]
async fn test_render_no_sandbox_selected_detail_empty() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.focus = Focus::Detail;
    render_to_string(&mut terminal, &mut app); // must not panic
}

// ── Notification bar ──────────────────────────────────────────────────────────

#[tokio::test]
async fn test_render_notification_visible_in_footer() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.notify("Sandbox started", false);
    let buf = render_to_string(&mut terminal, &mut app);
    assert!(buf.contains("Sandbox started"));
}

#[tokio::test]
async fn test_render_error_notification_visible() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.notify("Something went wrong", true);
    let buf = render_to_string(&mut terminal, &mut app);
    assert!(buf.contains("Something went wrong"));
}

// ── Edge cases ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_render_very_small_terminal_no_panic() {
    let backend = TestBackend::new(20, 5);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = make_app();
    terminal.draw(|f| crate::ui::render(f, &mut app)).expect("draw");
}

#[tokio::test]
async fn test_render_all_tabs_sequentially_no_panic() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("box", SandboxStatus::Running));
    seed_logs(&mut app, "box");
    seed_metrics(&mut app, "box");
    seed_fs(&mut app, "box");
    for tab in DetailTab::all() {
        app.tab = *tab;
        render_to_string(&mut terminal, &mut app);
    }
}

#[tokio::test]
async fn test_render_many_sandboxes_no_panic() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    for i in 0..20 {
        let status = if i % 2 == 0 { SandboxStatus::Running } else { SandboxStatus::Stopped };
        app.sandboxes.push(make_sandbox(&format!("sandbox-{i:02}"), status));
    }
    seed_logs(&mut app, "sandbox-00");
    render_to_string(&mut terminal, &mut app);
}

#[tokio::test]
async fn test_render_crashed_sandbox_no_panic() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("crasher", SandboxStatus::Crashed));
    seed_logs(&mut app, "crasher");
    let buf = render_to_string(&mut terminal, &mut app);
    assert!(buf.contains("crasher"));
}
