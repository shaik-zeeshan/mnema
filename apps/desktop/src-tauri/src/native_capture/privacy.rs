#[cfg(target_os = "macos")]
use super::runtime::NativeCaptureRuntime;
#[cfg(target_os = "macos")]
use capture_types::CaptureErrorResponse;
#[cfg(target_os = "macos")]
use std::sync::Mutex;
#[cfg(target_os = "macos")]
use tauri::Manager;

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
    DynamicPrivacySettingsMutation,
    MetadataSettingsMutation,
    WorkspaceAppChanged,
    WorkspaceFocusChanged,
    FallbackPoll,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PrivacyRefreshMode {
    Full,
    StaticExcludedAppsOnly,
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
            excluded_window_ids: decision.excluded_window_ids,
        })
}

#[cfg(target_os = "macos")]
pub(crate) fn dynamic_privacy_features_enabled(
    privacy: &capture_metadata::PrivacySettings,
) -> bool {
    privacy
        .excluded_website_rules
        .iter()
        .any(|rule| rule.enabled)
        || privacy.browser_title_rules.iter().any(|rule| rule.enabled)
        || privacy.private_browser_exclusion_enabled
}

#[cfg(target_os = "macos")]
fn refresh_mode_for_reason(
    reason: PrivacyRefreshReason,
    settings: &capture_types::RecordingSettings,
) -> PrivacyRefreshMode {
    let static_reason = matches!(
        reason,
        PrivacyRefreshReason::StaticAppRuleMutation
            | PrivacyRefreshReason::WorkspaceAppChanged
            | PrivacyRefreshReason::WorkspaceFocusChanged
            | PrivacyRefreshReason::FallbackPoll
    );
    if static_reason && !dynamic_privacy_features_enabled(&settings.privacy) {
        PrivacyRefreshMode::StaticExcludedAppsOnly
    } else {
        PrivacyRefreshMode::Full
    }
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
    let decision = crate::native_capture::metadata::refresh_metadata_state(
        app_handle
            .state::<crate::native_capture::CaptureMetadataState>()
            .inner(),
        &settings.metadata,
        &settings.privacy,
    );
    let filter = privacy_filter_from_decision(decision.clone());
    InitialPrivacyFilter { decision, filter }
}

#[cfg(target_os = "macos")]
fn collect_full_privacy_filter_update(app_handle: &tauri::AppHandle) -> PrivacyFilterUpdate {
    let current = collect_initial_privacy_filter(app_handle);
    let latest_applied = crate::native_capture::metadata::latest_applied_privacy_decision(
        app_handle
            .state::<crate::native_capture::CaptureMetadataState>()
            .inner(),
    );
    let filter = current.filter.or_else(|| {
        latest_applied
            .privacy_filter_applied
            .then_some(empty_privacy_filter())
    });
    PrivacyFilterUpdate {
        decision: current.decision,
        filter,
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
pub(super) fn collect_privacy_filter_update(app_handle: &tauri::AppHandle) -> PrivacyFilterUpdate {
    collect_full_privacy_filter_update(app_handle)
}

#[cfg(target_os = "macos")]
fn collect_privacy_filter_update_for_mode(
    app_handle: &tauri::AppHandle,
    mode: PrivacyRefreshMode,
) -> PrivacyFilterUpdate {
    match mode {
        PrivacyRefreshMode::Full => collect_full_privacy_filter_update(app_handle),
        PrivacyRefreshMode::StaticExcludedAppsOnly => {
            collect_static_privacy_filter_update(app_handle)
        }
    }
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
    if reason == PrivacyRefreshReason::FallbackPoll && state.static_fallback_suppressed {
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
    let (generation, reason, settings) = {
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
        let settings = app_handle
            .state::<crate::native_capture::RecordingSettingsState>()
            .lock()
            .expect("recording settings state poisoned")
            .settings
            .clone();
        (generation, reason, settings)
    };
    let mode = refresh_mode_for_reason(reason, &settings);
    if privacy_refresh_debug_log_enabled(reason) {
        super::debug_log::log(format!(
            "privacy refresh collector started (reason={reason:?}, generation={generation}, mode={mode:?})"
        ));
    }
    let app_handle = app_handle.clone();
    std::thread::spawn(move || {
        let update = collect_privacy_filter_update_for_mode(&app_handle, mode);
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
    settings: &capture_types::RecordingSettings,
    outcome: Option<capture_screen::PrivacyFilterApplyOutcome>,
) {
    let Some(outcome) = outcome else {
        return;
    };
    if refresh_mode_for_reason(PrivacyRefreshReason::FallbackPoll, settings)
        == PrivacyRefreshMode::StaticExcludedAppsOnly
    {
        record_privacy_filter_apply_outcome(
            app_handle,
            PrivacyRefreshMode::StaticExcludedAppsOnly,
            outcome,
        );
    }
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
                code: "privacy_filter_apply_failed".to_string(),
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
        excluded_window_ids: Vec::new(),
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use crate::native_capture::settings::default_recording_settings;
    use capture_metadata::{BrowserTitleRule, BrowserTitleRuleMatchType, WebsiteRule};

    #[test]
    fn enabled_website_rule_disables_static_fast_path() {
        let mut settings = default_recording_settings();
        settings.privacy.private_browser_exclusion_enabled = false;
        settings.privacy.excluded_website_rules = vec![WebsiteRule {
            id: "site".to_string(),
            enabled: true,
            pattern: "example.com".to_string(),
            host: Some("example.com".to_string()),
            include_subdomains: true,
            path_prefix: None,
            port: None,
        }];

        assert_eq!(
            refresh_mode_for_reason(PrivacyRefreshReason::FallbackPoll, &settings),
            PrivacyRefreshMode::Full
        );
    }

    #[test]
    fn enabled_title_rule_disables_static_fast_path() {
        let mut settings = default_recording_settings();
        settings.privacy.private_browser_exclusion_enabled = false;
        settings.privacy.browser_title_rules = vec![BrowserTitleRule {
            id: "title".to_string(),
            enabled: true,
            match_type: BrowserTitleRuleMatchType::Substring,
            pattern: "secret".to_string(),
        }];

        assert_eq!(
            refresh_mode_for_reason(PrivacyRefreshReason::FallbackPoll, &settings),
            PrivacyRefreshMode::Full
        );
    }

    #[test]
    fn private_browser_exclusion_disables_static_fast_path() {
        let mut settings = default_recording_settings();
        settings.privacy.private_browser_exclusion_enabled = true;

        assert_eq!(
            refresh_mode_for_reason(PrivacyRefreshReason::FallbackPoll, &settings),
            PrivacyRefreshMode::Full
        );
    }

    #[test]
    fn metadata_enabled_alone_does_not_disable_static_fast_path() {
        let mut settings = default_recording_settings();
        settings.metadata.enabled = true;
        settings.privacy.private_browser_exclusion_enabled = false;

        assert_eq!(
            refresh_mode_for_reason(PrivacyRefreshReason::FallbackPoll, &settings),
            PrivacyRefreshMode::StaticExcludedAppsOnly
        );
    }

    #[test]
    fn metadata_settings_mutation_forces_full_refresh() {
        let mut settings = default_recording_settings();
        settings.privacy.private_browser_exclusion_enabled = false;

        assert_eq!(
            refresh_mode_for_reason(PrivacyRefreshReason::MetadataSettingsMutation, &settings),
            PrivacyRefreshMode::Full
        );
    }
}
