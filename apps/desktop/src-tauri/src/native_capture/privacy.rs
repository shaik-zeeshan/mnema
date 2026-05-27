#[cfg(target_os = "macos")]
use super::runtime::NativeCaptureRuntime;
#[cfg(target_os = "macos")]
use capture_types::CaptureErrorResponse;
#[cfg(target_os = "macos")]
use std::sync::Mutex;
#[cfg(target_os = "macos")]
use tauri::Manager;

/// Error code reported when a privacy-filter apply fails specifically because no
/// capture display is available (display sleep, screen lock, lid close, monitor
/// disconnect). The segment loop treats this as a transient liveness condition
/// to recover from, distinct from a genuine privacy-filter failure.
#[cfg(target_os = "macos")]
pub(super) const PRIVACY_FILTER_DISPLAY_UNAVAILABLE_CODE: &str =
    "privacy_filter_display_unavailable";

#[cfg(target_os = "macos")]
#[derive(Debug, Clone)]
pub(super) struct InitialPrivacyFilter {
    decision: capture_metadata::PrivacyFilterDecision,
    filter: Option<capture_screen::PrivacyContentFilter>,
}

#[cfg(target_os = "macos")]
impl InitialPrivacyFilter {
    pub(super) fn screen_capture_filter(&self) -> Option<capture_screen::PrivacyContentFilter> {
        self.filter.clone()
    }

