//! The per-source mid-session mask: a user-scoped pause on one source, carried
//! through the same paused-flag seam as the inactivity pause.
//!
//! The property under test everywhere here is the truth table — a source records
//! only when *neither* the user mask nor a system condition holds, and only the
//! user ever clears a mask.

use super::inactivity::{ActivitySnapshot, AudioActivitySourceState, InactivityState};
use super::lifecycle::unmasked_sources;
use capture_types::CaptureSources;

fn sources(screen: bool, microphone: bool, system_audio: bool) -> CaptureSources {
    CaptureSources {
        screen,
        microphone,
        system_audio,
    }
}

/// Everything is loudly active: every `should_resume_*` would fire on its own.
fn active_snapshot() -> ActivitySnapshot {
    ActivitySnapshot {
        system_input_idle_ms: Some(0),
        screen_activity_enabled: true,
        screen_activity_idle_ms: Some(0),
        microphone_activity: AudioActivitySourceState {
            enabled: true,
            idle_ms: Some(0),
            latest_normalized_level: Some(0.9),
        },
        system_audio_activity: AudioActivitySourceState {
            enabled: true,
            idle_ms: Some(0),
            latest_normalized_level: Some(0.9),
        },
    }
}

fn enabled_inactivity() -> InactivityState {
    InactivityState {
        enabled: true,
        idle_timeout_seconds: 10,
        ..InactivityState::default()
    }
}

#[test]
fn masking_a_source_reports_it_paused_without_touching_its_siblings() {
    let mut inactivity = InactivityState::default();
    inactivity.set_user_masked_sources(sources(false, true, false));

    assert!(inactivity.is_microphone_paused());
    assert!(!inactivity.is_screen_paused());
    assert!(!inactivity.is_system_audio_paused());
    assert!(inactivity.has_user_masked_source());
    assert_eq!(inactivity.user_masked_sources(), sources(false, true, false));
}

#[test]
fn a_mask_and_an_inactivity_pause_both_hold_and_the_mask_outlives_it() {
    let mut inactivity = enabled_inactivity();
    // Both conditions on the same source at once — the point of the truth table.
    inactivity.set_family_paused_states(false, true, false);
    inactivity.set_user_masked_sources(sources(false, true, false));
    assert!(inactivity.is_microphone_paused());

    // Inactivity releases its half; the mask still holds the source off.
    inactivity.set_family_paused_states(false, false, false);
    assert!(
        inactivity.is_microphone_paused(),
        "clearing the inactivity pause must not lift the user mask"
    );

    // Only the user clears the other half.
    inactivity.set_user_masked_sources(CaptureSources::default());
    assert!(!inactivity.is_microphone_paused());
}

#[test]
fn activity_never_resumes_a_masked_source() {
    for mask in [
        sources(true, false, false),
        sources(false, true, false),
        sources(false, false, true),
    ] {
        let mut inactivity = enabled_inactivity();
        inactivity.set_family_paused_states(true, true, true);
        inactivity.set_user_masked_sources(mask.clone());
        let snapshot = active_snapshot();

        // Screen's resume has a 2s settle guard; nothing marked its pause start,
        // so the guard is already elapsed here.
        assert_eq!(
            inactivity.should_resume_screen_from_inactivity(60_000, snapshot),
            !mask.screen,
            "screen resume must be refused only for the masked source ({mask:?})"
        );
        assert_eq!(
            inactivity.should_resume_microphone_from_inactivity(60_000, snapshot),
            !mask.microphone,
            "microphone resume must be refused only for the masked source ({mask:?})"
        );
        assert_eq!(
            inactivity.should_resume_system_audio_from_inactivity(60_000, snapshot),
            !mask.system_audio,
            "system-audio resume must be refused only for the masked source ({mask:?})"
        );
    }
}

#[test]
fn inactivity_does_not_re_pause_a_masked_source() {
    let mut inactivity = enabled_inactivity();
    inactivity.set_user_masked_sources(sources(true, true, true));
    let snapshot = ActivitySnapshot {
        system_input_idle_ms: Some(600_000),
        screen_activity_enabled: true,
        screen_activity_idle_ms: Some(600_000),
        microphone_activity: AudioActivitySourceState {
            enabled: true,
            idle_ms: Some(600_000),
            latest_normalized_level: Some(0.0),
        },
        system_audio_activity: AudioActivitySourceState {
            enabled: true,
            idle_ms: Some(600_000),
            latest_normalized_level: Some(0.0),
        },
    };

    assert!(!inactivity.should_pause_screen_for_inactivity(600_000, snapshot));
    assert!(!inactivity.should_pause_microphone_for_inactivity(600_000, snapshot));
    assert!(!inactivity.should_pause_system_audio_for_inactivity(600_000, snapshot));
}

#[test]
fn the_legacy_all_source_resume_never_sweeps_a_mask_away() {
    let mut inactivity = enabled_inactivity();
    // Legacy global pause: `is_paused` with no per-family flag set.
    inactivity.is_paused = true;
    assert!(inactivity.should_resume_from_inactivity(60_000, active_snapshot()));

    inactivity.set_user_masked_sources(sources(false, false, true));
    assert!(
        !inactivity.should_resume_from_inactivity(60_000, active_snapshot()),
        "the legacy resume clears all three families and would unmask"
    );
}

#[test]
fn live_sources_are_the_requested_set_minus_the_mask() {
    let requested = sources(true, true, false);

    assert_eq!(
        unmasked_sources(&requested, &CaptureSources::default()),
        requested
    );
    assert_eq!(
        unmasked_sources(&requested, &sources(false, true, false)),
        sources(true, false, false)
    );
    // A mask on a source the session never requested changes nothing — the mask
    // only works inside `requested_sources`.
    assert_eq!(
        unmasked_sources(&requested, &sources(false, false, true)),
        requested
    );
}
