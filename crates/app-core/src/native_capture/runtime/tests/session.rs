//! Session start/stop/reset bookkeeping on the runtime.

use crate::native_capture::frame_artifacts::FrameArtifactMessage;
use crate::native_capture::runtime::*;
use crate::native_capture_inactivity::InactivityState;
use capture_runtime::{RuntimeController, RuntimeSignal, RuntimeState, SegmentPlanner};
use capture_types::{
    CaptureOutputFiles, CaptureSources, CaptureSupportResponse, ScreenResolution,
    ScreenResolutionPreset, SourceSessionMeta, SourceSessions, StartNativeCaptureRequest,
};
use tokio::sync::mpsc;

#[test]
fn validate_start_request_rejects_system_audio_when_not_supported() {
    let request = StartNativeCaptureRequest {
        capture_screen: true,
        capture_microphone: false,
        capture_system_audio: true,
    };
    let support = CaptureSupportResponse {
        platform: "macos".to_string(),
        native_capture_supported: true,
        supported_sources: CaptureSources {
            screen: true,
            microphone: true,
            system_audio: false,
        },
    };

    let error = validate_start_request(&request, &support).expect_err("must reject system audio");
    assert_eq!(error.code, "system_audio_unsupported");
}

// A system-audio-only session is allowed: the tap needs no screen, and refusing
// one would be new coupling written into a decoupling project (ADR 0052).
#[test]
fn validate_start_request_allows_a_system_audio_only_session() {
    let request = StartNativeCaptureRequest {
        capture_screen: false,
        capture_microphone: false,
        capture_system_audio: true,
    };
    let support = CaptureSupportResponse {
        platform: "macos".to_string(),
        native_capture_supported: true,
        supported_sources: CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        },
    };

    let sources = validate_start_request(&request, &support)
        .expect("system audio alone is a valid recording session");

    assert_eq!(
        sources,
        CaptureSources {
            screen: false,
            microphone: false,
            system_audio: true,
        }
    );
}

#[test]
fn mark_runtime_session_stopped_preserves_session_metadata() {
    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: false,
        }),
        current_segment_sources: None,
        output_files: Some(CaptureOutputFiles {
            screen_file: Some("/tmp/screen.mov".to_string()),
            screen_files: vec!["/tmp/screen.mov".to_string()],
            microphone_file: Some("/tmp/microphone.m4a".to_string()),
            microphone_files: vec!["/tmp/microphone.m4a".to_string()],
            system_audio_file: None,
            system_audio_files: Vec::new(),
        }),
        current_segment_output_files: None,
        current_segment_index: 1,
        screen_frame_rate: 5.0,
        screen_resolution: ScreenResolution::Preset {
            preset: ScreenResolutionPreset::Original,
        },
        effective_screen_bitrate_bps: None,
        microphone_device_id_for_capture: None,
        segment_loop_control: None,
        capture_clock: None,
        segment_schedule: None,
        segment_planner: None,
        microphone_planner: None,
        system_audio_planner: None,
        frame_artifact_tx: None,
        #[cfg(target_os = "macos")]
        recording_file: Some("/tmp/screen.mov".to_string()),
        #[cfg(target_os = "macos")]
        microphone_recording_file: Some("/tmp/microphone.mov".to_string()),
        #[cfg(target_os = "macos")]
        system_audio_recording_file: None,
        #[cfg(target_os = "macos")]
        active_screen_session: None,
        #[cfg(target_os = "macos")]
        active_microphone_session: None,
        active_system_audio_session: None,
        capture_suspension: None,
        #[cfg(target_os = "macos")]
        free_space_probe: None,
        away_probe: None,
        test_microphone_device: None,
        runtime_controller: RuntimeController::default(),
        runtime_state: RuntimeState::Idle,
        inactivity: InactivityState::default(),
        user_capture_paused: false,
        microphone_away_hold: false,
        system_audio_away_hold: false,
        system_audio_tap_started_at_unix_ms: None,
        microphone_waiting_for_device: false,
        microphone_lent_out: false,
        microphone_vad: Default::default(),
        source_sessions: Some(SourceSessions {
            screen: Some(SourceSessionMeta {
                session_id: "session-1".to_string(),
                started_at_unix_ms: 123,
            }),
            microphone: Some(SourceSessionMeta {
                session_id: "session-mic".to_string(),
                started_at_unix_ms: 123,
            }),
            system_audio: None,
        }),
        recording_started_at_unix_ms: None,
    };

    mark_runtime_session_stopped(&mut runtime);

    assert!(!runtime.is_running);
    assert_eq!(
        runtime
            .source_sessions
            .as_ref()
            .and_then(|sessions| sessions.microphone.as_ref()),
        Some(&SourceSessionMeta {
            session_id: "session-mic".to_string(),
            started_at_unix_ms: 123,
        })
    );
    assert!(runtime.requested_sources.is_some());
    assert!(runtime.output_files.is_some());
    assert!(runtime.frame_artifact_tx.is_none());
}

