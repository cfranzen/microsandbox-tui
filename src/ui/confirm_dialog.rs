//! Modal "Are you sure?" confirmation dialog shown before destructive actions.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::app::App;

/// Render the confirmation dialog on top of everything else, if one is pending.
pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let Some(action) = &app.confirm else {
        return;
    };
    let theme = &app.theme;

    let message = action.confirm_message();
    let height = 6;
    let rect = centred_rect(50, height, area);

    f.render_widget(Clear, rect);

    let block = Block::default()
        .title(Span::styled(" Are you sure? ", theme.danger_bold()))
        .borders(Borders::ALL)
        .border_type(theme.border_focused_type)
        .border_style(theme.danger())
        .style(theme.base_style());

    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let lines = vec![
        Line::from(Span::raw(message)),
        Line::from(""),
        Line::from(theme.hint_spans(&[("y/Enter", "confirm"), ("n/Esc", "cancel")])),
    ];

    f.render_widget(
        Paragraph::new(lines).alignment(ratatui::layout::Alignment::Center),
        inner,
    );
}

/// Compute a centred rectangle of a fixed height and a percentage of the width.
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
