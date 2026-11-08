//! Layer 2 for the AudioDrawer: the transport keys in the drawer's scope (and
//! only there), the layered Esc, the 1000 ms job poll and the 8 s sample bound on
//! both sides of their boundaries, and a repair write end to end.

use std::sync::Arc;
use std::time::Duration;

use capture_types::AppearanceSetting;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Entity, TestAppContext, VisualTestContext, WindowAppearance};
use serde_json::{json, Value};

use super::data::Write;
use super::fixture::{FakePlayer, FixtureAudio};
use super::model::{Segment, Src};
use super::transport::{Player, Transport, SAMPLE_SECONDS};
use super::{AudioDrawer, Deps, Seek, POLL};
use crate::{keymap, theme};

const MS: Duration = Duration::from_millis(1);
const SEGMENT: i64 = 901;
const DURATION: f64 = 50.;

fn segment() -> Segment {
    Segment {
        id: SEGMENT,
        source: Src::Mic,
        session_id: "sess-1104".into(),
        segment_index: 3,
        file_path: "/nonexistent/mic-3.m4a".into(),
        start_ms: 1_793_782_821_000,
        end_ms: 1_793_782_871_000,
    }
}

fn map(job_status: &str) -> Value {
    json!({
        "get_audio_segment": {
            "id": SEGMENT, "sourceKind": "microphone", "sourceSessionId": "sess-1104",
            "segmentIndex": 3, "filePath": "/nonexistent/mic-3.m4a",
            "startedAt": "2026-11-04T09:00:21.000Z", "endedAt": "2026-11-04T09:01:11.000Z"
        },
        "list_processing_jobs": [
            { "id": 1, "processor": "audio_transcription", "status": job_status, "queuedAt": "2026-11-04T09:01:12Z" },
            { "id": 2, "processor": "speaker_analysis", "status": "completed", "queuedAt": "2026-11-04T09:01:12Z" }
        ],
        "get_processing_result": {
            "resultText": "Can we ship it today? Yes, after the review.",
            "structuredPayloadJson": "{\"words\":[{\"startMs\":0,\"endMs\":400,\"text\":\"Can\"}]}"
        },
        "list_speaker_turns": [
            { "id": 11, "sessionId": "sess-1104", "clusterId": 7, "speakerLabel": "Unknown speaker 1",
              "startMs": 0, "endMs": 2000, "transcriptText": "Can we ship it today?" },
            { "id": 12, "sessionId": "sess-1104", "clusterId": 8, "speakerLabel": "Unknown speaker 2",
              "startMs": 2000, "endMs": 5000, "transcriptText": "Yes, after the review." }
        ],
        "list_speaker_clusters": [
            { "id": 7, "speakerLabel": "Unknown speaker 1" }, { "id": 8, "speakerLabel": "Unknown speaker 2" }
        ],
        "list_person_profiles": [ { "id": 3, "displayName": "Ana Ruiz", "embeddingCount": 4 } ]
    })
}

fn open(
    cx: &mut TestAppContext,
    map: Value,
) -> (
    Entity<AudioDrawer>,
    &mut VisualTestContext,
    Arc<FixtureAudio>,
    FakePlayer,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
        keymap::bind(&keymap::Bindings::default(), cx);
    });
    let backend = Arc::new(FixtureAudio::from_map(map));
    let player = FakePlayer::new(DURATION);
    let deps = Deps {
        backend: backend.clone(),
        open_player: player.factory(),
    };
    let (view, vcx) =
        cx.add_window_view(|window, cx| AudioDrawer::new(deps, segment(), 0, window, cx));
    vcx.run_until_parked();
    (view, vcx, backend, player)
}

fn keys(vcx: &mut VisualTestContext, k: &str) {
    vcx.update(|window, cx| window.render_frame(cx));
    vcx.simulate_keystrokes(k);
    vcx.run_until_parked();
}

#[gpui_kit::test]
fn space_plays_and_the_arrows_seek_5s_and_30s(cx: &mut TestAppContext) {
    let (_, vcx, _, player) = open(cx, map("completed"));
    keys(vcx, "space");
    assert!(player.state().1, "Space plays");
    keys(vcx, "right");
    assert_eq!(player.state().0, 5.);
    keys(vcx, "shift-right");
    assert_eq!(player.state().0, 35.);
    keys(vcx, "left");
    assert_eq!(player.state().0, 30.);
    keys(vcx, "shift-right shift-right");
    assert_eq!(player.state().0, DURATION, "clamped at the end");
    keys(vcx, "shift-left shift-left shift-left");
    assert_eq!(player.state().0, 0., "clamped at the start");
    keys(vcx, "space");
    assert!(!player.state().1, "Space pauses");
}

#[gpui_kit::test]
fn the_transport_keys_are_scoped_to_the_drawer(cx: &mut TestAppContext) {
    let (view, vcx, _, player) = open(cx, map("completed"));
    // Typing a name in the repair field is not a seek.
    view.update_in(vcx, |d, window, cx| d.open_repair(0, window, cx));
    keys(vcx, "right space");
    assert_eq!(player.state(), (0., false));
    // Focus elsewhere (the Timeline's stage): the drawer's keys don't fire.
    vcx.update(|window, cx| window.blur(cx));
    keys(vcx, "right space");
    assert_eq!(player.state(), (0., false));
}

