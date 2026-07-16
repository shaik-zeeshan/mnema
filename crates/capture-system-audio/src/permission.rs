//! Inferring whether system audio is allowed to record (ADR 0052).
//!
//! Core Audio process taps have their own TCC category ("Screen & System Audio
//! Recording") and, unlike the screen and the microphone, **no API to ask
//! whether it was granted**. A denied tap is not an error: it starts, its IOProc
//! fires on schedule, and every sample is zero — which is also exactly what a
//! quiet Mac looks like. The ADR rejected the private TCC SPI that would answer
//! this properly, so the answer here is a suspicion built from one fact: has a
//! tap ever delivered a sound?
//!
//! That gives the three states the UI renders — not yet requested, assumed
//! working, possibly blocked — and it is why the middle one is *assumed*: a
//! delivered sound proves the grant, but silence proves nothing.

use capture_types::CapturePermissionState;

/// What every tap since the swap has managed to deliver. Persisted (one
/// `app_settings` row) because a denial the user never fixes must still be
/// visible after a restart, and because "no tap has ever run" is a different
/// answer from "taps ran and heard nothing".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SystemAudioEvidence {
    /// No tap has run long enough to judge — including the fresh-install case,
    /// where the TCC prompt has not been raised yet.
    #[default]
    None,
    /// A tap ran and never delivered a sound.
    SilentSession,
    /// A tap delivered a sound. Terminal: a grant cannot be un-proved, and a
    /// later quiet session is just a quiet session.
    SoundHeard,
}

impl SystemAudioEvidence {
    /// The `app_settings` value. Stable — a rename silently resets every
    /// install's evidence to `None`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::SilentSession => "silent_session",
            Self::SoundHeard => "sound_heard",
        }
    }

    /// Anything unrecognised (or missing) reads as `None`: the honest answer to
    /// a value we cannot interpret is "we don't know yet", never "blocked".
    pub fn from_str(value: &str) -> Self {
        match value.trim() {
            "silent_session" => Self::SilentSession,
            "sound_heard" => Self::SoundHeard,
            _ => Self::None,
        }
    }

    /// Folds what a tap generation just observed into the stored evidence.
    /// Monotonic: evidence only ever gets stronger, so this is safe to call from
    /// every tick and only writes when the answer moves.
    pub fn observe(self, heard_sound: bool) -> Self {
        match (self, heard_sound) {
            (Self::SoundHeard, _) | (_, true) => Self::SoundHeard,
            _ => Self::SilentSession,
        }
    }
}

/// The tri-state the permission surfaces render.
///
/// `supported` is the existing macOS 15.0 runtime gate: below it there is no
/// system audio to be blocked from.
pub fn system_audio_permission_state(
    supported: bool,
    evidence: SystemAudioEvidence,
) -> CapturePermissionState {
    if !supported {
        return CapturePermissionState::Unsupported;
    }

    match evidence {
        SystemAudioEvidence::None => CapturePermissionState::NotDetermined,
        SystemAudioEvidence::SoundHeard => CapturePermissionState::AssumedWorking,
        SystemAudioEvidence::SilentSession => CapturePermissionState::PossiblyBlocked,
    }
}

/// How long a tap must run without a sound before its silence counts as
/// evidence.
///
/// ponytail: a naive threshold — a Mac that genuinely played nothing for this
/// long is indistinguishable from a denied one, so it earns the same hint. That
/// false positive is why the hint is worded "may be blocked" and is dismissible.
/// If it proves noisy in the soak, the upgrade is to require several silent
/// sessions (or weigh them against the user's own activity) rather than one.
pub const SILENT_SESSION_AFTER_MS: u64 = 60_000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_observed_yet_is_not_a_denial() {
        assert_eq!(
            system_audio_permission_state(true, SystemAudioEvidence::None),
            CapturePermissionState::NotDetermined
        );
    }

    #[test]
    fn a_delivered_sound_is_the_only_proof_of_a_grant() {
        assert_eq!(
            system_audio_permission_state(true, SystemAudioEvidence::SoundHeard),
            CapturePermissionState::AssumedWorking
        );
    }

    #[test]
    fn taps_that_only_ever_delivered_silence_are_suspect() {
        assert_eq!(
            system_audio_permission_state(true, SystemAudioEvidence::SilentSession),
            CapturePermissionState::PossiblyBlocked
        );
    }

    // The macOS 15.0 gate wins over any evidence: an unsupported OS has no
    // grant to be missing, and "possibly blocked" would send the user to a
    // Settings pane that cannot help them.
    #[test]
    fn an_unsupported_os_is_never_reported_as_blocked() {
        for evidence in [
            SystemAudioEvidence::None,
            SystemAudioEvidence::SilentSession,
            SystemAudioEvidence::SoundHeard,
        ] {
            assert_eq!(
                system_audio_permission_state(false, evidence),
                CapturePermissionState::Unsupported
            );
        }
    }

    #[test]
    fn one_sound_settles_it_and_later_silence_cannot_unsettle_it() {
        let evidence = SystemAudioEvidence::None.observe(false);
        assert_eq!(evidence, SystemAudioEvidence::SilentSession);

        let evidence = evidence.observe(true);
        assert_eq!(evidence, SystemAudioEvidence::SoundHeard);

        // Every quiet session after the grant is proved.
        assert_eq!(
            evidence.observe(false).observe(false),
            SystemAudioEvidence::SoundHeard
        );
    }

    #[test]
    fn evidence_round_trips_its_stored_value() {
        for evidence in [
            SystemAudioEvidence::None,
            SystemAudioEvidence::SilentSession,
            SystemAudioEvidence::SoundHeard,
        ] {
            assert_eq!(SystemAudioEvidence::from_str(evidence.as_str()), evidence);
        }
    }

    // A row written by a future build (or a corrupted one) must not accuse the
    // user of a denial.
    #[test]
    fn an_unreadable_stored_value_degrades_to_not_yet_judged() {
        for value in ["", "  ", "blocked", "sound", "SOUND_HEARD"] {
            assert_eq!(SystemAudioEvidence::from_str(value), SystemAudioEvidence::None);
        }
    }
}
