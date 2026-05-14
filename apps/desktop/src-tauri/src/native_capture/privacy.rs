#[cfg(target_os = "macos")]
use super::runtime::NativeCaptureRuntime;
#[cfg(target_os = "macos")]
use capture_types::CaptureErrorResponse;
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
pub(super) struct PrivacyFilterUpdate {
    decision: capture_metadata::PrivacyFilterDecision,
    filter: Option<capture_screen::PrivacyContentFilter>,
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
pub(super) fn collect_privacy_filter_update(app_handle: &tauri::AppHandle) -> PrivacyFilterUpdate {
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
pub(super) fn apply_privacy_filter_update(
    app_handle: &tauri::AppHandle,
    runtime: &mut NativeCaptureRuntime,
    update: PrivacyFilterUpdate,
) -> Result<(), CaptureErrorResponse> {
    if !capture_screen::screen_capture_session_is_live(runtime.active_screen_session.as_ref()) {
        return Ok(());
    }

    let Some(filter) = update.filter else {
        mark_privacy_decision_applied(app_handle, update.decision);
        return Ok(());
    };

    capture_screen::update_active_privacy_filter(&mut runtime.active_screen_session, filter)
        .map_err(|error| CaptureErrorResponse {
            code: "privacy_filter_apply_failed".to_string(),
            message: error.message,
        })?;
    mark_privacy_decision_applied(app_handle, update.decision);
    Ok(())
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
