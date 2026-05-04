use capture_runtime::{configure_debug_log, debug_log_files_exist, delete_debug_log_files};
use capture_types::{CaptureErrorResponse, NativeCaptureDebugLogStatus};
use std::path::{Path, PathBuf};
use std::sync::Once;
use tauri::Manager;

const NATIVE_CAPTURE_DEBUG_LOG_FILE_NAME: &str = "native-capture-debug.log";

pub(crate) fn native_capture_debug_log_path(app_handle: &tauri::AppHandle) -> PathBuf {
    if let Ok(config_dir) = app_handle.path().app_config_dir() {
        return config_dir.join(NATIVE_CAPTURE_DEBUG_LOG_FILE_NAME);
    }

    PathBuf::from(super::settings::default_save_directory())
        .join(NATIVE_CAPTURE_DEBUG_LOG_FILE_NAME)
}

pub(crate) fn configure(app_handle: &tauri::AppHandle, enabled: bool) {
    configure_debug_log(enabled, Some(native_capture_debug_log_path(app_handle)));
}

fn panic_payload_message(info: &std::panic::PanicHookInfo<'_>) -> String {
    if let Some(message) = info.payload().downcast_ref::<&str>() {
        return (*message).to_string();
    }

    if let Some(message) = info.payload().downcast_ref::<String>() {
        return message.clone();
    }

    "non-string panic payload".to_string()
}

pub(crate) fn install_panic_hook() {
    static PANIC_HOOK_INSTALLED: Once = Once::new();

    PANIC_HOOK_INSTALLED.call_once(|| {
        let previous_hook = std::panic::take_hook();

        std::panic::set_hook(Box::new(move |info| {
            let thread = std::thread::current();
            let thread_name = thread.name().unwrap_or("unnamed");
            let location = info
                .location()
                .map(|location| {
                    format!(
                        "{}:{}:{}",
                        location.file(),
                        location.line(),
                        location.column()
                    )
                })
                .unwrap_or_else(|| "unknown location".to_string());

            log_error(format!(
                "panic on thread '{thread_name}' at {location}: {}",
                panic_payload_message(info)
            ));

            previous_hook(info);
        }));
    });
}

pub(crate) fn log(message: impl AsRef<str>) {
    let message = message.as_ref();
    tauri_plugin_log::log::debug!("{message}");
    capture_runtime::write_debug_log_to_file(message);
}

pub(crate) fn log_info(message: impl AsRef<str>) {
    let message = message.as_ref();
    tauri_plugin_log::log::info!("{message}");
    capture_runtime::write_debug_log_to_file(message);
}

pub(crate) fn log_warn(message: impl AsRef<str>) {
    let message = message.as_ref();
    tauri_plugin_log::log::warn!("{message}");
    capture_runtime::write_debug_log_to_file(message);
}

pub(crate) fn log_error(message: impl AsRef<str>) {
    let message = message.as_ref();
    tauri_plugin_log::log::error!("{message}");
    capture_runtime::write_debug_log_to_file(message);
}

fn status_for_path(enabled: bool, path: &Path) -> NativeCaptureDebugLogStatus {
    NativeCaptureDebugLogStatus {
        enabled,
        path: path.to_string_lossy().to_string(),
        exists: debug_log_files_exist(path),
    }
}

fn delete_log_file_at_path(path: &Path) -> Result<(), CaptureErrorResponse> {
    match delete_debug_log_files(path) {
        Ok(()) => Ok(()),
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
    fn status_for_path_reports_enabled_flag_and_rotated_file_existence() {
        let dir = TestDir::new("status");
        let log_path = dir.path().join("native-capture-debug.log");

        let missing = status_for_path(true, &log_path);
        assert!(missing.enabled);
        assert_eq!(missing.path, log_path.to_string_lossy().to_string());
        assert!(!missing.exists);

        fs::write(
            log_path.with_file_name("native-capture-debug.log.1"),
            "hello",
        )
        .expect("rotated log file should write");

        let present = status_for_path(false, &log_path);
        assert!(!present.enabled);
        assert!(present.exists);
    }

    #[test]
    fn delete_log_file_at_path_removes_existing_file_and_backups() {
        let dir = TestDir::new("delete");
        let log_path = dir.path().join("native-capture-debug.log");
        fs::write(&log_path, "hello").expect("log file should write");
        fs::write(
            log_path.with_file_name("native-capture-debug.log.1"),
            "world",
        )
        .expect("backup log file should write");

        delete_log_file_at_path(&log_path).expect("existing log file should be deleted");
        assert!(!log_path.exists());
        assert!(!log_path
            .with_file_name("native-capture-debug.log.1")
            .exists());

        delete_log_file_at_path(&log_path).expect("missing log file should be ignored");
    }

    #[test]
    fn info_warn_and_error_logs_append_to_debug_log_file() {
        let dir = TestDir::new("levels");
        let log_path = dir.path().join("native-capture-debug.log");

        configure_debug_log(true, Some(log_path.clone()));
        log_info("info message");
        log_warn("warn message");
        log_error("error message");
        configure_debug_log(false, None);

        let contents = fs::read_to_string(&log_path).expect("log file should exist");
        assert!(contents.contains("info message"));
        assert!(contents.contains("warn message"));
        assert!(contents.contains("error message"));
    }
}
