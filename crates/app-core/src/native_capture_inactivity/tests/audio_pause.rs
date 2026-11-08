use super::*;

#[test]
fn audio_mode_keeps_counting_idle_from_last_threshold_qualified_audio_activity() {
    let mut state =
        inactivity_state_fixture(InactivityActivityMode::SystemInputOrScreenOrAudio, 100);

    let first_evaluation = state.evaluate_microphone_policy_for_snapshot(
        20_000,
        ActivitySnapshot {
            system_input_idle_ms: Some(15_000),
            screen_activity_enabled: true,
            screen_activity_idle_ms: Some(10_000),
            microphone_activity: AudioActivitySourceState {
                enabled: true,
                idle_ms: Some(0),
                latest_normalized_level: Some(0.20),
            },
            system_audio_activity: empty_audio_activity(),
        },
    );

    assert_eq!(
        first_evaluation.effective_idle.source,
        ActivitySourceKind::MicrophoneCapture
    );
    assert_eq!(first_evaluation.effective_idle.idle_ms, 0);
    assert!(first_evaluation.sources[2].available);

    let second_evaluation = state.evaluate_microphone_policy_for_snapshot(
        20_100,
        ActivitySnapshot {
            system_input_idle_ms: Some(15_100),
            screen_activity_enabled: true,
            screen_activity_idle_ms: Some(10_100),
            microphone_activity: AudioActivitySourceState {
                enabled: true,
                idle_ms: Some(0),
                latest_normalized_level: Some(0.0),
            },
            system_audio_activity: empty_audio_activity(),
        },
    );

    assert_eq!(
        second_evaluation.effective_idle.source,
        ActivitySourceKind::MicrophoneCapture
    );
    assert_eq!(second_evaluation.effective_idle.idle_ms, 100);
    assert!(second_evaluation.sources[2].available);
    assert_eq!(second_evaluation.sources[2].idle_ms, Some(100));
    assert_eq!(
        second_evaluation.sources[2].latest_normalized_level,
        Some(0.0)
    );
    assert!(!state.should_pause_microphone_for_inactivity(
        20_100,
        ActivitySnapshot {
            system_input_idle_ms: Some(15_100),
            screen_activity_enabled: true,
            screen_activity_idle_ms: Some(10_100),
            microphone_activity: AudioActivitySourceState {
                enabled: true,
                idle_ms: Some(0),
                latest_normalized_level: Some(0.0),
            },
            system_audio_activity: empty_audio_activity(),
        }
    ));
    assert!(state.should_pause_microphone_for_inactivity(
        30_001,
        ActivitySnapshot {
            system_input_idle_ms: Some(25_001),
            screen_activity_enabled: true,
            screen_activity_idle_ms: Some(20_001),
            microphone_activity: AudioActivitySourceState {
                enabled: true,
                idle_ms: Some(0),
                latest_normalized_level: Some(0.0),
            },
            system_audio_activity: empty_audio_activity(),
        }
    ));
}

#[test]
fn full_mode_recent_system_input_does_not_resume_paused_microphone_without_mic_activity() {
    let mut state =
        inactivity_state_fixture(InactivityActivityMode::SystemInputOrScreenOrAudio, 100);
    state.set_family_paused_states(false, true, false);

    let snapshot = ActivitySnapshot {
        system_input_idle_ms: Some(0),
        screen_activity_enabled: true,
        screen_activity_idle_ms: Some(0),
        microphone_activity: AudioActivitySourceState {
            enabled: true,
            idle_ms: Some(0),
            latest_normalized_level: Some(0.0),
        },
        system_audio_activity: empty_audio_activity(),
    };

    assert!(state.is_microphone_paused());
    assert!(
        !state.should_resume_microphone_from_inactivity(20_000, snapshot),
        "system input must not resume microphone without threshold-qualified mic activity"
    );
}

#[test]
fn full_mode_recent_system_input_does_not_resume_paused_system_audio_without_system_audio_activity()
{
    let mut state =
        inactivity_state_fixture(InactivityActivityMode::SystemInputOrScreenOrAudio, 100);
    state.set_family_paused_states(false, false, true);

    let snapshot = ActivitySnapshot {
        system_input_idle_ms: Some(0),
        screen_activity_enabled: true,
        screen_activity_idle_ms: Some(0),
        microphone_activity: empty_audio_activity(),
        system_audio_activity: AudioActivitySourceState {
            enabled: true,
            idle_ms: Some(0),
            latest_normalized_level: Some(0.0),
        },
    };

    assert!(state.is_system_audio_paused());
    assert!(
        !state.should_resume_system_audio_from_inactivity(20_000, snapshot),
        "system input must not resume system audio without threshold-qualified system-audio activity"
    );
}

