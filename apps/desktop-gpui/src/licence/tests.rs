//! Layer 2: the banner and the receipt mounted in the shell, over a fixture
//! backend (no app core, no keychain, no network).

use std::sync::Arc;
use std::time::Duration;

use capture_types::{AppearanceSetting, LicenseStatus, ResetDevicesOutcome};
use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    px, size, Bounds, Entity, Point, TestAppContext, WindowAppearance, WindowBounds, WindowHandle,
    WindowOptions,
};

use super::banner::fixtures::*;
use super::receipt::{Face, Flow};
use super::*;
use crate::keymap::{self, Bindings};
use crate::shell::{Shell, ShellModel};

fn open(cx: &mut TestAppContext) -> (WindowHandle<Root>, Entity<Licence>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
        keymap::bind(&Bindings::default(), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(1280.), px(800.)),
            })),
            ..Default::default()
        };
        let (window, shell) = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| Shell::new(ShellModel::default(), window, cx))
        })
        .expect("window");
        let licence = shell.read(cx).licence.clone();
        (window.downcast::<Root>().expect("root"), licence)
    })
}

fn with_backend(cx: &mut TestAppContext, licence: &Entity<Licence>, b: Arc<FixtureBackend>) {
    cx.update(|cx| licence.update(cx, |l, _| l.backend = b));
}

fn set(cx: &mut TestAppContext, licence: &Entity<Licence>, status: Option<LicenseStatus>) {
    cx.update(|cx| licence.update(cx, |l, cx| l.on_status(status, cx)));
    cx.run_until_parked();
}

fn has(cx: &mut TestAppContext, window: WindowHandle<Root>, id: &'static str) -> bool {
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some()
    })
    .unwrap()
}

