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
    MountSource, NetRuleAction, NetRuleDirection, NetworkRule, VolumeMountConfig,
};

use super::actions::{
    action_remove, action_terminate, action_toggle_start_stop, handle_confirm_key,
    handle_search_key, nav_fs_up, on_sandbox_selected, on_tab_switched, request_volume_refresh,
    scroll_down, scroll_up, submit_create_dialog, PendingAction,
};
use super::dialogs::{
    CreateDialog, DialogTab, DirPicker, EnvVarsDialog, MountKindChoice, MountsDialog,
    NetworkRulesDialog, PortsDialog, SubDialogMode, VolumesView, DRIVES_ENTRY, PICKER_VISIBLE_ROWS,
};
use super::{App, AppMessage, DetailTab, Focus};

/// Returns true if the point `(x, y)` falls within `rect`.
fn point_in_rect(x: u16, y: u16, rect: Rect) -> bool {
    rect.x <= x && x < rect.x + rect.width && rect.y <= y && y < rect.y + rect.height
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
    // make Esc silently do nothing on those terminals.
    // This ensures every physical keypress is handled exactly once,
    // regardless of how many events the terminal emits per keystroke.
    if key.kind == KeyEventKind::Release && key.code != KeyCode::Esc {
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
        } else if app.create_dialog.ports_dialog.visible {
            handle_ports_dialog_key(app, key.code, key.modifiers);
        } else if app.create_dialog.env_vars_dialog.visible {
            handle_env_vars_dialog_key(app, key.code, key.modifiers);
        } else if app.create_dialog.network_rules_dialog.visible {
            handle_network_rules_dialog_key(app, key.code, key.modifiers);
        } else if app.create_dialog.mounts_dialog.visible {
            handle_mounts_dialog_key(app, key.code, key.modifiers);
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
            request_volume_refresh(app);
        }
        KeyCode::Char('r') => {
            app.request_refresh();
            app.notify("Refreshing…", false);
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
    match code {
        KeyCode::Esc => {
            app.create_dialog = Default::default();
        }
        KeyCode::Tab | KeyCode::Down => app.create_dialog.next_field(),
        KeyCode::BackTab | KeyCode::Up => app.create_dialog.prev_field(),
        KeyCode::Left => {
            let prev = app.create_dialog.tab.prev();
            app.create_dialog.switch_tab(prev);
        }
        KeyCode::Right => {
            let next = app.create_dialog.tab.next();
            app.create_dialog.switch_tab(next);
        }
        KeyCode::Char(' ') if app.create_dialog.is_toggle_field() => {
            app.create_dialog.disable_network = !app.create_dialog.disable_network;
            app.create_dialog.error = None;
        }
        KeyCode::Enter => {
            let dlg = &app.create_dialog;
            if dlg.is_create_focused() {
                submit_create_dialog(app);
            } else if dlg.tab == DialogTab::Basic && dlg.field == 4 {
                let entries = app.create_dialog.ports.clone();
                app.create_dialog.ports_dialog = PortsDialog::open(entries);
            } else if dlg.tab == DialogTab::Basic && dlg.field == 5 {
                let entries = app.create_dialog.env_vars.clone();
                app.create_dialog.env_vars_dialog = EnvVarsDialog::open(entries);
            } else if dlg.tab == DialogTab::Basic && dlg.field == 6 {
                let initial = app.create_dialog.workdir.trim().to_owned();
                let start = if initial.is_empty() { "/" } else { &initial };
                app.create_dialog.dir_picker = DirPicker::open(start);
            } else if dlg.tab == DialogTab::Advanced && dlg.field == 6 {
                let entries = app.create_dialog.network_rules.clone();
                app.create_dialog.network_rules_dialog = NetworkRulesDialog::open(entries);
            } else if dlg.tab == DialogTab::Basic && dlg.field == 7 {
                let entries = app.create_dialog.mounts.clone();
                app.create_dialog.mounts_dialog = MountsDialog::open(entries);
            }
            // Enter on plain text fields moves to next field.
            else {
                app.create_dialog.next_field();
            }
        }
        KeyCode::Backspace => {
            if let Some(field) = app.create_dialog.current_field_mut() {
                field.pop();
            }
            app.create_dialog.error = None;
        }
        KeyCode::Char(c) => {
            if app.create_dialog.is_toggle_field() || app.create_dialog.is_create_focused() {
                return;
            }
            // Managed fields (ports, env vars, workdir) don't accept direct text input.
            if app.create_dialog.tab == DialogTab::Basic && matches!(app.create_dialog.field, 4..=7)
            {
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

fn handle_picker_key(app: &mut App, code: KeyCode, _mods: KeyModifiers) {
    if code == KeyCode::Esc {
        app.create_dialog.dir_picker.visible = false;
        return;
    }

    let picker = &mut app.create_dialog.dir_picker;
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
                app.create_dialog.workdir = chosen;
            }
            app.create_dialog.dir_picker.visible = false;
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

fn handle_ports_dialog_key(app: &mut App, code: KeyCode, _mods: KeyModifiers) {
    match app.create_dialog.ports_dialog.mode {
        SubDialogMode::List => {
            match code {
                KeyCode::Esc | KeyCode::Enter => {
                    // Sync confirmed entries back to the parent field and close.
                    let entries = app.create_dialog.ports_dialog.entries.clone();
                    app.create_dialog.ports = entries;
                    app.create_dialog.ports_dialog.visible = false;
                }
                KeyCode::Up => {
                    if app.create_dialog.ports_dialog.selected > 0 {
                        app.create_dialog.ports_dialog.selected -= 1;
                    }
                }
                KeyCode::Down => {
                    let len = app.create_dialog.ports_dialog.entries.len();
                    if len > 0 && app.create_dialog.ports_dialog.selected + 1 < len {
                        app.create_dialog.ports_dialog.selected += 1;
                    }
                }
                KeyCode::Char('a') | KeyCode::Char('A') => {
                    app.create_dialog.ports_dialog.mode = SubDialogMode::Add;
                    app.create_dialog.ports_dialog.host_input.clear();
                    app.create_dialog.ports_dialog.guest_input.clear();
                    app.create_dialog.ports_dialog.add_field = 0;
                    app.create_dialog.ports_dialog.error = None;
                }
                KeyCode::Char('d') | KeyCode::Delete => {
                    let dialog = &mut app.create_dialog.ports_dialog;
                    if !dialog.entries.is_empty() {
                        dialog.entries.remove(dialog.selected);
                        if dialog.selected >= dialog.entries.len() && dialog.selected > 0 {
                            dialog.selected -= 1;
                        }
                        dialog.error = None;
                    }
                }
                _ => {}
            }
        }
        SubDialogMode::Add => match code {
            KeyCode::Esc => {
                app.create_dialog.ports_dialog.mode = SubDialogMode::List;
                app.create_dialog.ports_dialog.error = None;
            }
            KeyCode::Tab | KeyCode::Down => {
                let f = app.create_dialog.ports_dialog.add_field;
                app.create_dialog.ports_dialog.add_field = (f + 1) % 2;
            }
            KeyCode::BackTab | KeyCode::Up => {
                let f = app.create_dialog.ports_dialog.add_field;
                app.create_dialog.ports_dialog.add_field = (f + 1) % 2;
            }
            KeyCode::Backspace => {
                let dialog = &mut app.create_dialog.ports_dialog;
                if dialog.add_field == 0 {
                    dialog.host_input.pop();
                } else {
                    dialog.guest_input.pop();
                }
                dialog.error = None;
            }
            KeyCode::Enter => {
                let dialog = &mut app.create_dialog.ports_dialog;
                if dialog.add_field == 0 {
                    // Move focus to the guest port field.
                    dialog.add_field = 1;
                } else {
                    let host = dialog.host_input.trim().parse::<u16>();
                    let guest = dialog.guest_input.trim().parse::<u16>();
                    match (host, guest) {
                        (Ok(h), Ok(g)) => {
                            dialog.entries.push((h, g));
                            dialog.selected = dialog.entries.len().saturating_sub(1);
                            dialog.mode = SubDialogMode::List;
                            dialog.error = None;
                        }
                        (Err(_), _) => {
                            dialog.error = Some("Invalid host port (0–65535)".into());
                        }
                        (_, Err(_)) => {
                            dialog.error = Some("Invalid guest port (0–65535)".into());
                        }
                    }
                }
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                let dialog = &mut app.create_dialog.ports_dialog;
                if dialog.add_field == 0 {
                    dialog.host_input.push(c);
                } else {
                    dialog.guest_input.push(c);
                }
                dialog.error = None;
            }
            _ => {}
        },
    }
}

fn handle_env_vars_dialog_key(app: &mut App, code: KeyCode, _mods: KeyModifiers) {
    match app.create_dialog.env_vars_dialog.mode {
        SubDialogMode::List => {
            match code {
                KeyCode::Esc | KeyCode::Enter => {
                    // Sync confirmed entries back to the parent field and close.
                    let entries = app.create_dialog.env_vars_dialog.entries.clone();
                    app.create_dialog.env_vars = entries;
                    app.create_dialog.env_vars_dialog.visible = false;
                }
                KeyCode::Up => {
                    if app.create_dialog.env_vars_dialog.selected > 0 {
                        app.create_dialog.env_vars_dialog.selected -= 1;
                    }
                }
                KeyCode::Down => {
                    let len = app.create_dialog.env_vars_dialog.entries.len();
                    if len > 0 && app.create_dialog.env_vars_dialog.selected + 1 < len {
                        app.create_dialog.env_vars_dialog.selected += 1;
                    }
                }
                KeyCode::Char('a') | KeyCode::Char('A') => {
                    app.create_dialog.env_vars_dialog.mode = SubDialogMode::Add;
                    app.create_dialog.env_vars_dialog.key_input.clear();
                    app.create_dialog.env_vars_dialog.value_input.clear();
                    app.create_dialog.env_vars_dialog.add_field = 0;
                    app.create_dialog.env_vars_dialog.error = None;
                }
                KeyCode::Char('d') | KeyCode::Delete => {
                    let dialog = &mut app.create_dialog.env_vars_dialog;
                    if !dialog.entries.is_empty() {
                        dialog.entries.remove(dialog.selected);
                        if dialog.selected >= dialog.entries.len() && dialog.selected > 0 {
                            dialog.selected -= 1;
                        }
                        dialog.error = None;
                    }
                }
                _ => {}
            }
        }
        SubDialogMode::Add => match code {
            KeyCode::Esc => {
                app.create_dialog.env_vars_dialog.mode = SubDialogMode::List;
                app.create_dialog.env_vars_dialog.error = None;
            }
            KeyCode::Tab | KeyCode::Down => {
                let f = app.create_dialog.env_vars_dialog.add_field;
                app.create_dialog.env_vars_dialog.add_field = (f + 1) % 2;
            }
            KeyCode::BackTab | KeyCode::Up => {
                let f = app.create_dialog.env_vars_dialog.add_field;
                app.create_dialog.env_vars_dialog.add_field = (f + 1) % 2;
            }
            KeyCode::Backspace => {
                let dialog = &mut app.create_dialog.env_vars_dialog;
                if dialog.add_field == 0 {
                    dialog.key_input.pop();
                } else {
                    dialog.value_input.pop();
                }
                dialog.error = None;
            }
            KeyCode::Enter => {
                let dialog = &mut app.create_dialog.env_vars_dialog;
                if dialog.add_field == 0 {
                    // Move focus to the value field.
                    dialog.add_field = 1;
                } else {
                    let key = dialog.key_input.trim().to_owned();
                    let value = dialog.value_input.clone();
                    if key.is_empty() {
                        dialog.error = Some("Key cannot be empty".into());
                    } else if key.contains('=') {
                        dialog.error = Some("Key must not contain '='".into());
                    } else {
                        dialog.entries.push((key, value));
                        dialog.selected = dialog.entries.len().saturating_sub(1);
                        dialog.mode = SubDialogMode::List;
                        dialog.error = None;
                    }
                }
            }
            KeyCode::Char(c) => {
                let dialog = &mut app.create_dialog.env_vars_dialog;
                // Disallow '=' in the key field.
                if dialog.add_field == 0 && c == '=' {
                    dialog.error = Some("Key must not contain '='".into());
                    return;
                }
                if dialog.add_field == 0 {
                    dialog.key_input.push(c);
                } else {
                    dialog.value_input.push(c);
                }
                dialog.error = None;
            }
            _ => {}
        },
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

fn handle_network_rules_dialog_key(app: &mut App, code: KeyCode, _mods: KeyModifiers) {
    match app.create_dialog.network_rules_dialog.mode {
        SubDialogMode::List => match code {
            KeyCode::Esc | KeyCode::Enter => {
                // Sync confirmed entries back to the parent field and close.
                let entries = app.create_dialog.network_rules_dialog.entries.clone();
                app.create_dialog.network_rules = entries;
                app.create_dialog.network_rules_dialog.visible = false;
            }
            KeyCode::Up => {
                if app.create_dialog.network_rules_dialog.selected > 0 {
                    app.create_dialog.network_rules_dialog.selected -= 1;
                }
            }
            KeyCode::Down => {
                let len = app.create_dialog.network_rules_dialog.entries.len();
                if len > 0 && app.create_dialog.network_rules_dialog.selected + 1 < len {
                    app.create_dialog.network_rules_dialog.selected += 1;
                }
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                let dialog = &mut app.create_dialog.network_rules_dialog;
                dialog.mode = SubDialogMode::Add;
                dialog.cidr_input.clear();
                dialog.action = NetRuleAction::Allow;
                dialog.direction = NetRuleDirection::Egress;
                dialog.error = None;
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                let dialog = &mut app.create_dialog.network_rules_dialog;
                if !dialog.entries.is_empty() {
                    dialog.entries.remove(dialog.selected);
                    if dialog.selected >= dialog.entries.len() && dialog.selected > 0 {
                        dialog.selected -= 1;
                    }
                    dialog.error = None;
                }
            }
            _ => {}
        },
        SubDialogMode::Add => match code {
            KeyCode::Esc => {
                app.create_dialog.network_rules_dialog.mode = SubDialogMode::List;
                app.create_dialog.network_rules_dialog.error = None;
            }
            KeyCode::Char('e') | KeyCode::Char('E') => {
                app.create_dialog.network_rules_dialog.direction = NetRuleDirection::Egress;
            }
            KeyCode::Char('i') | KeyCode::Char('I') => {
                app.create_dialog.network_rules_dialog.direction = NetRuleDirection::Ingress;
            }
            KeyCode::Char(' ') => {
                let dialog = &mut app.create_dialog.network_rules_dialog;
                dialog.action = match dialog.action {
                    NetRuleAction::Allow => NetRuleAction::Deny,
                    NetRuleAction::Deny => NetRuleAction::Allow,
                };
            }
            KeyCode::Backspace => {
                app.create_dialog.network_rules_dialog.cidr_input.pop();
                app.create_dialog.network_rules_dialog.error = None;
            }
            KeyCode::Enter => {
                let dialog = &mut app.create_dialog.network_rules_dialog;
                let cidr = dialog.cidr_input.trim().to_owned();
                match validate_cidr(&cidr) {
                    Ok(()) => {
                        dialog.entries.push(NetworkRule {
                            cidr,
                            action: dialog.action,
                            direction: dialog.direction,
                        });
                        dialog.selected = dialog.entries.len().saturating_sub(1);
                        dialog.mode = SubDialogMode::List;
                        dialog.error = None;
                    }
                    Err(e) => dialog.error = Some(e.to_string()),
                }
            }
            KeyCode::Char(c) => {
                app.create_dialog.network_rules_dialog.cidr_input.push(c);
                app.create_dialog.network_rules_dialog.error = None;
            }
            _ => {}
        },
    }
}

fn handle_mounts_dialog_key(app: &mut App, code: KeyCode, _mods: KeyModifiers) {
    match app.create_dialog.mounts_dialog.mode {
        SubDialogMode::List => match code {
            KeyCode::Esc | KeyCode::Enter => {
                // Sync confirmed entries back to the parent field and close.
                let entries = app.create_dialog.mounts_dialog.entries.clone();
                app.create_dialog.mounts = entries;
                app.create_dialog.mounts_dialog.visible = false;
            }
            KeyCode::Up => {
                if app.create_dialog.mounts_dialog.selected > 0 {
                    app.create_dialog.mounts_dialog.selected -= 1;
                }
            }
            KeyCode::Down => {
                let len = app.create_dialog.mounts_dialog.entries.len();
                if len > 0 && app.create_dialog.mounts_dialog.selected + 1 < len {
                    app.create_dialog.mounts_dialog.selected += 1;
                }
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                let dialog = &mut app.create_dialog.mounts_dialog;
                dialog.mode = SubDialogMode::Add;
                dialog.guest_input.clear();
                dialog.source_input.clear();
                dialog.kind = MountKindChoice::Bind;
                dialog.add_field = 0;
                dialog.error = None;
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                let dialog = &mut app.create_dialog.mounts_dialog;
                if !dialog.entries.is_empty() {
                    dialog.entries.remove(dialog.selected);
                    if dialog.selected >= dialog.entries.len() && dialog.selected > 0 {
                        dialog.selected -= 1;
                    }
                    dialog.error = None;
                }
            }
            _ => {}
        },
        SubDialogMode::Add => match code {
            KeyCode::Esc => {
                app.create_dialog.mounts_dialog.mode = SubDialogMode::List;
                app.create_dialog.mounts_dialog.error = None;
            }
            KeyCode::Tab | KeyCode::Down => {
                let f = app.create_dialog.mounts_dialog.add_field;
                app.create_dialog.mounts_dialog.add_field = (f + 1) % 2;
            }
            KeyCode::BackTab | KeyCode::Up => {
                let f = app.create_dialog.mounts_dialog.add_field;
                app.create_dialog.mounts_dialog.add_field = (f + 1) % 2;
            }
            KeyCode::Char('b') | KeyCode::Char('B')
                if app.create_dialog.mounts_dialog.add_field == 1 =>
            {
                app.create_dialog.mounts_dialog.kind = MountKindChoice::Bind;
            }
            KeyCode::Char('n') | KeyCode::Char('N')
                if app.create_dialog.mounts_dialog.add_field == 1 =>
            {
                app.create_dialog.mounts_dialog.kind = MountKindChoice::Named;
            }
            KeyCode::Backspace => {
                let dialog = &mut app.create_dialog.mounts_dialog;
                if dialog.add_field == 0 {
                    dialog.guest_input.pop();
                } else {
                    dialog.source_input.pop();
                }
                dialog.error = None;
            }
            KeyCode::Enter => {
                let dialog = &mut app.create_dialog.mounts_dialog;
                if dialog.add_field == 0 {
                    dialog.add_field = 1;
                } else {
                    let guest = dialog.guest_input.trim().to_owned();
                    let source_val = dialog.source_input.trim().to_owned();
                    if guest.is_empty() {
                        dialog.error = Some("Guest path cannot be empty".into());
                    } else if source_val.is_empty() {
                        dialog.error = Some("Host path / volume name cannot be empty".into());
                    } else {
                        let source = match dialog.kind {
                            MountKindChoice::Bind => MountSource::Bind(source_val),
                            MountKindChoice::Named => MountSource::Named(source_val),
                        };
                        dialog.entries.push(VolumeMountConfig {
                            guest_path: guest,
                            source,
                        });
                        dialog.selected = dialog.entries.len().saturating_sub(1);
                        dialog.mode = SubDialogMode::List;
                        dialog.error = None;
                    }
                }
            }
            KeyCode::Char(c) => {
                let dialog = &mut app.create_dialog.mounts_dialog;
                if dialog.add_field == 0 {
                    dialog.guest_input.push(c);
                } else {
                    dialog.source_input.push(c);
                }
                dialog.error = None;
            }
            _ => {}
        },
    }
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
