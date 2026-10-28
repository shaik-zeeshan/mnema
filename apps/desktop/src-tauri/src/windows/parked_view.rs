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

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tauri::{Manager, WebviewWindow};

/// How much of the window must overlap some monitor, on BOTH axes, to count as
/// meaningfully visible. Logical points: 100 physical px on a Retina display,
/// the frontend display-disconnect clamp's threshold (`$lib/render-idle-clamp`
/// `MIN_VISIBLE_PX`) on the Mac's own screen. A window smaller than this on an
/// axis needs its whole extent on-screen there instead (Quick Recall's idle bar
/// is 60 pt tall, the permission helper 62 pt).
const MIN_VISIBLE_PT: f64 = 50.0;

/// The per-axis threshold never drops below this, so a tiny window still counts
/// as parked from a 1-px sliver (AeroSpace's is 1 px = 0.5–1 pt).
const MIN_SLIVER_PT: f64 = 2.0;

/// Set while no display is lit (`screens_did_sleep` .. any wake). Every view is
/// hidden then, whatever its window's position.
static SCREENS_ASLEEP: AtomicBool = AtomicBool::new(false);

/// The hide state last sent to each window's view, by label. Every
/// `with_webview` call in tauri-runtime-wry 2.10.1 retains the WKWebView, its
/// controller and the NSWindow without releasing them, so it may only run when
/// the decision actually changes, not on every move or resize. Cleared on
/// `Destroyed` (`forget`) so a re-created window starts fresh.
static SENT: Mutex<Option<HashMap<String, bool>>> = Mutex::new(None);

/// `(x, y, width, height)` in the global logical-point space (top-left origin).
type Rect = (f64, f64, f64, f64);

/// A tao "physical" rect back in logical points. tao builds every monitor and
/// window rect as `points × that rect's own scale factor`, so two rects with
/// different scales (a Retina screen beside a 1x one) aren't in a shared pixel
/// space; dividing each by its own scale puts them back in AppKit's global
/// point space, where they are.
///
/// The division only round-trips when the rect and the scale saw the same
/// scale, and tao does not snapshot them: a monitor's position, size and scale
/// are three separate `backingScaleFactor` lookups (tao 0.34.8
/// `monitor.rs` `size`/`position`/`scale_factor`), which fall back to 1.0 while
/// the NSScreen is missing mid-reconfiguration. A steady state (screen present,
/// or missing for all three reads) is consistent; only a read straddling the
/// moment the NSScreen appears or vanishes mixes scales and moves the rect,
/// which can make an on-screen window look parked. So hiding takes a second,
/// fresh read that agrees (see `decide`), and every end of a display
/// reconfiguration re-decides every window (`refresh_all`). The window's rect and scale are
/// separate reads too; a scale change between them is followed by a
/// `ScaleFactorChanged` event that re-decides with consistent values.
fn to_points(x: i32, y: i32, width: u32, height: u32, scale: f64) -> Rect {
    (
        f64::from(x) / scale,
        f64::from(y) / scale,
        f64::from(width) / scale,
        f64::from(height) / scale,
    )
}

/// Whether `window` overlaps some monitor by at least `MIN_VISIBLE_PT` (or its
/// own extent, if smaller) on both axes. No monitors (a transient
/// mid-reconfiguration state) counts as visible: hiding on a missing list could
/// strand a blank window.
fn meaningfully_visible(monitors: &[Rect], window: Rect) -> bool {
    let (wx, wy, ww, wh) = window;
    let need_w = MIN_VISIBLE_PT.min(ww).max(MIN_SLIVER_PT);
    let need_h = MIN_VISIBLE_PT.min(wh).max(MIN_SLIVER_PT);
    monitors.is_empty()
        || monitors.iter().any(|&(mx, my, mw, mh)| {
            // Origins are signed: a display left of or above the primary is negative.
            let overlap_w = (wx + ww).min(mx + mw) - wx.max(mx);
            let overlap_h = (wy + wh).min(my + mh) - wy.max(my);
            overlap_w >= need_w && overlap_h >= need_h
        })
}

/// Every monitor's rect in points; `None` when tao can't list them.
fn monitor_rects(app_handle: &tauri::AppHandle) -> Option<Vec<Rect>> {
    let monitors = app_handle.available_monitors().ok()?;
    Some(
        monitors
            .iter()
            .map(|m| {
                let (p, s) = (m.position(), m.size());
                to_points(p.x, p.y, s.width, s.height, m.scale_factor())
            })
            .collect(),
    )
}

