//! "New Sandbox" creation modal dialog.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Padding, Paragraph},
    Frame,
};

use crate::app::{
    App, CreateDialog, DialogTab, ListField, MountKindChoice, DRIVES_ENTRY, PICKER_VISIBLE_ROWS,
};
use crate::sandbox::{
    MountSource, NetRuleAction, NetRuleDestKind, NetRuleDirection, NetworkRule, SecretConfig,
    VolumeMountConfig, ViolationActionChoice,
};
use crate::theme::Theme;

/// Number of entries shown at once inside an inline scrollable list field
/// (Env Vars, Mounts, Ports, Net Rules, Secrets).
const VISIBLE_LIST_ROWS: u16 = 4;
/// Height of a plain single-line bordered text/toggle field.
const FIELD_HEIGHT: u16 = 3;
/// Height of a bordered inline list field: border(2) + entries + hint(1).
const LIST_HEIGHT: u16 = VISIBLE_LIST_ROWS + 3;
const TALL_LIST_HEIGHT: u16 = 9;
const FIXED_CONTENT_HEIGHT: u16 = 33;

/// Canonical rows-per-column used to lay out an inline list field
/// (Env Vars, Mounts, Ports, Net Rules, Secrets) into multiple side-by-side
/// columns once it holds more entries than fit vertically. Shared with
/// `keys.rs::move_list_column` so a Left/Right "jump one column" key press
/// lands on the same column boundaries the renderer used.
pub(crate) fn list_column_rows(list: ListField) -> usize {
    match list {
        ListField::NetworkRules => (TALL_LIST_HEIGHT - 3) as usize,
        ListField::EnvVars | ListField::Mounts | ListField::Ports | ListField::Secrets => {
            VISIBLE_LIST_ROWS as usize
        }
    }
}

