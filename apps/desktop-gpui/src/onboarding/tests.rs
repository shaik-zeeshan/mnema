//! Onboarding behaviour: the step flow, the two hard gates (and the
//! permissions relaunch hold), close-before-done exits, Setup never blocking,
//! the 15 s voice take at 14 999 / 15 000 ms, and the Finale's capture start.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use capture_types::{
    AppearanceSetting, CapturePermissionState as P, RecordingSettings, RetentionPolicy,
};
use gpui_kit::{Entity, TestAppContext, VisualTestContext, WindowAppearance};
use serde_json::json;

use super::data::{Backend, Enrollment};
use super::fixture::FixtureBackend;
use super::flow::{self, Feature, Features, Intents, Step};
use super::storage::{self, Probe, ProbeState};
use super::voice::{self, Phase, Take};
use super::{FinalePhase, Moment, Onboarding};
use crate::theme;

const MS: Duration = Duration::from_millis(1);

fn init(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
    });
}

fn open<'a>(
    cx: &'a mut TestAppContext,
    file: &str,
) -> (
    Entity<Onboarding>,
    &'a mut VisualTestContext,
    Arc<FixtureBackend>,
    Rc<Cell<u32>>,
) {
    init(cx);
    let backend = Arc::new(FixtureBackend::load(file).unwrap());
    let b: Arc<dyn Backend> = backend.clone();
    let exits = Rc::new(Cell::new(0));
    let e = exits.clone();
    let (view, vcx) = cx.add_window_view(move |_, cx| {
        Onboarding::new(b.clone(), move |_, _| e.set(e.get() + 1), cx)
    });
    vcx.run_until_parked();
    (view, vcx, backend, exits)
}

fn go(view: &Entity<Onboarding>, vcx: &mut VisualTestContext, step: Step) {
    view.update(vcx, |o, cx| o.go(step, cx));
    vcx.run_until_parked();
}

#[test]
fn the_step_table_walks_eight_steps_with_one_round_trip() {
    let mut s = Step::Welcome;
    let mut path = vec![s];
    while let Some(n) = s.next() {
        s = n;
        path.push(s);
    }
    use Step::*;
    assert_eq!(
        path,
        vec![
            Welcome,
            Permissions,
            Capture,
            YourSettings,
            Setup,
            Voice,
            Finale
        ]
    );
    assert_eq!(
        Change.next(),
        Some(YourSettings),
        "Change settings is a round trip"
    );
    assert_eq!(Change.back(), Some(YourSettings));
    assert_eq!(
        Setup.back(),
        Some(YourSettings),
        "Setup steps back over the detour"
    );
    assert_eq!(Step::ALL.map(Step::position), [1, 2, 3, 4, 5, 6, 7, 8]);
    assert!(
        Change.suffix().is_some() && Voice.suffix().is_some(),
        "the two detours draw short ticks"
    );
    for step in Step::ALL {
        assert_eq!(Step::from_wire(step.wire()), Some(step));
    }
}

#[test]
fn only_capture_and_storage_can_hold_continue() {
    for step in Step::ALL {
        assert!(flow::can_continue(step, None));
        let held = !flow::can_continue(step, Some("blocked"));
        assert_eq!(held, step == Step::Capture, "{step:?}");
    }
}

#[test]
fn the_storage_gates_block_on_a_bad_folder_or_a_measured_shortfall_only() {
    let px = storage::ANCHOR_VIDEO_PIXELS;
    let probe = |exists, writable, free: Option<f64>| Probe {
        path: "/Users/ana/.mnema".into(),
        exists,
        writable,
        free,
    };
    assert!(
        storage::block_reason(Some(&probe(false, false, Some(1e12))), 0., 2., px)
            .unwrap()
            .contains("doesn't exist")
    );
    assert!(
        storage::block_reason(Some(&probe(true, false, Some(1e12))), 0., 2., px)
            .unwrap()
            .contains("not writable")
    );
    // An inability to measure never blocks (ADR 0040).
    assert_eq!(storage::block_reason(None, 0., 2., px), None);
    assert_eq!(
        storage::block_reason(Some(&probe(true, true, None)), 0., 2., px),
        None
    );
    // Gate 2 is reserve + downloads + one day, at the exact boundary.
    let need = storage::need_bytes(1e9, 2., px);
    assert_eq!(
        storage::block_reason(Some(&probe(true, true, Some(need))), 1e9, 2., px),
        None
    );
    let short =
        storage::block_reason(Some(&probe(true, true, Some(need - 1.))), 1e9, 2., px).unwrap();
    assert!(
        short.starts_with("Not enough room to record a day"),
        "{short}"
    );
    let models = storage::block_reason(Some(&probe(true, true, Some(1.5e9))), 1e9, 2., px).unwrap();
    assert!(
        models.starts_with("Not enough room for the downloads"),
        "{models}"
    );
}

