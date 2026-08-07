//! Metrics tab: live CPU/memory/disk gauges with history sparklines, plus
//! disk and network I/O throughput graphs.
//!
//! Laid out as a 2x2 grid of self-contained "cards" (Compute: CPU + Memory,
//! I/O: Disk + Network) so every stat gets one consistent rounded border
//! instead of the previous stack of mismatched full/partial-border boxes.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Padding, Paragraph, Sparkline},
    Frame,
};

use crate::app::App;
use crate::sandbox::MetricsSnapshot;
use crate::theme::Theme;

use super::util::fmt_bytes;

pub fn render(f: &mut Frame, app: &mut App, area: Rect) {
    let Some(sb) = app.selected_sandbox().cloned() else {
        return;
    };

    let block = Block::default().padding(Padding::new(1, 1, 1, 0));
    let inner = block.inner(area);
    f.render_widget(block, area);

    render_metrics(f, app, &sb, inner);
}

fn render_metrics(f: &mut Frame, app: &mut App, sb: &crate::sandbox::SandboxInfo, area: Rect) {
    let theme = app.theme;
    let name = sb.name.clone();
    let is_running = sb.status == crate::sandbox::SandboxStatus::Running;

    if !is_running {
        f.render_widget(
            Paragraph::new(Span::styled(
                "Sandbox is not running. Start it to see live metrics.",
                theme.muted(),
            )),
            area,
        );
        return;
    }

    let metrics = app.metrics.get(&name).cloned();

    if metrics.is_none() {
        app.request_metrics(&name);
        f.render_widget(
            Paragraph::new(Span::styled("Loading metrics…", theme.muted())),
            area,
        );
        return;
    }

    let m = metrics.unwrap_or_default();
    let history: Vec<MetricsSnapshot> = app
        .metrics_history
        .get(&name)
        .map(|h| h.iter().cloned().collect())
        .unwrap_or_default();

    // Two columns: "Compute" (CPU + Memory) on the left, "I/O" (Disk +
    // Network) on the right. Falls back to a single column on narrow
    // terminals so cards never get squeezed unreadably thin.
    let columns = if area.width >= 64 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area)
    };

    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
        .split(columns[0]);
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
        .split(columns[1]);

    render_cpu_card(f, &theme, &m, &history, pad(left[0]));
    render_memory_card(f, &theme, sb, &m, &history, pad(left[1]));
    render_disk_card(f, &theme, &m, &history, pad(right[0]));
    render_network_card(f, &theme, &m, &history, pad(right[1]));
}

/// Shrink a card's outer rect by a 1-cell gap on the right/bottom so
/// adjacent cards in the grid never touch borders.
fn pad(area: Rect) -> Rect {
    Rect {
        x: area.x,
        y: area.y,
        width: area.width.saturating_sub(1).max(1),
        height: area.height.saturating_sub(1).max(1),
    }
}

/// Draw a card's frame: a neutral rounded border, a label on the top-left,
/// and a right-aligned headline value styled in `value_color` (so status
/// still reads at a glance via the number, without tinting the whole
/// border).
fn card_frame(
    f: &mut Frame,
    theme: &Theme,
    label: &str,
    value: &str,
    value_color: Color,
    area: Rect,
) -> Rect {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(theme.border_type(false))
        .border_style(theme.border_style(false))
        .title_top(Line::from(Span::styled(
            format!(" {label} "),
            theme.title_accent(),
        )))
        .title_top(
            Line::from(Span::styled(
                format!(" {value} "),
                Style::default()
                    .fg(value_color)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            ))
            .alignment(Alignment::Right),
        )
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);
    inner
}

fn render_cpu_card(
    f: &mut Frame,
    theme: &Theme,
    m: &MetricsSnapshot,
    history: &[MetricsSnapshot],
    area: Rect,
) {
    let pct = m.cpu_percent.clamp(0.0, 100.0);
    let color = theme.gauge_color(pct);
    let inner = card_frame(f, theme, "CPU", &format!("{pct:.1}%"), color, area);
    if inner.height == 0 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner);

    render_bar(f, color, pct, rows[0]);
    render_history(
        f,
        theme,
        "",
        history
            .iter()
            .map(|s| s.cpu_percent.clamp(0.0, 100.0) as u64),
        |v| format!("{v}%"),
        rows[2],
    );
}

