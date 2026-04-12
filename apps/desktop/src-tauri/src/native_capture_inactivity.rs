use capture_types::{InactivityActivityMode, RecordingSettings};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActivitySourceKind {
    SystemInput,
    ScreenCapture,
    InternalFallback,
}

impl ActivitySourceKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ActivitySourceKind::SystemInput => "system_input",
            ActivitySourceKind::ScreenCapture => "screen_capture",
            ActivitySourceKind::InternalFallback => "internal_fallback",
        }
    }
}

const ALL_ACTIVITY_SOURCES: [ActivitySourceKind; 2] = [
    ActivitySourceKind::SystemInput,
    ActivitySourceKind::ScreenCapture,
];
const SYSTEM_INPUT_ONLY_SOURCES: [ActivitySourceKind; 1] = [ActivitySourceKind::SystemInput];
const HYBRID_SOURCES: [ActivitySourceKind; 2] = [
    ActivitySourceKind::SystemInput,
    ActivitySourceKind::ScreenCapture,
];

#[derive(Debug, Clone, Copy)]
pub(crate) struct ActivitySourceIdle {
    pub kind: ActivitySourceKind,
    pub idle_ms: u64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ActivitySourceSample {
    pub kind: ActivitySourceKind,
    pub available: bool,
    pub idle_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ActivitySnapshot {
    pub system_input_idle_ms: Option<u64>,
    pub screen_activity_idle_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct EffectiveIdle {
    pub source: ActivitySourceKind,
    pub idle_ms: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct ActivityPolicyEvaluation {
    pub effective_idle: EffectiveIdle,
    pub sources: Vec<ActivitySourceSample>,
}

#[derive(Debug, Clone)]
pub(crate) struct InactivityState {
    pub enabled: bool,
    pub idle_timeout_seconds: u64,
    pub activity_mode: InactivityActivityMode,
    pub last_activity_monotonic_ms: u64,
    pub is_paused: bool,
}

impl Default for InactivityState {
    fn default() -> Self {
        Self {
            enabled: false,
            idle_timeout_seconds: 0,
            activity_mode: InactivityActivityMode::SystemInputOrScreen,
            last_activity_monotonic_ms: 0,
            is_paused: false,
        }
    }
}

impl InactivityState {
    pub(crate) fn from_recording_settings(
        settings: &RecordingSettings,
        now_monotonic_ms: u64,
    ) -> Self {
        Self {
            enabled: settings.pause_capture_on_inactivity,
            idle_timeout_seconds: settings.idle_timeout_seconds,
            activity_mode: settings.inactivity_activity_mode,
            last_activity_monotonic_ms: now_monotonic_ms,
            is_paused: false,
        }
    }

    fn idle_timeout_ms(&self) -> u64 {
        self.idle_timeout_seconds.saturating_mul(1000)
    }

    fn fallback_idle(&self, now_monotonic_ms: u64) -> ActivitySourceIdle {
        ActivitySourceIdle {
            kind: ActivitySourceKind::InternalFallback,
            idle_ms: now_monotonic_ms.saturating_sub(self.last_activity_monotonic_ms),
        }
    }

    fn idle_for_source_kind(
        _now_monotonic_ms: u64,
        snapshot: ActivitySnapshot,
        kind: ActivitySourceKind,
    ) -> Option<u64> {
        match kind {
            ActivitySourceKind::SystemInput => snapshot.system_input_idle_ms,
            ActivitySourceKind::ScreenCapture => snapshot.screen_activity_idle_ms,
            ActivitySourceKind::InternalFallback => None,
        }
    }

    fn source_kinds_for_mode(&self) -> &'static [ActivitySourceKind] {
        match self.activity_mode {
            InactivityActivityMode::SystemInputOnly => &SYSTEM_INPUT_ONLY_SOURCES,
            InactivityActivityMode::SystemInputOrScreen => &HYBRID_SOURCES,
        }
    }

    fn source_samples(
        &self,
        now_monotonic_ms: u64,
        snapshot: ActivitySnapshot,
    ) -> Vec<ActivitySourceSample> {
        ALL_ACTIVITY_SOURCES
            .iter()
            .map(|kind| {
                let idle_ms = Self::idle_for_source_kind(now_monotonic_ms, snapshot, *kind);
                ActivitySourceSample {
                    kind: *kind,
                    available: idle_ms.is_some(),
                    idle_ms,
                }
            })
            .collect()
    }

    fn sample_idle_for_kind(
        samples: &[ActivitySourceSample],
        kind: ActivitySourceKind,
    ) -> Option<ActivitySourceIdle> {
        samples
            .iter()
            .find(|sample| sample.kind == kind)
            .and_then(|sample| {
                sample.idle_ms.map(|idle_ms| ActivitySourceIdle {
                    kind: sample.kind,
                    idle_ms,
                })
            })
    }

    fn min_idle_from_kinds(
        samples: &[ActivitySourceSample],
        source_kinds: &[ActivitySourceKind],
    ) -> Option<ActivitySourceIdle> {
        source_kinds
            .iter()
            .filter_map(|kind| Self::sample_idle_for_kind(samples, *kind))
            .min_by_key(|sample| sample.idle_ms)
    }

    fn evaluate_policy(
        &self,
        now_monotonic_ms: u64,
        snapshot: ActivitySnapshot,
    ) -> ActivityPolicyEvaluation {
        let sources = self.source_samples(now_monotonic_ms, snapshot);
        let fallback = self.fallback_idle(now_monotonic_ms);

        let selected =
            Self::min_idle_from_kinds(&sources, self.source_kinds_for_mode()).unwrap_or(fallback);

        ActivityPolicyEvaluation {
            effective_idle: EffectiveIdle {
                source: selected.kind,
                idle_ms: selected.idle_ms,
            },
            sources,
        }
    }

    pub(crate) fn should_pause_for_inactivity(
        &self,
        now_monotonic_ms: u64,
        snapshot: ActivitySnapshot,
    ) -> bool {
        if !self.enabled || self.is_paused {
            return false;
        }

        self.evaluate_policy(now_monotonic_ms, snapshot)
            .effective_idle
            .idle_ms
            >= self.idle_timeout_ms()
    }

    pub(crate) fn should_resume_from_inactivity(
        &self,
        now_monotonic_ms: u64,
        snapshot: ActivitySnapshot,
    ) -> bool {
        if !self.enabled || !self.is_paused {
            return false;
        }

        self.evaluate_policy(now_monotonic_ms, snapshot)
            .effective_idle
            .idle_ms
            < self.idle_timeout_ms()
    }

    pub(crate) fn effective_idle_for_snapshot(
        &self,
        now_monotonic_ms: u64,
        snapshot: ActivitySnapshot,
    ) -> EffectiveIdle {
        self.evaluate_policy(now_monotonic_ms, snapshot)
            .effective_idle
    }

    pub(crate) fn evaluate_policy_for_snapshot(
        &self,
        now_monotonic_ms: u64,
        snapshot: ActivitySnapshot,
    ) -> ActivityPolicyEvaluation {
        self.evaluate_policy(now_monotonic_ms, snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capture_types::{
        default_inactivity_activity_mode, default_video_bitrate, InactivityActivityMode,
        RecordingSettings, ScreenResolution, ScreenResolutionPreset,
    };

    #[test]
    fn inactivity_state_triggers_pause_after_timeout() {
        let settings = RecordingSettings {
            capture_screen: true,
            capture_microphone: false,
            capture_system_audio: false,
            segment_duration_seconds: 60,
            screen_frame_rate: 30,
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::Original,
            },
            video_bitrate: default_video_bitrate(),
            save_directory: "/tmp".to_string(),
            auto_start: false,
            pause_capture_on_inactivity: true,
            idle_timeout_seconds: 10,
            inactivity_activity_mode: default_inactivity_activity_mode(),
        };

        let state = InactivityState::from_recording_settings(&settings, 1_000);
        assert!(!state.should_pause_for_inactivity(
            10_999,
            ActivitySnapshot {
                system_input_idle_ms: None,
                screen_activity_idle_ms: None,
            }
        ));
        assert!(state.should_pause_for_inactivity(
            11_000,
            ActivitySnapshot {
                system_input_idle_ms: None,
                screen_activity_idle_ms: None,
            }
        ));
    }

    #[test]
    fn system_idle_is_primary_source_over_reported_activity() {
        let mut state = InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            activity_mode: InactivityActivityMode::SystemInputOnly,
            last_activity_monotonic_ms: 25_000,
            is_paused: false,
        };

        state.last_activity_monotonic_ms = 30_000;
        assert!(state.should_pause_for_inactivity(
            30_500,
            ActivitySnapshot {
                system_input_idle_ms: Some(10_000),
                screen_activity_idle_ms: None,
            }
        ));

        state.is_paused = true;
        assert!(!state.should_resume_from_inactivity(
            31_000,
            ActivitySnapshot {
                system_input_idle_ms: Some(10_000),
                screen_activity_idle_ms: None,
            }
        ));
        assert!(state.should_resume_from_inactivity(
            31_000,
            ActivitySnapshot {
                system_input_idle_ms: Some(2_000),
                screen_activity_idle_ms: None,
            }
        ));
    }

    #[test]
    fn hybrid_mode_uses_less_idle_source() {
        let state = InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            activity_mode: InactivityActivityMode::SystemInputOrScreen,
            last_activity_monotonic_ms: 1_000,
            is_paused: false,
        };

        let effective = state.effective_idle_for_snapshot(
            20_000,
            ActivitySnapshot {
                system_input_idle_ms: Some(12_000),
                screen_activity_idle_ms: Some(1_000),
            },
        );

        assert_eq!(effective.source, ActivitySourceKind::ScreenCapture);
        assert_eq!(effective.idle_ms, 1_000);
    }

    #[test]
    fn policy_evaluation_exposes_sources_with_availability_and_idles() {
        let state = InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            activity_mode: InactivityActivityMode::SystemInputOrScreen,
            last_activity_monotonic_ms: 1_000,
            is_paused: false,
        };

        let evaluation = state.evaluate_policy_for_snapshot(
            20_000,
            ActivitySnapshot {
                system_input_idle_ms: Some(12_000),
                screen_activity_idle_ms: Some(500),
            },
        );

        assert_eq!(evaluation.sources.len(), 2);
        assert_eq!(evaluation.sources[0].kind, ActivitySourceKind::SystemInput);
        assert!(evaluation.sources[0].available);
        assert_eq!(evaluation.sources[0].idle_ms, Some(12_000));
        assert_eq!(
            evaluation.sources[1].kind,
            ActivitySourceKind::ScreenCapture
        );
        assert!(evaluation.sources[1].available);
        assert_eq!(evaluation.sources[1].idle_ms, Some(500));
        assert_eq!(
            evaluation.effective_idle.source,
            ActivitySourceKind::ScreenCapture
        );
        assert_eq!(evaluation.effective_idle.idle_ms, 500);
    }

    #[test]
    fn fallback_idle_uses_monotonic_elapsed_time() {
        let state = InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            activity_mode: InactivityActivityMode::SystemInputOrScreen,
            last_activity_monotonic_ms: 5_000,
            is_paused: false,
        };

        let effective = state.effective_idle_for_snapshot(
            6_250,
            ActivitySnapshot {
                system_input_idle_ms: None,
                screen_activity_idle_ms: None,
            },
        );

        assert_eq!(effective.source, ActivitySourceKind::InternalFallback);
        assert_eq!(effective.idle_ms, 1_250);
    }
}
