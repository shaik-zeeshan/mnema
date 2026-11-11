//! Layer 2 for the Timeline: every §5 scrub and paging constant through the view
//! with `advance_clock`, on both sides of its boundary, plus the keys.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use capture_types::AppearanceSetting;
use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    point, px, size, AppContext as _, Bounds, Entity, ScrollDelta, TestAppContext,
    WindowAppearance, WindowBounds, WindowHandle, WindowOptions,
};
use serde_json::json;

use super::fixture::{Call, FixtureSource};
use super::model::Frame;
use super::paging::{HEAD_POLL, PAGE_SIZE, POLL_PAGE, PREFETCH_AHEAD};
use super::rail::READOUT_FLY;
use super::scrub_engine::{
    Mode, DISPLAY_SETTLE, DISPLAY_THROTTLE, EXACT_SETTLE, FAILURE_TTL, THROTTLE, THROTTLE_FAST,
    WARM_SETTLE,
};
use super::{TimelineView, STATUS_TTL};
use crate::theme;

const MS: Duration = Duration::from_millis(1);

/// `n` frames 2 s apart, newest first, app runs of 10 alternating Slack / Arc.
fn frames(n: i64) -> Vec<Frame> {
    let shot = Path::new(env!("CARGO_MANIFEST_DIR")).join("../web/src/assets/shots/desk-code.webp");
    (0..n)
        .map(|i| Frame {
            id: 10_000 - i,
            session_id: "s".into(),
            file_path: Arc::from(shot.as_path()),
            captured_at_ms: 1_793_782_916_000 - i * 2000,
            width: Some(2880),
            height: Some(1800),
            app_bundle_id: Some(
                if (i / 10) % 2 == 0 {
                    "com.slack"
                } else {
                    "com.arc"
                }
                .into(),
            ),
            app_name: Some(if (i / 10) % 2 == 0 { "Slack" } else { "Arc" }.into()),
            window_title: None,
            url: None,
            equivalence_hint: None,
        })
        .collect()
}

fn source(n: i64) -> FixtureSource {
    FixtureSource::from_map(&json!({
        "get_frame_preview": { "mimeType": "image/webp", "sourceKind": "original_frame" }
    }))
    .with_frames(frames(n))
}

fn open(
    cx: &mut TestAppContext,
    src: FixtureSource,
) -> (WindowHandle<Root>, Entity<TimelineView>, Arc<FixtureSource>) {
    let src = Arc::new(src);
    let s = src.clone();
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
        let (window, view) =
            gpui_kit::open_window(options, cx, |_, cx| cx.new(|cx| TimelineView::new(s, cx)))
                .expect("window");
        (window.downcast::<Root>().expect("root"), view)
    });
    cx.run_until_parked();
    frame(cx, window);
    (window, view, src)
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

fn count(src: &FixtureSource, f: impl Fn(&Call) -> bool) -> usize {
    src.calls().iter().filter(|c| f(c)).count()
}

fn preview_calls(src: &FixtureSource, id: i64) -> usize {
    count(src, |c| *c == Call::Preview(id))
}

fn step(cx: &mut TestAppContext, v: &Entity<TimelineView>, d: i64) {
    v.update(cx, |t, cx| t.step(d, cx));
    cx.run_until_parked();
}

