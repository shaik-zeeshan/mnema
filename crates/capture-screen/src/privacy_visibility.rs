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
/// would keep the costly filter on all day. Popup menus (101) and floating panels (3) opened
/// from the icon still count — and so does an `NSPopover` opened from it, which sits on this
/// same layer, so only a window inside the menu-bar strip is exempt.
const STATUS_WINDOW_LAYER: i64 = 25;

/// The menu-bar strip, from the top of the display. Status items measure 39 pt tall on a
/// notched M4 Air (24 pt on bars without a notch); a popover measured 326×226 pt.
const MENU_BAR_STRIP_PT: f64 = 50.0;

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

/// A menu-bar icon: a status-layer window no taller than the menu bar and lying within it. Both
/// `SCWindow.frame`/`SCDisplay.frame` and `CGWindowList` bounds are global, top-left origin.
fn is_menu_bar_icon(layer: i64, bounds: Rect, display: Rect) -> bool {
    layer == STATUS_WINDOW_LAYER
        && bounds.height <= MENU_BAR_STRIP_PT
        && bounds.y + bounds.height <= display.y + MENU_BAR_STRIP_PT
}

/// The one rule both the filter plan (`SCWindow`) and the frame guard (`CGWindowList`) use, so
/// an app the plan leaves out is exactly one the guard treats as hidden.
pub(crate) fn window_shows(layer: i64, bounds: Rect, display: Rect) -> bool {
    !is_menu_bar_icon(layer, bounds, display) && shows_on_display(bounds, display)
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

/// An on-screen window from `CGWindowList`, as the frame guard sees it. A missing layer or
/// bounds can't prove the window hidden, so it counts as showing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct OnScreenWindow {
    pub pid: i32,
    pub layer: Option<i64>,
    pub bounds: Option<Rect>,
}

/// What the live filter does about the privacy list on the display it captures. The session
/// writes it around each filter update; the stream handler reads it per frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct PrivacyFrameGuard {
    pub display: Rect,
    /// Listed apps the live filter leaves out (no window showed when it was built): guarded.
    pub unfiltered_bundle_ids: Vec<String>,
    /// Listed apps the live filter excludes — after a failed update, the ones both the old and
    /// the new filter exclude, since either may be live.
    pub excluded_bundle_ids: Vec<String>,
    /// Apps a landed update started excluding, still guarded in frames presented at or before
    /// `stale_until_host_secs` (host clock): frames composed under the old filter can still be
    /// queued when the update completes.
    pub stale_bundle_ids: Vec<String>,
    pub stale_until_host_secs: f64,
}

impl PrivacyFrameGuard {
    /// The apps to guard in a frame presented at `frame_host_secs`. An unreadable time (NaN)
    /// fails closed to the stale set too.
    fn watched_bundle_ids(&self, frame_host_secs: f64) -> Vec<&str> {
        let stale_frame = frame_host_secs.partial_cmp(&self.stale_until_host_secs)
            != Some(std::cmp::Ordering::Greater);
        let stale = self.stale_bundle_ids.iter().filter(|_| stale_frame);
        self.unfiltered_bundle_ids
            .iter()
            .chain(stale)
            .map(String::as_str)
            .collect()
    }
}

pub(crate) type SharedPrivacyFrameGuard = Arc<Mutex<PrivacyFrameGuard>>;

fn push_missing(into: &mut Vec<String>, from: &[String]) {
    for bundle_id in from {
        if !into.contains(bundle_id) {
            into.push(bundle_id.clone());
        }
    }
}

