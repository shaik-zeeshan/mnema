use std::sync::Arc;

use app_core::{general_app_log, AppCore};
use capture_types::{CaptureErrorResponse, GeneralAppLogStatus};
use tauri::State;

#[tauri::command]
pub fn get_general_app_log_status(
    core: State<'_, Arc<AppCore>>,
) -> Result<GeneralAppLogStatus, CaptureErrorResponse> {
    general_app_log::status(&core)
}

#[tauri::command]
pub fn open_general_app_log(
    core: State<'_, Arc<AppCore>>,
) -> Result<GeneralAppLogStatus, CaptureErrorResponse> {
    general_app_log::open(&core)
}

#[tauri::command]
pub fn delete_general_app_log(
    core: State<'_, Arc<AppCore>>,
) -> Result<GeneralAppLogStatus, CaptureErrorResponse> {
    general_app_log::delete(&core)
}
