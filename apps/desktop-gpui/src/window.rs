//! The main window (§3.4): 800×600, min 640×480, transparent titlebar with the
//! traffic lights in the 78 px inset, app-owned drag, sharingType set before
//! show. Close hides it (the tray, the Dock and ⌘⌥M bring it back).

use gpui_kit::{
    point, px, size, AnyWindowHandle, App, AppContext, Bounds, Entity, Global, TitlebarOptions,
    WeakEntity, WindowBounds, WindowOptions,
};

use crate::shell::{Shell, ShellModel, Surface};

pub const MAIN_SIZE: (f32, f32) = (800., 600.);
pub const MAIN_MIN: (f32, f32) = (640., 480.);

/// The one main window, kept for the life of the app.
struct Main {
    handle: AnyWindowHandle,
    shell: WeakEntity<Shell>,
}

impl Global for Main {}

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
    let (handle, shell) = gpui_kit::open_window(main_options(cx), cx, |window, cx| {
        crate::native::apply_sharing_type(window, developer_options);
        window.on_window_should_close(cx, |window, cx| {
            crate::native::order_out(window);
            crate::native::dock::refresh(cx);
            false
        });
        window.activate_window();
        cx.new(|cx| Shell::new(model, window, cx))
    })?;
    cx.set_global(Main {
        handle: handle.into(),
        shell: shell.downgrade(),
    });
    crate::native::dock::set_regular(true, cx);
    Ok(shell)
}

pub fn shell(cx: &App) -> Option<Entity<Shell>> {
    cx.try_global::<Main>().and_then(|m| m.shell.upgrade())
}

/// Show and focus the main window; its handle, as a dialog parent.
pub fn show_main(cx: &mut App) -> Option<AnyWindowHandle> {
    let handle = cx.try_global::<Main>()?.handle;
    handle
        .update(cx, |_, window, _| window.activate_window())
        .ok()?;
    cx.activate(true);
    crate::native::dock::set_regular(true, cx);
    Some(handle)
}

/// ⌘⌥M: hide when visible and focused, else show and focus.
pub fn toggle_main(cx: &mut App) {
    let Some(handle) = cx.try_global::<Main>().map(|m| m.handle) else {
        return;
    };
    let front = handle
        .update(cx, |_, window, _| {
            crate::native::is_visible(window) && window.is_window_active()
        })
        .unwrap_or(false);
    if front {
        let _ = handle.update(cx, |_, window, _| crate::native::order_out(window));
        crate::native::dock::refresh(cx);
    } else {
        show_main(cx);
    }
}

/// Show main on Settings, optionally at a tab (`focus_main_and_open_settings`).
pub fn open_settings(tab: Option<&str>, cx: &mut App) {
    show_main(cx);
    let Some(shell) = shell(cx) else {
        return;
    };
    shell.update(cx, |shell, cx| {
        shell.go(Surface::Settings, cx);
        if let Some(tab) = tab {
            let settings = shell.settings.clone();
            settings.update(cx, |s, cx| s.open_tab(tab, cx));
        }
    });
}

/// Every display asleep (`WindowSignal::HideAllViews`).
#[derive(Default)]
struct ScreensAsleep(bool);

impl Global for ScreensAsleep {}

pub fn set_screens_asleep(asleep: bool, cx: &mut App) {
    cx.set_global(ScreensAsleep(asleep));
}

/// Nothing of the main window can be seen (hidden, or every screen asleep):
/// periodic repaints and polls skip their work.
pub fn render_idle(cx: &mut App) -> bool {
    cx.try_global::<ScreensAsleep>().is_some_and(|s| s.0) || !main_visible(cx)
}

/// Whether the main window is on screen. Unknown (no main window, or called
/// from inside its own update) reads as visible.
pub fn main_visible(cx: &mut App) -> bool {
    let Some(handle) = cx.try_global::<Main>().map(|m| m.handle) else {
        return true;
    };
    handle
        .update(cx, |_, window, _| crate::native::is_visible(window))
        .unwrap_or(true)
}
