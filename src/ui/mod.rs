//! Top-level UI rendering dispatcher.

mod confirm_dialog;
mod create_dialog;
mod detail;
mod filesystem;
mod info;
mod layout;
mod logs;
mod sandbox_list;
mod util;
mod volumes;

use ratatui::Frame;

use crate::app::App;

/// Render the complete TUI for one frame.
pub fn render(f: &mut Frame, app: &mut App) {
    // Fill the whole screen with the theme's background/foreground first so
    // every widget that only sets a foreground color still sits on a
    // theme-correct background, independent of the terminal's own default.
    f.render_widget(
        ratatui::widgets::Block::default().style(app.theme.base_style()),
        f.area(),
    );

    let (list_area, detail_area, header_area, footer_area) = layout::split(f.area());
    app.mouse.list_area = list_area;
    app.mouse.detail_area = detail_area;

    layout::render_header(f, app, header_area);
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
    if app.confirm.is_some() {
        confirm_dialog::render(f, app, f.area());
    }
}
