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
    open_at(cx, fail, (1280., 800.))
}

pub(super) fn open_at(
    cx: &mut TestAppContext,
    fail: bool,
    (w, h): (f32, f32),
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
                size: size(px(w), px(h)),
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

pub(super) fn frame(cx: &mut TestAppContext, window: WindowHandle<Root>) {
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

#[gpui_kit::test]
fn a_provider_key_saves_to_the_vault_and_clears_live(cx: &mut TestAppContext) {
    let (window, v, backend) = open(cx, false);
    cx.update_window(window.into(), |_, window, cx| {
        v.update(cx, |s, cx| {
            let input = s.inputs.provider_key.clone();
            input.update(cx, |i, cx| {
                i.set_value("sk-test-0123456789abcd", window, cx)
            });
            s.save_provider_key("openai-2".into(), window, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    v.read_with(cx, |s, cx| {
        assert!(s.data.keys_saved.contains("openai-2"));
        assert_eq!(
            s.data.key_endings.get("openai-2").map(String::as_str),
            Some("9f2a")
        );
        assert_eq!(
            s.inputs.provider_key.read(cx).value().as_ref(),
            "",
            "the field forgets the key"
        );
    });
    v.update(cx, |s, cx| s.clear_provider_key("openai-2".into(), cx));
    cx.run_until_parked();
    v.read_with(cx, |s, _| {
        assert!(!s.data.keys_saved.contains("openai-2"));
        assert!(!s.data.key_endings.contains_key("openai-2"));
    });
    assert_eq!(
        backend.calls(),
        vec![
            Call::SaveKey("openai-2".into()),
            Call::ClearKey("openai-2".into())
        ]
    );
}

#[gpui_kit::test]
fn at_the_minimum_window_a_wide_control_wraps_inside_the_column(cx: &mut TestAppContext) {
    // `.setting-row`: the copy keeps 240 px and the control drops under it,
    // right-aligned; ≤800 px the rail is 200 wide and the padding 24.
    let (w, h) = crate::window::MAIN_MIN;
    let (window, v, _) = open_at(cx, false, (w, h));
    v.update(cx, |s, cx| s.open_tab("video", cx));
    cx.run_until_parked();
    frame(cx, window);
    let (rate, cat) = cx
        .update_window(window.into(), |_, window, _| {
            (
                window.find("capture-rate").bounds(),
                window.find("settings-cat-General").bounds(),
            )
        })
        .unwrap();
    assert!(
        cat.right() <= px(super::rail::RAIL_W_TIGHT),
        "the rail narrows to 200"
    );
    assert_eq!(
        rate.right(),
        px(w) - crate::theme::space::S5,
        "right-aligned in the column"
    );
    assert!(
        rate.left() >= px(super::rail::RAIL_W_TIGHT) + crate::theme::space::S5,
        "inside the column"
    );
}

/// The page column's margins inside the scroll area at a window size: (left, width, right).
fn column_at(cx: &mut TestAppContext, w: f32) -> (f32, f32, f32) {
    let (_, v, _) = open_at(cx, false, (w, 800.));
    v.read_with(cx, |s, _| {
        let area = s.scroll.bounds();
        let col = s
            .scroll
            .bounds_for_item(s.item_base)
            .expect("first section");
        (
            f32::from(col.left() - area.left()),
            f32::from(col.size.width),
            f32::from(area.right() - col.right()),
        )
    })
}

#[gpui_kit::test]
fn at_the_golden_width_the_page_column_keeps_its_place(cx: &mut TestAppContext) {
    // 1280 − 252 rail = 1028: the 48 px gutter, 760 px column, the rest to its right.
    assert_eq!(column_at(cx, 1280.), (48., 760., 220.));
}

#[gpui_kit::test]
fn on_a_wide_window_the_page_column_centres_right_of_the_rail(cx: &mut TestAppContext) {
    let (left, width, right) = column_at(cx, 2400.);
    assert_eq!(width, 760.);
    assert!((left - right).abs() <= 1., "centred: {left} vs {right}");
}

#[gpui_kit::test]
fn a_third_party_notice_opens_its_source_url(cx: &mut TestAppContext) {
    // Tall enough that About's notices are on screen without scrolling.
    let (w, v, _) = open_at(cx, false, (1280., 6000.));
    v.update(cx, |s, cx| s.open_tab("about", cx));
    frame(cx, w);
    let (ix, url) = cx.read(|cx| {
        let notices = v.read(cx).data.notices.clone().expect("notices");
        notices["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .enumerate()
            .find_map(|(i, e)| Some((i, e["sourceUrl"].as_str()?.to_string())))
            .expect("an entry with a source")
    });
    let id = crate::ui::nid("notice-source", ix as u64);
    cx.update_window(w.into(), |_, window, cx| {
        assert_eq!(window.find(id.clone()).role(), Some(gpui_kit::Role::Link));
        window.click(id, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.opened_url(), Some(url));
}

/// The rail tracks the section at the top of the column as the user scrolls
/// (`scroll-spy.ts`). Real app: "Providers" stayed lit over Transcription
/// (capture 43): GPUI's tracked child bounds are content positions, and the spy
/// read them as on-screen ones, so every scroll fell back to the first section.
#[gpui_kit::test]
fn rail_follows_scrolling_through_intelligence(cx: &mut TestAppContext) {
    use gpui_kit::{InputEvent as _, ScrollDelta};
    let (window, v, _) = open(cx, false);
    v.update(cx, |s, cx| s.open_tab("ai", cx));
    frame(cx, window);
    frame(cx, window);
    // Each section's top, measured with the column at the top.
    let (ids, tops, max) = v.read_with(cx, |s, _| {
        assert_eq!(f32::from(s.scroll.offset().y), 0.);
        let view = s.scroll.bounds();
        let tops: Vec<f32> = (0..s.items.len())
            .map(|ix| {
                f32::from(s.scroll.bounds_for_item(s.item_base + ix).unwrap().top() - view.top())
            })
            .collect();
        (s.items.clone(), tops, f32::from(s.scroll.max_offset().y))
    });
    assert!(ids.len() >= 5, "Intelligence has its sections: {ids:?}");
    let mut at = 0f32;
    let mut stops = (1..ids.len())
        .chain((0..ids.len() - 1).rev())
        .collect::<Vec<_>>();
    stops.push(0);
    for ix in stops {
        // Park this section's top a little under the column top, as a wheel would.
        let target = (tops[ix] - 40.).clamp(0., max);
        cx.update_window(window.into(), |_, window, cx| {
            // A wheel over the column (right of the rail, under the bar).
            let position = gpui_kit::point(px(900.), px(400.));
            window.dispatch_event(
                gpui_kit::MouseMoveEvent {
                    position,
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                gpui_kit::ScrollEvent {
                    position,
                    delta: ScrollDelta::Pixels(gpui_kit::point(px(0.), px(at - target))),
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
        })
        .unwrap();
        at = target;
        frame(cx, window);
        let want = if target >= max - 2. {
            *ids.last().unwrap()
        } else {
            ids[ix]
        };
        v.read_with(cx, |s, _| {
            assert_eq!(
                f32::from(-s.scroll.offset().y).round(),
                target.round(),
                "scrolled to {ix}"
            );
            assert_eq!(s.section, want, "stop {ix} at {target} px");
        });
    }
    // A rail click from a scrolled column parks its section 24 px under the top.
    let from = (tops[3] - 40.).clamp(0., max);
    v.update(cx, |s, cx| {
        s.scroll.set_offset(gpui_kit::point(px(0.), px(-from)));
        s.navigate(ids[1], cx);
    });
    frame(cx, window);
    v.read_with(cx, |s, _| {
        let want = (tops[1] - 24.).clamp(0., max);
        assert_eq!(f32::from(-s.scroll.offset().y).round(), want.round());
        assert_eq!(s.section, ids[1]);
    });
}

/// The tray flips the microphone source while an auto-start edit waits for its
/// 450 ms: the re-read merges (the edit stays, the mic takes the tray's value),
/// and the next sources save sends the tray's mic, never the stale one.
#[gpui_kit::test]
fn an_outside_change_under_an_unsaved_edit_merges_and_is_never_written_back(
    cx: &mut TestAppContext,
) {
    let (_, v, b) = open(cx, false);
    v.update(cx, |s, cx| s.edit(|d| d.auto_start = !d.auto_start, cx));
    let edited = v.read_with(cx, |s, _| s.d().auto_start);

    let mut outside = settings_data().expect("fixture");
    let tray = outside
        .settings
        .as_ref()
        .map(|s| !s.capture_microphone)
        .unwrap();
    outside.settings.as_mut().unwrap().capture_microphone = tray;
    v.update(cx, |s, cx| s.land(outside, cx));
    assert_eq!(
        v.read_with(cx, |s, _| (s.d().capture_microphone, s.d().auto_start)),
        (tray, edited),
        "the edit stays; the untouched field takes the re-read value"
    );

    advance(cx, RECORDING_DEBOUNCE);
    let timing = b.last_sent(Unit::CaptureTiming).expect("the edit saved");
    assert_eq!(timing.auto_start, edited);

    // A source switch next saves at once: with the tray's mic.
    v.update(cx, |s, cx| {
        s.edit(|d| d.capture_screen = !d.capture_screen, cx)
    });
    cx.run_until_parked();
    let sent = b.last_sent(Unit::CaptureSources).expect("sources saved");
    assert_eq!(sent.capture_microphone, tray, "never the stale value");
}

/// A clean page adopts a re-read whole.
#[gpui_kit::test]
fn a_clean_page_adopts_an_outside_change(cx: &mut TestAppContext) {
    let (_, v, b) = open(cx, false);
    let mut outside = settings_data().expect("fixture");
    let tray = outside
        .settings
        .as_ref()
        .map(|s| !s.capture_microphone)
        .unwrap();
    outside.settings.as_mut().unwrap().capture_microphone = tray;
    v.update(cx, |s, cx| s.land(outside, cx));
    advance(cx, RECORDING_DEBOUNCE);
    assert_eq!(v.read_with(cx, |s, _| s.d().capture_microphone), tray);
    assert!(b.calls().is_empty(), "nothing to save");
}

/// Window focus re-reads the permissions and access hints, never the drafts.
#[gpui_kit::test]
fn window_focus_rechecks_access_without_touching_an_unsaved_edit(cx: &mut TestAppContext) {
    let (_, v, b) = open(cx, false);
    v.update(cx, |s, cx| s.edit(|d| d.auto_start = !d.auto_start, cx));
    let draft = v.read_with(cx, |s, _| s.d());
    assert!(!v.read_with(cx, |s, _| s.data.system_audio_blocked));
    v.update(cx, |s, cx| s.on_window_focus(cx));
    cx.run_until_parked();
    assert_eq!(saves(&b, Call::Recheck), 1);
    assert!(v.read_with(cx, |s, _| s.data.system_audio_blocked));
    assert_eq!(
        v.read_with(cx, |s, _| s.d()),
        draft,
        "the edit is untouched"
    );
    assert_eq!(
        v.read_with(cx, |s, _| s.gate(Unit::CaptureTiming).dirty),
        true,
        "and still saves"
    );
}

// ── Copy buttons ─────────────────────────────────────────────────────────────

fn clipboard(cx: &mut TestAppContext) -> Option<String> {
    cx.read(|cx| cx.read_from_clipboard().and_then(|c| c.text()))
}

fn click_and_label(cx: &mut TestAppContext, window: WindowHandle<Root>, id: &str) -> String {
    let id = gpui_kit::SharedString::from(id.to_string());
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(id.clone(), cx);
    })
    .unwrap();
    cx.run_until_parked();
    label_of(cx, window, &id)
}

fn label_of(cx: &mut TestAppContext, window: WindowHandle<Root>, id: &str) -> String {
    let id = gpui_kit::SharedString::from(id.to_string());
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.find(id).label().unwrap_or_default().to_string()
    })
    .unwrap()
}

#[gpui_kit::test]
fn about_copy_details_and_notices_copy_and_say_copied_for_2s(cx: &mut TestAppContext) {
    // Tall enough that the notices' button is on screen without scrolling.
    let (window, v, _) = open_at(cx, false, (1280., 2400.));
    v.update(cx, |s, cx| s.open_tab("about", cx));
    cx.run_until_parked();
    frame(cx, window);
    assert_eq!(click_and_label(cx, window, "about-copy"), "Copied");
    let details = clipboard(cx).unwrap_or_default();
    assert!(
        details == "Mnema 0.1.23 (macos/aarch64) day.mnema",
        "{details}"
    );
    advance(cx, Duration::from_millis(1999));
    assert_eq!(label_of(cx, window, "about-copy"), "Copied", "at 1999 ms");
    advance(cx, MS);
    assert_eq!(
        label_of(cx, window, "about-copy"),
        "Copy version and build details to the clipboard",
        "reverted at 2 s"
    );
    assert_eq!(click_and_label(cx, window, "notices-copy"), "Copied");
    let notices = clipboard(cx).unwrap_or_default();
    assert!(
        notices.len() > 200,
        "the full notices: {} bytes",
        notices.len()
    );
}

#[gpui_kit::test]
fn a_model_folder_copies_its_path(cx: &mut TestAppContext) {
    let (window, v, _) = open(cx, false);
    v.update(cx, |s, cx| s.open_tab("semantic-search", cx));
    cx.run_until_parked();
    frame(cx, window);
    assert_eq!(
        click_and_label(cx, window, "ss-model-row-copy-folder"),
        "Copied"
    );
    assert_eq!(
        clipboard(cx).as_deref(),
        Some("/Users/ana/Library/Application Support/day.mnema/config/semantic_search_models")
    );
}
