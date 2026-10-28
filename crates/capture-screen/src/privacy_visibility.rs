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

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

/// A window shows on the display when it covers at least this much of it, in points, on both
/// axes. AeroSpace parks the windows of other workspaces with a 1-pt strip still on screen.
const MIN_VISIBLE_EDGE_PT: f64 = 2.0;

/// `kCGStatusWindowLevel`: menu-bar status items. An app whose only window is its menu-bar
/// icon (1Password, most utilities) shows nothing worth hiding; excluding it for that icon
/// would keep the costly filter on all day. Only this exact layer — popup menus (101) and
/// floating panels (3) opened from the icon still count.
const STATUS_WINDOW_LAYER: i64 = 25;

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

/// The one rule both the filter plan (`SCWindow`) and the frame guard (`CGWindowList`) use, so
/// an app the plan leaves out is exactly one the guard treats as hidden.
pub(crate) fn window_shows(layer: i64, bounds: Rect, display: Rect) -> bool {
    layer != STATUS_WINDOW_LAYER && shows_on_display(bounds, display)
}

/// A window from the ScreenCaptureKit shareable content, as the filter plan sees it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ShareableWindow {
    pub owner_bundle_id: Option<String>,
    pub on_screen: bool,
    pub layer: i64,
    pub bounds: Rect,
}

/// The apps with a window showing on `display`.
pub(crate) fn showing_bundle_ids(
    windows: impl IntoIterator<Item = ShareableWindow>,
    display: Rect,
) -> BTreeSet<String> {
    windows
        .into_iter()
        .filter(|window| window.on_screen && window_shows(window.layer, window.bounds, display))
        .filter_map(|window| window.owner_bundle_id)
        .collect()
}

/// An on-screen window from `CGWindowList`, as the frame guard sees it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct OnScreenWindow {
    pub pid: i32,
    pub layer: i64,
    pub bounds: Rect,
}

/// The excluded apps the live filter leaves out (no window showed when it was built) and the
/// display it captures. The session writes it around each filter update; the stream handler
/// reads it per frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct PrivacyFrameGuard {
    pub display: Rect,
    pub unfiltered_bundle_ids: Vec<String>,
}

pub(crate) type SharedPrivacyFrameGuard = Arc<Mutex<PrivacyFrameGuard>>;

/// The guard while a filter update is in flight: the apps either filter leaves out, so one
/// dropping an exclusion can't let a frame through before the guard covers it.
pub(crate) fn in_flight_guard(
    current: &PrivacyFrameGuard,
    next: &PrivacyFrameGuard,
) -> PrivacyFrameGuard {
    let mut unfiltered_bundle_ids = current.unfiltered_bundle_ids.clone();
    for bundle_id in &next.unfiltered_bundle_ids {
        if !unfiltered_bundle_ids.contains(bundle_id) {
            unfiltered_bundle_ids.push(bundle_id.clone());
        }
    }
    PrivacyFrameGuard {
        display: next.display,
        unfiltered_bundle_ids,
    }
}

/// The guard once the update settles. Only a landed update narrows it to the new filter's: a
/// failed or timed-out one may still land later, so the union stays until one succeeds.
pub(crate) fn settled_guard(
    in_flight: PrivacyFrameGuard,
    next: PrivacyFrameGuard,
    update_landed: bool,
) -> PrivacyFrameGuard {
    if update_landed {
        next
    } else {
        in_flight
    }
}

