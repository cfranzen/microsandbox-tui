//! Metrics tab: live CPU/memory/disk gauges with history sparklines, plus
//! disk and network I/O throughput graphs.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::Span,
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

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // CPU gauge
            Constraint::Length(4), // CPU history sparkline
            Constraint::Length(3), // Memory gauge
            Constraint::Length(4), // Memory history sparkline
            Constraint::Length(3), // Disk usage gauge (writable overlay)
            Constraint::Length(4), // Disk I/O history sparkline
            Constraint::Length(4), // Network I/O history sparkline
            Constraint::Min(0),    // padding
        ])
        .split(area);

    // --- CPU gauge ---
    let cpu_pct = m.cpu_percent.clamp(0.0, 100.0);
    let cpu_color = theme.gauge_color(cpu_pct);
    f.render_widget(
        Gauge::default()
            .block(
                Block::default()
                    .title(Span::styled(" CPU ", theme.text()))
                    .borders(Borders::ALL)
                    .border_style(theme.muted()),
            )
            .gauge_style(Style::default().fg(cpu_color))
            .percent(cpu_pct as u16)
            .label(format!("{:.1}%", cpu_pct)),
        chunks[0],
    );

    render_sparkline(
        f,
        &theme,
        " CPU history ",
        history
            .iter()
            .map(|s| s.cpu_percent.clamp(0.0, 100.0) as u64),
        chunks[1],
    );

    // --- Memory gauge ---
    let mem_total_mib = sb.memory_mib as u64;
    let mem_used_mib = m.memory_bytes / 1_048_576;
    let mem_pct = mem_used_mib
        .checked_mul(100)
        .and_then(|v| v.checked_div(mem_total_mib))
        .unwrap_or(0)
        .min(100);
    let mem_color = theme.gauge_color(mem_pct as f64);
    f.render_widget(
        Gauge::default()
            .block(
                Block::default()
                    .title(Span::styled(" Memory ", theme.text()))
                    .borders(Borders::ALL)
                    .border_style(theme.muted()),
            )
            .gauge_style(Style::default().fg(mem_color))
            .percent(mem_pct as u16)
            .label(format!("{} / {} MiB", mem_used_mib, mem_total_mib)),
        chunks[2],
    );

    render_sparkline(
        f,
        &theme,
        " Memory history (MiB) ",
        history
            .iter()
            .map(|s| s.memory_bytes / 1_048_576)
            .collect::<Vec<_>>()
            .into_iter(),
        chunks[3],
    );

    // --- Disk usage gauge (writable overlay) ---
    render_disk_usage(f, &theme, &m, chunks[4]);

    // --- Disk I/O history (read+write bytes/sample, i.e. throughput) ---
    let disk_title = format!(
        " Disk I/O   ↑ {}  ↓ {} ",
        fmt_bytes(m.disk_write_bytes),
        fmt_bytes(m.disk_read_bytes)
    );
    render_sparkline(
        f,
        &theme,
        &disk_title,
        rate_samples(&history, |s| s.disk_read_bytes + s.disk_write_bytes),
        chunks[5],
    );

    // --- Network I/O history (rx+tx bytes/sample, i.e. throughput) ---
    let net_title = format!(
        " Net I/O    ↑ {}  ↓ {} ",
        fmt_bytes(m.net_tx_bytes),
        fmt_bytes(m.net_rx_bytes)
    );
    render_sparkline(
        f,
        &theme,
        &net_title,
        rate_samples(&history, |s| s.net_rx_bytes + s.net_tx_bytes),
        chunks[6],
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

/// Render the writable-overlay disk usage gauge when the SDK reports it.
/// Not every backend/config surfaces this, so we fall back to a plain hint.
fn render_disk_usage(f: &mut Frame, theme: &Theme, m: &MetricsSnapshot, area: Rect) {
    let block = Block::default()
        .title(Span::styled(" Disk usage ", theme.text()))
        .borders(Borders::ALL)
        .border_style(theme.muted());

    match (m.disk_used_bytes, m.disk_free_bytes) {
        (Some(used), Some(free)) => {
            let total = used + free;
            let pct = used
                .checked_mul(100)
                .and_then(|v| v.checked_div(total))
                .unwrap_or(0)
                .min(100);
            let color = theme.gauge_color(pct as f64);
            f.render_widget(
                Gauge::default()
                    .block(block)
                    .gauge_style(Style::default().fg(color))
                    .percent(pct as u16)
                    .label(format!("{} / {} used", fmt_bytes(used), fmt_bytes(total))),
                area,
            );
        }
        _ => {
            f.render_widget(
                Paragraph::new(Span::styled(" not reported by this sandbox", theme.muted()))
                    .block(block),
                area,
            );
        }
    }
}

/// Render a labelled sparkline of recent samples for one metric.
fn render_sparkline(
    f: &mut Frame,
    theme: &Theme,
    title: &str,
    data: impl Iterator<Item = u64>,
    area: Rect,
) {
    let samples: Vec<u64> = data.collect();
    let block = Block::default()
        .title(Span::styled(title.to_owned(), theme.muted()))
        .borders(Borders::LEFT | Borders::RIGHT);

    if samples.is_empty() {
        f.render_widget(block, area);
        return;
    }

    f.render_widget(
        Sparkline::default()
            .block(block)
            .data(&samples)
            .style(theme.accent()),
        area,
    );
}
