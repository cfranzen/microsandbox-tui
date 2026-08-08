//! Translates terminal key and mouse events into [`App`] state changes.
//!
//! This module owns the entire input-dispatch tree: the global key map in
//! [`handle_event`], and one handler per modal (create-sandbox dialog and
//! its sub-dialogs, the directory picker, the volumes view). Handlers that
//! aren't about dispatching a key event directly (sandbox actions, dialog
//! submission, filtering, scrolling) live in [`super::actions`].

use crossterm::event::{
    Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;

use crate::sandbox::{
    MountSource, NetRuleDestKind, NetRuleDirection, NetRuleProtocol, NetworkRule, SecretConfig,
    SecretHostKind, SecretHostPattern, VolumeMountConfig,
};

use super::actions::{
    action_exec, action_remove, action_shell, action_terminate, action_toggle_start_stop,
    handle_confirm_key, handle_search_key, nav_fs_up, on_sandbox_selected, on_tab_switched,
    request_volume_refresh, scroll_down, scroll_up, submit_create_dialog, PendingAction,
};
use super::dialogs::{
    CreateDialog, DialogTab, DirPicker, EnvVarAddDialog, ExecDialog, ListField, MountAddDialog,
    MountKindChoice, NetRuleAddDialog, PortAddDialog, SecretAddDialog, SubDialogMode, VolumesView,
    DRIVES_ENTRY, PICKER_VISIBLE_ROWS,
};
use super::{App, AppMessage, DetailTab, Focus};

/// Returns true if the point `(x, y)` falls within `rect`.
fn point_in_rect(x: u16, y: u16, rect: Rect) -> bool {
    rect.x <= x && x < rect.x + rect.width && rect.y <= y && y < rect.y + rect.height
}

fn request_volume_refresh_if_runtime(app: &App) {
    if tokio::runtime::Handle::try_current().is_ok() {
        request_volume_refresh(app);
    }
}

fn move_network_rule(app: &mut App, delta: isize) {
    let selected = app.create_dialog.network_rules_selected;
    let len = app.create_dialog.network_rules.len();
    if len < 2 {
        return;
    }
    let target = if delta < 0 {
        selected.saturating_sub(1)
    } else {
        (selected + 1).min(len - 1)
    };
    if target == selected {
        return;
    }
    app.create_dialog.network_rules.swap(selected, target);
    app.create_dialog.network_rules_selected = target;
}

pub(crate) fn handle_event(app: &mut App, event: Event) {
    let key = match event {
        Event::Key(key) => key,
        Event::Mouse(mouse) => {
            handle_mouse_event(app, mouse);
            return;
        }
        _ => return,
    };

    // Only act on key presses or repeats; ignore all release events, except
    // for Esc — some terminals (notably on Windows) only emit a release
    // event for the Esc key, so treating it like every other release would
    // make Esc silently do nothing on those terminals. Other terminals
    // (notably those using the Kitty keyboard protocol) emit BOTH a press
    // and a release event for every key, including Esc. To still handle
    // each physical Esc keypress exactly once, we track whether the press
    // was already acted on and swallow the paired release; if no press was
    // seen (the Windows-only-release case) the release itself is handled.
    if key.code == KeyCode::Esc {
        if key.kind == KeyEventKind::Release {
            if app.esc_press_handled {
                app.esc_press_handled = false;
                return;
            }
        } else {
            app.esc_press_handled = true;
        }
    } else if key.kind == KeyEventKind::Release {
        return;
    }

    // The confirmation dialog takes priority over every other modal so a
    // pending destructive action (triggered from the main view or from
    // within the Volumes view) is always resolved before anything else.
    if app.confirm.is_some() {
        handle_confirm_key(app, key.code);
        return;
    }

    // Modal dialog steals all input; the dir picker overlays the dialog.
    if app.create_dialog.visible {
        if app.create_dialog.dir_picker.visible {
            handle_picker_key(app, key.code, key.modifiers);
        } else if app.create_dialog.port_add.visible {
            handle_port_add_key(app, key.code, key.modifiers);
        } else if app.create_dialog.env_var_add.visible {
            handle_env_var_add_key(app, key.code, key.modifiers);
        } else if app.create_dialog.net_rule_add.visible {
            handle_net_rule_add_key(app, key.code, key.modifiers);
        } else if app.create_dialog.mount_add.visible {
            handle_mount_add_key(app, key.code, key.modifiers);
        } else if app.create_dialog.secret_add.visible {
            handle_secret_add_key(app, key.code, key.modifiers);
        } else {
            handle_dialog_key(app, key.code, key.modifiers);
        }
        return;
    }

    // The Volumes view is a separate top-level modal.
    if app.volumes_view.visible {
        handle_volumes_view_key(app, key.code, key.modifiers);
        return;
    }

    // The "Exec" dialog is a separate top-level modal.
    if app.exec_dialog.visible {
        handle_exec_dialog_key(app, key.code);
        return;
    }

    // Search/filter input steals key input while active.
    if app.search_active {
        handle_search_key(app, key.code);
        return;
    }

    match key.code {
        // Global quit
        KeyCode::Char('q') | KeyCode::Char('Q') => app.should_quit = true,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.should_quit = true
        }

        // Esc: move focus back to the sandbox list from the detail panel
        KeyCode::Esc => {
            app.focus = Focus::SandboxList;
        }

        // Focus/tab navigation: Left/Right and Tab/Shift+Tab are equivalent.
        // From the sandbox list, Right/Tab move into the detail panel; from
        // the detail panel, Left/Shift+Tab cycle its tabs backward and step
        // back out to the sandbox list once the leftmost tab is reached.
        KeyCode::Tab | KeyCode::Right => nav_right(app),
        KeyCode::BackTab | KeyCode::Left => nav_left(app),

        // Navigation depends on focus
        KeyCode::Up => {
            if app.focus == Focus::SandboxList {
                app.select_prev();
                on_sandbox_selected(app);
            } else {
                scroll_up(app);
            }
        }
        KeyCode::Down => {
            if app.focus == Focus::SandboxList {
                app.select_next();
                on_sandbox_selected(app);
            } else {
                scroll_down(app);
            }
        }

        // Sandbox actions (only when focus is on the list)
        KeyCode::Char('/') if app.focus == Focus::SandboxList => {
            app.search_active = true;
        }
        KeyCode::Char('s') if app.focus == Focus::SandboxList => {
            action_toggle_start_stop(app);
        }
        KeyCode::Char('t') if app.focus == Focus::SandboxList => {
            action_terminate(app);
        }
        KeyCode::Char('e') if app.focus == Focus::SandboxList => {
            action_exec(app);
        }
        KeyCode::Char('h') if app.focus == Focus::SandboxList => {
            action_shell(app);
        }
        KeyCode::Char('d') if app.focus == Focus::SandboxList => {
            action_remove(app);
        }
        KeyCode::Enter => {
            if app.new_sandbox_selected() {
                app.create_dialog = CreateDialog::open_with_config(&app.config);
            } else if app.focus == Focus::SandboxList {
                app.focus = Focus::Detail;
            }
        }
        KeyCode::Char('n') => {
            app.create_dialog = CreateDialog::open_with_config(&app.config);
        }
        KeyCode::Char('v') => {
            app.volumes_view = VolumesView::open();
            request_volume_refresh_if_runtime(app);
        }
        KeyCode::Char('r') => {
            app.request_refresh();
            app.notify("Refreshing…", false);
        }
        KeyCode::Char('T') => {
            app.toggle_theme();
        }

        // Filesystem navigation
        KeyCode::Backspace if app.focus == Focus::Detail && app.tab == DetailTab::Filesystem => {
            nav_fs_up(app);
        }

        _ => {}
    }
}

