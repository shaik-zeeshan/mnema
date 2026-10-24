//! App Privacy Exclusion, applied only while it has something to hide.
//!
//! Excluding a *running* app from the ScreenCaptureKit display filter makes WindowServer
//! compose a second copy of the display on the GPU for the stream, even when that app shows
//! nothing. Measured 2026-10-24 (M4 Air, whole-Mac, bare SCK stream, Mnema quit): a plain
//! display filter costs ~+0.2 W; the same stream excluding one running app ~+2.4 W (GPU busy
//! 64%, DRAM power ×5), and WindowServer stays in that mode minutes after the stream stops.
//! Excluded apps are usually invisible: menu-bar only, hidden, minimized, on another Space,
//! or parked by AeroSpace as a 1-pt corner sliver.
//!
//! So the plan excludes an app only while one of its windows shows on the captured display
//! (`lib.rs` `plan_privacy_content_filter`), and the stream handler drops every frame taken
//! while a running excluded app the filter leaves out shows a window — until the next filter
//! refresh (the 1 s privacy poll, or a focus change) excludes it.

use std::sync::{Arc, Mutex};

/// A window shows on the display when it covers at least this much of it, in points, on both
/// axes. AeroSpace parks the windows of other workspaces with a 1-pt strip still on screen.
const MIN_VISIBLE_EDGE_PT: f64 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

pub(crate) fn shows_on_display(window: Rect, display: Rect) -> bool {
    let width = (window.x + window.width).min(display.x + display.width) - window.x.max(display.x);
    let height =
        (window.y + window.height).min(display.y + display.height) - window.y.max(display.y);
    width >= MIN_VISIBLE_EDGE_PT && height >= MIN_VISIBLE_EDGE_PT
}

/// The excluded apps the live filter leaves out (no window showed when it was built) and the
/// display it captures. The session writes it around each filter update; the stream handler
/// reads it per frame.
#[derive(Debug, Clone, Default)]
pub(crate) struct PrivacyFrameGuard {
    pub display: Rect,
    pub unfiltered_bundle_ids: Vec<String>,
}

pub(crate) type SharedPrivacyFrameGuard = Arc<Mutex<PrivacyFrameGuard>>;

/// Whether any of `windows` (owner pid, bounds) belongs to `pids` and shows on `display`.
pub(crate) fn any_window_shows(windows: &[(i32, Rect)], pids: &[i32], display: Rect) -> bool {
    windows
        .iter()
        .any(|(pid, bounds)| pids.contains(pid) && shows_on_display(*bounds, display))
}

/// Whether the frame the stream just delivered must be dropped because a running excluded app
/// the filter leaves out shows a window now. The check runs after the frame was composed, so a
/// window that showed in it still shows here. Fails closed when the window list can't be read.
#[cfg(target_os = "macos")]
pub(crate) fn frame_must_be_dropped(guard: &SharedPrivacyFrameGuard) -> bool {
    let guard = guard
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.unfiltered_bundle_ids.is_empty() {
        return false;
    }
    let pids = running_pids(&guard.unfiltered_bundle_ids);
    if pids.is_empty() {
        return false;
    }
    match on_screen_capturable_windows() {
        Some(windows) => any_window_shows(&windows, &pids, guard.display),
        None => true,
    }
}

#[cfg(target_os = "macos")]
fn running_pids(bundle_ids: &[String]) -> Vec<i32> {
    bundle_ids
        .iter()
        .flat_map(|bundle_id| {
            cidre::ns::RunningApp::with_bundle_id(&cidre::ns::String::with_str(bundle_id))
                .iter()
                .map(|app| app.pid())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// On-screen windows as (owner pid, bounds), skipping `sharingType = .none` ones, which no
/// capture can see (Mnema's own windows, outside developer options).
#[cfg(target_os = "macos")]
fn on_screen_capturable_windows() -> Option<Vec<(i32, Rect)>> {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
    use core_foundation::number::CFNumber;
    use core_foundation::string::{CFString, CFStringRef};
    use core_graphics::window::{
        copy_window_info, kCGNullWindowID, kCGWindowBounds, kCGWindowListExcludeDesktopElements,
        kCGWindowListOptionOnScreenOnly, kCGWindowOwnerPID, kCGWindowSharingNone,
        kCGWindowSharingState,
    };

    type Dict = CFDictionary<CFString, CFType>;
    fn get(dict: &Dict, key: &CFString) -> Option<CFType> {
        dict.find(key).map(|value| value.clone())
    }
    fn number(dict: &Dict, key: &CFString) -> Option<f64> {
        let value = get(dict, key)?;
        unsafe { CFNumber::wrap_under_get_rule(value.as_CFTypeRef() as _) }.to_f64()
    }
    let key = |key: CFStringRef| unsafe { CFString::wrap_under_get_rule(key) };
    let (pid_key, sharing_key, bounds_key) = unsafe {
        (
            key(kCGWindowOwnerPID),
            key(kCGWindowSharingState),
            key(kCGWindowBounds),
        )
    };
    let (x, y, width, height) = (
        CFString::new("X"),
        CFString::new("Y"),
        CFString::new("Width"),
        CFString::new("Height"),
    );

    let options = kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements;
    let infos = copy_window_info(options, kCGNullWindowID)?;
    Some(
        infos
            .iter()
            .filter_map(|value| {
                let dict = unsafe { Dict::wrap_under_get_rule(*value as CFDictionaryRef) };
                if number(&dict, &sharing_key) == Some(f64::from(kCGWindowSharingNone)) {
                    return None;
                }
                let pid = number(&dict, &pid_key)? as i32;
                let bounds = get(&dict, &bounds_key)?;
                let bounds = unsafe { Dict::wrap_under_get_rule(bounds.as_CFTypeRef() as _) };
                Some((
                    pid,
                    Rect {
                        x: number(&bounds, &x)?,
                        y: number(&bounds, &y)?,
                        width: number(&bounds, &width)?,
                        height: number(&bounds, &height)?,
                    },
                ))
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const DISPLAY: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 1710.0,
        height: 1112.0,
    };

    #[test]
    fn aerospace_parked_sliver_does_not_show() {
        // Measured: WhatsApp parked by AeroSpace at (1709, 1060), 1690×1052 on a 1710×1112 display.
        let parked = Rect {
            x: 1709.0,
            y: 1060.0,
            width: 1690.0,
            height: 1052.0,
        };
        assert!(!shows_on_display(parked, DISPLAY));
        let other_display = Rect {
            x: 1710.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        };
        assert!(!shows_on_display(other_display, DISPLAY));
    }

    #[test]
    fn a_window_partly_on_the_display_shows() {
        assert!(shows_on_display(
            Rect {
                x: 10.0,
                y: 49.0,
                width: 1690.0,
                height: 1052.0
            },
            DISPLAY
        ));
        assert!(shows_on_display(
            Rect {
                x: 1700.0,
                y: -500.0,
                width: 400.0,
                height: 520.0
            },
            DISPLAY
        ));
    }

    #[test]
    fn only_watched_pids_with_showing_windows_drop_a_frame() {
        let shown = Rect {
            x: 100.0,
            y: 100.0,
            width: 400.0,
            height: 300.0,
        };
        let parked = Rect {
            x: 1709.0,
            y: 1078.0,
            width: 1690.0,
            height: 1052.0,
        };
        assert!(any_window_shows(&[(7, parked), (9, shown)], &[9], DISPLAY));
        assert!(!any_window_shows(&[(7, shown), (9, parked)], &[9], DISPLAY));
        assert!(!any_window_shows(&[(7, shown)], &[], DISPLAY));
    }
}