#[test]
fn full_mode_threshold_qualified_mic_activity_resumes_paused_microphone() {
    let mut state =
        inactivity_state_fixture(InactivityActivityMode::SystemInputOrScreenOrAudio, 100);
    state.set_family_paused_states(false, true, false);

    let snapshot = ActivitySnapshot {
        system_input_idle_ms: Some(state.idle_timeout_ms() + 1),
        screen_activity_enabled: true,
        screen_activity_idle_ms: Some(state.idle_timeout_ms() + 1),
        microphone_activity: AudioActivitySourceState {
            enabled: true,
            idle_ms: Some(0),
            latest_normalized_level: Some(0.20),
        },
        system_audio_activity: empty_audio_activity(),
    };

    assert!(state.should_resume_microphone_from_inactivity(20_000, snapshot));
}

#[test]
fn full_mode_threshold_qualified_system_audio_activity_resumes_paused_system_audio() {
    let mut state =
        inactivity_state_fixture(InactivityActivityMode::SystemInputOrScreenOrAudio, 100);
    state.set_family_paused_states(false, false, true);

    let snapshot = ActivitySnapshot {
        system_input_idle_ms: Some(state.idle_timeout_ms() + 1),
        screen_activity_enabled: true,
        screen_activity_idle_ms: Some(state.idle_timeout_ms() + 1),
        microphone_activity: empty_audio_activity(),
        system_audio_activity: AudioActivitySourceState {
            enabled: true,
            idle_ms: Some(0),
            latest_normalized_level: Some(0.20),
        },
    };

    assert!(state.should_resume_system_audio_from_inactivity(20_000, snapshot));
}

#[test]
fn default_mode_pauses_microphone_after_true_audio_inactivity_despite_recent_system_input() {
    let mut state = inactivity_state_fixture(default_inactivity_activity_mode(), 100);

    assert_eq!(
        state.activity_mode,
        InactivityActivityMode::SystemInputOrScreen
    );

    let active_snapshot = ActivitySnapshot {
        system_input_idle_ms: Some(0),
        screen_activity_enabled: true,
        screen_activity_idle_ms: Some(0),
        microphone_activity: AudioActivitySourceState {
            enabled: true,
            idle_ms: Some(0),
            latest_normalized_level: Some(0.20),
        },
        system_audio_activity: empty_audio_activity(),
    };
    let idle_snapshot = ActivitySnapshot {
        system_input_idle_ms: Some(0),
        screen_activity_enabled: true,
        screen_activity_idle_ms: Some(0),
        microphone_activity: AudioActivitySourceState {
            enabled: true,
            idle_ms: Some(0),
            latest_normalized_level: Some(0.0),
        },
        system_audio_activity: empty_audio_activity(),
    };

    assert!(!state.should_pause_microphone_for_inactivity(20_000, active_snapshot));
    assert!(
        state.should_pause_microphone_for_inactivity(30_001, idle_snapshot),
        "microphone should pause after >10s without threshold-qualified audio activity even when system input remains recent"
    );
}

#[test]
fn default_mode_pauses_system_audio_after_true_audio_inactivity_despite_recent_system_input() {
    let mut state = inactivity_state_fixture(default_inactivity_activity_mode(), 100);

    assert_eq!(
        state.activity_mode,
        InactivityActivityMode::SystemInputOrScreen
    );

    let active_snapshot = ActivitySnapshot {
        system_input_idle_ms: Some(0),
        screen_activity_enabled: true,
        screen_activity_idle_ms: Some(0),
        microphone_activity: empty_audio_activity(),
        system_audio_activity: AudioActivitySourceState {
            enabled: true,
            idle_ms: Some(0),
            latest_normalized_level: Some(0.20),
        },
    };
    let idle_snapshot = ActivitySnapshot {
        system_input_idle_ms: Some(0),
        screen_activity_enabled: true,
        screen_activity_idle_ms: Some(0),
        microphone_activity: empty_audio_activity(),
        system_audio_activity: AudioActivitySourceState {
            enabled: true,
            idle_ms: Some(0),
            latest_normalized_level: Some(0.0),
        },
    };

    assert!(!state.should_pause_system_audio_for_inactivity(20_000, active_snapshot));
    assert!(
        state.should_pause_system_audio_for_inactivity(30_001, idle_snapshot),
        "system audio should pause after >10s without threshold-qualified audio activity even when system input remains recent"
    );
}

