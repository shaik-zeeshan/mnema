use super::*;

#[cfg(target_os = "macos")]
#[test]
fn inactivity_resume_no_longer_requires_planner_for_legacy_soft_resume() {
    let mut runtime = paused_runtime_fixture();
    runtime.segment_planner = None;

    resume_runtime_from_inactivity(&mut runtime)
        .expect("legacy soft-resume should succeed without planner");

    assert!(runtime.is_running);
    assert_eq!(runtime.runtime_state, RuntimeState::Running);
    assert!(!runtime.inactivity.is_paused);
}

#[cfg(target_os = "macos")]
#[test]
fn inactivity_resume_sets_current_segment_sources_from_requested() {
    let mut runtime = paused_runtime_fixture();
    runtime.segment_planner = Some(SegmentPlanner::with_date_prefix(
        "/tmp/native-capture-tests",
        "native-session-resume",
        "2026/04/19",
    ));
    runtime.microphone_planner = Some(SegmentPlanner::with_date_prefix(
        "/tmp/native-capture-tests",
        "native-session-resume-mic",
        "2026/04/19",
    ));
    runtime.system_audio_planner = Some(SegmentPlanner::with_date_prefix(
        "/tmp/native-capture-tests",
        "native-session-resume-system",
        "2026/04/19",
    ));
    assert!(runtime.current_segment_sources.is_none());

    resume_runtime_from_inactivity(&mut runtime).expect("resume should succeed");

    assert_eq!(
        runtime.current_segment_sources,
        Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: false,
        })
    );
    assert_eq!(runtime.current_segment_sources, runtime.requested_sources,);
}

#[cfg(target_os = "macos")]
#[test]
fn inactivity_resume_with_paused_audio_refreshes_sources_without_planning_system_audio() {
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        }),
        current_segment_index: 1,
        screen_frame_rate: 5.0,
        screen_resolution: ScreenResolution::default(),
        segment_loop_control: None,
        capture_clock: Some(CaptureClock::start_now()),
        segment_schedule: Some(SegmentSchedule::new(std::time::Duration::from_secs(60))),
        segment_planner: Some(SegmentPlanner::with_date_prefix(
            "/tmp/native-capture-tests",
            "native-session-resume-refresh",
            "2026/04/22",
        )),
        microphone_planner: Some(SegmentPlanner::with_date_prefix(
            "/tmp/native-capture-tests",
            "native-session-resume-refresh-mic",
            "2026/04/22",
        )),
        system_audio_planner: None,
        source_sessions: None,
        runtime_controller,
        runtime_state,
        inactivity: InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            system_audio_paused: true,
            is_paused: true,
            ..InactivityState::default()
        },
        ..Default::default()
    };

    resume_runtime_from_inactivity(&mut runtime)
        .expect("paused-audio refresh should be a tolerant no-op");

    assert!(runtime.system_audio_planner.is_none());
    assert!(runtime.inactivity.is_paused);
    assert!(runtime.inactivity.is_system_audio_paused());
    assert_eq!(
        runtime.current_segment_sources,
        Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: false,
        })
    );
}

#[cfg(target_os = "macos")]
#[test]
fn inactivity_resume_does_not_restart_or_plan_dedicated_outputs_for_legacy_soft_resume() {
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        }),
        current_segment_index: 1,
        screen_frame_rate: 5.0,
        screen_resolution: ScreenResolution::default(),
        segment_loop_control: None,
        capture_clock: Some(CaptureClock::start_now()),
        segment_schedule: Some(SegmentSchedule::new(std::time::Duration::from_secs(60))),
        segment_planner: Some(SegmentPlanner::with_date_prefix(
            "/tmp/native-capture-tests",
            "native-session-resume-screen",
            "2026/04/22",
        )),
        microphone_planner: Some(SegmentPlanner::with_date_prefix(
            "/tmp/native-capture-tests",
            "native-session-resume-mic",
            "2026/04/22",
        )),
        system_audio_planner: Some(SegmentPlanner::with_date_prefix(
            "/tmp/native-capture-tests",
            "native-session-resume-system-audio",
            "2026/04/22",
        )),
        runtime_controller,
        runtime_state,
        inactivity: InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            is_paused: true,
            ..InactivityState::default()
        },
        ..Default::default()
    };

    resume_runtime_from_inactivity(&mut runtime)
        .expect("legacy soft-resume should succeed without dedicated planning");

    assert_eq!(
        runtime.current_segment_sources,
        Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        })
    );
    assert!(runtime.microphone_recording_file.is_none());
    assert!(runtime.system_audio_recording_file.is_none());
}

