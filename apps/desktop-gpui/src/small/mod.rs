//! The small windows (BUILD-PROMPT §3.4 / §3.6): CLI Access request, Update,
//! the permission helper, and Developer (+ mini). Each is its own GPUI window:
//! no native chrome (the shared `WindowStrip` instead), a transparent background
//! under a 12 px-corner frame, sharingType set before the first show. One of
//! each at a time: opening an open one brings it forward.

pub mod access;
pub mod debug;
pub mod helper;
pub mod parts;
pub mod update;

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use app_core::{WindowRequest, WindowSignal};
use gpui_kit::base::Root;
use gpui_kit::prelude::*;
use gpui_kit::{
    point, px, size, transparent_black, AnyWindowHandle, App, AppContext, Bounds, Entity, Global,
    Render, WeakEntity, Window, WindowBackgroundAppearance, WindowBounds, WindowKind,
    WindowOptions,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Access,
    Update,
    Helper,
    Debug,
}

/// The open small windows, so a second request reuses the first.
#[derive(Default)]
struct Open(HashMap<Kind, (AnyWindowHandle, gpui_kit::AnyWeakEntity)>);

impl Global for Open {}

/// The permission helper's gap above the work area's bottom edge (`mx-helper { bottom: 28px }`).
pub const HELPER_BOTTOM_GAP: f32 = 28.;

/// Options for a small window of `win` points (min `min`) on the primary display:
/// centred, or (the helper) bottom-centre 28 pt above the work area.
pub fn options(win: (f32, f32), min: (f32, f32), kind: WindowKind, cx: &App) -> WindowOptions {
    let s = size(px(win.0), px(win.1));
    let origin = cx
        .primary_display()
        .map(|d| {
            if kind == WindowKind::PopUp {
                let a = d.visible_bounds();
                return point(
                    a.origin.x + (a.size.width - s.width) / 2.,
                    a.origin.y + a.size.height - s.height - px(HELPER_BOTTOM_GAP),
                );
            }
            let a = d.bounds();
            point(
                a.origin.x + (a.size.width - s.width) / 2.,
                a.origin.y + (a.size.height - s.height) / 2.,
            )
        })
        .unwrap_or(point(px(160.), px(160.)));
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(origin, s))),
        // No titlebar: `native::chrome::frameless` hides AppKit's leftover buttons
        // and rounds the content (12 px; the helper 8 px) before the first show.
        titlebar: None,
        app_owns_titlebar_drag: true,
        window_background: WindowBackgroundAppearance::Transparent,
        window_min_size: Some(size(px(min.0), px(min.1))),
        is_minimizable: false,
        kind,
        focus: false,
        show: false,
        ..Default::default()
    }
}

/// Bring the open `kind` window forward (calling `reuse`), or open a new one
/// from `build`. `build` runs inside the new window, before its first show.
pub fn open_or_focus<V: Render + 'static>(
    kind: Kind,
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
    reuse: impl FnOnce(Entity<V>, &mut Window, &mut App),
) -> Option<Entity<V>> {
    let existing = cx
        .try_global::<Open>()
        .and_then(|o| o.0.get(&kind).cloned());
    if let Some((handle, weak)) = existing {
        if let Some(view) = weak.upgrade().and_then(|v| v.downcast::<V>().ok()) {
            let v = view.clone();
            if handle
                .update(cx, |_, window, cx| {
                    reuse(v, window, cx);
                    if kind != Kind::Helper {
                        window.activate_window();
                    }
                })
                .is_ok()
            {
                return Some(view);
            }
        }
    }
    let (win, min) = match kind {
        Kind::Access => (access::SIZE, access::MIN),
        Kind::Update => (update::SIZE, update::MIN),
        Kind::Helper => (helper::SIZE, helper::SIZE),
        Kind::Debug => (debug::SIZE, debug::MIN),
    };
    let wkind = if kind == Kind::Helper {
        WindowKind::PopUp
    } else {
        WindowKind::Normal
    };
    let developer = crate::shell::developer_options(cx);
    let mut built: Option<Entity<V>> = None;
    let opened = cx.open_window(options(win, min, wkind, cx), |window, cx| {
        crate::native::apply_sharing_type(window, developer);
        crate::native::chrome::frameless(window, if kind == Kind::Helper { 8. } else { 12. });
        let view = build(window, cx);
        built = Some(view.clone());
        cx.new(|cx| Root::new(view, window, cx).bg(transparent_black()))
    });
    let (handle, view) = match (opened, built) {
        (Ok(handle), Some(view)) => (handle, view),
        (Err(e), _) => {
            log::error!("small window {kind:?} failed to open: {e:#}");
            return None;
        }
        _ => return None,
    };
    let any: AnyWindowHandle = handle.into();
    let _ = any.update(cx, |_, window, _| {
        if kind == Kind::Helper {
            crate::native::order_front_regardless(window);
        } else {
            window.activate_window();
        }
    });
    let weak: WeakEntity<V> = view.downgrade();
    cx.default_global::<Open>()
        .0
        .insert(kind, (any, weak.into()));
    // Shown (its show is queued, so a visibility read would still say no).
    crate::native::dock::set_regular(true, cx);
    Some(view)
}

/// Close the `kind` window if it is open (no "closed" answer: `remove_window`).
pub fn close(kind: Kind, cx: &mut App) {
    let Some((handle, _)) = cx
        .try_global::<Open>()
        .and_then(|o| o.0.get(&kind).cloned())
    else {
        return;
    };
    let _ = handle.update(cx, |_, window, _| window.remove_window());
    cx.default_global::<Open>().0.remove(&kind);
}

/// `Host::open_window` for the small windows. `false` = not one of ours.
pub fn open_request(req: &WindowRequest, cx: &mut App) -> bool {
    match req {
        WindowRequest::CliAccessRequest => access::open(cx),
        WindowRequest::Update => update::open(cx),
        WindowRequest::PermissionHelper { kind } => helper::open(kind, cx),
        WindowRequest::Main | WindowRequest::Settings { .. } => return false,
    }
    true
}

/// `Host::window_signal` for the small windows. `false` = not one of ours.
pub fn signal(signal: &WindowSignal, cx: &mut App) -> bool {
    match signal {
        // The request was answered: tear the window down without it reading as a dismissal.
        WindowSignal::CloseCliAccessRequest => close(Kind::Access, cx),
        _ => return false,
    }
    true
}
