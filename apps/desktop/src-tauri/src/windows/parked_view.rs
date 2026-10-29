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

/// How much of the window must be on-screen, on BOTH axes, to count as
/// meaningfully visible. Logical points, the same threshold as the frontend
/// display-disconnect clamp (`$lib/render-idle-clamp` `MIN_VISIBLE_PT`). A
/// window smaller than this on an axis needs its whole extent on-screen there
/// instead (Quick Recall's idle bar is 60 pt tall, the permission helper 62 pt).
const MIN_VISIBLE_PT: f64 = 50.0;

/// The per-axis threshold never drops below this, so a tiny window still counts
/// as parked from a 1-px sliver (AeroSpace's is 1 px = 0.5–1 pt).
const MIN_SLIVER_PT: f64 = 2.0;

/// Set while no display is lit (`screens_did_sleep` .. any wake). Every view is
/// hidden then, whatever its window's position.
static SCREENS_ASLEEP: AtomicBool = AtomicBool::new(false);

/// The hide state last sent to each window's view, by label, so a move or
/// resize that doesn't change the decision costs no AppKit round trip. A
/// missing entry means "shown": a WKWebView starts un-hidden. Only touched on
/// the main thread (window events and `refresh_all`'s hop), so the
/// read-decide-write never races. Cleared on `Destroyed` (`forget`) so a
/// re-created window starts fresh.
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
/// scale: a monitor's position, size and scale are three separate
/// `backingScaleFactor` lookups (tao 0.34.8 `monitor.rs`
/// `size`/`position`/`scale_factor`). Every read here runs on the main thread,
/// where AppKit only updates `NSScreen` between run-loop turns, so the three
/// agree; every end of a display reconfiguration re-decides every window
/// (`refresh_all`). The window's rect and scale are separate reads too; a
/// scale change between them is followed by a `ScaleFactorChanged` event that
/// re-decides with consistent values.
fn to_points(x: i32, y: i32, width: u32, height: u32, scale: f64) -> Rect {
    (
        f64::from(x) / scale,
        f64::from(y) / scale,
        f64::from(width) / scale,
        f64::from(height) / scale,
    )
}

