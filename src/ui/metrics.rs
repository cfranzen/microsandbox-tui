//! Metrics tab: CPU bar, memory bar, disk I/O, network I/O, uptime.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph},
    Frame,
};

use crate::app::App;

pub fn render(f: &mut Frame, app: &mut App, area: Rect) {
    let name = match app.selected_sandbox() {
        Some(sb) => sb.name.clone(),
        None => return,
    };

    let Some(sb_info) = app.selected_sandbox().cloned() else {
        return;
    };

    let is_running = sb_info.status == crate::sandbox::SandboxStatus::Running;

    if !is_running {
        f.render_widget(
            Paragraph::new(Span::styled(
                "Sandbox is not running. Start it to see metrics.",
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

    // Layout: 8 rows of metric widgets
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // CPU gauge
            Constraint::Length(1), // spacer
            Constraint::Length(3), // Memory gauge
            Constraint::Length(1), // spacer
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

    // --- Memory gauge ---
    let mem_total_mib = sb_info.memory_mib as u64;
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

    // --- Disk I/O ---
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Disk  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!(
                    "↑ {}  ↓ {}",
                    fmt_bytes(m.disk_write_bytes),
                    fmt_bytes(m.disk_read_bytes)
                ),
                Style::default().fg(Color::White),
            ),
        ])),
        chunks[4],
    );

    // --- Network I/O ---
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Net   ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!(
                    "↑ {}  ↓ {}",
                    fmt_bytes(m.net_tx_bytes),
                    fmt_bytes(m.net_rx_bytes)
                ),
                Style::default().fg(Color::White),
            ),
        ])),
        chunks[5],
    );

    // --- Uptime ---
    let uptime_str =
        humantime::format_duration(std::time::Duration::from_secs(m.uptime_secs)).to_string();

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Uptime", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!(" {}", uptime_str),
                Style::default().fg(Color::White),
            ),
        ])),
        chunks[6],
    );
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