#[test]
fn user_paused_screen_session_still_reports_running_for_resume_controls() {
    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: false,
        }),
        user_capture_paused: true,
        ..Default::default()
    };
    #[cfg(target_os = "macos")]
    {
        runtime.recording_file = None;
        runtime.active_screen_session = None;
    }

    let session = session_from_runtime(&runtime);

    assert!(session.is_running);
    assert!(session.is_user_paused);
}

#[test]
fn stopped_session_from_runtime_preserves_finalized_metadata() {
    let runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        }),
        current_segment_sources: None,
        output_files: Some(CaptureOutputFiles {
            screen_file: Some("/tmp/screen.mov".to_string()),
            screen_files: vec!["/tmp/screen.mov".to_string()],
            microphone_file: Some("/tmp/microphone.m4a".to_string()),
            microphone_files: vec!["/tmp/microphone.m4a".to_string()],
            system_audio_file: Some("/tmp/system-audio.m4a".to_string()),
            system_audio_files: vec!["/tmp/system-audio.m4a".to_string()],
        }),
        current_segment_output_files: None,
        current_segment_index: 1,
        screen_frame_rate: 5.0,
        screen_resolution: ScreenResolution::Preset {
            preset: ScreenResolutionPreset::Original,
        },
        effective_screen_bitrate_bps: None,
        microphone_device_id_for_capture: None,
        segment_loop_control: None,
        capture_clock: None,
        segment_schedule: None,
        segment_planner: None,
        microphone_planner: None,
        system_audio_planner: None,
        frame_artifact_tx: None,
        #[cfg(target_os = "macos")]
        recording_file: None,
        #[cfg(target_os = "macos")]
        microphone_recording_file: None,
        #[cfg(target_os = "macos")]
        system_audio_recording_file: None,
        #[cfg(target_os = "macos")]
        active_screen_session: None,
        #[cfg(target_os = "macos")]
        active_microphone_session: None,
        active_system_audio_session: None,
        capture_suspension: None,
        #[cfg(target_os = "macos")]
        free_space_probe: None,
        away_probe: None,
        test_microphone_device: None,
        runtime_controller: RuntimeController::default(),
        runtime_state: RuntimeState::Idle,
        inactivity: InactivityState::default(),
        user_capture_paused: false,
        microphone_away_hold: false,
        system_audio_away_hold: false,
        system_audio_tap_started_at_unix_ms: None,
        microphone_waiting_for_device: false,
        microphone_lent_out: false,
        microphone_vad: Default::default(),
        source_sessions: Some(SourceSessions {
            screen: Some(SourceSessionMeta {
                session_id: "session-1".to_string(),
                started_at_unix_ms: 123,
            }),
            microphone: Some(SourceSessionMeta {
                session_id: "session-mic".to_string(),
                started_at_unix_ms: 123,
            }),
            system_audio: Some(SourceSessionMeta {
                session_id: "session-system".to_string(),
                started_at_unix_ms: 123,
            }),
        }),
        recording_started_at_unix_ms: None,
    };

    let session = stopped_session_from_runtime(&runtime);

    assert!(!session.is_running);
    assert_eq!(
        session.source_sessions,
        Some(SourceSessions {
            screen: Some(SourceSessionMeta {
                session_id: "session-1".to_string(),
                started_at_unix_ms: 123,
            }),
            microphone: Some(SourceSessionMeta {
                session_id: "session-mic".to_string(),
                started_at_unix_ms: 123,
            }),
            system_audio: Some(SourceSessionMeta {
                session_id: "session-system".to_string(),
                started_at_unix_ms: 123,
            }),
        })
    );
    assert!(session
        .requested_sources
        .as_ref()
        .is_some_and(|sources| { sources.screen && sources.microphone && sources.system_audio }));
}