/// Whether the on-screen part of `window`, across all monitors, spans at least
/// `MIN_VISIBLE_PT` (or the window's own extent, if smaller) on both axes and
/// covers at least that much area. Pooled, not per monitor: a 60-pt bar split
/// 30/30 across two stacked displays is fully visible yet fails 50 pt on each.
/// No monitors (a transient mid-reconfiguration state) counts as visible:
/// hiding on a missing list could strand a blank window.
fn meaningfully_visible(monitors: &[Rect], window: Rect) -> bool {
    let (wx, wy, ww, wh) = window;
    let need_w = MIN_VISIBLE_PT.min(ww).max(MIN_SLIVER_PT);
    let need_h = MIN_VISIBLE_PT.min(wh).max(MIN_SLIVER_PT);
    if monitors.is_empty() {
        return true;
    }
    // ponytail: area + bounding box of the per-monitor pieces, not a true
    // "union contains a need_w x need_h square" test. Slivers along two edges
    // on two different monitors could pass; macOS displays tile without
    // overlap, so the area isn't double-counted.
    let (mut area, mut left, mut top, mut right, mut bottom) =
        (0.0, f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for &(mx, my, mw, mh) in monitors {
        // Origins are signed: a display left of or above the primary is negative.
        let (x0, y0) = (wx.max(mx), wy.max(my));
        let (x1, y1) = ((wx + ww).min(mx + mw), (wy + wh).min(my + mh));
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        area += (x1 - x0) * (y1 - y0);
        (left, top, right, bottom) = (left.min(x0), top.min(y0), right.max(x1), bottom.max(y1));
    }
    right - left >= need_w && bottom - top >= need_h && area >= need_w * need_h
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
/// window hidden.
fn decide(screens_asleep: bool, parked: bool) -> bool {
    screens_asleep || parked
}

/// Whether `hidden` differs from what the view last got. `None` (never sent)
/// is "shown", so a new on-screen window costs no send at all.
fn needs_send(sent: Option<bool>, hidden: bool) -> bool {
    sent.unwrap_or(false) != hidden
}

/// Whether the closure was queued; only then is the decision recorded as sent.
#[cfg(target_os = "macos")]
#[allow(deprecated, unexpected_cfgs)]
fn set_view_hidden(window: &WebviewWindow, hidden: bool) -> bool {
    use cocoa::base::{id, BOOL, NO, YES};
    use objc::{msg_send, sel, sel_impl};
    // SAFETY: `with_wk_webview` runs this closure on the main thread with the
    // window's live WKWebView and its NSWindow. Every message matches its
    // AppKit signature: `isHidden` -> BOOL, `setHidden:(BOOL)` -> void,
    // `firstResponder` -> NSResponder*, `makeFirstResponder:(NSResponder*)`
    // -> BOOL.
    super::with_wk_webview(window, move |wv, ns_window| unsafe {
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
        let responder: id = msg_send![ns_window, firstResponder];
        if responder.is_null() || responder == ns_window {
            let _: BOOL = msg_send![ns_window, makeFirstResponder: wv];
        }
    })
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
    let hidden = decide(asleep, parked(window, monitors));
    if !needs_send(sent_hidden(label), hidden) || !set_view_hidden(window, hidden) {
        return;
    }
    SENT.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(label.to_string(), hidden);
}

/// Re-decide one window's view: hidden while the screens sleep or the window is
/// parked. Called on every move/resize/focus of a webview window, which tauri
/// delivers on the main thread.
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
///
/// Callable from any thread (the wake recovery runs on its own): the work hops
/// to the main thread, inline when already there. tao reads `NSScreen` for the
/// monitor list on the calling thread (tauri-runtime-wry 2.10.1
/// `available_monitors`), which AppKit only allows on main.
pub fn refresh_all(app_handle: &tauri::AppHandle) {
    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
        let asleep = SCREENS_ASLEEP.load(Ordering::Relaxed);
        let monitors = if asleep { None } else { monitor_rects(&handle) };
        for window in handle.webview_windows().values() {
            apply(window, asleep, monitors.as_deref());
        }
    });
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
        assert!(!meaningfully_visible(
            &[AIR],
            (1709.5, 1111.5, 1200.0, 800.0)
        ));
    }

    #[test]
    fn on_screen_and_partly_off_windows_are_visible() {
        assert!(meaningfully_visible(&[AIR], (100.0, 50.0, 1200.0, 800.0)));
        assert!(meaningfully_visible(
            &[AIR],
            (1500.0, 1000.0, 1200.0, 800.0)
        ));
    }

    #[test]
    fn negative_origin_monitor_counts() {
        let left = (-1280.0, 0.0, 1280.0, 720.0);
        assert!(meaningfully_visible(
            &[AIR, left],
            (-1000.0, 100.0, 600.0, 400.0)
        ));
        assert!(!meaningfully_visible(
            &[AIR],
            (-1000.0, 100.0, 600.0, 400.0)
        ));
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
    fn short_window_split_across_stacked_monitors_is_visible() {
        // A 1x monitor stacked on top of the Air; Quick Recall's 60-pt bar
        // straddles the seam, 30 pt on each. Neither alone reaches 50 pt.
        let top = (0.0, -1080.0, 1920.0, 1080.0);
        assert!(meaningfully_visible(
            &[top, AIR],
            (300.0, -30.0, 800.0, 60.0)
        ));
    }

    #[test]
    fn wide_strip_on_one_axis_is_still_parked() {
        // Full width on-screen but a 3-pt strip of height: the pooled area
        // clears 50x50, the height doesn't.
        assert!(!meaningfully_visible(&[AIR], (0.0, 1109.0, 1200.0, 800.0)));
    }

    #[test]
    fn wake_keeps_a_parked_window_hidden() {
        let is_parked = !meaningfully_visible(&[AIR], (1709.5, 1111.5, 1200.0, 800.0));
        assert!(decide(false, is_parked));
    }

    #[test]
    fn sleep_hides_an_on_screen_window() {
        let is_parked = !meaningfully_visible(&[AIR], (100.0, 50.0, 1200.0, 800.0));
        assert!(decide(true, is_parked));
        assert!(!decide(false, is_parked));
    }

    #[test]
    fn a_never_parked_new_window_sends_nothing() {
        // Every send is an AppKit round trip; a new window's first move/focus
        // must not pay one just because nothing was recorded yet.
        assert!(!needs_send(None, decide(false, false)));
        assert!(needs_send(None, true));
        assert!(needs_send(Some(true), false));
        assert!(!needs_send(Some(true), true));
    }
}