/// Move focus/selection one step to the right: from the sandbox list into
/// the detail panel, or forward through the detail panel's tabs.
fn nav_right(app: &mut App) {
    match app.focus {
        Focus::SandboxList => app.focus = Focus::Detail,
        Focus::Detail => {
            app.next_tab();
            on_tab_switched(app);
        }
    }
}

/// Move focus/selection one step to the left: backward through the detail
/// panel's tabs, stepping out to the sandbox list once the leftmost tab
/// (the first entry of [`DetailTab::all`]) is reached. No-op on the
/// sandbox list, which is already the leftmost pane.
fn nav_left(app: &mut App) {
    if app.focus == Focus::Detail {
        if app.tab == DetailTab::all()[0] {
            app.focus = Focus::SandboxList;
        } else {
            app.prev_tab();
            on_tab_switched(app);
        }
    }
}

/// Translate a mouse event into the equivalent list/detail-panel action.
/// Ignored while a modal dialog or the search box is active, to keep scope
/// limited to the main view (list selection, tab switching, scrolling).
fn handle_mouse_event(app: &mut App, mouse: MouseEvent) {
    if app.confirm.is_some()
        || app.create_dialog.visible
        || app.volumes_view.visible
        || app.exec_dialog.visible
        || app.search_active
    {
        return;
    }

    let (x, y) = (mouse.column, mouse.row);
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some(tab) = tab_at(app, x, y) {
                app.tab = tab;
                app.focus = Focus::Detail;
                on_tab_switched(app);
            } else if let Some(target) = card_at(app, x, y) {
                app.selected = target.unwrap_or(app.sandboxes.len());
                app.focus = Focus::SandboxList;
                on_sandbox_selected(app);
            } else if point_in_rect(x, y, app.mouse.detail_area) {
                app.focus = Focus::Detail;
            } else if point_in_rect(x, y, app.mouse.list_area) {
                app.focus = Focus::SandboxList;
            }
        }
        MouseEventKind::ScrollUp => {
            if point_in_rect(x, y, app.mouse.list_area) {
                app.select_prev();
                on_sandbox_selected(app);
            } else if point_in_rect(x, y, app.mouse.detail_area) {
                scroll_up(app);
            }
        }
        MouseEventKind::ScrollDown => {
            if point_in_rect(x, y, app.mouse.list_area) {
                app.select_next();
                on_sandbox_selected(app);
            } else if point_in_rect(x, y, app.mouse.detail_area) {
                scroll_down(app);
            }
        }
        _ => {}
    }
}

/// Returns the detail tab whose rendered rect contains `(x, y)`, if any.
fn tab_at(app: &App, x: u16, y: u16) -> Option<DetailTab> {
    app.mouse
        .tab_rects
        .iter()
        .find(|(rect, _)| point_in_rect(x, y, *rect))
        .map(|(_, tab)| *tab)
}

/// Returns the sandbox card whose rendered rect contains `(x, y)`, if any.
/// `Some(None)` is the "New Sandbox" placeholder card.
fn card_at(app: &App, x: u16, y: u16) -> Option<Option<usize>> {
    app.mouse
        .card_rects
        .iter()
        .find(|(rect, _)| point_in_rect(x, y, *rect))
        .map(|(_, idx)| *idx)
}

