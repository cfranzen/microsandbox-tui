//! Top-level UI rendering dispatcher.

mod create_dialog;
mod detail;
mod filesystem;
mod info;
mod layout;
mod logs;
mod sandbox_list;
mod volumes;

use ratatui::Frame;

use crate::app::App;

/// Render the complete TUI for one frame.
pub fn render(f: &mut Frame, app: &mut App) {
    let (list_area, detail_area, header_area, footer_area) = layout::split(f.area());
    app.mouse.list_area = list_area;
    app.mouse.detail_area = detail_area;

    layout::render_header(f, header_area);
    sandbox_list::render(f, app, list_area);
    detail::render(f, app, detail_area);
    layout::render_footer(f, app, footer_area);

    // Modal on top of everything
    if app.create_dialog.visible {
        create_dialog::render(f, app, f.area());
    }
    if app.volumes_view.visible {
        volumes::render(f, app, f.area());
    }
}