/// Render the create-sandbox dialog centred over the full terminal area.
pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let dlg = &app.create_dialog;
    let theme = &app.theme;
    if !dlg.visible {
        return;
    }

    // Per-tab visual row heights. Every tab is a 1:1 field-to-row mapping
    // except Basic, where CPUs/Max CPUs and Memory/Max Memory each share a
    // row (see `render` below for the pairing).
    let content_height: u16 = FIXED_CONTENT_HEIGHT;
    // borders(2) + top padding(1) + tab bar(1) + top separator(1) + content
    // + bottom separator(1) + action row (hint/error + create button, 3)
    let popup_height = 9 + content_height;

    let popup = centred_rect(70, popup_height, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(Span::styled(" New Sandbox ", theme.accent_bold()))
        .borders(Borders::ALL)
        .border_type(theme.border_unfocused_type)
        .border_style(theme.accent())
        .style(theme.base_style())
        .padding(Padding::new(1, 1, 1, 0));

    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let mut constraints = vec![Constraint::Length(1), Constraint::Length(1)];
    match dlg.tab {
        DialogTab::Basic => constraints.extend([
            Constraint::Length(FIELD_HEIGHT),
            Constraint::Length(FIELD_HEIGHT),
            Constraint::Length(FIELD_HEIGHT),
            Constraint::Length(FIELD_HEIGHT),
            Constraint::Length(FIELD_HEIGHT),
            Constraint::Min(0),
        ]),
        DialogTab::GuestOs => constraints.extend([
            Constraint::Length(FIELD_HEIGHT),
            Constraint::Length(FIELD_HEIGHT),
            Constraint::Length(LIST_HEIGHT),
            Constraint::Length(LIST_HEIGHT),
            Constraint::Min(0),
        ]),
        DialogTab::Network => constraints.extend([
            Constraint::Length(FIELD_HEIGHT),
            Constraint::Length(LIST_HEIGHT),
            Constraint::Min(TALL_LIST_HEIGHT),
        ]),
        DialogTab::Dns => constraints.extend([
            Constraint::Length(FIELD_HEIGHT),
            Constraint::Length(FIELD_HEIGHT),
            Constraint::Length(FIELD_HEIGHT),
            Constraint::Min(0),
        ]),
        DialogTab::Tls => constraints.extend([
            Constraint::Length(FIELD_HEIGHT),
            Constraint::Length(FIELD_HEIGHT),
            Constraint::Length(FIELD_HEIGHT),
            Constraint::Length(FIELD_HEIGHT),
            Constraint::Min(0),
        ]),
        DialogTab::Security => constraints.extend([
            Constraint::Length(FIELD_HEIGHT),
            Constraint::Length(FIELD_HEIGHT),
            Constraint::Length(FIELD_HEIGHT),
            Constraint::Length(FIELD_HEIGHT),
            Constraint::Length(LIST_HEIGHT),
            Constraint::Min(0),
        ]),
    }
    constraints.push(Constraint::Length(1)); // separator above create button
    constraints.push(Constraint::Length(3)); // action row: hint/error (left) + create button (right)

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    render_tab_bar(f, theme, dlg, chunks[0]);
    render_separator(f, theme, chunks[1]);

    match dlg.tab {
        DialogTab::Basic => {
            let workdir_summary = if dlg.workdir.is_empty() {
                "(none)".into()
            } else {
                dlg.workdir.clone()
            };
            render_field(f, theme, "Name", &dlg.name, dlg.field == 0, chunks[2], true, false);
            render_field(f, theme, "Image", &dlg.image, dlg.field == 1, chunks[3], true, false);
            render_field_pair(
                f,
                theme,
                "CPUs",
                &dlg.cpus,
                dlg.field == 2,
                "Max CPUs",
                &dlg.max_cpus,
                dlg.field == 3,
                chunks[4],
                true,
                false,
            );
            render_field_pair(
                f,
                theme,
                "Memory",
                &dlg.memory,
                dlg.field == 4,
                "Max Mem",
                &dlg.max_memory,
                dlg.field == 5,
                chunks[5],
                true,
                false,
            );
            render_managed_field_with_hint(
                f,
                theme,
                "Workdir ",
                &workdir_summary,
                "browse",
                dlg.field == 6,
                chunks[6],
            );
        }
        DialogTab::GuestOs => {
            render_field(f, theme, "Hostname", &dlg.hostname, dlg.field == 0, chunks[2], false, false);
            render_field(f, theme, "Shell", &dlg.shell, dlg.field == 1, chunks[3], true, false);
            render_list_field(
                f,
                theme,
                "Env Vars",
                &dlg.env_vars,
                dlg.env_vars_selected,
                dlg.field == 2,
                dlg.list_edit_mode,
                |(k, v)| format!("{k}={v}"),
                chunks[4],
                false,
                ListField::EnvVars,
            );
            render_list_field(
                f,
                theme,
                "Mounts  ",
                &dlg.mounts,
                dlg.mounts_selected,
                dlg.field == 3,
                dlg.list_edit_mode,
                format_mount_entry,
                chunks[5],
                false,
                ListField::Mounts,
            );
        }
        DialogTab::Network => {
            let row = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(33),
                    Constraint::Percentage(33),
                    Constraint::Percentage(34),
                ])
                .split(chunks[2]);
            render_toggle(
                f,
                theme,
                "Network access",
                !dlg.disable_network,
                dlg.field == 0,
                row[0],
                false,
                Some(("Enabled", "Disabled")),
            );
            render_radio_field(
                f,
                theme,
                "Default Ingress",
                &["Allow", "Deny"],
                if dlg.default_ingress_action == NetRuleAction::Allow { 0 } else { 1 },
                dlg.field == 1,
                row[1],
                dlg.field_disabled_by_network(DialogTab::Network, 1),
                Some(&[true, false]),
            );
            render_radio_field(
                f,
                theme,
                "Default Egress",
                &["Allow", "Deny"],
                if dlg.default_egress_action == NetRuleAction::Allow { 0 } else { 1 },
                dlg.field == 2,
                row[2],
                dlg.field_disabled_by_network(DialogTab::Network, 2),
                Some(&[true, false]),
            );
            render_list_field(
                f,
                theme,
                "Port Mappings",
                &dlg.ports,
                dlg.ports_selected,
                dlg.field == 3,
                dlg.list_edit_mode,
                |(h, g)| format!("{h} → {g}"),
                chunks[3],
                dlg.field_disabled_by_network(DialogTab::Network, 3),
                ListField::Ports,
            );
            render_list_field(
                f,
                theme,
                "Net Rules (first match wins)",
                &dlg.network_rules,
                dlg.network_rules_selected,
                dlg.field == 4,
                dlg.list_edit_mode,
                NetworkRule::summary,
                chunks[4],
                dlg.field_disabled_by_network(DialogTab::Network, 4),
                ListField::NetworkRules,
            );
        }
        DialogTab::Dns => {
            render_field(
                f,
                theme,
                "DNS Nameservers",
                &dlg.dns_nameservers,
                dlg.field == 0,
                chunks[2],
                false,
                dlg.field_disabled_by_network(DialogTab::Dns, 0),
            );
            render_field(
                f,
                theme,
                "DNS Timeout",
                &dlg.dns_query_timeout_ms,
                dlg.field == 1,
                chunks[3],
                false,
                dlg.field_disabled_by_network(DialogTab::Dns, 1),
            );
            render_toggle(
                f,
                theme,
                "Rebind Protection",
                dlg.dns_rebind_protection,
                dlg.field == 2,
                chunks[4],
                dlg.field_disabled_by_network(DialogTab::Dns, 2),
                None,
            );
        }
        DialogTab::Tls => {
            render_toggle(
                f,
                theme,
                "TLS",
                dlg.tls_enabled,
                dlg.field == 0,
                chunks[2],
                dlg.field_disabled_by_network(DialogTab::Tls, 0),
                None,
            );
            render_field(
                f,
                theme,
                "TLS Bypass",
                &dlg.tls_bypass_patterns,
                dlg.field == 1,
                chunks[3],
                false,
                dlg.field_disabled_by_network(DialogTab::Tls, 1),
            );
            render_field(
                f,
                theme,
                "TLS Ports",
                &dlg.tls_intercepted_ports,
                dlg.field == 2,
                chunks[4],
                false,
                dlg.field_disabled_by_network(DialogTab::Tls, 2),
            );
            let tls_row = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(chunks[5]);
            render_toggle(
                f,
                theme,
                "Verify Upstream",
                dlg.tls_verify_upstream,
                dlg.field == 3,
                tls_row[0],
                dlg.field_disabled_by_network(DialogTab::Tls, 3),
                None,
            );
            render_toggle(
                f,
                theme,
                "Block QUIC",
                dlg.tls_block_quic,
                dlg.field == 4,
                tls_row[1],
                dlg.field_disabled_by_network(DialogTab::Tls, 4),
                None,
            );
        }
        DialogTab::Security => {
            render_field(f, theme, "User", &dlg.user, dlg.field == 0, chunks[2], false, false);
            render_radio_field(
                f,
                theme,
                "Violation",
                &["BLOCK", "BLOCK+LOG", "BLOCK+TERM"],
                match dlg.violation_action {
                    ViolationActionChoice::Block => 0,
                    ViolationActionChoice::BlockAndLog => 1,
                    ViolationActionChoice::BlockAndTerminate => 2,
                },
                dlg.field == 1,
                chunks[3],
                false,
                None,
            );
            render_field(
                f,
                theme,
                "Pass Hosts",
                &dlg.violation_passthrough_hosts,
                dlg.field == 2,
                chunks[4],
                false,
                false,
            );
            render_field(
                f,
                theme,
                "Pass Patterns",
                &dlg.violation_passthrough_patterns,
                dlg.field == 3,
                chunks[5],
                false,
                false,
            );
            render_list_field(
                f,
                theme,
                "Secrets",
                &dlg.secrets,
                dlg.secrets_selected,
                dlg.field == 4,
                dlg.list_edit_mode,
                SecretConfig::summary,
                chunks[6],
                false,
                ListField::Secrets,
            );
        }
    }

    // Horizontal rule directly above the action row, then the action row
    // itself: hint/error on the left, Create button docked to the right,
    // both vertically centred within the row (button height 3: border,
    // text, border — hint/error sits on that same middle line).
    let separator_chunk = chunks[chunks.len() - 2];
    render_separator(f, theme, separator_chunk);

    let action_row = chunks[chunks.len() - 1];
    let button_width = create_button_width("+ Create Sandbox", action_row.width);
    let action_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(button_width)])
        .split(action_row);
    let message_chunk = Rect::new(
        action_cols[0].x,
        action_cols[0].y + action_cols[0].height.saturating_sub(1) / 2,
        action_cols[0].width,
        1,
    );
    render_create_button(f, theme, dlg.is_create_focused(), action_cols[1]);

    let mut hint_pairs: Vec<(&str, &str)> = vec![("Tab/↑↓/◄►", "navigate")];
    if dlg.is_create_focused() {
        hint_pairs.push(("Enter", "create sandbox"));
    } else {
        if dlg.focused_list().is_some() {
            hint_pairs.push(("Enter", "edit mode"));
        } else if dlg.tab == DialogTab::Basic && dlg.field == 6 {
            hint_pairs.push(("Enter", "browse"));
        } else if dlg.is_toggle_field() {
            hint_pairs.push(("Space", "toggle"));
        }
    }
    hint_pairs.push(("Esc", "cancel"));
    let hint_pairs = hint_pairs;
    if let Some(ref err) = dlg.error {
        f.render_widget(
            Paragraph::new(Span::styled(format!("✗ {err}"), theme.danger())),
            message_chunk,
        );
    } else {
        f.render_widget(Paragraph::new(theme.hint_line(&hint_pairs)), message_chunk);
    }

    if dlg.dir_picker.visible {
        render_dir_picker(f, app, area);
    }
    if dlg.port_add.visible {
        render_port_add_dialog(f, app, area);
    }
    if dlg.env_var_add.visible {
        render_env_var_add_dialog(f, app, area);
    }
    if dlg.net_rule_add.visible {
        render_net_rule_add_dialog(f, app, area);
    }
    if dlg.mount_add.visible {
        render_mount_add_dialog(f, app, area);
    }
    if dlg.secret_add.visible {
        render_secret_add_dialog(f, app, area);
    }
}