fn handle_dialog_key(app: &mut App, code: KeyCode, _mods: KeyModifiers) {
    if app.create_dialog.list_edit_mode {
        if let Some(list) = app.create_dialog.focused_list() {
            match code {
                KeyCode::Esc => {
                    app.create_dialog.list_edit_mode = false;
                    return;
                }
                KeyCode::Up => {
                    move_list_selection(app, list, -1);
                    return;
                }
                KeyCode::Down => {
                    move_list_selection(app, list, 1);
                    return;
                }
                KeyCode::Char('a') | KeyCode::Char('A') => {
                    open_list_add_dialog(app, list);
                    return;
                }
                KeyCode::Char('d') | KeyCode::Char('D') | KeyCode::Delete => {
                    delete_list_selected(app, list);
                    return;
                }
                KeyCode::Enter => {
                    open_list_edit_dialog(app, list);
                    return;
                }
                KeyCode::Char('+') if list == ListField::NetworkRules => {
                    move_network_rule(app, -1);
                    return;
                }
                KeyCode::Char('-') if list == ListField::NetworkRules => {
                    move_network_rule(app, 1);
                    return;
                }
                _ => {}
            }
        }
    }

    match code {
        KeyCode::Esc => {
            app.create_dialog = Default::default();
            return;
        }
        KeyCode::Tab | KeyCode::Down => app.create_dialog.next_field(),
        KeyCode::BackTab | KeyCode::Up => app.create_dialog.prev_field(),
        KeyCode::Char(' ') if app.create_dialog.is_toggle_field() => {
            if app
                .create_dialog
                .field_disabled_by_network(app.create_dialog.tab, app.create_dialog.field)
            {
                return;
            }
            match (app.create_dialog.tab, app.create_dialog.field) {
                (DialogTab::Network, 0) => {
                    app.create_dialog.disable_network = !app.create_dialog.disable_network;
                }
                (DialogTab::Network, 1) => {
                    app.create_dialog.default_ingress_action =
                        app.create_dialog.default_ingress_action.cycle();
                }
                (DialogTab::Network, 2) => {
                    app.create_dialog.default_egress_action =
                        app.create_dialog.default_egress_action.cycle();
                }
                (DialogTab::Dns, 2) => {
                    app.create_dialog.dns_rebind_protection =
                        !app.create_dialog.dns_rebind_protection;
                }
                (DialogTab::Security, 1) => {
                    app.create_dialog.tls_enabled = !app.create_dialog.tls_enabled;
                }
                (DialogTab::Security, 4) => {
                    app.create_dialog.tls_verify_upstream = !app.create_dialog.tls_verify_upstream;
                }
                (DialogTab::Security, 5) => {
                    app.create_dialog.tls_block_quic = !app.create_dialog.tls_block_quic;
                }
                _ => {}
            }
            app.create_dialog.error = None;
        }
        KeyCode::Left | KeyCode::Right if !app.create_dialog.is_create_focused() => {
            if app
                .create_dialog
                .field_disabled_by_network(app.create_dialog.tab, app.create_dialog.field)
            {
                let tab = if code == KeyCode::Left {
                    app.create_dialog.tab.prev()
                } else {
                    app.create_dialog.tab.next()
                };
                app.create_dialog.switch_tab(tab);
                return;
            }
            match (app.create_dialog.tab, app.create_dialog.field) {
                (DialogTab::Security, 6) => {
                    app.create_dialog.violation_action = app.create_dialog.violation_action.cycle();
                }
                _ => {
                    let tab = if code == KeyCode::Left {
                        app.create_dialog.tab.prev()
                    } else {
                        app.create_dialog.tab.next()
                    };
                    app.create_dialog.switch_tab(tab);
                }
            }
            app.create_dialog.error = None;
        }
        KeyCode::Enter => {
            let dlg = &app.create_dialog;
            if dlg.is_create_focused() {
                submit_create_dialog(app);
            } else if app
                .create_dialog
                .field_disabled_by_network(app.create_dialog.tab, app.create_dialog.field)
            {
                return;
            } else if app.create_dialog.focused_list().is_some() {
                app.create_dialog.list_edit_mode = true;
                app.create_dialog.error = None;
            } else if dlg.tab == DialogTab::Basic && dlg.field == 6 {
                let initial = app.create_dialog.workdir.trim().to_owned();
                let start = if initial.is_empty() { "/" } else { &initial };
                app.create_dialog.dir_picker = DirPicker::open(start);
            } else {
                app.create_dialog.next_field();
            }
        }
        KeyCode::Backspace => {
            if app
                .create_dialog
                .field_disabled_by_network(app.create_dialog.tab, app.create_dialog.field)
            {
                return;
            }
            if let Some(field) = app.create_dialog.current_field_mut() {
                field.pop();
            }
            app.create_dialog.error = None;
        }
        KeyCode::Char(c) => {
            if app.create_dialog.is_toggle_field()
                || app.create_dialog.is_create_focused()
                || app
                    .create_dialog
                    .field_disabled_by_network(app.create_dialog.tab, app.create_dialog.field)
            {
                return;
            }
            if app.create_dialog.focused_list().is_some() {
                return;
            }
            if app.create_dialog.tab == DialogTab::Basic && app.create_dialog.field == 6 {
                return;
            }
            if app.create_dialog.is_numeric_field() && !c.is_ascii_digit() {
                app.create_dialog.error = Some("Only digits allowed here".into());
                return;
            }
            app.create_dialog.error = None;
            if let Some(field) = app.create_dialog.current_field_mut() {
                field.push(c);
            }
        }
        _ => {}
    }
}

fn open_list_edit_dialog(app: &mut App, list: ListField) {
    match list {
        ListField::EnvVars => {
            if let Some(entry) = app
                .create_dialog
                .env_vars
                .get(app.create_dialog.env_vars_selected)
                .cloned()
            {
                app.create_dialog.env_var_add =
                    EnvVarAddDialog::open_for_edit(app.create_dialog.env_vars_selected, &entry);
            }
        }
        ListField::Mounts => {
            if let Some(entry) = app
                .create_dialog
                .mounts
                .get(app.create_dialog.mounts_selected)
                .cloned()
            {
                app.create_dialog.mount_add =
                    MountAddDialog::open_for_edit(app.create_dialog.mounts_selected, &entry);
                request_volume_refresh_if_runtime(app);
            }
        }
        ListField::Ports => {
            if let Some(entry) = app
                .create_dialog
                .ports
                .get(app.create_dialog.ports_selected)
                .copied()
            {
                app.create_dialog.port_add =
                    PortAddDialog::open_for_edit(app.create_dialog.ports_selected, entry);
            }
        }
        ListField::NetworkRules => {
            if let Some(entry) = app
                .create_dialog
                .network_rules
                .get(app.create_dialog.network_rules_selected)
                .cloned()
            {
                app.create_dialog.net_rule_add = NetRuleAddDialog::open_for_edit(
                    app.create_dialog.network_rules_selected,
                    &entry,
                );
            }
        }
        ListField::Secrets => {
            if let Some(entry) = app
                .create_dialog
                .secrets
                .get(app.create_dialog.secrets_selected)
                .cloned()
            {
                app.create_dialog.secret_add =
                    SecretAddDialog::open_for_edit(app.create_dialog.secrets_selected, &entry);
            }
        }
    }
}

/// Moves the selection cursor of the given inline list by `delta` (-1 or
/// +1), clamped to the list's bounds.
fn move_list_selection(app: &mut App, list: ListField, delta: i32) {
    fn move_sel(selected: &mut usize, len: usize, delta: i32) {
        if len == 0 {
            *selected = 0;
        } else if delta < 0 {
            if *selected > 0 {
                *selected -= 1;
            }
        } else if *selected + 1 < len {
            *selected += 1;
        }
    }

    let dlg = &mut app.create_dialog;
    match list {
        ListField::EnvVars => move_sel(&mut dlg.env_vars_selected, dlg.env_vars.len(), delta),
        ListField::Mounts => move_sel(&mut dlg.mounts_selected, dlg.mounts.len(), delta),
        ListField::Ports => move_sel(&mut dlg.ports_selected, dlg.ports.len(), delta),
        ListField::NetworkRules => {
            move_sel(&mut dlg.network_rules_selected, dlg.network_rules.len(), delta)
        }
        ListField::Secrets => move_sel(&mut dlg.secrets_selected, dlg.secrets.len(), delta),
    }
}

