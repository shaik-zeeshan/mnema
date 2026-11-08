#[cfg(target_os = "macos")]
use super::runtime::NativeCaptureRuntime;
#[cfg(target_os = "macos")]
use capture_types::CaptureErrorResponse;
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(target_os = "macos")]
use std::sync::Mutex;
#[cfg(target_os = "macos")]

/// Never-reset source of process-globally-unique collector tokens. Unlike the
/// per-runtime generation counter (which restarts at 0 on every capture-session
/// reset), this only ever increments, so a token minted for one collector can
/// never equal a token minted for another — even across a reset that re-uses the
/// same generation number.
#[cfg(target_os = "macos")]
static COLLECTION_TOKEN_SOURCE: AtomicU64 = AtomicU64::new(0);

/// Error code reported when a privacy-filter apply fails specifically because no
/// capture display is available (display sleep, screen lock, lid close, monitor
/// disconnect). The segment loop treats this as a transient liveness condition
/// to recover from, distinct from a genuine privacy-filter failure.
#[cfg(target_os = "macos")]
pub const PRIVACY_FILTER_DISPLAY_UNAVAILABLE_CODE: &str = "privacy_filter_display_unavailable";

#[cfg(target_os = "macos")]
#[derive(Debug, Clone)]
pub struct InitialPrivacyFilter {
    decision: capture_metadata::PrivacyFilterDecision,
    filter: Option<capture_screen::PrivacyContentFilter>,
    filter_system_audio: bool,
}

#[cfg(target_os = "macos")]
impl InitialPrivacyFilter {
    pub fn screen_capture_filter(&self) -> Option<capture_screen::PrivacyContentFilter> {
        self.filter.clone()
    }

    /// The same excluded apps the screen filter hides, for the system-audio tap's
    /// exclude list. Parity, not a feature: ScreenCaptureKit's content filter
    /// already silenced these apps' audio (ADR 0052). Gated on the
    /// `privacy.filterSystemAudio` Settings toggle — see
    /// [`system_audio_privacy_excluded_bundle_ids`].
    pub fn excluded_bundle_ids(&self) -> Vec<String> {
        system_audio_privacy_excluded_bundle_ids(
            self.filter_system_audio,
            &self.decision.excluded_bundle_ids,
        )
        .to_vec()
    }