/// Horizontal rule directly below the tab bar, matching the sandbox detail
/// view's tab/content divider.
fn render_separator(f: &mut Frame, theme: &Theme, area: Rect) {
    let line = "─".repeat(area.width as usize);
    f.render_widget(Paragraph::new(Span::styled(line, theme.muted())), area);
}

fn render_tab_bar(f: &mut Frame, theme: &Theme, dlg: &CreateDialog, area: Rect) {
    let tab = dlg.tab;
    let labels = [
        (DialogTab::Basic.title(), tab == DialogTab::Basic, false),
        (DialogTab::GuestOs.title(), tab == DialogTab::GuestOs, false),
        (DialogTab::Network.title(), tab == DialogTab::Network, false),
        (
            DialogTab::Dns.title(),
            tab == DialogTab::Dns,
            dlg.is_tab_disabled(DialogTab::Dns),
        ),
        (
            DialogTab::Tls.title(),
            tab == DialogTab::Tls,
            dlg.is_tab_disabled(DialogTab::Tls),
        ),
        (DialogTab::Security.title(), tab == DialogTab::Security, false),
    ];
    let (spans, _) = theme.tab_bar(&labels);

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_field(
    f: &mut Frame,
    theme: &Theme,
    label: &str,
    value: &str,
    focused: bool,
    area: Rect,
    required: bool,
    disabled: bool,
) {
    let label_style = if disabled {
        theme.disabled().add_modifier(Modifier::BOLD)
    } else if focused {
        theme.text_bold()
    } else {
        theme.muted().add_modifier(Modifier::BOLD)
    };

    let title = if required {
        Line::from(vec![
            Span::styled(format!(" {label} "), label_style),
            Span::styled("* ", theme.danger()),
        ])
    } else {
        Line::from(vec![Span::styled(format!(" {label} "), label_style)])
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(theme.border_type(focused))
        .border_style(theme.border_style(focused));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let text_area = Rect::new(
        inner.x,
        inner.y + inner.height.saturating_sub(1),
        inner.width,
        inner.height.min(1),
    );

    let display = if focused {
        format!(" {value}▌")
    } else {
        format!(" {value}")
    };

    let value_style = if disabled {
        theme.disabled()
    } else if focused {
        theme.text()
    } else {
        theme.secondary()
    };

    f.render_widget(
        Paragraph::new(Span::styled(display, value_style)),
        text_area,
    );
}

/// Render two text fields side by side within a single row, e.g. CPUs / Max
/// CPUs or Memory / Max Memory.
fn render_field_pair(
    f: &mut Frame,
    theme: &Theme,
    label_a: &str,
    value_a: &str,
    focused_a: bool,
    label_b: &str,
    value_b: &str,
    focused_b: bool,
    area: Rect,
    required_a: bool,
    required_b: bool,
) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);
    render_field(f, theme, label_a, value_a, focused_a, cols[0], required_a, false);
    render_field(f, theme, label_b, value_b, focused_b, cols[1], required_b, false);
}

fn render_toggle(
    f: &mut Frame,
    theme: &Theme,
    label: &str,
    value: bool,
    focused: bool,
    area: Rect,
    disabled: bool,
    labels: Option<(&str, &str)>,
) {
    let on_text = labels.map(|v| v.0).unwrap_or("On");
    let off_text = labels.map(|v| v.1).unwrap_or("Off");
    render_radio_field(
        f,
        theme,
        label,
        &[on_text, off_text],
        if value { 0 } else { 1 },
        focused,
        area,
        disabled,
        Some(&[true, false]),
    );
}

/// Render a fixed set of mutually-exclusive choices as inline radio
/// options (`● Selected  ○ Other  ○ Other`), toggled only by Space (see
/// `CreateDialog::is_toggle_field` / the popups' own key handlers) — never
/// by Left/Right, which are reserved for switching dialog tabs. Used for
/// every field with a small fixed value set: network access, ingress/
/// egress defaults, mount kind, rebind protection, net rule action/
/// direction, TLS on/off, verify upstream, block QUIC, and violation
/// action.
///
/// `colors`, when given, must have one entry per option: `true` renders
/// that option green when selected (e.g. Allow/Enabled/On), `false` red
/// (Deny/Disabled/Off). `None` renders the selected option in the normal
/// focus/text color instead, for choices with no inherent good/bad
/// connotation (e.g. the violation action).
fn render_radio_field(
    f: &mut Frame,
    theme: &Theme,
    label: &str,
    options: &[&str],
    selected: usize,
    focused: bool,
    area: Rect,
    disabled: bool,
    colors: Option<&[bool]>,
) {
    let label_style = if disabled {
        theme.disabled().add_modifier(Modifier::BOLD)
    } else if focused {
        theme.text_bold()
    } else {
        theme.muted().add_modifier(Modifier::BOLD)
    };

    let block = Block::default()
        .title(Span::styled(format!(" {label} "), label_style))
        .borders(Borders::ALL)
        .border_type(theme.border_type(focused))
        .border_style(theme.border_style(focused));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let text_area = Rect::new(
        inner.x,
        inner.y + inner.height.saturating_sub(1),
        inner.width,
        inner.height.min(1),
    );

    let mut spans = vec![Span::raw(" ")];
    for (i, opt) in options.iter().enumerate() {
        let is_sel = i == selected;
        let bullet = if is_sel { "\u{25cf}" } else { "\u{25cb}" };
        let style = if disabled {
            theme.disabled()
        } else if is_sel {
            match colors {
                Some(c) if c[i] => theme.success_bold(),
                Some(_) => theme.danger_bold(),
                None if focused => theme.accent_bold(),
                None => theme.text_bold(),
            }
        } else {
            theme.muted()
        };
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(format!("{bullet} {opt}"), style));
    }

    f.render_widget(Paragraph::new(Line::from(spans)), text_area);
}