#[test]
fn default_system_audio_threshold_accepts_quieter_playback_than_microphone() {
    let mut microphone_state = inactivity_state_fixture(default_inactivity_activity_mode(), 50);
    let mut system_audio_state = inactivity_state_fixture(default_inactivity_activity_mode(), 50);
    let now = 20_000;

    let quiet_microphone_snapshot = ActivitySnapshot {
        system_input_idle_ms: Some(microphone_state.idle_timeout_ms() + 1),
        screen_activity_enabled: true,
        screen_activity_idle_ms: Some(microphone_state.idle_timeout_ms() + 1),
        microphone_activity: AudioActivitySourceState {
            enabled: true,
            idle_ms: Some(0),
            latest_normalized_level: Some(0.03),
        },
        system_audio_activity: empty_audio_activity(),
    };
    let quiet_system_audio_snapshot = ActivitySnapshot {
        system_input_idle_ms: Some(system_audio_state.idle_timeout_ms() + 1),
        screen_activity_enabled: true,
        screen_activity_idle_ms: Some(system_audio_state.idle_timeout_ms() + 1),
        microphone_activity: empty_audio_activity(),
        system_audio_activity: AudioActivitySourceState {
            enabled: true,
            idle_ms: Some(0),
            latest_normalized_level: Some(0.03),
        },
    };

    assert!(
        microphone_state.should_pause_microphone_for_inactivity(now, quiet_microphone_snapshot),
        "quiet 3% microphone peaks remain below the default microphone threshold"
    );
    assert!(
        !system_audio_state
            .should_pause_system_audio_for_inactivity(now, quiet_system_audio_snapshot),
        "quiet 3% system-audio peaks should still count as active playback"
    );
}

#[test]
fn family_pause_state_checks_are_independent() {
    let snapshot = ActivitySnapshot {
        system_input_idle_ms: Some(20_000),
        screen_activity_enabled: true,
        screen_activity_idle_ms: Some(500),
        microphone_activity: AudioActivitySourceState {
            enabled: true,
            idle_ms: Some(250),
            latest_normalized_level: Some(0.2),
        },
        system_audio_activity: empty_audio_activity(),
    };

    let mut screen_paused_state =
        inactivity_state_fixture(InactivityActivityMode::SystemInputOrScreenOrAudio, 100);
    screen_paused_state.set_family_paused_states(true, false, false);

    assert!(screen_paused_state.is_paused);
    assert!(screen_paused_state.is_screen_paused());
    assert!(!screen_paused_state.is_microphone_paused());
    assert!(!screen_paused_state.is_system_audio_paused());
    assert!(screen_paused_state.should_resume_screen_from_inactivity(20_000, snapshot));
    assert!(!screen_paused_state.should_resume_microphone_from_inactivity(20_000, snapshot));

    let mut microphone_paused_state =
        inactivity_state_fixture(InactivityActivityMode::SystemInputOrScreenOrAudio, 100);
    microphone_paused_state.set_family_paused_states(false, true, false);

    assert!(microphone_paused_state.is_paused);
    assert!(!microphone_paused_state.is_screen_paused());
    assert!(microphone_paused_state.is_microphone_paused());
    assert!(!microphone_paused_state.is_system_audio_paused());
    assert!(!microphone_paused_state.should_resume_screen_from_inactivity(20_000, snapshot));
    assert!(microphone_paused_state.should_resume_microphone_from_inactivity(20_000, snapshot));

    let mut system_audio_paused_state =
        inactivity_state_fixture(InactivityActivityMode::SystemInputOrScreenOrAudio, 100);
    system_audio_paused_state.set_family_paused_states(false, false, true);

    assert!(system_audio_paused_state.is_paused);
    assert!(!system_audio_paused_state.is_screen_paused());
    assert!(!system_audio_paused_state.is_microphone_paused());
    assert!(system_audio_paused_state.is_system_audio_paused());
}

#[test]
fn legacy_global_pause_state_applies_to_all_families() {
    let state = InactivityState {
        is_paused: true,
        ..InactivityState::default()
    };

    assert!(state.is_screen_paused());
    assert!(state.is_microphone_paused());
    assert!(state.is_system_audio_paused());
}

#[test]
fn legacy_pause_blocked_in_per_family_audio_mode() {
    // In SystemInputOrScreenOrAudio mode, the legacy global pause should
    // never fire even when idle exceeds the threshold — per-family handlers
    // own the pause/resume lifecycle in this mode.
    let mut state =
        inactivity_state_fixture(InactivityActivityMode::SystemInputOrScreenOrAudio, 50);
    let now = 1_000 + state.idle_timeout_ms() + 1;
    let snapshot = empty_activity_snapshot();

    assert!(
        !state.should_pause_for_inactivity(now, snapshot),
        "legacy pause must not fire in SystemInputOrScreenOrAudio mode"
    );
}

