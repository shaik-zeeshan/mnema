use super::*;
use capture_types::{
    default_appearance, default_audio_speech_detection_settings,
    default_audio_transcription_settings, default_inactivity_activity_mode,
    default_metadata_settings, default_microphone_vad_adapter, default_privacy_settings,
    default_video_bitrate, InactivityActivityMode, RecordingSettings, ScreenResolution,
    ScreenResolutionPreset,
};

mod audio_pause;

fn empty_audio_activity() -> AudioActivitySourceState {
    AudioActivitySourceState {
        enabled: false,
        idle_ms: None,
        latest_normalized_level: None,
    }
}

fn empty_activity_snapshot() -> ActivitySnapshot {
    ActivitySnapshot {
        system_input_idle_ms: None,
        screen_activity_enabled: false,
        screen_activity_idle_ms: None,
        microphone_activity: empty_audio_activity(),
        system_audio_activity: empty_audio_activity(),
    }
}

fn assert_approx_eq(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 0.000_1);
}

fn inactivity_state_fixture(
    activity_mode: InactivityActivityMode,
    audio_activity_sensitivity: u8,
) -> InactivityState {
    InactivityState {
        enabled: true,
        idle_timeout_seconds: 10,
        microphone_activity_sensitivity: audio_activity_sensitivity,
        system_audio_activity_sensitivity: audio_activity_sensitivity,
        activity_mode,
        last_activity_monotonic_ms: 1_000,
        ..InactivityState::default()
    }
}

#[test]
fn inactivity_state_triggers_screen_pause_after_timeout() {
    let settings = RecordingSettings {
        capture_screen: true,
        capture_microphone: false,
        capture_system_audio: false,
        segment_duration_seconds: 60,
        screen_frame_rate: 30.0,
        screen_resolution: ScreenResolution::Preset {
            preset: ScreenResolutionPreset::Original,
        },
        video_bitrate: default_video_bitrate(),
        save_directory: "/tmp".to_string(),
        auto_start: false,
        developer_options_enabled: false,
        preview_cache_ttl_seconds: capture_types::default_preview_cache_ttl_seconds(),
        follow_timeline_live: false,
        retention_policy: capture_types::default_retention_policy(),
        appearance: default_appearance(),
        time_format: Default::default(),
        ocr: capture_types::default_ocr_settings(),
        transcription: default_audio_transcription_settings(),
        speaker_analysis: capture_types::default_speaker_analysis_settings(),
        audio_speech_detection: default_audio_speech_detection_settings(),
        metadata: default_metadata_settings(),
        privacy: default_privacy_settings(),
        access: capture_types::AccessSettings::default(),
        ai_runtime: capture_types::AiRuntimeSettings::default(),
        user_context: capture_types::UserContextSettings::default(),
        semantic_search: capture_types::default_semantic_search_settings(),
        pause_capture_on_inactivity: true,
        idle_timeout_seconds: 10,
        microphone_activity_sensitivity: 50,
        system_audio_activity_sensitivity: 50,
        microphone_vad_adapter: default_microphone_vad_adapter(),
        inactivity_activity_mode: default_inactivity_activity_mode(),
    };

    let mut state = InactivityState::from_recording_settings(&settings, 1_000);
    assert!(!state.should_pause_screen_for_inactivity(
        10_999,
        ActivitySnapshot {
            system_input_idle_ms: None,
            screen_activity_enabled: true,
            screen_activity_idle_ms: None,
            microphone_activity: empty_audio_activity(),
            system_audio_activity: empty_audio_activity(),
        }
    ));
    assert!(state.should_pause_screen_for_inactivity(
        11_000,
        ActivitySnapshot {
            system_input_idle_ms: None,
            screen_activity_enabled: true,
            screen_activity_idle_ms: None,
            microphone_activity: empty_audio_activity(),
            system_audio_activity: empty_audio_activity(),
        }
    ));
}

#[test]
fn system_idle_is_primary_screen_source_over_reported_activity() {
    let mut state = inactivity_state_fixture(InactivityActivityMode::SystemInputOnly, 50);

    state.last_activity_monotonic_ms = 30_000;
    assert!(state.should_pause_screen_for_inactivity(
        30_500,
        ActivitySnapshot {
            system_input_idle_ms: Some(10_000),
            screen_activity_enabled: true,
            screen_activity_idle_ms: None,
            ..empty_activity_snapshot()
        }
    ));

    state.set_family_paused_states(true, false, false);
    assert!(!state.should_resume_screen_from_inactivity(
        31_000,
        ActivitySnapshot {
            system_input_idle_ms: Some(10_000),
            screen_activity_enabled: true,
            screen_activity_idle_ms: None,
            ..empty_activity_snapshot()
        }
    ));
    assert!(state.should_resume_screen_from_inactivity(
        31_000,
        ActivitySnapshot {
            system_input_idle_ms: Some(2_000),
            screen_activity_enabled: true,
            screen_activity_idle_ms: None,
            ..empty_activity_snapshot()
        }
    ));
}

