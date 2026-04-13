use capture_types::{
    default_audio_activity_sensitivity, InactivityActivityMode, RecordingSettings,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActivitySourceKind {
    SystemInput,
    ScreenCapture,
    MicrophoneCapture,
    SystemAudioCapture,
    InternalFallback,
}

impl ActivitySourceKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ActivitySourceKind::SystemInput => "system_input",
            ActivitySourceKind::ScreenCapture => "screen_capture",
            ActivitySourceKind::MicrophoneCapture => "microphone_capture",
            ActivitySourceKind::SystemAudioCapture => "system_audio_capture",
            ActivitySourceKind::InternalFallback => "internal_fallback",
        }
    }
}

const ALL_ACTIVITY_SOURCES: [ActivitySourceKind; 4] = [
    ActivitySourceKind::SystemInput,
    ActivitySourceKind::ScreenCapture,
    ActivitySourceKind::MicrophoneCapture,
    ActivitySourceKind::SystemAudioCapture,
];
const SYSTEM_INPUT_ONLY_SOURCES: [ActivitySourceKind; 1] = [ActivitySourceKind::SystemInput];
const HYBRID_SOURCES: [ActivitySourceKind; 2] = [
    ActivitySourceKind::SystemInput,
    ActivitySourceKind::ScreenCapture,
];
const AUDIO_HYBRID_SOURCES: [ActivitySourceKind; 4] = [
    ActivitySourceKind::SystemInput,
    ActivitySourceKind::ScreenCapture,
    ActivitySourceKind::MicrophoneCapture,
    ActivitySourceKind::SystemAudioCapture,
];
// Map the 0-100 sensitivity slider onto a bounded normalized audio threshold.
// Higher sensitivity lowers the threshold so quieter audio counts as activity
// more easily, while still avoiding a zero-threshold "everything is active"
// policy.
const MIN_AUDIO_ACTIVITY_THRESHOLD: f32 = 0.05;
const MAX_AUDIO_ACTIVITY_THRESHOLD: f32 = 0.80;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ActivitySourceIdle {
    pub kind: ActivitySourceKind,
    pub idle_ms: u64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ActivitySourceSample {
    pub kind: ActivitySourceKind,
    pub enabled: bool,
    pub available: bool,
    pub idle_ms: Option<u64>,
    pub latest_normalized_level: Option<f32>,
    pub activity_threshold: Option<f32>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AudioActivitySourceState {
    pub enabled: bool,
    pub idle_ms: Option<u64>,
    pub latest_normalized_level: Option<f32>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ActivitySnapshot {
    pub system_input_idle_ms: Option<u64>,
    pub screen_activity_idle_ms: Option<u64>,
    pub microphone_activity: AudioActivitySourceState,
    pub system_audio_activity: AudioActivitySourceState,
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
    pub audio_activity_sensitivity: u8,
    pub activity_mode: InactivityActivityMode,
    pub last_activity_monotonic_ms: u64,
    pub last_microphone_activity_monotonic_ms: Option<u64>,
    pub last_system_audio_activity_monotonic_ms: Option<u64>,
    pub is_paused: bool,
}

impl Default for InactivityState {
    fn default() -> Self {
        Self {
            enabled: false,
            idle_timeout_seconds: 0,
            audio_activity_sensitivity: default_audio_activity_sensitivity(),
            activity_mode: InactivityActivityMode::SystemInputOrScreen,
            last_activity_monotonic_ms: 0,
            last_microphone_activity_monotonic_ms: None,
            last_system_audio_activity_monotonic_ms: None,
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
            audio_activity_sensitivity: settings.audio_activity_sensitivity,
            activity_mode: settings.inactivity_activity_mode,
            last_activity_monotonic_ms: now_monotonic_ms,
            last_microphone_activity_monotonic_ms: None,
            last_system_audio_activity_monotonic_ms: None,
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

    pub(crate) fn audio_activity_threshold(&self) -> f32 {
        let sensitivity_fraction = (self.audio_activity_sensitivity.min(100) as f32) / 100.0;
        MAX_AUDIO_ACTIVITY_THRESHOLD
            - (sensitivity_fraction * (MAX_AUDIO_ACTIVITY_THRESHOLD - MIN_AUDIO_ACTIVITY_THRESHOLD))
    }

    fn audio_source_sample(
        now_monotonic_ms: u64,
        kind: ActivitySourceKind,
        source: AudioActivitySourceState,
        activity_threshold: f32,
        last_threshold_qualified_monotonic_ms: &mut Option<u64>,
    ) -> ActivitySourceSample {
        let currently_above_threshold = source.enabled
            && source.idle_ms.is_some()
            && source
                .latest_normalized_level
                .is_some_and(|level| level >= activity_threshold);

        if currently_above_threshold {
            *last_threshold_qualified_monotonic_ms = source
                .idle_ms
                .map(|idle_ms| now_monotonic_ms.saturating_sub(idle_ms));
        } else if !source.enabled {
            *last_threshold_qualified_monotonic_ms = None;
        }

        let idle_ms = if source.enabled {
            last_threshold_qualified_monotonic_ms
                .map(|last_activity_ms| now_monotonic_ms.saturating_sub(last_activity_ms))
        } else {
            None
        };
        let available = source.enabled && idle_ms.is_some();

        ActivitySourceSample {
            kind,
            enabled: source.enabled,
            available,
            idle_ms,
            latest_normalized_level: source.latest_normalized_level,
            activity_threshold: Some(activity_threshold),
        }
    }

    fn source_kinds_for_mode(&self) -> &'static [ActivitySourceKind] {
        match self.activity_mode {
            InactivityActivityMode::SystemInputOnly => &SYSTEM_INPUT_ONLY_SOURCES,
            InactivityActivityMode::SystemInputOrScreen => &HYBRID_SOURCES,
            InactivityActivityMode::SystemInputOrScreenOrAudio => &AUDIO_HYBRID_SOURCES,
        }
    }

    fn source_samples(
        &mut self,
        now_monotonic_ms: u64,
        snapshot: ActivitySnapshot,
    ) -> Vec<ActivitySourceSample> {
        let activity_threshold = self.audio_activity_threshold();
        let mut sources = Vec::with_capacity(ALL_ACTIVITY_SOURCES.len());

        for kind in ALL_ACTIVITY_SOURCES {
            sources.push(match kind {
                ActivitySourceKind::SystemInput => ActivitySourceSample {
                    kind,
                    enabled: true,
                    available: snapshot.system_input_idle_ms.is_some(),
                    idle_ms: snapshot.system_input_idle_ms,
                    latest_normalized_level: None,
                    activity_threshold: None,
                },
                ActivitySourceKind::ScreenCapture => ActivitySourceSample {
                    kind,
                    enabled: true,
                    available: snapshot.screen_activity_idle_ms.is_some(),
                    idle_ms: snapshot.screen_activity_idle_ms,
                    latest_normalized_level: None,
                    activity_threshold: None,
                },
                ActivitySourceKind::MicrophoneCapture => Self::audio_source_sample(
                    now_monotonic_ms,
                    kind,
                    snapshot.microphone_activity,
                    activity_threshold,
                    &mut self.last_microphone_activity_monotonic_ms,
                ),
                ActivitySourceKind::SystemAudioCapture => Self::audio_source_sample(
                    now_monotonic_ms,
                    kind,
                    snapshot.system_audio_activity,
                    activity_threshold,
                    &mut self.last_system_audio_activity_monotonic_ms,
                ),
                ActivitySourceKind::InternalFallback => ActivitySourceSample {
                    kind,
                    enabled: false,
                    available: false,
                    idle_ms: None,
                    latest_normalized_level: None,
                    activity_threshold: None,
                },
            });
        }

        sources
    }

    fn sample_idle_for_kind(
        samples: &[ActivitySourceSample],
        kind: ActivitySourceKind,
    ) -> Option<ActivitySourceIdle> {
        samples
            .iter()
            .find(|sample| sample.kind == kind)
            .and_then(|sample| {
                sample.available.then_some(())?;
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
        &mut self,
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
        &mut self,
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
        &mut self,
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
        &mut self,
        now_monotonic_ms: u64,
        snapshot: ActivitySnapshot,
    ) -> EffectiveIdle {
        self.evaluate_policy(now_monotonic_ms, snapshot)
            .effective_idle
    }

    pub(crate) fn evaluate_policy_for_snapshot(
        &mut self,
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
            screen_activity_idle_ms: None,
            microphone_activity: empty_audio_activity(),
            system_audio_activity: empty_audio_activity(),
        }
    }

    fn assert_approx_eq(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 0.000_1);
    }

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
            native_capture_debug_logging_enabled: false,
            pause_capture_on_inactivity: true,
            idle_timeout_seconds: 10,
            audio_activity_sensitivity: 50,
            inactivity_activity_mode: default_inactivity_activity_mode(),
        };

        let mut state = InactivityState::from_recording_settings(&settings, 1_000);
        assert!(!state.should_pause_for_inactivity(
            10_999,
            ActivitySnapshot {
                system_input_idle_ms: None,
                screen_activity_idle_ms: None,
                microphone_activity: empty_audio_activity(),
                system_audio_activity: empty_audio_activity(),
            }
        ));
        assert!(state.should_pause_for_inactivity(
            11_000,
            ActivitySnapshot {
                system_input_idle_ms: None,
                screen_activity_idle_ms: None,
                microphone_activity: empty_audio_activity(),
                system_audio_activity: empty_audio_activity(),
            }
        ));
    }

    #[test]
    fn system_idle_is_primary_source_over_reported_activity() {
        let mut state = InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            audio_activity_sensitivity: 50,
            activity_mode: InactivityActivityMode::SystemInputOnly,
            last_activity_monotonic_ms: 25_000,
            last_microphone_activity_monotonic_ms: None,
            last_system_audio_activity_monotonic_ms: None,
            is_paused: false,
        };

        state.last_activity_monotonic_ms = 30_000;
        assert!(state.should_pause_for_inactivity(
            30_500,
            ActivitySnapshot {
                system_input_idle_ms: Some(10_000),
                screen_activity_idle_ms: None,
                ..empty_activity_snapshot()
            }
        ));

        state.is_paused = true;
        assert!(!state.should_resume_from_inactivity(
            31_000,
            ActivitySnapshot {
                system_input_idle_ms: Some(10_000),
                screen_activity_idle_ms: None,
                ..empty_activity_snapshot()
            }
        ));
        assert!(state.should_resume_from_inactivity(
            31_000,
            ActivitySnapshot {
                system_input_idle_ms: Some(2_000),
                screen_activity_idle_ms: None,
                ..empty_activity_snapshot()
            }
        ));
    }

    #[test]
    fn hybrid_mode_uses_less_idle_source() {
        let mut state = InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            audio_activity_sensitivity: 50,
            activity_mode: InactivityActivityMode::SystemInputOrScreen,
            last_activity_monotonic_ms: 1_000,
            last_microphone_activity_monotonic_ms: None,
            last_system_audio_activity_monotonic_ms: None,
            is_paused: false,
        };

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
    fn policy_evaluation_exposes_sources_with_availability_and_idles() {
        let mut state = InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            audio_activity_sensitivity: 50,
            activity_mode: InactivityActivityMode::SystemInputOrScreen,
            last_activity_monotonic_ms: 1_000,
            last_microphone_activity_monotonic_ms: None,
            last_system_audio_activity_monotonic_ms: None,
            is_paused: false,
        };

        let evaluation = state.evaluate_policy_for_snapshot(
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
            0.425,
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
            0.425,
        );
        assert_eq!(
            evaluation.effective_idle.source,
            ActivitySourceKind::ScreenCapture
        );
        assert_eq!(evaluation.effective_idle.idle_ms, 500);
    }

    #[test]
    fn fallback_idle_uses_monotonic_elapsed_time() {
        let mut state = InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            audio_activity_sensitivity: 50,
            activity_mode: InactivityActivityMode::SystemInputOrScreen,
            last_activity_monotonic_ms: 5_000,
            last_microphone_activity_monotonic_ms: None,
            last_system_audio_activity_monotonic_ms: None,
            is_paused: false,
        };

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
    fn audio_mode_uses_audio_source_with_lowest_idle_when_level_meets_threshold() {
        let mut state = InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            audio_activity_sensitivity: 100,
            activity_mode: InactivityActivityMode::SystemInputOrScreenOrAudio,
            last_activity_monotonic_ms: 1_000,
            last_microphone_activity_monotonic_ms: None,
            last_system_audio_activity_monotonic_ms: None,
            is_paused: false,
        };

        let evaluation = state.evaluate_policy_for_snapshot(
            20_000,
            ActivitySnapshot {
                system_input_idle_ms: Some(12_000),
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
            0.05,
        );
        assert_approx_eq(
            evaluation.sources[3]
                .activity_threshold
                .expect("system-audio threshold should be present"),
            0.05,
        );
    }

    #[test]
    fn audio_mode_ignores_audio_sources_below_threshold_or_when_disabled() {
        let mut state = InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            audio_activity_sensitivity: 0,
            activity_mode: InactivityActivityMode::SystemInputOrScreenOrAudio,
            last_activity_monotonic_ms: 1_000,
            last_microphone_activity_monotonic_ms: None,
            last_system_audio_activity_monotonic_ms: None,
            is_paused: false,
        };

        let evaluation = state.evaluate_policy_for_snapshot(
            20_000,
            ActivitySnapshot {
                system_input_idle_ms: Some(12_000),
                screen_activity_idle_ms: Some(11_000),
                microphone_activity: AudioActivitySourceState {
                    enabled: true,
                    idle_ms: Some(500),
                    latest_normalized_level: Some(0.79),
                },
                system_audio_activity: AudioActivitySourceState {
                    enabled: false,
                    idle_ms: Some(400),
                    latest_normalized_level: Some(1.0),
                },
            },
        );

        assert_eq!(
            evaluation.effective_idle.source,
            ActivitySourceKind::ScreenCapture
        );
        assert_eq!(evaluation.effective_idle.idle_ms, 11_000);
        assert!(!evaluation.sources[2].available);
        assert_eq!(evaluation.sources[2].idle_ms, None);
        assert_eq!(evaluation.sources[2].latest_normalized_level, Some(0.79));
        assert_approx_eq(
            evaluation.sources[2]
                .activity_threshold
                .expect("microphone threshold should be present"),
            0.8,
        );
        assert!(!evaluation.sources[3].enabled);
        assert!(!evaluation.sources[3].available);
        assert_eq!(evaluation.sources[3].idle_ms, None);
        assert_eq!(evaluation.sources[3].latest_normalized_level, Some(1.0));
    }

    #[test]
    fn audio_mode_keeps_counting_idle_from_last_threshold_qualified_audio_activity() {
        let mut state = InactivityState {
            enabled: true,
            idle_timeout_seconds: 10,
            audio_activity_sensitivity: 100,
            activity_mode: InactivityActivityMode::SystemInputOrScreenOrAudio,
            last_activity_monotonic_ms: 1_000,
            last_microphone_activity_monotonic_ms: None,
            last_system_audio_activity_monotonic_ms: None,
            is_paused: false,
        };

        let first_evaluation = state.evaluate_policy_for_snapshot(
            20_000,
            ActivitySnapshot {
                system_input_idle_ms: Some(15_000),
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

        let second_evaluation = state.evaluate_policy_for_snapshot(
            20_100,
            ActivitySnapshot {
                system_input_idle_ms: Some(15_100),
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
        assert!(!state.should_pause_for_inactivity(
            20_100,
            ActivitySnapshot {
                system_input_idle_ms: Some(15_100),
                screen_activity_idle_ms: Some(10_100),
                microphone_activity: AudioActivitySourceState {
                    enabled: true,
                    idle_ms: Some(0),
                    latest_normalized_level: Some(0.0),
                },
                system_audio_activity: empty_audio_activity(),
            }
        ));
        assert!(state.should_pause_for_inactivity(
            30_001,
            ActivitySnapshot {
                system_input_idle_ms: Some(25_001),
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
}
