//! Main layout: header, two-column body, footer.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::app::App;

/// Returns (list_area, detail_area, header_area, footer_area).
pub fn split(area: Rect) -> (Rect, Rect, Rect, Rect) {
    // Vertical split: header(1) / body / footer(1)
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    let header_area = vertical[0];
    let body_area = vertical[1];
    let footer_area = vertical[2];

    // Horizontal split: list(27%) | detail(73%)
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(27), Constraint::Percentage(73)])
        .split(body_area);

    (horizontal[0], horizontal[1], header_area, footer_area)
}

/// Render the top header bar.
pub fn render_header(f: &mut Frame, area: Rect) {
    let title = Span::styled(
        " Microsandbox TUI ",
        Style::default()
            .fg(Color::Black)
            .bg(Color::White)
            .add_modifier(Modifier::BOLD),
    );
    let subtitle = Span::styled(
        " Manage microsandbox microVMs",
        Style::default().fg(Color::DarkGray),
    );
    let version = Span::styled(" v0.1.0 ", Style::default().fg(Color::DarkGray));

    let line = Line::from(vec![title, subtitle]);
    let version_line = Line::from(vec![version]).alignment(Alignment::Right);

    // Two overlapping paragraphs: left-aligned title, right-aligned version
    f.render_widget(
        Paragraph::new(line).style(Style::default().bg(Color::Black)),
        area,
    );

    // Overlay the version on the right
    f.render_widget(
        Paragraph::new(version_line).style(Style::default().bg(Color::Black)),
        area,
    );
}

/// Render the bottom footer bar with keybind hints and optional notification.
pub fn render_footer(f: &mut Frame, app: &App, area: Rect) {
    // Show notification if active, otherwise show keybind hints
    if let Some(ref n) = app.notification {
        let style = if n.is_error {
            Style::default().fg(Color::Red).bg(Color::Black)
        } else {
            Style::default().fg(Color::Green).bg(Color::Black)
        };
        let prefix = if n.is_error { "✗ " } else { "✓ " };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(prefix, style.add_modifier(Modifier::BOLD)),
                Span::styled(n.message.as_str(), style),
            ]))
            .style(Style::default().bg(Color::Black)),
            area,
        );
        return;
    }

    // Context-sensitive hints
    let dim = Style::default().fg(Color::DarkGray);
    let key = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);

    let mut spans: Vec<Span> = vec![
        Span::styled("q", key),
        Span::styled(" quit  ", dim),
        Span::styled("↑↓/jk", key),
        Span::styled(" navigate  ", dim),
        Span::styled("Tab", key),
        Span::styled(" switch panel  ", dim),
    ];

    if app.create_dialog.visible {
        spans.extend([
            Span::styled("Tab", key),
            Span::styled(" next field  ", dim),
            Span::styled("Enter", key),
            Span::styled(" create  ", dim),
            Span::styled("Esc", key),
            Span::styled(" cancel", dim),
        ]);
    } else {
        use crate::app::Focus;
        if app.focus == Focus::SandboxList && !app.new_sandbox_selected() {
            spans.extend([
                Span::styled("s", key),
                Span::styled(" start  ", dim),
                Span::styled("S", key),
                Span::styled(" stop  ", dim),
                Span::styled("K", key),
                Span::styled(" kill  ", dim),
                Span::styled("d", key),
                Span::styled(" remove  ", dim),
            ]);
        }
        spans.extend([
            Span::styled("n", key),
            Span::styled(" new  ", dim),
            Span::styled("r", key),
            Span::styled(" refresh", dim),
        ]);
    }

    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(Color::Black)),
        area,
    );
}
