//! Licensing Tauri commands over `app_core::licensing` (names must stay
//! byte-identical to the frontend `invoke(...)`).

use std::sync::Arc;

use app_core::{AppCore, AppInfraState};
use capture_types::{ActivateLicenseResult, LicenseDevices, LicenseStatus, ResetDevicesOutcome};
use tauri::State;

#[tauri::command]
pub async fn get_license_status(
    state: State<'_, AppInfraState>,
    core: State<'_, Arc<AppCore>>,
) -> Result<LicenseStatus, String> {
    app_core::licensing::get_license_status(&core, &state).await
}

#[tauri::command]
pub async fn start_trial(
    state: State<'_, AppInfraState>,
    core: State<'_, Arc<AppCore>>,
) -> Result<LicenseStatus, String> {
    app_core::licensing::start_trial(&core, &state).await
}

#[tauri::command]
pub async fn activate_license(
    key: String,
    state: State<'_, AppInfraState>,
    core: State<'_, Arc<AppCore>>,
) -> Result<ActivateLicenseResult, String> {
    app_core::licensing::activate_license(key, &core, &state).await
}

#[tauri::command]
pub async fn refresh_license_now(core: State<'_, Arc<AppCore>>) -> Result<(), String> {
    app_core::licensing::refresh_license_now(Arc::clone(&core)).await
}

#[tauri::command]
pub async fn reset_license_devices(
    core: State<'_, Arc<AppCore>>,
) -> Result<ResetDevicesOutcome, String> {
    app_core::licensing::reset::reset_license_devices(Arc::clone(&core)).await
}

#[tauri::command]
pub async fn get_license_devices() -> Result<Option<LicenseDevices>, String> {
    app_core::licensing::reset::get_license_devices().await
}