/// Whether any of `windows` belongs to `pids` and shows on `display`.
pub(crate) fn any_window_shows(windows: &[OnScreenWindow], pids: &[i32], display: Rect) -> bool {
    windows.iter().any(|window| {
        pids.contains(&window.pid) && window_shows(window.layer, window.bounds, display)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameVerdict {
    Keep,
    /// A running app the filter leaves out shows a window.
    DropShowing,
    /// The window list couldn't be read: fail closed.
    DropUnreadable,
}

/// Whether a frame must be dropped. The window list is read only when an app the filter leaves
/// out is running, which keeps the common case (nothing left out) free.
pub(crate) fn frame_verdict(
    guard: &PrivacyFrameGuard,
    running_pids: impl FnOnce(&[String]) -> Vec<i32>,
    windows: impl FnOnce() -> Option<Vec<OnScreenWindow>>,
) -> FrameVerdict {
    if guard.unfiltered_bundle_ids.is_empty() {
        return FrameVerdict::Keep;
    }
    let pids = running_pids(&guard.unfiltered_bundle_ids);
    if pids.is_empty() {
        return FrameVerdict::Keep;
    }
    match windows() {
        Some(windows) if any_window_shows(&windows, &pids, guard.display) => {
            FrameVerdict::DropShowing
        }
        Some(_) => FrameVerdict::Keep,
        None => FrameVerdict::DropUnreadable,
    }
}

/// The stream handler's side of the guard: reads it per frame and logs state changes.
// ponytail: one per segment, so a rotation mid-drop restarts the dropped-frame count.
#[derive(Debug, Default)]
pub(crate) struct PrivacyFrameDropper {
    guard: SharedPrivacyFrameGuard,
    dropped_frames: u64,
    window_list_unreadable: bool,
}

impl PrivacyFrameDropper {
    pub fn new(guard: SharedPrivacyFrameGuard) -> Self {
        Self {
            guard,
            ..Self::default()
        }
    }

    /// Whether the frame the stream just delivered must be dropped. The check runs after the
    /// frame was composed, so a window that showed in it still shows here.
    pub fn frame_must_be_dropped(&mut self) -> bool {
        // Copy and release: the checks below call into AppKit and WindowServer.
        let guard = self
            .guard
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let verdict = frame_verdict(&guard, running_pids, on_screen_windows);
        self.record(verdict)
    }

    fn record(&mut self, verdict: FrameVerdict) -> bool {
        let unreadable = verdict == FrameVerdict::DropUnreadable;
        if unreadable && !self.window_list_unreadable {
            capture_runtime::warn_log!(
                "privacy frame guard can't read the window list; dropping frames until it can"
            );
        }
        self.window_list_unreadable = unreadable;
        if verdict == FrameVerdict::Keep {
            if self.dropped_frames > 0 {
                capture_runtime::debug_log!(
                    "privacy frame guard stopped dropping frames after {} frame(s)",
                    self.dropped_frames
                );
                self.dropped_frames = 0;
            }
            return false;
        }
        if self.dropped_frames == 0 {
            capture_runtime::debug_log!(
                "privacy frame guard dropping frames: a left-out excluded app shows a window"
            );
        }
        self.dropped_frames += 1;
        true
    }
}

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

/// On-screen windows, skipping `sharingType = .none` ones, which no capture can see (Mnema's
/// own windows, outside developer options).
fn on_screen_windows() -> Option<Vec<OnScreenWindow>> {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_foundation::string::{CFString, CFStringRef};
    use core_graphics::window::{
        copy_window_info, kCGNullWindowID, kCGWindowBounds, kCGWindowLayer,
        kCGWindowListExcludeDesktopElements, kCGWindowListOptionOnScreenOnly, kCGWindowOwnerPID,
        kCGWindowSharingNone, kCGWindowSharingState,
    };

    fn value(dict: &CFDictionary, key: &CFString) -> Option<CFType> {
        let value = dict.find(key.as_CFTypeRef())?;
        // SAFETY: CGWindowList dictionaries hold CF objects as values, kept alive by `dict`;
        // the get rule retains the wrapper for as long as it lives.
        Some(unsafe { CFType::wrap_under_get_rule(*value) })
    }
    fn number(dict: &CFDictionary, key: &CFString) -> Option<f64> {
        value(dict, key)?.downcast::<CFNumber>()?.to_f64()
    }
    // SAFETY: the `kCGWindow*` keys are immutable CFString constants exported by CoreGraphics.
    let key = |key: CFStringRef| unsafe { CFString::wrap_under_get_rule(key) };
    // SAFETY: reading the CoreGraphics key statics; they are initialised before any Rust code
    // runs and never written.
    let (pid_key, layer_key, sharing_key, bounds_key) = unsafe {
        (
            key(kCGWindowOwnerPID),
            key(kCGWindowLayer),
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
            .filter_map(|item| {
                // SAFETY: the array's elements are non-null CF objects it keeps alive; the type
                // is checked by `downcast` before the value is used as a dictionary.
                let dict =
                    unsafe { CFType::wrap_under_get_rule(*item) }.downcast::<CFDictionary>()?;
                if number(&dict, &sharing_key) == Some(f64::from(kCGWindowSharingNone)) {
                    return None;
                }
                let bounds = value(&dict, &bounds_key)?.downcast::<CFDictionary>()?;
                Some(OnScreenWindow {
                    pid: number(&dict, &pid_key)? as i32,
                    layer: number(&dict, &layer_key)? as i64,
                    bounds: Rect {
                        x: number(&bounds, &x)?,
                        y: number(&bounds, &y)?,
                        width: number(&bounds, &width)?,
                        height: number(&bounds, &height)?,
                    },
                })
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
    const SHOWN: Rect = Rect {
        x: 100.0,
        y: 100.0,
        width: 400.0,
        height: 300.0,
    };
    const PARKED: Rect = Rect {
        x: 1709.0,
        y: 1078.0,
        width: 1690.0,
        height: 1052.0,
    };
    const OTHER_DISPLAY: Rect = Rect {
        x: 1710.0,
        y: 0.0,
        width: 800.0,
        height: 600.0,
    };

    fn window(pid: i32, bounds: Rect) -> OnScreenWindow {
        OnScreenWindow {
            pid,
            layer: 0,
            bounds,
        }
    }

    fn guard(bundle_ids: &[&str]) -> PrivacyFrameGuard {
        PrivacyFrameGuard {
            display: DISPLAY,
            unfiltered_bundle_ids: bundle_ids.iter().map(|id| id.to_string()).collect(),
        }
    }

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
        assert!(!shows_on_display(OTHER_DISPLAY, DISPLAY));
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
    fn a_window_shows_from_exactly_two_points_on_both_axes() {
        let at = |x: f64, y: f64| Rect {
            x,
            y,
            width: 500.0,
            height: 500.0,
        };
        assert!(shows_on_display(at(1708.0, 1110.0), DISPLAY));
        assert!(!shows_on_display(at(1708.5, 1110.0), DISPLAY));
        assert!(!shows_on_display(at(1708.0, 1110.5), DISPLAY));
    }

    #[test]
    fn a_status_item_does_not_show_but_its_menus_and_panels_do() {
        assert!(!window_shows(STATUS_WINDOW_LAYER, SHOWN, DISPLAY));
        assert!(window_shows(101, SHOWN, DISPLAY));
        assert!(window_shows(3, SHOWN, DISPLAY));
        assert!(window_shows(0, SHOWN, DISPLAY));
    }

    #[test]
    fn showing_bundle_ids_skips_hidden_elsewhere_ownerless_and_status_windows() {
        let shareable = |owner: Option<&str>, on_screen: bool, layer: i64, bounds: Rect| {
            ShareableWindow {
                owner_bundle_id: owner.map(str::to_string),
                on_screen,
                layer,
                bounds,
            }
        };
        let showing = showing_bundle_ids(
            [
                shareable(Some("com.shown"), true, 0, SHOWN),
                shareable(Some("com.offscreen"), false, 0, SHOWN),
                shareable(Some("com.elsewhere"), true, 0, OTHER_DISPLAY),
                shareable(Some("com.parked"), true, 0, PARKED),
                shareable(Some("com.menubar"), true, STATUS_WINDOW_LAYER, SHOWN),
                shareable(None, true, 0, SHOWN),
            ],
            DISPLAY,
        );
        assert_eq!(showing, BTreeSet::from(["com.shown".to_string()]));
    }

    #[test]
    fn only_watched_pids_with_showing_windows_drop_a_frame() {
        let windows = [window(7, PARKED), window(9, SHOWN)];
        assert!(any_window_shows(&windows, &[9], DISPLAY));
        assert!(!any_window_shows(&[window(7, SHOWN), window(9, PARKED)], &[9], DISPLAY));
        assert!(!any_window_shows(&[window(7, SHOWN)], &[], DISPLAY));
    }

    #[test]
    fn unreadable_window_list_drops_the_frame() {
        assert_eq!(
            frame_verdict(&guard(&["com.secret"]), |_| vec![9], || None),
            FrameVerdict::DropUnreadable
        );
    }

    #[test]
    fn no_unfiltered_apps_keeps_frames_without_reading_windows() {
        let verdict = frame_verdict(
            &guard(&[]),
            |_| panic!("running apps read with nothing left out"),
            || panic!("window list read with nothing left out"),
        );
        assert_eq!(verdict, FrameVerdict::Keep);
    }

    #[test]
    fn unfiltered_app_not_running_keeps_the_frame() {
        let verdict = frame_verdict(
            &guard(&["com.secret"]),
            |_| Vec::new(),
            || panic!("window list read with no left-out app running"),
        );
        assert_eq!(verdict, FrameVerdict::Keep);
    }

    #[test]
    fn a_launched_app_showing_a_window_drops_the_frame() {
        let running = |bundle_ids: &[String]| {
            assert_eq!(bundle_ids, ["com.secret".to_string()]);
            vec![9]
        };
        assert_eq!(
            frame_verdict(&guard(&["com.secret"]), running, || Some(vec![window(
                9, SHOWN
            )])),
            FrameVerdict::DropShowing
        );
        // The guard's display rect decides: the same window on another display keeps the frame.
        assert_eq!(
            frame_verdict(&guard(&["com.secret"]), running, || Some(vec![window(
                9,
                OTHER_DISPLAY
            )])),
            FrameVerdict::Keep
        );
    }

    #[test]
    fn dropping_an_exclusion_guards_both_sets_until_the_update_lands() {
        let current = guard(&["com.a"]);
        let next = PrivacyFrameGuard {
            display: OTHER_DISPLAY,
            unfiltered_bundle_ids: vec!["com.b".to_string(), "com.a".to_string()],
        };
        let in_flight = in_flight_guard(&current, &next);
        assert_eq!(
            in_flight,
            PrivacyFrameGuard {
                display: OTHER_DISPLAY,
                unfiltered_bundle_ids: vec!["com.a".to_string(), "com.b".to_string()],
            }
        );
        assert_eq!(settled_guard(in_flight, next.clone(), true), next);
    }

    #[test]
    fn failed_update_keeps_the_union_guard() {
        let in_flight = in_flight_guard(&guard(&["com.a"]), &guard(&["com.b"]));
        assert_eq!(
            settled_guard(in_flight.clone(), guard(&["com.b"]), false),
            in_flight
        );
    }
}