fn render_memory_card(
    f: &mut Frame,
    theme: &Theme,
    sb: &crate::sandbox::SandboxInfo,
    m: &MetricsSnapshot,
    history: &[MetricsSnapshot],
    area: Rect,
) {
    let total_mib = sb.memory_mib as u64;
    let used_mib = m.memory_bytes / 1_048_576;
    let pct = used_mib
        .checked_mul(100)
        .and_then(|v| v.checked_div(total_mib))
        .unwrap_or(0)
        .min(100) as f64;
    let color = theme.gauge_color(pct);
    let inner = card_frame(
        f,
        theme,
        "Memory",
        &format!("{used_mib} / {total_mib} MiB"),
        color,
        area,
    );
    if inner.height == 0 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner);

    render_bar(f, color, pct, rows[0]);
    render_history(
        f,
        theme,
        "MiB",
        history.iter().map(|s| s.memory_bytes / 1_048_576),
        |v| format!("{v}"),
        rows[2],
    );
}

fn render_disk_card(
    f: &mut Frame,
    theme: &Theme,
    m: &MetricsSnapshot,
    history: &[MetricsSnapshot],
    area: Rect,
) {
    let used = m.disk_used_bytes;
    let free = m.disk_free_bytes;
    let pct = match (used, free) {
        (Some(used), Some(free)) => {
            let total = used + free;
            used.checked_mul(100)
                .and_then(|v| v.checked_div(total))
                .unwrap_or(0)
                .min(100) as f64
        }
        _ => 0.0,
    };
    let color = if used.is_some() {
        theme.gauge_color(pct)
    } else {
        theme.text_muted
    };
    let value = match (used, free) {
        (Some(used), Some(free)) => {
            format!("{} / {} used", fmt_bytes(used), fmt_bytes(used + free))
        }
        _ => "n/a".to_string(),
    };
    let inner = card_frame(f, theme, "Disk", &value, color, area);
    if inner.height == 0 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // usage bar (or hint)
            Constraint::Length(1), // I/O throughput label
            Constraint::Min(0),    // I/O sparkline
        ])
        .split(inner);

    if used.is_some() {
        render_bar(f, color, pct, rows[0]);
    } else {
        f.render_widget(
            Paragraph::new(Span::styled(
                "usage not reported by this sandbox",
                theme.muted(),
            )),
            rows[0],
        );
    }

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("I/O  ", theme.muted()),
            Span::styled(
                format!("↑ {}", fmt_bytes(m.disk_write_bytes)),
                theme.success(),
            ),
            Span::raw("  "),
            Span::styled(format!("↓ {}", fmt_bytes(m.disk_read_bytes)), theme.info()),
        ])),
        rows[1],
    );
    render_history(
        f,
        theme,
        "",
        rate_samples(history, |s| s.disk_read_bytes + s.disk_write_bytes),
        fmt_bytes,
        rows[2],
    );
}

fn render_network_card(
    f: &mut Frame,
    theme: &Theme,
    m: &MetricsSnapshot,
    history: &[MetricsSnapshot],
    area: Rect,
) {
    let value = format!(
        "↑ {}  ↓ {}",
        fmt_bytes(m.net_tx_bytes),
        fmt_bytes(m.net_rx_bytes)
    );
    let inner = card_frame(f, theme, "Network", &value, theme.text, area);
    if inner.height == 0 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("throughput  ", theme.muted()),
            Span::styled(
                format!(
                    "↑ {}/s",
                    fmt_bytes(latest_rate(history, |s| s.net_tx_bytes))
                ),
                theme.success(),
            ),
            Span::raw("  "),
            Span::styled(
                format!(
                    "↓ {}/s",
                    fmt_bytes(latest_rate(history, |s| s.net_rx_bytes))
                ),
                theme.info(),
            ),
        ])),
        rows[0],
    );
    render_history(
        f,
        theme,
        "",
        rate_samples(history, |s| s.net_rx_bytes + s.net_tx_bytes),
        fmt_bytes,
        rows[1],
    );
}

