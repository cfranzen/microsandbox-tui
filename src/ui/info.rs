//! Info tab: sandbox configuration and timestamps.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Padding, Paragraph},
    Frame,
};

use crate::app::App;
use crate::sandbox::MetricsSnapshot;
use crate::theme::Theme;

use super::util::fmt_bytes;

/// Left column width for key labels, e.g. `"Name"` padded to
/// `"Name      "`. Centralized here so every row lines up without
/// hand-counted spaces baked into each label string.
const LABEL_WIDTH: usize = 12;

pub fn render(f: &mut Frame, app: &mut App, area: Rect) {
    let Some(sb) = app.selected_sandbox().cloned() else {
        return;
    };

    // Best-effort: if metrics for this sandbox were already fetched (e.g.
    // the user visited the Metrics tab), reuse the cached snapshot to show
    // uptime/disk capacity here too, without forcing a fetch from the Info
    // tab.
    let metrics = app.metrics.get(&sb.name).cloned();

    // A single 1-character padded block gives the whole section consistent
    // left/right/top margins instead of hand-inserted blanks in each label.
    let block = Block::default().padding(Padding::new(1, 1, 1, 0));
    let inner = block.inner(area);
    f.render_widget(block, area);

    render_config(f, &app.theme, &sb, metrics.as_ref(), inner);
}

/// Render the General + Timestamps sections.
fn render_config(
    f: &mut Frame,
    theme: &Theme,
    sb: &crate::sandbox::SandboxInfo,
    metrics: Option<&MetricsSnapshot>,
    area: Rect,
) {
    let key = theme.muted().add_modifier(Modifier::BOLD);
    let val = theme.text();
    let heading = theme.heading();

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

    // "Age" (time since creation) and "Uptime" (time since the sandbox's
    // guest process last started) both look like durations, so make each
    // row's meaning explicit and keep uptime with the other timestamps.
    let uptime = if sb.status == crate::sandbox::SandboxStatus::Running {
        metrics
            .map(|m| {
                humantime::format_duration(std::time::Duration::from_secs(m.uptime_secs))
                    .to_string()
            })
            .unwrap_or_else(|| "—".into())
    } else {
        "not running".into()
    };

    let status_str = format!("{:?}", sb.status);
    let status_color = theme.status_color(sb.status);

    // Build strings before constructing Line values to avoid temporary lifetime issues.
    let cpus_str = sb.cpus.to_string();
    let memory_str = format!("{} MiB", sb.memory_mib);
    let workdir_str = sb.workdir.clone().unwrap_or_else(|| "—".into());
    let disk_capacity_str = metrics
        .and_then(|m| match (m.disk_used_bytes, m.disk_free_bytes) {
            (Some(used), Some(free)) => Some(fmt_bytes(used + free)),
            _ => None,
        })
        .unwrap_or_else(|| "—".into());

    let lines: Vec<Line> = vec![
        Line::from(Span::styled("General", heading)),
        Line::raw(""),
        kv("Name", &sb.name, key, val),
        Line::from(vec![
            Span::styled(label("Status"), key),
            Span::styled(
                &status_str,
                Style::default()
                    .fg(status_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        kv("Image", &sb.image, key, val),
        kv("Working dir", &workdir_str, key, val),
        kv("CPUs", &cpus_str, key, val),
        kv("Memory", &memory_str, key, val),
        kv("Max disk", &disk_capacity_str, key, val),
        Line::raw(""),
        Line::from(Span::styled("Timestamps", heading)),
        Line::raw(""),
        kv("Created", &created, key, val),
        kv("Updated", &updated, key, val),
        kv("Age", &age, key, val),
        kv("Uptime", &uptime, key, val),
    ];

    f.render_widget(Paragraph::new(lines), area);
}

/// Build a `"Label      value"` row with the label padded to
/// [`LABEL_WIDTH`] columns.
fn kv<'a>(label_text: &'a str, value: &'a str, key_style: Style, val_style: Style) -> Line<'a> {
    Line::from(vec![
        Span::styled(label(label_text), key_style),
        Span::styled(value.to_owned(), val_style),
    ])
}

fn label(text: &str) -> String {
    format!("{text:<LABEL_WIDTH$}")
}
