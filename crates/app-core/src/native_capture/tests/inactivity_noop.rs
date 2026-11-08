use super::*;

#[cfg(target_os = "macos")]
#[test]
fn pause_microphone_for_inactivity_noop_when_microphone_not_requested() {
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: false, // microphone not requested
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

    pause_microphone_for_inactivity(&mut runtime).expect("should succeed as noop");
    assert!(
        !runtime.inactivity.microphone_paused,
        "microphone_paused should not be set when source not requested"
    );
    assert!(
        !runtime.inactivity.is_paused,
        "is_paused should not be set when source not requested"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn inactivity_audio_transition_does_not_keep_trying_to_pause_unrequested_microphone() {
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: false,
        }),
        runtime_controller,
        runtime_state,
        inactivity: InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            last_activity_monotonic_ms: 0,
            ..InactivityState::default()
        },
        ..Default::default()
    };

    let idle_snapshot = ActivitySnapshot {
        system_input_idle_ms: Some(20_000),
        screen_activity_enabled: true,
        screen_activity_idle_ms: Some(20_000),
        microphone_activity: AudioActivitySourceState::default(),
        system_audio_activity: AudioActivitySourceState::default(),
    };

    assert!(
        !runtime
            .inactivity
            .should_pause_microphone_for_inactivity(20_000, idle_snapshot),
        "unrequested microphone should never report a pause transition"
    );

    process_inactivity_audio_transitions_for_snapshot(&mut runtime, 20_000, idle_snapshot)
        .expect("unrequested microphone transition should be a noop");

    assert!(!runtime.inactivity.is_microphone_paused());

    assert!(
        !runtime
            .inactivity
            .should_pause_microphone_for_inactivity(21_000, idle_snapshot),
        "repeated idle ticks should stay as noops for unrequested microphone"
    );

    process_inactivity_audio_transitions_for_snapshot(&mut runtime, 21_000, idle_snapshot)
        .expect("repeated unrequested microphone transition should stay a noop");

    assert!(!runtime.inactivity.is_microphone_paused());
}

#[cfg(target_os = "macos")]
#[test]
fn resume_microphone_from_inactivity_noop_when_microphone_not_requested() {
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: false, // microphone not requested
            system_audio: false,
        }),
        runtime_controller,
        runtime_state,
        inactivity: InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            microphone_paused: true,
            is_paused: true,
            ..InactivityState::default()
        },
        ..Default::default()
    };

    resume_microphone_from_inactivity(&mut runtime).expect("should succeed as noop");
    assert!(
        runtime.inactivity.microphone_paused,
        "microphone_paused should remain set when source not requested"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn pause_system_audio_for_inactivity_noop_when_system_audio_not_requested() {
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: false, // system audio not requested
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

    pause_system_audio_for_inactivity(&mut runtime).expect("should succeed as noop");
    assert!(
        !runtime.inactivity.system_audio_paused,
        "system_audio_paused should not be set when source not requested"
    );
    assert!(
        !runtime.inactivity.is_paused,
        "is_paused should not be set when source not requested"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn inactivity_audio_transition_does_not_keep_trying_to_pause_unrequested_system_audio() {
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: false,
        }),
        runtime_controller,
        runtime_state,
        inactivity: InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            last_activity_monotonic_ms: 0,
            ..InactivityState::default()
        },
        ..Default::default()
    };

    let idle_snapshot = ActivitySnapshot {
        system_input_idle_ms: Some(20_000),
        screen_activity_enabled: true,
        screen_activity_idle_ms: Some(20_000),
        microphone_activity: AudioActivitySourceState::default(),
        system_audio_activity: AudioActivitySourceState::default(),
    };

    assert!(
        !runtime
            .inactivity
            .should_pause_system_audio_for_inactivity(20_000, idle_snapshot),
        "unrequested system audio should never report a pause transition"
    );

    process_inactivity_audio_transitions_for_snapshot(&mut runtime, 20_000, idle_snapshot)
        .expect("unrequested system audio transition should be a noop");

    assert!(!runtime.inactivity.is_system_audio_paused());

    assert!(
        !runtime
            .inactivity
            .should_pause_system_audio_for_inactivity(21_000, idle_snapshot),
        "repeated idle ticks should stay as noops for unrequested system audio"
    );

    process_inactivity_audio_transitions_for_snapshot(&mut runtime, 21_000, idle_snapshot)
        .expect("repeated unrequested system audio transition should stay a noop");

    assert!(!runtime.inactivity.is_system_audio_paused());
}