/// A single-row percentage bar rendered with a borderless [`Gauge`], used
/// beneath a card's title instead of nesting another bordered box.
fn render_bar(f: &mut Frame, color: Color, pct: f64, area: Rect) {
    f.render_widget(
        Gauge::default()
            .gauge_style(Style::default().fg(color))
            .percent(pct.clamp(0.0, 100.0) as u16)
            .label(format!("{pct:.1}%")),
        area,
    );
}

/// A dedicated "History" section inside a card: a heading (with an
/// optional unit suffix, e.g. `History (MiB)`), a sparkline of `data`, and
/// a y-axis gutter on the right showing the series' max (top) and min
/// (bottom) value via `format_value`, so a graph's scale is never a
/// mystery.
fn render_history(
    f: &mut Frame,
    theme: &Theme,
    unit: &str,
    data: impl Iterator<Item = u64>,
    format_value: impl Fn(u64) -> String,
    area: Rect,
) {
    let samples: Vec<u64> = data.collect();
    if area.height == 0 {
        return;
    }

    let (heading_area, graph_area) = if area.height > 1 {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(area);
        (Some(rows[0]), rows[1])
    } else {
        (None, area)
    };

    if let Some(heading_area) = heading_area {
        let mut spans = vec![Span::styled("History", theme.heading())];
        if !unit.is_empty() {
            spans.push(Span::styled(format!(" ({unit})"), theme.muted()));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), heading_area);
    }

    if samples.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled("no data yet", theme.muted())),
            graph_area,
        );
        return;
    }

    let min = *samples.iter().min().unwrap_or(&0);
    let max = *samples.iter().max().unwrap_or(&0);
    let min_label = format_value(min);
    let max_label = format_value(max);
    let axis_width = (min_label.len().max(max_label.len()) as u16 + 1).clamp(4, 12);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(axis_width)])
        .split(graph_area);
    let (graph_area, axis_area) = (cols[0], cols[1]);

    f.render_widget(
        Sparkline::default().data(&samples).style(theme.accent()),
        graph_area,
    );

    // Axis gutter: max value pinned to the top row, min value pinned to
    // the bottom row, matching the sparkline's vertical extent.
    let axis_lines: Vec<Line> = if axis_area.height <= 1 {
        vec![Line::styled(max_label.clone(), theme.muted())]
    } else {
        let mut lines = Vec::with_capacity(axis_area.height as usize);
        lines.push(Line::styled(max_label.clone(), theme.muted()));
        for _ in 1..axis_area.height.saturating_sub(1) {
            lines.push(Line::raw(""));
        }
        lines.push(Line::styled(min_label.clone(), theme.muted()));
        lines
    };
    f.render_widget(
        Paragraph::new(axis_lines).block(Block::default().padding(Padding::new(1, 0, 0, 0))),
        axis_area,
    );
}

/// Turn a history of cumulative counters into per-sample deltas (i.e. a
/// throughput-over-time series), since the raw counters only ever grow and
/// would otherwise render as a flat, uninformative ramp.
fn rate_samples<'a>(
    history: &'a [MetricsSnapshot],
    metric: impl Fn(&MetricsSnapshot) -> u64 + 'a,
) -> impl Iterator<Item = u64> + 'a {
    history
        .windows(2)
        .map(move |pair| metric(&pair[1]).saturating_sub(metric(&pair[0])))
}

/// Most recent per-sample delta for `metric`, i.e. the current throughput.
fn latest_rate(history: &[MetricsSnapshot], metric: impl Fn(&MetricsSnapshot) -> u64) -> u64 {
    history
        .windows(2)
        .last()
        .map(|pair| metric(&pair[1]).saturating_sub(metric(&pair[0])))
        .unwrap_or(0)
}
