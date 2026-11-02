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

/// How long after a host time the guard still treats a frame as composed before it. A frame's
/// PTS is when it was presented; the update completion handler, or a refresh, can run while
/// frames composed before it are still on their way, so a cutoff with no margin lets the last
/// of them through.
const STALE_FRAME_MARGIN_SECS: f64 = 0.5;

/// A drop that goes on across this many guard replacements (about one per privacy poll, ~1 s)
/// is one the filter refreshes can't end — a window `CGWindowList` shows that ScreenCaptureKit's
/// window list doesn't, or updates that keep failing — so it is logged at Info, once.
const PERSISTENT_DROP_REFRESHES: u32 = 10;

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

/// A menu-bar icon: a status-layer window lying wholly within the menu-bar strip. Both
/// `SCWindow.frame`/`SCDisplay.frame` and `CGWindowList` bounds are global, top-left origin.
// ponytail: rects can't tell whether this display has a menu bar at all (secondary display
// without separate Spaces, full-screen Space, auto-hidden bar); a small status-layer window
// at the top of such a display stays exempt. Upgrade: per-display strip from NSScreen
// `frame` − `visibleFrame`.
fn is_menu_bar_icon(layer: i64, bounds: Rect, display: Rect) -> bool {
    layer == STATUS_WINDOW_LAYER
        && bounds.y >= display.y
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
    /// The processes of those apps the live filter names. ScreenCaptureKit excludes running
    /// processes, not bundle ids, so another process of an excluded app — a second copy, or
    /// one relaunched since the filter was built — isn't hidden by it and is guarded.
    pub excluded_pids: Vec<i32>,
    /// Apps a landed update started excluding, still guarded in frames presented at or before
    /// `stale_until_host_secs` (host clock): frames composed under the old filter can still be
    /// queued when the update completes.
    pub stale_bundle_ids: Vec<String>,
    /// Processes a landed update started naming — a relaunch of an app excluded before and
    /// after — still guarded in frames presented at or before `stale_until_host_secs`.
    pub stale_pids: Vec<i32>,
    pub stale_until_host_secs: f64,
    /// Host time the session last wrote this guard; it ends a drop the dropper keeps going
    /// (`PrivacyFrameDropper`).
    pub replaced_at_host_secs: f64,
}

impl PrivacyFrameGuard {
    /// The apps to guard in a frame presented at `frame_host_secs`. An unreadable time (NaN)
    /// fails closed to the stale set too.
    fn watched_bundle_ids(&self, frame_host_secs: f64) -> Vec<&str> {
        let stale_frame = self.is_stale_frame(frame_host_secs);
        let stale = self.stale_bundle_ids.iter().filter(|_| stale_frame);
        self.unfiltered_bundle_ids
            .iter()
            .chain(stale)
            .map(String::as_str)
            .collect()
    }

    fn is_stale_frame(&self, frame_host_secs: f64) -> bool {
        !composed_after(frame_host_secs, self.stale_until_host_secs)
    }
}

/// Whether a frame presented at `frame_host_secs` was composed after `host_secs`. An unreadable
/// time (NaN) says no, which keeps the frame guarded.
fn composed_after(frame_host_secs: f64, host_secs: f64) -> bool {
    frame_host_secs > host_secs + STALE_FRAME_MARGIN_SECS
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
/// (one just added to the list while it shows a window). Stale processes aren't counted as
/// excluded either.
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
        excluded_pids: next
            .excluded_pids
            .iter()
            .filter(|pid| current.excluded_pids.contains(pid) && !current.stale_pids.contains(pid))
            .copied()
            .collect(),
        ..PrivacyFrameGuard::default()
    }
}

/// The guard once the update settles, `landed_at_host_secs` being the host time it was seen to
/// land. A failed or timed-out update may still land later, so the in-flight union stays until
/// one succeeds. A landed one narrows to the new filter's, except that apps and processes it
/// started excluding stay guarded until a frame presented after it lands.
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
    let stale_pids = next
        .excluded_pids
        .iter()
        .filter(|pid| !in_flight.excluded_pids.contains(pid))
        .copied()
        .collect();
    PrivacyFrameGuard {
        stale_bundle_ids,
        stale_pids,
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
        stale_pids: current.stale_pids.clone(),
        stale_until_host_secs: current.stale_until_host_secs,
        ..next
    }
}