fn click(cx: &mut TestAppContext, window: WindowHandle<Root>, id: &'static str) {
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn press(cx: &mut TestAppContext, window: WindowHandle<Root>, key: &str) {
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press(key, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn face(cx: &mut TestAppContext, licence: &Entity<Licence>) -> Option<Face> {
    cx.read(|cx| {
        let r = &licence.read(cx).receipt;
        r.face.clone().filter(|_| r.open)
    })
}

fn start(cx: &mut TestAppContext, licence: &Entity<Licence>, flow: Flow) {
    cx.update(|cx| licence.update(cx, |l, cx| l.start(flow, cx)));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn banner_shows_only_in_its_states(cx: &mut TestAppContext) {
    let (window, licence) = open(cx);
    assert!(
        !has(cx, window, "licence-banner"),
        "no status yet → nothing"
    );
    for (status, shows) in [
        (licensed(), false),
        (trial(8), false),
        (TRIAL_NOT_STARTED, false),
        (licensed_over_cap(), false),
        (licensed_pending(4), false),
        (trial(7), true),
        (READ_ONLY, true),
        (REVOKED, true),
        (licensed_lapsed(), true),
        (licensed_pending(3), true),
    ] {
        set(cx, &licence, Some(status.clone()));
        assert_eq!(has(cx, window, "licence-banner"), shows, "{status:?}");
    }
    // Low disk is the last resort, under a healthy licence.
    set(cx, &licence, Some(licensed()));
    cx.update(|cx| {
        licence.update(cx, |l, cx| {
            l.low_disk = true;
            cx.notify();
        })
    });
    assert!(has(cx, window, "licence-banner-storage"));
}

#[gpui_kit::test]
fn firm_banners_have_no_dismiss_and_their_actions(cx: &mut TestAppContext) {
    let (window, licence) = open(cx);
    set(cx, &licence, Some(READ_ONLY));
    assert!(has(cx, window, "licence-banner-buy"));
    assert!(has(cx, window, "licence-banner-enter"));
    assert!(!has(cx, window, "licence-banner-dismiss"));
    set(cx, &licence, Some(licensed_lapsed()));
    assert!(has(cx, window, "licence-banner-recheck"));
    assert!(!has(cx, window, "licence-banner-dismiss"));
}

#[gpui_kit::test]
fn dismiss_hides_until_the_count_drops(cx: &mut TestAppContext) {
    let (window, licence) = open(cx);
    set(cx, &licence, Some(trial(5)));
    click(cx, window, "licence-banner-dismiss");
    assert!(!has(cx, window, "licence-banner"));
    set(cx, &licence, Some(trial(5)));
    assert!(
        !has(cx, window, "licence-banner"),
        "same count stays dismissed"
    );
    set(cx, &licence, Some(trial(4)));
    assert!(
        has(cx, window, "licence-banner"),
        "escalation re-surfaces it"
    );
}

#[gpui_kit::test]
fn recheck_runs_the_refresh_off_thread(cx: &mut TestAppContext) {
    let (window, licence) = open(cx);
    let backend = Arc::new(FixtureBackend::default());
    with_backend(cx, &licence, backend.clone());
    set(cx, &licence, Some(licensed_lapsed()));
    click(cx, window, "licence-banner-recheck");
    assert_eq!(*backend.calls.lock().unwrap(), vec!["refresh"]);
    assert!(!cx.read(|cx| licence.read(cx).rechecking));
}

#[gpui_kit::test]
fn receipt_morphs_as_results_land(cx: &mut TestAppContext) {
    let (window, licence) = open(cx);
    set(cx, &licence, Some(trial(3)));
    start(cx, &licence, Flow::Activate);
    assert_eq!(face(cx, &licence), Some(Face::Working));
    assert!(has(cx, window, "licence-receipt"));
    assert!(has(cx, window, "receipt-dismiss"));
    set(cx, &licence, Some(trial(3)));
    assert_eq!(
        face(cx, &licence),
        Some(Face::Working),
        "not the result yet"
    );
    set(cx, &licence, Some(licensed_pending(5)));
    assert!(matches!(face(cx, &licence), Some(Face::Pending { .. })));
    set(cx, &licence, Some(licensed()));
    assert!(matches!(face(cx, &licence), Some(Face::Activated { .. })));
    click(cx, window, "receipt-continue");
    assert!(!has(cx, window, "licence-receipt"));
}

#[gpui_kit::test]
fn every_face_renders_its_buttons(cx: &mut TestAppContext) {
    let (window, licence) = open(cx);
    start(cx, &licence, Flow::Activate);
    set(cx, &licence, Some(licensed_over_cap()));
    for id in ["receipt-buy", "receipt-help", "receipt-reset"] {
        assert!(has(cx, window, id), "{id}");
    }
    start(cx, &licence, Flow::Renewed);
    let done = app_core::CoreEvent::LicenseDeepLinkDone(serde_json::json!({
        "outcome": "failed", "message": "We couldn't confirm your renewal yet."
    }));
    cx.update(|cx| on_event(&licence, &done, cx));
    assert!(matches!(face(cx, &licence), Some(Face::Failed { .. })));
    assert!(has(cx, window, "receipt-close"));
    assert!(has(cx, window, "receipt-settings"));
    // A `license_status` event's JSON payload reaches the banner and receipt.
    start(cx, &licence, Flow::Claim);
    let status = serde_json::to_value(licensed()).unwrap();
    cx.update(|cx| on_event(&licence, &app_core::CoreEvent::LicenseStatus(status), cx));
    assert!(matches!(face(cx, &licence), Some(Face::Activated { .. })));
    assert_eq!(
        cx.read(|cx| licence.read(cx).status.clone()),
        Some(licensed())
    );
}

#[gpui_kit::test]
fn esc_and_the_close_button_close_it(cx: &mut TestAppContext) {
    let (window, licence) = open(cx);
    start(cx, &licence, Flow::Claim);
    press(cx, window, "escape");
    assert!(!has(cx, window, "licence-receipt"));
    start(cx, &licence, Flow::Claim);
    click(cx, window, "modal-close");
    assert!(!has(cx, window, "licence-receipt"));
}

#[gpui_kit::test]
fn the_working_copy_softens_after_30_s(cx: &mut TestAppContext) {
    let (_window, licence) = open(cx);
    start(cx, &licence, Flow::Claim);
    let slow = |cx: &mut TestAppContext| cx.read(|cx| licence.read(cx).receipt.slow);
    cx.executor()
        .advance_clock(receipt::SLOW_AFTER - Duration::from_millis(1));
    cx.run_until_parked();
    assert!(!slow(cx));
    cx.executor().advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    assert!(slow(cx));
    // A new episode starts fresh, and an old timer can't flip it.
    start(cx, &licence, Flow::Activate);
    assert!(!slow(cx));
}

#[gpui_kit::test]
fn dismissed_mid_flight_success_is_quiet_over_cap_reopens(cx: &mut TestAppContext) {
    let (window, licence) = open(cx);
    start(cx, &licence, Flow::Claim);
    click(cx, window, "receipt-dismiss");
    set(cx, &licence, Some(licensed()));
    assert!(!has(cx, window, "licence-receipt"), "success stays quiet");
    start(cx, &licence, Flow::Claim);
    click(cx, window, "receipt-dismiss");
    set(cx, &licence, Some(licensed_over_cap()));
    assert!(
        has(cx, window, "receipt-reset"),
        "the device limit re-opens it"
    );
}

#[gpui_kit::test]
fn a_late_answer_never_lands_on_a_closed_receipt(cx: &mut TestAppContext) {
    let (window, licence) = open(cx);
    start(cx, &licence, Flow::Activate);
    set(cx, &licence, Some(licensed()));
    click(cx, window, "receipt-continue");
    set(cx, &licence, Some(licensed_over_cap()));
    let done = app_core::CoreEvent::LicenseDeepLinkDone(serde_json::json!({ "outcome": "failed" }));
    cx.update(|cx| on_event(&licence, &done, cx));
    assert!(!has(cx, window, "licence-receipt"));
}

#[gpui_kit::test]
fn free_up_devices_confirms_then_resets(cx: &mut TestAppContext) {
    let (window, licence) = open(cx);
    let backend = Arc::new(FixtureBackend {
        confirm: true,
        reset: Some(ResetDevicesOutcome::RateLimited { retry_at_ms: None }),
        ..Default::default()
    });
    with_backend(cx, &licence, backend.clone());
    start(cx, &licence, Flow::Activate);
    set(cx, &licence, Some(licensed_over_cap()));
    click(cx, window, "receipt-reset");
    assert_eq!(
        *backend.calls.lock().unwrap(),
        vec!["confirm_reset", "reset_devices"]
    );
    assert!(has(cx, window, "receipt-reset-note"));
    assert!(cx
        .read(|cx| licence.read(cx).receipt.reset_note.clone())
        .unwrap()
        .ends_with("again later."));
}

#[gpui_kit::test]
fn declining_the_confirm_resets_nothing(cx: &mut TestAppContext) {
    let (window, licence) = open(cx);
    let backend = Arc::new(FixtureBackend::default());
    with_backend(cx, &licence, backend.clone());
    start(cx, &licence, Flow::Activate);
    set(cx, &licence, Some(licensed_over_cap()));
    click(cx, window, "receipt-reset");
    assert_eq!(*backend.calls.lock().unwrap(), vec!["confirm_reset"]);
    assert!(!has(cx, window, "receipt-reset-note"));
}

#[gpui_kit::test]
fn a_reset_answer_from_an_old_episode_is_dropped(cx: &mut TestAppContext) {
    let (window, licence) = open(cx);
    let backend = Arc::new(FixtureBackend {
        confirm: true,
        reset: Some(ResetDevicesOutcome::Reset),
        ..Default::default()
    });
    with_backend(cx, &licence, backend.clone());
    start(cx, &licence, Flow::Activate);
    set(cx, &licence, Some(licensed_over_cap()));
    // Click without parking, then a new link arrives before the confirm answers.
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("receipt-reset", cx);
    })
    .unwrap();
    cx.update(|cx| licence.update(cx, |l, cx| l.start(Flow::Renewed, cx)));
    cx.run_until_parked();
    assert_eq!(*backend.calls.lock().unwrap(), vec!["confirm_reset"]);
    assert_eq!(
        cx.read(|cx| licence.read(cx).receipt.reset_note.clone()),
        None
    );
    assert_eq!(face(cx, &licence), Some(Face::Working));
}

#[gpui_kit::test]
fn a_link_before_main_waits_for_the_next_shell(cx: &mut TestAppContext) {
    cx.update(|cx| announce(Flow::Claim, cx));
    let (window, licence) = open(cx);
    assert!(has(cx, window, "licence-receipt"));
    assert_eq!(
        cx.read(|cx| licence.read(cx).receipt.flow),
        Some(Flow::Claim)
    );
    assert!(cx.read(|cx| cx
        .try_global::<PendingEpisode>()
        .is_some_and(|p| p.0.is_none())));
}

/// Each face the receipt can land on, from a fresh episode.
fn land(cx: &mut TestAppContext, licence: &Entity<Licence>, face: &str) {
    match face {
        "working" => start(cx, licence, Flow::Claim),
        "renewed" => {
            set(cx, licence, Some(licensed()));
            start(cx, licence, Flow::Renewed);
            set(
                cx,
                licence,
                Some(licensed_with(
                    "Ada Lovelace",
                    1_763_072_000_000,
                    capture_types::Activation::Activated,
                )),
            );
        }
        "failed" => {
            start(cx, licence, Flow::Renewed);
            let done = app_core::CoreEvent::LicenseDeepLinkDone(serde_json::json!({
                "outcome": "failed", "message": "We couldn't confirm your renewal yet."
            }));
            cx.update(|cx| on_event(licence, &done, cx));
        }
        _ => {
            start(cx, licence, Flow::Claim);
            let status = match face {
                "pending" => licensed_pending(3),
                "overcap" => licensed_over_cap(),
                _ => licensed(),
            };
            set(cx, licence, Some(status));
        }
    }
}

/// face → its buttons, the primary (focused) one last.
const FACES: [(&str, &[&str]); 6] = [
    ("working", &["receipt-dismiss"]),
    ("activated", &["receipt-continue"]),
    ("renewed", &["receipt-continue"]),
    ("pending", &["receipt-continue"]),
    ("overcap", &["receipt-buy", "receipt-help", "receipt-reset"]),
    ("failed", &["receipt-close", "receipt-settings"]),
];

fn role(cx: &mut TestAppContext, window: WindowHandle<Root>, id: &'static str) -> gpui_kit::Role {
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window
            .find(id)
            .role()
            .unwrap_or_else(|| panic!("{id} has no role"))
    })
    .unwrap()
}