#[test]
fn reset_runtime_after_start_error_clears_per_source_start_state() {
    let mut runtime_controller = RuntimeController::default();
    runtime_controller
        .apply(RuntimeSignal::StartRequested)
        .expect("idle runtime should enter starting state");

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
        segment_planner: Some(SegmentPlanner::with_date_prefix(
            "/tmp/native-capture-tests",
            "screen-session",
            "2026/04/22",
        )),
        microphone_planner: Some(SegmentPlanner::with_date_prefix(
            "/tmp/native-capture-tests",
            "microphone-session",
            "2026/04/22",
        )),
        system_audio_planner: Some(SegmentPlanner::with_date_prefix(
            "/tmp/native-capture-tests",
            "system-audio-session",
            "2026/04/22",
        )),
        runtime_state: runtime_controller.state(),
        runtime_controller,
        source_sessions: Some(SourceSessions {
            screen: Some(SourceSessionMeta {
                session_id: "screen-session".to_string(),
                started_at_unix_ms: 123,
            }),
            microphone: Some(SourceSessionMeta {
                session_id: "microphone-session".to_string(),
                started_at_unix_ms: 123,
            }),
            system_audio: Some(SourceSessionMeta {
                session_id: "system-audio-session".to_string(),
                started_at_unix_ms: 123,
            }),
        }),
        ..Default::default()
    };

    reset_runtime_after_start_error(&mut runtime);

    assert!(!runtime.is_running);
    assert!(runtime.segment_planner.is_none());
    assert!(runtime.microphone_planner.is_none());
    assert!(runtime.system_audio_planner.is_none());
    assert!(runtime.source_sessions.is_none());
    assert!(runtime.requested_sources.is_none());
    assert_eq!(runtime.current_segment_index, 0);
    assert_eq!(runtime.runtime_state, RuntimeState::Idle);
}

#[cfg(target_os = "macos")]
#[test]
fn ensure_dedicated_audio_planners_seed_from_source_sessions() {
    let mut runtime = NativeCaptureRuntime {
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        }),
        segment_planner: Some(SegmentPlanner::with_date_prefix(
            "/tmp/native-capture-tests",
            "screen-session",
            "2026/04/22",
        )),
        source_sessions: Some(SourceSessions {
            screen: Some(SourceSessionMeta {
                session_id: "screen-session".to_string(),
                started_at_unix_ms: 123,
            }),
            microphone: Some(SourceSessionMeta {
                session_id: "microphone-session".to_string(),
                started_at_unix_ms: 123,
            }),
            system_audio: Some(SourceSessionMeta {
                session_id: "system-audio-session".to_string(),
                started_at_unix_ms: 123,
            }),
        }),
        ..Default::default()
    };

    let microphone_planner = ensure_microphone_planner_for_runtime(&mut runtime, "testing")
        .expect("microphone planner seeding should succeed")
        .expect("microphone planner should be returned");
    let system_audio_planner = ensure_system_audio_planner_for_runtime(&mut runtime, "testing")
        .expect("system audio planner seeding should succeed")
        .expect("system audio planner should be returned");

    assert_eq!(
        microphone_planner.save_root_dir(),
        "/tmp/native-capture-tests"
    );
    assert_eq!(microphone_planner.session_id(), "microphone-session");
    assert_eq!(microphone_planner.date_prefix(), "2026/04/22");
    assert_eq!(
        system_audio_planner.save_root_dir(),
        "/tmp/native-capture-tests"
    );
    assert_eq!(system_audio_planner.session_id(), "system-audio-session");
    assert_eq!(system_audio_planner.date_prefix(), "2026/04/22");
    assert_ne!(microphone_planner.session_id(), "screen-session");
    assert_ne!(system_audio_planner.session_id(), "screen-session");
}