fn on_screen_window_shows(window: &OnScreenWindow, display: Rect) -> bool {
    match (window.layer, window.bounds) {
        (Some(layer), Some(bounds)) => window_shows(layer, bounds, display),
        _ => true,
    }
}

/// Whether any of `windows` belongs to `pids` and shows on `display`.
pub(crate) fn any_window_shows(windows: &[OnScreenWindow], pids: &[i32], display: Rect) -> bool {
    windows
        .iter()
        .any(|window| pids.contains(&window.pid) && on_screen_window_shows(window, display))
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
/// when a process the filter leaves out is running, which keeps the common case (nothing left
/// out) free.
pub(crate) fn frame_verdict(
    guard: &PrivacyFrameGuard,
    frame_host_secs: f64,
    mut running_pids: impl FnMut(&[&str]) -> Vec<i32>,
    windows: impl FnOnce() -> Option<Vec<OnScreenWindow>>,
) -> FrameVerdict {
    let watched = guard.watched_bundle_ids(frame_host_secs);
    let mut pids = Vec::new();
    if !watched.is_empty() {
        pids = running_pids(&watched);
    }
    if !guard.excluded_bundle_ids.is_empty() {
        let stale_frame = guard.is_stale_frame(frame_host_secs);
        let excluded = guard
            .excluded_bundle_ids
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        pids.extend(running_pids(&excluded).into_iter().filter(|pid| {
            !guard.excluded_pids.contains(pid) || (stale_frame && guard.stale_pids.contains(pid))
        }));
    }
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

/// The stream handler's side of the guard: reads it per frame and logs state changes. Segment
/// rotation hands it to the next segment, so a drop it keeps going survives the rotation.
#[derive(Debug, Default)]
pub(crate) struct PrivacyFrameDropper {
    guard: SharedPrivacyFrameGuard,
    dropped_frames: u64,
    window_list_unreadable: bool,
    /// Guard replacements the current drop has gone on across, and the last one's stamp.
    drop_refreshes: u32,
    drop_seen_under: Option<f64>,
    persistent_drop_reported: bool,
    /// `replaced_at_host_secs` of the guard a watched app was last seen showing under.
    showing_seen_under: Option<f64>,
}

impl PrivacyFrameDropper {
    pub fn new(guard: SharedPrivacyFrameGuard) -> Self {
        Self {
            guard,
            ..Self::default()
        }
    }

    /// Whether the frame the stream just delivered, presented at `frame_host_secs` (its PTS,
    /// host clock), must be dropped.
    pub fn frame_must_be_dropped(&mut self, frame_host_secs: f64) -> bool {
        // Copy and release: the checks below call into AppKit and WindowServer.
        let guard = self
            .guard
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let verdict = self.verdict(&guard, frame_host_secs, running_pids, on_screen_windows);
        self.record(verdict, guard.replaced_at_host_secs)
    }

    /// The check runs after the frame was composed, so a window that showed in it may have
    /// closed by now. Once a watched app is seen showing, frames keep dropping until the session
    /// next replaces the guard — its refresh re-read the windows — and for frames presented up
    /// to the stale margin after that, which can still have been composed before it.
    fn verdict(
        &mut self,
        guard: &PrivacyFrameGuard,
        frame_host_secs: f64,
        running_pids: impl FnMut(&[&str]) -> Vec<i32>,
        windows: impl FnOnce() -> Option<Vec<OnScreenWindow>>,
    ) -> FrameVerdict {
        if let Some(seen_under) = self.showing_seen_under {
            // Same stamp, same guard: a host clock that didn't move keeps dropping.
            if seen_under == guard.replaced_at_host_secs
                || !composed_after(frame_host_secs, guard.replaced_at_host_secs)
            {
                return FrameVerdict::DropShowing;
            }
            self.showing_seen_under = None;
        }
        let verdict = frame_verdict(guard, frame_host_secs, running_pids, windows);
        if verdict == FrameVerdict::DropShowing {
            self.showing_seen_under = Some(guard.replaced_at_host_secs);
        }
        verdict
    }

    /// `guard_replaced_at_host_secs` stamps the guard the verdict was made under.
    fn record(&mut self, verdict: FrameVerdict, guard_replaced_at_host_secs: f64) -> bool {
        if verdict == FrameVerdict::Keep {
            // Only a kept frame ends an unreadable spell, so a list flapping between unreadable
            // and showing warns once.
            self.window_list_unreadable = false;
            self.drop_refreshes = 0;
            self.drop_seen_under = None;
            self.persistent_drop_reported = false;
            if self.dropped_frames > 0 {
                capture_runtime::debug_log!(
                    "privacy frame guard stopped dropping frames after {} frame(s)",
                    self.dropped_frames
                );
                self.dropped_frames = 0;
            }
            return false;
        }
        if verdict == FrameVerdict::DropUnreadable && !self.window_list_unreadable {
            self.window_list_unreadable = true;
            capture_runtime::warn_log!(
                "privacy frame guard can't read the window list; dropping frames until it can"
            );
        }
        if self.dropped_frames == 0 {
            let reason = match verdict {
                FrameVerdict::DropUnreadable => "the window list can't be read",
                _ => "a left-out excluded app shows a window",
            };
            capture_runtime::debug_log!("privacy frame guard dropping frames: {reason}");
        }
        self.dropped_frames += 1;
        if self.drop_seen_under != Some(guard_replaced_at_host_secs) {
            self.drop_seen_under = Some(guard_replaced_at_host_secs);
            self.drop_refreshes += 1;
        }
        if self.drop_refreshes >= PERSISTENT_DROP_REFRESHES && !self.persistent_drop_reported {
            self.persistent_drop_reported = true;
            capture_runtime::info_log!(
                "privacy frame guard still dropping frames after {} filter refreshes ({} frame(s)); the privacy filter can't hide a window the guard sees",
                self.drop_refreshes,
                self.dropped_frames
            );
        }
        true
    }
}

/// Which processes show a window on a display, as `CGWindowList` sees them. The privacy poll
/// takes one just before each shareable-content read; while a later one is the same and the
/// filter excludes nothing, the read would find nothing new to hide: a listed app that starts
/// showing a window — even one launched too recently to look up by bundle id — adds its pid.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ShowingSnapshot {
    display: Rect,
    pids: BTreeSet<i32>,
}

impl ShowingSnapshot {
    fn new(windows: &[OnScreenWindow], display: Rect) -> Self {
        Self {
            display,
            pids: windows
                .iter()
                .filter(|window| on_screen_window_shows(window, display))
                .map(|window| window.pid)
                .collect(),
        }
    }

    /// `None` when the display or the window list can't be read.
    pub fn read(display_id: u32) -> Option<Self> {
        let bounds = cidre::cg::DirectDisplayId(display_id).bounds();
        let display = Rect {
            x: bounds.origin.x,
            y: bounds.origin.y,
            width: bounds.size.width,
            height: bounds.size.height,
        };
        // A display that is gone reads as a zero rect.
        if display.width <= 0.0 || display.height <= 0.0 {
            return None;
        }
        Some(Self::new(&on_screen_windows()?, display))
    }

    #[cfg(test)]
    pub fn for_test(pids: &[i32]) -> Self {
        Self {
            display: Rect::default(),
            pids: pids.iter().copied().collect(),
        }
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
/// Desktop elements stay in: the shareable content the plan reads lists them too (Finder's
/// desktop icons and the Dock's wallpaper span the display), so a listed app's desktop-level
/// window counts for both, and other apps' don't matter to either.
fn on_screen_windows() -> Option<Vec<OnScreenWindow>> {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_foundation::string::{CFString, CFStringRef};
    use core_graphics::window::{
        copy_window_info, kCGNullWindowID, kCGWindowBounds, kCGWindowLayer,
        kCGWindowListOptionOnScreenOnly, kCGWindowOwnerPID,
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
    thread_local! {
        // CFString isn't Send/Sync, so built once per thread rather than in a static.
        static RECT_KEYS: [CFString; 4] = ["X", "Y", "Width", "Height"].map(CFString::new);
    }

    let infos = copy_window_info(kCGWindowListOptionOnScreenOnly, kCGNullWindowID)?;
    Some(RECT_KEYS.with(|[x, y, width, height]| {
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
                            x: number(&bounds, x)?,
                            y: number(&bounds, y)?,
                            width: number(&bounds, width)?,
                            height: number(&bounds, height)?,
                        })
                    });
                Some(OnScreenWindow {
                    pid: number(&dict, &pid_key)? as i32,
                    layer: number(&dict, &layer_key).map(|layer| layer as i64),
                    bounds,
                })
            })
            .collect()
    }))
}

#[cfg(test)]
mod tests;