/// The guard while a filter update is in flight: every listed app either filter may leave
/// out — the current one's left-out and stale apps, the next one's left-out apps (it may land
/// before the update returns), and the apps the next one excludes that the live one doesn't yet
/// (one just added to the list while it shows a window).
pub(crate) fn in_flight_guard(
    current: &PrivacyFrameGuard,
    next: &PrivacyFrameGuard,
) -> PrivacyFrameGuard {
    let mut unfiltered_bundle_ids = current.unfiltered_bundle_ids.clone();
    push_missing(&mut unfiltered_bundle_ids, &current.stale_bundle_ids);
    push_missing(&mut unfiltered_bundle_ids, &next.unfiltered_bundle_ids);
    let newly_excluded = next
        .excluded_bundle_ids
        .iter()
        .filter(|bundle_id| !current.excluded_bundle_ids.contains(bundle_id))
        .cloned()
        .collect::<Vec<_>>();
    push_missing(&mut unfiltered_bundle_ids, &newly_excluded);
    PrivacyFrameGuard {
        display: next.display,
        unfiltered_bundle_ids,
        excluded_bundle_ids: next
            .excluded_bundle_ids
            .iter()
            .filter(|bundle_id| current.excluded_bundle_ids.contains(bundle_id))
            .cloned()
            .collect(),
        stale_bundle_ids: Vec::new(),
        stale_until_host_secs: 0.0,
    }
}

/// The guard once the update settles, `landed_at_host_secs` being the host time it was seen to
/// land. A failed or timed-out update may still land later, so the in-flight union stays until
/// one succeeds. A landed one narrows to the new filter's, except that apps it started
/// excluding stay guarded until a frame presented after it lands.
pub(crate) fn settled_guard(
    in_flight: PrivacyFrameGuard,
    next: PrivacyFrameGuard,
    landed_at_host_secs: Option<f64>,
) -> PrivacyFrameGuard {
    let Some(landed_at_host_secs) = landed_at_host_secs else {
        return in_flight;
    };
    let stale_bundle_ids = in_flight
        .unfiltered_bundle_ids
        .into_iter()
        .filter(|bundle_id| !next.unfiltered_bundle_ids.contains(bundle_id))
        .collect();
    PrivacyFrameGuard {
        stale_bundle_ids,
        stale_until_host_secs: landed_at_host_secs,
        ..next
    }
}

/// The guard after a refresh that keeps the live filter: the new left-out apps, keeping any
/// apps a recent update still guards in stale frames.
pub(crate) fn refreshed_guard(
    current: &PrivacyFrameGuard,
    next: PrivacyFrameGuard,
) -> PrivacyFrameGuard {
    PrivacyFrameGuard {
        stale_bundle_ids: current.stale_bundle_ids.clone(),
        stale_until_host_secs: current.stale_until_host_secs,
        ..next
    }
}

