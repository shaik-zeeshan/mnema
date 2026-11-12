//! More flows from the parity pass: the ⋯ menu by keyboard, "Start recording"
//! while a start is in flight, a refresh dropping the open segment, and the
//! developer panel's duplicate-of link; the head poll off the Timeline, OCR
//! re-checks on a timer and stale side data.

use std::sync::Arc;
use std::time::Duration;

use capture_types::AppearanceSetting;
use gpui_kit::{AppContext as _, Entity, TestAppContext, WindowAppearance, WindowOptions};
use serde_json::json;

use super::data::{OCR_POLL, OCR_STILL_RUNNING, OCR_WAIT};
use super::fixture::Call;
use super::flow_tests::{click, has, mic_segment, ocr_done, press, shell, source_with};
use super::paging::{HEAD_POLL, POLL_PAGE};
use super::scrub_engine::EXACT_SETTLE;
use super::tests::{advance, count, frame, frames, open};
use super::{OcrStatus, TimelineEvent, TimelineView, DUPLICATE_LOOKUP};
use crate::ui::nid;

/// Enter on the ⋯ trigger opens the menu with focus on its first live row;
/// Esc closes it and hands focus back to the trigger.
#[gpui_kit::test]
fn the_frame_menu_opens_by_keyboard_and_esc_returns_focus(cx: &mut TestAppContext) {
    let (w, t, _) = shell(cx, source_with(5, json!({})));
    let trigger = t.read_with(cx, |t, _| t.menu_trigger.clone());
    cx.update_window(w.into(), |_, window, cx| window.focus(&trigger, cx))
        .unwrap();
    press(cx, w, "enter");
    frame(cx, w);
    let (open, first) = t.read_with(cx, |t, _| (t.menu_open, t.menu_items[0].clone()));
    assert!(open, "Enter on the trigger opens the menu");
    let focused = cx
        .update_window(w.into(), |_, window, _| first.is_focused(window))
        .unwrap();
    assert!(focused, "focus moves to Copy image");
    press(cx, w, "c");
    assert!(
        t.read_with(cx, |t, _| t.status.is_none()),
        "C inside the menu isn't the Timeline's shortcut"
    );
    press(cx, w, "escape");
    assert!(!t.read_with(cx, |t, _| t.menu_open), "Esc closes the menu");
    let back = cx
        .update_window(w.into(), |_, window, _| trigger.is_focused(window))
        .unwrap();
    assert!(back, "and focus returns to the trigger");
}

/// "Start recording" waits while a start is in flight (`loadingStart`).
#[gpui_kit::test]
fn start_recording_waits_while_a_start_is_in_flight(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx, source_with(0, json!({})));
    let starts = std::rc::Rc::new(std::cell::Cell::new(0));
    let n = starts.clone();
    let _sub = cx.update(|cx| {
        cx.subscribe(&v, move |_, e: &TimelineEvent, _| {
            if *e == TimelineEvent::StartRecording {
                n.set(n.get() + 1);
            }
        })
    });
    v.update(cx, |t, cx| {
        t.capture.starting = true;
        cx.notify();
    });
    click(cx, w, "timeline-start");
    assert_eq!(starts.get(), 0, "disabled while starting");
    v.update(cx, |t, cx| {
        t.capture.starting = false;
        cx.notify();
    });
    click(cx, w, "timeline-start");
    assert_eq!(starts.get(), 1);
}

/// A refresh that no longer lists the open segment closes its drawer; a
/// pinned (landed) one stays.
#[gpui_kit::test]
fn a_refresh_that_drops_the_open_segment_closes_its_drawer(cx: &mut TestAppContext) {
    let (w, v, src) = open(
        cx,
        source_with(30, json!({ "list_audio_segments": mic_segment() })),
    );
    click(cx, w, nid("audio-bar", 77));
    assert_eq!(v.read_with(cx, |t, _| t.selected_audio), Some((77, 0)));
    src.audio.lock().unwrap().clear();
    click(cx, w, "timeline-refresh");
    assert_eq!(v.read_with(cx, |t, _| t.selected_audio), None);
}

/// Developer options: 200 ms after the playhead rests, the panel names the
/// frame this one duplicates; its link jumps there.
#[gpui_kit::test]
fn the_developer_panel_links_the_frame_it_duplicates(cx: &mut TestAppContext) {
    let earlier = json!({
        "id": 9990, "sessionId": "s", "filePath": "/nonexistent/f.webp",
        "capturedAt": "2026-11-04T09:01:36Z", "width": 2880, "height": 1800
    });
    // The shell lays the stage out at full height (a bare view collapses it).
    let (w, v, _) = shell(
        cx,
        source_with(
            30,
            json!({ "get_earliest_earlier_equivalent_frame": earlier }),
        ),
    );
    v.update(cx, |t, cx| {
        t.developer = true;
        t.step(1, cx);
    });
    cx.run_until_parked();
    advance(cx, DUPLICATE_LOOKUP - Duration::from_millis(1));
    assert!(
        v.read_with(cx, |t, _| t.duplicate_of.is_none()),
        "waits 200 ms"
    );
    advance(cx, Duration::from_millis(1));
    assert_eq!(
        v.read_with(cx, |t, _| t.duplicate_of.as_ref().map(|f| f.id)),
        Some(9990)
    );
    advance(cx, EXACT_SETTLE);
    assert!(has(cx, w, "dev-duplicate-of"));
    click(cx, w, "dev-duplicate-of");
    assert_eq!(
        v.read_with(cx, |t, _| t.paging.active_frame().map(|f| f.id)),
        Some(9990)
    );
}