#[cfg(target_os = "macos")]
#[test]
fn ensure_system_audio_planner_persists_missing_source_session_from_existing_planner() {
    let mut runtime = NativeCaptureRuntime {
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: true,
        }),
        segment_planner: Some(SegmentPlanner::with_date_prefix(
            "/tmp/native-capture-tests",
            "screen-session",
            "2026/04/22",
        )),
        system_audio_planner: Some(SegmentPlanner::with_date_prefix(
            "/tmp/native-capture-tests",
            "system-audio-session",
            "2026/04/22",
        )),
        source_sessions: None,
        ..Default::default()
    };

    let planner = ensure_system_audio_planner_for_runtime(&mut runtime, "testing")
        .expect("existing planner should be returned")
        .expect("system audio planner should exist");

    assert_eq!(planner.session_id(), "system-audio-session");
    assert_eq!(
        runtime
            .source_sessions
            .as_ref()
            .and_then(|sessions| sessions.system_audio.as_ref())
            .map(|session| session.session_id.as_str()),
        Some("system-audio-session")
    );
}

#[cfg(target_os = "macos")]
#[test]
fn microphone_planner_for_runtime_does_not_fall_back_to_screen_planner() {
    let runtime = NativeCaptureRuntime {
        segment_planner: Some(SegmentPlanner::with_date_prefix(
            "/tmp/native-capture-tests",
            "screen-session",
            "2026/04/22",
        )),
        ..Default::default()
    };

    assert!(microphone_planner_for_runtime(&runtime).is_none());
}

#[cfg(target_os = "macos")]
#[test]
fn system_audio_planner_for_runtime_does_not_fall_back_to_screen_planner() {
    let runtime = NativeCaptureRuntime {
        segment_planner: Some(SegmentPlanner::with_date_prefix(
            "/tmp/native-capture-tests",
            "screen-session",
            "2026/04/22",
        )),
        ..Default::default()
    };

    assert!(system_audio_planner_for_runtime(&runtime).is_none());
}

#[test]
fn should_rotate_segment_only_after_boundary_crossing() {
    assert!(!should_rotate_segment(1, 1));
    assert!(should_rotate_segment(1, 2));
    assert!(should_rotate_segment(3, 5));
}

#[test]
fn should_recover_from_segment_finalize_error_accepts_wrapped_missing_screen_output() {
    let error = capture_writers::aggregate_output_processing_failures(vec![
        "screen output missing: expected screen recording file at /tmp/screen.mov".to_string(),
    ])
    .expect_err("single missing screen output failure should aggregate");

    assert!(should_recover_from_segment_finalize_error(&error));
}

#[test]
fn should_recover_from_segment_finalize_error_rejects_missing_screen_output_with_extra_failures() {
    let error = capture_writers::aggregate_output_processing_failures(vec![
        "screen output missing: expected screen recording file at /tmp/screen.mov".to_string(),
        "system audio output conversion failed: missing source recording".to_string(),
    ])
    .expect_err("multiple output failures should aggregate");

    assert!(!should_recover_from_segment_finalize_error(&error));
}

#[test]
fn should_recover_from_segment_finalize_error_accepts_missing_screen_output_without_path() {
    let error = capture_writers::aggregate_output_processing_failures(vec![
        "screen output missing: expected screen recording file".to_string(),
    ])
    .expect_err("single missing screen output failure should aggregate");

    assert!(should_recover_from_segment_finalize_error(&error));
}

