//! The segment machinery's host side on a `FakeHost`: the paths the desktop
//! tests could only run with `None` (no `AppHandle`).

use super::*;
use crate::native_capture::app_notifications::AppNotificationsState;
use crate::native_capture::lifecycle::NativeCaptureState;
use crate::test_support::{core_in, FakeHost};
use crate::{CoreEvent, DialogKind, MessageDialog};
use std::time::{Duration, Instant};

fn fake_core(dir: &Path) -> (Arc<FakeHost>, Arc<crate::AppCore>) {
    let host = Arc::new(FakeHost::default());
    let core = Arc::new(core_in(host.clone(), dir));
    (host, core)
}

fn running_screen_runtime() -> NativeCaptureRuntime {
    NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: false,
        }),
        output_files: Some(crate::native_capture::segments::empty_output_files()),
        ..Default::default()
    }
}

/// The bell notification ids an `AppNotificationsChanged` payload lists.
fn notification_ids(event: &CoreEvent) -> Option<Vec<String>> {
    let CoreEvent::AppNotificationsChanged(payload) = event else {
        return None;
    };
    Some(
        payload
            .as_array()?
            .iter()
            .filter_map(|n| n["id"].as_str().map(str::to_string))
            .collect(),
    )
}

#[test]
fn a_license_block_stop_shows_the_error_dialog_and_keeps_the_reason_in_the_bell() {
    let dir = TestDir::new("license-block-stop-host");
    let (host, core) = fake_core(dir.path());
    core.manage(AppNotificationsState::default());

    let mut runtime = running_screen_runtime();
    crate::native_capture::segments::graceful_stop_for_license_block(
        Some(&core),
        &mut runtime,
        false,
    );
    let mut revoked = running_screen_runtime();
    crate::native_capture::segments::graceful_stop_for_license_block(
        Some(&core),
        &mut revoked,
        true,
    );

    assert!(!runtime.is_running && !revoked.is_running);
    assert_eq!(
        *host.messages.lock().unwrap(),
        vec![
            MessageDialog {
                title: "Recording stopped — trial ended".to_string(),
                message: "Your trial has ended. Buy a license to resume recording — everything you already recorded stays browsable and searchable.".to_string(),
                kind: DialogKind::Error,
            },
            MessageDialog {
                title: "Recording stopped — license revoked".to_string(),
                message: "This license has been revoked. Contact support if you think this is a mistake — everything you already recorded stays browsable and searchable.".to_string(),
                kind: DialogKind::Error,
            },
        ]
    );
    let lists: Vec<Vec<String>> = host
        .events
        .lock()
        .unwrap()
        .iter()
        .filter_map(notification_ids)
        .collect();
    assert_eq!(
        lists.last().unwrap(),
        &vec![
            "capture_trial_ended_stopped".to_string(),
            "capture_license_revoked_stopped".to_string()
        ]
    );
}

#[test]
fn a_license_block_stop_without_a_core_shows_nothing() {
    let mut runtime = running_screen_runtime();
    crate::native_capture::segments::graceful_stop_for_license_block(None, &mut runtime, false);
    assert!(!runtime.is_running);
}

#[test]
fn persisting_committed_audio_segments_raises_audio_segments_changed() {
    let dir = TestDir::new("persist-audio-segments-host");
    let (host, core) = fake_core(dir.path());
    let infra = core
        .block_on(::app_infra::AppInfra::initialize(&dir.path().join("infra")))
        .expect("app infra should initialize");
    core.manage::<crate::AppInfraState>(Arc::new(infra));
    core.manage(crate::recording_settings::RecordingSettingsState::default());
    let microphone_file =
        write_existing_audio_placeholder(&dir.path().join("segment-microphone.m4a"));
    let output_files = CaptureOutputFiles {
        microphone_file: Some(microphone_file.clone()),
        microphone_files: vec![microphone_file],
        ..crate::native_capture::segments::empty_output_files()
    };

    crate::native_capture::segments::persist_committed_audio_segments(
        Some(&core),
        Some(&independent_source_sessions_fixture()),
        Some(&SegmentSchedule::new(Duration::from_secs(60))),
        1,
        Some(&output_files),
    );

    assert_eq!(
        *host.events.lock().unwrap(),
        vec![CoreEvent::AudioSegmentsChanged]
    );
}

#[test]
fn persisting_no_committed_audio_segments_raises_nothing() {
    let dir = TestDir::new("persist-no-audio-segments-host");
    let (host, core) = fake_core(dir.path());

    crate::native_capture::segments::persist_committed_audio_segments(
        Some(&core),
        Some(&independent_source_sessions_fixture()),
        Some(&SegmentSchedule::new(Duration::from_secs(60))),
        1,
        Some(&crate::native_capture::segments::empty_output_files()),
    );

    assert!(host.events.lock().unwrap().is_empty());
}

#[test]
fn a_segment_loop_whose_session_ended_on_its_own_announces_it_and_refreshes_the_tray() {
    let dir = TestDir::new("segment-loop-ended-host");
    let (host, core) = fake_core(dir.path());
    let state = NativeCaptureState::default();
    core.manage(state.clone());

    // A lifecycle that is not running: the loop's first tick falls out, as it
    // does after an internal end (a fatal failure, a lost source).
    let control = crate::native_capture::segments::spawn_segment_loop(core.clone());

    let deadline = Instant::now() + Duration::from_secs(10);
    while host.events.lock().unwrap().is_empty() {
        assert!(
            Instant::now() < deadline,
            "the loop never announced the end"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let session = state.lock().unwrap().session();
    assert_eq!(
        *host.events.lock().unwrap(),
        vec![CoreEvent::NativeCaptureSessionChanged(
            serde_json::to_value(&session).unwrap()
        )]
    );
    while host
        .tray_refreshes
        .load(std::sync::atomic::Ordering::SeqCst)
        == 0
    {
        assert!(
            Instant::now() < deadline,
            "the loop never refreshed the tray"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(control);
}

#[test]
fn a_stopped_segment_loop_leaves_the_announcement_to_its_stopper() {
    let dir = TestDir::new("segment-loop-stopped-host");
    let (host, core) = fake_core(dir.path());
    let state = NativeCaptureState::default();
    // Running with no schedule: the loop parks on the lock until we stop it.
    state.lock().unwrap().runtime_mut().is_running = true;
    let held = state.lock().unwrap();
    core.manage(state.clone());

    let control = crate::native_capture::segments::spawn_segment_loop(core.clone());
    control
        .stop
        .store(true, std::sync::atomic::Ordering::Relaxed);
    drop(held);

    std::thread::sleep(Duration::from_millis(200));
    assert!(host.events.lock().unwrap().is_empty());
    assert_eq!(
        host.tray_refreshes
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}