#[cfg(target_os = "macos")]
#[test]
fn pause_screen_for_inactivity_noop_when_screen_not_requested() {
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: false,
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

    let idle_snapshot = ActivitySnapshot {
        system_input_idle_ms: Some(20_000),
        screen_activity_enabled: false,
        screen_activity_idle_ms: Some(20_000),
        microphone_activity: AudioActivitySourceState::default(),
        system_audio_activity: AudioActivitySourceState::default(),
    };

    assert!(
        !runtime
            .inactivity
            .should_pause_screen_for_inactivity(20_000, idle_snapshot),
        "unrequested screen should never report a pause transition"
    );

    pause_screen_for_inactivity(&mut runtime).expect("screen pause should noop when unrequested");

    assert!(
        !runtime.inactivity.is_screen_paused(),
        "screen_paused should not be set when screen is not requested"
    );
    assert!(
        !runtime.inactivity.is_paused,
        "is_paused should not be set when screen is not requested"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn resume_system_audio_from_inactivity_noop_when_system_audio_not_requested() {
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: false, // system audio not requested
        }),
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

    resume_system_audio_from_inactivity(&mut runtime).expect("should succeed as noop");
    assert!(
        runtime.inactivity.system_audio_paused,
        "system_audio_paused should remain set when source not requested"
    );
}

// Regression, the ADR 0052 headline scenario: screen + system audio with the mic
// off, the user walks away, both families pause for inactivity (the tap stays
// alive by design), then the display sleeps. The suspension leaves
// `current_segment_output_files` at `None` because nothing was live. Music starts,
// system audio resumes on its own — nothing gates it on the screen any more — and
// `advance_segment` opens a real file the tap records into. The resume tracked
// that path with `as_mut()`, which silently no-ops on `None`, so the file was
// tracked nowhere: up to five minutes of real audio on disk with no
// `audio_segment` row, never transcribed, never searchable, invisible to
// retention. Both halves are pinned here — that the `None` state is reachable at
// all, and that tracking a resumed file into it creates the map rather than
// dropping the file. Driving the resume itself would need a live Core Audio tap,
// which no headless test has.
#[cfg(target_os = "macos")]
#[test]
fn a_system_audio_resume_into_a_suspended_segment_still_tracks_its_output_file() {
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();
    let screen_and_system_audio = CaptureSources {
        screen: true,
        microphone: false,
        system_audio: true,
    };

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(screen_and_system_audio.clone()),
        current_segment_sources: Some(screen_and_system_audio),
        current_segment_index: 1,
        current_segment_output_files: Some(CaptureOutputFiles {
            screen_file: Some("/tmp/screen.mov".to_string()),
            screen_files: vec!["/tmp/screen.mov".to_string()],
            microphone_file: None,
            microphone_files: Vec::new(),
            system_audio_file: None,
            system_audio_files: Vec::new(),
        }),
        recording_file: Some("/tmp/screen.mov".to_string()),
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

    crate::native_capture::segments::suspend_screen_capture(
        None,
        &mut runtime,
        &CaptureErrorResponse {
            code: "capture_display_unavailable".to_string(),
            message: "display asleep".to_string(),
        },
        CaptureSuspensionKind::DisplayUnavailable,
    )
    .expect("a display sleep should suspend the screen");

    assert!(
        runtime.current_segment_output_files.is_none(),
        "with both families paused the suspension leaves no segment outputs — the state a resume lands in"
    );

    crate::native_capture_output::set_current_system_audio_output_file(
        crate::native_capture::segments::current_segment_output_files_mut(&mut runtime),
        "/tmp/system-audio-resumed.m4a".to_string(),
    );

    let outputs = runtime
        .current_segment_output_files
        .as_ref()
        .expect("tracking a resumed file must create the segment outputs, not drop it");
    assert_eq!(
        outputs.system_audio_file.as_deref(),
        Some("/tmp/system-audio-resumed.m4a")
    );
    assert_eq!(
        outputs.system_audio_files,
        vec!["/tmp/system-audio-resumed.m4a".to_string()]
    );
}

#[cfg(target_os = "macos")]
#[test]
fn resume_system_audio_from_inactivity_without_a_tap_clears_the_flag_and_does_no_writer_work() {
    // With no tap (and no system-audio planner) there is no writer work to do,
    // but the resume must still succeed and clear the paused flag: "paused for
    // inactivity" tracks activity, not backend health. It used to stay paused
    // because the resume asked the screen backend for permission (ADR 0052).
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: true,
        }),
        current_segment_index: 1,
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

    resume_system_audio_from_inactivity(&mut runtime)
        .expect("resume should do nothing rather than fail when no tap is running");

    assert!(
        !runtime.inactivity.is_system_audio_paused(),
        "the resume must clear the paused flag even with no tap to resume"
    );
    assert!(
        runtime.system_audio_planner.is_none(),
        "no tap means no writer to plan a file for"
    );
    assert!(
        runtime.system_audio_recording_file.is_none(),
        "with no tap there is no file to write"
    );
    // The screen's bookkeeping is untouched by an audio-only resume.
    assert_eq!(runtime.recording_file.as_deref(), Some("/tmp/screen.mov"));
}

