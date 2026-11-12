use super::*;

#[cfg(target_os = "macos")]
#[test]
fn stop_capture_runtime_accepts_idle_recording_boundary() {
    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        user_capture_paused: true,
        runtime_controller: RuntimeController::default(),
        runtime_state: RuntimeState::Idle,
        ..Default::default()
    };

    stop_capture_runtime(&mut runtime, None)
        .expect("idle recording boundary should not request another stop transition");

    assert_eq!(runtime.runtime_state, RuntimeState::Idle);
}

// Lock pauses the microphone for inactivity, and a user pause/resume used to
// leave that standing: the resume's fresh file then missed every rotation.
#[cfg(target_os = "macos")]
#[test]
fn stop_capture_runtime_clears_inactivity_pauses_so_a_resume_rotates_the_microphone() {
    let sources = CaptureSources {
        screen: true,
        microphone: true,
        system_audio: false,
    };
    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(sources.clone()),
        runtime_controller: RuntimeController::default(),
        runtime_state: RuntimeState::Idle,
        inactivity: InactivityState {
            enabled: true,
            microphone_paused: true,
            is_paused: true,
            ..InactivityState::default()
        },
        ..Default::default()
    };

    stop_capture_runtime(&mut runtime, None).expect("idle stop should succeed");

    assert!(!runtime.inactivity.is_microphone_paused());
    let rotated = active_sources_for_inactivity_paused_state(
        &sources,
        runtime.inactivity.screen_paused,
        runtime.inactivity.microphone_paused,
        runtime.inactivity.system_audio_paused,
    )
    .expect("sources stay active");
    assert!(rotated.microphone);
}

#[cfg(target_os = "macos")]
#[test]
fn stop_capture_runtime_clears_the_microphone_away_hold() {
    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        runtime_controller: RuntimeController::default(),
        runtime_state: RuntimeState::Idle,
        microphone_away_hold: true,
        ..Default::default()
    };

    stop_capture_runtime(&mut runtime, None).expect("idle stop should succeed");

    assert!(!runtime.microphone_away_hold);
}