/// Whether any of `windows` belongs to `pids` and shows on `display`.
pub(crate) fn any_window_shows(windows: &[OnScreenWindow], pids: &[i32], display: Rect) -> bool {
    windows.iter().any(|window| {
        pids.contains(&window.pid)
            && match (window.layer, window.bounds) {
                (Some(layer), Some(bounds)) => window_shows(layer, bounds, display),
                _ => true,
            }
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

/// Whether a frame presented at `frame_host_secs` must be dropped. The window list is read only
/// when an app the filter leaves out is running, which keeps the common case (nothing left out)
/// free.
pub(crate) fn frame_verdict(
    guard: &PrivacyFrameGuard,
    frame_host_secs: f64,
    running_pids: impl FnOnce(&[&str]) -> Vec<i32>,
    windows: impl FnOnce() -> Option<Vec<OnScreenWindow>>,
) -> FrameVerdict {
    let watched = guard.watched_bundle_ids(frame_host_secs);
    if watched.is_empty() {
        return FrameVerdict::Keep;
    }
    let pids = running_pids(&watched);
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

    /// Whether the frame the stream just delivered, presented at `frame_host_secs` (its PTS,
    /// host clock), must be dropped. The check runs after the frame was composed, so a window
    /// that showed in it still shows here.
    pub fn frame_must_be_dropped(&mut self, frame_host_secs: f64) -> bool {
        // Copy and release: the checks below call into AppKit and WindowServer.
        let guard = self
            .guard
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let verdict = frame_verdict(&guard, frame_host_secs, running_pids, on_screen_windows);
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

fn running_pids(bundle_ids: &[&str]) -> Vec<i32> {
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

/// On-screen windows with an owner pid. The guard only looks at listed apps' pids, so it needs
/// no `sharingType` rule of its own; one with no pid can't be attributed and is skipped.
fn on_screen_windows() -> Option<Vec<OnScreenWindow>> {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_foundation::string::{CFString, CFStringRef};
    use core_graphics::window::{
        copy_window_info, kCGNullWindowID, kCGWindowBounds, kCGWindowLayer,
        kCGWindowListExcludeDesktopElements, kCGWindowListOptionOnScreenOnly, kCGWindowOwnerPID,
    };

    fn value(dict: &CFDictionary, key: &CFString) -> Option<CFType> {
        let value = dict.find(key.as_CFTypeRef())?;
        // SAFETY: CGWindowList dictionaries hold CF objects as values, kept alive by `dict`;
        // the get rule retains the wrapper for as long as it lives.
        Some(unsafe { CFType::wrap_under_get_rule(*value) })
    }
    fn number(dict: &CFDictionary, key: &CFString) -> Option<f64> {
        value(dict, key)?
            .downcast::<CFNumber>()?
            .to_f64()
            .filter(|number| number.is_finite())
    }
    // SAFETY: the `kCGWindow*` keys are immutable CFString constants exported by CoreGraphics.
    let key = |key: CFStringRef| unsafe { CFString::wrap_under_get_rule(key) };
    // SAFETY: reading the CoreGraphics key statics; they are initialised before any Rust code
    // runs and never written.
    let (pid_key, layer_key, bounds_key) = unsafe {
        (
            key(kCGWindowOwnerPID),
            key(kCGWindowLayer),
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
                let bounds = value(&dict, &bounds_key)
                    .and_then(|bounds| bounds.downcast::<CFDictionary>())
                    .and_then(|bounds| {
                        Some(Rect {
                            x: number(&bounds, &x)?,
                            y: number(&bounds, &y)?,
                            width: number(&bounds, &width)?,
                            height: number(&bounds, &height)?,
                        })
                    });
                Some(OnScreenWindow {
                    pid: number(&dict, &pid_key)? as i32,
                    layer: number(&dict, &layer_key).map(|layer| layer as i64),
                    bounds,
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

    /// Measured: a Control Centre status item on a notched M4 Air.
    const MENU_BAR_ICON: Rect = Rect {
        x: 1403.0,
        y: 0.0,
        width: 34.0,
        height: 39.0,
    };
    /// Measured: an `NSPopover` opened from a status item, also on the status layer.
    const POPOVER: Rect = Rect {
        x: 1250.0,
        y: 0.0,
        width: 326.0,
        height: 226.0,
    };

    fn window(pid: i32, bounds: Rect) -> OnScreenWindow {
        OnScreenWindow {
            pid,
            layer: Some(0),
            bounds: Some(bounds),
        }
    }

    fn ids(bundle_ids: &[&str]) -> Vec<String> {
        bundle_ids.iter().map(|id| id.to_string()).collect()
    }

    fn guard(bundle_ids: &[&str]) -> PrivacyFrameGuard {
        PrivacyFrameGuard {
            display: DISPLAY,
            unfiltered_bundle_ids: ids(bundle_ids),
            ..PrivacyFrameGuard::default()
        }
    }

    fn filter_guard(unfiltered: &[&str], excluded: &[&str]) -> PrivacyFrameGuard {
        PrivacyFrameGuard {
            excluded_bundle_ids: ids(excluded),
            ..guard(unfiltered)
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
    fn a_menu_bar_icon_does_not_show_but_its_menus_panels_and_popovers_do() {
        assert!(!window_shows(STATUS_WINDOW_LAYER, MENU_BAR_ICON, DISPLAY));
        assert!(window_shows(STATUS_WINDOW_LAYER, POPOVER, DISPLAY));
        assert!(window_shows(STATUS_WINDOW_LAYER, SHOWN, DISPLAY));
        assert!(window_shows(101, MENU_BAR_ICON, DISPLAY));
        assert!(window_shows(101, SHOWN, DISPLAY));
        assert!(window_shows(3, SHOWN, DISPLAY));
        assert!(window_shows(0, SHOWN, DISPLAY));
    }

    #[test]
    fn a_status_layer_strip_below_the_menu_bar_shows() {
        let lower = Rect {
            y: 300.0,
            ..MENU_BAR_ICON
        };
        assert!(window_shows(STATUS_WINDOW_LAYER, lower, DISPLAY));
        // The menu bar of a display below the main one sits at that display's top.
        let below = Rect {
            y: 1112.0,
            ..DISPLAY
        };
        let icon_there = Rect {
            y: 1112.0,
            ..MENU_BAR_ICON
        };
        assert!(!window_shows(STATUS_WINDOW_LAYER, icon_there, below));
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
                shareable(
                    Some("com.menubar"),
                    true,
                    STATUS_WINDOW_LAYER,
                    MENU_BAR_ICON,
                ),
                shareable(Some("com.popover"), true, STATUS_WINDOW_LAYER, POPOVER),
                shareable(None, true, 0, SHOWN),
            ],
            DISPLAY,
        );
        assert_eq!(
            showing,
            BTreeSet::from(["com.popover".to_string(), "com.shown".to_string()])
        );
    }

    #[test]
    fn only_watched_pids_with_showing_windows_drop_a_frame() {
        let windows = [window(7, PARKED), window(9, SHOWN)];
        assert!(any_window_shows(&windows, &[9], DISPLAY));
        assert!(!any_window_shows(&[window(7, SHOWN), window(9, PARKED)], &[9], DISPLAY));
        assert!(!any_window_shows(&[window(7, SHOWN)], &[], DISPLAY));
    }

    #[test]
    fn a_watched_window_without_layer_or_bounds_counts_as_showing() {
        let no_bounds = OnScreenWindow {
            bounds: None,
            ..window(9, PARKED)
        };
        let no_layer = OnScreenWindow {
            layer: None,
            ..window(9, PARKED)
        };
        assert!(any_window_shows(&[no_bounds], &[9], DISPLAY));
        assert!(any_window_shows(&[no_layer], &[9], DISPLAY));
        assert!(!any_window_shows(&[no_bounds], &[7], DISPLAY));
    }

    #[test]
    fn unreadable_window_list_drops_the_frame() {
        assert_eq!(
            frame_verdict(&guard(&["com.secret"]), 1.0, |_| vec![9], || None),
            FrameVerdict::DropUnreadable
        );
    }

    #[test]
    fn no_unfiltered_apps_keeps_frames_without_reading_windows() {
        let verdict = frame_verdict(
            &guard(&[]),
            1.0,
            |_| panic!("running apps read with nothing left out"),
            || panic!("window list read with nothing left out"),
        );
        assert_eq!(verdict, FrameVerdict::Keep);
    }

    #[test]
    fn unfiltered_app_not_running_keeps_the_frame() {
        let verdict = frame_verdict(
            &guard(&["com.secret"]),
            1.0,
            |_| Vec::new(),
            || panic!("window list read with no left-out app running"),
        );
        assert_eq!(verdict, FrameVerdict::Keep);
    }

    #[test]
    fn a_launched_app_showing_a_window_drops_the_frame() {
        let running = |bundle_ids: &[&str]| {
            assert_eq!(bundle_ids, ["com.secret"]);
            vec![9]
        };
        assert_eq!(
            frame_verdict(&guard(&["com.secret"]), 1.0, running, || Some(vec![
                window(9, SHOWN)
            ])),
            FrameVerdict::DropShowing
        );
        // The guard's display rect decides: the same window on another display keeps the frame.
        assert_eq!(
            frame_verdict(&guard(&["com.secret"]), 1.0, running, || Some(vec![
                window(9, OTHER_DISPLAY)
            ])),
            FrameVerdict::Keep
        );
    }

    #[test]
    fn dropping_an_exclusion_guards_both_sets_until_the_update_lands() {
        let current = filter_guard(&["com.a"], &["com.b"]);
        let next = PrivacyFrameGuard {
            display: OTHER_DISPLAY,
            ..filter_guard(&["com.b", "com.a"], &[])
        };
        let in_flight = in_flight_guard(&current, &next);
        assert_eq!(
            in_flight,
            PrivacyFrameGuard {
                display: OTHER_DISPLAY,
                unfiltered_bundle_ids: ids(&["com.a", "com.b"]),
                ..PrivacyFrameGuard::default()
            }
        );
        // Nothing narrows: the stale set is empty.
        let settled = settled_guard(in_flight, next.clone(), Some(5.0));
        assert_eq!(settled.unfiltered_bundle_ids, next.unfiltered_bundle_ids);
        assert!(settled.stale_bundle_ids.is_empty());
    }

    #[test]
    fn an_app_added_to_the_list_while_it_shows_is_guarded_in_flight() {
        let current = filter_guard(&[], &["com.kept"]);
        let next = filter_guard(&[], &["com.kept", "com.added"]);
        let in_flight = in_flight_guard(&current, &next);
        assert_eq!(in_flight.unfiltered_bundle_ids, ids(&["com.added"]));
        assert_eq!(in_flight.excluded_bundle_ids, ids(&["com.kept"]));
    }

    #[test]
    fn an_app_the_update_starts_excluding_stays_guarded_in_stale_frames() {
        let current = filter_guard(&["com.a"], &[]);
        let next = filter_guard(&[], &["com.a"]);
        let settled = settled_guard(in_flight_guard(&current, &next), next.clone(), Some(5.0));
        assert_eq!(settled.unfiltered_bundle_ids, Vec::<String>::new());
        assert_eq!(settled.excluded_bundle_ids, ids(&["com.a"]));
        assert_eq!(settled.watched_bundle_ids(4.9), ["com.a"]);
        assert_eq!(settled.watched_bundle_ids(5.0), ["com.a"]);
        assert_eq!(settled.watched_bundle_ids(f64::NAN), ["com.a"]);
        assert!(settled.watched_bundle_ids(5.1).is_empty());

        // A refresh that keeps the filter keeps the stale set.
        let refreshed = refreshed_guard(&settled, filter_guard(&["com.b"], &["com.a"]));
        assert_eq!(refreshed.watched_bundle_ids(4.9), ["com.b", "com.a"]);
        assert_eq!(refreshed.watched_bundle_ids(5.1), ["com.b"]);

        // The next update guards the stale apps too while it is in flight.
        let in_flight = in_flight_guard(&settled, &filter_guard(&[], &["com.a"]));
        assert_eq!(in_flight.unfiltered_bundle_ids, ids(&["com.a"]));
    }

    #[test]
    fn a_stale_frame_drops_on_an_app_the_update_started_excluding() {
        let next = filter_guard(&[], &["com.a"]);
        let settled = settled_guard(in_flight_guard(&guard(&["com.a"]), &next), next, Some(5.0));
        let showing = || Some(vec![window(9, SHOWN)]);
        assert_eq!(
            frame_verdict(&settled, 4.0, |_| vec![9], showing),
            FrameVerdict::DropShowing
        );
        assert_eq!(
            frame_verdict(&settled, 6.0, |_| panic!("nothing watched"), showing),
            FrameVerdict::Keep
        );
    }

    #[test]
    fn failed_update_keeps_the_union_guard() {
        let current = filter_guard(&["com.a"], &["com.x", "com.y"]);
        let next = filter_guard(&["com.b"], &["com.x"]);
        let in_flight = in_flight_guard(&current, &next);
        assert_eq!(in_flight.excluded_bundle_ids, ids(&["com.x"]));
        assert_eq!(settled_guard(in_flight.clone(), next, None), in_flight);
    }

    #[test]
    fn the_dropper_drops_on_drop_verdicts_and_keeps_on_keep() {
        let mut dropper = PrivacyFrameDropper::default();
        assert!(dropper.record(FrameVerdict::DropShowing));
        assert!(dropper.record(FrameVerdict::DropUnreadable));
        assert!(dropper.window_list_unreadable);
        assert!(!dropper.record(FrameVerdict::Keep));
        assert!(!dropper.window_list_unreadable);
        assert_eq!(dropper.dropped_frames, 0);
    }
}
