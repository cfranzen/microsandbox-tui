//! Top-level "Volumes" management view.
//!
//! Lists named volumes and lets the user create/remove them directly against
//! the SDK. Reached via the `v` key from the main view.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::Span,
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::app::{App, SubDialogMode};
use microsandbox::VolumeKind;

/// Render the Volumes view centred over the full terminal area.
pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let view = &app.volumes_view;
    if !view.visible {
        return;
    }

    match view.mode {
        SubDialogMode::List => render_list(f, app, area),
        SubDialogMode::Add => render_add(f, app, area),
    }
}

fn render_list(f: &mut Frame, app: &App, area: Rect) {
    let view = &app.volumes_view;
    let theme = &app.theme;

    let popup = centred_rect(70, 20, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(Span::styled(" Volumes ", theme.accent_bold()))
        .borders(Borders::ALL)
        .border_type(theme.border_unfocused_type)
        .border_style(theme.accent())
        .style(theme.base_style());

    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);

    if view.volumes.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                "  (no named volumes — press 'n' to create one)",
                theme.muted(),
            )),
            chunks[0],
        );
    } else {
        for (i, vol) in view.volumes.iter().enumerate() {
            if i as u16 >= chunks[0].height {
                break;
            }
            let is_sel = i == view.selected;
            let style = if is_sel {
                theme.selected()
            } else {
                theme.text()
            };
            let kind = match vol.kind {
                VolumeKind::Directory => "dir",
                VolumeKind::Disk => "disk",
            };
            let quota = vol
                .quota_mib
                .map(|q| format!("{q} MiB quota"))
                .unwrap_or_else(|| "unlimited".into());
            let entry = format!(
                "{:<24} {:<5} {:>10}  ({quota})",
                vol.name,
                kind,
                fmt_bytes(vol.used_bytes)
            );
            let row = Rect::new(chunks[0].x, chunks[0].y + i as u16, chunks[0].width, 1);
            f.render_widget(
                Paragraph::new(Span::styled(format!("  {entry}"), style)),
                row,
            );
        }
    }

    if let Some(ref err) = view.error {
        f.render_widget(
            Paragraph::new(Span::styled(format!("  ✗ {err}"), theme.danger())),
            chunks[1],
        );
    }

    f.render_widget(
        Paragraph::new(theme.hint_line(&[
            ("↑↓", "select"),
            ("n", "new"),
            ("d", "delete"),
            ("r", "refresh"),
            ("Esc", "close"),
        ])),
        chunks[2],
    );
}

fn render_add(f: &mut Frame, app: &App, area: Rect) {
    let view = &app.volumes_view;
    let theme = &app.theme;

    let popup = centred_rect(60, 8, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(Span::styled(" Create Volume ", theme.accent_bold()))
        .borders(Borders::ALL)
        .border_type(theme.border_unfocused_type)
        .border_style(theme.accent())
        .style(theme.base_style());

    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // name
            Constraint::Length(1), // kind summary
            Constraint::Length(1), // hint/error
        ])
        .split(inner);

    let name_block = Block::default()
        .title(" Name ")
        .borders(Borders::ALL)
        .border_style(theme.accent());
    f.render_widget(
        Paragraph::new(view.name_input.as_str()).block(name_block),
        chunks[0],
    );

    let kind_label = if view.disk { "Disk" } else { "Directory" };
    f.render_widget(
        Paragraph::new(Span::styled(
            format!("Kind: {kind_label} (space to toggle)"),
            theme.accent(),
        )),
        chunks[1],
    );

    if let Some(ref err) = view.error {
        f.render_widget(
            Paragraph::new(Span::styled(format!(" ✗ {err}"), theme.danger())),
            chunks[2],
        );
    } else {
        f.render_widget(
            Paragraph::new(theme.hint_line(&[
                ("Type", "name"),
                ("Enter", "create"),
                ("Esc", "cancel"),
            ])),
            chunks[2],
        );
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
        format!("{bytes} B")
    }
}

/// Compute a centred rect with the given percentage width and fixed height,
/// relative to `area`.
fn centred_rect(percent_width: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(height),
            Constraint::Fill(1),
        ])
        .split(area);

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_width) / 2),
            Constraint::Percentage(percent_width),
            Constraint::Percentage((100 - percent_width) / 2),
        ])
        .split(vertical[1]);

    horizontal[1]
}
