use capture_types::{CaptureErrorResponse, NativeCaptureDebugLogStatus};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::Manager;

const NATIVE_CAPTURE_DEBUG_LOG_FILE_NAME: &str = "native-capture-debug.log";

#[derive(Debug, Clone, Default)]
struct NativeCaptureDebugLogRuntime {
    enabled: bool,
    path: Option<PathBuf>,
}

fn runtime() -> &'static Mutex<NativeCaptureDebugLogRuntime> {
    static RUNTIME: OnceLock<Mutex<NativeCaptureDebugLogRuntime>> = OnceLock::new();
    RUNTIME.get_or_init(|| Mutex::new(NativeCaptureDebugLogRuntime::default()))
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

pub(crate) fn native_capture_debug_log_path(app_handle: &tauri::AppHandle) -> PathBuf {
    if let Ok(config_dir) = app_handle.path().app_config_dir() {
        return config_dir.join(NATIVE_CAPTURE_DEBUG_LOG_FILE_NAME);
    }

    PathBuf::from(crate::native_capture_settings::default_save_directory())
        .join(NATIVE_CAPTURE_DEBUG_LOG_FILE_NAME)
}

pub(crate) fn configure(app_handle: &tauri::AppHandle, enabled: bool) {
    let mut runtime = runtime()
        .lock()
        .expect("native capture debug log state poisoned");
    runtime.enabled = enabled;
    runtime.path = Some(native_capture_debug_log_path(app_handle));
}

fn append_log_line_to_path(path: &Path, message: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "[{}] {}", now_unix_ms(), message)
}

pub(crate) fn log(message: impl AsRef<str>) {
    let message = message.as_ref();
    eprintln!("{message}");

    let path = {
        let runtime = runtime()
            .lock()
            .expect("native capture debug log state poisoned");
        if !runtime.enabled {
            return;
        }
        runtime.path.clone()
    };

    if let Some(path) = path {
        let _ = append_log_line_to_path(&path, message);
    }
}

fn status_for_path(enabled: bool, path: &Path) -> NativeCaptureDebugLogStatus {
    NativeCaptureDebugLogStatus {
        enabled,
        path: path.to_string_lossy().to_string(),
        exists: path.exists(),
    }
}

fn delete_log_file_at_path(path: &Path) -> Result<(), CaptureErrorResponse> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CaptureErrorResponse {
            code: "io_error".to_string(),
            message: format!("Failed to delete native capture debug log: {error}"),
        }),
    }
}

pub(crate) fn status(app_handle: &tauri::AppHandle, enabled: bool) -> NativeCaptureDebugLogStatus {
    status_for_path(enabled, &native_capture_debug_log_path(app_handle))
}

pub(crate) fn delete(
    app_handle: &tauri::AppHandle,
    enabled: bool,
) -> Result<NativeCaptureDebugLogStatus, CaptureErrorResponse> {
    let path = native_capture_debug_log_path(app_handle);
    delete_log_file_at_path(&path)?;
    Ok(status_for_path(enabled, &path))
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
            let path =
                std::env::temp_dir().join(format!("native-capture-debug-log-{label}-{unique}"));
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
    fn append_log_line_to_path_creates_and_appends_log_file() {
        let dir = TestDir::new("append");
        let log_path = dir.path().join("native-capture-debug.log");

        append_log_line_to_path(&log_path, "first line").expect("first write should succeed");
        append_log_line_to_path(&log_path, "second line").expect("second write should succeed");

        let contents = fs::read_to_string(&log_path).expect("log file should exist");
        assert!(contents.contains("first line"));
        assert!(contents.contains("second line"));
    }

    #[test]
    fn status_for_path_reports_enabled_flag_and_file_existence() {
        let dir = TestDir::new("status");
        let log_path = dir.path().join("native-capture-debug.log");

        let missing = status_for_path(true, &log_path);
        assert!(missing.enabled);
        assert_eq!(missing.path, log_path.to_string_lossy().to_string());
        assert!(!missing.exists);

        fs::write(&log_path, "hello").expect("log file should write");

        let present = status_for_path(false, &log_path);
        assert!(!present.enabled);
        assert!(present.exists);
    }

    #[test]
    fn delete_log_file_at_path_removes_existing_file_and_ignores_missing_file() {
        let dir = TestDir::new("delete");
        let log_path = dir.path().join("native-capture-debug.log");
        fs::write(&log_path, "hello").expect("log file should write");

        delete_log_file_at_path(&log_path).expect("existing log file should be deleted");
        assert!(!log_path.exists());

        delete_log_file_at_path(&log_path).expect("missing log file should be ignored");
    }
}