    pub(super) fn mark_applied(self, app_handle: &tauri::AppHandle) {
        mark_privacy_decision_applied(app_handle, self.decision);
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
pub(super) struct PrivacyFilterUpdate {
    decision: capture_metadata::PrivacyFilterDecision,
    filter: Option<capture_screen::PrivacyContentFilter>,
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
pub(super) enum PrivacyRefreshMode {
    StaticExcludedAppsOnly,
    MetadataAndStaticApps,
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
pub(super) struct CollectedPrivacyFilterUpdate {
    pub generation: u64,
    pub reason: PrivacyRefreshReason,
    pub mode: PrivacyRefreshMode,
    pub update: PrivacyFilterUpdate,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Default)]
pub struct PrivacyFilterRefreshRuntime {
    requested_generation: u64,
    latest_reason: Option<PrivacyRefreshReason>,
    collecting_generation: Option<u64>,
    last_completed_generation: u64,
    completed_update: Option<CollectedPrivacyFilterUpdate>,
    static_fallback_suppressed: bool,
}

#[cfg(target_os = "macos")]
pub(super) fn privacy_filter_from_decision(
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
pub(super) fn privacy_refresh_debug_log_enabled(reason: PrivacyRefreshReason) -> bool {
    cfg!(feature = "privacy-refresh-trace") && reason != PrivacyRefreshReason::FallbackPoll
}

#[cfg(target_os = "macos")]
pub(super) fn collect_initial_privacy_filter(
    app_handle: &tauri::AppHandle,
) -> InitialPrivacyFilter {
    let settings = app_handle
        .state::<crate::native_capture::RecordingSettingsState>()
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .clone();
    let decision = collect_initial_privacy_filter_decision(
        app_handle
            .state::<crate::native_capture::CaptureMetadataState>()
            .inner(),
        &settings,
    );
    let filter = privacy_filter_from_decision(decision.clone());
    InitialPrivacyFilter { decision, filter }
}

#[cfg(target_os = "macos")]
fn collect_initial_privacy_filter_decision(
    metadata_state: &crate::native_capture::CaptureMetadataState,
    settings: &capture_types::RecordingSettings,
) -> capture_metadata::PrivacyFilterDecision {
    if settings.metadata.enabled {
        crate::native_capture::metadata::refresh_metadata_state(
            metadata_state,
            &settings.metadata,
            &settings.privacy,
        )
    } else {
        crate::native_capture::metadata::refresh_static_excluded_app_privacy_state(
            metadata_state,
            &settings.privacy,
        )
    }
}

#[cfg(target_os = "macos")]
fn collect_static_privacy_filter_update(app_handle: &tauri::AppHandle) -> PrivacyFilterUpdate {
    let settings = app_handle
        .state::<crate::native_capture::RecordingSettingsState>()
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .clone();
    let decision = crate::native_capture::metadata::refresh_static_excluded_app_privacy_state(
        app_handle
            .state::<crate::native_capture::CaptureMetadataState>()
            .inner(),
        &settings.privacy,
    );
    let latest_applied = crate::native_capture::metadata::latest_applied_privacy_decision(
        app_handle
            .state::<crate::native_capture::CaptureMetadataState>()
            .inner(),
    );
    let filter = privacy_filter_from_decision(decision.clone()).or_else(|| {
        latest_applied
            .privacy_filter_applied
            .then_some(empty_privacy_filter())
    });
    PrivacyFilterUpdate { decision, filter }
}

#[cfg(target_os = "macos")]
fn collect_metadata_privacy_filter_update(app_handle: &tauri::AppHandle) -> PrivacyFilterUpdate {
    let settings = app_handle
        .state::<crate::native_capture::RecordingSettingsState>()
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .clone();
    let decision = crate::native_capture::metadata::refresh_metadata_state(
        app_handle
            .state::<crate::native_capture::CaptureMetadataState>()
            .inner(),
        &settings.metadata,
        &settings.privacy,
    );
    let latest_applied = crate::native_capture::metadata::latest_applied_privacy_decision(
        app_handle
            .state::<crate::native_capture::CaptureMetadataState>()
            .inner(),
    );
    let filter = privacy_filter_from_decision(decision.clone()).or_else(|| {
        latest_applied
            .privacy_filter_applied
            .then_some(empty_privacy_filter())
    });
    PrivacyFilterUpdate { decision, filter }
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
pub(super) fn collect_privacy_filter_update(
    app_handle: &tauri::AppHandle,
    reason: PrivacyRefreshReason,
) -> (PrivacyRefreshMode, PrivacyFilterUpdate) {
    let settings = app_handle
        .state::<crate::native_capture::RecordingSettingsState>()
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .clone();
    let mode = privacy_refresh_mode(&settings, reason);
    let update = match mode {
        PrivacyRefreshMode::StaticExcludedAppsOnly => {
            collect_static_privacy_filter_update(app_handle)
        }
        PrivacyRefreshMode::MetadataAndStaticApps => {
            collect_metadata_privacy_filter_update(app_handle)
        }
    };
    (mode, update)
}

#[cfg(target_os = "macos")]
pub(crate) fn reset_privacy_filter_refresh_state(app_handle: &tauri::AppHandle) {
    if let Some(state) = app_handle.try_state::<crate::native_capture::PrivacyFilterRefreshState>()
    {
        *state.lock().expect("privacy filter refresh state poisoned") =
            PrivacyFilterRefreshRuntime::default();
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn request_privacy_filter_refresh(
    app_handle: &tauri::AppHandle,
    reason: PrivacyRefreshReason,
) {
    let capture_state = app_handle.state::<crate::native_capture::NativeCaptureState>();
    let control = {
        let Ok(runtime) = capture_state.lock() else {
            return;
        };
        let runtime = runtime.runtime();
        if !runtime.is_running || runtime.segment_loop_control.is_none() {
            if privacy_refresh_debug_log_enabled(reason) {
                super::debug_log::log(format!(
                    "privacy refresh skipped because recording is stopped (reason={reason:?})"
                ));
            }
            return;
        }
        runtime.segment_loop_control.clone()
    };

    let Some(refresh_state) =
        app_handle.try_state::<crate::native_capture::PrivacyFilterRefreshState>()
    else {
        return;
    };
    let mut state = refresh_state
        .lock()
        .expect("privacy filter refresh state poisoned");
    if reason != PrivacyRefreshReason::FallbackPoll {
        state.static_fallback_suppressed = false;
    }
    let metadata_enabled = app_handle
        .state::<crate::native_capture::RecordingSettingsState>()
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .metadata
        .enabled;
    if reason == PrivacyRefreshReason::FallbackPoll
        && state.static_fallback_suppressed
        && !metadata_enabled
    {
        return;
    }
    state.requested_generation = state.requested_generation.saturating_add(1);
    state.latest_reason = Some(reason);
    if privacy_refresh_debug_log_enabled(reason) {
        super::debug_log::log(format!(
            "privacy refresh requested (reason={reason:?}, generation={})",
            state.requested_generation
        ));
    }
    drop(state);
    if let Some(control) = control {
        control.notify();
    }
}

#[cfg(target_os = "macos")]
pub(super) fn maybe_start_privacy_filter_collection(app_handle: &tauri::AppHandle) {
    let Some(refresh_state) =
        app_handle.try_state::<crate::native_capture::PrivacyFilterRefreshState>()
    else {
        return;
    };
    let (generation, reason) = {
        let mut state = refresh_state
            .lock()
            .expect("privacy filter refresh state poisoned");
        if state.collecting_generation.is_some()
            || state.requested_generation <= state.last_completed_generation
        {
            return;
        }
        let generation = state.requested_generation;
        let reason = state
            .latest_reason
            .unwrap_or(PrivacyRefreshReason::FallbackPoll);
        state.collecting_generation = Some(generation);
        (generation, reason)
    };
    if privacy_refresh_debug_log_enabled(reason) {
        super::debug_log::log(format!(
            "privacy refresh collector started (reason={reason:?}, generation={generation})"
        ));
    }
    let app_handle = app_handle.clone();
    std::thread::spawn(move || {
        let (mode, update) = collect_privacy_filter_update(&app_handle, reason);
        if let Some(refresh_state) =
            app_handle.try_state::<crate::native_capture::PrivacyFilterRefreshState>()
        {
            {
                let mut state = refresh_state
                    .lock()
                    .expect("privacy filter refresh state poisoned");
                state.collecting_generation = None;
                state.last_completed_generation = generation;
                state.completed_update = Some(CollectedPrivacyFilterUpdate {
                    generation,
                    reason,
                    mode,
                    update,
                });
            }
        }
        if let Some(control) = app_handle
            .state::<crate::native_capture::NativeCaptureState>()
            .lock()
            .ok()
            .and_then(|runtime| runtime.runtime().segment_loop_control.clone())
        {
            control.notify();
        }
    });
}

#[cfg(target_os = "macos")]
pub(super) fn take_completed_privacy_filter_update(
    app_handle: &tauri::AppHandle,
) -> Option<CollectedPrivacyFilterUpdate> {
    let refresh_state =
        app_handle.try_state::<crate::native_capture::PrivacyFilterRefreshState>()?;
    let mut state = refresh_state
        .lock()
        .expect("privacy filter refresh state poisoned");
    let completed = state.completed_update.take()?;
    if completed.generation == state.requested_generation {
        Some(completed)
    } else {
        if privacy_refresh_debug_log_enabled(completed.reason) {
            super::debug_log::log(format!(
                "stale privacy refresh skipped (reason={:?}, completed_generation={}, requested_generation={})",
                completed.reason, completed.generation, state.requested_generation
            ));
        }
        None
    }
}

#[cfg(target_os = "macos")]
pub(super) fn record_privacy_filter_apply_outcome(
    app_handle: &tauri::AppHandle,
    mode: PrivacyRefreshMode,
    outcome: capture_screen::PrivacyFilterApplyOutcome,
) {
    if mode != PrivacyRefreshMode::StaticExcludedAppsOnly {
        return;
    }
    if let Some(refresh_state) =
        app_handle.try_state::<crate::native_capture::PrivacyFilterRefreshState>()
    {
        let mut state = refresh_state
            .lock()
            .expect("privacy filter refresh state poisoned");
        if outcome.request_satisfied {
            state.static_fallback_suppressed = true;
        }
    }
}

#[cfg(target_os = "macos")]
pub(super) fn record_initial_privacy_filter_outcome(
    app_handle: &tauri::AppHandle,
    _settings: &capture_types::RecordingSettings,
    outcome: Option<capture_screen::PrivacyFilterApplyOutcome>,
) {
    let Some(outcome) = outcome else {
        return;
    };
    record_privacy_filter_apply_outcome(
        app_handle,
        PrivacyRefreshMode::StaticExcludedAppsOnly,
        outcome,
    );
}

#[cfg(target_os = "macos")]
pub(super) fn apply_privacy_filter_update(
    app_handle: &tauri::AppHandle,
    runtime: &mut NativeCaptureRuntime,
    update: PrivacyFilterUpdate,
) -> Result<capture_screen::PrivacyFilterApplyOutcome, CaptureErrorResponse> {
    if !capture_screen::screen_capture_session_is_live(runtime.active_screen_session.as_ref()) {
        return Ok(capture_screen::PrivacyFilterApplyOutcome {
            request_satisfied: false,
        });
    }

    let Some(filter) = update.filter else {
        mark_privacy_decision_applied(app_handle, update.decision);
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
    mark_privacy_decision_applied(app_handle, update.decision);
    Ok(outcome)
}

#[cfg(target_os = "macos")]
fn mark_privacy_decision_applied(
    app_handle: &tauri::AppHandle,
    decision: capture_metadata::PrivacyFilterDecision,
) {
    crate::native_capture::metadata::mark_applied_privacy_decision(
        app_handle
            .state::<crate::native_capture::CaptureMetadataState>()
            .inner(),
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
mod tests {
    use super::*;
    use crate::native_capture::settings::default_recording_settings;

    #[test]
    fn privacy_refresh_uses_metadata_collection_when_metadata_is_enabled() {
        let mut settings = default_recording_settings();
        settings.metadata.enabled = true;

        assert_eq!(
            privacy_refresh_mode(&settings, PrivacyRefreshReason::FallbackPoll),
            PrivacyRefreshMode::MetadataAndStaticApps
        );
        assert_eq!(
            privacy_refresh_mode(&settings, PrivacyRefreshReason::WorkspaceFocusChanged),
            PrivacyRefreshMode::MetadataAndStaticApps
        );
        assert!(settings.privacy.excluded_apps.is_empty());
    }

    #[test]
    fn privacy_refresh_keeps_static_fast_path_for_static_rule_mutations() {
        let mut settings = default_recording_settings();
        settings.metadata.enabled = true;

        assert_eq!(
            privacy_refresh_mode(&settings, PrivacyRefreshReason::StaticAppRuleMutation),
            PrivacyRefreshMode::StaticExcludedAppsOnly
        );
    }

    #[test]
    fn privacy_refresh_uses_metadata_collection_for_metadata_settings_mutations() {
        let mut settings = default_recording_settings();
        settings.metadata.enabled = false;

        assert_eq!(
            privacy_refresh_mode(&settings, PrivacyRefreshReason::MetadataSettingsMutation),
            PrivacyRefreshMode::MetadataAndStaticApps
        );
    }

    #[test]
    fn privacy_refresh_keeps_static_fast_path_when_metadata_is_disabled() {
        let mut settings = default_recording_settings();
        settings.metadata.enabled = false;

        assert_eq!(
            privacy_refresh_mode(&settings, PrivacyRefreshReason::FallbackPoll),
            PrivacyRefreshMode::StaticExcludedAppsOnly
        );
        assert_eq!(
            privacy_refresh_mode(&settings, PrivacyRefreshReason::WorkspaceFocusChanged),
            PrivacyRefreshMode::StaticExcludedAppsOnly
        );
    }

    #[test]
    fn initial_privacy_filter_collects_metadata_snapshot_when_metadata_is_enabled() {
        let mut settings = default_recording_settings();
        settings.metadata.enabled = true;
        let metadata_state = crate::native_capture::CaptureMetadataState::default();

        let _decision = collect_initial_privacy_filter_decision(&metadata_state, &settings);

        let runtime = metadata_state
            .lock()
            .expect("capture metadata state should lock");
        assert!(runtime.latest_snapshot().is_some());
    }
}
