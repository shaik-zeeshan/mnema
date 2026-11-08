//! Window chrome GPUI doesn't draw (the `windows/macos.rs` port): the
//! continuous-corner content mask and hidden traffic lights for the frameless
//! windows, and capture exclusion re-applied to every open window.

use gpui_kit::{App, Window};
use objc2::msg_send;
use objc2::runtime::AnyObject;
use objc2_app_kit::NSWindowButton;
use objc2_foundation::NSString;

/// A frameless window (`titlebar: None`): hide AppKit's leftover standard
/// buttons and round the content view with continuous corners of `radius` pt.
/// Run before the first show.
pub fn frameless(window: &Window, radius: f64) {
    let Some(ns_window) = super::ns_window(window) else {
        return;
    };
    for button in [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ] {
        if let Some(b) = ns_window.standardWindowButton(button) {
            b.setHidden(true);
        }
    }
    let Some(view) = ns_window.contentView() else {
        return;
    };
    view.setWantsLayer(true);
    // SAFETY: a layer-backed NSView's `layer` is a live CALayer (or nil, checked);
    // `setCornerRadius:` (CGFloat), `setMasksToBounds:` (BOOL) and
    // `setCornerCurve:` (NSString) all return void.
    unsafe {
        let layer: *mut AnyObject = msg_send![&*view, layer];
        let Some(layer) = layer.as_ref() else {
            return;
        };
        let _: () = msg_send![layer, setCornerRadius: radius];
        let _: () = msg_send![layer, setMasksToBounds: true];
        let _: () = msg_send![layer, setCornerCurve: &*NSString::from_str("continuous")];
    }
    // The shadow follows the content's alpha: recompute it for the round mask.
    ns_window.invalidateShadow();
}

/// Developer options changed: re-apply `sharingType` to every open window
/// (the main window is hidden, not destroyed, so creation alone isn't enough).
pub fn reapply_sharing(developer_options: bool, cx: &mut App) {
    for handle in cx.windows() {
        let _ = handle.update(cx, |_, window, _| {
            super::apply_sharing_type(window, developer_options)
        });
    }
}
