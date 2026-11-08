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
pub fn apply_sharing_type(window: &Window, developer_options: bool) {
    if let Some(ns_window) = ns_window(window) {
        ns_window.setSharingType(if developer_options {
            NSWindowSharingType::ReadOnly
        } else {
            NSWindowSharingType::None
        });
    }
}