/// Opens the "Add" popup associated with the given inline list.
fn open_list_add_dialog(app: &mut App, list: ListField) {
    match list {
        ListField::EnvVars => app.create_dialog.env_var_add = EnvVarAddDialog::open(),
        ListField::Mounts => {
            app.create_dialog.mount_add = MountAddDialog::open();
            request_volume_refresh_if_runtime(app);
        }
        ListField::Ports => app.create_dialog.port_add = PortAddDialog::open(),
        ListField::NetworkRules => app.create_dialog.net_rule_add = NetRuleAddDialog::open(),
        ListField::Secrets => app.create_dialog.secret_add = SecretAddDialog::open(),
    }
}

/// Deletes the currently selected entry of the given inline list, if any.
fn delete_list_selected(app: &mut App, list: ListField) {
    fn remove_selected<T>(entries: &mut Vec<T>, selected: &mut usize) {
        if entries.is_empty() {
            return;
        }
        entries.remove(*selected);
        if *selected >= entries.len() && *selected > 0 {
            *selected -= 1;
        }
    }

    let dlg = &mut app.create_dialog;
    match list {
        ListField::EnvVars => remove_selected(&mut dlg.env_vars, &mut dlg.env_vars_selected),
        ListField::Mounts => remove_selected(&mut dlg.mounts, &mut dlg.mounts_selected),
        ListField::Ports => remove_selected(&mut dlg.ports, &mut dlg.ports_selected),
        ListField::NetworkRules => {
            remove_selected(&mut dlg.network_rules, &mut dlg.network_rules_selected)
        }
        ListField::Secrets => remove_selected(&mut dlg.secrets, &mut dlg.secrets_selected),
    }
}

fn handle_picker_key(app: &mut App, code: KeyCode, _mods: KeyModifiers) {
    let mount_picker_visible = app.create_dialog.mount_add.dir_picker.visible;
    if code == KeyCode::Esc {
        if mount_picker_visible {
            app.create_dialog.mount_add.dir_picker.visible = false;
        } else {
            app.create_dialog.dir_picker.visible = false;
        }
        return;
    }

    let picker = if mount_picker_visible {
        &mut app.create_dialog.mount_add.dir_picker
    } else {
        &mut app.create_dialog.dir_picker
    };
    match code {
        KeyCode::Up => {
            if picker.selected > 0 {
                picker.selected -= 1;
                if picker.selected < picker.scroll_offset {
                    picker.scroll_offset = picker.selected;
                }
            }
        }
        KeyCode::Down => {
            if picker.selected + 1 < picker.entries.len() {
                picker.selected += 1;
                if picker.selected >= picker.scroll_offset + PICKER_VISIBLE_ROWS {
                    picker.scroll_offset = picker.selected + 1 - PICKER_VISIBLE_ROWS;
                }
            }
        }
        KeyCode::Enter => {
            let entry = picker
                .entries
                .get(picker.selected)
                .cloned()
                .unwrap_or_default();
            if entry == DRIVES_ENTRY {
                picker.show_drives();
            } else if picker.showing_drives {
                // Entry is a drive root — navigate into it.
                picker.navigate_to(entry);
            } else if entry == ".." {
                let parent = std::path::Path::new(&picker.path)
                    .parent()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|| picker.path.clone());
                if parent == picker.path {
                    // Already at a drive root — go to drive selection.
                    picker.show_drives();
                } else {
                    picker.navigate_to(parent);
                }
            } else {
                // Use Path::join so separators are correct on both Windows and Unix.
                let new_path = std::path::Path::new(&picker.path)
                    .join(&entry)
                    .to_string_lossy()
                    .into_owned();
                if std::path::Path::new(&new_path).is_dir() {
                    picker.navigate_to(new_path);
                }
            }
        }
        KeyCode::Char(' ') => {
            // Space confirms the current directory without descending.
            if !picker.showing_drives {
                let chosen = picker.path.clone();
                if mount_picker_visible {
                    app.create_dialog.mount_add.source_input = chosen;
                } else {
                    app.create_dialog.workdir = chosen;
                }
            }
            if mount_picker_visible {
                app.create_dialog.mount_add.dir_picker.visible = false;
            } else {
                app.create_dialog.dir_picker.visible = false;
            }
        }
        // Jump to filesystem root / drive list.
        KeyCode::Char('/') => {
            picker.show_drives();
        }
        // Jump to home directory.
        KeyCode::Char('~') => {
            if let Some(home) = std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(|v| v.to_string_lossy().into_owned())
            {
                picker.navigate_to(home);
            }
        }
        _ => {}
    }
}

fn handle_port_add_key(app: &mut App, code: KeyCode, _mods: KeyModifiers) {
    match code {
        KeyCode::Esc => {
            app.create_dialog.port_add.visible = false;
        }
        KeyCode::Tab | KeyCode::Down | KeyCode::BackTab | KeyCode::Up => {
            let f = app.create_dialog.port_add.add_field;
            app.create_dialog.port_add.add_field = (f + 1) % 2;
        }
        KeyCode::Backspace => {
            let dlg = &mut app.create_dialog.port_add;
            if dlg.add_field == 0 {
                dlg.host_input.pop();
            } else {
                dlg.guest_input.pop();
            }
            dlg.error = None;
        }
        KeyCode::Enter => {
            if app.create_dialog.port_add.add_field == 0 {
                app.create_dialog.port_add.add_field = 1;
            } else {
                let host = app.create_dialog.port_add.host_input.trim().parse::<u16>();
                let guest = app.create_dialog.port_add.guest_input.trim().parse::<u16>();
                match (host, guest) {
                    (Ok(h), Ok(g)) => {
                        if let Some(index) = app.create_dialog.port_add.editing_index {
                            app.create_dialog.ports[index] = (h, g);
                            app.create_dialog.ports_selected = index;
                        } else {
                            app.create_dialog.ports.push((h, g));
                            app.create_dialog.ports_selected = app.create_dialog.ports.len() - 1;
                        }
                        app.create_dialog.port_add.visible = false;
                    }
                    (Err(_), _) => {
                        app.create_dialog.port_add.error = Some("Invalid host port (0–65535)".into());
                    }
                    (_, Err(_)) => {
                        app.create_dialog.port_add.error = Some("Invalid guest port (0–65535)".into());
                    }
                }
            }
        }
        KeyCode::Char(c) if c.is_ascii_digit() => {
            let dlg = &mut app.create_dialog.port_add;
            if dlg.add_field == 0 {
                dlg.host_input.push(c);
            } else {
                dlg.guest_input.push(c);
            }
            dlg.error = None;
        }
        _ => {}
    }
}