#[gpui_kit::test]
fn escape_closes_repair_then_follow_then_the_drawer(cx: &mut TestAppContext) {
    let (view, vcx, _, _) = open(cx, map("completed"));
    let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let c = closed.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&view, move |_, e, _| {
            if matches!(e, super::DrawerEvent::Close) {
                c.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        })
        .detach()
    });
    view.update_in(vcx, |d, window, cx| {
        d.open_repair(1, window, cx);
        d.follow_detached = true;
        window.focus(&d.focus, cx);
    });
    keys(vcx, "escape");
    view.update(vcx, |d, _| assert!(d.repair.is_none() && d.follow_detached));
    keys(vcx, "escape");
    view.update(vcx, |d, _| assert!(!d.follow_detached));
    assert!(!closed.load(std::sync::atomic::Ordering::SeqCst));
    keys(vcx, "escape");
    assert!(closed.load(std::sync::atomic::Ordering::SeqCst));
}

#[gpui_kit::test]
fn a_running_job_is_polled_every_1000ms(cx: &mut TestAppContext) {
    let (view, vcx, backend, _) = open(cx, map("running"));
    let loads = |b: &FixtureAudio| *b.loads.lock().unwrap();
    assert_eq!(loads(&backend), 1);
    view.update(vcx, |d, _| {
        assert_eq!(d.data.status, super::view_model::TranscriptStatus::Running)
    });
    vcx.executor().advance_clock(POLL - MS);
    vcx.run_until_parked();
    assert_eq!(loads(&backend), 1, "999 ms");
    vcx.executor().advance_clock(MS);
    vcx.run_until_parked();
    assert_eq!(loads(&backend), 2, "1000 ms");
}

#[gpui_kit::test]
fn a_finished_job_is_not_polled(cx: &mut TestAppContext) {
    let (_, vcx, backend, _) = open(cx, map("completed"));
    vcx.executor().advance_clock(POLL * 3);
    vcx.run_until_parked();
    assert_eq!(*backend.loads.lock().unwrap(), 1);
}

#[test]
fn samples_play_8s_each_then_stop() {
    let player = FakePlayer::new(DURATION);
    let mut t = Transport::default();
    t.set_player(Box::new(player.clone()));
    t.play_samples(&[1000, 20_000]);
    assert_eq!(player.state(), (1., true));
    let at = |s: f64| player.clone().seek(s);
    at(1. + SAMPLE_SECONDS - 0.001);
    t.tick();
    assert_eq!(
        player.state().0,
        1. + SAMPLE_SECONDS - 0.001,
        "7.999 s: still the first"
    );
    at(1. + SAMPLE_SECONDS);
    t.tick();
    assert_eq!(
        player.state(),
        (20., true),
        "8 s: the merge candidate's head"
    );
    at(20. + SAMPLE_SECONDS);
    t.tick();
    assert!(!player.state().1, "the last sample ends playback");
    // Any explicit seek ends sample mode.
    t.play_samples(&[1000, 20_000]);
    t.seek(Seek::By(5.));
    at(30.);
    t.tick();
    assert_eq!(player.state(), (30., true));
}

#[gpui_kit::test]
fn naming_a_voice_writes_once_and_closes_the_panel(cx: &mut TestAppContext) {
    let (view, vcx, backend, _) = open(cx, map("completed"));
    view.update_in(vcx, |d, window, cx| d.open_repair(0, window, cx));
    vcx.simulate_input("Daniel Park");
    vcx.run_until_parked();
    view.update_in(vcx, |d, _, cx| {
        let (_, can_create, pick) = super::repair::picks(d, "Daniel Park");
        assert!(can_create && pick == Some(super::repair::Pick::New));
        d.write(
            7,
            Write::NameAndLink {
                cluster: 7,
                name: "Daniel Park".into(),
            },
            true,
            cx,
        );
        // One write at a time: a second press while busy does nothing.
        d.write(7, Write::Confirm { cluster: 7 }, false, cx);
    });
    vcx.run_until_parked();
    assert_eq!(
        backend.writes(),
        vec![Write::NameAndLink {
            cluster: 7,
            name: "Daniel Park".into()
        }]
    );
    view.update(vcx, |d, _| {
        assert!(d.repair.is_none() && d.busy_cluster.is_none())
    });
}

#[gpui_kit::test]
fn a_failed_write_keeps_the_panel_open_with_the_error(cx: &mut TestAppContext) {
    let (view, vcx, backend, _) = open(cx, map("completed"));
    *backend.write_error.lock().unwrap() = Some("database is locked".into());
    view.update_in(vcx, |d, window, cx| {
        d.open_repair(0, window, cx);
        d.write(
            7,
            Write::Link {
                cluster: 7,
                person: 3,
            },
            true,
            cx,
        );
    });
    vcx.run_until_parked();
    view.update(vcx, |d, _| {
        assert!(d.repair.is_some());
        assert_eq!(d.correction_error.as_deref(), Some("database is locked"));
    });
}