#[cfg(target_os = "macos")]
#[test]
fn stop_capture_runtime_commits_the_system_audio_segment_on_an_audio_only_stop() {
    // The shape a stop lands in during an audio-only session (or a stop while
    // the screen is asleep): no screen session at all, only the tap's `.m4a`
    // tracked in the current segment. The stop must persist that recording into
    // the committed output_files — the other stop test asserts nothing about
    // outputs, so a regression silently dropping audio-only segments would
    // still pass it.
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let sys_path = temp.path().join("segment-0001-sys.m4a");
    write_valid_m4a_audio_file(&sys_path);
    let sys = sys_path.to_string_lossy().into_owned();

    let mut current = crate::native_capture::segments::empty_output_files();
    crate::native_capture_output::set_current_system_audio_output_file(&mut current, sys.clone());

    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();
    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: false,
            microphone: false,
            system_audio: true,
        }),
        current_segment_index: 1,
        system_audio_recording_file: Some(sys.clone()),
        current_segment_output_files: Some(current),
        output_files: Some(crate::native_capture::segments::empty_output_files()),
        active_screen_session: None,
        runtime_controller,
        runtime_state,
        ..Default::default()
    };

    stop_capture_runtime(&mut runtime, None)
        .expect("an audio-only stop should succeed without a screen session");

    let committed = runtime
        .output_files
        .as_ref()
        .expect("committed output_files slot should exist");
    let committed_sys = committed
        .system_audio_file
        .iter()
        .chain(committed.system_audio_files.iter())
        .any(|file| file == &sys);
    assert!(
        committed_sys,
        "an audio-only stop must commit the current segment's finalized \
         system-audio `.m4a` into runtime.output_files (got {committed:?}); \
         otherwise the whole recording is silently lost on stop"
    );
    assert_eq!(
        runtime.runtime_state,
        RuntimeState::Idle,
        "the stop must run the runtime down to Idle"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn sleep_cleared_screen_state_is_eligible_for_possible_wake_recovery_resync() {
    let mut lifecycle = RecordingLifecycle::default();
    *lifecycle.runtime_mut() = running_screen_capture_runtime_fixture();

    assert!(!lifecycle.should_attempt_recovery_after_possible_wake());

    assert!(lifecycle.handle_system_will_sleep());

    assert!(lifecycle.should_attempt_recovery_after_possible_wake());
}

#[cfg(target_os = "macos")]
#[test]
fn paused_screen_state_is_not_eligible_for_possible_wake_recovery_resync() {
    let mut lifecycle = RecordingLifecycle::default();
    *lifecycle.runtime_mut() = screen_paused_runtime_fixture();

    assert!(!lifecycle.should_attempt_recovery_after_possible_wake());
}

#[cfg(target_os = "macos")]
#[test]
fn suspended_screen_state_is_not_eligible_for_possible_wake_recovery_resync() {
    let mut lifecycle = RecordingLifecycle::default();
    *lifecycle.runtime_mut() = running_screen_capture_runtime_fixture();

    // Sleep-clearing the screen state makes it eligible (the resync entry point).
    assert!(lifecycle.handle_system_will_sleep());
    assert!(lifecycle.should_attempt_recovery_after_possible_wake());

    // A live suspension owns the screen and re-arms through its own path; a
    // possible-wake resync must not fight it.
    let suspension_error = CaptureErrorResponse {
        code: "capture_low_disk".to_string(),
        message: "disk full".to_string(),
    };
    lifecycle.runtime_mut().capture_suspension = Some(CaptureSuspension::with_kind(
        CaptureSuspensionKind::LowDisk,
        &suspension_error,
    ));
    assert!(!lifecycle.should_attempt_recovery_after_possible_wake());
}

#[cfg(target_os = "macos")]
#[test]
fn user_paused_screen_state_is_not_eligible_for_possible_wake_recovery_resync() {
    let mut lifecycle = RecordingLifecycle::default();
    *lifecycle.runtime_mut() = running_screen_capture_runtime_fixture();

    assert!(lifecycle.handle_system_will_sleep());
    assert!(lifecycle.should_attempt_recovery_after_possible_wake());

    // A manual pause must survive a possible-wake resync.
    lifecycle.runtime_mut().user_capture_paused = true;
    assert!(!lifecycle.should_attempt_recovery_after_possible_wake());
}

#[cfg(target_os = "macos")]
#[test]
fn session_from_runtime_reports_not_running_when_screen_capture_is_broken_after_wake_failure() {
    let mut runtime = running_screen_capture_runtime_fixture();

    recover_screen_capture_after_wake_with_start_segment(
        &mut runtime,
        None,
        |_, _, _, _, _, _, _, _, _| {
            Err(CaptureErrorResponse {
                code: "capture_stream_start_failed".to_string(),
                message: "wake restart failed".to_string(),
            })
        },
    )
    .expect_err("wake recovery failure should bubble");

    let session = crate::native_capture::runtime::session_from_runtime(&runtime);

    assert!(!session.is_running);
    assert_eq!(
        session.requested_sources,
        Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        })
    );
    // The screen is broken; both audio families are untouched by that and keep
    // recording (ADR 0052).
    assert_eq!(
        runtime.current_segment_sources,
        Some(CaptureSources {
            screen: false,
            microphone: true,
            system_audio: true,
        })
    );
}

#[cfg(target_os = "macos")]
#[test]
fn close_frame_batches_for_stopped_screen_session_id_closes_stale_open_batch() {
    run_async_test(async {
        let dir = TestDir::new("close-frame-batches-stop");
        let infra = Arc::new(
            ::app_infra::AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize"),
        );
        let session_id = "native-session-stop-close";

        insert_open_frame_batch_fixture(
            infra.as_ref(),
            session_id,
            "/tmp/native-session-stop-close-segment-0001/frames/frame-1.png",
        )
        .await;

        crate::native_capture::segments::close_frame_batches_for_stopped_screen_session_id_async(
            &infra, session_id,
        )
        .await
        .expect("stop cleanup should close frame batches");

        assert_frame_batches_closed(infra.as_ref(), session_id).await;
    });
}

#[cfg(target_os = "macos")]
#[test]
fn close_frame_batches_for_stopped_screen_session_id_closes_inside_runtime() {
    // Stands in for the core runtime the closing future runs on.
    let core_runtime = tokio::runtime::Runtime::new().expect("core runtime should build");
    let core_handle = core_runtime.handle().clone();
    run_async_test(async {
        let dir = TestDir::new("close-frame-batches-stop-inside-runtime");
        let infra = Arc::new(
            ::app_infra::AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize"),
        );
        let session_id = "native-session-stop-close-inside-runtime";

        insert_open_frame_batch_fixture(
            infra.as_ref(),
            session_id,
            "/tmp/native-session-stop-close-inside-runtime-segment-0001/frames/frame-1.png",
        )
        .await;

        crate::native_capture::segments::close_frame_batches_for_stopped_screen_session_id(
            core_handle,
            &infra,
            session_id,
        )
        .expect("sync stop cleanup should close frame batches inside an existing runtime");

        assert_frame_batches_closed(infra.as_ref(), session_id).await;
    });
}
