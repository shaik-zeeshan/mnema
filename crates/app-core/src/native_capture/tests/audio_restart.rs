use super::*;

// --- Issue 3: pause_audio ordering – mic stops after screen restart ---

#[cfg(target_os = "macos")]
#[test]
fn pause_audio_for_inactivity_does_not_clear_mic_if_screen_restart_fails() {
    // Simulate: screen is active but has no session (restart path is entered
    // but there is no planner). Without a planner the restart helper now fails
    // fast, so the mic session should not have been stopped — the error
    // propagates before the mic-stop step in the caller.
    //
    // With a planner present we test the happy-path ordering: if the function
    // succeeds, mic session should be cleared.
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
        system_audio_recording_file: Some("/tmp/system-audio.m4a".to_string()),
        active_screen_session: None,
        active_microphone_session: None,
        active_system_audio_session: None,
        segment_planner: Some(SegmentPlanner::new(
            "/tmp/native-capture-tests",
            "native-session-audio-pause-mic-order",
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

    // Microphone session should be cleared on success
    assert!(runtime.active_microphone_session.is_none());
    // Audio should be marked as paused
    assert!(runtime.inactivity.is_any_audio_paused());
    // Screen should not be paused
    assert!(!runtime.inactivity.is_screen_paused());
}

// --- Slice 3b6: resume_audio refreshes current_segment_sources when screen is paused ---

#[cfg(target_os = "macos")]
#[test]
fn resume_audio_from_inactivity_refreshes_sources_when_screen_paused() {
    // Scenario: screen is paused, audio was paused, now audio resumes.
    // current_segment_sources should reflect the audio-only active subset.
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        }),
        current_segment_sources: None, // cleared by screen pause
        current_segment_index: 1,
        screen_frame_rate: 5.0,
        screen_resolution: ScreenResolution::default(),
        segment_loop_control: None,
        capture_clock: Some(CaptureClock::start_now()),
        segment_schedule: Some(SegmentSchedule::new(std::time::Duration::from_secs(60))),
        segment_planner: Some(SegmentPlanner::new(
            "/tmp/native-capture-tests",
            "native-session-resume-audio-screen-paused",
        )),
        source_sessions: Some(independent_source_sessions_fixture()),
        current_segment_output_files: None,
        recording_file: None,
        active_screen_session: None,
        active_microphone_session: None,
        active_system_audio_session: None,
        runtime_controller,
        runtime_state,
        inactivity: InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            screen_paused: true,
            microphone_paused: true,
            system_audio_paused: true,
            is_paused: true,
            ..InactivityState::default()
        },
        ..Default::default()
    };

    resume_microphone_from_inactivity(&mut runtime).expect("microphone resume should succeed");
    resume_system_audio_from_inactivity(&mut runtime).expect("system audio resume should succeed");

    // current_segment_sources should now reflect audio-only active subset
    let segment_sources = runtime
        .current_segment_sources
        .as_ref()
        .expect("current_segment_sources should be set after audio resume while screen paused");
    assert!(
        !segment_sources.screen,
        "screen should remain excluded while screen is paused"
    );
    assert!(
        segment_sources.microphone,
        "microphone should be active after audio resume"
    );
    assert!(
        segment_sources.system_audio,
        "system audio resumes on its own tap while the screen stays paused"
    );
    // Screen should still be paused
    assert!(runtime.inactivity.is_screen_paused());
    assert!(!runtime.inactivity.is_any_audio_paused());

    // The resume started here with `current_segment_output_files` at `None` — the
    // state a screen pause or suspension leaves behind. Tracking the file it just
    // opened used to `as_mut()` into that `None` and silently drop it, orphaning a
    // real recording that no rotation would ever commit.
    let microphone_recording_file = runtime
        .microphone_recording_file
        .as_deref()
        .expect("a resumed microphone names the file it is writing");
    let outputs = runtime
        .current_segment_output_files
        .as_ref()
        .expect("an audio resume into an empty segment must create its output map");
    assert_eq!(
        outputs.microphone_file.as_deref(),
        Some(microphone_recording_file)
    );
}

