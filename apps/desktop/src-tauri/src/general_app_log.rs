use capture_runtime::delete_debug_log_files;
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

fn delete_log_file_at_path(path: &Path) -> Result<(), CaptureErrorResponse> {
    delete_debug_log_files(path).map_err(|error| CaptureErrorResponse {
        code: "app_log_delete_failed".to_string(),
        message: format!("Failed to delete general application log: {error}"),
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

pub(crate) fn delete(
    app_handle: &tauri::AppHandle,
) -> Result<GeneralAppLogStatus, CaptureErrorResponse> {
    let path = general_app_log_path(app_handle)?;
    delete_log_file_at_path(&path)?;
    Ok(status_for_path(&path))
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

#[tauri::command]
pub fn delete_general_app_log(
    app_handle: tauri::AppHandle,
) -> Result<GeneralAppLogStatus, CaptureErrorResponse> {
    delete(&app_handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("general-app-log-{label}-{unique}"));
            fs::create_dir_all(&path).expect("test directory should be created");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn status_for_path_reports_file_existence() {
        let dir = TestDir::new("status");
        let log_path = dir.path().join("rust.log");

        let missing = status_for_path(&log_path);
        assert_eq!(missing.path, log_path.to_string_lossy().to_string());
        assert!(!missing.exists);

        fs::write(&log_path, "hello").expect("log file should write");

        let present = status_for_path(&log_path);
        assert!(present.exists);
    }

    #[test]
    fn delete_log_file_at_path_removes_existing_file_and_backups() {
        let dir = TestDir::new("delete");
        let log_path = dir.path().join("rust.log");
        fs::write(&log_path, "current").expect("log file should write");
        fs::write(log_path.with_file_name("rust.log.1"), "backup")
            .expect("backup log file should write");

        delete_log_file_at_path(&log_path).expect("log file should be deleted");
        assert!(!log_path.exists());
        assert!(!log_path.with_file_name("rust.log.1").exists());

        delete_log_file_at_path(&log_path).expect("missing log file should be ignored");
    }
}
