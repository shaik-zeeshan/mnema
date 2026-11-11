//! More flows from the parity pass: the ⋯ menu by keyboard, "Start recording"
//! while a start is in flight, a refresh dropping the open segment, and the
//! developer panel's duplicate-of link.

use std::time::Duration;

use gpui_kit::{AppContext as _, TestAppContext};
use serde_json::json;

use super::flow_tests::{click, has, mic_segment, press, shell, source_with};
use super::scrub_engine::EXACT_SETTLE;
use super::tests::{advance, frame, open};
use super::{TimelineEvent, DUPLICATE_LOOKUP};
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
