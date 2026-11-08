use super::*;

#[cfg(target_os = "macos")]
#[test]
fn pause_audio_for_inactivity_updates_current_segment_sources() {
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        }),
        current_segment_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        }),
        current_segment_index: 1,
        screen_frame_rate: 5.0,
        screen_resolution: ScreenResolution::default(),
        current_segment_output_files: Some(CaptureOutputFiles {
            screen_file: Some("/tmp/screen.mov".to_string()),
            screen_files: vec!["/tmp/screen.mov".to_string()],
            microphone_file: Some("/tmp/microphone.m4a".to_string()),
            microphone_files: vec!["/tmp/microphone.m4a".to_string()],
            system_audio_file: Some("/tmp/system-audio.m4a".to_string()),
            system_audio_files: vec!["/tmp/system-audio.m4a".to_string()],
        }),
        recording_file: Some("/tmp/screen.mov".to_string()),
        active_screen_session: None,
        active_microphone_session: None,
        active_system_audio_session: None,
        segment_planner: Some(SegmentPlanner::new(
            "/tmp/native-capture-tests",
            "native-session-audio-pause-sources",
        )),
        runtime_controller,
        runtime_state,
        inactivity: InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            ..InactivityState::default()
        },
        ..Default::default()
    };

    pause_system_audio_for_inactivity(&mut runtime).expect("system audio pause should succeed");
    pause_microphone_for_inactivity(&mut runtime).expect("microphone pause should succeed");

    // current_segment_sources should reflect only the active screen source
    let segment_sources = runtime
        .current_segment_sources
        .as_ref()
        .expect("current_segment_sources should be set");
    assert!(segment_sources.screen);
    assert!(
        !segment_sources.microphone,
        "microphone should be excluded after audio pause"
    );
    assert!(
        !segment_sources.system_audio,
        "system_audio should be excluded after audio pause"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn pause_audio_for_inactivity_clears_system_audio_recording_file() {
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: true,
        }),
        current_segment_sources: Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: true,
        }),
        current_segment_index: 1,
        screen_frame_rate: 5.0,
        screen_resolution: ScreenResolution::default(),
        current_segment_output_files: Some(CaptureOutputFiles {
            screen_file: Some("/tmp/screen.mov".to_string()),
            screen_files: vec!["/tmp/screen.mov".to_string()],
            microphone_file: None,
            microphone_files: Vec::new(),
            system_audio_file: Some("/tmp/system-audio.m4a".to_string()),
            system_audio_files: vec!["/tmp/system-audio.m4a".to_string()],
        }),
        recording_file: Some("/tmp/screen.mov".to_string()),
        system_audio_recording_file: Some("/tmp/system-audio.m4a".to_string()),
        active_screen_session: None,
        active_microphone_session: None,
        active_system_audio_session: None,
        segment_planner: Some(SegmentPlanner::new(
            "/tmp/native-capture-tests",
            "native-session-audio-pause-sysaudio",
        )),
        runtime_controller,
        runtime_state,
        inactivity: InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            ..InactivityState::default()
        },
        ..Default::default()
    };

    pause_system_audio_for_inactivity(&mut runtime).expect("system audio pause should succeed");

    // system_audio_recording_file should be cleared because system audio was suppressed
    assert!(
        runtime.system_audio_recording_file.is_none(),
        "system_audio_recording_file should be cleared when audio is paused"
    );
    assert!(runtime.inactivity.is_any_audio_paused());
    assert!(!runtime.inactivity.is_screen_paused());
}

