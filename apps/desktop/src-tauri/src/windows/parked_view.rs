// A window parked (almost) entirely off every screen keeps its WKWebView
// "visible": macOS reports a window with a sliver on-screen as visible, so it
// never occludes and `document.visibilityState` never goes hidden. Tiling
// window managers park windows this way on purpose (AeroSpace leaves a 1-px
// corner sliver for every window on another workspace), and so can a display
// disconnect. The page then keeps rendering and every render-idle-gated updater
// keeps ticking for pixels nobody can see: measured 2026-10-27 (runs 80–84,
// YouTube while recording) at +1.2 W whole-Mac, WebKit helpers 3.2% → 0.2% of a
// core with the window closed.
//
// So a parked window gets the same treatment as sleeping screens: its WKWebView
// view is hidden (`setHidden:`, never `orderOut:` on the window). WebKit drops
// view-visible state, the page goes hidden, and the frontend render-idle gate
// parks itself. Moving the window back on-screen reveals the view.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::WebviewWindow;

/// How much of the window must overlap some monitor, on BOTH axes, to count as
/// meaningfully visible. Physical pixels. Same threshold as the frontend
/// display-disconnect clamp (`$lib/render-idle-clamp` `MIN_VISIBLE_PX`).
const MIN_VISIBLE_PX: i64 = 100;

/// Set while no display is lit (`screens_did_sleep` .. any wake). Every view is
/// hidden then, whatever its window's position.
static SCREENS_ASLEEP: AtomicBool = AtomicBool::new(false);

/// `(x, y, width, height)` in the shared global physical-pixel space.
type Rect = (i64, i64, i64, i64);

/// Whether `window` overlaps some monitor by at least `MIN_VISIBLE_PX` on both
/// axes. No monitors (a transient mid-reconfiguration state) counts as visible:
/// hiding on a missing list could strand a blank window.
fn meaningfully_visible(monitors: &[Rect], window: Rect) -> bool {
    let (wx, wy, ww, wh) = window;
    monitors.is_empty()
        || monitors.iter().any(|&(mx, my, mw, mh)| {
            // Origins are signed: a display left of or above the primary is negative.
            let overlap_w = (wx + ww).min(mx + mw) - wx.max(mx);
            let overlap_h = (wy + wh).min(my + mh) - wy.max(my);
            overlap_w >= MIN_VISIBLE_PX && overlap_h >= MIN_VISIBLE_PX
        })
}

fn is_parked(window: &WebviewWindow) -> bool {
    let (Ok(pos), Ok(size), Ok(monitors)) = (
        window.outer_position(),
        window.outer_size(),
        window.available_monitors(),
    ) else {
        return false;
    };
    let monitors: Vec<Rect> = monitors
        .iter()
        .map(|m| {
            let (p, s) = (m.position(), m.size());
            (p.x.into(), p.y.into(), s.width.into(), s.height.into())
        })
        .collect();
    !meaningfully_visible(
        &monitors,
        (
            pos.x.into(),
            pos.y.into(),
            size.width.into(),
            size.height.into(),
        ),
    )
}

#[cfg(target_os = "macos")]
#[allow(deprecated, unexpected_cfgs)]
fn set_view_hidden(window: &WebviewWindow, hidden: bool) {
    use cocoa::base::{id, NO, YES};
    use objc::{msg_send, sel, sel_impl};
    let flag = if hidden { YES } else { NO };
    let _ = window.with_webview(move |webview| unsafe {
        let wv = webview.inner() as id;
        if wv.is_null() {
            return;
        }
        // AppKit ignores a no-op `setHidden:`, so per-move calls are cheap.
        let _: () = msg_send![wv, setHidden: flag];
    });
}

#[cfg(not(target_os = "macos"))]
fn set_view_hidden(_window: &WebviewWindow, _hidden: bool) {}

/// Re-decide one window's view: hidden while the screens sleep or the window is
/// parked. Called on every move/resize/focus of a webview window.
pub fn refresh(window: &WebviewWindow) {
    set_view_hidden(
        window,
        SCREENS_ASLEEP.load(Ordering::Relaxed) || is_parked(window),
    );
}

/// The screens-sleep side: `true` on `screens_did_sleep`, `false` on every wake
/// signal. A wake reveals every view except the parked ones.
pub fn set_screens_asleep(app_handle: &tauri::AppHandle, asleep: bool) {
    use tauri::Manager;
    SCREENS_ASLEEP.store(asleep, Ordering::Relaxed);
    for window in app_handle.webview_windows().values() {
        refresh(window);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AIR: Rect = (0, 0, 3420, 2224);

    #[test]
    fn aerospace_corner_sliver_is_parked() {
        // AeroSpace parks a hidden-workspace window at the bottom-right corner,
        // one pixel on-screen.
        assert!(!meaningfully_visible(&[AIR], (3419, 2223, 2400, 1600)));
    }

    #[test]
    fn on_screen_and_partly_off_windows_are_visible() {
        assert!(meaningfully_visible(&[AIR], (200, 100, 2400, 1600)));
        assert!(meaningfully_visible(&[AIR], (3000, 2000, 2400, 1600)));
    }

    #[test]
    fn negative_origin_monitor_counts() {
        let left = (-2560, 0, 2560, 1440);
        assert!(meaningfully_visible(&[AIR, left], (-2000, 200, 1200, 800)));
        assert!(!meaningfully_visible(&[AIR], (-2000, 200, 1200, 800)));
    }

    #[test]
    fn no_monitors_never_hides() {
        assert!(meaningfully_visible(&[], (99_999, 99_999, 10, 10)));
    }
}
