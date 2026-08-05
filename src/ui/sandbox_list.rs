//! Left panel: sandbox list with status cards.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};

use microsandbox::sandbox::SandboxStatus;

use crate::app::{App, Focus};
use crate::sandbox::SandboxInfo;
use crate::theme::Theme;

/// Height of a single sandbox card (lines inside the border).
const CARD_INNER_HEIGHT: u16 = 4;
/// Total card height including border.
const CARD_TOTAL_HEIGHT: u16 = CARD_INNER_HEIGHT + 2;
/// Height of the "New Sandbox" placeholder.
const NEW_CARD_HEIGHT: u16 = 3;

/// Render the sandbox list panel.
///
/// The panel has no border of its own — the rightmost column is reserved
/// for a vertical scrollbar, which doubles as the visual divider between
/// the sandbox list and the detail view.
pub fn render(f: &mut Frame, app: &mut App, area: Rect) {
    let theme = app.theme;
    let panel_focused = app.focus == Focus::SandboxList;
    let title = if app.search_active {
        format!(" Sandboxes — search: {}_ ", app.filter)
    } else if !app.filter.trim().is_empty() {
        format!(" Sandboxes — filter: {} ", app.filter)
    } else {
        " Sandboxes ".to_string()
    };
    let title_style = if panel_focused {
        theme.title_accent()
    } else {
        theme.muted()
    };

    // Reserve the rightmost column of the whole area for the divider
    // scrollbar; everything else renders in the remaining width.
    let content_width = area.width.saturating_sub(1);
    let title_area = Rect {
        x: area.x,
        y: area.y,
        width: content_width,
        height: area.height.min(1),
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(title, title_style))),
        title_area,
    );

    let inner = Rect {
        x: area.x,
        y: area.y + 1,
        width: content_width,
        height: area.height.saturating_sub(1),
    };

    if inner.height == 0 {
        return;
    }

    let visible = app.visible_indices();
    // "New Sandbox" placeholder is hidden while a filter is active.
    let show_new_card = app.filter.trim().is_empty();
    let total_items = visible.len() + usize::from(show_new_card);

    app.mouse.card_rects.clear();

    if total_items == 0 {
        let msg = Line::from(Span::styled("No sandboxes match the filter", theme.muted()));
        f.render_widget(Paragraph::new(msg), inner);
        let mut scrollbar_state = ScrollbarState::new(0).position(0);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_style(theme.border_style(false))
            .thumb_style(theme.border_style(panel_focused));
        f.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
        return;
    }

    // Position of the current selection within the displayed (filtered) order.
    let selected_pos = if app.new_sandbox_selected() {
        visible.len()
    } else {
        visible.iter().position(|&i| i == app.selected).unwrap_or(0)
    };

    // Simple scroll: figure out the first visible item based on selected
    let visible_height = inner.height;
    let first_visible = compute_first_visible(selected_pos, total_items, visible_height);

    let mut y = inner.y;
    let x = inner.x;
    let w = inner.width;

    for item_idx in first_visible..total_items {
        let selected = item_idx == selected_pos;

        if item_idx < visible.len() {
            // Regular sandbox card
            if y + CARD_TOTAL_HEIGHT > inner.y + visible_height {
                break;
            }
            let abs_idx = visible[item_idx];
            let card_area = Rect {
                x,
                y,
                width: w,
                height: CARD_TOTAL_HEIGHT,
            };
            app.mouse.card_rects.push((card_area, Some(abs_idx)));
            render_sandbox_card(
                f,
                &theme,
                &app.sandboxes[abs_idx],
                selected,
                panel_focused,
                card_area,
            );
            y += CARD_TOTAL_HEIGHT;
        } else {
            // "New Sandbox" entry
            if y + NEW_CARD_HEIGHT > inner.y + visible_height {
                break;
            }
            let card_area = Rect {
                x,
                y,
                width: w,
                height: NEW_CARD_HEIGHT,
            };
            app.mouse.card_rects.push((card_area, None));
            render_new_sandbox_card(f, &theme, selected, panel_focused, card_area);
        }
    }

    // Divider scrollbar: spans the full height of the panel (title row
    // included) and separates the sandbox list from the detail view.
    let mut scrollbar_state =
        ScrollbarState::new(total_items.saturating_sub(1)).position(first_visible);
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(None)
        .end_symbol(None)
        .track_style(theme.border_style(false))
        .thumb_style(theme.border_style(panel_focused));
    f.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
}