fn wheel(cx: &mut TestAppContext, window: WindowHandle<Root>, dy: f32) {
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.scroll("timeline", ScrollDelta::Pixels(point(px(0.), px(dy))), cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_exact_frame_comes_500ms_after_a_scrub_rests(cx: &mut TestAppContext) {
    let (_, v, src) = open(cx, source(40));
    assert_eq!(
        preview_calls(&src, 10_000),
        1,
        "a parked load fetches at once"
    );
    step(cx, &v, 1);
    advance(cx, EXACT_SETTLE - MS);
    assert_eq!(preview_calls(&src, 9_999), 0, "499 ms: not yet");
    advance(cx, MS);
    assert_eq!(preview_calls(&src, 9_999), 1, "500 ms: the exact frame");
}

#[gpui_kit::test]
fn the_scrub_tier_waits_160ms_at_rest_speed(cx: &mut TestAppContext) {
    let (_, v, src) = open(cx, source(40));
    step(cx, &v, 1);
    let batches = |s: &FixtureSource| count(s, |c| *c == Call::Scrub { enqueue: true });
    advance(cx, THROTTLE - MS);
    assert_eq!(batches(&src), 0, "159 ms");
    advance(cx, MS);
    assert_eq!(batches(&src), 1, "160 ms");
}

#[gpui_kit::test]
fn a_fast_scrub_throttles_at_240ms(cx: &mut TestAppContext) {
    let (window, _, src) = open(cx, source(400));
    // A first scrub, and its batch out of the way.
    wheel(cx, window, -8.);
    advance(cx, Duration::from_millis(300));
    // 1 px (no new frame) then 80 px 10 ms later = 8 px/ms: the fast throttle.
    wheel(cx, window, -1.);
    advance(cx, Duration::from_millis(10));
    wheel(cx, window, -80.);
    let batches = |s: &FixtureSource| count(s, |c| *c == Call::Scrub { enqueue: true });
    let before = batches(&src);
    advance(cx, THROTTLE_FAST - MS);
    assert_eq!(batches(&src), before, "239 ms");
    advance(cx, MS);
    assert_eq!(batches(&src), before + 1, "240 ms");
}

#[gpui_kit::test]
fn the_stage_swaps_at_most_every_80ms_and_parks_after_180ms(cx: &mut TestAppContext) {
    let (_, v, _) = open(cx, source(40).with_scrub_ready());
    // Warm the scrub tier around frame 2 (radius 4 covers 0..=6).
    step(cx, &v, 2);
    advance(cx, THROTTLE);
    advance(cx, Duration::from_millis(500));
    let shown =
        |cx: &mut TestAppContext| v.read_with(cx, |t, _| t.engine.display().map(|d| d.frame_id));
    step(cx, &v, 1); // → index 3, swaps
    assert_eq!(shown(cx), Some(9_997));
    advance(cx, DISPLAY_THROTTLE - MS);
    step(cx, &v, 1); // 79 ms: index 4, no swap
    assert_eq!(shown(cx), Some(9_997));
    advance(cx, MS);
    step(cx, &v, 1); // 80 ms: index 5, swaps
    assert_eq!(shown(cx), Some(9_995));
    advance(cx, DISPLAY_SETTLE - MS);
    assert_eq!(
        v.read_with(cx, |t, _| t.engine.mode()),
        Mode::Scrubbing,
        "179 ms"
    );
    advance(cx, MS);
    assert_eq!(
        v.read_with(cx, |t, _| t.engine.mode()),
        Mode::Parked,
        "180 ms"
    );
}

#[gpui_kit::test]
fn the_warm_up_reads_availability_1200ms_after_the_scrub(cx: &mut TestAppContext) {
    let (_, v, src) = open(cx, source(40));
    step(cx, &v, 3);
    let warms = |s: &FixtureSource| count(s, |c| *c == Call::Scrub { enqueue: false });
    advance(cx, WARM_SETTLE - MS);
    assert_eq!(warms(&src), 0, "1199 ms");
    advance(cx, MS);
    assert_eq!(warms(&src), 1, "1200 ms");
}

/// Fail frame 0's preview at t0, then park back on it so its 500 ms settle ends
/// at `t0 + end`; returns how often frame 0 was asked for.
fn failed_preview_calls_at(cx: &mut TestAppContext, end: Duration) -> usize {
    // No `get_frame_preview` in the map: every preview answers "gone".
    let (_, v, src) = open(
        cx,
        FixtureSource::from_map(&json!({})).with_frames(frames(10)),
    );
    assert_eq!(preview_calls(&src, 10_000), 1);
    advance(cx, end - EXACT_SETTLE);
    for i in [1, 0] {
        v.update(cx, |t, cx| t.scrub_to(i, false, cx));
        cx.run_until_parked();
    }
    advance(cx, EXACT_SETTLE);
    preview_calls(&src, 10_000)
}

#[gpui_kit::test]
fn a_failed_preview_is_still_cached_at_4999ms(cx: &mut TestAppContext) {
    assert_eq!(failed_preview_calls_at(cx, FAILURE_TTL - MS), 1);
}

#[gpui_kit::test]
fn a_failed_preview_is_refetched_at_5000ms(cx: &mut TestAppContext) {
    assert_eq!(failed_preview_calls_at(cx, FAILURE_TTL), 2);
}

#[gpui_kit::test]
fn the_image_cache_retries_a_failed_decode_after_5s(cx: &mut TestAppContext) {
    // Frames whose image file is gone: the stage's load fails and is cached.
    let gone: Vec<Frame> = frames(3)
        .into_iter()
        .map(|f| Frame {
            file_path: Arc::from(Path::new("/nonexistent/frame.jpg")),
            ..f
        })
        .collect();
    let (window, v, _) = open(cx, source(3).with_frames(gone));
    let images = v.read_with(cx, |t, _| t.images.clone());
    let loads = |cx: &mut TestAppContext| {
        cx.update_window(window.into(), |_, w, cx| {
            w.refresh();
            w.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        images.read_with(cx, |c, _| c.loads)
    };
    let first = loads(cx);
    assert_eq!(first, 1, "one load for the stage's frame");
    assert_eq!(loads(cx), first, "the failure is cached");
    advance(cx, FAILURE_TTL - MS);
    assert_eq!(loads(cx), first, "4999 ms");
    advance(cx, MS);
    assert_eq!(loads(cx), first + 1, "5000 ms: reloaded");
}

#[gpui_kit::test]
fn the_head_polls_every_1500ms(cx: &mut TestAppContext) {
    let (_, _, src) = open(cx, source(10));
    let polls = |s: &FixtureSource| {
        count(
            s,
            |c| matches!(c, Call::ListFrames { limit, .. } if *limit == POLL_PAGE),
        )
    };
    advance(cx, HEAD_POLL - MS);
    assert_eq!(polls(&src), 0, "1499 ms");
    advance(cx, MS);
    assert_eq!(polls(&src), 1, "1500 ms");
}

#[gpui_kit::test]
fn the_next_page_loads_120_frames_before_the_tail(cx: &mut TestAppContext) {
    let (_, v, src) = open(cx, source(300));
    let older = |s: &FixtureSource| {
        count(
            s,
            |c| matches!(c, Call::ListFrames { before: Some(_), limit } if *limit == PAGE_SIZE),
        )
    };
    step(cx, &v, (PAGE_SIZE - PREFETCH_AHEAD - 1) as i64);
    assert_eq!(older(&src), 0, "121 from the tail");
    step(cx, &v, 1);
    assert_eq!(older(&src), 1, "120 from the tail");
    assert_eq!(v.read_with(cx, |t, _| t.paging.frames.len()), 300);
}

#[gpui_kit::test]
fn wheel_anywhere_scrubs_and_the_readout_flies_190ms(cx: &mut TestAppContext) {
    let (window, v, _) = open(cx, source(40));
    let x = |cx: &mut TestAppContext| {
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.find("readout-name").bounds().origin.x
        })
        .unwrap()
    };
    // 9 slots older crosses Slack (0..=9) into Arc (10..).
    wheel(cx, window, -80.);
    assert_eq!(v.read_with(cx, |t, _| t.paging.active), 10);
    let start = x(cx);
    advance(cx, READOUT_FLY / 2);
    let mid = x(cx);
    advance(cx, READOUT_FLY - READOUT_FLY / 2 - MS);
    let at_189 = x(cx);
    advance(cx, MS);
    let at_190 = x(cx);
    advance(cx, Duration::from_millis(500));
    let landed = x(cx);
    assert!(start < mid && mid < landed, "older flies in from the left");
    // Ease-out (quart): the last millisecond is sub-pixel, so 189 ms is 'not past'.
    assert!(
        at_189 <= landed && at_189 > mid,
        "189 ms: still short of landing"
    );
    assert_eq!(at_190, landed, "190 ms: landed");
}

#[gpui_kit::test]
fn a_neutral_stage_note_clears_after_2200ms(cx: &mut TestAppContext) {
    let (_, v, _) = open(cx, source(3));
    v.update(cx, |t, cx| {
        t.set_status(Some("Copied frame 1".into()), None, false, cx)
    });
    advance(cx, STATUS_TTL - MS);
    assert!(v.read_with(cx, |t, _| t.status.is_some()), "2199 ms");
    advance(cx, MS);
    assert!(v.read_with(cx, |t, _| t.status.is_none()), "2200 ms");
    v.update(cx, |t, cx| {
        t.set_status(Some("boom".into()), None, true, cx)
    });
    advance(cx, STATUS_TTL * 2);
    assert!(v.read_with(cx, |t, _| t.status.is_some()), "errors stay");
}

#[gpui_kit::test]
fn arrows_step_one_and_shift_ten_and_j_opens_the_jumper(cx: &mut TestAppContext) {
    use crate::keymap::{self, Bindings};
    use crate::shell::{Shell, ShellModel};
    let src = Arc::new(source(40));
    let (window, shell) = cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
        keymap::bind(&Bindings::default(), cx);
        let (w, s) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| Shell::with_timeline_source(ShellModel::default(), src, window, cx))
        })
        .unwrap();
        (w.downcast::<Root>().unwrap(), s)
    });
    cx.run_until_parked();
    let t = shell.read_with(cx, |s, _| s.timeline.clone());
    let press = |cx: &mut TestAppContext, k: &str| {
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.press(k, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    let active = |cx: &mut TestAppContext| t.read_with(cx, |t, _| t.paging.active);
    press(cx, "left");
    assert_eq!(active(cx), 1);
    press(cx, "shift-left");
    assert_eq!(active(cx), 11);
    press(cx, "right");
    assert_eq!(active(cx), 10);
    press(cx, "shift-right");
    assert_eq!(active(cx), 0);
    press(cx, "j");
    assert!(t.read_with(cx, |t, _| t.jumper.open));
    press(cx, "escape");
    assert!(
        !t.read_with(cx, |t, _| t.jumper.open),
        "Esc closes the jumper first"
    );
}

/// An agent's link / a Quick Recall audio hit / an Insights audio ref lands on
/// the segment with its drawer open at the seek, on the frame nearest the
/// moment; a segment that's gone says so (`openBrokerCaptureResult`).
#[gpui_kit::test]
fn land_audio_opens_the_segment_at_its_moment_or_says_it_is_gone(cx: &mut TestAppContext) {
    let src = FixtureSource::from_map(&json!({
        "get_frame_preview": { "mimeType": "image/webp", "sourceKind": "original_frame" },
        "list_audio_segments": [{
            "id": 77, "sourceKind": "microphone", "segmentIndex": 1,
            "startedAt": "2026-11-04T09:01:36Z", "endedAt": "2026-11-04T09:02:06Z"
        }]
    }))
    .with_frames(frames(30));
    let (_, v, _) = open(cx, src);

    // 20 s before the newest frame, 1.5 s in: the latest frame at or before is 10 back.
    v.update(cx, |t, cx| t.land_audio(77, 1500, None, None, cx));
    cx.run_until_parked();
    v.read_with(cx, |t, _| {
        assert_eq!(
            t.selected_audio,
            Some((77, 1500)),
            "the drawer opens at the seek"
        );
        assert_eq!(t.paging.active_frame().map(|f| f.id), Some(9990));
        assert!(t.pinned_audio.is_some());
    });
    v.update(cx, |t, cx| t.close_drawer(cx));
    assert!(v.read_with(cx, |t, _| t.pinned_audio.is_none()));

    v.update(cx, |t, cx| t.land_audio(404, 0, None, None, cx));
    cx.run_until_parked();
    v.read_with(cx, |t, _| {
        assert_eq!(t.selected_audio, None);
        let status = t.status.as_ref().expect("a visible note");
        assert_eq!(status.message, "That capture is no longer available.");
        assert!(status.error);
    });
}

#[gpui_kit::test]
fn copy_image_puts_a_png_on_the_clipboard_and_copy_text_the_lines(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx, source(3));
    advance(cx, EXACT_SETTLE);
    frame(cx, w);
    let click = |cx: &mut TestAppContext, id: &'static str| {
        cx.update_window(w.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    let status = |cx: &mut TestAppContext| {
        v.read_with(cx, |t, _| t.status.as_ref().map(|s| s.message.clone()))
    };
    // The ⋯ trigger itself is drawn (timeline-populated golden) but the test
    // observer reads it as clipped inside the stage; open the menu as it does.
    let open_menu = |cx: &mut TestAppContext| {
        v.update(cx, |t, cx| {
            t.menu_open = true;
            cx.notify();
        });
        frame(cx, w);
    };
    open_menu(cx);
    click(cx, "action-copy");
    // The stored preview is WebP; most apps can't paste that, so it goes over as PNG.
    let image = cx.read(|cx| {
        cx.read_from_clipboard().and_then(|c| {
            c.into_entries().find_map(|e| match e {
                gpui_kit::ClipboardEntry::Image(i) => Some(i),
                _ => None,
            })
        })
    });
    let image = image.expect("an image on the clipboard");
    assert_eq!(image.format, gpui_kit::ImageFormat::Png);
    assert!(image.bytes.starts_with(b"\x89PNG"), "real PNG bytes");
    assert_eq!(status(cx).as_deref(), Some("Copied frame 10000"));

    let id = v.read_with(cx, |t, _| t.paging.active_frame().map(|f| f.id));
    v.update(cx, |t, cx| {
        t.ocr.frame_id = id;
        t.ocr.status = super::OcrStatus::Success;
        t.ocr.boxes = ["Pricing", "Pro plan · $12"]
            .map(|text| super::model::OcrBox {
                x: 0.,
                y: 0.,
                w: 0.1,
                h: 0.1,
                text: text.into(),
                confidence: 1.,
            })
            .to_vec();
        cx.notify();
    });
    open_menu(cx);
    click(cx, "action-copy-text");
    let text = cx.read(|cx| cx.read_from_clipboard().and_then(|c| c.text()));
    assert_eq!(text.as_deref(), Some("Pricing\nPro plan · $12"));
    assert_eq!(status(cx).as_deref(), Some("Copied 2 text regions"));
}

#[gpui_kit::test]
fn the_jumper_trigger_closes_an_open_jumper(cx: &mut TestAppContext) {
    let (w, v, _) = open(cx, source(3));
    advance(cx, EXACT_SETTLE);
    frame(cx, w);
    let click = |cx: &mut TestAppContext| {
        cx.update_window(w.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("jumper-trigger", cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    click(cx);
    assert!(
        v.read_with(cx, |t, _| t.jumper.open),
        "the trigger opens it"
    );
    click(cx);
    assert!(
        !v.read_with(cx, |t, _| t.jumper.open),
        "and closes it again"
    );
}