#[test]
fn screen_resume_waits_for_pause_guard_window() {
    let mut state = inactivity_state_fixture(InactivityActivityMode::SystemInputOrScreen, 50);
    state.set_family_paused_states(true, false, false);
    state.mark_screen_pause_started(20_000);

    let active_snapshot = ActivitySnapshot {
        system_input_idle_ms: Some(0),
        screen_activity_enabled: true,
        screen_activity_idle_ms: Some(0),
        microphone_activity: empty_audio_activity(),
        system_audio_activity: empty_audio_activity(),
    };

    assert!(
        !state.should_resume_screen_from_inactivity(21_999, active_snapshot),
        "screen activity immediately after a soft pause should not churn outputs back on"
    );
    assert!(state.should_resume_screen_from_inactivity(22_000, active_snapshot));

    state.set_family_paused_states(false, false, false);
    assert_eq!(state.screen_paused_at_monotonic_ms, None);
}

#[test]
fn hybrid_mode_screen_policy_uses_less_idle_source() {
    let mut state = inactivity_state_fixture(InactivityActivityMode::SystemInputOrScreen, 50);

    let effective = state.effective_idle_for_snapshot(
        20_000,
        ActivitySnapshot {
            system_input_idle_ms: Some(12_000),
            screen_activity_idle_ms: Some(1_000),
            ..empty_activity_snapshot()
        },
    );

    assert_eq!(effective.source, ActivitySourceKind::ScreenCapture);
    assert_eq!(effective.idle_ms, 1_000);
}

#[test]
fn screen_policy_evaluation_exposes_sources_with_availability_and_idles() {
    let mut state = inactivity_state_fixture(InactivityActivityMode::SystemInputOrScreen, 50);

    let evaluation = state.evaluate_screen_policy_for_snapshot(
        20_000,
        ActivitySnapshot {
            system_input_idle_ms: Some(12_000),
            screen_activity_idle_ms: Some(500),
            ..empty_activity_snapshot()
        },
    );

    assert_eq!(evaluation.sources.len(), 4);
    assert_eq!(evaluation.sources[0].kind, ActivitySourceKind::SystemInput);
    assert!(evaluation.sources[0].enabled);
    assert!(evaluation.sources[0].available);
    assert_eq!(evaluation.sources[0].idle_ms, Some(12_000));
    assert_eq!(
        evaluation.sources[1].kind,
        ActivitySourceKind::ScreenCapture
    );
    assert!(evaluation.sources[1].enabled);
    assert!(evaluation.sources[1].available);
    assert_eq!(evaluation.sources[1].idle_ms, Some(500));
    assert_eq!(
        evaluation.sources[2].kind,
        ActivitySourceKind::MicrophoneCapture
    );
    assert!(!evaluation.sources[2].enabled);
    assert!(!evaluation.sources[2].available);
    assert_approx_eq(
        evaluation.sources[2]
            .activity_threshold
            .expect("microphone threshold should be present"),
        0.08,
    );
    assert_eq!(
        evaluation.sources[3].kind,
        ActivitySourceKind::SystemAudioCapture
    );
    assert!(!evaluation.sources[3].enabled);
    assert!(!evaluation.sources[3].available);
    assert_approx_eq(
        evaluation.sources[3]
            .activity_threshold
            .expect("system-audio threshold should be present"),
        0.026,
    );
    assert_eq!(
        evaluation.effective_idle.source,
        ActivitySourceKind::ScreenCapture
    );
    assert_eq!(evaluation.effective_idle.idle_ms, 500);
}

#[test]
fn fallback_idle_uses_monotonic_elapsed_time() {
    let mut state = inactivity_state_fixture(InactivityActivityMode::SystemInputOrScreen, 50);
    state.last_activity_monotonic_ms = 5_000;

    let effective = state.effective_idle_for_snapshot(
        6_250,
        ActivitySnapshot {
            system_input_idle_ms: None,
            screen_activity_idle_ms: None,
            ..empty_activity_snapshot()
        },
    );

    assert_eq!(effective.source, ActivitySourceKind::InternalFallback);
    assert_eq!(effective.idle_ms, 1_250);
}

