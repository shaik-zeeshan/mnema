use super::*;

#[cfg(target_os = "macos")]
#[test]
fn pause_screen_for_inactivity_sets_screen_paused_preserves_audio() {
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

    pause_screen_for_inactivity(&mut runtime).expect("screen pause should succeed");

    assert!(runtime.inactivity.is_screen_paused());
    assert!(!runtime.inactivity.is_any_audio_paused());
    assert!(runtime.inactivity.is_paused);
    // Screen segment state should be cleared
    assert!(runtime.recording_file.is_none());
    // current_segment_output_files should be preserved with audio-only bookkeeping
    let output_files = runtime
        .current_segment_output_files
        .as_ref()
        .expect("current_segment_output_files should be preserved for ongoing audio");
    assert!(
        output_files.screen_file.is_none(),
        "screen_file should be cleared"
    );
    assert!(
        output_files.microphone_file.is_some(),
        "microphone_file should be preserved"
    );
    // current_segment_sources should reflect the audio-only active subset
    let segment_sources = runtime
        .current_segment_sources
        .as_ref()
        .expect("current_segment_sources should reflect active audio subset");
    assert!(
        !segment_sources.screen,
        "screen should be excluded after screen pause"
    );
    assert!(
        segment_sources.microphone,
        "microphone should remain active"
    );
    assert!(!segment_sources.system_audio);
    // Microphone recording file stays (audio continues independently)
    assert!(runtime.microphone_recording_file.is_some());
}

#[cfg(target_os = "macos")]
#[test]
fn pause_screen_for_inactivity_is_idempotent() {
    let mut runtime = screen_paused_runtime_fixture();
    assert!(runtime.inactivity.is_screen_paused());

    pause_screen_for_inactivity(&mut runtime).expect("idempotent screen pause should succeed");

    assert!(runtime.inactivity.is_screen_paused());
}

#[cfg(target_os = "macos")]
#[test]
fn resume_screen_from_inactivity_requires_requested_sources() {
    let mut runtime = screen_paused_runtime_fixture();
    runtime.requested_sources = None;

    let error =
        resume_screen_from_inactivity(&mut runtime, None).expect_err("missing sources should fail");

    assert_eq!(error.code, "invalid_runtime_state");
}

#[cfg(target_os = "macos")]
#[test]
fn screen_resume_waits_quietly_while_no_display_is_available() {
    // Regression: during a dark wake with the lid closed (2026-07-05), screen
    // "activity" blips triggered a doomed cold ScreenCaptureKit start on every
    // segment-loop iteration (~3/sec), each failing capture_display_unavailable.
    // A dead stream with no drawable display must defer, not start.
    assert!(should_defer_screen_resume_for_missing_display(false, false));
    // Display returned: attempt the cold start.
    assert!(!should_defer_screen_resume_for_missing_display(false, true));
    // Live stream soft-resume path never defers; it never cold-starts.
    assert!(!should_defer_screen_resume_for_missing_display(true, false));
    assert!(!should_defer_screen_resume_for_missing_display(true, true));
}

#[cfg(target_os = "macos")]
#[test]
fn resume_screen_from_inactivity_is_noop_when_not_paused() {
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
    resume_screen_from_inactivity(&mut runtime, None).expect("noop resume should succeed");
    assert!(!runtime.inactivity.is_screen_paused());
}

#[cfg(target_os = "macos")]
#[test]
fn pause_screen_preserves_audio_paused_state() {
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
            microphone_paused: true,
            system_audio_paused: true,
            is_paused: true,
            ..InactivityState::default()
        },
        ..Default::default()
    };

    pause_screen_for_inactivity(&mut runtime).expect("screen pause should succeed");

    // Both should now be paused
    assert!(runtime.inactivity.is_screen_paused());
    assert!(runtime.inactivity.is_any_audio_paused());
    assert!(runtime.inactivity.is_paused);
}

