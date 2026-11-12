//! Flows checked against `+page.svelte` (the flow-parity pass): which layer owns
//! a key, the OCR states, the audio lane, hand-offs, the ⋯ menu's open-page row,
//! the rail's keys, the shortcuts sheet and the audio-only stage.

use std::sync::Arc;

use capture_types::AppearanceSetting;
use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext as _, Entity, TestAppContext, WindowAppearance, WindowHandle, WindowOptions,
};
use serde_json::{json, Value};

use super::fixture::{Call, FixtureSource};
use super::scrub_engine::EXACT_SETTLE;
use super::tests::{advance, count, frame, frames, open};
use super::{OcrStatus, TimelineView};
use crate::theme;
use crate::ui::nid;

const PREVIEW: &str = "get_frame_preview";

/// A source over `extra` (merged into the map) with `n` frames.
pub(super) fn source_with(n: i64, extra: Value) -> FixtureSource {
    let mut map = json!({ PREVIEW: { "mimeType": "image/webp", "sourceKind": "original_frame" } });
    for (k, v) in extra.as_object().unwrap() {
        map[k] = v.clone();
    }
    FixtureSource::from_map(&map).with_frames(frames(n))
}

/// A mic segment over the newest frames (09:01:36 → 09:02:06).
pub(super) fn mic_segment() -> Value {
    json!([{
        "id": 77, "sourceKind": "microphone", "segmentIndex": 1,
        "startedAt": "2026-11-04T09:01:36Z", "endedAt": "2026-11-04T09:02:06Z"
    }])
}

/// One completed Apple Vision job with two lines.
pub(super) fn ocr_done() -> Value {
    let payload = json!({
        "provider": "apple_vision",
        "observations": [
            { "text": "Pricing", "confidence": 0.98,
              "boundingBox": { "x": 0.1, "y": 0.8, "width": 0.2, "height": 0.05 } },
            { "text": "Pro plan", "confidence": 0.9,
              "boundingBox": { "x": 0.1, "y": 0.7, "width": 0.2, "height": 0.05 } }
        ]
    });
    json!({
        "list_processing_jobs": [{ "id": 5, "processor": "ocr", "status": "completed" }],
        "get_processing_result": { "structuredPayloadJson": payload.to_string() }
    })
}

pub(super) fn click(
    cx: &mut TestAppContext,
    w: WindowHandle<Root>,
    id: impl Into<gpui_kit::ElementId>,
) {
    let id = id.into();
    cx.update_window(w.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

pub(super) fn has(
    cx: &mut TestAppContext,
    w: WindowHandle<Root>,
    id: impl Into<gpui_kit::ElementId>,
) -> bool {
    let id = id.into();
    cx.update_window(w.into(), |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some()
    })
    .unwrap()
}

/// The Timeline inside a real shell with the default key bindings.
pub(super) fn shell(
    cx: &mut TestAppContext,
    src: FixtureSource,
) -> (WindowHandle<Root>, Entity<TimelineView>, Arc<FixtureSource>) {
    use crate::keymap::{self, Bindings};
    use crate::shell::{Shell, ShellModel};
    let src = Arc::new(src);
    let s = src.clone();
    let (window, shell) = cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
        keymap::bind(&Bindings::default(), cx);
        let (w, s) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| Shell::with_timeline_source(ShellModel::default(), s, window, cx))
        })
        .unwrap();
        (w.downcast::<Root>().unwrap(), s)
    });
    cx.run_until_parked();
    let t = shell.read_with(cx, |s, _| s.timeline.clone());
    advance(cx, EXACT_SETTLE);
    frame(cx, window);
    (window, t, src)
}