/// Builds the inline radio-option spans shared by [`render_radio_field`]
/// and the popup dialogs that show a fixed-choice group without their own
/// bordered box (net rule Action/Direction, mount Kind).
fn radio_group_spans(
    theme: &Theme,
    options: &[&str],
    selected: usize,
    focused: bool,
    colors: Option<&[bool]>,
) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (i, opt) in options.iter().enumerate() {
        let is_sel = i == selected;
        let bullet = if is_sel { "●" } else { "○" };
        let style = if is_sel {
            match colors {
                Some(c) if c[i] => theme.success_bold(),
                Some(_) => theme.danger_bold(),
                None if focused => theme.accent_bold(),
                None => theme.text_bold(),
            }
        } else {
            theme.muted()
        };
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(format!("{bullet} {opt}"), style));
    }
    spans
}


/// Render an inline, scrollable list field (Env Vars, Mounts, Ports, Net
/// Rules, Secrets). Entries are laid out column-major, wrapping into
/// additional columns once they exceed [`list_column_rows`] rows; the view
/// scrolls both vertically (rows) and horizontally (columns) to keep the
/// selected entry visible. When focused, `a`/`d` add/delete entries and
/// Up/Down/Left/Right move the selection (handled in `keys.rs` via
/// `focused_list()`).
fn render_list_field<T>(
    f: &mut Frame,
    theme: &Theme,
    label: &str,
    entries: &[T],
    selected: usize,
    focused: bool,
    edit_mode: bool,
    format_entry: impl Fn(&T) -> String,
    area: Rect,
    disabled: bool,
    list: ListField,
) {
    let label_style = if disabled {
        theme.disabled().add_modifier(Modifier::BOLD)
    } else if focused {
        theme.text_bold()
    } else {
        theme.muted().add_modifier(Modifier::BOLD)
    };

    let block = Block::default()
        .title(Span::styled(format!(" {label} "), label_style))
        .borders(Borders::ALL)
        .border_type(theme.border_type(focused))
        .border_style(theme.border_style(focused));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let avail_rows = inner.height.saturating_sub(1) as usize;
    let rows = list_column_rows(list).min(avail_rows.max(1)).max(1);
    let list_area = Rect::new(inner.x, inner.y, inner.width, rows as u16);
    let hint_area = Rect::new(
        inner.x,
        inner.y + rows as u16,
        inner.width,
        inner.height.saturating_sub(rows as u16),
    );

    let mut total_cols = 1usize;
    let mut visible_cols = 1usize;
    let mut scroll_col = 0usize;

    if entries.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(" (none)", theme.muted())),
            list_area,
        );
    } else {
        let formatted: Vec<String> = entries.iter().map(&format_entry).collect();
        let max_entry_w = formatted.iter().map(|s| s.chars().count()).max().unwrap_or(0);
        // " text" margin(1) + a little breathing room between columns(2).
        let col_width = (max_entry_w as u16 + 3).max(10).min(list_area.width.max(1));

        total_cols = ((entries.len() + rows - 1) / rows).max(1);
        visible_cols = ((list_area.width / col_width).max(1) as usize).min(total_cols);

        let sel_col = selected / rows;
        scroll_col = if sel_col >= visible_cols { sel_col + 1 - visible_cols } else { 0 };
        scroll_col = scroll_col.min(total_cols.saturating_sub(visible_cols));

        for col_offset in 0..visible_cols {
            let col = scroll_col + col_offset;
            if col >= total_cols {
                break;
            }
            let x = list_area.x + col_offset as u16 * col_width;
            let col_w = col_width.min(list_area.width.saturating_sub(col_offset as u16 * col_width));
            for row in 0..rows {
                let idx = col * rows + row;
                if idx >= entries.len() {
                    break;
                }
                let is_sel = focused && edit_mode && idx == selected;
                let style = if disabled {
                    theme.disabled()
                } else if is_sel {
                    theme.selected()
                } else {
                    theme.text()
                };
                let max_w = (col_w as usize).saturating_sub(2);
                let text = &formatted[idx];
                let display = if text.chars().count() > max_w {
                    let truncated: String = text.chars().take(max_w.saturating_sub(1)).collect();
                    format!(" {truncated}…")
                } else {
                    format!(" {text}")
                };
                let row_area = Rect::new(x, list_area.y + row as u16, col_w, 1);
                f.render_widget(Paragraph::new(Span::styled(display, style)), row_area);
            }
        }
    }

    let hint_style = if disabled {
        theme.disabled()
    } else if focused {
        theme.accent()
    } else {
        theme.muted()
    };
    let cols_hint = if total_cols > visible_cols {
        format!(
            " · ◄► cols {}-{}/{total_cols}",
            scroll_col + 1,
            (scroll_col + visible_cols).min(total_cols)
        )
    } else {
        String::new()
    };
    let hint_text = if disabled {
        String::new()
    } else if focused && edit_mode {
        format!(" ↑↓ select{cols_hint} · Enter edit · a add · d delete")
    } else if focused {
        " press Enter to edit".to_owned()
    } else {
        String::new()
    };
    f.render_widget(
        Paragraph::new(Span::styled(hint_text, hint_style)),
        hint_area,
    );
}