#[test]
fn the_daily_estimate_is_the_measured_anchor() {
    let px = storage::ANCHOR_VIDEO_PIXELS;
    assert_eq!(storage::daily_mb(3., px), 270.);
    assert_eq!(storage::daily_mb(2., px), 405.);
    assert_eq!(storage::nearest_ladder_index(0.5), 3, "0.5 fps = every 2 s");
    assert_eq!(
        storage::nearest_ladder_index(7.5),
        0,
        "a legacy rate still lands on the ladder"
    );
    assert_eq!(storage::interval_phrase(60.), "once per minute");
    assert_eq!(storage::volume_label("/Volumes/Tiny/Mnema"), "Tiny");
    assert_eq!(storage::sentence_path("/Users/ana/.mnema", 28), "~/.mnema");
    let v = storage::verdict(&storage::Input {
        interval_s: 2.,
        pixels: px,
        retention: &RetentionPolicy::Never,
        probe: Some(&Probe {
            path: "/Volumes/Tiny/Mnema".into(),
            exists: true,
            writable: true,
            free: Some(1.9e9),
        }),
        state: ProbeState::Done,
        downloads: 1.1e9,
        semantic_on: true,
    });
    assert!(v.blocking);
    assert_eq!(
        v.clause.as_deref(),
        Some("there isn't room for the models yet.")
    );
    assert_eq!(v.repairs[0].1, storage::Repair::NoSemantic);
}

#[test]
fn toggles_cascade_both_ways_and_locks_refuse() {
    let perms = Intents {
        screen: true,
        microphone: true,
        system_audio: false,
    };
    let f = flow::resolve(perms, &flow::Saved::default());
    assert!(f.screen && f.microphone && f.system_audio && f.transcription && f.speaker_separation);
    assert!(!f.ai_features, "AI is never pre-ticked");
    let off = f
        .toggled(Feature::Microphone)
        .unwrap()
        .toggled(Feature::SystemAudio)
        .unwrap();
    assert!(
        !off.transcription && !off.speaker_separation,
        "the last audio source takes processing with it"
    );
    assert_eq!(
        off.toggled(Feature::Transcription),
        None,
        "a locked enable is refused"
    );
    let back = off.toggled(Feature::Microphone).unwrap();
    assert!(
        back.transcription && back.speaker_separation,
        "an audio source back on re-enables processing"
    );
    let no_mic = Features {
        perms: Intents::default(),
        ..Features::default()
    };
    let fix = no_mic.lock_fix(Feature::Microphone).unwrap();
    assert_eq!(fix.0, flow::LockFix::Grant);
}

#[test]
fn the_relaunch_is_the_only_permissions_hold() {
    assert_eq!(
        flow::screen_gate(P::Denied, false),
        (true, "Continue without screen recording")
    );
    assert_eq!(
        flow::screen_gate(P::NotDetermined, false),
        (true, "Continue")
    );
    assert_eq!(
        flow::screen_gate(P::Granted, true),
        (false, "Relaunch to continue")
    );
    assert!(
        flow::is_granted(P::Unsupported),
        "an unreportable grant must pass, not trap"
    );
}

#[test]
fn the_voice_take_ends_at_exactly_fifteen_seconds() {
    let t0 = std::time::Instant::now();
    let mut take = Take::default();
    assert!(take.start(t0));
    assert!(!take.start(t0), "one take at a time");
    assert!(
        !take.tick(t0 + voice::CLIP - MS),
        "still recording at 14 999 ms"
    );
    assert!(matches!(take.phase, Phase::Recording { .. }));
    assert_eq!(take.line(), "0:15 of about 0:15");
    assert!(take.tick(t0 + voice::CLIP), "the take ends at 15 000 ms");
    assert_eq!(take.phase, Phase::Judging);
    take.judged(Ok(Enrollment::TooShort { duration_ms: 6000 }));
    assert_eq!(
        voice::rejection(&Enrollment::TooShort { duration_ms: 6000 }).0,
        "Too short to learn from"
    );
    assert!(take.start(t0), "a rejection can be retried");
}