fn handle_env_var_add_key(app: &mut App, code: KeyCode, _mods: KeyModifiers) {
    match code {
        KeyCode::Esc => {
            app.create_dialog.env_var_add.visible = false;
        }
        KeyCode::Tab | KeyCode::Down | KeyCode::BackTab | KeyCode::Up => {
            let f = app.create_dialog.env_var_add.add_field;
            app.create_dialog.env_var_add.add_field = (f + 1) % 2;
        }
        KeyCode::Backspace => {
            let dlg = &mut app.create_dialog.env_var_add;
            if dlg.add_field == 0 {
                dlg.key_input.pop();
            } else {
                dlg.value_input.pop();
            }
            dlg.error = None;
        }
        KeyCode::Enter => {
            if app.create_dialog.env_var_add.add_field == 0 {
                app.create_dialog.env_var_add.add_field = 1;
            } else {
                let key = app.create_dialog.env_var_add.key_input.trim().to_owned();
                let value = app.create_dialog.env_var_add.value_input.clone();
                if key.is_empty() {
                    app.create_dialog.env_var_add.error = Some("Key cannot be empty".into());
                } else if key.contains('=') {
                    app.create_dialog.env_var_add.error = Some("Key must not contain '='".into());
                } else {
                    if let Some(index) = app.create_dialog.env_var_add.editing_index {
                        app.create_dialog.env_vars[index] = (key, value);
                        app.create_dialog.env_vars_selected = index;
                    } else {
                        app.create_dialog.env_vars.push((key, value));
                        app.create_dialog.env_vars_selected = app.create_dialog.env_vars.len() - 1;
                    }
                    app.create_dialog.env_var_add.visible = false;
                }
            }
        }
        KeyCode::Char(c) => {
            let dlg = &mut app.create_dialog.env_var_add;
            // Disallow '=' in the key field.
            if dlg.add_field == 0 && c == '=' {
                dlg.error = Some("Key must not contain '='".into());
                return;
            }
            if dlg.add_field == 0 {
                dlg.key_input.push(c);
            } else {
                dlg.value_input.push(c);
            }
            dlg.error = None;
        }
        _ => {}
    }
}

/// Parse and lightly validate a CIDR string (`a.b.c.d/prefix` shape check).
/// Full semantic validation happens in the SDK's policy builder; this just
/// catches obviously malformed input before it's added to the rule list.
pub(crate) fn validate_cidr(input: &str) -> Result<(), &'static str> {
    let (addr, prefix) = input.split_once('/').ok_or("CIDR must be `addr/prefix`")?;
    if addr.is_empty() {
        return Err("CIDR address cannot be empty");
    }
    let prefix_len: u8 = prefix.parse().map_err(|_| "CIDR prefix must be a number")?;
    if addr.contains(':') {
        if prefix_len > 128 {
            return Err("IPv6 prefix must be 0–128");
        }
    } else {
        if !addr.split('.').all(|o| o.parse::<u8>().is_ok()) || addr.split('.').count() != 4 {
            return Err("Invalid IPv4 address");
        }
        if prefix_len > 32 {
            return Err("IPv4 prefix must be 0–32");
        }
    }
    Ok(())
}

/// Parse a single port (`"8080"`) or port range (`"1000-2000"`) string.
fn parse_port_range(input: &str) -> Result<(u16, u16), &'static str> {
    if let Some((lo, hi)) = input.split_once('-') {
        let lo: u16 = lo.trim().parse().map_err(|_| "Invalid port range")?;
        let hi: u16 = hi.trim().parse().map_err(|_| "Invalid port range")?;
        if lo > hi {
            return Err("Range start must be <= end");
        }
        Ok((lo, hi))
    } else {
        let p: u16 = input.trim().parse().map_err(|_| "Invalid port")?;
        Ok((p, p))
    }
}

