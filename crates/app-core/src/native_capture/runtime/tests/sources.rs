//! Which sources a segment and an inactivity pause cover.

use crate::native_capture::runtime::*;
use crate::native_capture_inactivity::InactivityState;
use capture_types::{CaptureErrorResponse, CaptureOutputFiles, CaptureSources};

#[test]
fn active_sources_for_inactivity_excludes_screen_when_screen_paused() {
    let requested = CaptureSources {
        screen: true,
        microphone: true,
        system_audio: true,
    };

    let active = active_sources_for_inactivity_paused_state(&requested, true, false, false)
        .expect("audio sources should remain active");

    assert!(!active.screen);
    assert!(active.microphone);
    // Both audio families are independent of the screen session, so a screen
    // pause leaves both of them recording (ADR 0052).
    assert!(active.system_audio);
}

#[test]
fn active_sources_for_inactivity_excludes_audio_when_audio_paused() {
    let requested = CaptureSources {
        screen: true,
        microphone: true,
        system_audio: true,
    };

    let active = active_sources_for_inactivity_paused_state(&requested, false, true, true)
        .expect("screen source should remain active");

    assert!(active.screen);
    assert!(!active.microphone);
    assert!(!active.system_audio);
}

#[test]
fn active_sources_for_inactivity_returns_none_when_all_paused() {
    let requested = CaptureSources {
        screen: true,
        microphone: true,
        system_audio: true,
    };

    assert!(active_sources_for_inactivity_paused_state(&requested, true, true, true).is_none());
}

#[test]
fn active_sources_for_inactivity_returns_all_when_nothing_paused() {
    let requested = CaptureSources {
        screen: true,
        microphone: true,
        system_audio: true,
    };

    let active = active_sources_for_inactivity_paused_state(&requested, false, false, false)
        .expect("all sources should remain active");

    assert_eq!(active, requested);
}

#[test]
fn active_sources_for_inactivity_handles_screen_only_with_audio_pause() {
    let requested = CaptureSources {
        screen: true,
        microphone: false,
        system_audio: false,
    };

    let active = active_sources_for_inactivity_paused_state(&requested, false, true, true)
        .expect("screen-only capture should stay active when audio paused");

    assert!(active.screen);
    assert!(!active.microphone);
    assert!(!active.system_audio);
}

#[test]
fn active_sources_for_inactivity_returns_none_for_screen_only_with_screen_pause() {
    let requested = CaptureSources {
        screen: true,
        microphone: false,
        system_audio: false,
    };

    assert!(active_sources_for_inactivity_paused_state(&requested, true, false, false).is_none());
}

#[test]
fn current_segment_sources_for_runtime_returns_explicit_sources() {
    let runtime = NativeCaptureRuntime {
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        }),
        current_segment_sources: Some(CaptureSources {
            screen: true,
            microphone: false,
            system_audio: false,
        }),
        ..Default::default()
    };

    let sources =
        current_segment_sources_for_runtime(&runtime).expect("should return explicit sources");

    assert!(sources.screen);
    assert!(!sources.microphone);
    assert!(!sources.system_audio);
}

#[test]
fn mark_runtime_session_stopped_clears_current_segment_sources() {
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
        ..Default::default()
    };

    mark_runtime_session_stopped(&mut runtime);

    assert!(runtime.current_segment_sources.is_none());
}

#[cfg(target_os = "macos")]
#[test]
fn stopping_the_session_clears_the_away_holds() {
    let mut runtime = NativeCaptureRuntime {
        microphone_away_hold: true,
        system_audio_away_hold: true,
        microphone_waiting_for_device: true,
        ..Default::default()
    };

    mark_runtime_session_stopped(&mut runtime);

    assert!(!runtime.microphone_away_hold);
    assert!(!runtime.system_audio_away_hold);
    assert!(!runtime.microphone_waiting_for_device);
}

// A take that panicked never handed the microphone back; the stopped session's
// lend must not hold the next session's microphone.
#[cfg(target_os = "macos")]
#[test]
fn stopping_the_session_clears_a_lend_that_never_came_back() {
    let mut runtime = NativeCaptureRuntime {
        microphone_lent_out: true,
        ..Default::default()
    };

    mark_runtime_session_stopped(&mut runtime);

    assert!(!runtime.microphone_lent_out);
}

#[test]
fn current_segment_sources_for_runtime_fallback_respects_audio_paused() {
    // When current_segment_sources is None but sessions/outputs exist, the
    // fallback should gate system_audio with the audio_paused flag rather
    // than returning raw requested_sources.
    let runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        }),
        current_segment_sources: None,
        #[cfg(target_os = "macos")]
        current_segment_output_files: Some(CaptureOutputFiles {
            screen_file: Some("/tmp/screen.mov".to_string()),
            screen_files: vec!["/tmp/screen.mov".to_string()],
            microphone_file: None,
            microphone_files: Vec::new(),
            system_audio_file: None,
            system_audio_files: Vec::new(),
        }),
        #[cfg(target_os = "macos")]
        active_screen_session: None,
        #[cfg(target_os = "macos")]
        active_microphone_session: None,
        active_system_audio_session: None,
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

    let sources =
        current_segment_sources_for_runtime(&runtime).expect("should return fallback sources");

    assert!(sources.screen, "screen should be active");
    assert!(
        !sources.microphone,
        "microphone should be excluded when audio is paused"
    );
    assert!(
        !sources.system_audio,
        "system_audio should be excluded when audio is paused"
    );
}

