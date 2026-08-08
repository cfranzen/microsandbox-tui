//! "New Sandbox" creation modal dialog.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Padding, Paragraph},
    Frame,
};

use crate::app::{App, DialogTab, MountKindChoice, DRIVES_ENTRY, PICKER_VISIBLE_ROWS};
use crate::sandbox::{MountSource, NetRuleDestKind, NetworkRule, SecretConfig, VolumeMountConfig};
use crate::theme::Theme;

/// Number of entries shown at once inside an inline scrollable list field
/// (Env Vars, Mounts, Ports, Net Rules, Secrets).
const VISIBLE_LIST_ROWS: u16 = 4;
/// Height of a plain single-line bordered text/toggle field.
const FIELD_HEIGHT: u16 = 3;
/// Height of a bordered inline list field: border(2) + entries + hint(1).
const LIST_HEIGHT: u16 = VISIBLE_LIST_ROWS + 3;

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
    let row_heights: &[u16] = match dlg.tab {
        DialogTab::Basic => &[
            FIELD_HEIGHT, // name
            FIELD_HEIGHT, // image
            FIELD_HEIGHT, // cpus / max cpus
            FIELD_HEIGHT, // memory / max memory
            FIELD_HEIGHT, // workdir
        ],
        DialogTab::GuestOs => &[
            FIELD_HEIGHT, // hostname
            FIELD_HEIGHT, // user
            FIELD_HEIGHT, // shell
            LIST_HEIGHT,  // env vars
            LIST_HEIGHT,  // mounts
        ],
        DialogTab::Network => &[
            FIELD_HEIGHT, // no net
            LIST_HEIGHT,  // ports
            LIST_HEIGHT,  // network rules
        ],
        DialogTab::Security => &[
            LIST_HEIGHT, // secrets
        ],
    };
    let content_height: u16 = row_heights.iter().sum();
    // borders(2) + tab bar(1) + spacer(1) + content + create button(1) + hint/error(1)
    let popup_height = 6 + content_height;

    let popup = centred_rect(70, popup_height, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(Span::styled(" New Sandbox ", theme.accent_bold()))
        .borders(Borders::ALL)
        .border_type(theme.border_unfocused_type)
        .border_style(theme.accent())
        .style(theme.base_style())
        .padding(Padding::horizontal(1));

    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let mut constraints = vec![Constraint::Length(1), Constraint::Length(1)];
    constraints.extend(row_heights.iter().map(|h| Constraint::Length(*h)));
    constraints.push(Constraint::Length(1)); // Create button
    constraints.push(Constraint::Length(1)); // hint / error

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    render_tab_bar(f, theme, dlg.tab, chunks[0]);

    match dlg.tab {
        DialogTab::Basic => {
            let workdir_summary = if dlg.workdir.is_empty() {
                "(none)".into()
            } else {
                dlg.workdir.clone()
            };
            render_field(f, theme, "Name    ", &dlg.name, dlg.field == 0, chunks[2]);
            render_field(f, theme, "Image   ", &dlg.image, dlg.field == 1, chunks[3]);
            render_field_pair(
                f,
                theme,
                "CPUs    ",
                &dlg.cpus,
                dlg.field == 2,
                "Max CPUs",
                &dlg.max_cpus,
                dlg.field == 3,
                chunks[4],
            );
            render_field_pair(
                f,
                theme,
                "Memory  ",
                &dlg.memory,
                dlg.field == 4,
                "Max Mem ",
                &dlg.max_memory,
                dlg.field == 5,
                chunks[5],
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
            render_field(
                f,
                theme,
                "Hostname",
                &dlg.hostname,
                dlg.field == 0,
                chunks[2],
            );
            render_field(f, theme, "User    ", &dlg.user, dlg.field == 1, chunks[3]);
            render_field(f, theme, "Shell   ", &dlg.shell, dlg.field == 2, chunks[4]);
            render_list_field(
                f,
                theme,
                "Env Vars",
                &dlg.env_vars,
                dlg.env_vars_selected,
                dlg.field == 3,
                |(k, v)| format!("{k}={v}"),
                chunks[5],
            );
            render_list_field(
                f,
                theme,
                "Mounts  ",
                &dlg.mounts,
                dlg.mounts_selected,
                dlg.field == 4,
                format_mount_entry,
                chunks[6],
            );
        }
        DialogTab::Network => {
            render_toggle(
                f,
                theme,
                "No Net  ",
                dlg.disable_network,
                dlg.field == 0,
                chunks[2],
            );
            render_list_field(
                f,
                theme,
                "Ports   ",
                &dlg.ports,
                dlg.ports_selected,
                dlg.field == 1,
                |(h, g)| format!("{h} → {g}"),
                chunks[3],
            );
            render_list_field(
                f,
                theme,
                "Net Rules",
                &dlg.network_rules,
                dlg.network_rules_selected,
                dlg.field == 2,
                NetworkRule::summary,
                chunks[4],
            );
        }
        DialogTab::Security => {
            render_list_field(
                f,
                theme,
                "Secrets ",
                &dlg.secrets,
                dlg.secrets_selected,
                dlg.field == 0,
                SecretConfig::summary,
                chunks[2],
            );
        }
    }

    // Create button — always the second-to-last chunk.
    let create_chunk = chunks[chunks.len() - 2];
    render_create_button(f, theme, dlg.is_create_focused(), create_chunk);

    // Hint / error — always the last chunk.
    let message_chunk = chunks[chunks.len() - 1];
    let hint_pairs: &[(&str, &str)] = if dlg.is_create_focused() {
        &[("Enter", "create sandbox"), ("Esc", "cancel")]
    } else if dlg.focused_list().is_some() {
        &[
            ("Tab/◄►", "navigate"),
            ("↑↓", "select"),
            ("a", "add"),
            ("d", "delete"),
            ("Esc", "cancel"),
        ]
    } else {
        match (dlg.tab, dlg.field) {
            (DialogTab::Basic, 6) => &[
                ("Tab/↑↓", "navigate"),
                ("◄►", "tab"),
                ("Enter", "browse"),
                ("Esc", "cancel"),
            ],
            (DialogTab::Network, 0) => &[
                ("Tab/↑↓", "navigate"),
                ("◄►", "tab"),
                ("Space", "toggle"),
                ("Esc", "cancel"),
            ],
            _ => &[("Tab/↑↓", "navigate"), ("◄►", "tab"), ("Esc", "cancel")],
        }
    };
    if let Some(ref err) = dlg.error {
        f.render_widget(
            Paragraph::new(Span::styled(format!("✗ {err}"), theme.danger())),
            message_chunk,
        );
    } else {
        f.render_widget(Paragraph::new(theme.hint_line(hint_pairs)), message_chunk);
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

fn render_tab_bar(f: &mut Frame, theme: &Theme, tab: DialogTab, area: Rect) {
    let labels = [
        (DialogTab::Basic.title(), tab == DialogTab::Basic),
        (DialogTab::GuestOs.title(), tab == DialogTab::GuestOs),
        (DialogTab::Network.title(), tab == DialogTab::Network),
        (DialogTab::Security.title(), tab == DialogTab::Security),
    ];
    let (spans, _) = theme.tab_bar(&labels);

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_field(f: &mut Frame, theme: &Theme, label: &str, value: &str, focused: bool, area: Rect) {
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

    let display = if focused {
        format!(" {value}▌")
    } else {
        format!(" {value}")
    };

    let value_style = if focused {
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
) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);
    render_field(f, theme, label_a, value_a, focused_a, cols[0]);
    render_field(f, theme, label_b, value_b, focused_b, cols[1]);
}

fn render_toggle(
    f: &mut Frame,
    theme: &Theme,
    label: &str,
    value: bool,
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
    let (text, style) = if value {
        (" ● On", theme.success())
    } else {
        (" ○ Off", theme.muted())
    };

    f.render_widget(Paragraph::new(Span::styled(text, style)), text_area);
}

/// Render an inline, scrollable list field (Env Vars, Mounts, Ports, Net
/// Rules, Secrets). The list scrolls automatically to keep the selected
/// entry visible; when focused, `a`/`d` add/delete entries and Up/Down move
/// the selection (handled in `keys.rs` via `focused_list()`).
fn render_list_field<T>(
    f: &mut Frame,
    theme: &Theme,
    label: &str,
    entries: &[T],
    selected: usize,
    focused: bool,
    format_entry: impl Fn(&T) -> String,
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

    let rows = inner.height.saturating_sub(1);
    let list_area = Rect::new(inner.x, inner.y, inner.width, rows);
    let hint_area = Rect::new(inner.x, inner.y + rows, inner.width, 1);

    if entries.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(" (none)", theme.muted())),
            list_area,
        );
    } else {
        let visible = rows as usize;
        let scroll = if visible > 0 && selected >= visible {
            selected + 1 - visible
        } else {
            0
        };
        let end = (scroll + visible).min(entries.len());
        for (row, idx) in (scroll..end).enumerate() {
            let is_sel = focused && idx == selected;
            let style = if is_sel {
                theme.selected()
            } else {
                theme.text()
            };
            let max_w = (list_area.width as usize).saturating_sub(2);
            let text = format_entry(&entries[idx]);
            let display = if text.chars().count() > max_w {
                let truncated: String = text.chars().take(max_w.saturating_sub(1)).collect();
                format!(" {truncated}…")
            } else {
                format!(" {text}")
            };
            let row_area = Rect::new(list_area.x, list_area.y + row as u16, list_area.width, 1);
            f.render_widget(Paragraph::new(Span::styled(display, style)), row_area);
        }
    }

    let hint_style = if focused {
        theme.accent()
    } else {
        theme.muted()
    };
    let hint_text = if focused {
        " ↑↓ select · a add · d delete"
    } else if entries.is_empty() {
        " press Enter/a to add"
    } else {
        ""
    };
    f.render_widget(
        Paragraph::new(Span::styled(hint_text, hint_style)),
        hint_area,
    );
}