/// Render the Create Sandbox button, docked to the bottom-right corner of
/// the form below the horizontal separator, styled as a real bordered
/// button so it reads as clickable/actionable rather than as a plain field.
/// Width (columns) of the bordered Create Sandbox button, capped to fit
/// within `max_width`: borders(2) + padding(2) + the label text.
fn create_button_width(label: &str, max_width: u16) -> u16 {
    (label.chars().count() as u16 + 4).min(max_width)
}

/// Render the Create Sandbox button into the (already width-fitted) `area`.
/// Uses the same "+" glyph as the "+ New Sandbox" placeholder card in the
/// main sandbox list, for visual consistency between the two entry points.
fn render_create_button(f: &mut Frame, theme: &Theme, focused: bool, area: Rect) {
    let (border_style, label_style) = if focused {
        (theme.accent_bold(), theme.selected())
    } else {
        (theme.muted(), theme.muted())
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(theme.border_type(focused))
        .border_style(border_style)
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);

    f.render_widget(
        Paragraph::new(Line::from(Span::styled("+ Create Sandbox", label_style))),
        inner,
    );
}

/// Format one volume mount entry for the inline Mounts list.
fn format_mount_entry(mount: &VolumeMountConfig) -> String {
    match &mount.source {
        MountSource::Bind(host) => format!("{} <- bind {host}", mount.guest_path),
        MountSource::Named(name) => format!("{} <- volume {name}", mount.guest_path),
    }
}

/// Render a read-only field whose value is managed via a sub-dialog or picker.
/// `action_hint` is shown when focused, e.g. `"browse"`.
fn render_managed_field_with_hint(
    f: &mut Frame,
    theme: &Theme,
    label: &str,
    summary: &str,
    action_hint: &str,
    focused: bool,
    area: Rect,
) {
    let label_style = if focused {
        theme.text_bold()
    } else {
        theme.muted().add_modifier(Modifier::BOLD)
    };

    let block = Block::default()
        .title(Span::styled(format!(" {label} "), label_style))
        .borders(Borders::ALL)
        .border_type(theme.border_type(focused))
        .border_style(theme.border_style(focused));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let text_area = Rect::new(
        inner.x,
        inner.y + inner.height.saturating_sub(1),
        inner.width,
        inner.height.min(1),
    );

    let (text, style) = if focused {
        let indicator = format!("  [↵ {action_hint}]");
        let max_summary = (inner.width as usize).saturating_sub(indicator.len() + 1);
        let truncated = if summary.len() > max_summary {
            format!("{}…", &summary[..max_summary.saturating_sub(1)])
        } else {
            summary.to_owned()
        };
        (format!(" {truncated}{indicator}"), theme.accent())
    } else {
        (format!(" {summary}"), theme.secondary())
    };

    f.render_widget(Paragraph::new(Span::styled(text, style)), text_area);
}

/// Render the "Add Port Mapping" popup.
fn render_port_add_dialog(f: &mut Frame, app: &App, area: Rect) {
    let dialog = &app.create_dialog.port_add;
    let theme = &app.theme;
    if !dialog.visible {
        return;
    }

    // border(2) + spacer(1) + host field(3) + guest field(3) + spacer(1) +
    // hint(1) = 11
    let popup = centred_rect(52, 11, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(Span::styled(
            if dialog.editing_index.is_some() { " Edit Port Mapping " } else { " Add Port Mapping " },
            theme.accent_bold(),
        ))
        .borders(Borders::ALL)
        .border_type(theme.border_unfocused_type)
        .border_style(theme.accent())
        .padding(Padding::horizontal(1));

    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // top spacer
            Constraint::Length(3), // host port
            Constraint::Length(3), // guest port
            Constraint::Length(1), // spacer above hint/error
            Constraint::Length(1), // hint/error
        ])
        .split(inner);

    render_field(
        f,
        theme,
        "Host Port ",
        &dialog.host_input,
        dialog.add_field == 0,
        chunks[1],
        false,
        false,
    );
    render_field(
        f,
        theme,
        "Guest Port",
        &dialog.guest_input,
        dialog.add_field == 1,
        chunks[2],
        false,
        false,
    );

    if let Some(ref err) = dialog.error {
        f.render_widget(
            Paragraph::new(Span::styled(format!("✗ {err}"), theme.danger())),
            chunks[4],
        );
    } else {
        f.render_widget(
            Paragraph::new(theme.hint_line(&[
                ("Tab", "field"),
                ("Enter", "add"),
                ("Esc", "cancel"),
            ])),
            chunks[4],
        );
    }
}

/// Render the "Add Environment Variable" popup.
fn render_env_var_add_dialog(f: &mut Frame, app: &App, area: Rect) {
    let dialog = &app.create_dialog.env_var_add;
    let theme = &app.theme;
    if !dialog.visible {
        return;
    }

    // border(2) + spacer(1) + key field(3) + value field(3) + spacer(1) +
    // hint(1) = 11
    let popup = centred_rect(60, 11, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(Span::styled(
            if dialog.editing_index.is_some() {
                " Edit Environment Variable "
            } else {
                " Add Environment Variable "
            },
            theme.accent_bold(),
        ))
        .borders(Borders::ALL)
        .border_type(theme.border_unfocused_type)
        .border_style(theme.accent())
        .padding(Padding::horizontal(1));

    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // top spacer
            Constraint::Length(3), // key
            Constraint::Length(3), // value
            Constraint::Length(1), // spacer above hint/error
            Constraint::Length(1), // hint/error
        ])
        .split(inner);

    render_field(
        f,
        theme,
        "Key  ",
        &dialog.key_input,
        dialog.add_field == 0,
        chunks[1],
        false,
        false,
    );
    render_field(
        f,
        theme,
        "Value",
        &dialog.value_input,
        dialog.add_field == 1,
        chunks[2],
        false,
        false,
    );

    if let Some(ref err) = dialog.error {
        f.render_widget(
            Paragraph::new(Span::styled(format!("✗ {err}"), theme.danger())),
            chunks[4],
        );
    } else {
        f.render_widget(
            Paragraph::new(theme.hint_line(&[
                ("Tab", "field"),
                ("Enter", "add"),
                ("Esc", "cancel"),
            ])),
            chunks[4],
        );
    }
}