/// Whether `window` sits parked off every monitor. Missing geometry counts as
/// not parked: hiding on a failed read could strand a blank window.
fn parked(window: &WebviewWindow, monitors: Option<&[Rect]>) -> bool {
    let (Some(monitors), Ok(pos), Ok(size), Ok(scale)) = (
        monitors,
        window.outer_position(),
        window.outer_size(),
        window.scale_factor(),
    ) else {
        return false;
    };
    !meaningfully_visible(
        monitors,
        to_points(pos.x, pos.y, size.width, size.height, scale),
    )
}

/// The whole hide decision: hidden while the screens sleep (no geometry read),
/// or while the window is parked off every monitor, so a wake keeps a parked
/// window hidden. Biased toward visible: parked takes two agreeing reads, the
/// second fresh. A mixed-scale monitor read (see `to_points`) is a one-read
/// glitch at the instant an NSScreen appears or vanishes, so two in a row
/// don't happen. The re-read costs only parked windows, which rarely move.
fn decide(screens_asleep: bool, parked_now: bool, parked_on_reread: impl FnOnce() -> bool) -> bool {
    screens_asleep || (parked_now && parked_on_reread())
}

/// Whether the closure was queued; only then is the decision recorded as sent.
#[cfg(target_os = "macos")]
#[allow(deprecated, unexpected_cfgs)]
fn set_view_hidden(window: &WebviewWindow, hidden: bool) -> bool {
    use cocoa::base::{id, nil, BOOL, NO, YES};
    use objc::{msg_send, sel, sel_impl};
    // SAFETY: `with_webview` runs this closure on the main thread with the
    // window's live WKWebView; `inner()` is that NSView (null-checked below).
    // Every message matches its AppKit signature: `isHidden` -> BOOL,
    // `setHidden:(BOOL)` -> void, `window` -> NSWindow* (nil-checked),
    // `firstResponder` -> NSResponder*, `makeFirstResponder:(NSResponder*)`
    // -> BOOL.
    window
        .with_webview(move |webview| unsafe {
            let wv = webview.inner() as id;
            if wv.is_null() {
                return;
            }
            let is_hidden: BOOL = msg_send![wv, isHidden];
            if (is_hidden == YES) == hidden {
                return;
            }
            let _: () = msg_send![wv, setHidden: if hidden { YES } else { NO }];
            if hidden {
                return;
            }
            // Hiding a view that holds first responder makes AppKit hand focus back
            // to the window, so a revealed page would ignore the keyboard until
            // clicked. Give it back unless something else took focus meanwhile.
            let ns_window: id = msg_send![wv, window];
            if ns_window == nil {
                return;
            }
            let responder: id = msg_send![ns_window, firstResponder];
            if responder == nil || responder == ns_window {
                let _: BOOL = msg_send![ns_window, makeFirstResponder: wv];
            }
        })
        .is_ok()
}

#[cfg(not(target_os = "macos"))]
fn set_view_hidden(_window: &WebviewWindow, _hidden: bool) -> bool {
    true
}

fn sent_hidden(label: &str) -> Option<bool> {
    let sent = SENT.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    sent.as_ref()?.get(label).copied()
}

/// Re-decide one window against an already-read monitor list (`None` when
/// asleep, where no geometry is needed). Sends to the view only on a change.
fn apply(window: &WebviewWindow, asleep: bool, monitors: Option<&[Rect]>) {
    let label = window.label();
    let sent = sent_hidden(label);
    let hidden = decide(asleep, parked(window, monitors), || {
        parked(window, monitor_rects(window.app_handle()).as_deref())
    });
    if sent == Some(hidden) || !set_view_hidden(window, hidden) {
        return;
    }
    SENT.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(label.to_string(), hidden);
}

/// Re-decide one window's view: hidden while the screens sleep or the window is
/// parked. Called on every move/resize/focus of a webview window.
pub fn refresh(window: &WebviewWindow) {
    let asleep = SCREENS_ASLEEP.load(Ordering::Relaxed);
    let monitors = if asleep {
        None
    } else {
        monitor_rects(window.app_handle())
    };
    apply(window, asleep, monitors.as_deref());
}

/// Re-decide every window against one monitor read. Runs on each sleep/wake
/// signal and at the end of every display reconfiguration, so a window whose
/// display returns is revealed (and one stranded by a removal hidden) without
/// waiting for the window to move.
pub fn refresh_all(app_handle: &tauri::AppHandle) {
    let asleep = SCREENS_ASLEEP.load(Ordering::Relaxed);
    let monitors = if asleep {
        None
    } else {
        monitor_rects(app_handle)
    };
    for window in app_handle.webview_windows().values() {
        apply(window, asleep, monitors.as_deref());
    }
}

