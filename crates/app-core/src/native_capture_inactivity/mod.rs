use capture_types::{
    default_microphone_activity_sensitivity, default_system_audio_activity_sensitivity,
    InactivityActivityMode, RecordingSettings,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivitySourceKind {
    SystemInput,
    ScreenCapture,
    MicrophoneCapture,
    SystemAudioCapture,
    InternalFallback,
}

impl ActivitySourceKind {
    pub fn as_str(self) -> &'static str {
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
pub struct ActivitySourceIdle {
    pub kind: ActivitySourceKind,
    pub idle_ms: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct ActivitySourceSample {
    pub kind: ActivitySourceKind,
    pub enabled: bool,
    pub available: bool,
    pub idle_ms: Option<u64>,
    pub latest_normalized_level: Option<f32>,
    pub activity_threshold: Option<f32>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AudioActivitySourceState {
    pub enabled: bool,
    pub idle_ms: Option<u64>,
    pub latest_normalized_level: Option<f32>,
}

#[derive(Debug, Clone, Copy)]
pub struct ActivitySnapshot {
    pub system_input_idle_ms: Option<u64>,
    pub screen_activity_enabled: bool,
    pub screen_activity_idle_ms: Option<u64>,
    pub microphone_activity: AudioActivitySourceState,
    pub system_audio_activity: AudioActivitySourceState,
}

#[derive(Debug, Clone, Copy)]
pub struct EffectiveIdle {
    pub source: ActivitySourceKind,
    pub idle_ms: u64,
}

#[derive(Debug, Clone)]
pub struct ActivityPolicyEvaluation {
    pub effective_idle: EffectiveIdle,
    pub sources: Vec<ActivitySourceSample>,
}

#[derive(Debug, Clone)]
pub struct ActivityPolicyEvaluations {
    pub screen: ActivityPolicyEvaluation,
    pub microphone: ActivityPolicyEvaluation,
    pub system_audio: ActivityPolicyEvaluation,
}

#[derive(Debug, Clone)]
pub struct InactivityState {
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
    pub fn from_recording_settings(settings: &RecordingSettings, now_monotonic_ms: u64) -> Self {
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

    pub fn set_family_paused_states(
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

    pub fn mark_screen_pause_started(&mut self, now_monotonic_ms: u64) {
        self.screen_paused_at_monotonic_ms = Some(now_monotonic_ms);
    }

    fn has_legacy_global_pause_state(&self) -> bool {
        self.is_paused
            && !self.screen_paused
            && !self.microphone_paused
            && !self.system_audio_paused
    }

    pub fn is_screen_paused(&self) -> bool {
        self.screen_paused || self.has_legacy_global_pause_state()
    }

    pub fn is_microphone_paused(&self) -> bool {
        self.microphone_paused || self.has_legacy_global_pause_state()
    }

    pub fn is_system_audio_paused(&self) -> bool {
        self.system_audio_paused || self.has_legacy_global_pause_state()
    }

    /// Returns true when either microphone or system audio is paused.
    /// This is kept only for the desktop crate's tests that assert family pause
    /// state (not `cfg(test)`: that flag doesn't cross the crate boundary).
    pub fn is_any_audio_paused(&self) -> bool {
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

    pub fn microphone_activity_threshold(&self) -> f32 {
        Self::sensitivity_to_threshold(
            self.microphone_activity_sensitivity,
            MIN_MICROPHONE_ACTIVITY_THRESHOLD,
            MAX_MICROPHONE_ACTIVITY_THRESHOLD,
        )
    }

    pub fn system_audio_activity_threshold(&self) -> f32 {
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

    pub fn evaluate_policies_for_snapshot(
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

    pub fn should_pause_screen_for_inactivity(
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

    pub fn should_resume_screen_from_inactivity(
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

    pub fn should_pause_microphone_for_inactivity(
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

    pub fn should_resume_microphone_from_inactivity(
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

    pub fn should_pause_system_audio_for_inactivity(
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

    pub fn should_resume_system_audio_from_inactivity(
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

    pub fn effective_idle_for_snapshot(
        &mut self,
        now_monotonic_ms: u64,
        snapshot: ActivitySnapshot,
    ) -> EffectiveIdle {
        self.evaluate_policy_for_snapshot(now_monotonic_ms, snapshot)
            .effective_idle
    }

    pub fn evaluate_policy_for_snapshot(
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

    pub fn should_pause_for_inactivity(
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

    pub fn should_resume_from_inactivity(
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

    pub fn evaluate_screen_policy_for_snapshot(
        &mut self,
        now_monotonic_ms: u64,
        snapshot: ActivitySnapshot,
    ) -> ActivityPolicyEvaluation {
        self.evaluate_policies_for_snapshot(now_monotonic_ms, snapshot)
            .screen
    }

    pub fn evaluate_microphone_policy_for_snapshot(
        &mut self,
        now_monotonic_ms: u64,
        snapshot: ActivitySnapshot,
    ) -> ActivityPolicyEvaluation {
        self.evaluate_policies_for_snapshot(now_monotonic_ms, snapshot)
            .microphone
    }

    pub fn evaluate_system_audio_policy_for_snapshot(
        &mut self,
        now_monotonic_ms: u64,
        snapshot: ActivitySnapshot,
    ) -> ActivityPolicyEvaluation {
        self.evaluate_policies_for_snapshot(now_monotonic_ms, snapshot)
            .system_audio
    }
}

#[cfg(test)]
mod tests;