#[gpui_kit::test]
fn every_face_keeps_ids_and_roles_on_the_new_paper(cx: &mut TestAppContext) {
    use gpui_kit::Role;
    let (window, licence) = open(cx);
    for (face, buttons) in FACES {
        land(cx, &licence, face);
        assert_eq!(role(cx, window, "licence-receipt"), Role::Dialog, "{face}");
        assert_eq!(role(cx, window, "receipt-body"), Role::Group, "{face}");
        assert_eq!(role(cx, window, "modal-close"), Role::Button, "{face}");
        for &id in buttons {
            assert_eq!(role(cx, window, id), Role::Button, "{face}: {id}");
        }
        click(cx, window, "modal-close");
        assert!(!has(cx, window, "licence-receipt"), "{face}: × closes");
    }
}

#[gpui_kit::test]
fn the_primary_action_takes_focus_on_open_and_each_morph(cx: &mut TestAppContext) {
    let (window, licence) = open(cx);
    for (face, buttons) in FACES {
        land(cx, &licence, face);
        let primary = *buttons.last().unwrap();
        has(cx, window, primary); // paint: the focus is moved after it
        cx.run_until_parked();
        let focused = cx
            .update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                let on_handle = licence
                    .read(cx)
                    .primary
                    .as_ref()
                    .is_some_and(|h| h.is_focused(window));
                (on_handle, window.find(primary).focused())
            })
            .unwrap();
        assert!(focused.0, "{face}: focus on the primary action");
        assert_eq!(focused.1, Some(true), "{face}: on {primary}");
        press(cx, window, "escape");
        assert!(!has(cx, window, "licence-receipt"), "{face}: Esc closes");
    }
}