/// Drop a destroyed window's sent state, so a window re-created under the same
/// label gets its first decision sent.
pub fn forget(label: &str) {
    let mut sent = SENT.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(sent) = sent.as_mut() {
        sent.remove(label);
    }
}

/// The screens-sleep side: `true` on `screens_did_sleep`, `false` on every wake
/// signal. A wake reveals every view except the parked ones.
pub fn set_screens_asleep(app_handle: &tauri::AppHandle, asleep: bool) {
    SCREENS_ASLEEP.store(asleep, Ordering::Relaxed);
    refresh_all(app_handle);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A MacBook Air's built-in display in points (3420×2224 px at 2x).
    const AIR: Rect = (0.0, 0.0, 1710.0, 1112.0);
    /// A 1x 1080p monitor.
    const HD: Rect = (0.0, 0.0, 1920.0, 1080.0);

    #[test]
    fn aerospace_corner_sliver_is_parked() {
        // AeroSpace parks a hidden-workspace window at the bottom-right corner,
        // one pixel on-screen (half a point on Retina).
        assert!(!meaningfully_visible(&[AIR], (1709.5, 1111.5, 1200.0, 800.0)));
    }

    #[test]
    fn on_screen_and_partly_off_windows_are_visible() {
        assert!(meaningfully_visible(&[AIR], (100.0, 50.0, 1200.0, 800.0)));
        assert!(meaningfully_visible(&[AIR], (1500.0, 1000.0, 1200.0, 800.0)));
    }

    #[test]
    fn negative_origin_monitor_counts() {
        let left = (-1280.0, 0.0, 1280.0, 720.0);
        assert!(meaningfully_visible(&[AIR, left], (-1000.0, 100.0, 600.0, 400.0)));
        assert!(!meaningfully_visible(&[AIR], (-1000.0, 100.0, 600.0, 400.0)));
    }

    #[test]
    fn no_monitors_never_hides() {
        assert!(meaningfully_visible(&[], (99_999.0, 99_999.0, 10.0, 10.0)));
    }

    #[test]
    fn short_window_fully_on_a_1x_monitor_is_visible() {
        // Quick Recall's idle bar: 800×60 pt, under the 100-px threshold the
        // check once demanded on a 1x display.
        assert!(meaningfully_visible(&[HD], (560.0, 162.0, 800.0, 60.0)));
    }

    #[test]
    fn sliver_of_a_short_window_is_parked() {
        assert!(!meaningfully_visible(&[HD], (560.0, 1079.0, 800.0, 60.0)));
        assert!(!meaningfully_visible(&[HD], (1919.0, 162.0, 800.0, 60.0)));
    }

    #[test]
    fn rects_are_compared_in_points_across_scale_factors() {
        // Retina Air at the origin, a 1x 1080p monitor to its right at 1710 pt.
        let air = to_points(0, 0, 3420, 2224, 2.0);
        let hd = to_points(1710, 0, 1920, 1080, 1.0);
        // A 2x-backed window whose visible part is on the 1x monitor:
        // (3000, 100, 800, 600) pt. In raw tao pixels it sits at x = 6000,
        // right of both monitors' pixel rects.
        let window = to_points(6000, 200, 1600, 1200, 2.0);
        assert_eq!(window, (3000.0, 100.0, 800.0, 600.0));
        assert!(meaningfully_visible(&[air, hd], window));
    }

    #[test]
    fn wake_keeps_a_parked_window_hidden() {
        let is_parked = !meaningfully_visible(&[AIR], (1709.5, 1111.5, 1200.0, 800.0));
        assert!(decide(false, is_parked, || true));
    }

    #[test]
    fn hiding_takes_two_parked_reads() {
        // A one-read glitch (the re-read says on-screen) keeps the view visible.
        assert!(!decide(false, true, || false));
        // On-screen or asleep: decided without a re-read.
        assert!(!decide(false, false, || unreachable!()));
        assert!(decide(true, false, || unreachable!()));
    }

    #[test]
    fn sleep_hides_an_on_screen_window() {
        let is_parked = !meaningfully_visible(&[AIR], (100.0, 50.0, 1200.0, 800.0));
        assert!(decide(true, is_parked, || unreachable!()));
        assert!(!decide(false, is_parked, || unreachable!()));
    }
}
