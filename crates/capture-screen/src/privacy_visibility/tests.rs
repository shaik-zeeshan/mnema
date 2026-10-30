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
fn a_status_layer_window_reaching_into_the_strip_from_above_shows() {
    // A display below the main one; the window hangs over the seam from the one above.
    let below = Rect {
        y: 1112.0,
        ..DISPLAY
    };
    let hanging = Rect {
        x: 400.0,
        y: 1112.0 - 20.0,
        width: 300.0,
        height: 45.0,
    };
    assert!(window_shows(STATUS_WINDOW_LAYER, hanging, below));
}

#[test]
fn showing_bundle_ids_skips_hidden_elsewhere_ownerless_and_status_windows() {
    let shareable =
        |owner: Option<&str>, on_screen: bool, layer: i64, bounds: Rect| ShareableWindow {
            owner_bundle_id: owner.map(str::to_string),
            on_screen,
            layer,
            bounds,
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
    assert!(!any_window_shows(
        &[window(7, SHOWN), window(9, PARKED)],
        &[9],
        DISPLAY
    ));
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
    assert_eq!(settled.watched_bundle_ids(f64::NAN), ["com.a"]);
    assert!(settled.watched_bundle_ids(5.6).is_empty());

    // A refresh that keeps the filter keeps the stale set.
    let refreshed = refreshed_guard(&settled, filter_guard(&["com.b"], &["com.a"]));
    assert_eq!(refreshed.watched_bundle_ids(4.9), ["com.b", "com.a"]);
    assert_eq!(refreshed.watched_bundle_ids(5.6), ["com.b"]);

    // The next update guards the stale apps too while it is in flight.
    let in_flight = in_flight_guard(&settled, &filter_guard(&[], &["com.a"]));
    assert_eq!(in_flight.unfiltered_bundle_ids, ids(&["com.a"]));
}

#[test]
fn a_stale_frame_drops_on_an_app_the_update_started_excluding() {
    let next = PrivacyFrameGuard {
        excluded_pids: vec![9],
        ..filter_guard(&[], &["com.a"])
    };
    let settled = settled_guard(in_flight_guard(&guard(&["com.a"]), &next), next, Some(5.0));
    let showing = || Some(vec![window(9, SHOWN)]);
    assert_eq!(
        frame_verdict(&settled, 4.0, |_| vec![9], showing),
        FrameVerdict::DropShowing
    );
    assert_eq!(
        frame_verdict(&settled, 6.0, |_| vec![9], showing),
        FrameVerdict::Keep
    );
}

#[test]
fn the_stale_cutoff_covers_frames_presented_just_after_the_update_landed() {
    let next = filter_guard(&[], &["com.a"]);
    let settled = settled_guard(in_flight_guard(&guard(&["com.a"]), &next), next, Some(5.0));
    // Composed under the old filter, presented just after the completion handler ran.
    assert_eq!(settled.watched_bundle_ids(5.0), ["com.a"]);
    assert_eq!(settled.watched_bundle_ids(5.3), ["com.a"]);
    assert_eq!(
        settled.watched_bundle_ids(5.0 + STALE_FRAME_MARGIN_SECS),
        ["com.a"]
    );
    assert!(settled
        .watched_bundle_ids(5.0 + STALE_FRAME_MARGIN_SECS + 0.01)
        .is_empty());
}

#[test]
fn a_second_process_of_an_excluded_app_is_guarded() {
    let excluded = PrivacyFrameGuard {
        excluded_pids: vec![9],
        ..filter_guard(&[], &["com.a"])
    };
    let running = |bundle_ids: &[&str]| {
        assert_eq!(bundle_ids, ["com.a"]);
        vec![9, 12]
    };
    // The new copy (pid 12) shows a window the filter, built for pid 9, doesn't hide.
    assert_eq!(
        frame_verdict(&excluded, 1.0, running, || Some(vec![window(12, SHOWN)])),
        FrameVerdict::DropShowing
    );
    // The filtered copy showing is fine: the filter hides it.
    assert_eq!(
        frame_verdict(&excluded, 1.0, running, || Some(vec![window(9, SHOWN)])),
        FrameVerdict::Keep
    );
    // Only the filtered copy runs: the window list isn't even read.
    assert_eq!(
        frame_verdict(&excluded, 1.0, |_| vec![9], || panic!("nothing watched")),
        FrameVerdict::Keep
    );
    // In flight, only the pids both filters name count as excluded.
    let next = PrivacyFrameGuard {
        excluded_pids: vec![9, 12],
        ..filter_guard(&[], &["com.a"])
    };
    assert_eq!(in_flight_guard(&excluded, &next).excluded_pids, vec![9]);
}

#[test]
fn a_process_the_update_starts_excluding_stays_guarded_in_stale_frames() {
    // com.a is excluded before and after; the update adds its relaunched copy, pid 12.
    let current = PrivacyFrameGuard {
        excluded_pids: vec![9],
        ..filter_guard(&[], &["com.a"])
    };
    let next = PrivacyFrameGuard {
        excluded_pids: vec![9, 12],
        ..filter_guard(&[], &["com.a"])
    };
    let settled = settled_guard(in_flight_guard(&current, &next), next.clone(), Some(5.0));
    let running = |_: &[&str]| vec![9, 12];
    let showing = || Some(vec![window(12, SHOWN)]);
    // Composed under the old filter, which didn't name pid 12.
    assert_eq!(
        frame_verdict(&settled, 4.9, running, showing),
        FrameVerdict::DropShowing
    );
    assert_eq!(
        frame_verdict(&settled, f64::NAN, running, showing),
        FrameVerdict::DropShowing
    );
    assert_eq!(
        frame_verdict(&settled, 5.6, running, showing),
        FrameVerdict::Keep
    );
    // A refresh that keeps the filter keeps it guarded in stale frames.
    let refreshed = refreshed_guard(&settled, next.clone());
    assert_eq!(
        frame_verdict(&refreshed, 4.9, running, showing),
        FrameVerdict::DropShowing
    );
    // So does the next update while it is in flight.
    let in_flight = in_flight_guard(&settled, &next);
    assert_eq!(
        frame_verdict(&in_flight, 4.9, running, showing),
        FrameVerdict::DropShowing
    );
}

#[test]
fn a_window_seen_showing_keeps_dropping_until_the_guard_is_replaced() {
    let mut dropper = PrivacyFrameDropper::default();
    let watched = PrivacyFrameGuard {
        replaced_at_host_secs: 1.0,
        ..guard(&["com.secret"])
    };
    let shown = || Some(vec![window(9, SHOWN)]);
    let closed = || Some(Vec::new());
    assert_eq!(
        dropper.verdict(&watched, 2.0, |_| vec![9], shown),
        FrameVerdict::DropShowing
    );
    // Closed by the time frame N+1 is checked, which may have been composed with it.
    assert_eq!(
        dropper.verdict(&watched, 3.0, |_| vec![9], closed),
        FrameVerdict::DropShowing
    );
    // The session replaced the guard at 10: frames up to the margin after it still drop.
    let replaced = PrivacyFrameGuard {
        replaced_at_host_secs: 10.0,
        ..watched.clone()
    };
    assert_eq!(
        dropper.verdict(&replaced, 10.4, |_| vec![9], closed),
        FrameVerdict::DropShowing
    );
    assert_eq!(
        dropper.verdict(&replaced, f64::NAN, |_| vec![9], closed),
        FrameVerdict::DropShowing
    );
    assert_eq!(
        dropper.verdict(&replaced, 10.6, |_| vec![9], closed),
        FrameVerdict::Keep
    );
    assert_eq!(dropper.showing_seen_under, None);
    // An unreadable window list drops that frame only.
    assert_eq!(
        dropper.verdict(&replaced, 11.0, |_| vec![9], || None),
        FrameVerdict::DropUnreadable
    );
    assert_eq!(
        dropper.verdict(&replaced, 12.0, |_| vec![9], closed),
        FrameVerdict::Keep
    );
}

#[test]
fn a_rebuild_after_a_failed_update_keeps_the_union_guard() {
    let current = filter_guard(&["com.a"], &["com.x"]);
    let failed = settled_guard(
        in_flight_guard(&current, &filter_guard(&["com.x"], &["com.a"])),
        filter_guard(&["com.x"], &["com.a"]),
        None,
    );
    assert_eq!(failed.unfiltered_bundle_ids, ids(&["com.a", "com.x"]));
    // The next poll rebuilds (no state is recorded as applied): its in-flight guard keeps
    // every app the failed union guards, even if the new plan is the old filter again.
    let rebuild = in_flight_guard(&failed, &current);
    assert_eq!(rebuild.unfiltered_bundle_ids, ids(&["com.a", "com.x"]));
}

#[test]
fn a_display_left_of_and_above_the_main_one_is_guarded_in_global_coordinates() {
    // A secondary display arranged up and to the left: negative origin, top-left based.
    let left = Rect {
        x: -1920.0,
        y: -300.0,
        width: 1920.0,
        height: 1080.0,
    };
    let there = Rect {
        x: -1500.0,
        y: -100.0,
        width: 600.0,
        height: 400.0,
    };
    assert!(window_shows(0, there, left));
    assert!(!window_shows(0, there, DISPLAY));
    assert!(!window_shows(0, SHOWN, left));
    // Its menu bar sits at its own top, y = -300.
    let icon = Rect {
        x: -400.0,
        y: -300.0,
        ..MENU_BAR_ICON
    };
    assert!(!window_shows(STATUS_WINDOW_LAYER, icon, left));
    let popover = Rect {
        x: -400.0,
        y: -300.0,
        ..POPOVER
    };
    assert!(window_shows(STATUS_WINDOW_LAYER, popover, left));
    // A window straddling the seam shows on both.
    let straddling = Rect {
        x: -200.0,
        y: 100.0,
        width: 400.0,
        height: 300.0,
    };
    assert!(window_shows(0, straddling, left));
    assert!(window_shows(0, straddling, DISPLAY));
}

#[test]
fn the_showing_snapshot_counts_only_windows_that_show_on_the_display() {
    let snapshot = ShowingSnapshot::new(
        &[
            window(7, SHOWN),
            window(8, PARKED),
            window(9, OTHER_DISPLAY),
            OnScreenWindow {
                layer: Some(STATUS_WINDOW_LAYER),
                ..window(10, MENU_BAR_ICON)
            },
            OnScreenWindow {
                bounds: None,
                ..window(11, PARKED)
            },
        ],
        DISPLAY,
    );
    assert_eq!(snapshot.pids, BTreeSet::from([7, 11]));
    assert_ne!(snapshot, ShowingSnapshot::new(&[window(7, SHOWN)], DISPLAY));
    // The display moved: not the same snapshot.
    let moved = Rect { x: 10.0, ..DISPLAY };
    assert_ne!(
        ShowingSnapshot::new(&[window(7, SHOWN)], DISPLAY),
        ShowingSnapshot::new(&[window(7, SHOWN)], moved)
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
fn a_drop_that_outlasts_many_filter_refreshes_is_reported_once() {
    // The filter plan (ScreenCaptureKit's window list) never excludes an app the guard
    // (CGWindowList) sees showing: every poll replaces the guard and the drop goes on.
    let mut dropper = PrivacyFrameDropper::default();
    for refresh in 1..PERSISTENT_DROP_REFRESHES {
        // Several frames per guard count as one refresh.
        assert!(dropper.record(FrameVerdict::DropShowing, refresh as f64));
        assert!(dropper.record(FrameVerdict::DropShowing, refresh as f64));
        assert!(!dropper.persistent_drop_reported);
    }
    dropper.record(FrameVerdict::DropShowing, PERSISTENT_DROP_REFRESHES as f64);
    assert!(dropper.persistent_drop_reported);
    // A kept frame ends the spell; the next one counts afresh.
    dropper.record(FrameVerdict::Keep, 100.0);
    assert!(!dropper.persistent_drop_reported);
    dropper.record(FrameVerdict::DropShowing, 101.0);
    assert!(!dropper.persistent_drop_reported);
}

#[test]
fn a_window_list_flapping_between_unreadable_and_showing_warns_once() {
    let mut dropper = PrivacyFrameDropper::default();
    assert!(dropper.record(FrameVerdict::DropUnreadable, 0.0));
    assert!(dropper.record(FrameVerdict::DropShowing, 0.0));
    // Still in the same unreadable spell: the next unreadable frame must not warn again.
    assert!(dropper.window_list_unreadable);
}

#[test]
fn the_dropper_drops_on_drop_verdicts_and_keeps_on_keep() {
    let mut dropper = PrivacyFrameDropper::default();
    assert!(dropper.record(FrameVerdict::DropShowing, 0.0));
    assert!(dropper.record(FrameVerdict::DropUnreadable, 0.0));
    assert!(dropper.window_list_unreadable);
    assert!(!dropper.record(FrameVerdict::Keep, 0.0));
    assert!(!dropper.window_list_unreadable);
    assert_eq!(dropper.dropped_frames, 0);
}