#[cfg(target_os = "macos")]
#[test]
fn resume_screen_preserves_audio_paused_state() {
    let mut runtime = screen_paused_runtime_fixture();
    // Set both screen and audio paused
    runtime
        .inactivity
        .set_family_paused_states(true, true, true);

    // Resume should fail since start_segment will fail (no real screen capture),
    // but we can test with the _with_start_segment variant pattern.
    // Instead, validate that the is_noop path preserves audio state.
    // Un-pause screen so resume is a noop:
    runtime
        .inactivity
        .set_family_paused_states(false, true, true);

    resume_screen_from_inactivity(&mut runtime, None).expect("noop resume should succeed");

    assert!(!runtime.inactivity.is_screen_paused());
    assert!(runtime.inactivity.is_any_audio_paused());
    assert!(runtime.inactivity.is_paused);
}

#[cfg(target_os = "macos")]
#[test]
fn resume_screen_from_inactivity_requires_segment_planner() {
    let mut runtime = screen_paused_runtime_fixture();
    runtime.segment_planner = None;

    let error =
        resume_screen_from_inactivity(&mut runtime, None).expect_err("missing planner should fail");

    assert_eq!(error.code, "invalid_runtime_state");
}

#[cfg(target_os = "macos")]
#[test]
fn resume_screen_from_inactivity_requires_segment_schedule() {
    let mut runtime = screen_paused_runtime_fixture();
    runtime.segment_schedule = None;

    let error = resume_screen_from_inactivity(&mut runtime, None)
        .expect_err("missing schedule should fail");

    assert_eq!(error.code, "invalid_runtime_state");
}

#[cfg(target_os = "macos")]
#[test]
fn resume_screen_from_inactivity_requires_capture_clock() {
    let mut runtime = screen_paused_runtime_fixture();
    runtime.capture_clock = None;

    let error =
        resume_screen_from_inactivity(&mut runtime, None).expect_err("missing clock should fail");

    assert_eq!(error.code, "invalid_runtime_state");
}

