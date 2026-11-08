//! objc2 bits GPUI doesn't give us. S7 grows this into the full native layer.

use gpui_kit::Window;
use objc2::msg_send;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSWindow, NSWindowSharingType};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// The NSWindow behind a GPUI window. GPUI's handle is the content NSView.
pub fn ns_window(window: &Window) -> Option<&NSWindow> {
    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
    else {
        return None;
    };
    // SAFETY: GPUI's AppKit handle is its live content view; `-window` returns the
    // NSWindow that owns it, alive as long as the GPUI `Window` we borrow.
    unsafe {
        let view = handle.ns_view.as_ptr().cast::<AnyObject>();
        let ns_window: *mut NSWindow = msg_send![view, window];
        ns_window.as_ref()
    }
}

/// Mnema never records itself: `none`, or `read-only` with developer options
/// (so screenshots of the app work). Must run before the window's first show.
/// Show without becoming key or activating the app (the permission helper must
/// never take focus from System Settings).
pub fn order_front_regardless(window: &Window) {
    if let Some(ns_window) = ns_window(window) {
        ns_window.orderFrontRegardless();
    }
}

/// Keep above normal windows (the developer window's pin and mini mode).
pub fn set_always_on_top(window: &Window, on: bool) {
    if let Some(ns_window) = ns_window(window) {
        // NSFloatingWindowLevel = 3, NSNormalWindowLevel = 0.
        ns_window.setLevel(if on { 3 } else { 0 });
    }
}

/// The content min size, changed at runtime (mini mode lowers it before shrinking).
pub fn set_min_size(window: &Window, w: f32, h: f32) {
    if let Some(ns_window) = ns_window(window) {
        ns_window.setContentMinSize(objc2_foundation::NSSize::new(w as f64, h as f64));
    }
}

pub fn apply_sharing_type(window: &Window, developer_options: bool) {
    if let Some(ns_window) = ns_window(window) {
        ns_window.setSharingType(if developer_options {
            NSWindowSharingType::ReadOnly
        } else {
            NSWindowSharingType::None
        });
    }
}