#[gpui_kit::test]
fn the_flow_advances_and_capture_holds_on_a_tiny_volume(cx: &mut TestAppContext) {
    let (view, vcx, _, _) = open(cx, "tiny");
    view.update(vcx, |o, cx| {
        assert!(o.loaded);
        assert_eq!(o.step, Step::Welcome);
        o.next(cx);
        assert_eq!(o.step, Step::Permissions);
        o.next(cx);
        assert_eq!(o.step, Step::Capture);
    });
    vcx.run_until_parked();
    view.update(vcx, |o, cx| {
        assert_eq!(o.probe_state, ProbeState::Done);
        assert!(
            o.capture_block().is_some(),
            "1.9 GB can't hold the downloads + a day"
        );
        o.next(cx);
        assert_eq!(o.step, Step::Capture, "held: Continue does nothing");
        // Gate 2 satisfied: drop the downloads that don't fit and record less often.
        o.toggle(Feature::SemanticSearch, cx);
        o.toggle(Feature::SpeakerSeparation, cx);
        o.toggle(Feature::Transcription, cx);
        o.set_rate(10, cx);
        assert_eq!(o.capture_block(), None);
        o.next(cx);
        assert_eq!(o.step, Step::YourSettings);
    });
}

#[gpui_kit::test]
fn a_read_only_folder_holds_until_another_is_chosen(cx: &mut TestAppContext) {
    let (view, vcx, _, _) = open(cx, "granted");
    go(&view, vcx, Step::Capture);
    view.update(vcx, |o, cx| {
        o.probe = Some(Probe {
            path: "/Volumes/Archive/Mnema".into(),
            exists: true,
            writable: false,
            free: Some(1.2e12),
        });
        assert!(o.capture_block().unwrap().contains("not writable"));
        o.next(cx);
        assert_eq!(o.step, Step::Capture);
        o.set_folder(String::new(), cx);
    });
    vcx.run_until_parked();
    view.update(vcx, |o, cx| {
        assert_eq!(
            o.capture_block(),
            None,
            "the default folder probes writable"
        );
        o.next(cx);
        assert_eq!(o.step, Step::YourSettings);
    });
}

#[gpui_kit::test]
fn permissions_moments_advance_one_at_a_time_and_a_denial_never_gates(cx: &mut TestAppContext) {
    let (view, vcx, backend, _) = open(cx, "denied");
    go(&view, vcx, Step::Permissions);
    view.update(vcx, |o, cx| {
        assert_eq!(o.current_moment(), Some(Moment::Screen));
        o.advance_permissions(cx);
        assert_eq!(
            o.current_moment(),
            Some(Moment::Microphone),
            "a screen denial continues"
        );
        o.request(Moment::Microphone, cx);
    });
    vcx.run_until_parked();
    assert!(backend
        .calls()
        .contains(&"request_permission:microphone".to_string()));
    view.update(vcx, |o, cx| {
        o.advance_permissions(cx);
        assert_eq!(o.current_moment(), Some(Moment::SystemAudio));
        o.advance_permissions(cx);
        assert_eq!(
            o.current_moment(),
            Some(Moment::Accessibility),
            "Zen is installed: offered last"
        );
        o.advance_permissions(cx);
        assert_eq!(o.step, Step::Capture);
    });
}

#[gpui_kit::test]
fn a_same_process_screen_grant_holds_until_relaunch(cx: &mut TestAppContext) {
    let (view, vcx, backend, _) = open(cx, "default");
    go(&view, vcx, Step::Permissions);
    view.update(vcx, |o, cx| {
        o.screen_needs_relaunch = true;
        o.advance_permissions(cx);
        o.next(cx);
        assert_eq!(
            o.step,
            Step::Permissions,
            "Continue is Relaunch while the grant is stale"
        );
    });
    vcx.run_until_parked();
    assert!(backend.calls().contains(&"relaunch".to_string()));
}

