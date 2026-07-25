//! Filesystem tab: directory listing, kind icons, size column, navigate up.

use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

use microsandbox::sandbox::SandboxStatus;

use crate::app::App;
use crate::sandbox::{FsEntry, LocalFsEntryKind};

pub fn render(f: &mut Frame, app: &mut App, area: Rect) {
    let (name, is_running) = match app.selected_sandbox() {
        Some(sb) => (sb.name.clone(), sb.status == SandboxStatus::Running),
        None => return,
    };

    if !is_running {
        f.render_widget(
            Paragraph::new(Span::styled(
                "Sandbox is not running. Start it to browse the filesystem.",
                Style::default().fg(Color::DarkGray),
            )),
            area,
        );
        return;
    }

    let fs_path = app.fs_path.clone();
    let entries = app.fs_entries.get(&(name.clone(), fs_path.clone())).cloned();

    if entries.is_none() {
        app.request_fs(&name, &fs_path);
        f.render_widget(
            Paragraph::new(Span::styled(
                "Loading filesystem…",
                Style::default().fg(Color::DarkGray),
            )),
            area,
        );
        return;
    }

    let entries = entries.unwrap_or_default();

    let path_line = Paragraph::new(Line::from(vec![
        Span::styled(" Path: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            fs_path.clone(),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "  (Backspace to go up)",
            Style::default().fg(Color::DarkGray),
        ),
    ]));

    // Split: path line(1) + table
    use ratatui::layout::{Direction, Layout};
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    f.render_widget(path_line, chunks[0]);

    if entries.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                "  (empty directory)",
                Style::default().fg(Color::DarkGray),
            )),
            chunks[1],
        );
        return;
    }

    let visible_height = chunks[1].height as usize;
    let scroll = app.fs_scroll.min(entries.len().saturating_sub(1));

    let header = Row::new(vec![
        Cell::from(Span::styled(
            "Kind",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            "Path",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            "Size",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )),
    ])
    .height(1)
    .bottom_margin(1);

    let rows: Vec<Row> = entries
        .iter()
        .skip(scroll)
        .take(visible_height.saturating_sub(2))
        .map(|e| entry_row(e))
        .collect();

    let widths = [
        Constraint::Length(4),
        Constraint::Min(10),
        Constraint::Length(10),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::NONE),
        )
        .column_spacing(1);

    f.render_widget(table, chunks[1]);
}

fn entry_row(e: &FsEntry) -> Row<'_> {
    let (icon, color) = match e.kind {
        LocalFsEntryKind::Directory => ("dir ", Color::Blue),
        LocalFsEntryKind::File => ("file", Color::White),
        LocalFsEntryKind::Symlink => ("link", Color::Cyan),
        LocalFsEntryKind::Other => ("?   ", Color::DarkGray),
    };

    let size_str = fmt_bytes(e.size);

    let path = e.path.rsplit('/').next().unwrap_or(&e.path);

    Row::new(vec![
        Cell::from(Span::styled(icon, Style::default().fg(color))),
        Cell::from(Span::styled(
            path.to_owned(),
            Style::default().fg(color).add_modifier(if e.kind == LocalFsEntryKind::Directory {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }),
        )),
        Cell::from(Span::styled(size_str, Style::default().fg(Color::DarkGray))),
    ])
}

fn fmt_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1}G", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1}M", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.0}K", bytes as f64 / 1024.0)
    } else {
        format!("{b}", b = bytes)
    }
}
