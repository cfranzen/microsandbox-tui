//! "New Sandbox" creation modal dialog.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

use crate::app::App;

/// Render the create-sandbox dialog centred over the full terminal area.
pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let dlg = &app.create_dialog;
    if !dlg.visible {
        return;
    }

    // Calculate a centred popup area (60% wide, ~16 rows tall)
    let popup = centred_rect(60, 16, area);
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

    // Split the inner area for labels + fields + error + hint
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // spacer
            Constraint::Length(3), // name
            Constraint::Length(3), // image
            Constraint::Length(3), // cpus
            Constraint::Length(3), // memory
            Constraint::Length(1), // spacer
            Constraint::Length(1), // error or hint
        ])
        .split(inner);

    render_field(f, "Name   ", &dlg.name, dlg.field == 0, chunks[1]);
    render_field(f, "Image  ", &dlg.image, dlg.field == 1, chunks[2]);
    render_field(f, "CPUs   ", &dlg.cpus, dlg.field == 2, chunks[3]);
    render_field(f, "Memory ", &dlg.memory, dlg.field == 3, chunks[4]);

    // Error or keybind hint
    if let Some(ref err) = dlg.error {
        f.render_widget(
            Paragraph::new(Span::styled(
                format!("  ✗ {err}"),
                Style::default().fg(Color::Red),
            )),
            chunks[6],
        );
    } else {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("  Tab", Style::default().fg(Color::Yellow)),
                Span::styled(" next field  ", Style::default().fg(Color::DarkGray)),
                Span::styled("Enter", Style::default().fg(Color::Yellow)),
                Span::styled(" create  ", Style::default().fg(Color::DarkGray)),
                Span::styled("Esc", Style::default().fg(Color::Yellow)),
                Span::styled(" cancel", Style::default().fg(Color::DarkGray)),
            ])),
            chunks[6],
        );
    }
}

fn render_field(f: &mut Frame, label: &str, value: &str, focused: bool, area: Rect) {
    let border_color = if focused { Color::Cyan } else { Color::DarkGray };
    let label_color = if focused { Color::White } else { Color::DarkGray };

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

    // Show value with a blinking cursor indicator when focused
    let display = if focused {
        format!("{value}▌")
    } else {
        value.to_owned()
    };

    f.render_widget(
        Paragraph::new(Span::styled(
            display,
            Style::default().fg(if focused { Color::White } else { Color::Gray }),
        )),
        inner,
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
