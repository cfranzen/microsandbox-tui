//! Main layout: header, two-column body, footer.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::Modifier,
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
pub fn render_header(f: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let title = Span::styled(" Microsandbox TUI ", theme.badge());
    let subtitle = Span::styled(" Manage microsandbox microVMs", theme.muted());
    let version = Span::styled(" v0.1.0 ", theme.muted());

    let line = Line::from(vec![title, subtitle]);
    let version_line = Line::from(vec![version]).alignment(Alignment::Right);

    // Two overlapping paragraphs: left-aligned title, right-aligned version
    f.render_widget(Paragraph::new(line).style(theme.base_style()), area);

    // Overlay the version on the right
    f.render_widget(Paragraph::new(version_line).style(theme.base_style()), area);
}

/// Render the bottom footer bar with keybind hints and optional notification.
pub fn render_footer(f: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;

    // Show notification if active, otherwise show keybind hints
    if let Some(ref n) = app.notification {
        let style = if n.is_error {
            theme.danger()
        } else {
            theme.success()
        };
        let prefix = if n.is_error { "✗ " } else { "✓ " };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(prefix, style.add_modifier(Modifier::BOLD)),
                Span::styled(n.message.as_str(), style),
            ]))
            .style(theme.base_style()),
            area,
        );
        return;
    }

    // Context-sensitive hints
    let mut pairs: Vec<(&str, &str)> = vec![("q", "quit"), ("↑↓←→/Tab", "navigate")];

    if app.confirm.is_some() {
        pairs = vec![("y/Enter", "confirm"), ("n/Esc", "cancel")];
    } else if app.search_active {
        pairs = vec![
            ("Type", "to filter"),
            ("Enter", "confirm"),
            ("Esc", "clear & exit"),
        ];
    } else if app.create_dialog.visible {
        pairs.extend([
            ("Tab", "next field"),
            ("Enter", "create"),
            ("Esc", "cancel"),
        ]);
    } else if app.volumes_view.visible {
        pairs.extend([
            ("↑↓", "select"),
            ("n", "new"),
            ("d", "delete"),
            ("Esc", "close"),
        ]);
    } else {
        pairs.extend([
            ("n", "new"),
            ("/", "search"),
            ("v", "volumes"),
            ("r", "refresh"),
            ("T", "theme"),
        ]);
    }

    let spans = theme.hint_spans(&pairs);

    f.render_widget(
        Paragraph::new(Line::from(spans)).style(theme.base_style()),
        area,
    );
}
