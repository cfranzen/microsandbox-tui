//! Unit tests for application state and input handling.

use super::*;
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};

use super::keys::validate_cidr;
use crate::sandbox::{
    MountSource, NetRuleAction, NetRuleDirection, NetworkRule, VolumeInfo, VolumeMountConfig,
};

// ── Helpers ─────────────────────────────────────────────────────────────

/// Build an App with a disconnected message channel (the sender is kept alive
/// so the channel stays open; the receiver is dropped and never polled).
fn make_app() -> App {
    let (tx, _rx) = mpsc::unbounded_channel();
    App::new(tx)
}

/// Construct a sandbox info with the given name and status.
fn make_sandbox(name: &str, status: Status) -> SandboxInfo {
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

/// Build a key-press Event.
fn key_press(code: KeyCode) -> Event {
    Event::Key(KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    })
}

/// Build a key-press Event with modifiers.
fn key_press_mod(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    })
}

/// Build a key-release Event (should be ignored by handle_event).
fn key_release(code: KeyCode) -> Event {
    Event::Key(KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Release,
        state: KeyEventState::NONE,
    })
}

/// Build a mouse Event at the given column/row.
fn mouse_event(kind: MouseEventKind, column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

// ── App initial state ────────────────────────────────────────────────────

#[test]
fn test_initial_state() {
    let app = make_app();
    assert!(app.sandboxes.is_empty());
    assert_eq!(app.selected, 0);
    assert_eq!(app.focus, Focus::SandboxList);
    assert_eq!(app.tab, DetailTab::Logs);
    assert!(!app.should_quit);
    assert!(!app.create_dialog.visible);
    assert!(app.notification.is_none());
    assert_eq!(app.fs_path, "/");
}

// ── selected_sandbox ────────────────────────────────────────────────────

#[test]
fn test_selected_sandbox_empty() {
    let app = make_app();
    assert!(app.selected_sandbox().is_none());
}

#[test]
fn test_selected_sandbox_first() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("alpha", Status::Running));
    assert_eq!(app.selected_sandbox().unwrap().name, "alpha");
}

#[test]
fn test_selected_sandbox_second() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("alpha", Status::Running));
    app.sandboxes.push(make_sandbox("beta", Status::Stopped));
    app.selected = 1;
    assert_eq!(app.selected_sandbox().unwrap().name, "beta");
}

// ── new_sandbox_selected ─────────────────────────────────────────────────

#[test]
fn test_new_sandbox_selected_empty_list() {
    // With no sandboxes selected==0==len() → new sandbox is selected
    let app = make_app();
    assert!(app.new_sandbox_selected());
}

#[test]
fn test_new_sandbox_selected_false_when_sandbox_focused() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("alpha", Status::Running));
    assert!(!app.new_sandbox_selected());
}

#[test]
fn test_new_sandbox_selected_true_at_end() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("alpha", Status::Running));
    app.selected = 1; // == len()
    assert!(app.new_sandbox_selected());
}

// ── log_stream_target ────────────────────────────────────────────────────

#[test]
fn test_log_stream_target_none_when_not_on_logs_tab() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("alpha", Status::Running));
    app.tab = DetailTab::Info;
    assert!(app.log_stream_target().is_none());
}

#[test]
fn test_log_stream_target_none_when_sandbox_stopped() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("alpha", Status::Stopped));
    app.tab = DetailTab::Logs;
    assert!(app.log_stream_target().is_none());
}

#[test]
fn test_log_stream_target_some_when_running_on_logs_tab() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("alpha", Status::Running));
    app.tab = DetailTab::Logs;
    assert_eq!(app.log_stream_target().as_deref(), Some("alpha"));
}

#[test]
fn test_log_stream_target_none_when_no_sandbox_selected() {
    let app = make_app();
    assert!(app.log_stream_target().is_none());
}

// ── LogStreamEntry message handling ──────────────────────────────────────

#[test]
fn test_handle_message_log_stream_entry_appends() {
    use microsandbox::logs::LogCursor;
    use microsandbox::sandbox::{LogEntry, LogSource};

    let mut app = make_app();
    let entry = LogEntry {
        timestamp: chrono::Utc::now(),
        source: LogSource::Stdout,
        session_id: None,
        data: b"hello".as_slice().into(),
        cursor: LogCursor::empty(),
    };
    app.handle_message(AppMessage::LogStreamEntry("alpha".into(), entry));
    assert_eq!(app.logs.get("alpha").map(|v| v.len()), Some(1));
}

#[test]
fn test_handle_message_log_stream_entry_caps_at_max_lines() {
    use microsandbox::logs::LogCursor;
    use microsandbox::sandbox::{LogEntry, LogSource};

    let mut app = make_app();
    for _ in 0..(MAX_LOG_LINES + 10) {
        let entry = LogEntry {
            timestamp: chrono::Utc::now(),
            source: LogSource::Stdout,
            session_id: None,
            data: b"x".as_slice().into(),
            cursor: LogCursor::empty(),
        };
        app.handle_message(AppMessage::LogStreamEntry("alpha".into(), entry));
    }
    assert_eq!(app.logs.get("alpha").map(|v| v.len()), Some(MAX_LOG_LINES));
}

// ── metrics history ───────────────────────────────────────────────────────

#[test]
fn test_handle_message_metrics_appends_history() {
    let mut app = make_app();
    for i in 0..3 {
        let m = MetricsSnapshot {
            cpu_percent: i as f64,
            ..Default::default()
        };
        app.handle_message(AppMessage::Metrics("alpha".into(), Ok(Some(m))));
    }
    let history = app.metrics_history.get("alpha").unwrap();
    assert_eq!(history.len(), 3);
    assert_eq!(history.back().unwrap().cpu_percent, 2.0);
}

#[test]
fn test_handle_message_metrics_history_caps_at_limit() {
    let mut app = make_app();
    for _ in 0..(METRICS_HISTORY_LEN + 15) {
        let m = MetricsSnapshot::default();
        app.handle_message(AppMessage::Metrics("alpha".into(), Ok(Some(m))));
    }
    let history = app.metrics_history.get("alpha").unwrap();
    assert_eq!(history.len(), METRICS_HISTORY_LEN);
}

// ── select_next / select_prev ────────────────────────────────────────────

#[test]
fn test_select_next_empty_does_nothing() {
    let mut app = make_app();
    app.select_next();
    assert_eq!(app.selected, 0);
}

