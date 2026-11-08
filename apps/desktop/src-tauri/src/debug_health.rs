//! Tauri command for the developer window's health rows; the gather and the
//! rules live in `app_core::debug_health`.

use std::sync::Arc;

use app_core::debug_health::{self as health, FeatureHealthDto};
use app_core::debug_status::SemanticWorkerHealthState;
use app_core::recording_settings::RecordingSettingsState;
use app_core::{AppCore, AppInfraState};
use tauri::State;

#[tauri::command]
pub async fn get_debug_health(
    core: State<'_, Arc<AppCore>>,
    infra: State<'_, AppInfraState>,
    worker_health: State<'_, SemanticWorkerHealthState>,
    settings_state: State<'_, RecordingSettingsState>,
) -> Result<Vec<FeatureHealthDto>, String> {
    health::get_debug_health(&core, &infra, &worker_health, &settings_state).await
}
