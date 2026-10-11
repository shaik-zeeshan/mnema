// Quick Recall is a bar sized to its content: the page measures itself and
// calls `set_quick_recall_frame(height)` (60 idle, ≤ 420 with results, ~570 in
// Quick Look). The top edge stays at a fixed anchor — centred, ~15 % down the
// work area — so the bar grows and shrinks downward. Size and anchor are set in
// one native frame change; `setSize` alone keeps Cocoa's bottom-left origin and
// would grow the bar upward. A Rust command needs no window capability.

use tauri::WebviewWindow;

use super::QUICK_RECALL_WINDOW_LABEL;

/// The bar's width in logical points (`recall.html` `.rc-float`).
pub(super) const WIDTH: f64 = 800.0;
/// The idle bar: the field row alone.
pub(super) const IDLE_HEIGHT: f64 = 60.0;
/// Where the top edge sits, as a fraction of the work area's height.
const TOP_FRACTION: f64 = 0.15;

/// The bar's frame inside a work area, in logical points with `top` measured
/// DOWN from the area's top edge: `(x, top, width, height)`. A bar too tall to
/// hang from the anchor is lifted until its bottom fits.
fn bar_frame(area_x: f64, area_width: f64, area_height: f64, height: f64) -> (f64, f64, f64, f64) {
    let width = WIDTH.min(area_width);
    let height = height.min(area_height).max(1.0);
    let top = (area_height * TOP_FRACTION)
        .min(area_height - height)
        .max(0.0)
        .round();
    let x = (area_x + (area_width - width) / 2.0).round();
    (x, top, width, height.round())
}

/// Resize the bar to `height` logical points and re-anchor its top edge.
#[tauri::command]
pub fn set_quick_recall_frame(window: WebviewWindow, height: f64) -> Result<(), String> {
    if window.label() != QUICK_RECALL_WINDOW_LABEL {
        return Err("set_quick_recall_frame is only for the Quick Recall window".into());
    }
    if !height.is_finite() {
        return Err(format!("invalid Quick Recall height: {height}"));
    }
    anchor_quick_recall(&window, Some(height));
    Ok(())
}

/// Put the bar at its anchor — at `height`, or its current height (on summon).
#[cfg(target_os = "macos")]
pub(super) fn anchor_quick_recall(window: &WebviewWindow, height: Option<f64>) {
    let target = window.clone();
    let _ = window.run_on_main_thread(move || set_native_frame(&target, height));
}

#[cfg(target_os = "macos")]
#[allow(deprecated, unexpected_cfgs)]
fn set_native_frame(window: &WebviewWindow, height: Option<f64>) {
    use cocoa::base::{id, nil, YES};
    use cocoa::foundation::{NSPoint, NSRect, NSSize};
    use objc::{class, msg_send, sel, sel_impl};

    let Ok(ns_window) = window.ns_window() else {
        return;
    };
    unsafe {
        let ns_window = ns_window as id;
        // The bar's own screen, like the `center()` it replaces.
        let mut screen: id = msg_send![ns_window, screen];
        if screen == nil {
            screen = msg_send![class!(NSScreen), mainScreen];
        }
        if screen == nil {
            return;
        }
        let area: NSRect = msg_send![screen, visibleFrame];
        let current: NSRect = msg_send![ns_window, frame];
        let (x, top, width, height) = bar_frame(
            area.origin.x,
            area.size.width,
            area.size.height,
            height.unwrap_or(current.size.height),
        );
        // Cocoa's y grows up from the bottom of the screen.
        let y = area.origin.y + area.size.height - top - height;
        let frame = NSRect::new(NSPoint::new(x, y), NSSize::new(width, height));
        let _: () = msg_send![ns_window, setFrame: frame display: YES];
        // The transparent window's shadow follows its last shape until told.
        let _: () = msg_send![ns_window, invalidateShadow];
    }
}

// ponytail: two calls, so the bar may jump for a frame; no native single-step
// frame setter is wired off macOS (see SUPPORTS.md).
#[cfg(not(target_os = "macos"))]
pub(super) fn anchor_quick_recall(window: &WebviewWindow, height: Option<f64>) {
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        return;
    };
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let current = window
        .inner_size()
        .map(|size| size.to_logical::<f64>(scale).height)
        .unwrap_or(IDLE_HEIGHT);
    let (x, top, width, height) = bar_frame(
        area.position.x as f64 / scale,
        area.size.width as f64 / scale,
        area.size.height as f64 / scale,
        height.unwrap_or(current),
    );
    let _ = window.set_size(tauri::LogicalSize::new(width, height));
    let _ = window.set_position(tauri::LogicalPosition::new(
        x,
        area.position.y as f64 / scale + top,
    ));
}

#[cfg(test)]
mod tests {
    use super::bar_frame;

    #[test]
    fn the_bar_hangs_centred_from_fifteen_percent_down_at_every_height() {
        // 1440×875 work area at x=0: top edge at 131 for idle, results and Quick Look.
        for height in [60.0, 420.0, 570.0] {
            assert_eq!(
                bar_frame(0.0, 1440.0, 875.0, height),
                (320.0, 131.0, 800.0, height)
            );
        }
    }

    #[test]
    fn a_secondary_display_offsets_the_centre() {
        assert_eq!(bar_frame(1440.0, 1920.0, 1050.0, 60.0).0, 1440.0 + 560.0);
    }

    #[test]
    fn a_bar_too_tall_for_the_anchor_is_lifted_to_fit() {
        // 600-high area: the anchor (90) + 570 would overrun by 60.
        assert_eq!(
            bar_frame(0.0, 1024.0, 600.0, 570.0),
            (112.0, 30.0, 800.0, 570.0)
        );
        // Taller than the whole area: clamped to it, top edge at the area top.
        assert_eq!(
            bar_frame(0.0, 1024.0, 600.0, 900.0),
            (112.0, 0.0, 800.0, 600.0)
        );
    }

    #[test]
    fn a_narrow_screen_and_a_bogus_height_stay_on_screen() {
        assert_eq!(bar_frame(0.0, 700.0, 500.0, -5.0), (0.0, 75.0, 700.0, 1.0));
    }
}