/// Render a single sandbox card.
fn render_sandbox_card(
    f: &mut Frame,
    theme: &Theme,
    sb: &SandboxInfo,
    selected: bool,
    panel_focused: bool,
    area: Rect,
) {
    let (status_color, status_dot) = status_style(theme, sb.status);

    let highlight = selected && panel_focused;

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(theme.border_type(highlight))
        .border_style(theme.border_style(highlight))
        .title(Span::styled(" Sandbox ", theme.title_accent()))
        .title_alignment(ratatui::layout::Alignment::Right);

    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height < 1 || inner.width < 2 {
        return;
    }

    // Line 1: status dot + name (+ selected checkmark)
    let name_style = if selected {
        theme.text_bold()
    } else {
        theme.secondary()
    };
    let name_line = Line::from(vec![
        Span::styled(status_dot, Style::default().fg(status_color)),
        Span::raw(" "),
        Span::styled(
            truncate(&sb.name, inner.width.saturating_sub(4) as usize),
            name_style,
        ),
        if selected {
            Span::styled(" ✓", theme.success())
        } else {
            Span::raw("")
        },
    ]);

    // Line 2: image name
    let image_line = Line::from(vec![Span::styled(
        format!(
            "  {}",
            truncate(&sb.image, inner.width.saturating_sub(3) as usize)
        ),
        theme.muted(),
    )]);

    // Line 3: cpu · memory · age
    let age = sb
        .created_at
        .map(|t| {
            let secs = (chrono::Utc::now() - t).num_seconds().max(0) as u64;
            humantime::format_duration(std::time::Duration::from_secs(secs)).to_string()
        })
        .unwrap_or_else(|| "—".into());

    let resource_line = Line::from(vec![Span::styled(
        format!(
            "  {}cpus · {}MiB · {}",
            sb.cpus,
            sb.memory_mib,
            truncate(&age, 10)
        ),
        theme.muted(),
    )]);

    // Line 4: action hints
    let action_line = if selected && panel_focused {
        match sb.status {
            SandboxStatus::Running => {
                Line::from(theme.hint_spans(&[("s", "top"), ("t", "erm"), ("d", "el")]))
            }
            SandboxStatus::Stopped => Line::from(theme.hint_spans(&[("s", "tart"), ("d", "el")])),
            _ => Line::from(theme.hint_spans(&[("d", "el")])),
        }
    } else {
        Line::raw("")
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);

    f.render_widget(Paragraph::new(name_line), chunks[0]);
    f.render_widget(Paragraph::new(image_line), chunks[1]);
    f.render_widget(Paragraph::new(resource_line), chunks[2]);
    f.render_widget(Paragraph::new(action_line), chunks[3]);
}

/// Render the "New Sandbox" placeholder card.
fn render_new_sandbox_card(
    f: &mut Frame,
    theme: &Theme,
    selected: bool,
    panel_focused: bool,
    area: Rect,
) {
    let highlight = selected && panel_focused;
    let style = theme.border_style(highlight);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(theme.border_type(highlight))
        .border_style(style);

    let inner = block.inner(area);
    f.render_widget(block, area);

    let label = Line::from(vec![Span::styled(
        "+ New Sandbox",
        style.add_modifier(Modifier::BOLD),
    )]);
    f.render_widget(
        Paragraph::new(label).alignment(ratatui::layout::Alignment::Center),
        inner,
    );
}

//--------------------------------------------------------------------------------------------------
// Helpers
//--------------------------------------------------------------------------------------------------

fn status_style(theme: &Theme, status: SandboxStatus) -> (ratatui::style::Color, &'static str) {
    let glyph = match status {
        SandboxStatus::Running => "●",
        SandboxStatus::Stopped => "■",
        SandboxStatus::Crashed => "✗",
        _ => "○",
    };
    (theme.status_color(status), glyph)
}

fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.len() <= max {
        s.to_owned()
    } else {
        format!("{}…", &s[..max.saturating_sub(1)])
    }
}

/// Given the selected index and available height (in lines), compute the
/// first item index that keeps the selected item visible.
fn compute_first_visible(selected: usize, _total: usize, visible_height: u16) -> usize {
    // Each sandbox card is CARD_TOTAL_HEIGHT rows, "New" card is NEW_CARD_HEIGHT.
    // For simplicity: estimate how many items fit and scroll so selected is visible.
    let items_fit = (visible_height / CARD_TOTAL_HEIGHT).max(1) as usize;
    if selected < items_fit {
        0
    } else {
        selected.saturating_sub(items_fit - 1)
    }
}