    pub fn mark_applied(self, core: &crate::AppCore) {
        mark_privacy_decision_applied(core, self.decision);
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
pub struct PrivacyFilterUpdate {
    decision: capture_metadata::PrivacyFilterDecision,
    filter: Option<capture_screen::PrivacyContentFilter>,
    filter_system_audio: bool,
}

#[cfg(target_os = "macos")]
impl PrivacyFilterUpdate {
    /// The apps this update excludes, for the system-audio tap's exclude list.
    /// Gated on the `privacy.filterSystemAudio` Settings toggle — see
    /// [`system_audio_privacy_excluded_bundle_ids`].
    pub fn excluded_bundle_ids(&self) -> &[String] {
        system_audio_privacy_excluded_bundle_ids(
            self.filter_system_audio,
            &self.decision.excluded_bundle_ids,
        )
    }
}

/// The privacy-list half of the system-audio tap's effective exclude list — the
/// ONE gate every path (capture start, privacy-edit push, fallback poll) goes
/// through. With the `privacy.filterSystemAudio` toggle off, the privacy ids are
/// withheld so the tap excludes only Mnema's own process (that self-exclusion
/// lives inside `capture_system_audio::compute_exclude_list` and is never
/// toggleable). The SCREEN filter never routes through here; it stays gated on
/// the privacy list alone.
#[cfg(target_os = "macos")]
fn system_audio_privacy_excluded_bundle_ids(
    filter_system_audio: bool,
    excluded_bundle_ids: &[String],
) -> &[String] {
    if filter_system_audio {
        excluded_bundle_ids
    } else {
        &[]
    }
}

#[cfg(target_os = "macos")]
pub type PrivacyFilterRefreshState = Mutex<PrivacyFilterRefreshRuntime>;

#[cfg(not(target_os = "macos"))]
pub type PrivacyFilterRefreshState = std::sync::Mutex<()>;

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivacyRefreshReason {
    StaticAppRuleMutation,
    MetadataSettingsMutation,
    WorkspaceAppChanged,
    WorkspaceFocusChanged,
    FallbackPoll,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivacyRefreshMode {
    StaticExcludedAppsOnly,
    MetadataAndStaticApps,
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
pub struct CollectedPrivacyFilterUpdate {
    pub generation: u64,
    pub reason: PrivacyRefreshReason,
    pub mode: PrivacyRefreshMode,
    pub update: PrivacyFilterUpdate,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Default)]
pub struct PrivacyFilterRefreshRuntime {
    pub requested_generation: u64,
    pub latest_reason: Option<PrivacyRefreshReason>,
    collecting_generation: Option<u64>,
    /// Process-globally-unique id of the in-flight collector. The per-runtime
    /// `generation` counter restarts from 0 on every `reset_privacy_filter_refresh_state`
    /// (the runtime is replaced with `default()`), so a stale collector from a
    /// previous session can hold the SAME generation number a new session later
    /// re-claims — making `generation` alone unsafe to match a completion on.
    /// This token is minted from a never-reset global, so it can never collide
    /// across a reset.
    collecting_token: Option<u64>,
    last_completed_generation: u64,
    completed_update: Option<CollectedPrivacyFilterUpdate>,
    pub static_fallback_suppressed: bool,
}

#[cfg(target_os = "macos")]
impl PrivacyFilterRefreshRuntime {
    /// Claims the next collection generation, or `None` when a collector is
    /// already in flight or there is nothing newer to collect. The returned
    /// generation marks this runtime as collecting; the collector must hand it
    /// back to [`Self::complete_collection`].
    pub fn begin_collection(&mut self) -> Option<(u64, u64, PrivacyRefreshReason)> {
        if self.collecting_generation.is_some()
            || self.requested_generation <= self.last_completed_generation
        {
            return None;
        }
        let generation = self.requested_generation;
        let reason = self
            .latest_reason
            .unwrap_or(PrivacyRefreshReason::FallbackPoll);
        let token = COLLECTION_TOKEN_SOURCE.fetch_add(1, Ordering::Relaxed);
        self.collecting_generation = Some(generation);
        self.collecting_token = Some(token);
        Some((generation, token, reason))
    }

    /// Records the result of a finished collection — but only if `token` is still
    /// the in-flight one. A collector spawned for a *previous* session can land
    /// its write-back after [`reset_privacy_filter_refresh_state`] (or a later
    /// collection) has replaced the runtime; matching on the per-runtime
    /// `generation` is NOT enough, because the reset restarts that counter at 0
    /// and the new session can re-claim the SAME generation the stale collector
    /// still holds (the equal-generation collision). Applying that stale write
    /// would clobber the live session's snapshot with the previous session's data
    /// and poison `last_completed_generation`. The process-global `token` cannot
    /// collide across a reset, so it is the safe match key. Returns whether the
    /// result was applied.
    pub fn complete_collection(
        &mut self,
        token: u64,
        generation: u64,
        completed: CollectedPrivacyFilterUpdate,
    ) -> bool {
        if self.collecting_token != Some(token) {
            return false;
        }
        self.collecting_generation = None;
        self.collecting_token = None;
        self.last_completed_generation = generation;
        self.completed_update = Some(completed);
        true
    }
}

#[cfg(target_os = "macos")]
pub fn privacy_filter_from_decision(
    decision: capture_metadata::PrivacyFilterDecision,
) -> Option<capture_screen::PrivacyContentFilter> {
    decision
        .privacy_filter_applied
        .then_some(capture_screen::PrivacyContentFilter {
            display_id: 0,
            excluded_bundle_ids: decision.excluded_bundle_ids,
        })
}

#[cfg(target_os = "macos")]
pub fn privacy_refresh_debug_log_enabled(reason: PrivacyRefreshReason) -> bool {
    cfg!(feature = "privacy-refresh-trace") && reason != PrivacyRefreshReason::FallbackPoll
}

#[cfg(target_os = "macos")]
pub fn collect_initial_privacy_filter(core: &crate::AppCore) -> InitialPrivacyFilter {
    let settings = core
        .state::<crate::recording_settings::RecordingSettingsState>()
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .clone();
    let decision = collect_initial_privacy_filter_decision(
        &core.state::<super::metadata::CaptureMetadataState>(),
        &settings,
    );
    let filter = privacy_filter_from_decision(decision.clone());
    InitialPrivacyFilter {
        decision,
        filter,
        filter_system_audio: settings.privacy.filter_system_audio,
    }
}

#[cfg(target_os = "macos")]
fn collect_initial_privacy_filter_decision(
    metadata_state: &super::metadata::CaptureMetadataState,
    settings: &capture_types::RecordingSettings,
) -> capture_metadata::PrivacyFilterDecision {
    if settings.metadata.enabled {
        // `collect_initial_privacy_filter` is only ever called from synchronous
        // segment-start / resume / recovery paths that hold the
        // `NativeCaptureState` mutex, so the active-tab URL must come from the
        // cache (no live AX/AppleScript read) to avoid stalling under the lock.
        super::metadata::refresh_metadata_state(
            metadata_state,
            &settings.metadata,
            &settings.privacy,
            super::metadata::BrowserUrlReadMode::Cached,
        )
    } else {
        super::metadata::refresh_static_excluded_app_privacy_state(
            metadata_state,
            &settings.privacy,
        )
    }
}

#[cfg(target_os = "macos")]
fn collect_static_privacy_filter_update(core: &crate::AppCore) -> PrivacyFilterUpdate {
    let settings = core
        .state::<crate::recording_settings::RecordingSettingsState>()
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .clone();
    let decision = super::metadata::refresh_static_excluded_app_privacy_state(
        &core.state::<super::metadata::CaptureMetadataState>(),
        &settings.privacy,
    );
    let latest_applied = super::metadata::latest_applied_privacy_decision(
        &core.state::<super::metadata::CaptureMetadataState>(),
    );
    let filter = privacy_filter_from_decision(decision.clone()).or_else(|| {
        latest_applied
            .privacy_filter_applied
            .then_some(empty_privacy_filter())
    });
    PrivacyFilterUpdate {
        decision,
        filter,
        filter_system_audio: settings.privacy.filter_system_audio,
    }
}

#[cfg(target_os = "macos")]
fn collect_metadata_privacy_filter_update(
    core: &crate::AppCore,
    browser_url_read_mode: super::metadata::BrowserUrlReadMode,
) -> PrivacyFilterUpdate {
    let settings = core
        .state::<crate::recording_settings::RecordingSettingsState>()
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .clone();
    let decision = super::metadata::refresh_metadata_state(
        &core.state::<super::metadata::CaptureMetadataState>(),
        &settings.metadata,
        &settings.privacy,
        browser_url_read_mode,
    );
    let latest_applied = super::metadata::latest_applied_privacy_decision(
        &core.state::<super::metadata::CaptureMetadataState>(),
    );
    let filter = privacy_filter_from_decision(decision.clone()).or_else(|| {
        latest_applied
            .privacy_filter_applied
            .then_some(empty_privacy_filter())
    });
    PrivacyFilterUpdate {
        decision,
        filter,
        filter_system_audio: settings.privacy.filter_system_audio,
    }
}

#[cfg(target_os = "macos")]
fn privacy_refresh_mode(
    settings: &capture_types::RecordingSettings,
    reason: PrivacyRefreshReason,
) -> PrivacyRefreshMode {
    if reason == PrivacyRefreshReason::MetadataSettingsMutation {
        PrivacyRefreshMode::MetadataAndStaticApps
    } else if settings.metadata.enabled && reason != PrivacyRefreshReason::StaticAppRuleMutation {
        PrivacyRefreshMode::MetadataAndStaticApps
    } else {
        PrivacyRefreshMode::StaticExcludedAppsOnly
    }
}

#[cfg(target_os = "macos")]
pub fn collect_privacy_filter_update(
    core: &crate::AppCore,
    reason: PrivacyRefreshReason,
    browser_url_read_mode: super::metadata::BrowserUrlReadMode,
) -> (PrivacyRefreshMode, PrivacyFilterUpdate) {
    let settings = core
        .state::<crate::recording_settings::RecordingSettingsState>()
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .clone();
    let mode = privacy_refresh_mode(&settings, reason);
    let update = match mode {
        PrivacyRefreshMode::StaticExcludedAppsOnly => collect_static_privacy_filter_update(core),
        PrivacyRefreshMode::MetadataAndStaticApps => {
            collect_metadata_privacy_filter_update(core, browser_url_read_mode)
        }
    };
    (mode, update)
}

#[cfg(target_os = "macos")]
pub fn reset_privacy_filter_refresh_state(core: &crate::AppCore) {
    if let Some(state) = core.try_state::<PrivacyFilterRefreshState>() {
        *state.lock().expect("privacy filter refresh state poisoned") =
            PrivacyFilterRefreshRuntime::default();
    }
}

#[cfg(target_os = "macos")]
pub fn take_completed_privacy_filter_update(
    core: &crate::AppCore,
) -> Option<CollectedPrivacyFilterUpdate> {
    let refresh_state = core.try_state::<PrivacyFilterRefreshState>()?;
    let mut state = refresh_state
        .lock()
        .expect("privacy filter refresh state poisoned");
    let completed = state.completed_update.take()?;
    if completed.generation == state.requested_generation {
        Some(completed)
    } else {
        if privacy_refresh_debug_log_enabled(completed.reason) {
            crate::debug_log::log(format!(
                "stale privacy refresh skipped (reason={:?}, completed_generation={}, requested_generation={})",
                completed.reason, completed.generation, state.requested_generation
            ));
        }
        None
    }
}

#[cfg(target_os = "macos")]
pub fn record_privacy_filter_apply_outcome(
    core: &crate::AppCore,
    mode: PrivacyRefreshMode,
    outcome: capture_screen::PrivacyFilterApplyOutcome,
) {
    if mode != PrivacyRefreshMode::StaticExcludedAppsOnly {
        return;
    }
    if let Some(refresh_state) = core.try_state::<PrivacyFilterRefreshState>() {
        let mut state = refresh_state
            .lock()
            .expect("privacy filter refresh state poisoned");
        if outcome.request_satisfied {
            state.static_fallback_suppressed = true;
        }
    }
}

#[cfg(target_os = "macos")]
pub fn record_initial_privacy_filter_outcome(
    core: &crate::AppCore,
    _settings: &capture_types::RecordingSettings,
    outcome: Option<capture_screen::PrivacyFilterApplyOutcome>,
) {
    let Some(outcome) = outcome else {
        return;
    };
    record_privacy_filter_apply_outcome(core, PrivacyRefreshMode::StaticExcludedAppsOnly, outcome);
}

#[cfg(target_os = "macos")]
pub fn apply_privacy_filter_update(
    core: &crate::AppCore,
    runtime: &mut NativeCaptureRuntime,
    update: PrivacyFilterUpdate,
) -> Result<capture_screen::PrivacyFilterApplyOutcome, CaptureErrorResponse> {
    if !capture_screen::screen_capture_session_is_live(runtime.active_screen_session.as_ref()) {
        return Ok(capture_screen::PrivacyFilterApplyOutcome {
            request_satisfied: false,
        });
    }

    let Some(filter) = update.filter else {
        mark_privacy_decision_applied(core, update.decision);
        return Ok(capture_screen::PrivacyFilterApplyOutcome {
            request_satisfied: true,
        });
    };

    let outcome =
        capture_screen::update_active_privacy_filter(&mut runtime.active_screen_session, filter)
            .map_err(|error| CaptureErrorResponse {
                code: if error.kind
                    == capture_screen::PrivacyFilterApplyErrorKind::DisplayUnavailable
                {
                    PRIVACY_FILTER_DISPLAY_UNAVAILABLE_CODE.to_string()
                } else {
                    "privacy_filter_apply_failed".to_string()
                },
                message: error.message,
            })?;
    mark_privacy_decision_applied(core, update.decision);
    Ok(outcome)
}

#[cfg(target_os = "macos")]
fn mark_privacy_decision_applied(
    core: &crate::AppCore,
    decision: capture_metadata::PrivacyFilterDecision,
) {
    super::metadata::mark_applied_privacy_decision(
        &core.state::<super::metadata::CaptureMetadataState>(),
        decision,
    );
}

#[cfg(target_os = "macos")]
fn empty_privacy_filter() -> capture_screen::PrivacyContentFilter {
    capture_screen::PrivacyContentFilter {
        display_id: 0,
        excluded_bundle_ids: Vec::new(),
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests;
