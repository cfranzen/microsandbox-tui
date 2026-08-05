//! Logs tab: coloured log lines, source badges, scroll.

use microsandbox::sandbox::LogSource;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::App;
use crate::theme::Theme;

pub fn render(f: &mut Frame, app: &mut App, area: Rect) {
    let theme = app.theme;
    let name = match app.selected_sandbox() {
        Some(sb) => sb.name.clone(),
        None => return,
    };

    let entries = app.logs.get(&name);

    if entries.is_none() {
        // Trigger a fetch if we have no data yet
        app.request_logs(&name);
        f.render_widget(
            Paragraph::new(Span::styled("Loading logs…", theme.muted())),
            area,
        );
        return;
    }

    let entries = entries.unwrap();

    if entries.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled("No log entries found.", theme.muted())),
            area,
        );
        return;
    }

    let lines: Vec<Line> = entries
        .iter()
        .map(|e| {
            let (badge, badge_color) = source_badge(&theme, e.source);
            let ts = e.timestamp.format("%H:%M:%S").to_string();
            let text = String::from_utf8_lossy(&e.data);
            let text = text.trim_end_matches('\n');

            Line::from(vec![
                Span::styled(format!("{ts} "), theme.muted()),
                Span::styled(
                    format!("[{badge}] "),
                    ratatui::style::Style::default()
                        .fg(badge_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    text.to_string(),
                    ratatui::style::Style::default().fg(message_color(&theme, e.source)),
                ),
            ])
        })
        .collect();

    let total_lines = lines.len();
    let visible_height = area.height as usize;

    // Clamp scroll so the view doesn't go past the end
    let max_scroll = total_lines.saturating_sub(visible_height);
    // Auto-scroll to bottom (scroll_lock = 0 means "follow tail")
    let scroll = if app.log_scroll == 0 {
        max_scroll
    } else {
        app.log_scroll.min(max_scroll)
    };

    // Summary line count at bottom
    let count_line = format!(" {} lines ", total_lines);

    let widget = Paragraph::new(lines).scroll((scroll as u16, 0)).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(theme.muted())
            .title_bottom(Span::styled(count_line, theme.muted()))
            .title_alignment(ratatui::layout::Alignment::Right),
    );

    f.render_widget(widget, area);
}

fn source_badge(theme: &Theme, source: LogSource) -> (&'static str, Color) {
    match source {
        LogSource::Stdout => ("OUT", theme.success),
        LogSource::Stderr => ("ERR", theme.danger),
        LogSource::Output => ("PTY", theme.info),
        LogSource::System => ("SYS", theme.text_muted),
    }
}

fn message_color(theme: &Theme, source: LogSource) -> Color {
    match source {
        LogSource::Stdout => theme.text,
        LogSource::Stderr => theme.danger_text,
        LogSource::Output => theme.text,
        LogSource::System => theme.text_muted,
    }
}