#[test]
fn legacy_combined_policy_keeps_existing_lowest_idle_behavior() {
    let mut state =
        inactivity_state_fixture(InactivityActivityMode::SystemInputOrScreenOrAudio, 100);

    let evaluation = state.evaluate_policy_for_snapshot(
        20_000,
        ActivitySnapshot {
            system_input_idle_ms: Some(12_000),
            screen_activity_enabled: true,
            screen_activity_idle_ms: Some(8_000),
            microphone_activity: AudioActivitySourceState {
                enabled: true,
                idle_ms: Some(400),
                latest_normalized_level: Some(0.10),
            },
            system_audio_activity: AudioActivitySourceState {
                enabled: true,
                idle_ms: Some(250),
                latest_normalized_level: Some(0.20),
            },
        },
    );

    assert_eq!(
        evaluation.effective_idle.source,
        ActivitySourceKind::SystemAudioCapture
    );
    assert_eq!(evaluation.effective_idle.idle_ms, 250);
    assert!(evaluation.sources[2].available);
    assert!(evaluation.sources[3].available);
    assert_approx_eq(
        evaluation.sources[2]
            .activity_threshold
            .expect("microphone threshold should be present"),
        0.01,
    );
    assert_approx_eq(
        evaluation.sources[3]
            .activity_threshold
            .expect("system-audio threshold should be present"),
        0.002,
    );
}

#[test]
fn screen_and_audio_policy_evaluations_are_independent_in_full_mode() {
    let mut state =
        inactivity_state_fixture(InactivityActivityMode::SystemInputOrScreenOrAudio, 100);

    let evaluations = state.evaluate_policies_for_snapshot(
        20_000,
        ActivitySnapshot {
            system_input_idle_ms: Some(12_000),
            screen_activity_enabled: true,
            screen_activity_idle_ms: Some(8_000),
            microphone_activity: AudioActivitySourceState {
                enabled: true,
                idle_ms: Some(400),
                latest_normalized_level: Some(0.10),
            },
            system_audio_activity: AudioActivitySourceState {
                enabled: true,
                idle_ms: Some(250),
                latest_normalized_level: Some(0.20),
            },
        },
    );

    assert_eq!(
        evaluations.screen.effective_idle.source,
        ActivitySourceKind::ScreenCapture
    );
    assert_eq!(evaluations.screen.effective_idle.idle_ms, 8_000);
    // Microphone policy ignores system input and uses microphone activity only.
    assert_eq!(
        evaluations.microphone.effective_idle.source,
        ActivitySourceKind::MicrophoneCapture
    );
    assert_eq!(evaluations.microphone.effective_idle.idle_ms, 400);
    // System audio policy ignores system input and uses system-audio activity only.
    assert_eq!(
        evaluations.system_audio.effective_idle.source,
        ActivitySourceKind::SystemAudioCapture
    );
    assert_eq!(evaluations.system_audio.effective_idle.idle_ms, 250);
    assert!(evaluations.microphone.sources[2].available);
    assert!(evaluations.system_audio.sources[3].available);
    assert_approx_eq(
        evaluations.microphone.sources[2]
            .activity_threshold
            .expect("microphone threshold should be present"),
        0.01,
    );
    assert_approx_eq(
        evaluations.system_audio.sources[3]
            .activity_threshold
            .expect("system-audio threshold should be present"),
        0.002,
    );
}

#[test]
fn microphone_policy_ignores_system_audio_source() {
    let mut state =
        inactivity_state_fixture(InactivityActivityMode::SystemInputOrScreenOrAudio, 100);

    let evaluations = state.evaluate_policies_for_snapshot(
        20_000,
        ActivitySnapshot {
            system_input_idle_ms: Some(12_000),
            screen_activity_enabled: true,
            screen_activity_idle_ms: Some(11_000),
            microphone_activity: AudioActivitySourceState {
                enabled: true,
                idle_ms: Some(500),
                latest_normalized_level: Some(0.10),
            },
            system_audio_activity: AudioActivitySourceState {
                enabled: true,
                idle_ms: Some(100),
                latest_normalized_level: Some(0.50),
            },
        },
    );

    // Microphone policy should NOT consider system audio source
    assert_eq!(
        evaluations.microphone.effective_idle.source,
        ActivitySourceKind::MicrophoneCapture
    );
    assert_eq!(evaluations.microphone.effective_idle.idle_ms, 500);

    // System audio policy should NOT consider microphone source
    assert_eq!(
        evaluations.system_audio.effective_idle.source,
        ActivitySourceKind::SystemAudioCapture
    );
    assert_eq!(evaluations.system_audio.effective_idle.idle_ms, 100);
}