#[test]
fn test_select_next_advances() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("a", Status::Running));
    app.sandboxes.push(make_sandbox("b", Status::Running));
    app.select_next();
    assert_eq!(app.selected, 1);
}

#[test]
fn test_select_next_can_reach_new_sandbox_slot() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("a", Status::Running));
    app.select_next(); // selected = 1 == len() → "New Sandbox"
    assert!(app.new_sandbox_selected());
}

#[test]
fn test_select_next_stops_at_new_sandbox_slot() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("a", Status::Running));
    app.selected = 1; // already at "New Sandbox"
    app.select_next();
    assert_eq!(app.selected, 1); // stays
}

#[test]
fn test_select_prev_stops_at_zero() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("a", Status::Running));
    app.select_prev(); // already at 0
    assert_eq!(app.selected, 0);
}

#[test]
fn test_select_prev_moves_back() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("a", Status::Running));
    app.sandboxes.push(make_sandbox("b", Status::Running));
    app.selected = 1;
    app.select_prev();
    assert_eq!(app.selected, 0);
}

// ── next_tab / prev_tab ──────────────────────────────────────────────────

#[test]
fn test_next_tab_cycles_forward() {
    let mut app = make_app();
    assert_eq!(app.tab, DetailTab::Logs);
    app.next_tab();
    assert_eq!(app.tab, DetailTab::Filesystem);
    app.next_tab();
    assert_eq!(app.tab, DetailTab::Info);
    app.next_tab();
    assert_eq!(app.tab, DetailTab::Logs); // wraps
}

#[test]
fn test_prev_tab_cycles_backward() {
    let mut app = make_app();
    app.prev_tab();
    assert_eq!(app.tab, DetailTab::Info); // wraps
    app.prev_tab();
    assert_eq!(app.tab, DetailTab::Filesystem);
    app.prev_tab();
    assert_eq!(app.tab, DetailTab::Logs);
}

#[test]
fn test_tab_switch_resets_scroll() {
    let mut app = make_app();
    app.log_scroll = 10;
    app.fs_scroll = 5;
    app.next_tab();
    assert_eq!(app.log_scroll, 0);
    assert_eq!(app.fs_scroll, 0);
}

// ── notify ───────────────────────────────────────────────────────────────

#[test]
fn test_notify_sets_message() {
    let mut app = make_app();
    app.notify("hello", false);
    let n = app.notification.as_ref().unwrap();
    assert_eq!(n.message, "hello");
    assert!(!n.is_error);
}

#[test]
fn test_notify_error_flag() {
    let mut app = make_app();
    app.notify("boom", true);
    assert!(app.notification.as_ref().unwrap().is_error);
}

#[test]
fn test_notify_replaces_previous() {
    let mut app = make_app();
    app.notify("first", false);
    app.notify("second", true);
    assert_eq!(app.notification.as_ref().unwrap().message, "second");
}

// ── CreateDialog ─────────────────────────────────────────────────────────

#[test]
fn test_create_dialog_default_is_closed() {
    let dlg = CreateDialog::default();
    assert!(!dlg.visible);
    assert_eq!(dlg.tab, DialogTab::Basic);
    assert_eq!(dlg.field, 0);
    assert!(dlg.name.is_empty());
}

#[test]
fn test_create_dialog_open_has_defaults() {
    let dlg = CreateDialog::open();
    assert!(dlg.visible);
    assert_eq!(dlg.field, 0);
    assert_eq!(dlg.image, "alpine");
    assert_eq!(dlg.cpus, "1");
    assert_eq!(dlg.memory, "512");
    assert_eq!(dlg.shell, "/bin/sh");
    assert!(dlg.name.is_empty());
    assert!(dlg.error.is_none());
}

#[test]
fn test_create_dialog_next_field_wraps() {
    let mut dlg = CreateDialog::open();
    dlg.next_field();
    assert_eq!(dlg.field, 1);
    dlg.next_field();
    assert_eq!(dlg.field, 2);
    dlg.next_field();
    assert_eq!(dlg.field, 3);
    dlg.next_field();
    assert_eq!(dlg.field, 4);
    dlg.next_field();
    assert_eq!(dlg.field, 5);
    dlg.next_field();
    assert_eq!(dlg.field, 6);
    dlg.next_field();
    assert_eq!(dlg.field, 7);
    dlg.next_field();
    assert_eq!(dlg.field, 8); // Create button
    dlg.next_field();
    assert_eq!(dlg.field, 0);
}

#[test]
fn test_create_dialog_current_field_mut() {
    let mut dlg = CreateDialog::open();
    dlg.field = 0;
    dlg.current_field_mut().unwrap().push_str("mybox");
    assert_eq!(dlg.name, "mybox");
    dlg.field = 1;
    dlg.current_field_mut().unwrap().push_str("ubuntu");
    assert_eq!(dlg.image, "alpineubuntu");
    dlg.field = 2;
    dlg.current_field_mut().unwrap().push('4');
    assert_eq!(dlg.cpus, "14");
    dlg.field = 3;
    dlg.current_field_mut().unwrap().push_str("1024");
    assert_eq!(dlg.memory, "5121024");
    // Fields 4 (ports), 5 (env vars), and 6 (workdir) are managed by
    // sub-dialogs / the directory picker rather than direct text input.
    dlg.field = 4;
    assert!(dlg.current_field_mut().is_none());
    dlg.field = 5;
    assert!(dlg.current_field_mut().is_none());
    dlg.field = 6;
    assert!(dlg.current_field_mut().is_none());
}

// ── DetailTab helpers ────────────────────────────────────────────────────

#[test]
fn test_detail_tab_titles() {
    assert_eq!(DetailTab::Logs.title(), "Logs");
    assert_eq!(DetailTab::Filesystem.title(), "Filesystem");
    assert_eq!(DetailTab::Info.title(), "Info");
}

#[test]
fn test_detail_tab_all_has_three_entries() {
    assert_eq!(DetailTab::all().len(), 3);
}

// ── handle_message ───────────────────────────────────────────────────────

#[test]
fn test_handle_message_sandbox_list_populates() {
    let mut app = make_app();
    let list = vec![make_sandbox("alpha", Status::Running)];
    app.handle_message(AppMessage::SandboxList(Ok(list)));
    assert_eq!(app.sandboxes.len(), 1);
    assert_eq!(app.sandboxes[0].name, "alpha");
    assert!(app.last_refresh.is_some());
}