#[gpui_kit::test]
fn a_press_on_the_scrim_closes_it_and_on_the_paper_does_not(cx: &mut TestAppContext) {
    let (window, licence) = open(cx);
    for face in ["activated", "overcap"] {
        land(cx, &licence, face);
        // Let the scrim fade in: a press needs it visible.
        has(cx, window, "licence-receipt");
        cx.executor().advance_clock(crate::theme::motion::SLOW);
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("receipt-body", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(has(cx, window, "licence-receipt"), "{face}: paper keeps it");
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click_at("licence-receipt", gpui_kit::point(px(8.), px(8.)), cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(!has(cx, window, "licence-receipt"), "{face}: scrim closes");
    }
}

fn readout(cx: &mut TestAppContext, window: WindowHandle<Root>) -> String {
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window
            .find("receipt-readout")
            .label()
            .unwrap_or_default()
            .to_string()
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_scrub_lands_on_the_end_date_and_reduce_motion_skips_it(cx: &mut TestAppContext) {
    let (window, licence) = open(cx);
    let at = time::macros::datetime!(2024-06-01 12:00 UTC);
    cx.update(|cx| crate::ui::clock::freeze(Some(at), cx));
    let (today, end) = ("JUN 01 2024", "NOV 13 2024");
    land(cx, &licence, "activated");
    assert_eq!(readout(cx, window), today, "starts at today");
    cx.executor().advance_clock(Duration::from_millis(199));
    assert_eq!(readout(cx, window), today, "holds while the modal rises");
    cx.executor()
        .advance_clock(parts::CELEBRATE - Duration::from_millis(199));
    assert_eq!(readout(cx, window), end, "lands on the end date");
    click(cx, window, "receipt-continue");

    cx.update(|cx| cx.set_reduce_motion(true));
    land(cx, &licence, "activated");
    assert_eq!(readout(cx, window), end, "reduce motion: landed at once");
}

#[test]
fn the_scrub_holds_then_eases_out_then_lands_within_the_celebration() {
    let (t, l) = (ruler::timing, |ms| ruler::timing(ms).1);
    assert_eq!(t(0.), (0., 0.));
    assert_eq!(t(200.).0, 0., "holds 200 ms");
    assert!(t(201.).0 > 0.);
    assert!(t(450.).0 > 0.5, "ease-out: most of the way early");
    assert!(t(1000.).0 < 1.);
    assert!((t(1150.).0 - 1.).abs() < 1e-4);
    assert_eq!(l(1050.), 0., "lands only after the scrub");
    assert!(l(1051.) > 0.);
    assert!((l(1400.) - 1.).abs() < 1e-4);
    assert_eq!(parts::CELEBRATE, Duration::from_millis(1400));
}
