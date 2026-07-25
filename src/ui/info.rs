//! Info tab: full sandbox configuration dump.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::app::App;

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let Some(sb) = app.selected_sandbox() else {
        return;
    };

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

fn kv<'a>(label: &'a str, value: &'a str, key_style: Style, val_style: Style) -> Line<'a> {
    Line::from(vec![
        Span::styled(label.to_owned(), key_style),
        Span::styled(value.to_owned(), val_style),
    ])
}