#[test]
fn test_handle_message_sandbox_list_error_notifies() {
    let mut app = make_app();
    app.handle_message(AppMessage::SandboxList(Err(anyhow::anyhow!(
        "conn refused"
    ))));
    let n = app.notification.as_ref().unwrap();
    assert!(n.is_error);
    assert!(n.message.contains("conn refused"));
}

#[test]
fn test_handle_message_preserves_selection_by_name() {
    let mut app = make_app();
    app.sandboxes = vec![
        make_sandbox("alpha", Status::Running),
        make_sandbox("beta", Status::Stopped),
    ];
    app.selected = 1; // "beta"

    // Refresh with beta still present but in a new position (alpha removed)
    let new_list = vec![make_sandbox("beta", Status::Running)];
    app.handle_message(AppMessage::SandboxList(Ok(new_list)));
    assert_eq!(app.selected, 0); // beta moved to index 0
    assert_eq!(app.sandboxes[app.selected].name, "beta");
}

#[test]
fn test_handle_message_selection_clamps_when_sandbox_removed() {
    let mut app = make_app();
    app.sandboxes = vec![
        make_sandbox("alpha", Status::Running),
        make_sandbox("beta", Status::Stopped),
    ];
    app.selected = 1; // "beta" — will be removed

    let new_list = vec![make_sandbox("alpha", Status::Running)];
    app.handle_message(AppMessage::SandboxList(Ok(new_list)));
    // beta gone, selected should clamp to len() == 1 (new sandbox slot)
    assert!(app.selected <= app.sandboxes.len());
}

#[test]
fn test_handle_message_log_entries_stored() {
    let mut app = make_app();
    app.handle_message(AppMessage::LogEntries("mybox".into(), Ok(vec![])));
    assert!(app.logs.contains_key("mybox"));
}

#[test]
fn test_handle_message_log_error_notifies() {
    let mut app = make_app();
    app.handle_message(AppMessage::LogEntries(
        "mybox".into(),
        Err(anyhow::anyhow!("err")),
    ));
    assert!(app.notification.as_ref().unwrap().is_error);
}

#[test]
fn test_handle_message_metrics_stored() {
    let mut app = make_app();
    let m = MetricsSnapshot {
        cpu_percent: 42.0,
        memory_bytes: 1024,
        ..Default::default()
    };
    app.handle_message(AppMessage::Metrics("mybox".into(), Ok(Some(m))));
    assert_eq!(app.metrics["mybox"].cpu_percent, 42.0);
}

#[test]
fn test_handle_message_metrics_none_no_panic() {
    let mut app = make_app();
    app.handle_message(AppMessage::Metrics("mybox".into(), Ok(None)));
    assert!(!app.metrics.contains_key("mybox"));
}

#[test]
fn test_handle_message_metrics_error_notifies() {
    let mut app = make_app();
    app.handle_message(AppMessage::Metrics(
        "mybox".into(),
        Err(anyhow::anyhow!("x")),
    ));
    assert!(app.notification.as_ref().unwrap().is_error);
}

#[test]
fn test_handle_message_fs_entries_stored() {
    let mut app = make_app();
    let entries = vec![crate::sandbox::FsEntry {
        path: "/etc".into(),
        kind: crate::sandbox::LocalFsEntryKind::Directory,
        size: 0,
    }];
    app.handle_message(AppMessage::FsEntries(
        "mybox".into(),
        "/".into(),
        Ok(Some(entries)),
    ));
    assert!(app.fs_entries.contains_key(&("mybox".into(), "/".into())));
}

#[test]
fn test_handle_message_fs_none_no_panic() {
    let mut app = make_app();
    app.handle_message(AppMessage::FsEntries("mybox".into(), "/".into(), Ok(None)));
    assert!(app.fs_entries.is_empty());
}

#[test]
fn test_handle_message_fs_error_notifies() {
    let mut app = make_app();
    app.handle_message(AppMessage::FsEntries(
        "mybox".into(),
        "/".into(),
        Err(anyhow::anyhow!("x")),
    ));
    assert!(app.notification.as_ref().unwrap().is_error);
}

#[test]
fn test_handle_message_notification() {
    let mut app = make_app();
    app.handle_message(AppMessage::Notification("done".into(), false));
    let n = app.notification.as_ref().unwrap();
    assert_eq!(n.message, "done");
    assert!(!n.is_error);
}

// ── handle_event: key-event filtering ───────────────────────────────────

#[test]
fn test_release_events_are_ignored() {
    let mut app = make_app();
    // Releasing 'q' must not quit
    handle_event(&mut app, key_release(KeyCode::Char('q')));
    assert!(!app.should_quit);
}

#[test]
fn test_non_key_events_are_ignored() {
    let mut app = make_app();
    handle_event(&mut app, Event::FocusGained);
    assert!(!app.should_quit);
}

// ── handle_event: global keys ────────────────────────────────────────────

#[test]
fn test_q_quits() {
    let mut app = make_app();
    handle_event(&mut app, key_press(KeyCode::Char('q')));
    assert!(app.should_quit);
}

#[test]
fn test_shift_q_quits() {
    let mut app = make_app();
    handle_event(&mut app, key_press(KeyCode::Char('Q')));
    assert!(app.should_quit);
}

#[test]
fn test_ctrl_c_quits() {
    let mut app = make_app();
    handle_event(
        &mut app,
        key_press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL),
    );
    assert!(app.should_quit);
}

// ── handle_event: Esc behaviour ──────────────────────────────────────────

#[test]
fn test_esc_closes_dialog() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    assert!(app.create_dialog.visible);
    handle_event(&mut app, key_press(KeyCode::Esc));
    assert!(!app.create_dialog.visible);
}

#[test]
fn test_esc_does_not_quit() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    handle_event(&mut app, key_press(KeyCode::Esc));
    assert!(!app.should_quit);
}

#[test]
fn test_esc_outside_dialog_moves_focus_to_list() {
    let mut app = make_app();
    app.focus = Focus::Detail;
    handle_event(&mut app, key_press(KeyCode::Esc));
    assert_eq!(app.focus, Focus::SandboxList);
}

// ── handle_event: dialog input ───────────────────────────────────────────

#[test]
fn test_dialog_tab_advances_field() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    handle_event(&mut app, key_press(KeyCode::Tab));
    assert_eq!(app.create_dialog.field, 1);
}

#[test]
fn test_dialog_backtab_goes_back() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.field = 2;
    handle_event(&mut app, key_press(KeyCode::BackTab));
    assert_eq!(app.create_dialog.field, 1);
}