/// Render the Create Sandbox button at the bottom of the form.
fn render_create_button(f: &mut Frame, theme: &Theme, focused: bool, area: Rect) {
    // Use a multi-span Line so we can mix styles within a single row.
    // Focused:   ──────────────────── ✚ Create Sandbox ▶
    // Unfocused: ──────────────────── ✚ Create Sandbox ▶  (dimmed)
    let label = " ✚ Create Sandbox ";

    let (fill_style, label_style, arrow_style) = if focused {
        (theme.muted(), theme.selected(), theme.accent_bold())
    } else {
        (theme.muted(), theme.muted(), theme.muted())
    };

    // Arrow glyph that bookends the label, Powerline-style.
    let arrow = "▶";
    let label_width = label.chars().count() as u16 + arrow.chars().count() as u16;
    let fill_width = area.width.saturating_sub(label_width);

    let line = Line::from(vec![
        Span::styled("─".repeat(fill_width as usize), fill_style),
        Span::styled(label, label_style),
        Span::styled(arrow, arrow_style),
    ]);

    f.render_widget(Paragraph::new(line), area);
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

    // border(2) + spacer(1) + host field(3) + guest field(3) + hint(1) = 10
    let popup = centred_rect(52, 10, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(Span::styled(" Add Port Mapping ", theme.accent_bold()))
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
    );
    render_field(
        f,
        theme,
        "Guest Port",
        &dialog.guest_input,
        dialog.add_field == 1,
        chunks[2],
    );

    if let Some(ref err) = dialog.error {
        f.render_widget(
            Paragraph::new(Span::styled(format!("✗ {err}"), theme.danger())),
            chunks[3],
        );
    } else {
        f.render_widget(
            Paragraph::new(theme.hint_line(&[
                ("Tab", "field"),
                ("Enter", "add"),
                ("Esc", "cancel"),
            ])),
            chunks[3],
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

    // border(2) + spacer(1) + key field(3) + value field(3) + hint(1) = 10
    let popup = centred_rect(60, 10, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(Span::styled(
            " Add Environment Variable ",
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
    );
    render_field(
        f,
        theme,
        "Value",
        &dialog.value_input,
        dialog.add_field == 1,
        chunks[2],
    );

    if let Some(ref err) = dialog.error {
        f.render_widget(
            Paragraph::new(Span::styled(format!("✗ {err}"), theme.danger())),
            chunks[3],
        );
    } else {
        f.render_widget(
            Paragraph::new(theme.hint_line(&[
                ("Tab", "field"),
                ("Enter", "add"),
                ("Esc", "cancel"),
            ])),
            chunks[3],
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
/// the SDK's `RuleBuilder`: direction, action, destination kind/value/
/// group, protocol filter, and an optional guest-side port or port range.
fn render_net_rule_add_dialog(f: &mut Frame, app: &App, area: Rect) {
    let dialog = &app.create_dialog.net_rule_add;
    let theme = &app.theme;
    if !dialog.visible {
        return;
    }

    // border(2) + direction/action(1) + dest kind(1) + dest value(3) +
    // protocols(1) + ports(3) + hint(1) = 12
    let popup = centred_rect(70, 12, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(Span::styled(" Add Network Rule ", theme.accent_bold()))
        .borders(Borders::ALL)
        .border_type(theme.border_unfocused_type)
        .border_style(theme.accent())
        .padding(Padding::horizontal(1));

    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // direction / action
            Constraint::Length(1), // dest kind
            Constraint::Length(3), // dest value / group
            Constraint::Length(1), // protocols
            Constraint::Length(3), // ports
            Constraint::Length(1), // hint/error
        ])
        .split(inner);

    let style_for = |focused: bool| if focused { theme.accent_bold() } else { theme.text() };

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Direction: ", theme.muted()),
            Span::styled(dialog.direction.label(), style_for(dialog.add_field == 0)),
            Span::raw("    "),
            Span::styled("Action: ", theme.muted()),
            Span::styled(dialog.action.label(), style_for(dialog.add_field == 1)),
        ])),
        chunks[0],
    );

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Destination Kind: ", theme.muted()),
            Span::styled(dialog.dest_kind.label(), style_for(dialog.add_field == 2)),
        ])),
        chunks[1],
    );

    if dialog.dest_kind == NetRuleDestKind::Group {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Group: ", theme.muted()),
                Span::styled(dialog.dest_group.label(), style_for(dialog.add_field == 3)),
            ])),
            chunks[2],
        );
    } else if dialog.dest_kind == NetRuleDestKind::Any {
        f.render_widget(
            Paragraph::new(Span::styled(
                "(matches any destination)",
                theme.muted(),
            )),
            chunks[2],
        );
    } else {
        render_field(
            f,
            theme,
            "Value",
            &dialog.dest_input,
            dialog.add_field == 3,
            chunks[2],
        );
    }

    let proto_spans: Vec<Span> = crate::sandbox::NetRuleProtocol::ALL
        .iter()
        .enumerate()
        .flat_map(|(i, proto)| {
            let checked = dialog.protocols.contains(proto);
            let cursor_here = dialog.add_field == 4 && dialog.protocol_cursor == i;
            let box_style = if cursor_here {
                theme.accent_bold()
            } else if checked {
                theme.text()
            } else {
                theme.muted()
            };
            let mark = if checked { "[x]" } else { "[ ]" };
            vec![
                Span::styled(format!("{mark} {} ", proto.label()), box_style),
                Span::raw(" "),
            ]
        })
        .collect();
    f.render_widget(Paragraph::new(Line::from(proto_spans)), chunks[3]);

    render_field(
        f,
        theme,
        "Ports",
        &dialog.ports_input,
        dialog.add_field == 5,
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
                ("◄►/Space", "cycle/toggle"),
                ("Enter", "next/add"),
                ("Esc", "cancel"),
            ])),
            chunks[5],
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

    // border(2) + guest field(3) + source field(3) + kind line(1) + hint(1) = 10
    let popup = centred_rect(65, 10, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(Span::styled(" Add Volume Mount ", theme.accent_bold()))
        .borders(Borders::ALL)
        .border_type(theme.border_unfocused_type)
        .border_style(theme.accent())
        .padding(Padding::horizontal(1));

    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // guest path
            Constraint::Length(3), // host path / volume name
            Constraint::Length(1), // kind summary
            Constraint::Length(1), // hint/error
        ])
        .split(inner);

    render_field(
        f,
        theme,
        "Guest",
        &dialog.guest_input,
        dialog.add_field == 0,
        chunks[0],
    );
    let source_label = match dialog.kind {
        MountKindChoice::Bind => "Host ",
        MountKindChoice::Named => "Vol  ",
    };
    render_field(
        f,
        theme,
        source_label,
        &dialog.source_input,
        dialog.add_field == 1,
        chunks[1],
    );

    let kind_label = match dialog.kind {
        MountKindChoice::Bind => "Bind mount (b)",
        MountKindChoice::Named => "Named volume (n)",
    };
    f.render_widget(
        Paragraph::new(Span::styled(format!("Kind: {kind_label}"), theme.accent())),
        chunks[2],
    );

    if let Some(ref err) = dialog.error {
        f.render_widget(
            Paragraph::new(Span::styled(format!("✗ {err}"), theme.danger())),
            chunks[3],
        );
    } else {
        f.render_widget(
            Paragraph::new(theme.hint_line(&[
                ("Tab", "field"),
                ("b/n", "kind"),
                ("Enter", "add"),
                ("Esc", "cancel"),
            ])),
            chunks[3],
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
        .title(Span::styled(" Add Secret ", theme.accent_bold()))
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
    );
    let masked_value: String = "*".repeat(dialog.value_input.chars().count());
    render_field(
        f,
        theme,
        "Value  ",
        &masked_value,
        dialog.add_field == 1,
        chunks[1],
    );
    render_field(
        f,
        theme,
        "Hosts  ",
        &dialog.hosts_input,
        dialog.add_field == 2,
        chunks[2],
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
