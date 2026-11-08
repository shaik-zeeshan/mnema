//! Tauri commands for Retention Cleanup and Delete Recent Capture; the
//! behaviour lives in `app_core::app_infra`.

use std::sync::Arc;

use app_core::app_infra::{
    self as core_infra, DeleteRecentCaptureRequestDto, DeleteRecentCaptureSummaryDto,
    PreviewRetentionCleanupRequest,
};
use app_core::recording_settings::RecordingSettingsState;
use app_core::{AppCore, AppInfraState};
use tauri::State;

#[tauri::command]
pub async fn preview_retention_cleanup(
    request: Option<PreviewRetentionCleanupRequest>,
    core: State<'_, Arc<AppCore>>,
    infra: State<'_, AppInfraState>,
    settings: State<'_, RecordingSettingsState>,
) -> Result<::app_infra::RetentionCleanupSummary, String> {
    core_infra::preview_retention_cleanup(request, &core, &infra, &settings).await
}

#[tauri::command]
pub async fn run_retention_cleanup_now(
    core: State<'_, Arc<AppCore>>,
    infra: State<'_, AppInfraState>,
    settings: State<'_, RecordingSettingsState>,
) -> Result<::app_infra::RetentionCleanupSummary, String> {
    core_infra::run_retention_cleanup_now(&core, &infra, &settings).await
}

#[tauri::command]
pub async fn delete_recent_capture(
    request: DeleteRecentCaptureRequestDto,
    core: State<'_, Arc<AppCore>>,
    infra: State<'_, AppInfraState>,
) -> Result<DeleteRecentCaptureSummaryDto, String> {
    core_infra::delete_recent_capture(request, &core, &infra).await
}
