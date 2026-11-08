//! Tauri commands for the app-exclusion rules; the edits and their apply step
//! live in `app_core::privacy_redaction_sources`.

use std::sync::Arc;

use app_core::privacy_redaction_sources::apply;
use app_core::AppCore;
use capture_types::{CaptureErrorResponse, RecordingSettingsDomainUpdateResponse};
use tauri::State;

#[tauri::command]
pub fn add_privacy_excluded_app(
    bundle_id: String,
    display_name: String,
    core: State<'_, Arc<AppCore>>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    apply::add_privacy_excluded_app(bundle_id, display_name, &core)
}

#[tauri::command]
pub fn set_privacy_excluded_app_enabled(
    source_id: String,
    enabled: bool,
    core: State<'_, Arc<AppCore>>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    apply::set_privacy_excluded_app_enabled(source_id, enabled, &core)
}

#[tauri::command]
pub fn set_privacy_filter_system_audio(
    enabled: bool,
    core: State<'_, Arc<AppCore>>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    apply::set_privacy_filter_system_audio(enabled, &core)
}

#[tauri::command]
pub fn remove_privacy_excluded_app(
    source_id: String,
    core: State<'_, Arc<AppCore>>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    apply::remove_privacy_excluded_app(source_id, &core)
}