#[test]
fn idle_debug_info_serialization_includes_separate_family_fields() {
    use capture_types::{
        AudioActivityDecision, AudioActivitySample, IdleDebugInfo, RuntimeSourceStatus,
        RuntimeSourcesStatus,
    };

    let info = IdleDebugInfo {
        system_idle_ms: None,
        system_idle_available: false,
        inactivity_enabled: true,
        idle_timeout_seconds: 10,
        is_inactivity_paused: true,
        detector_source: "unavailable".to_string(),
        activity_mode: "system_input_or_screen_or_audio".to_string(),
        microphone_activity_sensitivity: 50,
        system_audio_activity_sensitivity: 50,
        screen_activity_last_unix_ms: None,
        screen_activity_idle_ms: None,
        microphone_activity_sample: AudioActivitySample {
            last_unix_ms: None,
            level: None,
        },
        microphone_activity_decision: AudioActivityDecision {
            enabled: true,
            idle_ms: None,
            activity_threshold: Some(0.08),
            detector: Some("webrtc".to_string()),
        },
        system_audio_activity_sample: AudioActivitySample {
            last_unix_ms: None,
            level: None,
        },
        system_audio_activity_decision: AudioActivityDecision {
            enabled: false,
            idle_ms: None,
            activity_threshold: Some(0.08),
            detector: Some("peak_level".to_string()),
        },
        microphone_vad: capture_types::MicrophoneVadStatus {
            configured_adapter: "silero".to_string(),
            effective_adapter: "webrtc".to_string(),
            fallback_reason: Some("Silero VAD runtime unavailable in test".to_string()),
        },
        effective_idle_ms: 250,
        effective_idle_source: "microphone_capture".to_string(),
        screen_effective_idle_ms: 8_000,
        screen_effective_idle_source: "screen_capture".to_string(),
        screen_paused: true,
        microphone_effective_idle_ms: 250,
        microphone_effective_idle_source: "microphone_capture".to_string(),
        microphone_paused: false,
        system_audio_effective_idle_ms: 500,
        system_audio_effective_idle_source: "system_audio_capture".to_string(),
        system_audio_paused: true,
        activity_sources: vec![],
        runtime_sources: RuntimeSourcesStatus {
            screen: RuntimeSourceStatus {
                requested: true,
                paused: true,
                session_active: Some(false),
                writer_active: Some(false),
                output_path: None,
                reason: None,
            },
            microphone: RuntimeSourceStatus {
                requested: true,
                paused: false,
                session_active: Some(true),
                writer_active: Some(true),
                output_path: Some("/tmp/microphone.mov".to_string()),
                reason: None,
            },
            system_audio: RuntimeSourceStatus {
                requested: false,
                paused: true,
                session_active: Some(false),
                writer_active: Some(false),
                output_path: None,
                reason: Some("not_requested".to_string()),
            },
        },
    };

    let json = serde_json::to_value(&info).expect("serialization should succeed");

    // Combined effective idle (legacy)
    assert_eq!(json["effectiveIdleMs"], 250);
    assert_eq!(json["effectiveActivitySource"], "microphone_capture");

    // Screen-family fields
    assert_eq!(json["screenEffectiveIdleMs"], 8_000);
    assert_eq!(json["screenEffectiveActivitySource"], "screen_capture");
    assert_eq!(json["screenPaused"], true);

    // Audio-family fields (now split into microphone and system_audio)
    assert_eq!(json["microphoneEffectiveIdleMs"], 250);
    assert_eq!(
        json["microphoneEffectiveActivitySource"],
        "microphone_capture"
    );
    assert_eq!(json["microphonePaused"], false);
    assert_eq!(json["systemAudioEffectiveIdleMs"], 500);
    assert_eq!(
        json["systemAudioEffectiveActivitySource"],
        "system_audio_capture"
    );
    assert_eq!(json["systemAudioPaused"], true);

    // VAD adapter state fields
    assert_eq!(json["microphoneVad"]["configuredAdapter"], "silero");
    assert_eq!(json["microphoneVad"]["effectiveAdapter"], "webrtc");
    assert_eq!(
        json["microphoneVad"]["fallbackReason"],
        "Silero VAD runtime unavailable in test"
    );

    // Runtime source status fields
    assert_eq!(json["runtimeSources"]["screen"]["requested"], true);
    assert_eq!(json["runtimeSources"]["screen"]["paused"], true);
    assert_eq!(json["runtimeSources"]["microphone"]["writerActive"], true);
    assert_eq!(
        json["runtimeSources"]["systemAudio"]["reason"],
        "not_requested"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn resume_screen_keeps_system_audio_stream_when_audio_writer_paused() {
    let mut runtime = screen_paused_with_system_audio_runtime_fixture(true);
    runtime.system_audio_planner = Some(SegmentPlanner::with_date_prefix(
        "/tmp/native-capture-tests",
        "paused-system-audio-session",
        "2026/04/22",
    ));

    let expected_screen_file =
        "/tmp/native-capture-tests/native-session-screen-audio-resume-segment-0002/screen.mov"
            .to_string();

    resume_screen_from_inactivity_with_start_segment(
        &mut runtime,
        None,
        |_segment_dir,
         _screen_output,
         sources,
         _fr,
         _res,
         _br,
         _mic,
         _tx,
         _mic_path| {
            assert!(
                !sources.system_audio,
                "the screen stream carries no audio; the tap keeps delivering the activity that resumes it"
            );
            assert!(!sources.microphone);
            assert!(sources.screen);
            Ok(resumed_segment_state_fixture(expected_screen_file.clone()))
        },
    )
    .expect("resume screen should succeed");

    assert!(!runtime.inactivity.is_screen_paused());
    assert!(runtime.inactivity.is_any_audio_paused());
    assert!(runtime.inactivity.is_paused);
    assert!(runtime.system_audio_recording_file.is_none());
}

#[cfg(target_os = "macos")]
#[test]
fn resume_screen_from_inactivity_does_not_require_system_audio_metadata_when_audio_paused() {
    let mut runtime = screen_paused_with_system_audio_runtime_fixture(true);
    runtime.system_audio_planner = None;
    runtime.source_sessions = None;

    let expected_screen_file =
        "/tmp/native-capture-tests/native-session-screen-audio-resume-segment-0002/screen.mov"
            .to_string();

    resume_screen_from_inactivity_with_start_segment(
        &mut runtime,
        None,
        |_, _, sources, _, _, _, _, _, _| {
            assert!(sources.screen);
            assert!(!sources.microphone);
            assert!(!sources.system_audio);

            Ok(resumed_segment_state_fixture(expected_screen_file.clone()))
        },
    )
    .expect("screen-only resume should not require system-audio metadata");

    assert!(runtime.system_audio_planner.is_none());
    assert!(runtime.system_audio_recording_file.is_none());
    assert!(runtime.inactivity.is_any_audio_paused());
}

#[cfg(target_os = "macos")]
#[test]
fn resume_screen_leaves_the_running_system_audio_tap_alone() {
    let mut runtime = screen_paused_with_system_audio_runtime_fixture(false);
    runtime.segment_planner = Some(SegmentPlanner::with_date_prefix(
        "/tmp/native-capture-tests",
        "screen-session",
        "2026/04/19",
    ));
    runtime.system_audio_planner = Some(SegmentPlanner::with_date_prefix(
        "/tmp/native-capture-tests",
        "system-audio-session",
        "2026/04/19",
    ));
    // The tap kept recording straight through the screen pause.
    runtime.system_audio_recording_file = Some("/tmp/system-audio.m4a".to_string());

    let expected_screen_file =
        "/tmp/native-capture-tests/native-session-screen-audio-resume-segment-0002/screen.mov"
            .to_string();

    resume_screen_from_inactivity_with_start_segment(
        &mut runtime,
        None,
        |_segment_dir, _screen_output, sources, _fr, _res, _br, _mic, _tx, _mic_path| {
            assert!(
                !sources.system_audio,
                "an unpaused system-audio family is already recording on its tap; the screen start must not re-plan it"
            );
            assert!(!sources.microphone);
            assert!(sources.screen);

            Ok(resumed_segment_state_fixture(expected_screen_file.clone()))
        },
    )
    .expect("resume screen should succeed");

    assert!(!runtime.inactivity.is_screen_paused());
    assert!(!runtime.inactivity.is_any_audio_paused());
    assert!(!runtime.inactivity.is_paused);
    assert_eq!(
        runtime.system_audio_recording_file,
        Some("/tmp/system-audio.m4a".to_string()),
        "the screen resume must not disturb the file the tap is writing"
    );
    // ...and it must be carried into the new segment's bookkeeping, or the
    // rotation that closes it would never commit it.
    assert_eq!(
        runtime
            .current_segment_output_files
            .as_ref()
            .and_then(|outputs| outputs.system_audio_file.as_deref()),
        Some("/tmp/system-audio.m4a")
    );
}

#[cfg(target_os = "macos")]
#[test]
fn resume_screen_while_audio_paused_preserves_audio_paused_state() {
    let mut runtime = screen_paused_with_system_audio_runtime_fixture(true);

    let expected_screen_file =
        "/tmp/native-capture-tests/native-session-screen-audio-resume-segment-0002/screen.mov"
            .to_string();

    resume_screen_from_inactivity_with_start_segment(
        &mut runtime,
        None,
        |_, _, _, _, _, _, _, _, _| Ok(resumed_segment_state_fixture(expected_screen_file.clone())),
    )
    .expect("resume screen should succeed");

    // Screen resumed, audio stays paused
    assert!(!runtime.inactivity.is_screen_paused());
    assert!(runtime.inactivity.is_any_audio_paused());
    assert!(runtime.inactivity.is_paused);
    assert_eq!(runtime.current_segment_index, 2);
    assert!(runtime.current_segment_output_files.is_some());
    assert!(runtime.recording_file.is_some());
}

#[cfg(target_os = "macos")]
#[test]
fn resume_screen_uses_contiguous_segment_index_when_schedule_has_advanced() {
    let mut runtime = screen_paused_runtime_fixture();
    runtime.current_segment_index = 4;
    runtime.segment_planner = Some(SegmentPlanner::with_date_prefix(
        "/tmp/native-capture-tests",
        "native-session-screen-pause",
        "2026/04/22",
    ));
    runtime.capture_clock = Some(CaptureClock::start_now());
    runtime.segment_schedule = Some(SegmentSchedule::new(std::time::Duration::from_millis(1)));

    std::thread::sleep(std::time::Duration::from_millis(20));

    let expected_date_prefix = current_date_prefix();
    let expected_screen_file = format!(
        "/tmp/native-capture-tests/{expected_date_prefix}/native-session-screen-pause-segment-0005.mov"
    );

    resume_screen_from_inactivity_with_start_segment(
        &mut runtime,
        None,
        |segment_dir, screen_output, _, _, _, _, _, _, _| {
            assert_eq!(
                segment_dir.to_string_lossy(),
                format!(
                    "/tmp/native-capture-tests/{expected_date_prefix}/.native-session-screen-pause-segment-0005"
                )
            );
            assert_eq!(
                screen_output.map(|path| path.to_string_lossy().to_string()),
                Some(expected_screen_file.clone())
            );

            Ok(resumed_segment_state_fixture(expected_screen_file.clone()))
        },
    )
    .expect("screen resume should keep numbering contiguous");

    assert_eq!(runtime.current_segment_index, 5);
}

#[cfg(target_os = "macos")]
#[test]
fn resume_screen_reanchors_segment_boundary_timing() {
    let mut runtime = screen_paused_runtime_fixture();
    runtime.segment_planner = Some(SegmentPlanner::with_date_prefix(
        "/tmp/native-capture-tests",
        "native-session-screen-pause",
        "2026/04/22",
    ));
    runtime.segment_schedule = Some(SegmentSchedule::new(std::time::Duration::from_millis(40)));
    runtime.capture_clock = Some(CaptureClock::start_now());

    std::thread::sleep(std::time::Duration::from_millis(70));

    let expected_screen_file =
        "/tmp/native-capture-tests/2026/04/22/native-session-screen-pause-segment-0002.mov"
            .to_string();

    resume_screen_from_inactivity_with_start_segment(
        &mut runtime,
        None,
        |_, _, _, _, _, _, _, _, _| Ok(resumed_segment_state_fixture(expected_screen_file.clone())),
    )
    .expect("screen resume should succeed");

    let sources = runtime
        .requested_sources
        .clone()
        .expect("requested sources should be preserved");
    let screen_planner = runtime
        .segment_planner
        .clone()
        .expect("screen planner should be preserved");
    let schedule = runtime
        .segment_schedule
        .clone()
        .expect("schedule should be preserved");
    let clock = runtime
        .capture_clock
        .clone()
        .expect("clock should be re-anchored");

    assert!(
        plan_live_rotation_segment(
            &runtime,
            &sources,
            &screen_planner,
            None,
            None,
            &schedule,
            &clock,
        )
        .is_none(),
        "screen resume should reset segment timing instead of catching up immediately"
    );

    std::thread::sleep(std::time::Duration::from_millis(70));

    let delayed_rotation = plan_live_rotation_segment(
        &runtime,
        &sources,
        &screen_planner,
        None,
        None,
        &schedule,
        runtime
            .capture_clock
            .as_ref()
            .expect("clock should still exist after screen resume"),
    )
    .expect("rotation should trigger after the resumed screen segment reaches duration");

    assert_eq!(delayed_rotation.next_index, 3);
}
