//! Right panel: tab bar + dispatch to tab-specific renderers.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use crate::app::{App, DetailTab, Focus};

pub fn render(f: &mut Frame, app: &mut App, area: Rect) {
    let panel_focused = app.focus == Focus::Detail;

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if panel_focused {
            BorderType::Thick
        } else {
            BorderType::Rounded
        })
        .border_style(Style::default().fg(if panel_focused {
            Color::Cyan
        } else {
            Color::DarkGray
        }));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height < 3 {
        return;
    }

    // Show sandbox name as a title inside the panel
    if let Some(sb) = app.selected_sandbox() {
        let title = Paragraph::new(Line::from(vec![Span::styled(
            sb.name.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )]));
        let title_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: 1,
        };
        f.render_widget(title, title_area);
    } else if app.sandboxes.is_empty() {
        let hint = Paragraph::new(Line::from(vec![Span::styled(
            "No sandboxes found. Press n to create one.",
            Style::default().fg(Color::DarkGray),
        )]));
        f.render_widget(hint, inner);
        return;
    } else {
        // "New Sandbox" selected - show create hint
        let hint = Paragraph::new(Line::from(vec![Span::styled(
            "Press Enter or n to create a new sandbox.",
            Style::default().fg(Color::DarkGray),
        )]));
        f.render_widget(hint, inner);
        return;
    }

    // Split: title(1) + tab_bar(1) + separator(1) + content
    let splits = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // sandbox name
            Constraint::Length(1), // tab bar
            Constraint::Length(1), // separator
            Constraint::Min(0),    // tab content
        ])
        .split(inner);

    render_tab_bar(f, app, splits[1]);
    render_separator(f, splits[2]);
    render_tab_content(f, app, splits[3]);
}

fn render_tab_bar(f: &mut Frame, app: &mut App, area: Rect) {
    let mut spans: Vec<Span> = Vec::new();
    app.mouse.tab_rects.clear();
    let mut x = area.x;

    for &tab in DetailTab::all() {
        let active = tab == app.tab;
        let style = if active {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let label = format!(" {} ", tab.title());
        let label_width = label.chars().count() as u16;
        spans.push(Span::styled(label, style));
        spans.push(Span::raw("  "));

        if x < area.x + area.width {
            let rect = Rect {
                x,
                y: area.y,
                width: label_width.min(area.x + area.width - x),
                height: 1,
            };
            app.mouse.tab_rects.push((rect, tab));
        }
        x += label_width + 2;
    }

    // Show ←/→ hint when detail is focused
    if app.focus == Focus::Detail {
        spans.push(Span::styled(
            "←→ switch",
            Style::default().fg(Color::DarkGray),
        ));
    }

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_separator(f: &mut Frame, area: Rect) {
    let line = "─".repeat(area.width as usize);
    f.render_widget(
        Paragraph::new(Span::styled(line, Style::default().fg(Color::DarkGray))),
        area,
    );
}

fn render_tab_content(f: &mut Frame, app: &mut App, area: Rect) {
    match app.tab {
        DetailTab::Logs => super::logs::render(f, app, area),
        DetailTab::Filesystem => super::filesystem::render(f, app, area),
        DetailTab::Info => super::info::render(f, app, area),
    }
}