/// Render the directory picker overlay on top of the dialog.
fn render_dir_picker(f: &mut Frame, app: &App, area: Rect) {
    let picker = &app.create_dialog.dir_picker;
    let theme = &app.theme;
    // Height: 1 border + 1 path + 1 separator + PICKER_VISIBLE_ROWS entries + 1 hint + 1 border
    let height = (PICKER_VISIBLE_ROWS + 4) as u16;
    let popup = centred_rect(60, height, area);
    f.render_widget(Clear, popup);

    let title = if picker.showing_drives {
        " Select Drive "
    } else {
        " Select Directory "
    };
    let block = Block::default()
        .title(Span::styled(title, theme.accent_bold()))
        .borders(Borders::ALL)
        .border_type(theme.border_unfocused_type)
        .border_style(theme.accent())
        .padding(Padding::horizontal(1));

    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let rows = (PICKER_VISIBLE_ROWS as u16).min(inner.height.saturating_sub(3));
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // current path or header
            Constraint::Length(1), // separator
            Constraint::Length(rows),
            Constraint::Length(1), // hint
        ])
        .split(inner);

    // Header line
    let header = if picker.showing_drives {
        "🖴 Available Drives".to_owned()
    } else {
        format!("📂 {}", picker.path)
    };
    f.render_widget(
        Paragraph::new(Span::styled(header, theme.accent())),
        chunks[0],
    );

    // Separator
    f.render_widget(
        Paragraph::new(Span::styled(
            "─".repeat(inner.width as usize),
            theme.muted(),
        )),
        chunks[1],
    );

    // Entries
    let visible_end = (picker.scroll_offset + rows as usize).min(picker.entries.len());
    for (row, idx) in (picker.scroll_offset..visible_end).enumerate() {
        let entry = &picker.entries[idx];
        let prefix = if entry == ".." {
            "↑ "
        } else if entry == DRIVES_ENTRY {
            ""
        } else if picker.showing_drives {
            "🖴 "
        } else {
            "▸ "
        };
        let is_selected = idx == picker.selected;
        let style = if is_selected {
            theme.selected()
        } else if entry == DRIVES_ENTRY {
            theme.accent()
        } else {
            theme.text()
        };
        let entry_area = Rect::new(chunks[2].x, chunks[2].y + row as u16, chunks[2].width, 1);
        f.render_widget(
            Paragraph::new(Span::styled(format!("{prefix}{entry}"), style)),
            entry_area,
        );
    }

    // Hint
    let hint_pairs: &[(&str, &str)] = if picker.showing_drives {
        &[
            ("↑↓", "navigate"),
            ("Enter", "select drive"),
            ("/", "drives"),
            ("~", "home"),
            ("Esc", "cancel"),
        ]
    } else {
        &[
            ("↑↓", "navigate"),
            ("Enter", "descend"),
            ("Space", "select"),
            ("/", "drives"),
            ("~", "home"),
            ("Esc", "cancel"),
        ]
    };
    f.render_widget(Paragraph::new(theme.hint_line(hint_pairs)), chunks[3]);
}

