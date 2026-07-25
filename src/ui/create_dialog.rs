//! "New Sandbox" creation modal dialog.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::app::{App, DialogTab, SubDialogMode, DRIVES_ENTRY, PICKER_VISIBLE_ROWS};

/// Render the create-sandbox dialog centred over the full terminal area.
pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let dlg = &app.create_dialog;
    if !dlg.visible {
        return;
    }

    let popup = centred_rect(65, 27, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(Span::styled(
            " New Sandbox ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let mut constraints = vec![Constraint::Length(1), Constraint::Length(1)];
    constraints.extend(std::iter::repeat(Constraint::Length(3)).take(dlg.field_count()));
    constraints.push(Constraint::Length(1));
    constraints.push(Constraint::Length(1));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    render_tab_bar(f, dlg.tab, chunks[0]);

    match dlg.tab {
        DialogTab::Basic => {
            let ports_summary = format_ports_summary(&dlg.ports);
            let env_summary = format_env_vars_summary(&dlg.env_vars);
            render_field(f, "Name    ", &dlg.name, dlg.field == 0, chunks[2]);
            render_field(f, "Image   ", &dlg.image, dlg.field == 1, chunks[3]);
            render_field(f, "CPUs    ", &dlg.cpus, dlg.field == 2, chunks[4]);
            render_field(f, "Memory  ", &dlg.memory, dlg.field == 3, chunks[5]);
            render_managed_field(f, "Ports   ", &ports_summary, dlg.field == 4, chunks[6]);
            render_managed_field(f, "Env Vars", &env_summary, dlg.field == 5, chunks[7]);
            render_field(f, "Workdir ", &dlg.workdir, dlg.field == 6, chunks[8]);
        }
        DialogTab::Advanced => {
            render_field(f, "Hostname", &dlg.hostname, dlg.field == 0, chunks[2]);
            render_field(f, "User    ", &dlg.user, dlg.field == 1, chunks[3]);
            render_field(f, "Shell   ", &dlg.shell, dlg.field == 2, chunks[4]);
            render_field(f, "Max CPUs", &dlg.max_cpus, dlg.field == 3, chunks[5]);
            render_field(f, "Max Mem ", &dlg.max_memory, dlg.field == 4, chunks[6]);
            render_toggle(f, "No Net  ", dlg.disable_network, dlg.field == 5, chunks[7]);
        }
    }

    let message_chunk = chunks[2 + dlg.field_count() + 1];
    let hint = match (dlg.tab, dlg.field) {
        (DialogTab::Basic, 6) => {
            "Tab/↑↓ field   ◄► tab   Ctrl+F browse   Enter create   Esc cancel"
        }
        (DialogTab::Basic, 4) | (DialogTab::Basic, 5) => {
            "Tab/↑↓ field   ◄► tab   Enter manage   Esc cancel"
        }
        (DialogTab::Advanced, 5) => {
            "Tab/↑↓ field   ◄► tab   Space toggle   Enter create   Esc cancel"
        }
        _ => "Tab/↑↓ field   ◄► tab   Enter create   Esc cancel",
    };
    if let Some(ref err) = dlg.error {
        f.render_widget(
            Paragraph::new(Span::styled(
                format!("  ✗ {err}"),
                Style::default().fg(Color::Red),
            )),
            message_chunk,
        );
    } else {
        f.render_widget(Paragraph::new(hint), message_chunk);
    }

    if dlg.dir_picker.visible {
        render_dir_picker(f, app, area);
    }
    if dlg.ports_dialog.visible {
        render_ports_dialog(f, app, area);
    }
    if dlg.env_vars_dialog.visible {
        render_env_vars_dialog(f, app, area);
    }
}

fn render_tab_bar(f: &mut Frame, tab: DialogTab, area: Rect) {
    let basic = if tab == DialogTab::Basic {
        Span::styled("[Basic]", Style::default().fg(Color::Black).bg(Color::Cyan))
    } else {
        Span::styled("[Basic]", Style::default().fg(Color::Gray))
    };
    let advanced = if tab == DialogTab::Advanced {
        Span::styled(
            "[Advanced]",
            Style::default().fg(Color::Black).bg(Color::Cyan),
        )
    } else {
        Span::styled("[Advanced]", Style::default().fg(Color::Gray))
    };

    f.render_widget(
        Paragraph::new(Line::from(vec![basic, Span::raw(" "), advanced])),
        area,
    );
}

fn render_field(f: &mut Frame, label: &str, value: &str, focused: bool, area: Rect) {
    let border_color = if focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };
    let label_color = if focused {
        Color::White
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .title(Span::styled(
            format!(" {label} "),
            Style::default()
                .fg(label_color)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(if focused {
            BorderType::Thick
        } else {
            BorderType::Rounded
        })
        .border_style(Style::default().fg(border_color));

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

    f.render_widget(
        Paragraph::new(Span::styled(
            display,
            Style::default().fg(if focused { Color::White } else { Color::Gray }),
        )),
        text_area,
    );
}

fn render_toggle(f: &mut Frame, label: &str, value: bool, focused: bool, area: Rect) {
    let border_color = if focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };
    let label_color = if focused {
        Color::White
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .title(Span::styled(
            format!(" {label} "),
            Style::default()
                .fg(label_color)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(if focused {
            BorderType::Thick
        } else {
            BorderType::Rounded
        })
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let text_area = Rect::new(
        inner.x,
        inner.y + inner.height.saturating_sub(1),
        inner.width,
        inner.height.min(1),
    );
    let (text, color) = if value {
        (" ● On", Color::Green)
    } else {
        (" ○ Off", Color::DarkGray)
    };

    f.render_widget(
        Paragraph::new(Span::styled(text, Style::default().fg(color))),
        text_area,
    );
}

/// Format a list of port mappings as a compact summary string.
fn format_ports_summary(ports: &[(u16, u16)]) -> String {
    if ports.is_empty() {
        "(none)".into()
    } else {
        ports.iter().map(|(h, g)| format!("{h}:{g}")).collect::<Vec<_>>().join(", ")
    }
}

/// Format a list of env vars as a compact summary string.
fn format_env_vars_summary(vars: &[(String, String)]) -> String {
    if vars.is_empty() {
        "(none)".into()
    } else {
        vars.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(", ")
    }
}

/// Render a read-only field whose value is managed via a sub-dialog.
/// Shows a summary of current entries and a `[↵ manage]` indicator when focused.
fn render_managed_field(f: &mut Frame, label: &str, summary: &str, focused: bool, area: Rect) {
    let border_color = if focused { Color::Cyan } else { Color::DarkGray };
    let label_color = if focused { Color::White } else { Color::DarkGray };

    let block = Block::default()
        .title(Span::styled(
            format!(" {label} "),
            Style::default().fg(label_color).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(if focused { BorderType::Thick } else { BorderType::Rounded })
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let text_area = Rect::new(
        inner.x,
        inner.y + inner.height.saturating_sub(1),
        inner.width,
        inner.height.min(1),
    );

    let (text, color) = if focused {
        let indicator = "  [↵ manage]";
        let max_summary = (inner.width as usize).saturating_sub(indicator.len() + 1);
        let truncated = if summary.len() > max_summary {
            format!("{}…", &summary[..max_summary.saturating_sub(1)])
        } else {
            summary.to_owned()
        };
        (format!(" {truncated}{indicator}"), Color::Cyan)
    } else {
        (format!(" {summary}"), Color::Gray)
    };

    f.render_widget(
        Paragraph::new(Span::styled(text, Style::default().fg(color))),
        text_area,
    );
}

/// Render the ports management sub-dialog.
fn render_ports_dialog(f: &mut Frame, app: &App, area: Rect) {
    let dialog = &app.create_dialog.ports_dialog;
    if !dialog.visible {
        return;
    }

    match dialog.mode {
        SubDialogMode::List => {
            let visible_rows = dialog.entries.len().max(3).min(8) as u16;
            // border(2) + entries + error(1) + hint(1)
            let height = visible_rows + 4;
            let popup = centred_rect(52, height, area);
            f.render_widget(Clear, popup);

            let block = Block::default()
                .title(Span::styled(
                    " Manage Ports ",
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Yellow));

            let inner = block.inner(popup);
            f.render_widget(block, popup);

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Fill(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                ])
                .split(inner);

            if dialog.entries.is_empty() {
                f.render_widget(
                    Paragraph::new(Span::styled(
                        "  (no ports configured)",
                        Style::default().fg(Color::DarkGray),
                    )),
                    chunks[0],
                );
            } else {
                for (i, (host, guest)) in dialog.entries.iter().enumerate() {
                    if i as u16 >= chunks[0].height {
                        break;
                    }
                    let is_sel = i == dialog.selected;
                    let style = if is_sel {
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    };
                    let row = Rect::new(chunks[0].x, chunks[0].y + i as u16, chunks[0].width, 1);
                    f.render_widget(
                        Paragraph::new(Span::styled(format!("  {host}:{guest}"), style)),
                        row,
                    );
                }
            }

            if let Some(ref err) = dialog.error {
                f.render_widget(
                    Paragraph::new(Span::styled(
                        format!("  ✗ {err}"),
                        Style::default().fg(Color::Red),
                    )),
                    chunks[1],
                );
            }

            f.render_widget(
                Paragraph::new(Span::styled(
                    "  ↑↓ select   a add   d delete   Esc close",
                    Style::default().fg(Color::DarkGray),
                )),
                chunks[2],
            );
        }
        SubDialogMode::Add => {
            // border(2) + spacer(1) + host field(3) + guest field(3) + spacer(1) + hint(1) = 11
            let popup = centred_rect(52, 11, area);
            f.render_widget(Clear, popup);

            let block = Block::default()
                .title(Span::styled(
                    " Add Port Mapping ",
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Yellow));

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

            render_field(f, "Host Port ", &dialog.host_input, dialog.add_field == 0, chunks[1]);
            render_field(f, "Guest Port", &dialog.guest_input, dialog.add_field == 1, chunks[2]);

            if let Some(ref err) = dialog.error {
                f.render_widget(
                    Paragraph::new(Span::styled(
                        format!(" ✗ {err}"),
                        Style::default().fg(Color::Red),
                    )),
                    chunks[3],
                );
            } else {
                f.render_widget(
                    Paragraph::new(Span::styled(
                        " Tab field   Enter add   Esc cancel",
                        Style::default().fg(Color::DarkGray),
                    )),
                    chunks[3],
                );
            }
        }
    }
}

/// Render the env vars management sub-dialog.
fn render_env_vars_dialog(f: &mut Frame, app: &App, area: Rect) {
    let dialog = &app.create_dialog.env_vars_dialog;
    if !dialog.visible {
        return;
    }

    match dialog.mode {
        SubDialogMode::List => {
            let visible_rows = dialog.entries.len().max(3).min(8) as u16;
            let height = visible_rows + 4;
            let popup = centred_rect(60, height, area);
            f.render_widget(Clear, popup);

            let block = Block::default()
                .title(Span::styled(
                    " Manage Environment Variables ",
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Yellow));

            let inner = block.inner(popup);
            f.render_widget(block, popup);

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Fill(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                ])
                .split(inner);

            if dialog.entries.is_empty() {
                f.render_widget(
                    Paragraph::new(Span::styled(
                        "  (no environment variables configured)",
                        Style::default().fg(Color::DarkGray),
                    )),
                    chunks[0],
                );
            } else {
                for (i, (key, val)) in dialog.entries.iter().enumerate() {
                    if i as u16 >= chunks[0].height {
                        break;
                    }
                    let is_sel = i == dialog.selected;
                    let style = if is_sel {
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    };
                    // Truncate long lines to fit the popup width.
                    let max_w = chunks[0].width.saturating_sub(4) as usize;
                    let entry = format!("{key}={val}");
                    let display = if entry.len() > max_w {
                        format!("{}…", &entry[..max_w.saturating_sub(1)])
                    } else {
                        entry
                    };
                    let row = Rect::new(chunks[0].x, chunks[0].y + i as u16, chunks[0].width, 1);
                    f.render_widget(
                        Paragraph::new(Span::styled(format!("  {display}"), style)),
                        row,
                    );
                }
            }

            if let Some(ref err) = dialog.error {
                f.render_widget(
                    Paragraph::new(Span::styled(
                        format!("  ✗ {err}"),
                        Style::default().fg(Color::Red),
                    )),
                    chunks[1],
                );
            }

            f.render_widget(
                Paragraph::new(Span::styled(
                    "  ↑↓ select   a add   d delete   Esc close",
                    Style::default().fg(Color::DarkGray),
                )),
                chunks[2],
            );
        }
        SubDialogMode::Add => {
            // border(2) + spacer(1) + key field(3) + value field(3) + spacer(1) + hint(1) = 11
            let popup = centred_rect(60, 11, area);
            f.render_widget(Clear, popup);

            let block = Block::default()
                .title(Span::styled(
                    " Add Environment Variable ",
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Yellow));

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

            render_field(f, "Key  ", &dialog.key_input, dialog.add_field == 0, chunks[1]);
            render_field(f, "Value", &dialog.value_input, dialog.add_field == 1, chunks[2]);

            if let Some(ref err) = dialog.error {
                f.render_widget(
                    Paragraph::new(Span::styled(
                        format!(" ✗ {err}"),
                        Style::default().fg(Color::Red),
                    )),
                    chunks[3],
                );
            } else {
                f.render_widget(
                    Paragraph::new(Span::styled(
                        " Tab field   Enter add   Esc cancel",
                        Style::default().fg(Color::DarkGray),
                    )),
                    chunks[3],
                );
            }
        }
    }
}

/// Render the directory picker overlay on top of the dialog.
fn render_dir_picker(f: &mut Frame, app: &App, area: Rect) {
    let picker = &app.create_dialog.dir_picker;
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
        .title(Span::styled(
            title,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Yellow));

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
        " 🖴 Available Drives".to_owned()
    } else {
        format!(" 📂 {}", picker.path)
    };
    f.render_widget(
        Paragraph::new(Span::styled(header, Style::default().fg(Color::Yellow))),
        chunks[0],
    );

    // Separator
    f.render_widget(
        Paragraph::new(Span::styled(
            "─".repeat(inner.width as usize),
            Style::default().fg(Color::DarkGray),
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
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else if entry == DRIVES_ENTRY {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::White)
        };
        let entry_area = Rect::new(
            chunks[2].x,
            chunks[2].y + row as u16,
            chunks[2].width,
            1,
        );
        f.render_widget(
            Paragraph::new(Span::styled(format!(" {prefix}{entry}"), style)),
            entry_area,
        );
    }

    // Hint
    let hint = if picker.showing_drives {
        "↑↓ navigate   Enter select drive   / drives   ~ home   Esc cancel"
    } else {
        "↑↓ navigate   Enter descend   Space select   / drives   ~ home   Esc cancel"
    };
    f.render_widget(
        Paragraph::new(Span::styled(hint, Style::default().fg(Color::DarkGray))),
        chunks[3],
    );
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