#[test]
fn mark_runtime_session_stopped_clears_frame_artifact_worker() {
    let (tx, _rx) = mpsc::channel::<FrameArtifactMessage>(1);
    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        frame_artifact_tx: Some(tx),
        ..Default::default()
    };

    mark_runtime_session_stopped(&mut runtime);

    assert!(runtime.frame_artifact_tx.is_none());
}

#[cfg(target_os = "macos")]
#[test]
fn session_from_runtime_reports_running_during_privacy_suspension_with_live_microphone() {
    let privacy_error = CaptureErrorResponse {
        code: "privacy_filter_apply_failed".to_string(),
        message: "privacy filter application failed".to_string(),
    };

    let runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        }),
        current_segment_sources: Some(CaptureSources {
            screen: false,
            microphone: true,
            system_audio: false,
        }),
        current_segment_output_files: Some(CaptureOutputFiles {
            screen_file: None,
            screen_files: Vec::new(),
            microphone_file: Some("/tmp/privacy-suspended-microphone.m4a".to_string()),
            microphone_files: vec!["/tmp/privacy-suspended-microphone.m4a".to_string()],
            system_audio_file: None,
            system_audio_files: Vec::new(),
        }),
        recording_file: None,
        microphone_recording_file: Some("/tmp/privacy-suspended-microphone.m4a".to_string()),
        system_audio_recording_file: None,
        active_screen_session: None,
        active_microphone_session: None,
        active_system_audio_session: None,
        capture_suspension: Some(CaptureSuspension::with_kind(
            CaptureSuspensionKind::PrivacyFilter,
            &privacy_error,
        )),
        ..Default::default()
    };

    let session = crate::native_capture::runtime::session_from_runtime(&runtime);

    assert!(
        session.is_running,
        "privacy suspension intentionally stops screen/system-audio while microphone continues"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn segment_planner_date_refresh_updates_all_runtime_planners() {
    let mut runtime = NativeCaptureRuntime {
        segment_planner: Some(SegmentPlanner::with_date_prefix(
            "/tmp/native-capture-tests",
            "screen-session",
            "2026/04/16",
        )),
        microphone_planner: Some(SegmentPlanner::with_date_prefix(
            "/tmp/native-capture-tests",
            "microphone-session",
            "2026/04/16",
        )),
        system_audio_planner: Some(SegmentPlanner::with_date_prefix(
            "/tmp/native-capture-tests",
            "system-audio-session",
            "2026/04/16",
        )),
        ..Default::default()
    };

    let refreshed = crate::native_capture::runtime::refresh_runtime_planner_dates(&mut runtime);

    assert_eq!(
        runtime
            .segment_planner
            .as_ref()
            .map(|planner| planner.date_prefix()),
        Some(refreshed.as_str())
    );
    assert_eq!(
        runtime
            .microphone_planner
            .as_ref()
            .map(|planner| planner.date_prefix()),
        Some(refreshed.as_str())
    );
    assert_eq!(
        runtime
            .system_audio_planner
            .as_ref()
            .map(|planner| planner.date_prefix()),
        Some(refreshed.as_str())
    );
    assert_eq!(refreshed.split('/').count(), 3);
}

#[test]
fn recording_start_is_reported_while_running_and_cleared_on_stop() {
    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        recording_started_at_unix_ms: Some(123),
        ..Default::default()
    };
    assert_eq!(
        session_from_runtime(&runtime).recording_started_at_unix_ms,
        Some(123)
    );

    mark_runtime_session_stopped(&mut runtime);

    assert_eq!(runtime.recording_started_at_unix_ms, None);
    assert_eq!(
        session_from_runtime(&runtime).recording_started_at_unix_ms,
        None
    );
    assert_eq!(
        stopped_session_from_runtime(&runtime).recording_started_at_unix_ms,
        None
    );
}
