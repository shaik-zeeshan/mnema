//! The main window (§3.4): 800×600, min 640×480, transparent titlebar with the
//! traffic lights in the 78 px inset, app-owned drag, sharingType set before show.

use gpui_kit::{
    point, px, size, App, AppContext, Bounds, Entity, TitlebarOptions, WindowBounds, WindowOptions,
};

use crate::shell::{Shell, ShellModel};

pub const MAIN_SIZE: (f32, f32) = (800., 600.);
pub const MAIN_MIN: (f32, f32) = (640., 480.);

pub fn main_options(cx: &App) -> WindowOptions {
    let win = size(px(MAIN_SIZE.0), px(MAIN_SIZE.1));
    let origin = cx
        .primary_display()
        .map(|d| {
            let a = d.bounds();
            point(
                a.origin.x + (a.size.width - win.width) / 2.,
                a.origin.y + (a.size.height - win.height) / 2.,
            )
        })
        .unwrap_or(point(px(120.), px(120.)));
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(origin, win))),
        titlebar: Some(TitlebarOptions {
            title: Some("Mnema".into()),
            appears_transparent: true,
            // Traffic lights vertically centred in the 38 px bar.
            traffic_light_position: Some(point(px(14.), px(12.))),
        }),
        app_owns_titlebar_drag: true,
        window_min_size: Some(size(px(MAIN_MIN.0), px(MAIN_MIN.1))),
        focus: true,
        show: false,
        ..Default::default()
    }
}

/// Open the main window hidden, set its sharingType, then show it.
pub fn open_main(model: ShellModel, cx: &mut App) -> anyhow::Result<Entity<Shell>> {
    let developer_options = model.developer_options;
    let (_, shell) = gpui_kit::open_window(main_options(cx), cx, |window, cx| {
        crate::native::apply_sharing_type(window, developer_options);
        // shortcut: close = quit until S7 adds the tray / Dock reopen (then close = hide).
        window.on_window_should_close(cx, |_, cx| {
            cx.quit();
            true
        });
        window.activate_window();
        cx.new(|cx| Shell::new(model, window, cx))
    })?;
    Ok(shell)
}
