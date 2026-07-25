//! "New Sandbox" creation modal dialog.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::app::{App, DialogTab, DRIVES_ENTRY, PICKER_VISIBLE_ROWS};

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
            render_field(f, "Name    ", &dlg.name, dlg.field == 0, chunks[2]);
            render_field(f, "Image   ", &dlg.image, dlg.field == 1, chunks[3]);
            render_field(f, "CPUs    ", &dlg.cpus, dlg.field == 2, chunks[4]);
            render_field(f, "Memory  ", &dlg.memory, dlg.field == 3, chunks[5]);
            render_field(f, "Ports   ", &dlg.ports, dlg.field == 4, chunks[6]);
            render_field(f, "Env Vars", &dlg.env_vars, dlg.field == 5, chunks[7]);
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
    let hint = if dlg.tab == DialogTab::Basic && dlg.field == 6 {
        "Tab/↑↓ field   ◄► tab   Ctrl+F browse   Enter create   Esc cancel"
    } else {
        "Tab/↑↓ field   ◄► tab   Space toggle   Enter create   Esc cancel"
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