#[cfg(target_os = "macos")]
#[test]
fn live_audio_inactivity_pause_detaches_system_audio_writer_truth_while_screen_stays_active() {
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: true,
        }),
        current_segment_sources: Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: true,
        }),
        current_segment_index: 1,
        screen_frame_rate: 5.0,
        screen_resolution: ScreenResolution::default(),
        current_segment_output_files: Some(CaptureOutputFiles {
            screen_file: Some("/tmp/screen.mov".to_string()),
            screen_files: vec!["/tmp/screen.mov".to_string()],
            microphone_file: None,
            microphone_files: Vec::new(),
            system_audio_file: Some("/tmp/system-audio.m4a".to_string()),
            system_audio_files: vec!["/tmp/system-audio.m4a".to_string()],
        }),
        recording_file: Some("/tmp/screen.mov".to_string()),
        system_audio_recording_file: Some("/tmp/system-audio.m4a".to_string()),
        active_screen_session: None,
        active_microphone_session: None,
        active_system_audio_session: None,
        runtime_controller,
        runtime_state,
        inactivity: InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            system_audio_activity_sensitivity: 100,
            last_activity_monotonic_ms: 1_000,
            ..InactivityState::default()
        },
        ..Default::default()
    };

    let active_snapshot = ActivitySnapshot {
        system_input_idle_ms: Some(0),
        screen_activity_enabled: true,
        screen_activity_idle_ms: Some(0),
        microphone_activity: AudioActivitySourceState::default(),
        system_audio_activity: AudioActivitySourceState {
            enabled: true,
            idle_ms: Some(0),
            latest_normalized_level: Some(0.20),
        },
    };
    process_inactivity_audio_transitions_for_snapshot(&mut runtime, 20_000, active_snapshot)
        .expect("active system audio should not pause");
    assert!(!runtime.inactivity.is_system_audio_paused());

    let silent_snapshot = ActivitySnapshot {
        system_input_idle_ms: Some(0),
        screen_activity_enabled: true,
        screen_activity_idle_ms: Some(0),
        microphone_activity: AudioActivitySourceState::default(),
        system_audio_activity: AudioActivitySourceState {
            enabled: true,
            idle_ms: Some(0),
            latest_normalized_level: Some(0.0),
        },
    };
    process_inactivity_audio_transitions_for_snapshot(&mut runtime, 30_001, silent_snapshot)
        .expect("silent system audio should pause after timeout");

    assert!(runtime.inactivity.is_system_audio_paused());
    assert!(!runtime.inactivity.is_screen_paused());
    assert!(runtime.recording_file.is_some());
    assert!(!system_audio_writer_active_for_runtime(&runtime));
    assert!(runtime.system_audio_recording_file.is_none());
    let outputs = runtime
        .current_segment_output_files
        .as_ref()
        .expect("screen output bookkeeping should stay present");
    assert!(outputs.screen_file.is_some());
    assert!(outputs.system_audio_file.is_none());
    assert!(outputs.system_audio_files.is_empty());
    let sources = runtime
        .current_segment_sources
        .as_ref()
        .expect("screen source should remain active");
    assert!(sources.screen);
    assert!(!sources.system_audio);

    process_inactivity_audio_transitions_for_snapshot(&mut runtime, 30_002, silent_snapshot)
        .expect("silent raw samples should not resume system audio");

    assert!(runtime.inactivity.is_system_audio_paused());
    assert!(!runtime.inactivity.is_screen_paused());
    assert!(!system_audio_writer_active_for_runtime(&runtime));
    assert!(runtime.system_audio_recording_file.is_none());
}

