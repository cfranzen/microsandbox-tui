//! Info tab: sandbox configuration, timestamps, and live metrics
//! (CPU/memory/disk gauges with history sparklines, disk/network I/O, uptime).

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph, Sparkline},
    Frame,
};

use crate::app::App;
use crate::sandbox::MetricsSnapshot;

pub fn render(f: &mut Frame, app: &mut App, area: Rect) {
    let Some(sb) = app.selected_sandbox().cloned() else {
        return;
    };

    // Split: config/timestamp block (fixed height) + live metrics (remaining space).
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(11), Constraint::Min(0)])
        .split(area);

    render_config(f, &sb, chunks[0]);
    render_metrics(f, app, &sb, chunks[1]);
}

/// Render the general config + timestamps section (previously the "Info" tab).
fn render_config(f: &mut Frame, sb: &crate::sandbox::SandboxInfo, area: Rect) {
    let key = Style::default()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::BOLD);
    let val = Style::default().fg(Color::White);
    let heading = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);

    let age = sb
        .created_at
        .map(|t| {
            let secs = (chrono::Utc::now() - t).num_seconds().max(0) as u64;
            humantime::format_duration(std::time::Duration::from_secs(secs)).to_string()
        })
        .unwrap_or_else(|| "—".into());

    let created = sb
        .created_at
        .map(|t| t.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| "—".into());

    let updated = sb
        .updated_at
        .map(|t| t.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| "—".into());

    let status_str = format!("{:?}", sb.status);
    let status_color = match sb.status {
        microsandbox::sandbox::SandboxStatus::Running => Color::Green,
        microsandbox::sandbox::SandboxStatus::Stopped => Color::Yellow,
        microsandbox::sandbox::SandboxStatus::Crashed => Color::Red,
        _ => Color::DarkGray,
    };

    // Build strings before constructing Line values to avoid temporary lifetime issues.
    let cpus_str = sb.cpus.to_string();
    let memory_str = format!("{} MiB", sb.memory_mib);

    let lines: Vec<Line> = vec![
        Line::from(Span::styled("  General", heading)),
        Line::raw(""),
        kv("  Name       ", &sb.name, key, val),
        Line::from(vec![
            Span::styled("  Status     ", key),
            Span::styled(
                &status_str,
                Style::default()
                    .fg(status_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        kv("  Image      ", &sb.image, key, val),
        kv("  CPUs       ", &cpus_str, key, val),
        kv("  Memory     ", &memory_str, key, val),
        Line::raw(""),
        Line::from(Span::styled("  Timestamps", heading)),
        Line::raw(""),
        kv("  Created    ", &created, key, val),
        kv("  Age        ", &age, key, val),
        kv("  Updated    ", &updated, key, val),
    ];

    f.render_widget(Paragraph::new(lines), area);
}

/// Render the live metrics section (previously the "Metrics" tab).
fn render_metrics(f: &mut Frame, app: &mut App, sb: &crate::sandbox::SandboxInfo, area: Rect) {
    let name = sb.name.clone();
    let is_running = sb.status == crate::sandbox::SandboxStatus::Running;

    if !is_running {
        f.render_widget(
            Paragraph::new(Span::styled(
                "Sandbox is not running. Start it to see live metrics.",
                Style::default().fg(Color::DarkGray),
            )),
            area,
        );
        return;
    }

    let metrics = app.metrics.get(&name).cloned();

    if metrics.is_none() {
        app.request_metrics(&name);
        f.render_widget(
            Paragraph::new(Span::styled(
                "Loading metrics…",
                Style::default().fg(Color::DarkGray),
            )),
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
            Constraint::Length(1), // Disk I/O
            Constraint::Length(1), // Network I/O
            Constraint::Length(1), // Uptime
            Constraint::Min(0),    // padding
        ])
        .split(area);

    // --- CPU gauge ---
    let cpu_pct = m.cpu_percent.clamp(0.0, 100.0);
    let cpu_color = gauge_color(cpu_pct);
    f.render_widget(
        Gauge::default()
            .block(
                Block::default()
                    .title(Span::styled(" CPU ", Style::default().fg(Color::White)))
                    .borders(Borders::LEFT | Borders::TOP | Borders::RIGHT)
                    .border_style(Style::default().fg(Color::DarkGray)),
            )
            .gauge_style(Style::default().fg(cpu_color))
            .percent(cpu_pct as u16)
            .label(format!("{:.1}%", cpu_pct)),
        chunks[0],
    );

    render_sparkline(
        f,
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
    let mem_color = gauge_color(mem_pct as f64);
    f.render_widget(
        Gauge::default()
            .block(
                Block::default()
                    .title(Span::styled(" Memory ", Style::default().fg(Color::White)))
                    .borders(Borders::LEFT | Borders::TOP | Borders::RIGHT)
                    .border_style(Style::default().fg(Color::DarkGray)),
            )
            .gauge_style(Style::default().fg(mem_color))
            .percent(mem_pct as u16)
            .label(format!("{} / {} MiB", mem_used_mib, mem_total_mib)),
        chunks[2],
    );

    render_sparkline(
        f,
        " Memory history (MiB) ",
        history
            .iter()
            .map(|s| s.memory_bytes / 1_048_576)
            .collect::<Vec<_>>()
            .into_iter(),
        chunks[3],
    );

    // --- Disk usage gauge (writable overlay) ---
    render_disk_usage(f, &m, chunks[4]);

    // --- Disk I/O ---
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Disk I/O  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!(
                    "↑ {}  ↓ {}",
                    fmt_bytes(m.disk_write_bytes),
                    fmt_bytes(m.disk_read_bytes)
                ),
                Style::default().fg(Color::White),
            ),
        ])),
        chunks[5],
    );

    // --- Network I/O ---
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Net       ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!(
                    "↑ {}  ↓ {}",
                    fmt_bytes(m.net_tx_bytes),
                    fmt_bytes(m.net_rx_bytes)
                ),
                Style::default().fg(Color::White),
            ),
        ])),
        chunks[6],
    );

    // --- Uptime ---
    let uptime_str =
        humantime::format_duration(std::time::Duration::from_secs(m.uptime_secs)).to_string();

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Uptime    ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!(" {}", uptime_str),
                Style::default().fg(Color::White),
            ),
        ])),
        chunks[7],
    );
}