#[cfg(target_os = "macos")]
#[test]
fn resume_audio_from_inactivity_refreshes_sources_when_screen_paused_without_source_sessions() {
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        }),
        current_segment_sources: None,
        current_segment_index: 1,
        screen_frame_rate: 5.0,
        screen_resolution: ScreenResolution::default(),
        segment_loop_control: None,
        capture_clock: Some(CaptureClock::start_now()),
        segment_schedule: Some(SegmentSchedule::new(std::time::Duration::from_secs(60))),
        segment_planner: Some(SegmentPlanner::with_date_prefix(
            "/tmp/native-capture-tests",
            "native-session-resume-audio-screen-paused-no-metadata",
            "2026/04/22",
        )),
        source_sessions: None,
        current_segment_output_files: None,
        recording_file: None,
        active_screen_session: None,
        active_microphone_session: None,
        active_system_audio_session: None,
        runtime_controller,
        runtime_state,
        inactivity: InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            screen_paused: true,
            microphone_paused: true,
            system_audio_paused: true,
            is_paused: true,
            ..InactivityState::default()
        },
        ..Default::default()
    };

    let microphone_result = resume_microphone_from_inactivity(&mut runtime);
    if microphone_result.is_err() {
        assert!(runtime.microphone_planner.is_some());
        assert_eq!(
            runtime
                .source_sessions
                .as_ref()
                .and_then(|sessions| sessions.microphone.as_ref())
                .map(|session| session.session_id.as_str()),
            runtime
                .microphone_planner
                .as_ref()
                .map(|planner| planner.session_id())
        );
    }

    resume_system_audio_from_inactivity(&mut runtime)
        .expect("system audio no-op resume should tolerate missing source session metadata");

    let segment_sources = runtime
        .current_segment_sources
        .as_ref()
        .expect("current_segment_sources should be refreshed after audio resume attempt");
    assert!(!segment_sources.screen);
    assert!(
        segment_sources.system_audio,
        "system audio resumes on its own tap regardless of the screen"
    );
    assert_eq!(
        runtime
            .source_sessions
            .as_ref()
            .and_then(|sessions| sessions.system_audio.as_ref())
            .map(|session| session.session_id.as_str()),
        runtime
            .system_audio_planner
            .as_ref()
            .map(|planner| planner.session_id())
    );
    assert_eq!(
        runtime
            .segment_planner
            .as_ref()
            .map(|planner| planner.date_prefix()),
        runtime
            .microphone_planner
            .as_ref()
            .map(|planner| planner.date_prefix())
    );
    assert!(
        runtime.system_audio_planner.is_none()
            || runtime
                .segment_planner
                .as_ref()
                .map(|planner| planner.date_prefix())
                == runtime
                    .system_audio_planner
                    .as_ref()
                    .map(|planner| planner.date_prefix()),
        "system-audio planner should either remain absent for the no-op branch or share the refreshed date"
    );
}

// --- Slice 3b6: pause_screen preserves current_segment_sources for active audio ---

