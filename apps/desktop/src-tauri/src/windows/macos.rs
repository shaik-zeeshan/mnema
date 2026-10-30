// The AppKit glue every Mnema window goes through: capture exclusion
// (`sharingType`), the rounded content view, and `with_wk_webview`, the
// leak-free way to reach a window's WKWebView.

use tauri::{Manager, WebviewWindow};

use crate::native_capture;

/// Hide one of Mnema's own windows from every screen capture, including Mnema's.
///
/// Without this the app records itself: Quick Recall's results land back in the
/// index they came from, and an Ask AI answer on screen is OCR'd, indexed, and
/// later retrieved by the model's own `search` tool as observed history.
///
/// `NSWindow.sharingType = .none` rather than injecting our own bundle id into
/// the ScreenCaptureKit privacy filter: that would force the app-excluding
/// filter constructor permanently for every user, which ADR 0006 measured at
/// ~867 mW against ~649 mW unfiltered. This costs nothing at capture time.
///
/// Exclusion is the default and developer options is the only way off it: UI
/// work needs screenshots, and a `.none` window is invisible to ⌘⇧4, CleanShot
/// and screen sharing alike, not just to Mnema's own recorder.
///
/// ponytail: reuses `developer_options_enabled` instead of carrying a setting of
/// its own — enabling developer options therefore also makes Mnema
/// screenshottable AND self-recordable. Split it out if that ever surprises
/// someone.
#[allow(deprecated, unexpected_cfgs)]
fn apply_macos_capture_exclusion(window: &WebviewWindow) {
    use cocoa::base::id;
    use objc::{msg_send, sel, sel_impl};

    // A missing or poisoned settings state reads as "developer options off":
    // the window stays excluded, never accidentally recordable.
    let developer_options_enabled = window
        .app_handle()
        .try_state::<native_capture::RecordingSettingsState>()
        .and_then(|state| {
            state
                .lock()
                .ok()
                .map(|runtime| runtime.settings.developer_options_enabled)
        })
        .unwrap_or(false);

    let Ok(ns_window) = window.ns_window() else {
        return;
    };

    // NSWindowSharingNone / NSWindowSharingReadOnly (AppKit's default). Set
    // both ways so a developer-options toggle applies to already-open windows.
    const NS_WINDOW_SHARING_NONE: u64 = 0;
    const NS_WINDOW_SHARING_READ_ONLY: u64 = 1;
    let sharing_type = if developer_options_enabled {
        NS_WINDOW_SHARING_READ_ONLY
    } else {
        NS_WINDOW_SHARING_NONE
    };

    // SAFETY: `ns_window()` succeeded, so the pointer is this window's live
    // NSWindow, which `window` keeps alive for the call. `setSharingType:`
    // takes an NSWindowSharingType (NSUInteger, u64 on 64-bit) and returns
    // void, matching the message sent here.
    unsafe {
        let ns_window = ns_window as id;
        let _: () = msg_send![ns_window, setSharingType: sharing_type];
    }
}

/// Re-apply capture exclusion to every open window after developer options
/// changes; the main window is hidden on close, not destroyed, so creation
/// alone would leave it on the old setting until relaunch.
pub fn refresh_capture_exclusion(app_handle: &tauri::AppHandle) {
    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
        for window in handle.webview_windows().values() {
            apply_macos_capture_exclusion(window);
        }
    });
}

/// The single seam every window creation path ends on. Anything that builds a
/// `WebviewWindow` without coming through here is skipping capture exclusion.
///
/// The AppKit calls run on the main thread: the CLI access request and Update
/// windows are built on tokio workers. `run_on_main_thread` runs inline when
/// already there; off it, the block joins the same event-loop queue as the
/// window's own creation and every later `show()`/setter, so it runs after the
/// window exists and before any show the caller issues next. Callers must build
/// the window hidden for that ordering to keep it unrecordable. Never blocks,
/// so a main thread waiting on the caller can't deadlock it.
pub(crate) fn finish_macos_window(window: &WebviewWindow, corner_radius: Option<f64>) {
    let target = window.clone();
    let _ = window.run_on_main_thread(move || {
        apply_macos_capture_exclusion(&target);
        if let Some(radius) = corner_radius {
            apply_macos_rounded_content_view(&target, radius);
        }
    });
}