fn handle_net_rule_add_key(app: &mut App, code: KeyCode, _mods: KeyModifiers) {
    let field = app.create_dialog.net_rule_add.add_field;
    match code {
        KeyCode::Esc => {
            app.create_dialog.net_rule_add.visible = false;
        }
        KeyCode::Tab | KeyCode::Down => {
            app.create_dialog.net_rule_add.add_field = (field + 1) % NetRuleAddDialog::FIELD_COUNT;
        }
        KeyCode::BackTab | KeyCode::Up => {
            app.create_dialog.net_rule_add.add_field =
                (field + NetRuleAddDialog::FIELD_COUNT - 1) % NetRuleAddDialog::FIELD_COUNT;
        }
        KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') if field == 0 => {
            let dlg = &mut app.create_dialog.net_rule_add;
            dlg.direction = dlg.direction.cycle();
            dlg.error = None;
        }
        KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') if field == 1 => {
            let dlg = &mut app.create_dialog.net_rule_add;
            dlg.action = dlg.action.cycle();
            dlg.error = None;
        }
        KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') if field == 2 => {
            let dlg = &mut app.create_dialog.net_rule_add;
            dlg.dest_kind = dlg.dest_kind.cycle();
            dlg.error = None;
        }
        KeyCode::Left | KeyCode::Right | KeyCode::Char(' ')
            if field == 3 && app.create_dialog.net_rule_add.dest_kind == NetRuleDestKind::Group =>
        {
            let dlg = &mut app.create_dialog.net_rule_add;
            dlg.dest_group = dlg.dest_group.cycle();
            dlg.error = None;
        }
        KeyCode::Backspace if field == 3 => {
            app.create_dialog.net_rule_add.dest_input.pop();
            app.create_dialog.net_rule_add.error = None;
        }
        KeyCode::Char(c)
            if field == 3 && app.create_dialog.net_rule_add.dest_kind.needs_text_value() =>
        {
            app.create_dialog.net_rule_add.dest_input.push(c);
            app.create_dialog.net_rule_add.error = None;
        }
        KeyCode::Left if field == 4 => {
            let dlg = &mut app.create_dialog.net_rule_add;
            dlg.protocol_cursor =
                (dlg.protocol_cursor + NetRuleProtocol::ALL.len() - 1) % NetRuleProtocol::ALL.len();
        }
        KeyCode::Right if field == 4 => {
            let dlg = &mut app.create_dialog.net_rule_add;
            dlg.protocol_cursor = (dlg.protocol_cursor + 1) % NetRuleProtocol::ALL.len();
        }
        KeyCode::Char(' ') if field == 4 => {
            let dlg = &mut app.create_dialog.net_rule_add;
            let proto = NetRuleProtocol::ALL[dlg.protocol_cursor];
            if proto.is_icmp() && dlg.direction == NetRuleDirection::Ingress {
                dlg.error = Some("ICMP protocols are egress-only".into());
            } else if let Some(pos) = dlg.protocols.iter().position(|p| *p == proto) {
                dlg.protocols.remove(pos);
                dlg.error = None;
            } else {
                dlg.protocols.push(proto);
                dlg.error = None;
            }
        }
        KeyCode::Backspace if field == 5 => {
            app.create_dialog.net_rule_add.ports_input.pop();
            app.create_dialog.net_rule_add.error = None;
        }
        KeyCode::Char(c) if field == 5 && (c.is_ascii_digit() || c == '-') => {
            app.create_dialog.net_rule_add.ports_input.push(c);
            app.create_dialog.net_rule_add.error = None;
        }
        KeyCode::Enter => {
            if field == NetRuleAddDialog::FIELD_COUNT - 1 {
                submit_net_rule(app);
            } else {
                app.create_dialog.net_rule_add.add_field = field + 1;
            }
        }
        _ => {}
    }
}

/// Validates and, if valid, appends the popup's in-progress rule to the
/// Network tab's rule list, closing the popup.
fn submit_net_rule(app: &mut App) {
    let dlg = app.create_dialog.net_rule_add.clone();

    if dlg.direction == NetRuleDirection::Ingress && dlg.protocols.iter().any(|p| p.is_icmp()) {
        app.create_dialog.net_rule_add.error = Some("ICMP protocols are egress-only".into());
        return;
    }

    let dest_value = dlg.dest_input.trim().to_owned();
    if dlg.dest_kind.needs_text_value() {
        if dest_value.is_empty() {
            app.create_dialog.net_rule_add.error = Some("Destination value is required".into());
            return;
        }
        match dlg.dest_kind {
            NetRuleDestKind::Ip => {
                if dest_value.parse::<std::net::IpAddr>().is_err() {
                    app.create_dialog.net_rule_add.error = Some("Invalid IP address".into());
                    return;
                }
            }
            NetRuleDestKind::Cidr => {
                if let Err(e) = validate_cidr(&dest_value) {
                    app.create_dialog.net_rule_add.error = Some(e.to_string());
                    return;
                }
            }
            NetRuleDestKind::Domain | NetRuleDestKind::DomainSuffix => {
                if dest_value.contains(char::is_whitespace) {
                    app.create_dialog.net_rule_add.error =
                        Some("Domain must not contain whitespace".into());
                    return;
                }
            }
            _ => {}
        }
    }

    let port_range = if dlg.ports_input.trim().is_empty() {
        None
    } else {
        match parse_port_range(dlg.ports_input.trim()) {
            Ok(pr) => Some(pr),
            Err(e) => {
                app.create_dialog.net_rule_add.error = Some(e.to_owned());
                return;
            }
        }
    };

    let rule = NetworkRule {
        direction: dlg.direction,
        action: dlg.action,
        dest_kind: dlg.dest_kind,
        dest_value,
        dest_group: dlg.dest_group,
        protocols: dlg.protocols,
        port_range,
    };
    if let Some(index) = dlg.editing_index {
        app.create_dialog.network_rules[index] = rule;
        app.create_dialog.network_rules_selected = index;
    } else {
        app.create_dialog.network_rules.push(rule);
        app.create_dialog.network_rules_selected = app.create_dialog.network_rules.len() - 1;
    }
    app.create_dialog.net_rule_add.visible = false;
}