#[gpui_kit::test]
fn closing_before_done_exits_and_after_done_only_closes(cx: &mut TestAppContext) {
    let (view, vcx, _, exits) = open(cx, "default");
    view.update_in(vcx, |o, window, cx| o.request_close(window, cx));
    assert_eq!(
        exits.get(),
        1,
        "closing before onboarding completes ends the app"
    );
    assert_eq!(flow::close_action(true), flow::CloseAction::Close);
    assert_eq!(flow::close_action(false), flow::CloseAction::Exit);
}

#[gpui_kit::test]
fn setup_never_blocks_continue(cx: &mut TestAppContext) {
    let (view, vcx, backend, _) = open(cx, "granted");
    go(&view, vcx, Step::Setup);
    assert!(
        backend
            .calls()
            .iter()
            .any(|c| c.starts_with("start_download:pyannote")),
        "speakrs first: {:?}",
        backend.calls()
    );
    view.update(vcx, |o, cx| {
        assert!(o.dl.in_flight.is_some(), "a download is in flight");
        assert!(o.can_continue(), "in flight is progress, not a problem");
        let event = json!({"provider": "speakrs", "modelId": "pyannote-community-1-wespeaker", "status": "failed", "downloadedBytes": 0, "message": "offline"});
        o.on_download_event(&event, cx);
        assert!(o.can_continue(), "a failure never disables Continue");
        o.next(cx);
        assert_eq!(o.step, Step::Voice);
    });
}

#[gpui_kit::test]
fn the_voice_screen_records_fifteen_seconds_then_enrolls(cx: &mut TestAppContext) {
    let (view, vcx, backend, _) = open(cx, "installed");
    go(&view, vcx, Step::Voice);
    view.update(vcx, |o, cx| {
        assert!(o.voice_ready(), "speakrs is on disk");
        o.record(cx);
    });
    vcx.executor().advance_clock(voice::CLIP - MS);
    vcx.run_until_parked();
    view.update(vcx, |o, _| {
        assert!(
            matches!(o.take.phase, Phase::Recording { .. }),
            "still recording at 14 999 ms"
        )
    });
    vcx.executor().advance_clock(voice::LEVEL_POLL);
    vcx.run_until_parked();
    view.update(vcx, |o, _| {
        assert_eq!(o.take.phase, Phase::Enrolled, "past 15 000 ms: judged")
    });
    assert!(backend.calls().contains(&"record_clip:15000".to_string()));
}

#[gpui_kit::test]
fn the_finale_saves_completes_and_starts_capture(cx: &mut TestAppContext) {
    let (view, vcx, backend, exits) = open(cx, "finale");
    go(&view, vcx, Step::Finale);
    view.update(vcx, |o, _| {
        assert_eq!(o.finale, FinalePhase::Running);
        assert!(o.completed);
    });
    let finish: Vec<String> = backend
        .calls()
        .into_iter()
        .filter(|c| c.starts_with("finish"))
        .collect();
    assert_eq!(
        finish,
        vec!["finish:fps=0.5:start=screen=true,mic=true,sys=true".to_string()],
        "one atomic commit"
    );
    vcx.executor().advance_clock(super::actions::EVIDENCE_POLL);
    vcx.run_until_parked();
    view.update(vcx, |o, _| {
        let ev = o.evidence.as_ref().expect("first frame");
        assert!(ev.ocr.as_deref().unwrap().starts_with("Quarterly review"));
    });
    view.update_in(vcx, |o, window, cx| o.request_close(window, cx));
    assert_eq!(exits.get(), 0, "after completion, closing doesn't exit");
}

#[gpui_kit::test]
fn nothing_granted_finishes_idle_without_starting_capture(cx: &mut TestAppContext) {
    let (view, vcx, backend, _) = open(cx, "default");
    view.update(vcx, |o, cx| {
        o.features.system_audio = false;
        o.go(Step::Finale, cx);
    });
    vcx.run_until_parked();
    view.update(vcx, |o, _| assert_eq!(o.finale, FinalePhase::Idle));
    assert!(
        backend.calls().iter().any(|c| c.ends_with("start=none")),
        "{:?}",
        backend.calls()
    );
}

#[test]
fn every_fixture_parses_into_real_settings() {
    for file in [
        "default",
        "granted",
        "denied",
        "tiny",
        "installed",
        "finale",
    ] {
        let map = crate::shell::fixture::load("onboarding", file).unwrap();
        let s: Result<RecordingSettings, _> =
            serde_json::from_value(map["get_recording_settings"].clone());
        assert!(s.is_ok(), "{file}: {:?}", s.err());
    }
}