/// Render the "Add Network Rule" popup, exposing the full expressiveness of
/// the SDK's `RuleBuilder`: action, direction, destination kind/value/
/// group, protocol filter, and an optional comma-separated list of
/// guest-side ports/ranges to apply the rule to.
fn render_net_rule_add_dialog(f: &mut Frame, app: &App, area: Rect) {
    let dialog = &app.create_dialog.net_rule_add;
    let theme = &app.theme;
    if !dialog.visible {
        return;
    }

    // border(2) + spacer(1) + action/direction(3) + destination(3) +
    // protocols(3) + apply-to-ports(3) + spacer(1) + hint(1) = 17
    let popup = centred_rect(70, 17, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(Span::styled(
            if dialog.editing_index.is_some() { " Edit Network Rule " } else { " Add Network Rule " },
            theme.accent_bold(),
        ))
        .borders(Borders::ALL)
        .border_type(theme.border_unfocused_type)
        .border_style(theme.accent())
        .padding(Padding::horizontal(1));

    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // top spacer
            Constraint::Length(3), // action / direction
            Constraint::Length(3), // destination kind + value/group
            Constraint::Length(3), // protocols
            Constraint::Length(3), // apply to ports
            Constraint::Length(1), // spacer above hint/error
            Constraint::Length(1), // hint/error
        ])
        .split(inner);

    let style_for = |focused: bool| if focused { theme.accent_bold() } else { theme.text() };

    // Action / Direction — bordered box, Action first.
    {
        let focused = dialog.add_field == 0 || dialog.add_field == 1;
        let label_style = if focused {
            theme.text_bold()
        } else {
            theme.muted().add_modifier(Modifier::BOLD)
        };
        let inner_box = Block::default()
            .title(Span::styled(" Action / Direction ", label_style))
            .borders(Borders::ALL)
            .border_type(theme.border_type(focused))
            .border_style(theme.border_style(focused));
        let box_inner = inner_box.inner(chunks[1]);
        f.render_widget(inner_box, chunks[1]);
        let text_area = Rect::new(
            box_inner.x,
            box_inner.y + box_inner.height.saturating_sub(1),
            box_inner.width,
            box_inner.height.min(1),
        );
        let mut spans = vec![Span::styled(" Action: ", theme.muted())];
        spans.extend(radio_group_spans(
            theme,
            &["Allow", "Deny"],
            if dialog.action == NetRuleAction::Allow { 0 } else { 1 },
            dialog.add_field == 0,
            Some(&[true, false]),
        ));
        spans.push(Span::raw("    "));
        spans.push(Span::styled("Direction: ", theme.muted()));
        spans.extend(radio_group_spans(
            theme,
            &["EGRESS", "INGRESS", "ANY"],
            match dialog.direction {
                NetRuleDirection::Egress => 0,
                NetRuleDirection::Ingress => 1,
                NetRuleDirection::Any => 2,
            },
            dialog.add_field == 1,
            None,
        ));
        f.render_widget(Paragraph::new(Line::from(spans)), text_area);
    }

    // Destination — kind + value/group in the same bordered box.
    {
        let focused = dialog.add_field == 2 || dialog.add_field == 3;
        let label_style = if focused {
            theme.text_bold()
        } else {
            theme.muted().add_modifier(Modifier::BOLD)
        };
        let inner_box = Block::default()
            .title(Span::styled(" Destination ", label_style))
            .borders(Borders::ALL)
            .border_type(theme.border_type(focused))
            .border_style(theme.border_style(focused));
        let box_inner = inner_box.inner(chunks[2]);
        f.render_widget(inner_box, chunks[2]);
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1)])
            .flex(ratatui::layout::Flex::End)
            .split(box_inner);
        let kind_and_value: Vec<Span> = {
            let mut spans = vec![
                Span::styled(" Kind: ", theme.muted()),
                Span::styled(dialog.dest_kind.label(), style_for(dialog.add_field == 2)),
            ];
            match dialog.dest_kind {
                NetRuleDestKind::Group => {
                    spans.push(Span::raw("    "));
                    spans.push(Span::styled("Group: ", theme.muted()));
                    spans.push(Span::styled(
                        dialog.dest_group.label(),
                        style_for(dialog.add_field == 3),
                    ));
                }
                NetRuleDestKind::Any => {
                    spans.push(Span::raw("    "));
                    spans.push(Span::styled("(matches any destination)", theme.muted()));
                }
                _ => {
                    spans.push(Span::raw("    "));
                    spans.push(Span::styled("Value: ", theme.muted()));
                    let display = if dialog.add_field == 3 {
                        format!("{}▌", dialog.dest_input)
                    } else {
                        dialog.dest_input.clone()
                    };
                    spans.push(Span::styled(display, style_for(dialog.add_field == 3)));
                }
            }
            spans
        };
        f.render_widget(Paragraph::new(Line::from(kind_and_value)), rows[0]);
    }

    // Protocols — bordered box.
    {
        let focused = dialog.add_field == 4;
        let label_style = if focused {
            theme.text_bold()
        } else {
            theme.muted().add_modifier(Modifier::BOLD)
        };
        let inner_box = Block::default()
            .title(Span::styled(" Protocols ", label_style))
            .borders(Borders::ALL)
            .border_type(theme.border_type(focused))
            .border_style(theme.border_style(focused));
        let box_inner = inner_box.inner(chunks[3]);
        f.render_widget(inner_box, chunks[3]);
        let text_area = Rect::new(
            box_inner.x,
            box_inner.y + box_inner.height.saturating_sub(1),
            box_inner.width,
            box_inner.height.min(1),
        );
        let proto_spans: Vec<Span> = crate::sandbox::NetRuleProtocol::ALL
            .iter()
            .enumerate()
            .flat_map(|(i, proto)| {
                let checked = dialog.protocols.contains(proto);
                let cursor_here = focused && dialog.protocol_cursor == i;
                let box_style = if cursor_here {
                    theme.accent_bold()
                } else if checked {
                    theme.text()
                } else {
                    theme.muted()
                };
                let mark = if checked { "[x]" } else { "[ ]" };
                vec![
                    Span::styled(format!(" {mark} {} ", proto.label()), box_style),
                    Span::raw(" "),
                ]
            })
            .collect();
        f.render_widget(Paragraph::new(Line::from(proto_spans)), text_area);
    }

    // Apply to Ports — bordered box with an inline format hint.
    {
        let focused = dialog.add_field == 5;
        let label_style = if focused {
            theme.text_bold()
        } else {
            theme.muted().add_modifier(Modifier::BOLD)
        };
        let inner_box = Block::default()
            .title(Span::styled(" Apply to Ports ", label_style))
            .borders(Borders::ALL)
            .border_type(theme.border_type(focused))
            .border_style(theme.border_style(focused));
        let box_inner = inner_box.inner(chunks[4]);
        f.render_widget(inner_box, chunks[4]);
        let display = if focused {
            format!(" {}▌", dialog.ports_input)
        } else if dialog.ports_input.is_empty() {
            " (any port)".to_owned()
        } else {
            format!(" {}", dialog.ports_input)
        };
        let value_style = if focused { theme.text() } else { theme.secondary() };
        let value_area = Rect::new(box_inner.x, box_inner.y, box_inner.width, 1);
        f.render_widget(Paragraph::new(Span::styled(display, value_style)), value_area);
        if box_inner.height > 1 {
            let hint_area = Rect::new(box_inner.x, box_inner.y + 1, box_inner.width, 1);
            f.render_widget(
                Paragraph::new(Span::styled(
                    " e.g. 80,443,8000-9000 (comma-separated ports/ranges)",
                    theme.muted(),
                )),
                hint_area,
            );
        }
    }

    if let Some(ref err) = dialog.error {
        f.render_widget(
            Paragraph::new(Span::styled(format!("✗ {err}"), theme.danger())),
            chunks[6],
        );
    } else {
        f.render_widget(
            Paragraph::new(theme.hint_line(&[
                ("Tab", "field"),
                ("◄►/Space", "cycle/toggle"),
                ("Enter", "next/add"),
                ("Esc", "cancel"),
            ])),
            chunks[6],
        );
    }
}