/// The head poll runs only while the shell shows the Timeline; a return checks
/// the head at once and re-arms the 1.5 s poll.
#[gpui_kit::test]
fn the_head_poll_pauses_off_the_timeline_and_checks_on_return(cx: &mut TestAppContext) {
    use crate::shell::{Shell, ShellModel, Surface};
    let src = Arc::new(source_with(10, json!({})));
    let s = src.clone();
    let shell = cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
        let (_, shell) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| Shell::with_timeline_source(ShellModel::default(), s, window, cx))
        })
        .unwrap();
        shell
    });
    cx.run_until_parked();
    let polls = || {
        count(
            &src,
            |c| matches!(c, Call::ListFrames { limit, .. } if *limit == POLL_PAGE),
        )
    };
    advance(cx, HEAD_POLL);
    assert_eq!(polls(), 1, "polls on the Timeline");
    shell.update(cx, |s, cx| s.go(Surface::Insights, cx));
    for _ in 0..4 {
        advance(cx, HEAD_POLL);
    }
    assert_eq!(polls(), 1, "no head calls off the Timeline");
    shell.update(cx, |s, cx| s.go(Surface::Timeline, cx));
    cx.run_until_parked();
    assert_eq!(polls(), 2, "a return checks at once");
    advance(cx, HEAD_POLL - Duration::from_millis(1));
    assert_eq!(polls(), 2);
    advance(cx, Duration::from_millis(1));
    assert_eq!(polls(), 3, "then every 1.5 s again");
}

/// Opens the OCR overlay over a job that answers "still running" `pending`
/// times, then re-checks every 750 ms up to `30 s` on the test clock.
fn ocr_after(cx: &mut TestAppContext, pending: u32) -> Entity<TimelineView> {
    let (w, v, src) = open(cx, source_with(5, ocr_done()));
    advance(cx, EXACT_SETTLE);
    *src.ocr_pending.lock().unwrap() = pending;
    click(cx, w, "ocr-toggle");
    // One timer per re-check: the clock moves a poll at a time.
    for _ in 0..(OCR_WAIT.as_millis() / OCR_POLL.as_millis()) {
        advance(cx, OCR_POLL);
    }
    assert_eq!(
        v.read_with(cx, |t, _| t.ocr.status),
        OcrStatus::Running,
        "30 s"
    );
    assert_eq!(count(&src, |c| matches!(c, Call::OcrJob(_))), 40);
    advance(cx, OCR_POLL);
    v
}

/// `loadOcrFromJob`'s 30 s cap, on both sides: the 30.75 s re-check is the
/// last that can land; still running then says so.
#[gpui_kit::test]
fn a_running_ocr_job_is_rechecked_on_a_timer_up_to_30s(cx: &mut TestAppContext) {
    // The first look and 40 re-checks (0.75 … 30 s) are pending; 30.75 s lands.
    let v = ocr_after(cx, 41);
    v.read_with(cx, |t, _| {
        assert_eq!(t.ocr.status, OcrStatus::Success);
        assert_eq!(t.ocr.boxes.len(), 2);
    });

    let v = ocr_after(cx, 42);
    v.read_with(cx, |t, _| {
        assert_eq!(t.ocr.status, OcrStatus::Error);
        assert_eq!(t.ocr.error.as_deref(), Some(OCR_STILL_RUNNING));
    });
}

/// A new active frame drops the pending OCR's re-checks with it.
#[gpui_kit::test]
fn moving_the_playhead_stops_the_ocr_rechecks(cx: &mut TestAppContext) {
    let (w, v, src) = open(cx, source_with(5, ocr_done()));
    advance(cx, EXACT_SETTLE);
    *src.ocr_pending.lock().unwrap() = u32::MAX;
    click(cx, w, "ocr-toggle");
    advance(cx, OCR_POLL);
    let jobs = || count(&src, |c| matches!(c, Call::OcrJob(_)));
    assert_eq!(jobs(), 1);
    v.update(cx, |t, cx| t.step(1, cx));
    cx.run_until_parked();
    advance(cx, OCR_POLL * 4);
    assert_eq!(jobs(), 1, "no re-check after the frame changed");
}

/// Two side-data fetches in flight: whichever finishes last, the newer
/// window's answer is the one that stays (the seed varies the order).
#[gpui_kit::test(iterations = 12)]
fn an_older_side_data_answer_never_lands_over_a_newer_one(cx: &mut TestAppContext) {
    use super::model::{AudioSegment, AudioSource};
    const DAY: i64 = 86_400_000;
    let newest = frames(1)[0].captured_at_ms;
    let seg = |id, start_ms: i64| AudioSegment {
        id,
        source: AudioSource::Microphone,
        segment_index: 0,
        start_ms,
        end_ms: start_ms + 30_000,
    };
    let mut src = source_with(10, json!({}));
    src.audio_in_range = true;
    *src.audio.lock().unwrap() = vec![seg(1, newest - 10_000), seg(2, newest - DAY)];
    let (_, v, _) = open(cx, src);
    v.update(cx, |t, cx| {
        let live = t.paging.frames.clone();
        // A window a day back first (it lists segment 2), then the live one (1).
        t.paging.frames = live
            .iter()
            .cloned()
            .map(|mut f| {
                f.captured_at_ms -= DAY;
                f
            })
            .collect();
        t.on_core_event(&app_core::CoreEvent::AudioSegmentsChanged, cx);
        t.paging.frames = live;
        t.on_core_event(&app_core::CoreEvent::AudioSegmentsChanged, cx);
    });
    cx.run_until_parked();
    v.read_with(cx, |t, _| {
        assert_eq!(t.audio.iter().map(|s| s.id).collect::<Vec<_>>(), vec![1]);
        assert!(!t.audio_loading);
    });
}