#[test]
fn screen_idle_does_not_trigger_legacy_global_pause_when_microphone_is_active() {
    for activity_mode in [
        InactivityActivityMode::SystemInputOrScreen,
        InactivityActivityMode::SystemInputOrScreenOrAudio,
    ] {
        let mut state = inactivity_state_fixture(activity_mode, 100);
        let now = 20_000;
        let snapshot = ActivitySnapshot {
            system_input_idle_ms: Some(state.idle_timeout_ms() + 1),
            screen_activity_enabled: true,
            screen_activity_idle_ms: Some(state.idle_timeout_ms() + 1),
            microphone_activity: AudioActivitySourceState {
                enabled: true,
                idle_ms: Some(0),
                latest_normalized_level: Some(0.20),
            },
            system_audio_activity: empty_audio_activity(),
        };

        assert!(
            state.should_pause_screen_for_inactivity(now, snapshot),
            "screen/video should pause when screen family is idle in {activity_mode:?}"
        );
        assert!(
            !state.should_pause_microphone_for_inactivity(now, snapshot),
            "threshold-active microphone should not family-pause in {activity_mode:?}"
        );
        assert!(
            !state.should_pause_for_inactivity(now, snapshot),
            "threshold-active microphone must prevent legacy all-source pause in {activity_mode:?}"
        );
    }
}

#[test]
fn screen_idle_does_not_trigger_legacy_global_pause_when_system_audio_is_active() {
    for activity_mode in [
        InactivityActivityMode::SystemInputOrScreen,
        InactivityActivityMode::SystemInputOrScreenOrAudio,
    ] {
        let mut state = inactivity_state_fixture(activity_mode, 100);
        let now = 20_000;
        let snapshot = ActivitySnapshot {
            system_input_idle_ms: Some(state.idle_timeout_ms() + 1),
            screen_activity_enabled: true,
            screen_activity_idle_ms: Some(state.idle_timeout_ms() + 1),
            microphone_activity: empty_audio_activity(),
            system_audio_activity: AudioActivitySourceState {
                enabled: true,
                idle_ms: Some(0),
                latest_normalized_level: Some(0.20),
            },
        };

        assert!(
            state.should_pause_screen_for_inactivity(now, snapshot),
            "screen/video should pause when screen family is idle in {activity_mode:?}"
        );
        assert!(
            !state.should_pause_system_audio_for_inactivity(now, snapshot),
            "threshold-active system audio should not family-pause in {activity_mode:?}"
        );
        assert!(
            !state.should_pause_for_inactivity(now, snapshot),
            "threshold-active system audio must prevent legacy all-source pause in {activity_mode:?}"
        );
    }
}

#[test]
fn legacy_global_pause_still_fires_when_hybrid_screen_idle_and_audio_inactive() {
    let mut state = inactivity_state_fixture(InactivityActivityMode::SystemInputOrScreen, 100);
    let now = 20_000;
    let snapshot = ActivitySnapshot {
        system_input_idle_ms: Some(state.idle_timeout_ms() + 1),
        screen_activity_enabled: true,
        screen_activity_idle_ms: Some(state.idle_timeout_ms() + 1),
        microphone_activity: AudioActivitySourceState {
            enabled: true,
            idle_ms: Some(state.idle_timeout_ms() + 1),
            latest_normalized_level: Some(0.0),
        },
        system_audio_activity: AudioActivitySourceState {
            enabled: true,
            idle_ms: Some(state.idle_timeout_ms() + 1),
            latest_normalized_level: Some(0.0),
        },
    };

    assert!(
        state.should_pause_for_inactivity(now, snapshot),
        "legacy global pause should remain available when no configured family is active"
    );
}

#[test]
fn legacy_resume_blocked_when_per_family_pause_is_active() {
    // When is_paused=true because a per-family flag is set, the legacy
    // resume path must not fire — only the per-family resume should clear
    // that state.
    let mut state =
        inactivity_state_fixture(InactivityActivityMode::SystemInputOrScreenOrAudio, 50);
    state.set_family_paused_states(true, false, false); // screen paused via per-family
    let now = 1_000; // idle < timeout → would trigger resume in legacy path
    let snapshot = ActivitySnapshot {
        system_input_idle_ms: Some(0), // recent activity
        screen_activity_idle_ms: Some(0),
        ..empty_activity_snapshot()
    };

    assert!(
        state.is_paused,
        "is_paused should be true from per-family pause"
    );
    assert!(
        !state.should_resume_from_inactivity(now, snapshot),
        "legacy resume must not fire when per-family pause is active"
    );
}

#[test]
fn legacy_resume_fires_for_legacy_global_pause() {
    // When is_paused=true with no per-family flags (legacy pause), the
    // legacy resume path should still work.
    let mut state = inactivity_state_fixture(InactivityActivityMode::SystemInputOrScreen, 50);
    state.is_paused = true; // legacy global pause (no family flags)
    let now = 1_000; // same as last_activity → idle=0 < timeout
    let snapshot = ActivitySnapshot {
        system_input_idle_ms: Some(0),
        screen_activity_idle_ms: Some(0),
        ..empty_activity_snapshot()
    };

    assert!(
        state.should_resume_from_inactivity(now, snapshot),
        "legacy resume should fire for legacy global pause"
    );
}