/// Render the "Add Volume Mount" popup.
fn render_mount_add_dialog(f: &mut Frame, app: &App, area: Rect) {
    let dialog = &app.create_dialog.mount_add;
    let theme = &app.theme;
    if !dialog.visible {
        return;
    }

    let popup = centred_rect(70, if dialog.new_volume_mode { 18 } else { 16 }, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(Span::styled(
            if dialog.editing_index.is_some() { " Edit Volume Mount " } else { " Add Volume Mount " },
            theme.accent_bold(),
        ))
        .borders(Borders::ALL)
        .border_type(theme.border_unfocused_type)
        .border_style(theme.accent())
        .padding(Padding::horizontal(1));

    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // top spacer
            Constraint::Length(3), // kind
            Constraint::Length(3), // guest path
            Constraint::Length(5), // host path / volume list
            Constraint::Length(if dialog.new_volume_mode { 3 } else { 1 }),
            Constraint::Length(1), // hint/error
        ])
        .split(inner);

    let kind_block = Block::default()
        .title(Span::styled(
            " Kind ",
            if dialog.add_field == 0 {
                theme.text_bold()
            } else {
                theme.muted().add_modifier(Modifier::BOLD)
            },
        ))
        .borders(Borders::ALL)
        .border_type(theme.border_type(dialog.add_field == 0))
        .border_style(theme.border_style(dialog.add_field == 0));
    let kind_inner = kind_block.inner(chunks[1]);
    f.render_widget(kind_block, chunks[1]);
    let mut kind_spans = vec![Span::raw(" ")];
    kind_spans.extend(radio_group_spans(
        theme,
        &["Bind", "Named"],
        if dialog.kind == MountKindChoice::Bind { 0 } else { 1 },
        dialog.add_field == 0,
        None,
    ));
    f.render_widget(Paragraph::new(Line::from(kind_spans)), kind_inner);

    render_field(
        f,
        theme,
        "Guest",
        &dialog.guest_input,
        dialog.add_field == 1,
        chunks[2],
        false,
        false,
    );
    match dialog.kind {
        MountKindChoice::Bind => {
            render_managed_field_with_hint(
                f,
                theme,
                "Host Path",
                if dialog.source_input.is_empty() {
                    "(none)"
                } else {
                    &dialog.source_input
                },
                "browse",
                dialog.add_field == 2,
                chunks[3],
            );
        }
        MountKindChoice::Named => {
            let block = Block::default()
                .title(Span::styled(
                    " Volume ",
                    if dialog.add_field == 2 {
                        theme.text_bold()
                    } else {
                        theme.muted().add_modifier(Modifier::BOLD)
                    },
                ))
                .borders(Borders::ALL)
                .border_type(theme.border_type(dialog.add_field == 2))
                .border_style(theme.border_style(dialog.add_field == 2));
            let inner = block.inner(chunks[3]);
            f.render_widget(block, chunks[3]);
            if dialog.available_volumes.is_empty() {
                f.render_widget(
                    Paragraph::new(Span::styled(
                        " (no volumes yet — Ctrl-N to create)",
                        theme.muted(),
                    )),
                    inner,
                );
            } else {
                for (i, vol) in dialog.available_volumes.iter().take(inner.height as usize).enumerate() {
                    let style = if dialog.add_field == 2 && i == dialog.selected_volume {
                        theme.selected()
                    } else {
                        theme.text()
                    };
                    let row = Rect::new(inner.x, inner.y + i as u16, inner.width, 1);
                    f.render_widget(
                        Paragraph::new(Span::styled(format!(" {}", vol.name), style)),
                        row,
                    );
                }
            }
        }
    }

    if dialog.new_volume_mode {
        let kind = if dialog.new_volume_disk { "Disk" } else { "Directory" };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("New volume: ", theme.accent_bold()),
                Span::styled(dialog.new_volume_name.as_str(), theme.text()),
                Span::styled(format!("  Kind: {kind} (Space toggles)"), theme.muted()),
            ])),
            chunks[4],
        );
    }

    if let Some(ref err) = dialog.error {
        f.render_widget(
            Paragraph::new(Span::styled(format!("✗ {err}"), theme.danger())),
            chunks[5],
        );
    } else {
        f.render_widget(
            Paragraph::new(theme.hint_line(&[
                ("Tab", "field"),
                ("Space", "kind"),
                ("Ctrl-F", "browse bind"),
                ("Ctrl-N", "new volume"),
                ("Enter", "add"),
                ("Esc", "cancel"),
            ])),
            chunks[5],
        );
    }
}

/// Render the "Add Secret" popup, mirroring the SDK's `SecretBuilder`.
fn render_secret_add_dialog(f: &mut Frame, app: &App, area: Rect) {
    let dialog = &app.create_dialog.secret_add;
    let theme = &app.theme;
    if !dialog.visible {
        return;
    }

    // border(2) + env(3) + value(3) + hosts(3) + toggles(2) + hint(1) = 14
    let popup = centred_rect(65, 14, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(Span::styled(
            if dialog.editing_index.is_some() { " Edit Secret " } else { " Add Secret " },
            theme.accent_bold(),
        ))
        .borders(Borders::ALL)
        .border_type(theme.border_unfocused_type)
        .border_style(theme.accent())
        .padding(Padding::horizontal(1));

    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // env
            Constraint::Length(3), // value
            Constraint::Length(3), // hosts
            Constraint::Length(1), // toggle row 1: headers / basic auth
            Constraint::Length(1), // toggle row 2: query / body / tls identity
            Constraint::Length(1), // hint/error
        ])
        .split(inner);

    render_field(
        f,
        theme,
        "Env Var",
        &dialog.env_input,
        dialog.add_field == 0,
        chunks[0],
        false,
        false,
    );
    let masked_value: String = "*".repeat(dialog.value_input.chars().count());
    render_field(
        f,
        theme,
        "Value  ",
        &masked_value,
        dialog.add_field == 1,
        chunks[1],
        false,
        false,
    );
    render_field(
        f,
        theme,
        "Hosts  ",
        &dialog.hosts_input,
        dialog.add_field == 2,
        chunks[2],
        false,
        false,
    );

    let toggle_span = |on: bool, label: &str, focused: bool| {
        let style = if focused {
            theme.accent_bold()
        } else if on {
            theme.text()
        } else {
            theme.muted()
        };
        let mark = if on { "[x]" } else { "[ ]" };
        Span::styled(format!("{mark} {label}  "), style)
    };

    f.render_widget(
        Paragraph::new(Line::from(vec![
            toggle_span(dialog.inject_headers, "Headers", dialog.add_field == 3),
            toggle_span(
                dialog.inject_basic_auth,
                "Basic Auth",
                dialog.add_field == 4,
            ),
        ])),
        chunks[3],
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![
            toggle_span(dialog.inject_query, "Query", dialog.add_field == 5),
            toggle_span(dialog.inject_body, "Body", dialog.add_field == 6),
            toggle_span(
                dialog.require_tls_identity,
                "Require TLS Identity",
                dialog.add_field == 7,
            ),
        ])),
        chunks[4],
    );

    if let Some(ref err) = dialog.error {
        f.render_widget(
            Paragraph::new(Span::styled(format!("✗ {err}"), theme.danger())),
            chunks[5],
        );
    } else {
        f.render_widget(
            Paragraph::new(theme.hint_line(&[
                ("Tab", "field"),
                ("Space", "toggle"),
                ("Enter", "next/add"),
                ("Esc", "cancel"),
            ])),
            chunks[5],
        );
    }
}

/// Compute a centred rect with the given percentage width and fixed height,
/// relative to `area`.
fn centred_rect(percent_width: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(height),
            Constraint::Fill(1),
        ])
        .split(area);

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_width) / 2),
            Constraint::Percentage(percent_width),
            Constraint::Percentage((100 - percent_width) / 2),
        ])
        .split(vertical[1]);

    horizontal[1]
}