#[test]
fn test_dialog_backtab_from_first_wraps_to_last() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    // field == 0
    handle_event(&mut app, key_press(KeyCode::BackTab));
    assert_eq!(app.create_dialog.field, 8); // Create button
}

#[test]
fn test_dialog_char_appends_to_field() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.field = 0; // name
    handle_event(&mut app, key_press(KeyCode::Char('x')));
    assert_eq!(app.create_dialog.name, "x");
}

#[test]
fn test_dialog_backspace_removes_char() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.name = "ab".into();
    handle_event(&mut app, key_press(KeyCode::Backspace));
    assert_eq!(app.create_dialog.name, "a");
}

#[test]
fn test_dialog_enter_with_empty_name_sets_error() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.name = "".into();

    app.create_dialog.field = app.create_dialog.form_field_count(); // Create button
    handle_event(&mut app, key_press(KeyCode::Enter));
    assert!(app.create_dialog.error.is_some());
    // Dialog stays open on validation error
    assert!(app.create_dialog.visible);
}

#[test]
fn test_dialog_enter_with_empty_image_sets_error() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.name = "mybox".into();
    app.create_dialog.image = "".into();

    app.create_dialog.field = app.create_dialog.form_field_count(); // Create button
    handle_event(&mut app, key_press(KeyCode::Enter));
    assert!(app.create_dialog.error.is_some());
    assert!(app.create_dialog.visible);
}

#[test]
fn test_dialog_left_right_switches_tab() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.field = 3;
    handle_event(&mut app, key_press(KeyCode::Right));
    assert_eq!(app.create_dialog.tab, DialogTab::Advanced);
    assert_eq!(app.create_dialog.field, 0);

    app.create_dialog.field = 4;
    handle_event(&mut app, key_press(KeyCode::Left));
    assert_eq!(app.create_dialog.tab, DialogTab::Basic);
    assert_eq!(app.create_dialog.field, 0);
}

#[test]
fn test_dialog_space_toggles_disable_network() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.switch_tab(DialogTab::Advanced);
    app.create_dialog.field = 5;
    handle_event(&mut app, key_press(KeyCode::Char(' ')));
    assert!(app.create_dialog.disable_network);
}

#[test]
fn test_dialog_ports_sub_dialog_add_entry() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.field = 4;
    handle_event(&mut app, key_press(KeyCode::Enter));
    assert!(app.create_dialog.ports_dialog.visible);
    handle_event(&mut app, key_press(KeyCode::Char('a')));
    for ch in "8080".chars() {
        handle_event(&mut app, key_press(KeyCode::Char(ch)));
    }
    handle_event(&mut app, key_press(KeyCode::Enter));
    for ch in "80".chars() {
        handle_event(&mut app, key_press(KeyCode::Char(ch)));
    }
    handle_event(&mut app, key_press(KeyCode::Enter));
    handle_event(&mut app, key_press(KeyCode::Esc));
    assert_eq!(app.create_dialog.ports, vec![(8080, 80)]);
}

#[test]
fn test_dialog_env_vars_sub_dialog_add_entry() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.field = 5;
    handle_event(&mut app, key_press(KeyCode::Enter));
    assert!(app.create_dialog.env_vars_dialog.visible);
    handle_event(&mut app, key_press(KeyCode::Char('a')));
    for ch in "FOO".chars() {
        handle_event(&mut app, key_press(KeyCode::Char(ch)));
    }
    handle_event(&mut app, key_press(KeyCode::Enter));
    for ch in "bar".chars() {
        handle_event(&mut app, key_press(KeyCode::Char(ch)));
    }
    handle_event(&mut app, key_press(KeyCode::Enter));
    handle_event(&mut app, key_press(KeyCode::Esc));
    assert_eq!(
        app.create_dialog.env_vars,
        vec![("FOO".to_string(), "bar".to_string())]
    );
}

#[test]
fn test_dialog_numeric_filter_on_advanced_max_cpus() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.switch_tab(DialogTab::Advanced);
    app.create_dialog.field = 3; // max_cpus
    handle_event(&mut app, key_press(KeyCode::Char('x')));
    assert!(app.create_dialog.error.is_some());
    assert!(app.create_dialog.max_cpus.is_empty());
}

#[test]
fn test_dialog_toggle_field_ignores_char_input() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.switch_tab(DialogTab::Advanced);
    app.create_dialog.field = 5;
    handle_event(&mut app, key_press(KeyCode::Char('x')));
    assert!(!app.create_dialog.disable_network);
    assert!(app.create_dialog.error.is_none());
}

#[test]
fn test_dialog_network_rules_sub_dialog_add_entry() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.switch_tab(DialogTab::Advanced);
    app.create_dialog.field = 6;
    handle_event(&mut app, key_press(KeyCode::Enter));
    assert!(app.create_dialog.network_rules_dialog.visible);
    handle_event(&mut app, key_press(KeyCode::Char('a')));
    handle_event(&mut app, key_press(KeyCode::Char('i')));
    handle_event(&mut app, key_press(KeyCode::Char(' ')));
    for ch in "10.0.0.0/8".chars() {
        handle_event(&mut app, key_press(KeyCode::Char(ch)));
    }
    handle_event(&mut app, key_press(KeyCode::Enter));
    handle_event(&mut app, key_press(KeyCode::Esc));
    assert_eq!(
        app.create_dialog.network_rules,
        vec![NetworkRule {
            cidr: "10.0.0.0/8".into(),
            action: NetRuleAction::Deny,
            direction: NetRuleDirection::Ingress,
        }]
    );
}

#[test]
fn test_dialog_network_rules_invalid_cidr_shows_error() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.switch_tab(DialogTab::Advanced);
    app.create_dialog.field = 6;
    handle_event(&mut app, key_press(KeyCode::Enter));
    handle_event(&mut app, key_press(KeyCode::Char('a')));
    for ch in "not-a-cidr".chars() {
        handle_event(&mut app, key_press(KeyCode::Char(ch)));
    }
    handle_event(&mut app, key_press(KeyCode::Enter));
    assert!(app.create_dialog.network_rules_dialog.error.is_some());
    assert!(app.create_dialog.network_rules_dialog.entries.is_empty());
}

#[test]
fn test_dialog_network_rules_delete_entry() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.network_rules_dialog = NetworkRulesDialog::open(vec![NetworkRule {
        cidr: "1.2.3.0/24".into(),
        action: NetRuleAction::Allow,
        direction: NetRuleDirection::Egress,
    }]);
    handle_event(&mut app, key_press(KeyCode::Char('d')));
    assert!(app.create_dialog.network_rules_dialog.entries.is_empty());
}

