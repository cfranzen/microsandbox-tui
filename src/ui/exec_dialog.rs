//! Modal "Exec" dialog: prompts for a command line, then opens a new host
//! terminal running it inside the selected sandbox.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::Span,
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::app::App;

/// Render the exec dialog on top of everything else, if it's open.
pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let dialog = &app.exec_dialog;
    if !dialog.visible {
        return;
    }
    let theme = &app.theme;

    let popup = centred_rect(60, 8, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(Span::styled(
            format!(" Exec in '{}' ", dialog.sandbox_name),
            theme.accent_bold(),
        ))
        .borders(Borders::ALL)
        .border_type(theme.border_unfocused_type)
        .border_style(theme.accent())
        .style(theme.base_style());

    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // command input
            Constraint::Length(1), // spacing
            Constraint::Length(1), // hint/error
        ])
        .split(inner);

    let command_block = Block::default()
        .title(" Command ")
        .borders(Borders::ALL)
        .border_style(theme.accent());
    f.render_widget(
        Paragraph::new(dialog.command.as_str()).block(command_block),
        chunks[0],
    );

    if let Some(ref err) = dialog.error {
        f.render_widget(
            Paragraph::new(Span::styled(format!(" ✗ {err}"), theme.danger())),
            chunks[2],
        );
    } else {
        f.render_widget(
            Paragraph::new(theme.hint_line(&[
                ("Type", "command"),
                ("Enter", "run in new terminal"),
                ("Esc", "cancel"),
            ])),
            chunks[2],
        );
    }
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
