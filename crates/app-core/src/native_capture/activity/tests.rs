use super::*;
use crate::native_capture_inactivity::{ActivitySourceSample, EffectiveIdle};
use capture_types::CaptureSources;
use capture_vad::MicrophoneVadRuntime;

#[test]
fn audio_projection_keeps_raw_samples_separate_from_qualified_policy_fields() {
    let projection = idle_debug_audio_projection(
        &ActivityPolicyEvaluation {
            effective_idle: EffectiveIdle {
                source: ActivitySourceKind::MicrophoneCapture,
                idle_ms: 250,
            },
            sources: vec![
                ActivitySourceSample {
                    kind: ActivitySourceKind::MicrophoneCapture,
                    enabled: true,
                    available: true,
                    idle_ms: Some(250),
                    latest_normalized_level: Some(0.4),
                    activity_threshold: Some(0.3),
                },
                ActivitySourceSample {
                    kind: ActivitySourceKind::SystemAudioCapture,
                    enabled: false,
                    available: false,
                    idle_ms: None,
                    latest_normalized_level: Some(0.1),
                    activity_threshold: Some(0.2),
                },
            ],
        },
        RawActivityReading {
            last_unix_ms: Some(10),
            level: Some(0.9),
        },
        RawActivityReading {
            last_unix_ms: Some(20),
            level: Some(0.8),
        },
    );

    assert_eq!(projection.microphone.raw_sample.last_unix_ms, Some(10));
    assert_eq!(projection.microphone.raw_sample.level, Some(0.9));
    assert!(projection.microphone.qualified.enabled);
    assert_eq!(projection.microphone.qualified.idle_ms, Some(250));
    assert_eq!(
        projection.microphone.qualified.activity_threshold,
        Some(0.3)
    );

    assert_eq!(projection.system_audio.raw_sample.last_unix_ms, Some(20));
    assert_eq!(projection.system_audio.raw_sample.level, Some(0.8));
    assert!(!projection.system_audio.qualified.enabled);
    assert_eq!(projection.system_audio.qualified.idle_ms, None);
    assert_eq!(
        projection.system_audio.qualified.activity_threshold,
        Some(0.2)
    );
}

#[test]
fn current_activity_snapshot_marks_audio_sources_enabled_from_requested_sources() {
    let _guard = audio_activity_state_test_guard();
    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        }),
        ..Default::default()
    };

    let snapshot = current_activity_snapshot(&mut runtime);

    assert!(snapshot.microphone_activity.enabled);
    assert!(snapshot.system_audio_activity.enabled);
}

#[test]
fn current_activity_snapshot_for_debug_does_not_consume_microphone_vad_speech_pulse() {
    let _guard = audio_activity_state_test_guard();
    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: false,
            microphone: true,
            system_audio: false,
        }),
        microphone_vad: MicrophoneVadRuntime::new(capture_types::MicrophoneVadAdapter::Webrtc),
        ..Default::default()
    };
    let samples = voiced_like_frame_16khz_30ms();
    runtime
        .microphone_vad
        .process_pcm_frame(MicrophonePcmVadFrame {
            samples: &samples,
            sample_rate_hz: 16_000,
            captured_at_unix_ms: 1_000,
            normalized_peak_level: 0.7,
        })
        .expect("voiced-like frame should produce a VAD decision");

    let debug_snapshot = current_activity_snapshot_for_debug(&mut runtime);
    assert_eq!(
        debug_snapshot.microphone_activity.latest_normalized_level,
        Some(1.0)
    );

    let policy_snapshot = current_activity_snapshot(&mut runtime);
    assert_eq!(
        policy_snapshot.microphone_activity.latest_normalized_level,
        Some(1.0),
        "debug polling must not consume the one-shot VAD speech pulse before policy evaluation"
    );

    let next_policy_snapshot = current_activity_snapshot(&mut runtime);
    assert_eq!(
        next_policy_snapshot
            .microphone_activity
            .latest_normalized_level,
        None,
        "policy evaluation still consumes the one-shot pulse after observing it"
    );
}

