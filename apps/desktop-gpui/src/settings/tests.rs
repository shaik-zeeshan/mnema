//! Layer 2 for Settings: every §5 autosave window on both sides of its
//! boundary through the view with `advance_clock`, the privacy block, the
//! ReloadButton's minimum spin, and navigation.

use std::sync::Arc;
use std::time::Duration;

use capture_types::{AppearanceSetting, MicrophonePreferenceMode};
use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    point, px, size, AppContext as _, Bounds, Entity, TestAppContext, WindowAppearance,
    WindowBounds, WindowHandle, WindowOptions,
};

use super::autosave::{SaveStatus, Unit, MIC_DEBOUNCE, RECORDING_DEBOUNCE};
use super::fixture::{settings_data, Call, FixtureBackend};
use super::groups::{GroupId, SectionId};
use super::reload::MIN_SPIN;
use super::Settings;
use crate::theme;

const MS: Duration = Duration::from_millis(1);

fn open(
    cx: &mut TestAppContext,
    fail: bool,
) -> (WindowHandle<Root>, Entity<Settings>, Arc<FixtureBackend>) {
    let backend = Arc::new(FixtureBackend {
        fail,
        ..Default::default()
    });
    let b = backend.clone();
    let (window, view) = cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(0.), px(0.)),
                size: size(px(1280.), px(800.)),
            })),
            ..Default::default()
        };
        let (window, view) = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| {
                let mut s = Settings::with_backend(b, window, cx);
                s.set_data(settings_data().expect("fixture"), cx);
                s
            })
        })
        .expect("window");
        (window.downcast::<Root>().expect("root"), view)
    });
    cx.run_until_parked();
    frame(cx, window);
    (window, view, backend)
}

