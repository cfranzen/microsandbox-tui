//! Left panel: sandbox list with status cards.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols::line,
    text::{Line, Span},
    widgets::{
        Block, Borders, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
    },
    Frame,
};

use microsandbox::sandbox::SandboxStatus;

use crate::app::{App, Focus};
use crate::sandbox::SandboxInfo;
use crate::theme::Theme;

use super::util::fmt_bytes;

/// Height of a single sandbox card (lines inside the border/padding):
/// name, separator, image, resources, separator, actions bar.
const CARD_INNER_HEIGHT: u16 = 6;
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

    let sandbox_cards_area = Rect {
        x: area.x,
        y: area.y + 1,
        width: content_width,
        height: area.height.saturating_sub(1),
    };

    if sandbox_cards_area.height == 0 {
        return;
    }

    let visible = app.visible_indices();
    // "New Sandbox" placeholder is hidden while a filter is active.
    let show_new_card = app.filter.trim().is_empty();
    let total_items = visible.len() + usize::from(show_new_card);

    app.mouse.card_rects.clear();

    if total_items == 0 {
        let msg = Line::from(Span::styled("No sandboxes match the filter", theme.muted()));
        f.render_widget(Paragraph::new(msg), sandbox_cards_area);
        let mut scrollbar_state = ScrollbarState::new(0).position(0);
        render_scrollbar(f, area, theme, panel_focused, &mut scrollbar_state);
        return;
    }

    // Position of the current selection within the displayed (filtered) order.
    let selected_pos = if app.new_sandbox_selected() {
        visible.len()
    } else {
        visible.iter().position(|&i| i == app.selected).unwrap_or(0)
    };

    // Simple scroll: figure out the first visible item based on selected
    let visible_height = sandbox_cards_area.height;
    let first_visible = compute_first_visible(selected_pos, total_items, visible_height);

    let mut y = sandbox_cards_area.y;
    let x = sandbox_cards_area.x;
    let w = sandbox_cards_area.width;

    for item_idx in first_visible..total_items {
        let selected = item_idx == selected_pos;

        if item_idx < visible.len() {
            // Regular sandbox card
            if y + CARD_TOTAL_HEIGHT > sandbox_cards_area.y + visible_height {
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
            let sb = &app.sandboxes[abs_idx];
            let disk_used_bytes = app.metrics.get(&sb.name).and_then(|m| m.disk_used_bytes);
            render_sandbox_card(
                f,
                &theme,
                sb,
                disk_used_bytes,
                selected,
                panel_focused,
                card_area,
            );
            y += CARD_TOTAL_HEIGHT;
        } else {
            // "New Sandbox" entry
            if y + NEW_CARD_HEIGHT > sandbox_cards_area.y + visible_height {
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
    render_scrollbar(f, area, theme, panel_focused, &mut scrollbar_state);
}

fn render_scrollbar(
    f: &mut Frame,
    area: Rect,
    theme: Theme,
    panel_focused: bool,
    scrollbar_state: &mut ScrollbarState,
) {
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(None)
        .thumb_symbol(line::VERTICAL)
        .track_symbol(Some(line::VERTICAL))
        .end_symbol(None)
        .track_style(theme.border_style(false))
        .thumb_style(theme.border_style(panel_focused));
    f.render_stateful_widget(scrollbar, area, scrollbar_state);
}

/// Render a single sandbox card.
fn render_sandbox_card(
    f: &mut Frame,
    theme: &Theme,
    sb: &SandboxInfo,
    disk_used_bytes: Option<u64>,
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
        .padding(Padding::horizontal(1))
        .title(Span::styled(" Sandbox ", theme.title_accent()))
        .title_alignment(Alignment::Right);

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
            truncate(&sb.name, inner.width.saturating_sub(3) as usize),
            name_style,
        ),
        if selected {
            Span::styled(" ✓", theme.success())
        } else {
            Span::raw("")
        },
    ]);

    // Line 2: image name, on its own dedicated line.
    let image_line = Line::from(vec![Span::styled(
        format!(
            "Image: {}",
            truncate(&sb.image, inner.width.saturating_sub(7) as usize)
        ),
        theme.muted(),
    )]);

    // Line 3: cpu · memory · disk · age
    let age = sb
        .created_at
        .map(|t| {
            let secs = (chrono::Utc::now() - t).num_seconds().max(0) as u64;
            humantime::format_duration(std::time::Duration::from_secs(secs)).to_string()
        })
        .unwrap_or_else(|| "—".into());
    let disk = disk_used_bytes.map(fmt_bytes).unwrap_or_else(|| "—".into());

    let resource_line = Line::from(vec![Span::styled(
        format!(
            "{}cpus · {}MiB · {} disk · {}",
            sb.cpus,
            sb.memory_mib,
            disk,
            truncate(&age, 10)
        ),
        theme.muted(),
    )]);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Title
            Constraint::Length(1), // Separator
            Constraint::Length(1), // Image
            Constraint::Length(1), // Metrics
            Constraint::Length(1), // Separator
            Constraint::Length(1), // Actions bar
        ])
        .split(inner);

    f.render_widget(Paragraph::new(name_line), chunks[0]);
    f.render_widget(Paragraph::new(Line::from(Span::styled("─".repeat(inner.width as usize), theme.muted()))), chunks[1]);
    f.render_widget(Paragraph::new(image_line), chunks[2]);
    f.render_widget(Paragraph::new(resource_line), chunks[3]);
    f.render_widget(Paragraph::new(Line::from(Span::styled("─".repeat(inner.width as usize), theme.muted()))), chunks[4]);
    render_actions_bar(f, *theme, sb.status, chunks[5]);
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
        .padding(Padding::horizontal(1))
        .border_type(theme.border_type(highlight))
        .border_style(style);

    let inner = block.inner(area);
    f.render_widget(block, area);

    let label = Line::from(vec![Span::styled(
        "+ New Sandbox",
        style.add_modifier(Modifier::BOLD),
    )]);
    f.render_widget(Paragraph::new(label).alignment(Alignment::Center), inner);
}