pub(super) fn press(cx: &mut TestAppContext, w: WindowHandle<Root>, k: &str) {
    cx.update_window(w.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press(k, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// `isTimelineShortcutSuppressedTarget`: inside the jump picker O does
/// nothing; L does nothing while the "latest" button is hidden.
#[gpui_kit::test]
fn timeline_keys_stay_out_of_the_jumper_and_latest_needs_its_button(cx: &mut TestAppContext) {
    let (w, t, src) = shell(cx, source_with(40, json!({})));
    let lists = |src: &FixtureSource| count(src, |c| matches!(c, Call::ListFrames { .. }));
    let before = lists(&src);
    press(cx, w, "l");
    assert_eq!(lists(&src), before, "at the head, L has nothing to do");

    press(cx, w, "j");
    assert!(t.read_with(cx, |t, _| t.jumper.open));
    press(cx, w, "o");
    assert!(
        !t.read_with(cx, |t, _| t.ocr.visible),
        "O inside the picker is the picker's"
    );
    press(cx, w, "escape");
    press(cx, w, "o");
    assert!(
        t.read_with(cx, |t, _| t.ocr.visible),
        "outside it, O shows OCR"
    );
}

/// The same with the audio drawer focused: its own keys only.
#[gpui_kit::test]
fn timeline_keys_stay_out_of_the_audio_drawer(cx: &mut TestAppContext) {
    use super::audio::fixture::{FakePlayer, FixtureAudio};
    let (w, t, _) = shell(
        cx,
        source_with(40, json!({ "list_audio_segments": mic_segment() })),
    );
    let audio = Arc::new(FixtureAudio::load("drawer").unwrap());
    let id = audio.opened().unwrap().id;
    t.update(cx, |t, cx| {
        t.drawer_deps = super::audio::Deps {
            backend: audio.clone(),
            open_player: FakePlayer::new(50.).factory(),
        };
        t.audio.push(super::model::AudioSegment {
            id,
            source: super::model::AudioSource::Microphone,
            segment_index: 3,
            start_ms: 1_793_782_821_000,
            end_ms: 1_793_782_871_000,
        });
        t.selected_audio = Some((id, 0));
        cx.notify();
    });
    frame(cx, w);
    assert!(t.read_with(cx, |t, _| t.drawer.is_some()));
    press(cx, w, "o");
    press(cx, w, "left");
    t.read_with(cx, |t, _| {
        assert!(!t.ocr.visible, "O in the drawer isn't the Timeline's");
        assert_eq!(
            t.paging.active, 0,
            "← in the drawer seeks audio, not frames"
        );
    });
}

/// `loadOcrForActiveFrame` + the reset effect: the provider shows beside the
/// toggle, the count beside its label, and moving the playhead hides it all.
#[gpui_kit::test]
fn ocr_shows_its_provider_and_hides_when_the_frame_changes(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx, source_with(5, ocr_done()));
    advance(cx, EXACT_SETTLE);
    click(cx, w, "ocr-toggle");
    v.read_with(cx, |t, _| {
        assert_eq!(t.ocr.status, OcrStatus::Success);
        assert_eq!(t.ocr.provider.as_deref(), Some("Apple Vision"));
        assert_eq!(t.ocr.boxes.len(), 2);
        assert_eq!(
            super::ocr_overlay::toggle_tip(t),
            "Hide OCR data for the active frame (O)"
        );
    });
    assert!(has(cx, w, "ocr-provider"), "the provider chip");
    assert!(has(cx, w, "ocr-rerun"));

    v.update(cx, |t, cx| t.step(1, cx));
    cx.run_until_parked();
    v.read_with(cx, |t, _| {
        assert!(!t.ocr.visible, "a new frame hides OCR");
        assert_eq!(t.ocr.frame_id, None);
        assert_eq!(t.ocr.status, OcrStatus::Idle);
    });
    assert!(!has(cx, w, "ocr-provider"));
}

/// `ocrRerunDisabled` with OCR off in Settings, "run OCR" for a frame never
/// OCR'd, and a failed job's own error.
#[gpui_kit::test]
fn rerun_respects_the_ocr_setting_and_a_failed_job_says_why(cx: &mut TestAppContext) {
    let off = json!({ "get_recording_settings": { "ocr": { "enabled": false } } });
    let (w, v, src) = open(cx, source_with(3, off));
    advance(cx, EXACT_SETTLE);
    click(cx, w, "ocr-toggle");
    assert_eq!(v.read_with(cx, |t, _| t.ocr.status), OcrStatus::Missing);
    assert!(v.read_with(cx, |t, _| t.rerun_disabled()));
    click(cx, w, "ocr-rerun");
    assert_eq!(
        count(&src, |c| matches!(c, Call::RerunOcr(_))),
        0,
        "OCR is off"
    );

    let (w, v, src) = open(cx, source_with(3, json!({})));
    advance(cx, EXACT_SETTLE);
    click(cx, w, "ocr-toggle");
    assert!(!v.read_with(cx, |t, _| t.rerun_disabled()));
    click(cx, w, "ocr-rerun");
    assert_eq!(count(&src, |c| matches!(c, Call::RerunOcr(_))), 1);

    let failed = json!({
        "list_processing_jobs": [{ "id": 9, "processor": "ocr", "status": "failed",
                                   "lastError": "Vision request failed" }]
    });
    let (w, v, _) = open(cx, source_with(3, failed));
    advance(cx, EXACT_SETTLE);
    click(cx, w, "ocr-toggle");
    v.read_with(cx, |t, _| {
        assert_eq!(t.ocr.status, OcrStatus::Error);
        assert_eq!(t.ocr.error.as_deref(), Some("Vision request failed"));
    });
}

/// `onAudioSegmentBarClick`: a bar opens its drawer, the same bar closes it.
#[gpui_kit::test]
fn the_open_audio_bar_closes_its_drawer(cx: &mut TestAppContext) {
    let (w, v, _) = open(
        cx,
        source_with(30, json!({ "list_audio_segments": mic_segment() })),
    );
    click(cx, w, nid("audio-bar", 77));
    assert_eq!(v.read_with(cx, |t, _| t.selected_audio), Some((77, 0)));
    click(cx, w, nid("audio-bar", 77));
    assert_eq!(v.read_with(cx, |t, _| t.selected_audio), None);
}

/// "audio unavailable" carries a retry that reloads the lane.
#[gpui_kit::test]
fn a_failed_audio_lane_retries(cx: &mut TestAppContext) {
    let reject = json!({ "list_audio_segments": { "__reject": "database is locked" } });
    let (w, v, src) = open(cx, source_with(30, reject));
    assert_eq!(
        v.read_with(cx, |t, _| t.audio_error.clone()).as_deref(),
        Some("database is locked")
    );
    assert!(has(cx, w, "audio-lane-retry"));
    *src.fail_audio.lock().unwrap() = None;
    click(cx, w, "audio-lane-retry");
    v.read_with(cx, |t, _| {
        assert_eq!(t.audio_error, None);
    });
}

/// A hand-off from Insights / Chat replaces the Recall landing; a frame that's
/// gone says so in plain words.
#[gpui_kit::test]
fn a_hand_off_drops_the_recall_landing_and_a_gone_frame_says_so(cx: &mut TestAppContext) {
    let (_, v, _) = open(cx, source_with(30, json!({})));
    v.update(cx, |t, cx| {
        t.land(
            super::model::Landing {
                query: "pricing".into(),
                frame_id: 9995,
                screen: true,
                by_meaning: false,
            },
            cx,
        )
    });
    cx.run_until_parked();
    assert!(v.read_with(cx, |t, _| t.landing.is_some()));
    v.update(cx, |t, cx| t.jump_to(9990, cx));
    cx.run_until_parked();
    v.read_with(cx, |t, _| {
        assert!(t.landing.is_none(), "only a Recall hit carries a query");
        assert_eq!(t.paging.active_frame().map(|f| f.id), Some(9990));
    });
    v.update(cx, |t, cx| t.jump_to(1, cx));
    cx.run_until_parked();
    let s = v.read_with(cx, |t, _| t.status.clone()).expect("a note");
    assert_eq!(s.message, "That capture is no longer available.");
    assert!(s.error);
}

/// `openCurrentFrameUrl`: no page is a neutral note and the menu stays open.
#[gpui_kit::test]
fn opening_a_frame_without_a_page_says_so_and_keeps_the_menu(cx: &mut TestAppContext) {
    let mut fs = frames(3);
    fs[0].url = Some("example.com/pricing".into());
    let src = source_with(0, json!({})).with_frames(fs);
    let (w, v, _) = open(cx, src);
    advance(cx, EXACT_SETTLE);
    v.update(cx, |t, cx| {
        t.menu_open = true;
        cx.notify();
    });
    frame(cx, w);
    click(cx, w, "action-open-host");
    v.read_with(cx, |t, _| {
        let s = t.status.as_ref().expect("a note");
        assert_eq!(s.message, "No openable URL for this frame");
        assert!(!s.error, "neutral, it clears itself");
        assert!(t.menu_open, "only an opened page closes the menu");
    });
}

/// The rail is a slider: focused (a click focuses it), Home / End / Page keys scrub.
#[gpui_kit::test]
fn the_focused_rail_takes_home_end_and_page_keys(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx, source_with(40, json!({})));
    // The rail isn't marked for the test observer (the marker reorders its paint
    // layers in test builds), so focus it the way its mouse-down does.
    let focus = v.read_with(cx, |t, _| t.rail_focus.clone());
    cx.update_window(w.into(), |_, window, cx| window.focus(&focus, cx))
        .unwrap();
    let active = |cx: &mut TestAppContext| v.read_with(cx, |t, _| t.paging.active);
    let start = active(cx);
    press(cx, w, "pagedown");
    assert_eq!(active(cx), start + 10);
    press(cx, w, "end");
    assert_eq!(active(cx), 39, "End is the oldest loaded frame");
    press(cx, w, "pageup");
    assert_eq!(active(cx), 29);
    press(cx, w, "home");
    assert_eq!(active(cx), 0);
}

/// The shortcuts sheet: Timeline and Frame groups, rows that can't act left out.
#[gpui_kit::test]
fn the_shortcuts_sheet_has_timeline_and_frame_groups(cx: &mut TestAppContext) {
    let (_, v, _) = open(
        cx,
        source_with(5, json!({ "list_audio_segments": mic_segment() })),
    );
    let b = crate::keymap::Bindings::default();
    let groups = cx.update(|cx| v.read(cx).help_groups(&b, cx));
    let titles: Vec<_> = groups.iter().map(|g| g.title).collect();
    assert_eq!(titles, ["Timeline", "Frame"]);
    let labels =
        |i: usize| -> Vec<String> { groups[i].rows.iter().map(|r| r.label.clone()).collect() };
    assert!(
        !labels(0).contains(&"Jump to latest".to_string()),
        "at the head"
    );
    assert!(labels(0).contains(&"Close the top open surface".to_string()));
    assert!(labels(1).contains(&"Show OCR panel".to_string()));
    assert!(labels(1).contains(&"Play audio at this moment".to_string()));
}

/// Screen off, audio recorded: the stage lists the newest segments and each
/// opens its drawer.
#[gpui_kit::test]
fn the_audio_only_stage_opens_a_recent_segment(cx: &mut TestAppContext) {
    let (w, v, _) = open(
        cx,
        source_with(0, json!({ "list_audio_segments": mic_segment() })),
    );
    assert!(has(cx, w, "timeline-record-screen"));
    click(cx, w, nid("cover-seg", 77));
    assert_eq!(v.read_with(cx, |t, _| t.selected_audio), Some((77, 0)));
}