fn voiced_like_frame_16khz_30ms() -> Vec<i16> {
    (0..480)
        .map(|sample_index| {
            let phase = sample_index % 160;
            let envelope = if phase < 80 { phase } else { 160 - phase };
            let carrier = if sample_index % 32 < 16 { 1 } else { -1 };
            (carrier * envelope as i32 * 220) as i16
        })
        .collect()
}

/// Serializes the tests that read the process-global audio activity peaks.
/// `current_activity_snapshot` *drains* them, so two such tests running in
/// parallel consume each other's sample.
fn audio_activity_state_test_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(target_os = "macos")]
#[test]
fn current_activity_snapshot_for_debug_does_not_drain_system_audio_peak() {
    // Debug should observe the peak without draining it, and the next normal
    // take should still receive the same peak.
    let _guard = audio_activity_state_test_guard();
    capture_system_audio::reset_system_audio_activity();
    capture_system_audio::record_system_audio_activity_for_tests(0.20, 10_000, 20_000);

    let mut runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: true,
        }),
        ..Default::default()
    };

    let debug_snapshot = current_activity_snapshot_for_debug(&mut runtime);

    assert_eq!(
        debug_snapshot.system_audio_activity.latest_normalized_level,
        Some(0.20)
    );
    assert_eq!(
        capture_system_audio::take_system_audio_activity_window_peak_level(),
        Some(0.20)
    );

    capture_system_audio::reset_system_audio_activity();
}

#[test]
fn idle_debug_activity_sources_include_audio_fields() {
    let policy = ActivityPolicyEvaluation {
        effective_idle: crate::native_capture_inactivity::EffectiveIdle {
            source: crate::native_capture_inactivity::ActivitySourceKind::MicrophoneCapture,
            idle_ms: 250,
        },
        sources: vec![crate::native_capture_inactivity::ActivitySourceSample {
            kind: crate::native_capture_inactivity::ActivitySourceKind::MicrophoneCapture,
            enabled: true,
            available: true,
            idle_ms: Some(250),
            latest_normalized_level: Some(0.35),
            activity_threshold: Some(0.4),
        }],
    };

    let sources = idle_debug_activity_sources(&policy);

    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].kind, "microphone_capture");
    assert!(sources[0].enabled);
    assert!(sources[0].available);
    assert_eq!(sources[0].idle_ms, Some(250));
    assert_eq!(sources[0].latest_normalized_level, Some(0.35));
    assert_eq!(sources[0].activity_threshold, Some(0.4));
    assert!(sources[0].selected);
}

#[test]
fn idle_debug_family_fields_reflect_independent_screen_and_audio_evaluations() {
    use crate::native_capture_inactivity::{
        ActivityPolicyEvaluation, ActivityPolicyEvaluations, ActivitySourceKind, EffectiveIdle,
        InactivityState,
    };

    let policies = ActivityPolicyEvaluations {
        screen: ActivityPolicyEvaluation {
            effective_idle: EffectiveIdle {
                source: ActivitySourceKind::ScreenCapture,
                idle_ms: 8_000,
            },
            sources: vec![],
        },
        microphone: ActivityPolicyEvaluation {
            effective_idle: EffectiveIdle {
                source: ActivitySourceKind::MicrophoneCapture,
                idle_ms: 250,
            },
            sources: vec![],
        },
        system_audio: ActivityPolicyEvaluation {
            effective_idle: EffectiveIdle {
                source: ActivitySourceKind::MicrophoneCapture,
                idle_ms: 250,
            },
            sources: vec![],
        },
    };

    let inactivity = InactivityState::default();

    let fields = idle_debug_family_fields(&policies, &inactivity);

    assert_eq!(fields.screen_effective_idle_ms, 8_000);
    assert_eq!(fields.screen_effective_idle_source, "screen_capture");
    assert_eq!(fields.microphone_effective_idle_ms, 250);
    assert_eq!(
        fields.microphone_effective_idle_source,
        "microphone_capture"
    );
    assert!(!fields.screen_paused);
    assert!(!fields.microphone_paused);
    assert!(!fields.system_audio_paused);
}

