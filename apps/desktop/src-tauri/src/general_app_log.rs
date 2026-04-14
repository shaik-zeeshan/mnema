use capture_types::{CaptureErrorResponse, GeneralAppLogStatus};
use std::path::{Path, PathBuf};
use tauri::Manager;
use tauri_plugin_opener::OpenerExt;

fn general_app_log_path(app_handle: &tauri::AppHandle) -> Result<PathBuf, CaptureErrorResponse> {
    app_handle
        .path()
        .app_log_dir()
        .map(|path| path.join(crate::APP_LOG_FILE_NAME).with_extension("log"))
        .map_err(|error| CaptureErrorResponse {
            code: "app_log_path_unavailable".to_string(),
            message: format!("Failed to resolve general application log path: {error}"),
        })
}

fn status_for_path(path: &Path) -> GeneralAppLogStatus {
    GeneralAppLogStatus {
        path: path.to_string_lossy().to_string(),
        exists: path.is_file(),
    }
}

fn open_target_path(path: &Path, exists: bool) -> Result<PathBuf, CaptureErrorResponse> {
    if exists {
        return Ok(path.to_path_buf());
    }

    path.parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| CaptureErrorResponse {
            code: "app_log_path_unavailable".to_string(),
            message: format!(
                "Failed to resolve containing directory for general application log path '{}'",
                path.display()
            ),
        })
}

pub(crate) fn status(
    app_handle: &tauri::AppHandle,
) -> Result<GeneralAppLogStatus, CaptureErrorResponse> {
    let path = general_app_log_path(app_handle)?;
    Ok(status_for_path(&path))
}

pub(crate) fn open(
    app_handle: &tauri::AppHandle,
) -> Result<GeneralAppLogStatus, CaptureErrorResponse> {
    let path = general_app_log_path(app_handle)?;
    let status = status_for_path(&path);
    let target = open_target_path(&path, status.exists)?;
    let target_kind = if status.exists { "file" } else { "folder" };

    app_handle
        .opener()
        .open_path(target.to_string_lossy().to_string(), None::<String>)
        .map_err(|error| CaptureErrorResponse {
            code: "app_log_open_failed".to_string(),
            message: format!(
                "Failed to open general application log {target_kind} '{}': {error}",
                target.display()
            ),
        })?;

    Ok(status)
}

#[tauri::command]
pub fn get_general_app_log_status(
    app_handle: tauri::AppHandle,
) -> Result<GeneralAppLogStatus, CaptureErrorResponse> {
    status(&app_handle)
}

#[tauri::command]
pub fn open_general_app_log(
    app_handle: tauri::AppHandle,
) -> Result<GeneralAppLogStatus, CaptureErrorResponse> {
    open(&app_handle)
}
