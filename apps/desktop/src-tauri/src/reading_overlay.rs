//! The "Mnema is reading this screen" outline overlay (round-4 decision G3).
//!
//! Current-frame capture is IMPLICIT — collapsing the Quick Access window to the
//! bar *is* the gesture, there is no "attach screen?" modal. G3 only allows that
//! because indication is maximally explicit; the pair was designed together. This
//! is the indication: a transparent, click-through, full-screen window that draws
//! an accent outline around the display being captured plus a small named label,
//! shown for exactly as long as the bar is collapsed.
//!
//! Three things make it safe:
//!
//!   * **Never in its own shot.** The one-shot content filter
//!     (`capture_screen::current_frame`) excludes Mnema by pid AND bundle id and
//!     excludes whole *apps*, so every window this process owns — this overlay
//!     included — is blanked. Nothing here has to opt in.
//!   * **Never takes input.** `set_ignore_cursor_events` (macOS:
//!     `-[NSWindow setIgnoresMouseEvents:]`) plus `pointer-events: none` in the
//!     page.
//!   * **Never takes focus.** It is ordered front WITHOUT being made key:
//!     Tauri's `show()` routes to `makeKeyAndOrderFront:`, which would steal key
//!     status from the collapsed bar mid-typing.
//!
//! Lifecycle is owned by the ONE seam that already collapses the window —
//! `quick_recall_set_collapsed` in `windows.rs` — plus the dismiss chokepoint.
//! No second state machine, so the overlay cannot disagree with the bar.

use tauri::{Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::native_capture::debug_log;

pub(crate) const READING_OVERLAY_WINDOW_LABEL: &str = "reading-overlay";

/// Show the outline over the display being captured, building the window on
/// first use. Best-effort throughout: a failure here must never break the ask.
pub(crate) fn show_reading_overlay(app: &tauri::AppHandle) {
    let window = match app.get_webview_window(READING_OVERLAY_WINDOW_LABEL) {
        Some(window) => window,
        None => match build_reading_overlay_window(app) {
            Ok(window) => window,
            Err(error) => {
                debug_log::log_warn(format!(
                    "failed to build the reading overlay window: {error}"
                ));
                return;
            }
        },
    };

    fit_overlay_to_captured_display(app, &window);
    order_front_without_key(&window);
}

pub(crate) fn hide_reading_overlay(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window(READING_OVERLAY_WINDOW_LABEL) {
        let _ = window.hide();
    }
}

fn build_reading_overlay_window(app: &tauri::AppHandle) -> Result<WebviewWindow, String> {
    let built = WebviewWindowBuilder::new(
        app,
        READING_OVERLAY_WINDOW_LABEL,
        // A static page, not a SvelteKit route: the app shell's layout does
        // licensing checks, IPC and event wiring that have no business running
        // in a decoration. See `static/reading-overlay.html`.
        WebviewUrl::App("reading-overlay.html".into()),
    )
    .title("mnema · reading this screen")
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .resizable(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .visible_on_all_workspaces(true)
    .focused(false)
    .visible(false)
    .build()
    .map_err(|error| error.to_string())?;

    if let Err(error) = built.set_ignore_cursor_events(true) {
        // Load-bearing: an overlay that eats clicks is worse than no overlay, so
        // refuse to show one we could not make click-through.
        let _ = built.close();
        return Err(format!(
            "failed to make the reading overlay click-through: {error}"
        ));
    }

    Ok(built)
}

/// Size and place the overlay over the display the shot is taken of.
///
/// ponytail: v1 is the primary display's **work area**, matching slice 10's
/// `displays().first()` one-display capture. Work area rather than full frame
/// because the menu bar and Dock draw above a floating window, so a full-frame
/// outline would have its top edge invisible. Per-display fan-out arrives when
/// the capture itself gains a display picker.
fn fit_overlay_to_captured_display(app: &tauri::AppHandle, window: &WebviewWindow) {
    let Ok(Some(monitor)) = app.primary_monitor() else {
        return;
    };
    let area = monitor.work_area();
    let _ = window.set_position(area.position);
    let _ = window.set_size(area.size);
}

/// Order the overlay in front without making it key.
///
/// `WebviewWindow::show()` is `makeKeyAndOrderFront:` on macOS, which would pull
/// key status off the collapsed Quick Access bar the user is typing into. Same
/// trick, opposite direction, as `order_out_quick_recall_panel` in `windows.rs`.
#[cfg(target_os = "macos")]
#[allow(deprecated, unexpected_cfgs)]
fn order_front_without_key(window: &WebviewWindow) {
    use cocoa::base::{id, nil};
    use objc::{msg_send, sel, sel_impl};

    let Ok(ns_window) = window.ns_window() else {
        let _ = window.show();
        return;
    };
    unsafe {
        let ns_window = ns_window as id;
        let _: () = msg_send![ns_window, orderFront: nil];
    }
}

#[cfg(not(target_os = "macos"))]
fn order_front_without_key(window: &WebviewWindow) {
    let _ = window.show();
}