#[cfg(target_os = "macos")]
#[test]
fn paused_runtime_fixture_has_no_current_segment_sources() {
    let runtime = paused_runtime_fixture();

    assert!(runtime.current_segment_sources.is_none());
    assert!(current_segment_sources_for_runtime(&runtime).is_none());
}

#[cfg(target_os = "macos")]
#[test]
fn pause_microphone_for_inactivity_sets_microphone_paused_preserves_screen() {
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: false,
        }),
        current_segment_index: 1,
        screen_frame_rate: 5.0,
        screen_resolution: ScreenResolution::default(),
        current_segment_output_files: Some(CaptureOutputFiles {
            screen_file: Some("/tmp/screen.mov".to_string()),
            screen_files: vec!["/tmp/screen.mov".to_string()],
            microphone_file: None,
            microphone_files: Vec::new(),
            system_audio_file: None,
            system_audio_files: Vec::new(),
        }),
        recording_file: Some("/tmp/screen.mov".to_string()),
        active_screen_session: None,
        active_microphone_session: None,
        active_system_audio_session: None,
        runtime_controller,
        runtime_state,
        inactivity: InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            ..InactivityState::default()
        },
        ..Default::default()
    };

    pause_microphone_for_inactivity(&mut runtime).expect("microphone pause should succeed");

    assert!(runtime.inactivity.is_microphone_paused());
    assert!(!runtime.inactivity.is_screen_paused());
    assert!(runtime.inactivity.is_paused);
    // Screen segment state should remain intact
    assert!(runtime.current_segment_output_files.is_some());
    assert!(runtime.recording_file.is_some());
}

#[cfg(target_os = "macos")]
#[test]
fn pause_microphone_for_inactivity_clears_backend_truth_and_current_output() {
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: false,
        }),
        current_segment_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: false,
        }),
        current_segment_index: 1,
        screen_frame_rate: 5.0,
        screen_resolution: ScreenResolution::default(),
        current_segment_output_files: Some(CaptureOutputFiles {
            screen_file: Some("/tmp/screen.mov".to_string()),
            screen_files: vec!["/tmp/screen.mov".to_string()],
            microphone_file: Some("/tmp/microphone.m4a".to_string()),
            microphone_files: vec!["/tmp/microphone.m4a".to_string()],
            system_audio_file: None,
            system_audio_files: Vec::new(),
        }),
        recording_file: Some("/tmp/screen.mov".to_string()),
        microphone_recording_file: Some("/tmp/microphone.m4a".to_string()),
        active_screen_session: None,
        active_microphone_session: None,
        active_system_audio_session: None,
        runtime_controller,
        runtime_state,
        inactivity: InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            ..InactivityState::default()
        },
        ..Default::default()
    };

    pause_microphone_for_inactivity(&mut runtime).expect("microphone pause should succeed");

    assert!(runtime.inactivity.is_microphone_paused());
    assert!(!microphone_backend_active_for_runtime(&runtime));
    assert!(runtime.active_microphone_session.is_none());
    assert!(runtime.microphone_recording_file.is_none());
    let outputs = runtime
        .current_segment_output_files
        .as_ref()
        .expect("screen outputs should stay present");
    assert!(outputs.microphone_file.is_none());
    assert!(outputs.microphone_files.is_empty());
    let sources = runtime
        .current_segment_sources
        .as_ref()
        .expect("screen source should remain active");
    assert!(sources.screen);
    assert!(!sources.microphone);
}