fn handle_mount_add_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    if app.create_dialog.mount_add.dir_picker.visible {
        handle_picker_key(app, code, mods);
        return;
    }
    match code {
        KeyCode::Esc => {
            if app.create_dialog.mount_add.new_volume_mode {
                app.create_dialog.mount_add.new_volume_mode = false;
                app.create_dialog.mount_add.error = None;
            } else {
                app.create_dialog.mount_add.visible = false;
            }
        }
        KeyCode::Tab => {
            let f = app.create_dialog.mount_add.add_field;
            app.create_dialog.mount_add.add_field = (f + 1) % 3;
        }
        KeyCode::BackTab => {
            let f = app.create_dialog.mount_add.add_field;
            app.create_dialog.mount_add.add_field = if f == 0 { 2 } else { f - 1 };
        }
        KeyCode::Char('b') | KeyCode::Char('B') if app.create_dialog.mount_add.add_field == 0 => {
            app.create_dialog.mount_add.kind = MountKindChoice::Bind;
            app.create_dialog.mount_add.error = None;
        }
        KeyCode::Char('n') | KeyCode::Char('N') if app.create_dialog.mount_add.add_field == 0 => {
            app.create_dialog.mount_add.kind = MountKindChoice::Named;
            app.create_dialog.mount_add.sync_selected_volume_from_source();
            app.create_dialog.mount_add.error = None;
        }
        KeyCode::Char('f')
            if mods.contains(KeyModifiers::CONTROL)
                && app.create_dialog.mount_add.kind == MountKindChoice::Bind
                && app.create_dialog.mount_add.add_field == 2 =>
        {
            let initial = app.create_dialog.mount_add.source_input.trim().to_owned();
            let start = if initial.is_empty() { "/" } else { &initial };
            app.create_dialog.mount_add.dir_picker = DirPicker::open(start);
        }
        KeyCode::Char('n')
            if mods.contains(KeyModifiers::CONTROL)
                && app.create_dialog.mount_add.kind == MountKindChoice::Named
                && app.create_dialog.mount_add.add_field == 2 =>
        {
            app.create_dialog.mount_add.new_volume_mode = true;
            app.create_dialog.mount_add.new_volume_name.clear();
            app.create_dialog.mount_add.new_volume_disk = false;
            app.create_dialog.mount_add.error = None;
        }
        KeyCode::Char(' ') if app.create_dialog.mount_add.new_volume_mode => {
            app.create_dialog.mount_add.new_volume_disk = !app.create_dialog.mount_add.new_volume_disk;
        }
        KeyCode::Backspace => {
            let dlg = &mut app.create_dialog.mount_add;
            if dlg.new_volume_mode {
                dlg.new_volume_name.pop();
            } else if dlg.add_field == 1 {
                dlg.guest_input.pop();
            } else if dlg.add_field == 2 && dlg.kind == MountKindChoice::Bind {
                dlg.source_input.pop();
            }
            dlg.error = None;
        }
        KeyCode::Enter => {
            if app.create_dialog.mount_add.new_volume_mode {
                let name = app.create_dialog.mount_add.new_volume_name.trim().to_owned();
                if name.is_empty() {
                    app.create_dialog.mount_add.error = Some("Name cannot be empty".into());
                } else {
                    let tx = app.msg_tx.clone();
                    let disk = app.create_dialog.mount_add.new_volume_disk;
                    app.create_dialog.mount_add.new_volume_mode = false;
                    app.create_dialog.mount_add.source_input = name.clone();
                    tokio::spawn(async move {
                        let result = crate::sandbox::create_volume(&name, disk, None).await;
                        let (msg, is_err) = match result {
                            Ok(()) => (format!("Created volume '{name}'"), false),
                            Err(e) => (format!("Create volume failed: {e}"), true),
                        };
                        let _ = tx.send(AppMessage::Notification(msg, is_err));
                        if let Ok(list) = crate::sandbox::list_volumes().await {
                            let _ = tx.send(AppMessage::VolumeList(Ok(list)));
                        }
                    });
                }
            } else if app.create_dialog.mount_add.add_field < 2 {
                app.create_dialog.mount_add.add_field += 1;
            } else {
                let guest = app.create_dialog.mount_add.guest_input.trim().to_owned();
                let source_val = app.create_dialog.mount_add.source_input.trim().to_owned();
                if guest.is_empty() {
                    app.create_dialog.mount_add.error = Some("Guest path cannot be empty".into());
                } else if source_val.is_empty() {
                    app.create_dialog.mount_add.error =
                        Some("Host path / volume name cannot be empty".into());
                } else {
                    let source = match app.create_dialog.mount_add.kind {
                        MountKindChoice::Bind => MountSource::Bind(source_val),
                        MountKindChoice::Named => MountSource::Named(source_val),
                    };
                    let mount = VolumeMountConfig {
                        guest_path: guest,
                        source,
                    };
                    if let Some(index) = app.create_dialog.mount_add.editing_index {
                        app.create_dialog.mounts[index] = mount;
                        app.create_dialog.mounts_selected = index;
                    } else {
                        app.create_dialog.mounts.push(mount);
                        app.create_dialog.mounts_selected = app.create_dialog.mounts.len() - 1;
                    }
                    app.create_dialog.mount_add.visible = false;
                }
            }
        }
        KeyCode::Char(c) => {
            let dlg = &mut app.create_dialog.mount_add;
            if dlg.new_volume_mode {
                dlg.new_volume_name.push(c);
            } else if dlg.add_field == 1 {
                dlg.guest_input.push(c);
            } else if dlg.add_field == 2 && dlg.kind == MountKindChoice::Bind {
                dlg.source_input.push(c);
            }
            dlg.error = None;
        }
        KeyCode::Up
            if app.create_dialog.mount_add.kind == MountKindChoice::Named
                && app.create_dialog.mount_add.add_field == 2 =>
        {
            let dlg = &mut app.create_dialog.mount_add;
            if dlg.selected_volume > 0 {
                dlg.selected_volume -= 1;
            }
            if let Some(vol) = dlg.available_volumes.get(dlg.selected_volume) {
                dlg.source_input = vol.name.clone();
            }
        }
        KeyCode::Down
            if app.create_dialog.mount_add.kind == MountKindChoice::Named
                && app.create_dialog.mount_add.add_field == 2 =>
        {
            let dlg = &mut app.create_dialog.mount_add;
            if dlg.selected_volume + 1 < dlg.available_volumes.len() {
                dlg.selected_volume += 1;
            }
            if let Some(vol) = dlg.available_volumes.get(dlg.selected_volume) {
                dlg.source_input = vol.name.clone();
            }
        }
        _ => {}
    }
}

fn handle_secret_add_key(app: &mut App, code: KeyCode, _mods: KeyModifiers) {
    let field = app.create_dialog.secret_add.add_field;
    let is_toggle = matches!(field, 3..=7);
    match code {
        KeyCode::Esc => {
            app.create_dialog.secret_add.visible = false;
        }
        KeyCode::Tab | KeyCode::Down => {
            app.create_dialog.secret_add.add_field = (field + 1) % SecretAddDialog::FIELD_COUNT;
        }
        KeyCode::BackTab | KeyCode::Up => {
            app.create_dialog.secret_add.add_field =
                (field + SecretAddDialog::FIELD_COUNT - 1) % SecretAddDialog::FIELD_COUNT;
        }
        KeyCode::Char(' ') if is_toggle => {
            let dlg = &mut app.create_dialog.secret_add;
            match field {
                3 => dlg.inject_headers = !dlg.inject_headers,
                4 => dlg.inject_basic_auth = !dlg.inject_basic_auth,
                5 => dlg.inject_query = !dlg.inject_query,
                6 => dlg.inject_body = !dlg.inject_body,
                7 => dlg.require_tls_identity = !dlg.require_tls_identity,
                _ => {}
            }
            dlg.error = None;
        }
        KeyCode::Backspace if !is_toggle => {
            let dlg = &mut app.create_dialog.secret_add;
            match field {
                0 => {
                    dlg.env_input.pop();
                }
                1 => {
                    dlg.value_input.pop();
                }
                2 => {
                    dlg.hosts_input.pop();
                }
                _ => {}
            }
            dlg.error = None;
        }
        KeyCode::Char(c) if !is_toggle => {
            let dlg = &mut app.create_dialog.secret_add;
            match field {
                0 => {
                    dlg.env_input.push(c);
                }
                1 => {
                    dlg.value_input.push(c);
                }
                2 => {
                    dlg.hosts_input.push(c);
                }
                _ => {}
            }
            dlg.error = None;
        }
        KeyCode::Enter => {
            if field == SecretAddDialog::FIELD_COUNT - 1 {
                submit_secret(app);
            } else {
                app.create_dialog.secret_add.add_field = field + 1;
            }
        }
        _ => {}
    }
}