// The pause/resume cycle the family runs with no screen at all — the shape an
// audio-only session lives in, and the one the old resume action could never
// reach (it demanded a live ScreenCaptureKit session).
#[cfg(target_os = "macos")]
#[test]
fn system_audio_inactivity_pause_and_resume_need_no_screen() {
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();
    let audio_only = CaptureSources {
        screen: false,
        microphone: false,
        system_audio: true,
    };

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(audio_only.clone()),
        current_segment_sources: Some(audio_only.clone()),
        current_segment_index: 1,
        current_segment_output_files: Some(CaptureOutputFiles {
            screen_file: None,
            screen_files: Vec::new(),
            microphone_file: None,
            microphone_files: Vec::new(),
            system_audio_file: Some("/tmp/system-audio.m4a".to_string()),
            system_audio_files: vec!["/tmp/system-audio.m4a".to_string()],
        }),
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

    pause_system_audio_for_inactivity(&mut runtime)
        .expect("system audio must pause with no screen session in sight");

    assert!(runtime.inactivity.is_system_audio_paused());
    assert!(runtime.system_audio_recording_file.is_none());
    let outputs = runtime
        .current_segment_output_files
        .as_ref()
        .expect("segment bookkeeping survives the pause");
    assert!(outputs.system_audio_file.is_none());
    assert!(outputs.system_audio_files.is_empty());
    assert!(
        runtime.current_segment_sources.is_none(),
        "pausing the only source leaves nothing recording"
    );

    resume_system_audio_from_inactivity(&mut runtime)
        .expect("system audio must resume with no screen session in sight");

    assert!(
        !runtime.inactivity.is_system_audio_paused(),
        "the resume must not defer on screen state it no longer reads"
    );
    assert_eq!(runtime.current_segment_sources, Some(audio_only));
}

#[cfg(target_os = "macos")]
#[test]
fn resume_screen_from_inactivity_never_plans_system_audio() {
    let mut runtime = screen_paused_with_system_audio_runtime_fixture(false);
    let expected_date_prefix = current_date_prefix();
    runtime.segment_planner = Some(SegmentPlanner::with_date_prefix(
        "/tmp/native-capture-tests",
        "native-session-screen",
        "2026/04/22",
    ));
    runtime.system_audio_planner = None;
    runtime.source_sessions = None;

    let expected_screen_file = format!(
        "/tmp/native-capture-tests/{expected_date_prefix}/native-session-screen-segment-0002.mov"
    );

    resume_screen_from_inactivity_with_start_segment(
        &mut runtime,
        None,
        |_, screen_output, sources, _, _, _, _, _, _| {
            assert!(sources.screen);
            assert!(!sources.microphone);
            assert!(
                !sources.system_audio,
                "a screen resume starts the screen alone; the tap is already running"
            );
            assert_eq!(
                screen_output,
                Some(std::path::Path::new(expected_screen_file.as_str()))
            );

            Ok(resumed_segment_state_fixture(expected_screen_file.clone()))
        },
    )
    .expect("screen resume should not require system-audio metadata");

    assert!(
        runtime.system_audio_planner.is_none(),
        "the system-audio planner is seeded when the tap starts, not by a screen resume"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn microphone_reconnect_blocked_when_audio_inactivity_paused() {
    let mut runtime = audio_paused_runtime_fixture();
    // Ensure screen_paused is false but audio_paused is true
    runtime
        .inactivity
        .set_family_paused_states(false, true, true);

    let state = MicrophoneControllerState {
        devices: vec![capture_types::MicrophoneDevice {
            id: "mic-1".to_string(),
            name: "Mic 1".to_string(),
            is_default: false,
        }],
        preference: MicrophonePreference {
            mode: MicrophonePreferenceMode::SpecificDevice,
            device_id: Some("mic-1".to_string()),
        },
        disconnect_policy: MicrophoneDisconnectPolicy::WaitForSameDevice,
        effective_device: Some(capture_types::MicrophoneDevice {
            id: "mic-1".to_string(),
            name: "Mic 1".to_string(),
            is_default: false,
        }),
    };

    // Audio is paused via audio_paused, so reconnect should be blocked
    assert!(!should_reconnect_waiting_microphone_session(
        &runtime, &state
    ));
}

#[cfg(target_os = "macos")]
#[test]
fn microphone_reconnect_allowed_when_only_screen_paused() {
    let runtime_controller = running_runtime_controller();
    let runtime_state = runtime_controller.state();

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: false,
        }),
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
    runtime
        .inactivity
        .set_family_paused_states(true, false, false);

    let state = MicrophoneControllerState {
        devices: vec![capture_types::MicrophoneDevice {
            id: "mic-1".to_string(),
            name: "Mic 1".to_string(),
            is_default: false,
        }],
        preference: MicrophonePreference {
            mode: MicrophonePreferenceMode::SpecificDevice,
            device_id: Some("mic-1".to_string()),
        },
        disconnect_policy: MicrophoneDisconnectPolicy::WaitForSameDevice,
        effective_device: Some(capture_types::MicrophoneDevice {
            id: "mic-1".to_string(),
            name: "Mic 1".to_string(),
            is_default: false,
        }),
    };

    // Screen is paused but audio is not, so reconnect should be allowed
    assert!(should_reconnect_waiting_microphone_session(
        &runtime, &state
    ));
}