#[test]
fn test_validate_cidr_accepts_valid_ipv4() {
    assert!(validate_cidr("10.0.0.0/8").is_ok());
    assert!(validate_cidr("192.168.1.1/32").is_ok());
}

#[test]
fn test_validate_cidr_rejects_malformed() {
    assert!(validate_cidr("not-a-cidr").is_err());
    assert!(validate_cidr("10.0.0.0/99").is_err());
    assert!(validate_cidr("999.0.0.0/8").is_err());
    assert!(validate_cidr("10.0.0.0").is_err());
}

#[test]
fn test_dialog_mounts_sub_dialog_add_bind_entry() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.field = 7;
    handle_event(&mut app, key_press(KeyCode::Enter));
    assert!(app.create_dialog.mounts_dialog.visible);
    handle_event(&mut app, key_press(KeyCode::Char('a')));
    for ch in "/data".chars() {
        handle_event(&mut app, key_press(KeyCode::Char(ch)));
    }
    handle_event(&mut app, key_press(KeyCode::Enter));
    for ch in "/host/data".chars() {
        handle_event(&mut app, key_press(KeyCode::Char(ch)));
    }
    handle_event(&mut app, key_press(KeyCode::Enter));
    handle_event(&mut app, key_press(KeyCode::Esc));
    assert_eq!(
        app.create_dialog.mounts,
        vec![VolumeMountConfig {
            guest_path: "/data".into(),
            source: MountSource::Bind("/host/data".into()),
        }]
    );
}

#[test]
fn test_dialog_mounts_sub_dialog_add_named_entry() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.mounts_dialog = MountsDialog::open(vec![]);
    handle_event(&mut app, key_press(KeyCode::Char('a')));
    for ch in "/cache".chars() {
        handle_event(&mut app, key_press(KeyCode::Char(ch)));
    }
    handle_event(&mut app, key_press(KeyCode::Enter)); // move to source field
    handle_event(&mut app, key_press(KeyCode::Char('n'))); // choose Named kind
    for ch in "my-cache".chars() {
        handle_event(&mut app, key_press(KeyCode::Char(ch)));
    }
    handle_event(&mut app, key_press(KeyCode::Enter));
    assert_eq!(
        app.create_dialog.mounts_dialog.entries,
        vec![VolumeMountConfig {
            guest_path: "/cache".into(),
            source: MountSource::Named("my-cache".into()),
        }]
    );
}

#[test]
fn test_dialog_mounts_delete_entry() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.mounts_dialog = MountsDialog::open(vec![VolumeMountConfig {
        guest_path: "/data".into(),
        source: MountSource::Bind("/host".into()),
    }]);
    handle_event(&mut app, key_press(KeyCode::Char('d')));
    assert!(app.create_dialog.mounts_dialog.entries.is_empty());
}

#[tokio::test]
async fn test_v_key_opens_volumes_view() {
    let mut app = make_app();
    handle_event(&mut app, key_press(KeyCode::Char('v')));
    assert!(app.volumes_view.visible);
}

#[test]
fn test_volumes_view_esc_closes() {
    let mut app = make_app();
    app.volumes_view = VolumesView::open();
    handle_event(&mut app, key_press(KeyCode::Esc));
    assert!(!app.volumes_view.visible);
}

#[test]
fn test_volumes_view_navigation() {
    let mut app = make_app();
    app.volumes_view = VolumesView::open();
    app.volumes_view.volumes = vec![
        VolumeInfo {
            name: "a".into(),
            kind: microsandbox::VolumeKind::Directory,
            quota_mib: None,
            used_bytes: 0,
        },
        VolumeInfo {
            name: "b".into(),
            kind: microsandbox::VolumeKind::Directory,
            quota_mib: None,
            used_bytes: 0,
        },
    ];
    handle_event(&mut app, key_press(KeyCode::Down));
    assert_eq!(app.volumes_view.selected, 1);
    handle_event(&mut app, key_press(KeyCode::Up));
    assert_eq!(app.volumes_view.selected, 0);
}

#[test]
fn test_volumes_view_add_mode_toggle_and_validation() {
    let mut app = make_app();
    app.volumes_view = VolumesView::open();
    handle_event(&mut app, key_press(KeyCode::Char('n')));
    assert_eq!(app.volumes_view.mode, SubDialogMode::Add);
    handle_event(&mut app, key_press(KeyCode::Char(' ')));
    assert!(app.volumes_view.disk);
    handle_event(&mut app, key_press(KeyCode::Enter));
    assert!(app.volumes_view.error.is_some());
}

#[test]
fn test_handle_message_volume_list_updates_state() {
    let mut app = make_app();
    let volumes = vec![VolumeInfo {
        name: "vol1".into(),
        kind: microsandbox::VolumeKind::Disk,
        quota_mib: Some(1024),
        used_bytes: 512,
    }];
    app.handle_message(AppMessage::VolumeList(Ok(volumes.clone())));
    assert_eq!(app.volumes_view.volumes, volumes);
}

#[tokio::test]
async fn test_submit_parses_ports_correctly() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.name = "mybox".into();
    app.create_dialog.ports = vec![(8080, 80), (443, 443)];

    app.create_dialog.field = app.create_dialog.form_field_count(); // Create button
    handle_event(&mut app, key_press(KeyCode::Enter));
    assert!(!app.create_dialog.visible);
}

// ── handle_event: focus & navigation ────────────────────────────────────

#[test]
fn test_tab_switches_focus_list_to_detail() {
    let mut app = make_app();
    assert_eq!(app.focus, Focus::SandboxList);
    handle_event(&mut app, key_press(KeyCode::Tab));
    assert_eq!(app.focus, Focus::Detail);
}

#[test]
fn test_tab_switches_focus_detail_to_list() {
    let mut app = make_app();
    app.focus = Focus::Detail;
    handle_event(&mut app, key_press(KeyCode::Tab));
    assert_eq!(app.focus, Focus::SandboxList);
}

#[test]
fn test_n_opens_dialog() {
    let mut app = make_app();
    handle_event(&mut app, key_press(KeyCode::Char('n')));
    assert!(app.create_dialog.visible);
}

#[tokio::test]
async fn test_down_moves_selection_in_list() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("a", Status::Running));
    app.sandboxes.push(make_sandbox("b", Status::Running));
    handle_event(&mut app, key_press(KeyCode::Down));
    assert_eq!(app.selected, 1);
}

