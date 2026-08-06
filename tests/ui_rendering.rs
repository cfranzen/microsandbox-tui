//! Integration tests for the full UI rendering pipeline.
//!
//! Uses ratatui's [`TestBackend`] to render frames into an in-memory buffer
//! and asserts that key text is present.  Tests are async (`#[tokio::test]`)
//! because some render functions trigger background data fetches via
//! `tokio::spawn` when their caches are empty.  Where possible, caches are
//! pre-seeded to keep tests deterministic and spawn-free.

use ratatui::{backend::TestBackend, Terminal};
use tokio::sync::mpsc;

use microsandbox_tui::app::{App, AppMessage, CreateDialog, DetailTab, Focus};
use microsandbox_tui::sandbox::{
    FsEntry, LocalFsEntryKind, MetricsSnapshot, SandboxInfo, SandboxStatus,
};

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
        workdir: None,
    }
}

fn make_terminal() -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(120, 40)).expect("terminal")
}

fn render_to_string(terminal: &mut Terminal<TestBackend>, app: &mut App) -> String {
    terminal
        .draw(|f| microsandbox_tui::ui::render(f, app))
        .expect("draw");
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol().to_owned())
        .collect()
}

fn seed_logs(app: &mut App, name: &str) {
    app.logs.insert(name.into(), vec![]);
}

fn seed_metrics(app: &mut App, name: &str) {
    app.metrics.insert(name.into(), MetricsSnapshot::default());
}

fn seed_fs(app: &mut App, name: &str) {
    app.fs_entries.insert(
        (name.into(), "/".into()),
        vec![FsEntry {
            path: "/etc".into(),
            kind: LocalFsEntryKind::Directory,
            size: 0,
        }],
    );
}

// ── Smoke tests ───────────────────────────────────────────────────────────────

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
    assert!(buf.contains("MicroSandbox"), "header should show app name");
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
    app.sandboxes
        .push(make_sandbox("runner", SandboxStatus::Running));
    seed_logs(&mut app, "runner");
    let buf = render_to_string(&mut terminal, &mut app);
    assert!(buf.contains("runner"));
}

#[tokio::test]
async fn test_render_with_stopped_sandbox() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes
        .push(make_sandbox("stopper", SandboxStatus::Stopped));
    seed_logs(&mut app, "stopper");
    let buf = render_to_string(&mut terminal, &mut app);
    assert!(buf.contains("stopper"));
}

#[tokio::test]
async fn test_render_with_crashed_sandbox() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes
        .push(make_sandbox("crasher", SandboxStatus::Crashed));
    seed_logs(&mut app, "crasher");
    let buf = render_to_string(&mut terminal, &mut app);
    assert!(buf.contains("crasher"));
}

#[tokio::test]
async fn test_render_with_multiple_sandboxes() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes
        .push(make_sandbox("alpha", SandboxStatus::Running));
    app.sandboxes
        .push(make_sandbox("beta", SandboxStatus::Stopped));
    seed_logs(&mut app, "alpha");
    let buf = render_to_string(&mut terminal, &mut app);
    assert!(buf.contains("alpha"));
    assert!(buf.contains("beta"));
}

// ── Detail tabs ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_render_logs_tab_no_panic() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes
        .push(make_sandbox("box", SandboxStatus::Running));
    app.tab = DetailTab::Logs;
    seed_logs(&mut app, "box");
    render_to_string(&mut terminal, &mut app);
}

#[tokio::test]
async fn test_render_logs_tab_shows_label() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes
        .push(make_sandbox("box", SandboxStatus::Running));
    app.tab = DetailTab::Logs;
    seed_logs(&mut app, "box");
    let buf = render_to_string(&mut terminal, &mut app);
    assert!(buf.contains("Logs"));
}

#[tokio::test]
async fn test_render_info_tab_metrics_no_panic() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes
        .push(make_sandbox("box", SandboxStatus::Running));
    app.tab = DetailTab::Info;
    seed_metrics(&mut app, "box");
    render_to_string(&mut terminal, &mut app);
}

#[tokio::test]
async fn test_render_info_tab_shows_metrics_label() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes
        .push(make_sandbox("box", SandboxStatus::Running));
    app.tab = DetailTab::Info;
    seed_metrics(&mut app, "box");
    let buf = render_to_string(&mut terminal, &mut app);
    assert!(buf.contains("CPU"));
}