#[test]
fn audio_policy_ignores_screen_source_when_audio_inputs_are_unavailable() {
    let mut state = inactivity_state_fixture(InactivityActivityMode::SystemInputOrScreenOrAudio, 0);

    let evaluations = state.evaluate_policies_for_snapshot(
        20_000,
        ActivitySnapshot {
            system_input_idle_ms: Some(12_000),
            screen_activity_enabled: true,
            screen_activity_idle_ms: Some(11_000),
            microphone_activity: AudioActivitySourceState {
                enabled: true,
                idle_ms: Some(500),
                latest_normalized_level: Some(0.14),
            },
            system_audio_activity: AudioActivitySourceState {
                enabled: false,
                idle_ms: Some(400),
                latest_normalized_level: Some(1.0),
            },
        },
    );

    assert_eq!(
        evaluations.screen.effective_idle.source,
        ActivitySourceKind::ScreenCapture
    );
    assert_eq!(evaluations.screen.effective_idle.idle_ms, 11_000);
    // Microphone policy ignores system input. The microphone is below threshold
    // (0.14 < 0.15), so it is not available and falls back to internal idle.
    assert_eq!(
        evaluations.microphone.effective_idle.source,
        ActivitySourceKind::InternalFallback
    );
    assert_eq!(evaluations.microphone.effective_idle.idle_ms, 19_000);
    assert!(!evaluations.microphone.sources[2].available);
    assert_eq!(evaluations.microphone.sources[2].idle_ms, None);
    assert_eq!(
        evaluations.microphone.sources[2].latest_normalized_level,
        Some(0.14)
    );
    assert_approx_eq(
        evaluations.microphone.sources[2]
            .activity_threshold
            .expect("microphone threshold should be present"),
        0.15,
    );
    // System audio policy ignores system input, and disabled system audio falls
    // back to internal idle.
    assert_eq!(
        evaluations.system_audio.effective_idle.source,
        ActivitySourceKind::InternalFallback
    );
    assert_eq!(evaluations.system_audio.effective_idle.idle_ms, 19_000);
    assert!(!evaluations.system_audio.sources[3].enabled);
    assert!(!evaluations.system_audio.sources[3].available);
    assert_eq!(evaluations.system_audio.sources[3].idle_ms, None);
    assert_eq!(
        evaluations.system_audio.sources[3].latest_normalized_level,
        Some(1.0)
    );
}

#[test]
fn screen_and_microphone_pause_decisions_can_diverge_for_same_snapshot() {
    let mut state =
        inactivity_state_fixture(InactivityActivityMode::SystemInputOrScreenOrAudio, 100);

    let snapshot = ActivitySnapshot {
        system_input_idle_ms: None,
        screen_activity_enabled: true,
        screen_activity_idle_ms: Some(11_000),
        microphone_activity: AudioActivitySourceState {
            enabled: true,
            idle_ms: Some(250),
            latest_normalized_level: Some(0.2),
        },
        system_audio_activity: empty_audio_activity(),
    };

    assert!(state.should_pause_screen_for_inactivity(20_000, snapshot));
    assert!(!state.should_pause_microphone_for_inactivity(20_000, snapshot));
}

#[test]
fn microphone_and_system_audio_pause_decisions_can_diverge() {
    let mut state =
        inactivity_state_fixture(InactivityActivityMode::SystemInputOrScreenOrAudio, 100);

    let snapshot = ActivitySnapshot {
        system_input_idle_ms: Some(15_000),
        screen_activity_enabled: true,
        screen_activity_idle_ms: Some(15_000),
        microphone_activity: AudioActivitySourceState {
            enabled: true,
            idle_ms: Some(250),
            latest_normalized_level: Some(0.2),
        },
        system_audio_activity: AudioActivitySourceState {
            enabled: true,
            idle_ms: Some(12_000),
            latest_normalized_level: Some(0.0),
        },
    };

    // Microphone has recent activity (250ms), should NOT pause
    assert!(!state.should_pause_microphone_for_inactivity(20_000, snapshot));
    // System audio has old activity (12_000ms > 10_000 timeout), should pause
    // But system_input is at 15_000ms, and system_audio_source idle will be
    // from last threshold-qualified activity. First eval seeds the state.
    let eval = state.evaluate_system_audio_policy_for_snapshot(20_000, snapshot);
    // system_audio level is 0.0, below threshold of 0.05, so not threshold-qualified
    // system_input at 15_000ms > 10_000 timeout
    assert!(eval.effective_idle.idle_ms >= state.idle_timeout_ms());
}

#[test]
fn legacy_combined_pause_decision_still_uses_all_sources() {
    let mut state =
        inactivity_state_fixture(InactivityActivityMode::SystemInputOrScreenOrAudio, 100);

    let snapshot = ActivitySnapshot {
        system_input_idle_ms: None,
        screen_activity_enabled: true,
        screen_activity_idle_ms: Some(11_000),
        microphone_activity: AudioActivitySourceState {
            enabled: true,
            idle_ms: Some(250),
            latest_normalized_level: Some(0.2),
        },
        system_audio_activity: empty_audio_activity(),
    };

    assert!(!state.should_pause_for_inactivity(20_000, snapshot));

    state.is_paused = true;

    assert!(state.should_resume_from_inactivity(20_000, snapshot));
}