#[cfg(target_os = "macos")]
#[test]
fn resume_screen_from_inactivity_sets_current_segment_sources_reflecting_audio_paused() {
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

    // current_segment_sources should reflect that audio is still paused
    let segment_sources = runtime
        .current_segment_sources
        .as_ref()
        .expect("current_segment_sources should be set after screen resume");
    assert!(segment_sources.screen);
    assert!(
        !segment_sources.microphone,
        "microphone should remain excluded while audio is paused"
    );
    assert!(
        !segment_sources.system_audio,
        "system_audio should remain excluded while audio is paused"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn resume_screen_from_inactivity_sets_current_segment_sources_with_all_when_audio_active() {
    let mut runtime = screen_paused_with_system_audio_runtime_fixture(false);

    let expected_screen_file =
        "/tmp/native-capture-tests/native-session-screen-audio-resume-segment-0002/screen.mov"
            .to_string();

    resume_screen_from_inactivity_with_start_segment(
        &mut runtime,
        None,
        |_, _, _, _, _, _, _, _, _| Ok(resumed_segment_state_fixture(expected_screen_file.clone())),
    )
    .expect("resume screen should succeed");

    // current_segment_sources should include all requested sources when audio is active
    let segment_sources = runtime
        .current_segment_sources
        .as_ref()
        .expect("current_segment_sources should be set after screen resume");
    assert!(segment_sources.screen);
    assert!(
        segment_sources.microphone,
        "microphone should be included when audio is not paused"
    );
    assert!(
        segment_sources.system_audio,
        "system_audio should be included when audio is not paused"
    );
}

// --- Issue 1: audio pause clears stale system_audio bookkeeping ---

#[cfg(target_os = "macos")]
#[test]
fn pause_audio_for_inactivity_clears_system_audio_output_file_bookkeeping() {
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: true,
        }),
        current_segment_sources: Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: true,
        }),
        current_segment_index: 1,
        screen_frame_rate: 5.0,
        screen_resolution: ScreenResolution::default(),
        current_segment_output_files: Some(CaptureOutputFiles {
            screen_file: Some("/tmp/screen.mov".to_string()),
            screen_files: vec!["/tmp/screen.mov".to_string()],
            microphone_file: None,
            microphone_files: Vec::new(),
            system_audio_file: Some("/tmp/system-audio.m4a".to_string()),
            system_audio_files: vec!["/tmp/system-audio.m4a".to_string()],
        }),
        recording_file: Some("/tmp/screen.mov".to_string()),
        system_audio_recording_file: Some("/tmp/system-audio.m4a".to_string()),
        active_screen_session: None,
        active_microphone_session: None,
        active_system_audio_session: None,
        segment_planner: Some(SegmentPlanner::new(
            "/tmp/native-capture-tests",
            "native-session-audio-pause-output",
        )),
        runtime_controller,
        runtime_state,
        inactivity: InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            ..InactivityState::default()
        },
        ..Default::default()
    };

    pause_system_audio_for_inactivity(&mut runtime).expect("system audio pause should succeed");

    // The finished file has already moved into committed output bookkeeping.
    // Clear the live pointer and list so a later screen pause/rotation cannot
    // re-commit the old audio with a newer segment clock.
    let output_files = runtime
        .current_segment_output_files
        .as_ref()
        .expect("output files should still exist");
    assert!(
        output_files.system_audio_file.is_none(),
        "system_audio_file should be cleared from output bookkeeping"
    );
    assert!(output_files.system_audio_files.is_empty());
    // screen bookkeeping must remain intact
    assert!(output_files.screen_file.is_some());
}

// --- Issue 2: segment rotation uses active source subset ---

#[cfg(target_os = "macos")]
#[test]
fn inactivity_resume_sets_current_segment_sources_via_active_sources_helper() {
    // Legacy resume (is_paused=true, screen_paused=false, audio_paused=false)
    // should still set current_segment_sources equal to requested_sources
    // (the helper returns all sources when nothing is paused).
    let mut runtime = paused_runtime_fixture();
    assert!(runtime.current_segment_sources.is_none());

    resume_runtime_from_inactivity(&mut runtime).expect("resume should succeed");

    // For legacy resume both family flags are false, so the helper returns
    // the full requested set — same as the old behavior.
    assert_eq!(runtime.current_segment_sources, runtime.requested_sources,);
}

#[cfg(target_os = "macos")]
#[test]
fn inactivity_resume_soft_resume_preserves_segment_index_when_schedule_has_advanced() {
    let mut runtime = paused_runtime_fixture();
    runtime.current_segment_index = 4;
    runtime.segment_planner = Some(SegmentPlanner::with_date_prefix(
        "/tmp/native-capture-tests",
        "native-session-resume",
        "2026/04/19",
    ));
    runtime.capture_clock = Some(CaptureClock::start_now());
    runtime.segment_schedule = Some(SegmentSchedule::new(std::time::Duration::from_millis(1)));

    std::thread::sleep(std::time::Duration::from_millis(20));

    resume_runtime_from_inactivity(&mut runtime)
        .expect("soft-resume should preserve current segment numbering");

    assert_eq!(runtime.current_segment_index, 4);
}

#[cfg(target_os = "macos")]
#[test]
fn inactivity_resume_soft_resume_preserves_segment_boundary_timing() {
    let mut runtime = paused_runtime_fixture();
    runtime.segment_planner = Some(SegmentPlanner::with_date_prefix(
        "/tmp/native-capture-tests",
        "native-session-resume",
        "2026/04/19",
    ));
    runtime.segment_schedule = Some(SegmentSchedule::new(std::time::Duration::from_millis(40)));
    runtime.capture_clock = Some(CaptureClock::start_now());

    std::thread::sleep(std::time::Duration::from_millis(70));

    resume_runtime_from_inactivity(&mut runtime).expect("resume should succeed");

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
        .is_some(),
        "soft-resume should preserve the existing segment timing cadence"
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
            .expect("clock should still exist after resume"),
    )
    .expect("rotation should trigger after the new segment reaches its duration");

    assert_eq!(delayed_rotation.next_index, 2);
}