//--------------------------------------------------------------------------------------------------
// Helpers
//--------------------------------------------------------------------------------------------------

/// The action shortcuts available for a sandbox in a given status, each
/// with its own semantic color: start = success, stop = warning,
/// terminate/delete = danger.
fn action_items(theme: &Theme, status: SandboxStatus) -> Vec<(&'static str, &'static str, Color)> {
    match status {
        SandboxStatus::Running => vec![
            ("s", "top", theme.warning),
            ("t", "erm", theme.danger),
            ("d", "el", theme.danger),
        ],
        SandboxStatus::Stopped => vec![
            ("s", "tart", theme.success),
            ("d", "el", theme.danger)],
        _ => vec![
            ("d", "el", theme.danger)
        ],
    }
}

/// Render the action shortcuts distributed evenly across the full card
/// width. Each shortcut's key letter is bold; the color encodes the
/// action's meaning (see [`action_items`]).
fn render_actions_bar(f: &mut Frame, theme: Theme, status: SandboxStatus, area: Rect) {
    let items = action_items(&theme, status);
    if items.is_empty() {
        return;
    }

    let constraints: Vec<Constraint> = items
        .iter()
        .map(|_| Constraint::Ratio(1, items.len() as u32))
        .collect();
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);

    for (i, (key, rest, color)) in items.iter().enumerate() {
        let line = Line::from(vec![
            Span::styled(
                *key,
                Style::default().fg(*color).add_modifier(Modifier::BOLD).add_modifier(Modifier::UNDERLINED),
            ),
            Span::styled(*rest, Style::default().fg(*color)),
        ]);
        f.render_widget(Paragraph::new(line).alignment(Alignment::Center), cols[i]);
    }
}

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