/// Render the writable-overlay disk usage gauge when the SDK reports it.
/// Not every backend/config surfaces this, so we fall back to a plain hint.
fn render_disk_usage(f: &mut Frame, m: &MetricsSnapshot, area: Rect) {
    let block = Block::default()
        .title(Span::styled(
            " Disk usage ",
            Style::default().fg(Color::White),
        ))
        .borders(Borders::LEFT | Borders::TOP | Borders::RIGHT)
        .border_style(Style::default().fg(Color::DarkGray));

    match (m.disk_used_bytes, m.disk_free_bytes) {
        (Some(used), Some(free)) => {
            let total = used + free;
            let pct = used
                .checked_mul(100)
                .and_then(|v| v.checked_div(total))
                .unwrap_or(0)
                .min(100);
            let color = gauge_color(pct as f64);
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
                Paragraph::new(Span::styled(
                    " not reported by this sandbox",
                    Style::default().fg(Color::DarkGray),
                ))
                .block(block),
                area,
            );
        }
    }
}

fn gauge_color(pct: f64) -> Color {
    if pct >= 90.0 {
        Color::Red
    } else if pct >= 70.0 {
        Color::Yellow
    } else {
        Color::Green
    }
}

/// Render a labelled sparkline of recent samples for one metric.
fn render_sparkline(f: &mut Frame, title: &str, data: impl Iterator<Item = u64>, area: Rect) {
    let samples: Vec<u64> = data.collect();
    let block = Block::default()
        .title(Span::styled(
            title.to_owned(),
            Style::default().fg(Color::DarkGray),
        ))
        .borders(Borders::LEFT | Borders::RIGHT);

    if samples.is_empty() {
        f.render_widget(block, area);
        return;
    }

    f.render_widget(
        Sparkline::default()
            .block(block)
            .data(&samples)
            .style(Style::default().fg(Color::Cyan)),
        area,
    );
}

fn fmt_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GiB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MiB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

fn kv<'a>(label: &'a str, value: &'a str, key_style: Style, val_style: Style) -> Line<'a> {
    Line::from(vec![
        Span::styled(label.to_owned(), key_style),
        Span::styled(value.to_owned(), val_style),
    ])
}