#[test]
fn idle_debug_family_fields_show_screen_paused_audio_active() {
    use crate::native_capture_inactivity::{
        ActivityPolicyEvaluation, ActivityPolicyEvaluations, ActivitySourceKind, EffectiveIdle,
        InactivityState,
    };

    let policies = ActivityPolicyEvaluations {
        screen: ActivityPolicyEvaluation {
            effective_idle: EffectiveIdle {
                source: ActivitySourceKind::InternalFallback,
                idle_ms: 15_000,
            },
            sources: vec![],
        },
        microphone: ActivityPolicyEvaluation {
            effective_idle: EffectiveIdle {
                source: ActivitySourceKind::SystemAudioCapture,
                idle_ms: 100,
            },
            sources: vec![],
        },
        system_audio: ActivityPolicyEvaluation {
            effective_idle: EffectiveIdle {
                source: ActivitySourceKind::SystemAudioCapture,
                idle_ms: 100,
            },
            sources: vec![],
        },
    };

    let mut inactivity = InactivityState {
        enabled: true,
        idle_timeout_seconds: 10,
        ..InactivityState::default()
    };
    inactivity.set_family_paused_states(true, false, false);

    let fields = idle_debug_family_fields(&policies, &inactivity);

    assert!(fields.screen_paused);
    assert!(!fields.microphone_paused);
    assert!(!fields.system_audio_paused);
    assert_eq!(fields.screen_effective_idle_source, "internal_fallback");
    assert_eq!(
        fields.microphone_effective_idle_source,
        "system_audio_capture"
    );
}

#[test]
fn idle_debug_family_fields_show_audio_paused_screen_active() {
    use crate::native_capture_inactivity::{
        ActivityPolicyEvaluation, ActivityPolicyEvaluations, ActivitySourceKind, EffectiveIdle,
        InactivityState,
    };

    let policies = ActivityPolicyEvaluations {
        screen: ActivityPolicyEvaluation {
            effective_idle: EffectiveIdle {
                source: ActivitySourceKind::ScreenCapture,
                idle_ms: 500,
            },
            sources: vec![],
        },
        microphone: ActivityPolicyEvaluation {
            effective_idle: EffectiveIdle {
                source: ActivitySourceKind::SystemInput,
                idle_ms: 12_000,
            },
            sources: vec![],
        },
        system_audio: ActivityPolicyEvaluation {
            effective_idle: EffectiveIdle {
                source: ActivitySourceKind::SystemInput,
                idle_ms: 12_000,
            },
            sources: vec![],
        },
    };

    let mut inactivity = InactivityState {
        enabled: true,
        idle_timeout_seconds: 10,
        ..InactivityState::default()
    };
    inactivity.set_family_paused_states(false, true, true);

    let fields = idle_debug_family_fields(&policies, &inactivity);

    assert!(!fields.screen_paused);
    assert!(fields.microphone_paused);
    assert!(fields.system_audio_paused);
    assert_eq!(fields.screen_effective_idle_ms, 500);
    assert_eq!(fields.microphone_effective_idle_ms, 12_000);
}

#[test]
fn idle_debug_family_fields_both_paused() {
    use crate::native_capture_inactivity::{
        ActivityPolicyEvaluation, ActivityPolicyEvaluations, ActivitySourceKind, EffectiveIdle,
        InactivityState,
    };

    let policies = ActivityPolicyEvaluations {
        screen: ActivityPolicyEvaluation {
            effective_idle: EffectiveIdle {
                source: ActivitySourceKind::InternalFallback,
                idle_ms: 20_000,
            },
            sources: vec![],
        },
        microphone: ActivityPolicyEvaluation {
            effective_idle: EffectiveIdle {
                source: ActivitySourceKind::InternalFallback,
                idle_ms: 20_000,
            },
            sources: vec![],
        },
        system_audio: ActivityPolicyEvaluation {
            effective_idle: EffectiveIdle {
                source: ActivitySourceKind::InternalFallback,
                idle_ms: 20_000,
            },
            sources: vec![],
        },
    };

    let mut inactivity = InactivityState {
        enabled: true,
        idle_timeout_seconds: 10,
        ..InactivityState::default()
    };
    inactivity.set_family_paused_states(true, true, true);

    let fields = idle_debug_family_fields(&policies, &inactivity);

    assert!(fields.screen_paused);
    assert!(fields.microphone_paused);
    assert!(fields.system_audio_paused);
}