#[tokio::test]
async fn test_up_moves_selection_in_list() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("a", Status::Running));
    app.sandboxes.push(make_sandbox("b", Status::Running));
    app.selected = 1;
    handle_event(&mut app, key_press(KeyCode::Up));
    assert_eq!(app.selected, 0);
}

#[tokio::test]
async fn test_right_advances_tab_in_detail_focus() {
    let mut app = make_app();
    app.focus = Focus::Detail;
    handle_event(&mut app, key_press(KeyCode::Right));
    assert_eq!(app.tab, DetailTab::Filesystem);
}

#[tokio::test]
async fn test_left_goes_back_tab_in_detail_focus() {
    let mut app = make_app();
    app.focus = Focus::Detail;
    app.tab = DetailTab::Filesystem;
    handle_event(&mut app, key_press(KeyCode::Left));
    assert_eq!(app.tab, DetailTab::Logs);
}

#[test]
fn test_right_switches_focus_to_detail() {
    let mut app = make_app();
    assert_eq!(app.focus, Focus::SandboxList);
    handle_event(&mut app, key_press(KeyCode::Right));
    assert_eq!(app.focus, Focus::Detail);
}

#[test]
fn test_left_does_nothing_in_list_focus() {
    let mut app = make_app();
    assert_eq!(app.focus, Focus::SandboxList);
    handle_event(&mut app, key_press(KeyCode::Left));
    assert_eq!(app.focus, Focus::SandboxList);
    assert_eq!(app.tab, DetailTab::Logs); // unchanged
}

#[tokio::test]
async fn test_enter_on_sandbox_moves_focus_to_detail() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("a", Status::Running));
    handle_event(&mut app, key_press(KeyCode::Enter));
    assert_eq!(app.focus, Focus::Detail);
}

#[test]
fn test_enter_on_new_sandbox_opens_dialog() {
    let mut app = make_app();
    // selected == 0 == len() → new sandbox slot
    handle_event(&mut app, key_press(KeyCode::Enter));
    assert!(app.create_dialog.visible);
}

#[test]
fn test_new_sandbox_dialog_prefilled_from_config() {
    let mut app = make_app();
    app.config = AppConfig::parse(
        r#"
            image = "ubuntu:22.04"
            cpus = 4
            memory_mib = 2048
            hostname = "dev-box"
            workdir = "/workspace"
            user = "dev"
            shell = "/bin/bash"
        "#,
    )
    .unwrap();
    handle_event(&mut app, key_press(KeyCode::Char('n')));
    assert_eq!(app.create_dialog.image, "ubuntu:22.04");
    assert_eq!(app.create_dialog.cpus, "4");
    assert_eq!(app.create_dialog.memory, "2048");
    assert_eq!(app.create_dialog.hostname, "dev-box");
    assert_eq!(app.create_dialog.workdir, "/workspace");
    assert_eq!(app.create_dialog.user, "dev");
    assert_eq!(app.create_dialog.shell, "/bin/bash");
}

#[test]
fn test_new_sandbox_dialog_uses_builtin_defaults_when_config_empty() {
    let mut app = make_app();
    assert_eq!(app.config, AppConfig::default());
    handle_event(&mut app, key_press(KeyCode::Char('n')));
    assert_eq!(app.create_dialog.image, "alpine");
    assert_eq!(app.create_dialog.cpus, "1");
    assert_eq!(app.create_dialog.memory, "512");
    assert_eq!(app.create_dialog.shell, "/bin/sh");
}

// ── handle_event: sandbox actions ────────────────────────────────────────

#[tokio::test]
async fn test_s_start_notifies_when_sandbox_stopped() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("box1", Status::Stopped));
    handle_event(&mut app, key_press(KeyCode::Char('s')));
    let n = app.notification.as_ref().unwrap();
    assert!(!n.is_error);
    assert!(n.message.contains("box1"));
    assert!(app.confirm.is_none());
}

#[tokio::test]
async fn test_s_stop_opens_confirm_dialog_when_sandbox_running() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("box1", Status::Running));
    handle_event(&mut app, key_press(KeyCode::Char('s')));
    match app.confirm {
        Some(PendingAction::StopSandbox(ref name)) => assert_eq!(name, "box1"),
        _ => panic!("expected a pending StopSandbox confirmation"),
    }
    assert!(app.notification.is_none());
}

#[tokio::test]
async fn test_confirming_stop_runs_the_action() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("box1", Status::Running));
    handle_event(&mut app, key_press(KeyCode::Char('s')));
    assert!(app.confirm.is_some());
    handle_event(&mut app, key_press(KeyCode::Char('y')));
    assert!(app.confirm.is_none());
    let n = app.notification.as_ref().unwrap();
    assert!(!n.is_error);
    assert!(n.message.contains("box1"));
}

#[tokio::test]
async fn test_cancelling_confirm_dialog_leaves_sandbox_unchanged() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("box1", Status::Running));
    handle_event(&mut app, key_press(KeyCode::Char('s')));
    assert!(app.confirm.is_some());
    handle_event(&mut app, key_press(KeyCode::Esc));
    assert!(app.confirm.is_none());
    assert!(app.notification.is_none());
}

#[tokio::test]
async fn test_k_kill_opens_confirm_dialog_when_sandbox_running() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("box1", Status::Running));
    handle_event(&mut app, key_press(KeyCode::Char('K')));
    match app.confirm {
        Some(PendingAction::KillSandbox(ref name)) => assert_eq!(name, "box1"),
        _ => panic!("expected a pending KillSandbox confirmation"),
    }
}

#[tokio::test]
async fn test_d_remove_opens_confirm_dialog_when_sandbox_stopped() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("box1", Status::Stopped));
    handle_event(&mut app, key_press(KeyCode::Char('d')));
    match app.confirm {
        Some(PendingAction::RemoveSandbox(ref name)) => assert_eq!(name, "box1"),
        _ => panic!("expected a pending RemoveSandbox confirmation"),
    }
}

#[tokio::test]
async fn test_d_remove_error_when_sandbox_running() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("box1", Status::Running));
    handle_event(&mut app, key_press(KeyCode::Char('d')));
    assert!(app.notification.as_ref().unwrap().is_error);
    assert!(app.confirm.is_none());
}

// ── handle_event: search/filter ──────────────────────────────────────────

#[test]
fn test_slash_activates_search_mode() {
    let mut app = make_app();
    handle_event(&mut app, key_press(KeyCode::Char('/')));
    assert!(app.search_active);
}