#[test]
fn current_segment_sources_for_runtime_fallback_respects_screen_paused() {
    // When current_segment_sources is None but output files exist, the fallback
    // should gate screen with the screen_paused flag.
    let runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: false,
        }),
        current_segment_sources: None,
        #[cfg(target_os = "macos")]
        current_segment_output_files: Some(CaptureOutputFiles {
            screen_file: None,
            screen_files: Vec::new(),
            microphone_file: Some("/tmp/microphone.m4a".to_string()),
            microphone_files: vec!["/tmp/microphone.m4a".to_string()],
            system_audio_file: None,
            system_audio_files: Vec::new(),
        }),
        #[cfg(target_os = "macos")]
        active_screen_session: None,
        #[cfg(target_os = "macos")]
        active_microphone_session: None,
        active_system_audio_session: None,
        inactivity: InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            screen_paused: true,
            is_paused: true,
            ..InactivityState::default()
        },
        ..Default::default()
    };

    // On non-macOS the cfg-gated branch is skipped, so only test on macOS.
    #[cfg(target_os = "macos")]
    {
        let sources = current_segment_sources_for_runtime(&runtime)
            .expect("should return fallback audio-only sources");

        assert!(
            !sources.screen,
            "screen should be excluded when screen is paused"
        );
        assert!(sources.microphone, "microphone should remain active");
    }
}

#[test]
fn current_segment_sources_for_runtime_fallback_returns_all_when_nothing_paused() {
    let runtime = NativeCaptureRuntime {
        is_running: true,
        requested_sources: Some(CaptureSources {
            screen: true,
            microphone: true,
            system_audio: true,
        }),
        current_segment_sources: None,
        #[cfg(target_os = "macos")]
        current_segment_output_files: Some(CaptureOutputFiles {
            screen_file: Some("/tmp/screen.mov".to_string()),
            screen_files: vec!["/tmp/screen.mov".to_string()],
            microphone_file: None,
            microphone_files: Vec::new(),
            system_audio_file: None,
            system_audio_files: Vec::new(),
        }),
        #[cfg(target_os = "macos")]
        active_screen_session: None,
        #[cfg(target_os = "macos")]
        active_microphone_session: None,
        active_system_audio_session: None,
        inactivity: InactivityState::default(),
        ..Default::default()
    };

    let sources = current_segment_sources_for_runtime(&runtime)
        .expect("should return all sources when nothing paused");

    assert!(sources.screen);
    assert!(sources.microphone);
    assert!(sources.system_audio);
}

#[cfg(target_os = "macos")]
#[test]
fn current_segment_sources_for_runtime_masks_stale_screen_during_privacy_suspension() {
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
            screen: true,
            microphone: true,
            system_audio: true,
        }),
        microphone_recording_file: Some("/tmp/privacy-suspended-microphone.m4a".to_string()),
        capture_suspension: Some(CaptureSuspension::with_kind(
            CaptureSuspensionKind::PrivacyFilter,
            &privacy_error,
        )),
        ..Default::default()
    };

    let sources = current_segment_sources_for_runtime(&runtime)
        .expect("live microphone continuation should remain active");

    assert!(
        !sources.screen,
        "stale explicit sources must not re-enable suspended screen capture"
    );
    assert!(sources.microphone);
    assert!(
        sources.system_audio,
        "a screen suspension does not suspend the tap, so system audio stays active"
    );
}

// Inverts the old rule. System audio used to be forced inactive whenever the
// screen was paused, because it rode the screen's stream; on its own tap it keeps
// recording through a screen pause exactly as the microphone does.
#[test]
fn active_sources_for_inactivity_keeps_system_audio_when_screen_paused() {
    let requested = CaptureSources {
        screen: true,
        microphone: false,
        system_audio: true,
    };

    let active = active_sources_for_inactivity_paused_state(&requested, true, false, false)
        .expect("system audio alone keeps the active set non-empty");

    assert!(
        active.system_audio,
        "a paused screen must not stop system audio"
    );
    assert!(!active.screen);
}

#[test]
fn active_sources_for_inactivity_system_audio_answers_only_to_its_own_family() {
    let requested = CaptureSources {
        screen: true,
        microphone: true,
        system_audio: true,
    };

    // Screen paused, audio families active → system audio keeps recording.
    let active = active_sources_for_inactivity_paused_state(&requested, true, false, false)
        .expect("microphone should keep sources non-empty");
    assert!(active.system_audio, "system audio needs no screen session");
    assert!(active.microphone);
    assert!(!active.screen);

    // Its own family paused → inactive, whatever the screen is doing.
    let active = active_sources_for_inactivity_paused_state(&requested, false, true, true)
        .expect("screen should keep sources non-empty");
    assert!(!active.system_audio);
    assert!(active.screen);

    let active = active_sources_for_inactivity_paused_state(&requested, false, false, false)
        .expect("all sources active");
    assert!(active.system_audio);
}

// An audio-only session is a first-class session: no screen, no microphone.
#[test]
fn active_sources_for_inactivity_allows_a_system_audio_only_session() {
    let requested = CaptureSources {
        screen: false,
        microphone: false,
        system_audio: true,
    };

    let active = active_sources_for_inactivity_paused_state(&requested, false, false, false)
        .expect("a system-audio-only session has an active source");
    assert_eq!(active, requested);

    assert!(
        active_sources_for_inactivity_paused_state(&requested, false, false, true).is_none(),
        "pausing the only source empties the active set"
    );
}