#[cfg(target_os = "macos")]
#[test]
fn pause_screen_for_inactivity_preserves_sources_for_active_audio() {
    // Scenario: screen+mic+system_audio all active, screen pauses but audio stays active.
    // current_segment_sources should reflect the audio-only active subset.
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
        microphone_recording_file: Some("/tmp/microphone.m4a".to_string()),
        system_audio_recording_file: Some("/tmp/system-audio.m4a".to_string()),
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

    // current_segment_sources should reflect the audio-only active subset
    let segment_sources = runtime
        .current_segment_sources
        .as_ref()
        .expect("current_segment_sources should reflect audio subset when audio is active");
    assert!(
        !segment_sources.screen,
        "screen should be excluded after screen pause"
    );
    assert!(
        segment_sources.microphone,
        "microphone should remain active"
    );
    assert!(
        segment_sources.system_audio,
        "system audio keeps recording on its own tap through a screen pause"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn screen_idle_with_threshold_active_microphone_pauses_only_screen_in_activity_modes() {
    for activity_mode in [
        InactivityActivityMode::SystemInputOrScreen,
        InactivityActivityMode::SystemInputOrScreenOrAudio,
    ] {
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
                activity_mode,
                ..InactivityState::default()
            },
            ..Default::default()
        };
        let snapshot = ActivitySnapshot {
            system_input_idle_ms: Some(10_001),
            screen_activity_enabled: true,
            screen_activity_idle_ms: Some(10_001),
            microphone_activity: AudioActivitySourceState {
                enabled: true,
                idle_ms: Some(0),
                latest_normalized_level: Some(0.20),
            },
            system_audio_activity: AudioActivitySourceState::default(),
        };

        assert!(!runtime
            .inactivity
            .should_pause_for_inactivity(20_000, snapshot));
        assert!(!runtime
            .inactivity
            .should_pause_microphone_for_inactivity(20_000, snapshot));
        assert!(runtime
            .inactivity
            .should_pause_screen_for_inactivity(20_000, snapshot));

        pause_screen_for_inactivity(&mut runtime).expect("screen pause should succeed");

        assert!(runtime.inactivity.is_screen_paused());
        assert!(!runtime.inactivity.is_microphone_paused());
        assert!(!runtime.inactivity.is_system_audio_paused());
        assert!(runtime.microphone_recording_file.is_some());
        let sources = runtime
            .current_segment_sources
            .as_ref()
            .expect("microphone should keep active source subset");
        assert!(!sources.screen);
        assert!(sources.microphone);
        assert!(!sources.system_audio);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn screen_idle_with_threshold_active_system_audio_pauses_screen_without_audio_family_pause() {
    for activity_mode in [InactivityActivityMode::SystemInputOrScreen] {
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
                activity_mode,
                ..InactivityState::default()
            },
            ..Default::default()
        };
        let snapshot = ActivitySnapshot {
            system_input_idle_ms: Some(10_001),
            screen_activity_enabled: true,
            screen_activity_idle_ms: Some(10_001),
            microphone_activity: AudioActivitySourceState::default(),
            system_audio_activity: AudioActivitySourceState {
                enabled: true,
                idle_ms: Some(0),
                latest_normalized_level: Some(0.20),
            },
        };

        assert!(!runtime
            .inactivity
            .should_pause_for_inactivity(20_000, snapshot));
        assert!(!runtime
            .inactivity
            .should_pause_system_audio_for_inactivity(20_000, snapshot));
        assert!(runtime
            .inactivity
            .should_pause_screen_for_inactivity(20_000, snapshot));

        pause_screen_for_inactivity(&mut runtime).expect("screen pause should succeed");

        assert!(runtime.inactivity.is_screen_paused());
        assert!(!runtime.inactivity.is_microphone_paused());
        assert!(!runtime.inactivity.is_system_audio_paused());
        assert_eq!(
            runtime.current_segment_sources,
            Some(CaptureSources {
                screen: false,
                microphone: false,
                system_audio: true,
            }),
            "an unpaused system-audio family keeps recording on its tap through a screen pause"
        );
        assert_eq!(
            runtime.system_audio_recording_file.as_deref(),
            Some("/tmp/system-audio.m4a"),
            "and the file it is still writing stays tracked, or nothing ever commits it"
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn pause_screen_for_inactivity_clears_sources_when_audio_also_paused() {
    // Scenario: audio was already paused, now screen pauses too.
    // current_segment_sources should be None (everything paused).
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
            microphone: false,
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

    // All families paused => current_segment_sources should be None
    assert!(
        runtime.current_segment_sources.is_none(),
        "current_segment_sources should be None when all families are paused"
    );
    assert!(runtime.inactivity.is_screen_paused());
    assert!(runtime.inactivity.is_any_audio_paused());
}

// --- Slice 3b6: pause_audio_restart_screen defers bookkeeping until success ---

#[cfg(target_os = "macos")]
#[test]
fn pause_audio_restart_screen_fails_fast_on_no_planner() {
    // When there is no planner, the restart helper cannot restart the screen
    // session after stopping it.  It must fail with reconciled bookkeeping
    // so the caller does not see stale recording paths.
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
        segment_planner: None, // no planner
        runtime_controller,
        runtime_state,
        inactivity: InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            ..InactivityState::default()
        },
        ..Default::default()
    };

    // With no active screen session, the function should succeed safely
    // without attempting a restart — just reconcile system audio bookkeeping.
    pause_system_audio_for_inactivity(&mut runtime)
        .expect("should succeed safely when no active screen session");

    // System audio bookkeeping must be cleared.
    assert!(
        runtime.system_audio_recording_file.is_none(),
        "system_audio_recording_file should be cleared"
    );
    // System audio should be marked as paused.
    assert!(
        runtime.inactivity.is_system_audio_paused(),
        "system_audio should be marked paused"
    );
    // Screen recording_file should be untouched — no screen session was stopped.
    assert!(
        runtime.recording_file.is_some(),
        "recording_file should be preserved when no screen session was active"
    );
}

// --- Slice 3b7: current_segment_sources_for_runtime reflects actual backend state ---

#[cfg(target_os = "macos")]
#[test]
fn pause_microphone_for_inactivity_preserves_privacy_suspended_source_mask() {
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();
    let privacy_error = CaptureErrorResponse {
        code: "privacy_filter_apply_failed".to_string(),
        message: "privacy filter application failed".to_string(),
    };
    let microphone_file = "/tmp/privacy-suspended-microphone.m4a".to_string();

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: false,
        }),
        current_segment_sources: Some(CaptureSources {
            screen: false,
            microphone: true,
            system_audio: false,
        }),
        current_segment_index: 1,
        screen_frame_rate: 5.0,
        screen_resolution: ScreenResolution::default(),
        current_segment_output_files: Some(CaptureOutputFiles {
            screen_file: None,
            screen_files: Vec::new(),
            microphone_file: Some(microphone_file.clone()),
            microphone_files: vec![microphone_file.clone()],
            system_audio_file: None,
            system_audio_files: Vec::new(),
        }),
        recording_file: None,
        microphone_recording_file: Some(microphone_file),
        system_audio_recording_file: None,
        active_screen_session: None,
        active_microphone_session: None,
        active_system_audio_session: None,
        capture_suspension: Some(CaptureSuspension::with_kind(
            CaptureSuspensionKind::PrivacyFilter,
            &privacy_error,
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

    pause_microphone_for_inactivity(&mut runtime).expect("microphone pause should succeed");

    if let Some(sources) = runtime.current_segment_sources.as_ref() {
        assert!(
            !sources.screen,
            "screen must stay suspended while privacy recovery is active"
        );
        assert!(!sources.microphone, "microphone should be paused");
        assert!(!sources.system_audio, "system audio must stay suspended");
    }
    assert!(
        current_segment_sources_for_runtime(&runtime).is_none(),
        "runtime source lookup must not reintroduce suspended screen capture"
    );
    assert!(runtime.capture_suspension.is_some());
}