#[test]
fn test_typing_in_search_mode_updates_filter_live() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("alpha", Status::Running));
    app.sandboxes.push(make_sandbox("beta", Status::Stopped));
    handle_event(&mut app, key_press(KeyCode::Char('/')));
    handle_event(&mut app, key_press(KeyCode::Char('a')));
    handle_event(&mut app, key_press(KeyCode::Char('l')));
    assert_eq!(app.filter, "al");
    assert_eq!(app.visible_indices(), vec![0]);
}

#[test]
fn test_backspace_in_search_mode_edits_filter() {
    let mut app = make_app();
    handle_event(&mut app, key_press(KeyCode::Char('/')));
    handle_event(&mut app, key_press(KeyCode::Char('a')));
    handle_event(&mut app, key_press(KeyCode::Char('b')));
    handle_event(&mut app, key_press(KeyCode::Backspace));
    assert_eq!(app.filter, "a");
}

#[test]
fn test_esc_in_search_mode_clears_filter_and_exits() {
    let mut app = make_app();
    handle_event(&mut app, key_press(KeyCode::Char('/')));
    handle_event(&mut app, key_press(KeyCode::Char('x')));
    handle_event(&mut app, key_press(KeyCode::Esc));
    assert!(!app.search_active);
    assert!(app.filter.is_empty());
}

#[test]
fn test_enter_in_search_mode_keeps_filter_and_exits_typing() {
    let mut app = make_app();
    handle_event(&mut app, key_press(KeyCode::Char('/')));
    handle_event(&mut app, key_press(KeyCode::Char('x')));
    handle_event(&mut app, key_press(KeyCode::Enter));
    assert!(!app.search_active);
    assert_eq!(app.filter, "x");
}

#[test]
fn test_status_token_filters_by_status() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("alpha", Status::Running));
    app.sandboxes.push(make_sandbox("beta", Status::Stopped));
    app.filter = "status:running".to_string();
    assert_eq!(app.visible_indices(), vec![0]);
}

#[test]
fn test_new_sandbox_slot_hidden_while_filter_active() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("alpha", Status::Running));
    app.selected = 1; // "new sandbox" slot when no filter
    assert!(app.new_sandbox_selected());
    app.filter = "alpha".to_string();
    assert!(!app.new_sandbox_selected());
}

// ── handle_event: mouse support ──────────────────────────────────────────

#[test]
fn test_mouse_click_on_card_selects_it() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("box0", Status::Stopped));
    app.sandboxes.push(make_sandbox("box1", Status::Stopped));
    app.mouse.card_rects = vec![
        (Rect::new(0, 0, 10, 6), Some(0)),
        (Rect::new(0, 6, 10, 6), Some(1)),
    ];
    app.selected = 0;
    handle_event(
        &mut app,
        mouse_event(MouseEventKind::Down(MouseButton::Left), 3, 7),
    );
    assert_eq!(app.selected, 1);
    assert_eq!(app.focus, Focus::SandboxList);
}

#[tokio::test]
async fn test_mouse_click_on_tab_switches_tab_and_focus() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("box0", Status::Running));
    app.mouse.tab_rects = vec![
        (Rect::new(0, 0, 8, 1), DetailTab::Logs),
        (Rect::new(8, 0, 10, 1), DetailTab::Filesystem),
    ];
    app.tab = DetailTab::Logs;
    handle_event(
        &mut app,
        mouse_event(MouseEventKind::Down(MouseButton::Left), 9, 0),
    );
    assert_eq!(app.tab, DetailTab::Filesystem);
    assert_eq!(app.focus, Focus::Detail);
}

#[test]
fn test_mouse_scroll_over_list_moves_selection() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("box0", Status::Stopped));
    app.sandboxes.push(make_sandbox("box1", Status::Stopped));
    app.mouse.list_area = Rect::new(0, 0, 20, 20);
    app.selected = 0;
    handle_event(&mut app, mouse_event(MouseEventKind::ScrollDown, 2, 2));
    assert_eq!(app.selected, 1);
    handle_event(&mut app, mouse_event(MouseEventKind::ScrollUp, 2, 2));
    assert_eq!(app.selected, 0);
}

#[test]
fn test_mouse_scroll_over_detail_scrolls_logs() {
    let mut app = make_app();
    app.mouse.detail_area = Rect::new(20, 0, 20, 20);
    app.tab = DetailTab::Logs;
    app.log_scroll = 5;
    handle_event(&mut app, mouse_event(MouseEventKind::ScrollDown, 21, 2));
    assert_eq!(app.log_scroll, 8);
    handle_event(&mut app, mouse_event(MouseEventKind::ScrollUp, 21, 2));
    assert_eq!(app.log_scroll, 5);
}

#[test]
fn test_mouse_ignored_while_dialog_open() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("box0", Status::Stopped));
    app.create_dialog.visible = true;
    app.mouse.card_rects = vec![(Rect::new(0, 0, 10, 6), Some(0))];
    app.selected = 0;
    handle_event(
        &mut app,
        mouse_event(MouseEventKind::Down(MouseButton::Left), 3, 3),
    );
    // Selection must not change while the modal dialog steals input.
    assert_eq!(app.selected, 0);
}

#[test]
fn test_mouse_click_on_new_sandbox_card_selects_placeholder() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("box0", Status::Stopped));
    app.mouse.card_rects = vec![
        (Rect::new(0, 0, 10, 6), Some(0)),
        (Rect::new(0, 6, 10, 3), None),
    ];
    handle_event(
        &mut app,
        mouse_event(MouseEventKind::Down(MouseButton::Left), 3, 7),
    );
    assert!(app.new_sandbox_selected());
}

#[tokio::test]
async fn test_actions_ignored_in_detail_focus() {
    let mut app = make_app();
    app.sandboxes.push(make_sandbox("box1", Status::Stopped));
    app.focus = Focus::Detail;
    // 's' in detail focus should not produce a notification
    handle_event(&mut app, key_press(KeyCode::Char('s')));
    assert!(app.notification.is_none());
}

// ── scroll helpers ───────────────────────────────────────────────────────

#[tokio::test]
async fn test_down_scrolls_logs_in_detail_focus() {
    let mut app = make_app();
    app.focus = Focus::Detail;
    app.tab = DetailTab::Logs;
    handle_event(&mut app, key_press(KeyCode::Down));
    assert!(app.log_scroll > 0);
}

#[tokio::test]
async fn test_up_scrolls_logs_in_detail_focus() {
    let mut app = make_app();
    app.focus = Focus::Detail;
    app.tab = DetailTab::Logs;
    app.log_scroll = 6;
    handle_event(&mut app, key_press(KeyCode::Up));
    assert_eq!(app.log_scroll, 3);
}