#[allow(deprecated, unexpected_cfgs)]
fn apply_macos_rounded_content_view(window: &WebviewWindow, radius: f64) {
    use cocoa::base::{id, nil};
    use cocoa::foundation::NSString;
    use objc::{msg_send, sel, sel_impl};

    let Ok(ns_win) = window.ns_window() else {
        return;
    };

    unsafe {
        let ns_win = ns_win as id;
        let content_view: id = msg_send![ns_win, contentView];

        if content_view == nil {
            return;
        }

        let _: () = msg_send![content_view, setWantsLayer: true];

        let layer: id = msg_send![content_view, layer];
        if layer == nil {
            return;
        }

        let _: () = msg_send![layer, setCornerRadius: radius];
        let _: () = msg_send![layer, setMasksToBounds: true];

        let continuous = NSString::alloc(nil).init_str("continuous");
        let _: () = msg_send![layer, setCornerCurve: continuous];
    }
}

/// Run `f` on the main thread with the window's WKWebView and its NSWindow
/// (both non-null). Returns whether `f` was queued (or run, on the main thread).
///
/// Plain `with_webview` leaks: tauri-runtime-wry 2.10.1 (`lib.rs:3859-3867`)
/// hands the closure three pointers made by `Retained::into_raw` from fresh +1
/// retains (wry 0.54.4 `lib.rs:2447-2457`: `webview.clone()`,
/// `manager.clone()`, `webview.window()`), and nothing on the tauri side
/// (`PlatformWebview` has no `Drop`) ever releases them. So every call kept
/// the WKWebView, its user-content controller and the NSWindow alive one
/// retain longer. This releases exactly those three after `f`.
#[allow(deprecated, unexpected_cfgs)]
pub(crate) fn with_wk_webview(
    window: &WebviewWindow,
    f: impl FnOnce(cocoa::base::id, cocoa::base::id) + Send + 'static,
) -> bool {
    use cocoa::base::id;
    use objc::{msg_send, sel, sel_impl};
    fn release(ptr: id) {
        // SAFETY: each pointer is one `Retained::into_raw` +1 that the
        // `with_webview` closure owns (see above); released once, after `f`,
        // while the window still holds its own references.
        unsafe {
            let _: () = msg_send![ptr, release];
        }
    }
    window
        .with_webview(move |webview| {
            let owned = [
                webview.inner() as id,
                webview.controller() as id,
                webview.ns_window() as id,
            ];
            run_then_release(owned, release, |[wv, _, ns_window]| {
                if !wv.is_null() && !ns_window.is_null() {
                    f(wv, ns_window);
                }
            });
        })
        .is_ok()
}

/// Calls `f` with `owned`, then `release`s every non-null pointer once, from a
/// drop guard so a panic in `f` still releases them.
#[allow(deprecated)]
fn run_then_release(
    owned: [cocoa::base::id; 3],
    release: fn(cocoa::base::id),
    f: impl FnOnce([cocoa::base::id; 3]),
) {
    struct Guard([cocoa::base::id; 3], fn(cocoa::base::id));
    impl Drop for Guard {
        fn drop(&mut self) {
            for ptr in self.0 {
                if !ptr.is_null() {
                    (self.1)(ptr);
                }
            }
        }
    }
    let _guard = Guard(owned, release);
    f(owned);
}

#[cfg(test)]
mod tests {
    #[test]
    #[allow(deprecated)]
    fn with_wk_webview_releases_its_retains_even_when_the_closure_panics() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static RELEASED: AtomicUsize = AtomicUsize::new(0);
        fn count(_: cocoa::base::id) {
            RELEASED.fetch_add(1, Ordering::SeqCst);
        }
        // Fake non-null pointers: `count` never dereferences them.
        let owned = [8 as cocoa::base::id, 16 as _, 24 as _];
        let result = std::panic::catch_unwind(|| {
            super::run_then_release(owned, count, |_| panic!("closure panicked"))
        });
        assert!(result.is_err());
        assert_eq!(RELEASED.load(Ordering::SeqCst), 3);
    }
}