#[cfg(target_os = "macos")]
#[test]
fn live_audio_inactivity_pause_does_not_resume_microphone_without_threshold_activity() {
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: false,
        }),
        current_segment_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: false,
        }),
        current_segment_index: 1,
        screen_frame_rate: 5.0,
        screen_resolution: ScreenResolution::default(),
        current_segment_output_files: Some(CaptureOutputFiles {
            screen_file: Some("/tmp/screen.mov".to_string()),
            screen_files: vec!["/tmp/screen.mov".to_string()],
            microphone_file: Some("/tmp/microphone.m4a".to_string()),
            microphone_files: vec!["/tmp/microphone.m4a".to_string()],
            system_audio_file: None,
            system_audio_files: Vec::new(),
        }),
        recording_file: Some("/tmp/screen.mov".to_string()),
        microphone_recording_file: Some("/tmp/microphone.m4a".to_string()),
        active_screen_session: None,
        active_microphone_session: None,
        active_system_audio_session: None,
        runtime_controller,
        runtime_state,
        inactivity: InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            microphone_activity_sensitivity: 100,
            last_activity_monotonic_ms: 1_000,
            ..InactivityState::default()
        },
        ..Default::default()
    };

    let active_snapshot = ActivitySnapshot {
        system_input_idle_ms: Some(0),
        screen_activity_enabled: true,
        screen_activity_idle_ms: Some(0),
        microphone_activity: AudioActivitySourceState {
            enabled: true,
            idle_ms: Some(0),
            latest_normalized_level: Some(0.20),
        },
        system_audio_activity: AudioActivitySourceState::default(),
    };
    process_inactivity_audio_transitions_for_snapshot(&mut runtime, 20_000, active_snapshot)
        .expect("active microphone should not pause");
    assert!(!runtime.inactivity.is_microphone_paused());

    let silent_snapshot = ActivitySnapshot {
        system_input_idle_ms: Some(0),
        screen_activity_enabled: true,
        screen_activity_idle_ms: Some(0),
        microphone_activity: AudioActivitySourceState {
            enabled: true,
            idle_ms: Some(0),
            latest_normalized_level: Some(0.0),
        },
        system_audio_activity: AudioActivitySourceState::default(),
    };
    process_inactivity_audio_transitions_for_snapshot(&mut runtime, 30_001, silent_snapshot)
        .expect("silent microphone should pause after timeout");

    assert!(runtime.inactivity.is_microphone_paused());
    assert!(!microphone_backend_active_for_runtime(&runtime));
    assert!(runtime.microphone_recording_file.is_none());

    process_inactivity_audio_transitions_for_snapshot(&mut runtime, 30_002, silent_snapshot)
        .expect("silent raw samples should not resume microphone");

    assert!(runtime.inactivity.is_microphone_paused());
    assert!(!microphone_backend_active_for_runtime(&runtime));
    assert!(runtime.active_microphone_session.is_none());
    assert!(runtime.microphone_recording_file.is_none());
}

#[cfg(target_os = "macos")]
#[test]
fn pause_microphone_for_inactivity_is_idempotent() {
    let mut runtime = audio_paused_runtime_fixture();
    assert!(runtime.inactivity.is_microphone_paused());

    pause_microphone_for_inactivity(&mut runtime)
        .expect("idempotent microphone pause should succeed");

    assert!(runtime.inactivity.is_microphone_paused());
}

// A bounded enrollment recording hard-stops the microphone session. Restoring it
// can fail — the recordings volume can go away while the 15 s take runs — and the
// caller only logs that failure. The one thing that can still re-arm a requested
// microphone with no session is `resume_microphone_from_inactivity`, and it only
// runs while the family is paused. So a failed restore must leave the inactivity
// pause standing; clearing it first strands the microphone dead for the rest of
// the session.
#[cfg(target_os = "macos")]
#[test]
fn a_failed_microphone_restore_leaves_the_inactivity_pause_standing() {
    let mut lifecycle = RecordingLifecycle::default();
    {
        let runtime = lifecycle.runtime_mut();
        *runtime = audio_paused_runtime_fixture();
        // The recordings volume is gone: opening the next microphone file fails
        // before any audio device is touched.
        runtime.microphone_planner = Some(SegmentPlanner::with_date_prefix(
            "/dev/null/mnema-recordings-volume-gone",
            "native-session-microphone",
            current_date_prefix(),
        ));
        assert!(runtime.inactivity.is_microphone_paused());
        assert!(runtime.active_microphone_session.is_none());
    }

    let error = lifecycle
        .restore_microphone_after_out_of_band_recording()
        .expect_err("restoring onto a vanished volume must fail");
    assert_eq!(error.code, "io_error");

    assert!(
        lifecycle.runtime().inactivity.is_microphone_paused(),
        "a failed restore must keep the microphone inactivity pause, or resume_microphone_from_inactivity can never re-arm the session"
    );
    assert!(
        lifecycle.runtime().active_microphone_session.is_none(),
        "no session was started, so the pause is the only thing that can wake one"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn resume_microphone_from_inactivity_requires_requested_sources() {
    let mut runtime = audio_paused_runtime_fixture();
    runtime.requested_sources = None;

    let error =
        resume_microphone_from_inactivity(&mut runtime).expect_err("missing sources should fail");

    assert_eq!(error.code, "invalid_runtime_state");
}

#[cfg(target_os = "macos")]
#[test]
fn resume_microphone_from_inactivity_is_noop_when_not_paused() {
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: false,
        }),
        runtime_controller,
        runtime_state,
        inactivity: InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            ..InactivityState::default()
        },
        ..Default::default()
    };

    // Not paused, so should be a no-op
    resume_microphone_from_inactivity(&mut runtime).expect("noop resume should succeed");
    assert!(!runtime.inactivity.is_microphone_paused());
}