#[tokio::test]
async fn test_render_info_tab_with_metrics_data() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes
        .push(make_sandbox("box", SandboxStatus::Running));
    app.tab = DetailTab::Info;
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

#[tokio::test]
async fn test_render_filesystem_tab_no_panic() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes
        .push(make_sandbox("box", SandboxStatus::Running));
    app.tab = DetailTab::Filesystem;
    seed_fs(&mut app, "box");
    render_to_string(&mut terminal, &mut app);
}

#[tokio::test]
async fn test_render_filesystem_tab_shows_label() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes
        .push(make_sandbox("box", SandboxStatus::Running));
    app.tab = DetailTab::Filesystem;
    seed_fs(&mut app, "box");
    let buf = render_to_string(&mut terminal, &mut app);
    assert!(buf.contains("Filesystem"));
}

#[tokio::test]
async fn test_render_filesystem_tab_with_entries() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes
        .push(make_sandbox("box", SandboxStatus::Running));
    app.tab = DetailTab::Filesystem;
    app.fs_entries.insert(
        ("box".into(), "/".into()),
        vec![
            FsEntry {
                path: "/etc".into(),
                kind: LocalFsEntryKind::Directory,
                size: 0,
            },
            FsEntry {
                path: "/bin/sh".into(),
                kind: LocalFsEntryKind::File,
                size: 4096,
            },
        ],
    );
    let buf = render_to_string(&mut terminal, &mut app);
    assert!(buf.contains("etc") || buf.contains("/etc"));
}

#[tokio::test]
async fn test_render_info_tab_no_panic() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes
        .push(make_sandbox("infobox", SandboxStatus::Stopped));
    app.tab = DetailTab::Info;
    render_to_string(&mut terminal, &mut app);
}

#[tokio::test]
async fn test_render_info_tab_shows_sandbox_name() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes
        .push(make_sandbox("infobox", SandboxStatus::Stopped));
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
async fn test_render_create_dialog_closed_by_default() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    assert!(!app.create_dialog.visible);
    render_to_string(&mut terminal, &mut app);
}

#[tokio::test]
async fn test_render_create_dialog_shows_validation_error() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.error = Some("Name is required".into());
    let buf = render_to_string(&mut terminal, &mut app);
    assert!(buf.contains("Name is required"));
}

// ── Focus and panel state ─────────────────────────────────────────────────────

#[tokio::test]
async fn test_render_detail_focus_no_panic() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes
        .push(make_sandbox("box", SandboxStatus::Running));
    app.focus = Focus::Detail;
    seed_logs(&mut app, "box");
    render_to_string(&mut terminal, &mut app);
}

#[tokio::test]
async fn test_render_empty_list_detail_focus_no_panic() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.focus = Focus::Detail;
    render_to_string(&mut terminal, &mut app);
}

// ── Notification bar ──────────────────────────────────────────────────────────

#[tokio::test]
async fn test_render_info_notification_in_footer() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.notify("Sandbox started", false);
    let buf = render_to_string(&mut terminal, &mut app);
    assert!(buf.contains("Sandbox started"));
}

#[tokio::test]
async fn test_render_error_notification_in_footer() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.notify("Something went wrong", true);
    let buf = render_to_string(&mut terminal, &mut app);
    assert!(buf.contains("Something went wrong"));
}

// ── Edge cases ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_render_very_small_terminal_no_panic() {
    let mut terminal = Terminal::new(TestBackend::new(20, 5)).expect("terminal");
    let mut app = make_app();
    terminal
        .draw(|f| microsandbox_tui::ui::render(f, &mut app))
        .expect("draw");
}

#[tokio::test]
async fn test_render_all_tabs_sequentially_no_panic() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    app.sandboxes
        .push(make_sandbox("box", SandboxStatus::Running));
    seed_logs(&mut app, "box");
    seed_metrics(&mut app, "box");
    seed_fs(&mut app, "box");
    for tab in DetailTab::all() {
        app.tab = *tab;
        render_to_string(&mut terminal, &mut app);
    }
}

#[tokio::test]
async fn test_render_twenty_sandboxes_no_panic() {
    let mut terminal = make_terminal();
    let mut app = make_app();
    for i in 0..20 {
        let status = if i % 2 == 0 {
            SandboxStatus::Running
        } else {
            SandboxStatus::Stopped
        };
        app.sandboxes
            .push(make_sandbox(&format!("sandbox-{i:02}"), status));
    }
    seed_logs(&mut app, "sandbox-00");
    render_to_string(&mut terminal, &mut app);
}
