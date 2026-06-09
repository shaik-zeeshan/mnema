use capture_types::{
    default_microphone_activity_sensitivity, default_system_audio_activity_sensitivity,
    InactivityActivityMode, RecordingSettings,
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
const MICROPHONE_ONLY_SOURCES: [ActivitySourceKind; 1] = [ActivitySourceKind::MicrophoneCapture];
const SYSTEM_AUDIO_ONLY_SOURCES: [ActivitySourceKind; 1] = [ActivitySourceKind::SystemAudioCapture];
// Map the 0-100 sensitivity slider onto a bounded normalized audio threshold.
// Higher sensitivity lowers the threshold so quieter audio counts as activity
// more easily, while still avoiding a zero-threshold "everything is active"
// policy. Microphone peaks are calibrated around conversational speech, while
// ScreenCaptureKit system-audio peaks are usually lower and need a quieter
// threshold to avoid pausing while media is audibly playing.
const MIN_MICROPHONE_ACTIVITY_THRESHOLD: f32 = 0.01;
const MAX_MICROPHONE_ACTIVITY_THRESHOLD: f32 = 0.15;
const MIN_SYSTEM_AUDIO_ACTIVITY_THRESHOLD: f32 = 0.002;
const MAX_SYSTEM_AUDIO_ACTIVITY_THRESHOLD: f32 = 0.05;
const SCREEN_RESUME_MIN_PAUSED_MS: u64 = 2_000;

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
    pub screen_activity_enabled: bool,
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
pub(crate) struct ActivityPolicyEvaluations {
    pub screen: ActivityPolicyEvaluation,
    pub microphone: ActivityPolicyEvaluation,
    pub system_audio: ActivityPolicyEvaluation,
}

#[derive(Debug, Clone)]
pub(crate) struct InactivityState {
    pub enabled: bool,
    pub idle_timeout_seconds: u64,
    pub microphone_activity_sensitivity: u8,
    pub system_audio_activity_sensitivity: u8,
    pub activity_mode: InactivityActivityMode,
    pub last_activity_monotonic_ms: u64,
    pub last_microphone_activity_monotonic_ms: Option<u64>,
    pub last_system_audio_activity_monotonic_ms: Option<u64>,
    pub screen_paused_at_monotonic_ms: Option<u64>,
    pub screen_paused: bool,
    pub microphone_paused: bool,
    pub system_audio_paused: bool,
    pub is_paused: bool,
}

impl Default for InactivityState {
    fn default() -> Self {
        Self {
            enabled: false,
            idle_timeout_seconds: 0,
            microphone_activity_sensitivity: default_microphone_activity_sensitivity(),
            system_audio_activity_sensitivity: default_system_audio_activity_sensitivity(),
            activity_mode: InactivityActivityMode::SystemInputOrScreen,
            last_activity_monotonic_ms: 0,
            last_microphone_activity_monotonic_ms: None,
            last_system_audio_activity_monotonic_ms: None,
            screen_paused_at_monotonic_ms: None,
            screen_paused: false,
            microphone_paused: false,
            system_audio_paused: false,
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
            microphone_activity_sensitivity: settings.microphone_activity_sensitivity,
            system_audio_activity_sensitivity: settings.system_audio_activity_sensitivity,
            activity_mode: settings.inactivity_activity_mode,
            last_activity_monotonic_ms: now_monotonic_ms,
            last_microphone_activity_monotonic_ms: None,
            last_system_audio_activity_monotonic_ms: None,
            screen_paused_at_monotonic_ms: None,
            screen_paused: false,
            microphone_paused: false,
            system_audio_paused: false,
            is_paused: false,
        }
    }

    pub(crate) fn set_family_paused_states(
        &mut self,
        screen_paused: bool,
        microphone_paused: bool,
        system_audio_paused: bool,
    ) {
        self.screen_paused = screen_paused;
        if !screen_paused {
            self.screen_paused_at_monotonic_ms = None;
        }
        self.microphone_paused = microphone_paused;
        self.system_audio_paused = system_audio_paused;
        self.is_paused = screen_paused || microphone_paused || system_audio_paused;
    }

    pub(crate) fn mark_screen_pause_started(&mut self, now_monotonic_ms: u64) {
        self.screen_paused_at_monotonic_ms = Some(now_monotonic_ms);
    }

    fn has_legacy_global_pause_state(&self) -> bool {
        self.is_paused
            && !self.screen_paused
            && !self.microphone_paused
            && !self.system_audio_paused
    }

    pub(crate) fn is_screen_paused(&self) -> bool {
        self.screen_paused || self.has_legacy_global_pause_state()
    }

    pub(crate) fn is_microphone_paused(&self) -> bool {
        self.microphone_paused || self.has_legacy_global_pause_state()
    }

    pub(crate) fn is_system_audio_paused(&self) -> bool {
        self.system_audio_paused || self.has_legacy_global_pause_state()
    }

    /// Returns true when either microphone or system audio is paused.
    /// This is kept only for crate tests that assert family pause state.
    #[cfg(test)]
    pub(crate) fn is_any_audio_paused(&self) -> bool {
        self.is_microphone_paused() || self.is_system_audio_paused()
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

    pub(crate) fn microphone_activity_threshold(&self) -> f32 {
        Self::sensitivity_to_threshold(
            self.microphone_activity_sensitivity,
            MIN_MICROPHONE_ACTIVITY_THRESHOLD,
            MAX_MICROPHONE_ACTIVITY_THRESHOLD,
        )
    }

    pub(crate) fn system_audio_activity_threshold(&self) -> f32 {
        Self::sensitivity_to_threshold(
            self.system_audio_activity_sensitivity,
            MIN_SYSTEM_AUDIO_ACTIVITY_THRESHOLD,
            MAX_SYSTEM_AUDIO_ACTIVITY_THRESHOLD,
        )
    }

    fn sensitivity_to_threshold(sensitivity: u8, min_threshold: f32, max_threshold: f32) -> f32 {
        let sensitivity_fraction = (sensitivity.min(100) as f32) / 100.0;
        max_threshold - (sensitivity_fraction * (max_threshold - min_threshold))
    }

    fn threshold_audio_source_sample(
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

    fn decision_audio_source_sample(
        now_monotonic_ms: u64,
        kind: ActivitySourceKind,
        source: AudioActivitySourceState,
        activity_threshold: f32,
        last_decision_qualified_monotonic_ms: &mut Option<u64>,
    ) -> ActivitySourceSample {
        let decision_qualified = source.enabled
            && source
                .latest_normalized_level
                .is_some_and(|level| level >= activity_threshold);

        if decision_qualified {
            *last_decision_qualified_monotonic_ms =
                Some(now_monotonic_ms.saturating_sub(source.idle_ms.unwrap_or(0)));
        } else if !source.enabled {
            *last_decision_qualified_monotonic_ms = None;
        }

        let idle_ms = if source.enabled {
            last_decision_qualified_monotonic_ms
                .map(|last_activity_ms| now_monotonic_ms.saturating_sub(last_activity_ms))
        } else {
            None
        };

        ActivitySourceSample {
            kind,
            enabled: source.enabled,
            available: source.enabled && idle_ms.is_some(),
            idle_ms,
            latest_normalized_level: source.latest_normalized_level,
            activity_threshold: Some(activity_threshold),
        }
    }

    fn screen_source_kinds_for_mode(&self) -> &'static [ActivitySourceKind] {
        match self.activity_mode {
            InactivityActivityMode::SystemInputOnly => &SYSTEM_INPUT_ONLY_SOURCES,
            InactivityActivityMode::SystemInputOrScreen => &HYBRID_SOURCES,
            InactivityActivityMode::SystemInputOrScreenOrAudio => &HYBRID_SOURCES,
        }
    }

    fn microphone_source_kinds_for_mode(&self) -> &'static [ActivitySourceKind] {
        match self.activity_mode {
            InactivityActivityMode::SystemInputOnly => &MICROPHONE_ONLY_SOURCES,
            InactivityActivityMode::SystemInputOrScreen => &MICROPHONE_ONLY_SOURCES,
            InactivityActivityMode::SystemInputOrScreenOrAudio => &MICROPHONE_ONLY_SOURCES,
        }
    }

    fn system_audio_source_kinds_for_mode(&self) -> &'static [ActivitySourceKind] {
        match self.activity_mode {
            InactivityActivityMode::SystemInputOnly => &SYSTEM_AUDIO_ONLY_SOURCES,
            InactivityActivityMode::SystemInputOrScreen => &SYSTEM_AUDIO_ONLY_SOURCES,
            InactivityActivityMode::SystemInputOrScreenOrAudio => &SYSTEM_AUDIO_ONLY_SOURCES,
        }
    }

    fn combined_source_kinds_for_mode(&self) -> &'static [ActivitySourceKind] {
        match self.activity_mode {
            InactivityActivityMode::SystemInputOnly => &SYSTEM_INPUT_ONLY_SOURCES,
            InactivityActivityMode::SystemInputOrScreen => &HYBRID_SOURCES,
            InactivityActivityMode::SystemInputOrScreenOrAudio => &ALL_ACTIVITY_SOURCES,
        }
    }

    fn source_samples(
        &mut self,
        now_monotonic_ms: u64,
        snapshot: ActivitySnapshot,
    ) -> Vec<ActivitySourceSample> {
        let system_audio_threshold = self.system_audio_activity_threshold();
        let microphone_threshold = self.microphone_activity_threshold();
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
                ActivitySourceKind::MicrophoneCapture => Self::decision_audio_source_sample(
                    now_monotonic_ms,
                    kind,
                    snapshot.microphone_activity,
                    microphone_threshold,
                    &mut self.last_microphone_activity_monotonic_ms,
                ),
                ActivitySourceKind::SystemAudioCapture => Self::threshold_audio_source_sample(
                    now_monotonic_ms,
                    kind,
                    snapshot.system_audio_activity,
                    system_audio_threshold,
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

    fn evaluate_policy_from_samples(
        sources: &[ActivitySourceSample],
        fallback: ActivitySourceIdle,
        source_kinds: &[ActivitySourceKind],
    ) -> ActivityPolicyEvaluation {
        let selected = Self::min_idle_from_kinds(sources, source_kinds).unwrap_or(fallback);

        ActivityPolicyEvaluation {
            effective_idle: EffectiveIdle {
                source: selected.kind,
                idle_ms: selected.idle_ms,
            },
            sources: sources.to_vec(),
        }
    }

    pub(crate) fn evaluate_policies_for_snapshot(
        &mut self,
        now_monotonic_ms: u64,
        snapshot: ActivitySnapshot,
    ) -> ActivityPolicyEvaluations {
        let sources = self.source_samples(now_monotonic_ms, snapshot);
        let fallback = self.fallback_idle(now_monotonic_ms);

        ActivityPolicyEvaluations {
            screen: Self::evaluate_policy_from_samples(
                &sources,
                fallback,
                self.screen_source_kinds_for_mode(),
            ),
            microphone: Self::evaluate_policy_from_samples(
                &sources,
                fallback,
                self.microphone_source_kinds_for_mode(),
            ),
            system_audio: Self::evaluate_policy_from_samples(
                &sources,
                fallback,
                self.system_audio_source_kinds_for_mode(),
            ),
        }
    }

    pub(crate) fn should_pause_screen_for_inactivity(
        &mut self,
        now_monotonic_ms: u64,
        snapshot: ActivitySnapshot,
    ) -> bool {
        if !self.enabled || self.is_screen_paused() || !snapshot.screen_activity_enabled {
            return false;
        }

        self.evaluate_screen_policy_for_snapshot(now_monotonic_ms, snapshot)
            .effective_idle
            .idle_ms
            >= self.idle_timeout_ms()
    }

    pub(crate) fn should_resume_screen_from_inactivity(
        &mut self,
        now_monotonic_ms: u64,
        snapshot: ActivitySnapshot,
    ) -> bool {
        if !self.enabled || !self.is_screen_paused() || !snapshot.screen_activity_enabled {
            return false;
        }
        if !self.screen_resume_guard_elapsed(now_monotonic_ms) {
            return false;
        }

        self.evaluate_screen_policy_for_snapshot(now_monotonic_ms, snapshot)
            .effective_idle
            .idle_ms
            < self.idle_timeout_ms()
    }

    fn screen_resume_guard_elapsed(&self, now_monotonic_ms: u64) -> bool {
        self.screen_paused_at_monotonic_ms
            .map(|paused_at| {
                now_monotonic_ms.saturating_sub(paused_at) >= SCREEN_RESUME_MIN_PAUSED_MS
            })
            .unwrap_or(true)
    }

    pub(crate) fn should_pause_microphone_for_inactivity(
        &mut self,
        now_monotonic_ms: u64,
        snapshot: ActivitySnapshot,
    ) -> bool {
        if !self.enabled || self.is_microphone_paused() || !snapshot.microphone_activity.enabled {
            return false;
        }

        self.evaluate_microphone_policy_for_snapshot(now_monotonic_ms, snapshot)
            .effective_idle
            .idle_ms
            >= self.idle_timeout_ms()
    }

    pub(crate) fn should_resume_microphone_from_inactivity(
        &mut self,
        now_monotonic_ms: u64,
        snapshot: ActivitySnapshot,
    ) -> bool {
        if !self.enabled || !self.is_microphone_paused() || !snapshot.microphone_activity.enabled {
            return false;
        }

        self.evaluate_microphone_policy_for_snapshot(now_monotonic_ms, snapshot)
            .effective_idle
            .idle_ms
            < self.idle_timeout_ms()
    }

    pub(crate) fn should_pause_system_audio_for_inactivity(
        &mut self,
        now_monotonic_ms: u64,
        snapshot: ActivitySnapshot,
    ) -> bool {
        if !self.enabled || self.is_system_audio_paused() || !snapshot.system_audio_activity.enabled
        {
            return false;
        }

        self.evaluate_system_audio_policy_for_snapshot(now_monotonic_ms, snapshot)
            .effective_idle
            .idle_ms
            >= self.idle_timeout_ms()
    }

    pub(crate) fn should_resume_system_audio_from_inactivity(
        &mut self,
        now_monotonic_ms: u64,
        snapshot: ActivitySnapshot,
    ) -> bool {
        if !self.enabled
            || !self.is_system_audio_paused()
            || !snapshot.system_audio_activity.enabled
        {
            return false;
        }

        self.evaluate_system_audio_policy_for_snapshot(now_monotonic_ms, snapshot)
            .effective_idle
            .idle_ms
            < self.idle_timeout_ms()
    }

    pub(crate) fn effective_idle_for_snapshot(
        &mut self,
        now_monotonic_ms: u64,
        snapshot: ActivitySnapshot,
    ) -> EffectiveIdle {
        self.evaluate_policy_for_snapshot(now_monotonic_ms, snapshot)
            .effective_idle
    }

    pub(crate) fn evaluate_policy_for_snapshot(
        &mut self,
        now_monotonic_ms: u64,
        snapshot: ActivitySnapshot,
    ) -> ActivityPolicyEvaluation {
        let sources = self.source_samples(now_monotonic_ms, snapshot);
        let fallback = self.fallback_idle(now_monotonic_ms);

        Self::evaluate_policy_from_samples(
            &sources,
            fallback,
            self.combined_source_kinds_for_mode(),
        )
    }

    pub(crate) fn should_pause_for_inactivity(
        &mut self,
        now_monotonic_ms: u64,
        snapshot: ActivitySnapshot,
    ) -> bool {
        // Legacy global pause is superseded by per-family pause/resume in audio mode.
        if !self.enabled
            || self.is_paused
            || self.activity_mode == InactivityActivityMode::SystemInputOrScreenOrAudio
        {
            return false;
        }

        let idle_timeout_ms = self.idle_timeout_ms();
        let evaluations = self.evaluate_policies_for_snapshot(now_monotonic_ms, snapshot);
        let microphone_active = snapshot.microphone_activity.enabled
            && evaluations.microphone.effective_idle.idle_ms < idle_timeout_ms;
        let system_audio_active = snapshot.system_audio_activity.enabled
            && evaluations.system_audio.effective_idle.idle_ms < idle_timeout_ms;

        // In legacy hybrid mode, screen/system-input idle still owns the video
        // pause, but threshold-qualified audio activity must not escalate that
        // screen pause into the old all-source pause path.
        if self.activity_mode == InactivityActivityMode::SystemInputOrScreen
            && (microphone_active || system_audio_active)
        {
            return false;
        }

        Self::evaluate_policy_from_samples(
            &evaluations.screen.sources,
            self.fallback_idle(now_monotonic_ms),
            self.combined_source_kinds_for_mode(),
        )
        .effective_idle
        .idle_ms
            >= idle_timeout_ms
    }

    pub(crate) fn should_resume_from_inactivity(
        &mut self,
        now_monotonic_ms: u64,
        snapshot: ActivitySnapshot,
    ) -> bool {
        // Only resume via legacy path when the pause was a legacy global pause
        // (is_paused=true with no per-family flags set). Per-family pauses set
        // is_paused=true through set_family_paused_states, but should only be
        // cleared by their own per-family resume handlers.
        if !self.enabled || !self.has_legacy_global_pause_state() {
            return false;
        }

        self.evaluate_policy_for_snapshot(now_monotonic_ms, snapshot)
            .effective_idle
            .idle_ms
            < self.idle_timeout_ms()
    }

    pub(crate) fn evaluate_screen_policy_for_snapshot(
        &mut self,
        now_monotonic_ms: u64,
        snapshot: ActivitySnapshot,
    ) -> ActivityPolicyEvaluation {
        self.evaluate_policies_for_snapshot(now_monotonic_ms, snapshot)
            .screen
    }

    pub(crate) fn evaluate_microphone_policy_for_snapshot(
        &mut self,
        now_monotonic_ms: u64,
        snapshot: ActivitySnapshot,
    ) -> ActivityPolicyEvaluation {
        self.evaluate_policies_for_snapshot(now_monotonic_ms, snapshot)
            .microphone
    }

    pub(crate) fn evaluate_system_audio_policy_for_snapshot(
        &mut self,
        now_monotonic_ms: u64,
        snapshot: ActivitySnapshot,
    ) -> ActivityPolicyEvaluation {
        self.evaluate_policies_for_snapshot(now_monotonic_ms, snapshot)
            .system_audio
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capture_types::{
        default_appearance, default_audio_speech_detection_settings,
        default_audio_transcription_settings, default_inactivity_activity_mode,
        default_metadata_settings, default_microphone_vad_adapter, default_privacy_settings,
        default_video_bitrate, InactivityActivityMode, RecordingSettings, ScreenResolution,
        ScreenResolutionPreset,
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
            screen_frame_rate: 30,
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::Original,
            },
            video_bitrate: default_video_bitrate(),
            save_directory: "/tmp".to_string(),
            auto_start: false,
            native_capture_debug_logging_enabled: false,
            developer_options_enabled: false,
            preview_cache_ttl_seconds: capture_types::default_preview_cache_ttl_seconds(),
            follow_timeline_live: false,
            retention_policy: capture_types::default_retention_policy(),
            appearance: default_appearance(),
            ocr: capture_types::default_ocr_settings(),
            transcription: default_audio_transcription_settings(),
            speaker_analysis: capture_types::default_speaker_analysis_settings(),
            audio_speech_detection: default_audio_speech_detection_settings(),
            metadata: default_metadata_settings(),
            privacy: default_privacy_settings(),
            access: capture_types::AccessSettings::default(),
            ai_runtime: capture_types::AiRuntimeSettings::default(),
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
        let mut state =
            inactivity_state_fixture(InactivityActivityMode::SystemInputOrScreenOrAudio, 0);

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
    fn full_mode_recent_system_input_does_not_resume_paused_system_audio_without_system_audio_activity(
    ) {
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
        let mut system_audio_state =
            inactivity_state_fixture(default_inactivity_activity_mode(), 50);
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
}