#[tokio::test]
async fn test_up_scroll_does_not_underflow() {
    let mut app = make_app();
    app.focus = Focus::Detail;
    app.tab = DetailTab::Logs;
    app.log_scroll = 0;
    handle_event(&mut app, key_press(KeyCode::Up));
    assert_eq!(app.log_scroll, 0);
}

#[tokio::test]
async fn test_backspace_navigates_fs_up() {
    let mut app = make_app();
    app.focus = Focus::Detail;
    app.tab = DetailTab::Filesystem;
    app.fs_path = "/usr/local".into();
    handle_event(&mut app, key_press(KeyCode::Backspace));
    assert_eq!(app.fs_path, "/usr");
}

#[tokio::test]
async fn test_backspace_at_root_stays_at_root() {
    let mut app = make_app();
    app.focus = Focus::Detail;
    app.tab = DetailTab::Filesystem;
    app.fs_path = "/".into();
    handle_event(&mut app, key_press(KeyCode::Backspace));
    // parent of "/" is "" which becomes "/"
    assert_eq!(app.fs_path, "/");
}

// ── r refresh key ────────────────────────────────────────────────────────

#[tokio::test]
async fn test_r_shows_refreshing_notification() {
    let mut app = make_app();
    handle_event(&mut app, key_press(KeyCode::Char('r')));
    let n = app.notification.as_ref().unwrap();
    assert!(!n.is_error);
    assert!(n.message.contains("efresh"));
}

// ── dialog: ESC on key-release closes the dialog (kitty keyboard fix) ───

#[test]
fn test_esc_release_closes_dialog() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    // Simulate a terminal that emits ESC on key-release rather than press.
    handle_event(&mut app, key_release(KeyCode::Esc));
    assert!(
        !app.create_dialog.visible,
        "Esc release must close the dialog"
    );
}

#[test]
fn test_other_release_events_still_ignored_when_dialog_open() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    // Releasing 'a' must not append to the name field.
    handle_event(&mut app, key_release(KeyCode::Char('a')));
    assert_eq!(
        app.create_dialog.name, "",
        "key-release must not modify dialog fields"
    );
}

// ── dialog: Up/Down arrow navigation between fields ──────────────────────

#[test]
fn test_dialog_down_advances_field() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    assert_eq!(app.create_dialog.field, 0);
    handle_event(&mut app, key_press(KeyCode::Down));
    assert_eq!(app.create_dialog.field, 1);
}

#[test]
fn test_dialog_down_wraps_from_last_to_first() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.field = 8; // Create button (last position)
    handle_event(&mut app, key_press(KeyCode::Down));
    assert_eq!(app.create_dialog.field, 0);
}

#[test]
fn test_dialog_up_goes_to_previous_field() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.field = 2;
    handle_event(&mut app, key_press(KeyCode::Up));
    assert_eq!(app.create_dialog.field, 1);
}

#[test]
fn test_dialog_up_wraps_from_first_to_last() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.field = 0;
    handle_event(&mut app, key_press(KeyCode::Up));
    assert_eq!(app.create_dialog.field, 8); // Create button
}

// ── dialog: digit-only filtering for CPUs and Memory ────────────────────

#[test]
fn test_dialog_cpus_accepts_digits() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.field = 2; // CPUs
    app.create_dialog.cpus.clear();
    handle_event(&mut app, key_press(KeyCode::Char('4')));
    assert_eq!(app.create_dialog.cpus, "4");
    assert!(app.create_dialog.error.is_none());
}

#[test]
fn test_dialog_cpus_rejects_letters() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.field = 2; // CPUs
    handle_event(&mut app, key_press(KeyCode::Char('x')));
    // Field must not change
    assert_eq!(app.create_dialog.cpus, "1");
    // An inline error must be shown
    assert!(app.create_dialog.error.is_some());
}

#[test]
fn test_dialog_memory_accepts_digits() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.field = 3; // Memory
    app.create_dialog.memory.clear();
    handle_event(&mut app, key_press(KeyCode::Char('2')));
    assert_eq!(app.create_dialog.memory, "2");
    assert!(app.create_dialog.error.is_none());
}

#[test]
fn test_dialog_memory_rejects_letters() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.field = 3; // Memory
    handle_event(&mut app, key_press(KeyCode::Char('m')));
    assert_eq!(app.create_dialog.memory, "512");
    assert!(app.create_dialog.error.is_some());
}

#[test]
fn test_dialog_backspace_clears_error() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.field = 2; // CPUs
                                 // Trigger an error first
    handle_event(&mut app, key_press(KeyCode::Char('x')));
    assert!(app.create_dialog.error.is_some());
    // Backspace must clear it
    handle_event(&mut app, key_press(KeyCode::Backspace));
    assert!(app.create_dialog.error.is_none());
}

// ── dialog: submit validation for CPUs and Memory ────────────────────────

#[test]
fn test_dialog_submit_zero_cpus_sets_error() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.name = "mybox".into();
    app.create_dialog.cpus = "0".into();

    app.create_dialog.field = app.create_dialog.form_field_count(); // Create button
    handle_event(&mut app, key_press(KeyCode::Enter));
    assert!(
        app.create_dialog.visible,
        "dialog stays open on invalid input"
    );
    let err = app.create_dialog.error.as_deref().unwrap_or("");
    assert!(
        err.contains("1") && err.contains("32"),
        "error must state valid range"
    );
}

#[test]
fn test_dialog_submit_cpus_above_max_sets_error() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.name = "mybox".into();
    app.create_dialog.cpus = "33".into();

    app.create_dialog.field = app.create_dialog.form_field_count(); // Create button
    handle_event(&mut app, key_press(KeyCode::Enter));
    assert!(app.create_dialog.visible);
    assert!(app.create_dialog.error.is_some());
}

#[test]
fn test_dialog_submit_memory_below_min_sets_error() {
    let mut app = make_app();
    app.create_dialog = CreateDialog::open();
    app.create_dialog.name = "mybox".into();
    app.create_dialog.memory = "32".into();

    app.create_dialog.field = app.create_dialog.form_field_count(); // Create button
    handle_event(&mut app, key_press(KeyCode::Enter));
    assert!(app.create_dialog.visible);
    let err = app.create_dialog.error.as_deref().unwrap_or("");
    assert!(err.contains("64"), "error must mention minimum memory");
}