/// Validates and, if valid, appends the popup's in-progress secret to the
/// Secrets tab's list, closing the popup.
fn submit_secret(app: &mut App) {
    let dlg = app.create_dialog.secret_add.clone();

    let env_var = dlg.env_input.trim().to_owned();
    if env_var.is_empty() {
        app.create_dialog.secret_add.error = Some("Env var name cannot be empty".into());
        return;
    }
    if env_var.contains('=') || env_var.contains('\0') {
        app.create_dialog.secret_add.error = Some("Env var must not contain '=' or NUL".into());
        return;
    }
    let value = dlg.value_input.clone();
    if value.is_empty() {
        app.create_dialog.secret_add.error = Some("Value cannot be empty".into());
        return;
    }
    let allowed_hosts: Vec<SecretHostPattern> = dlg
        .hosts_input
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| {
            if s == "*" {
                SecretHostPattern {
                    kind: SecretHostKind::Any,
                    value: String::new(),
                }
            } else if s.contains('*') {
                SecretHostPattern {
                    kind: SecretHostKind::Wildcard,
                    value: s.to_owned(),
                }
            } else {
                SecretHostPattern {
                    kind: SecretHostKind::Exact,
                    value: s.to_owned(),
                }
            }
        })
        .collect();
    if allowed_hosts.is_empty() {
        app.create_dialog.secret_add.error = Some("At least one allowed host is required".into());
        return;
    }

    let secret = SecretConfig {
        env_var,
        value,
        allowed_hosts,
        inject_headers: dlg.inject_headers,
        inject_basic_auth: dlg.inject_basic_auth,
        inject_query: dlg.inject_query,
        inject_body: dlg.inject_body,
        require_tls_identity: dlg.require_tls_identity,
    };
    if let Some(index) = dlg.editing_index {
        app.create_dialog.secrets[index] = secret;
        app.create_dialog.secrets_selected = index;
    } else {
        app.create_dialog.secrets.push(secret);
        app.create_dialog.secrets_selected = app.create_dialog.secrets.len() - 1;
    }
    app.create_dialog.secret_add.visible = false;
}


fn handle_volumes_view_key(app: &mut App, code: KeyCode, _mods: KeyModifiers) {
    match app.volumes_view.mode {
        SubDialogMode::List => match code {
            KeyCode::Esc => {
                app.volumes_view.visible = false;
            }
            KeyCode::Up => {
                if app.volumes_view.selected > 0 {
                    app.volumes_view.selected -= 1;
                }
            }
            KeyCode::Down => {
                let len = app.volumes_view.volumes.len();
                if len > 0 && app.volumes_view.selected + 1 < len {
                    app.volumes_view.selected += 1;
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                app.volumes_view.mode = SubDialogMode::Add;
                app.volumes_view.name_input.clear();
                app.volumes_view.disk = false;
                app.volumes_view.error = None;
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                if let Some(vol) = app
                    .volumes_view
                    .volumes
                    .get(app.volumes_view.selected)
                    .cloned()
                {
                    app.confirm = Some(PendingAction::RemoveVolume(vol.name));
                }
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                request_volume_refresh(app);
            }
            _ => {}
        },
        SubDialogMode::Add => match code {
            KeyCode::Esc => {
                app.volumes_view.mode = SubDialogMode::List;
                app.volumes_view.error = None;
            }
            KeyCode::Char(' ') => {
                app.volumes_view.disk = !app.volumes_view.disk;
            }
            KeyCode::Backspace => {
                app.volumes_view.name_input.pop();
                app.volumes_view.error = None;
            }
            KeyCode::Enter => {
                let name = app.volumes_view.name_input.trim().to_owned();
                if name.is_empty() {
                    app.volumes_view.error = Some("Name cannot be empty".into());
                } else {
                    let disk = app.volumes_view.disk;
                    app.volumes_view.mode = SubDialogMode::List;
                    app.volumes_view.error = None;
                    let tx = app.msg_tx.clone();
                    tokio::spawn(async move {
                        let result = crate::sandbox::create_volume(&name, disk, None).await;
                        let (msg, is_err) = match result {
                            Ok(()) => (format!("Created volume '{name}'"), false),
                            Err(e) => (format!("Create volume failed: {e}"), true),
                        };
                        let _ = tx.send(AppMessage::Notification(msg, is_err));
                        if let Ok(list) = crate::sandbox::list_volumes().await {
                            let _ = tx.send(AppMessage::VolumeList(Ok(list)));
                        }
                    });
                }
            }
            KeyCode::Char(c) => {
                app.volumes_view.name_input.push(c);
                app.volumes_view.error = None;
            }
            _ => {}
        },
    }
}

/// Handle a keypress while the "Exec" dialog is open.
fn handle_exec_dialog_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc => {
            app.exec_dialog = ExecDialog::default();
        }
        KeyCode::Backspace => {
            app.exec_dialog.command.pop();
            app.exec_dialog.error = None;
        }
        KeyCode::Enter => {
            let command = app.exec_dialog.command.trim().to_owned();
            if command.is_empty() {
                app.exec_dialog.error = Some("Command cannot be empty".into());
                return;
            }
            let name = app.exec_dialog.sandbox_name.clone();
            app.exec_dialog = ExecDialog::default();
            match crate::terminal_launcher::open_exec_terminal(&name, &command) {
                Ok(()) => app.notify(format!("Running '{command}' in '{name}'…"), false),
                Err(e) => app.notify(format!("Failed to open terminal: {e}"), true),
            }
        }
        KeyCode::Char(c) => {
            app.exec_dialog.command.push(c);
            app.exec_dialog.error = None;
        }
        _ => {}
    }
}
