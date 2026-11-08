//! objc2 bits GPUI doesn't give us: the NSWindow behind a GPUI window, sharingType,
//! panel facts, order-out, and the `terminate:` swizzle (ported from objc 0.2 in
//! `apps/desktop/src-tauri/src/windows.rs`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use gpui_kit::Window;
use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2::{msg_send, sel};
use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior, NSWindowSharingType, NSWindowStyleMask};
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

/// `none` (default) or `readonly` via `SPIKE_SHARING=readonly`, so screenshots work.
pub fn sharing_type_from_env() -> NSWindowSharingType {
    match std::env::var("SPIKE_SHARING").as_deref() {
        Ok("readonly") => NSWindowSharingType::ReadOnly,
        _ => NSWindowSharingType::None,
    }
}

/// Must run before the window is first ordered front.
pub fn apply_sharing_type(window: &Window) -> Option<NSWindowSharingType> {
    let ns_window = ns_window(window)?;
    ns_window.setSharingType(sharing_type_from_env());
    Some(ns_window.sharingType())
}

pub fn order_out(window: &Window) {
    if let Some(ns_window) = ns_window(window) {
        ns_window.orderOut(None);
    }
}

pub fn is_visible(window: &Window) -> bool {
    ns_window(window).is_some_and(|w| w.isVisible())
}

/// What §3.4 asks of the Quick Recall panel, read back from AppKit.
pub fn panel_report(window: &Window) -> String {
    let Some(w) = ns_window(window) else {
        return "no NSWindow".into();
    };
    let class = w.class().name().to_string_lossy().into_owned();
    let is_panel = AnyClass::get(c"NSPanel").is_some_and(|panel| {
        // SAFETY: `isKindOfClass:` takes a Class and returns BOOL.
        unsafe { msg_send![w, isKindOfClass: panel] }
    });
    let mask = w.styleMask();
    let behavior = w.collectionBehavior();
    format!(
        "class={class} is_panel={is_panel} non_activating={} level={} floating_or_higher={} all_spaces={} fullscreen_aux={} can_become_key={} sharing_type={}",
        mask.contains(NSWindowStyleMask::NonactivatingPanel),
        w.level(),
        w.level() >= 3,
        behavior.contains(NSWindowCollectionBehavior::CanJoinAllSpaces),
        behavior.contains(NSWindowCollectionBehavior::FullScreenAuxiliary),
        w.canBecomeKeyWindow(),
        w.sharingType().0,
    )
}

pub fn window_number(window: &Window) -> isize {
    ns_window(window).map_or(-1, |w| w.windowNumber())
}

static ORIGINAL_TERMINATE: OnceLock<usize> = OnceLock::new();
static SHUTDOWN_DONE: AtomicBool = AtomicBool::new(false);
static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);
static QUIT_TX: OnceLock<async_channel::Sender<()>> = OnceLock::new();

type TerminateFn = unsafe extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject);

unsafe extern "C-unwind" fn terminate(app: &AnyObject, cmd: Sel, sender: *mut AnyObject) {
    if SHUTDOWN_DONE.load(Ordering::SeqCst) {
        crate::spike::log("terminate: shutdown done -> original NSApplication terminate:");
        if let Some(&imp) = ORIGINAL_TERMINATE.get() {
            // SAFETY: `imp` is NSApplication's own `terminate:` IMP saved at install.
            unsafe {
                let original: TerminateFn = std::mem::transmute(imp);
                original(app, cmd, sender);
            }
        }
        return;
    }
    if !SHUTDOWN_REQUESTED.swap(true, Ordering::SeqCst) {
        crate::spike::log("terminate: intercepted, starting graceful shutdown");
        if let Some(tx) = QUIT_TX.get() {
            let _ = tx.try_send(());
        }
    } else {
        crate::spike::log("terminate: shutdown already running, ignoring");
    }
}

/// Swizzle `-[NSApplication terminate:]` so every quit path (⌘Q, Dock, tray, Apple
/// event, `cx.quit()`) asks us first. Returns the receiver of quit requests.
pub fn install_terminate_handler() -> Option<async_channel::Receiver<()>> {
    let (tx, rx) = async_channel::unbounded();
    QUIT_TX.set(tx).ok()?;
    let method = AnyClass::get(c"NSApplication")?.instance_method(sel!(terminate:))?;
    // SAFETY: `terminate` has the exact `terminate:` signature (self, _cmd, sender) -> void.
    let previous = unsafe {
        let imp: Imp = std::mem::transmute(terminate as TerminateFn);
        method.set_implementation(imp)
    };
    let _ = ORIGINAL_TERMINATE.set(previous as usize);
    crate::spike::log("installed NSApplication terminate: swizzle");
    Some(rx)
}

/// After the graceful shutdown: the next `terminate:` runs AppKit's own.
pub fn mark_shutdown_done() {
    SHUTDOWN_DONE.store(true, Ordering::SeqCst);
}

/// Test hook: pops `ns_menu` (a muda NSMenu) up at a screen point, as a click on the
/// status item would. Modal: returns once an item is chosen or the menu is dismissed.
pub fn pop_up_menu(ns_menu: *mut std::ffi::c_void, x: f64, y: f64) {
    let menu = ns_menu.cast::<AnyObject>();
    if menu.is_null() {
        return;
    }
    let at = objc2_foundation::NSPoint::new(x, y);
    // SAFETY: muda hands out its live NSMenu; popUpMenuPositioningItem:atLocation:inView:
    // takes (NSMenuItem?, NSPoint, NSView?) and returns BOOL.
    let _: bool = unsafe {
        msg_send![menu, popUpMenuPositioningItem: std::ptr::null_mut::<AnyObject>(), atLocation: at, inView: std::ptr::null_mut::<AnyObject>()]
    };
}