fn frame(cx: &mut TestAppContext, window: WindowHandle<Root>) {
    cx.update_window(window.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
}

fn advance(cx: &mut TestAppContext, d: Duration) {
    cx.executor().advance_clock(d);
    cx.run_until_parked();
}

fn saves(b: &FixtureBackend, c: Call) -> usize {
    b.calls().iter().filter(|x| **x == c).count()
}

#[gpui_kit::test]
fn recording_domains_save_450ms_after_the_last_edit(cx: &mut TestAppContext) {
    let (_, v, b) = open(cx, false);
    v.update(cx, |s, cx| {
        s.edit(|d| d.follow_timeline_live = !d.follow_timeline_live, cx)
    });
    advance(cx, RECORDING_DEBOUNCE - MS);
    assert_eq!(saves(&b, Call::Domain(Unit::Display)), 0, "449 ms: not yet");
    advance(cx, MS);
    assert_eq!(saves(&b, Call::Domain(Unit::Display)), 1, "450 ms: saved");
    assert_eq!(v.read_with(cx, |s, _| s.save_status()), SaveStatus::Saved);
}

#[gpui_kit::test]
fn an_edit_inside_the_window_restarts_it(cx: &mut TestAppContext) {
    let (_, v, b) = open(cx, false);
    v.update(cx, |s, cx| s.edit(|d| d.auto_start = !d.auto_start, cx));
    advance(cx, Duration::from_millis(300));
    v.update(cx, |s, cx| s.edit(|d| d.segment_duration_seconds = 60, cx));
    advance(cx, Duration::from_millis(300));
    assert_eq!(saves(&b, Call::Domain(Unit::CaptureTiming)), 0);
    advance(cx, Duration::from_millis(150));
    assert_eq!(saves(&b, Call::Domain(Unit::CaptureTiming)), 1);
}

#[gpui_kit::test]
fn source_switches_save_at_once(cx: &mut TestAppContext) {
    let (_, v, b) = open(cx, false);
    v.update(cx, |s, cx| {
        s.edit(|d| d.capture_screen = !d.capture_screen, cx)
    });
    cx.run_until_parked();
    assert_eq!(
        saves(&b, Call::Domain(Unit::CaptureSources)),
        1,
        "0 ms: saved with no clock advance"
    );
}

#[gpui_kit::test]
fn mic_saves_250ms_after_the_edit(cx: &mut TestAppContext) {
    let (_, v, b) = open(cx, false);
    v.update(cx, |s, cx| {
        s.edit_mic(
            |m| m.disconnect_policy = capture_types::MicrophoneDisconnectPolicy::WaitForSameDevice,
            cx,
        )
    });
    advance(cx, MIC_DEBOUNCE - MS);
    assert_eq!(saves(&b, Call::Mic), 0, "249 ms: not yet");
    advance(cx, MS);
    assert_eq!(saves(&b, Call::Mic), 1, "250 ms: saved");
}

#[gpui_kit::test]
fn a_specific_device_without_a_device_never_saves(cx: &mut TestAppContext) {
    let (_, v, b) = open(cx, false);
    v.update(cx, |s, cx| {
        s.edit_mic(
            |m| m.preference.mode = MicrophonePreferenceMode::SpecificDevice,
            cx,
        )
    });
    advance(cx, MIC_DEBOUNCE * 4);
    assert_eq!(saves(&b, Call::Mic), 0);
    assert_eq!(v.read_with(cx, |s, _| s.save_status()), SaveStatus::Blocked);
}

#[gpui_kit::test]
fn a_privacy_command_blocks_autosave_until_it_lands(cx: &mut TestAppContext) {
    let (_, v, b) = open(cx, false);
    v.update(cx, |s, cx| {
        s.privacy_in_flight = true;
        s.edit(|d| d.auto_start = !d.auto_start, cx);
    });
    advance(cx, RECORDING_DEBOUNCE * 3);
    assert_eq!(
        saves(&b, Call::Domain(Unit::CaptureTiming)),
        0,
        "blocked while the command runs"
    );
    v.update(cx, |s, cx| {
        s.privacy_in_flight = false;
        s.tick(cx);
    });
    advance(cx, RECORDING_DEBOUNCE - MS);
    assert_eq!(saves(&b, Call::Domain(Unit::CaptureTiming)), 0);
    advance(cx, MS);
    assert_eq!(
        saves(&b, Call::Domain(Unit::CaptureTiming)),
        1,
        "saves once it lands"
    );
}

#[gpui_kit::test]
fn a_privacy_command_cancels_a_pending_save(cx: &mut TestAppContext) {
    let (_, v, b) = open(cx, false);
    v.update(cx, |s, cx| s.edit(|d| d.auto_start = !d.auto_start, cx));
    advance(cx, Duration::from_millis(200));
    v.update(cx, |s, cx| {
        s.privacy_command(
            |d| {
                d.privacy.filter_system_audio = !d.privacy.filter_system_audio;
                Ok(())
            },
            cx,
        )
    });
    cx.run_until_parked();
    assert_eq!(saves(&b, Call::Privacy), 1);
    // The command landed and re-ticked: the timing edit saves a full window later.
    assert_eq!(saves(&b, Call::Domain(Unit::CaptureTiming)), 0);
    advance(cx, RECORDING_DEBOUNCE);
    assert_eq!(saves(&b, Call::Domain(Unit::CaptureTiming)), 1);
}

#[gpui_kit::test]
fn a_failed_save_shows_couldnt_save_and_retry_reruns_it(cx: &mut TestAppContext) {
    let (_, v, b) = open(cx, true);
    v.update(cx, |s, cx| {
        s.edit(
            |d| d.developer_options_enabled = !d.developer_options_enabled,
            cx,
        )
    });
    advance(cx, RECORDING_DEBOUNCE);
    assert_eq!(v.read_with(cx, |s, _| s.save_status()), SaveStatus::Failed);
    v.update(cx, |s, cx| s.retry(cx));
    cx.run_until_parked();
    assert_eq!(saves(&b, Call::Domain(Unit::Developer)), 2);
}

#[gpui_kit::test]
fn reload_spins_at_least_500ms(cx: &mut TestAppContext) {
    let (_, v, _) = open(cx, false);
    let now = cx.executor().now();
    v.update(cx, |s, _| s.reload.click("reload-test", now));
    advance(cx, MIN_SPIN - MS);
    let at = cx.executor().now();
    assert!(
        v.read_with(cx, |s, _| s.reload.spinning("reload-test", at, false)),
        "499 ms: spinning"
    );
    advance(cx, MS);
    let at = cx.executor().now();
    assert!(
        !v.read_with(cx, |s, _| s.reload.spinning("reload-test", at, false)),
        "500 ms: done"
    );
}

#[gpui_kit::test]
fn tab_deeplinks_select_group_and_section(cx: &mut TestAppContext) {
    let (window, v, _) = open(cx, false);
    v.update(cx, |s, cx| s.open_tab("keyboard", cx));
    frame(cx, window);
    v.read_with(cx, |s, _| {
        assert_eq!(s.group, GroupId::General);
        assert_eq!(s.section, SectionId::Shortcuts);
    });
    v.update(cx, |s, cx| s.open_tab("cli-access", cx));
    frame(cx, window);
    v.read_with(cx, |s, _| {
        assert_eq!(s.group, GroupId::Data);
        assert_eq!(s.section, SectionId::Access);
    });
    v.update(cx, |s, cx| s.open_tab("nope", cx));
    assert_eq!(
        v.read_with(cx, |s, _| s.group),
        GroupId::Data,
        "unknown tabs keep the group"
    );
}

#[gpui_kit::test]
fn every_group_renders(cx: &mut TestAppContext) {
    let (window, v, _) = open(cx, false);
    for tab in [
        "appearance",
        "capture",
        "intelligence",
        "storage",
        "license",
    ] {
        v.update(cx, |s, cx| s.open_tab(tab, cx));
        frame(cx, window);
    }
}
