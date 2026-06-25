use capture_runtime::{configure_debug_log, debug_log_files_exist, delete_debug_log_files};
use capture_types::{CaptureErrorResponse, NativeCaptureDebugLogStatus};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;
use tauri::Manager;
use tauri_plugin_opener::OpenerExt;

const NATIVE_CAPTURE_DEBUG_LOG_FILE_NAME: &str = "native-capture-debug.log";

/// Runtime gate for whether app-target Debug/Trace records reach the
/// `tauri_plugin_log` sinks (stderr + the `rust` application log file).
///
/// Mirrors the developer-options setting in release builds so that a packaged
/// app stays at Info verbosity until the user opts in. Info/Warn/Error are never
/// gated, and debug builds always emit Debug regardless (see
/// [`app_log_record_allowed`]).
static APP_DEBUG_LOGGING_ENABLED: AtomicBool = AtomicBool::new(false);

/// Mirror the developer-options setting into the app-log verbosity gate.
pub(crate) fn set_app_debug_logging_enabled(enabled: bool) {
    APP_DEBUG_LOGGING_ENABLED.store(enabled, Ordering::Relaxed);
}

/// Whether a record at `level` from an app target should reach the plugin sinks.
///
/// `Info`/`Warn`/`Error` always pass. `Debug`/`Trace` pass unconditionally in
/// debug builds, and in release builds only while developer options have enabled
/// debug logging.
pub(crate) fn app_log_record_allowed(level: tauri_plugin_log::log::Level) -> bool {
    record_allowed(
        level,
        cfg!(debug_assertions),
        APP_DEBUG_LOGGING_ENABLED.load(Ordering::Relaxed),
    )
}

/// Pure decision used by [`app_log_record_allowed`], split out so the
/// release-build gating branch is exercisable under a debug test build.
fn record_allowed(
    level: tauri_plugin_log::log::Level,
    debug_build: bool,
    runtime_enabled: bool,
) -> bool {
    if level <= tauri_plugin_log::log::Level::Info {
        return true;
    }
    debug_build || runtime_enabled
}

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

/// Resolve what to hand the OS opener: the base log file when it is present on
/// disk, otherwise its containing folder. The decision keys off the base file
/// existing (`base_exists`), not the rotation-aware status flag — when only a
/// rotated backup remains, revealing the folder still surfaces it.
fn open_target_path(path: &Path, base_exists: bool) -> Result<PathBuf, CaptureErrorResponse> {
    if base_exists {
        return Ok(path.to_path_buf());
    }

    path.parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| CaptureErrorResponse {
            code: "io_error".to_string(),
            message: format!(
                "Failed to resolve containing directory for native capture debug log path '{}'",
                path.display()
            ),
        })
}

pub(crate) fn status(app_handle: &tauri::AppHandle, enabled: bool) -> NativeCaptureDebugLogStatus {
    status_for_path(enabled, &native_capture_debug_log_path(app_handle))
}

pub(crate) fn open(
    app_handle: &tauri::AppHandle,
    enabled: bool,
) -> Result<NativeCaptureDebugLogStatus, CaptureErrorResponse> {
    let path = native_capture_debug_log_path(app_handle);
    let base_exists = path.is_file();
    let target = open_target_path(&path, base_exists)?;
    let target_kind = if base_exists { "file" } else { "folder" };

    app_handle
        .opener()
        .open_path(target.to_string_lossy().to_string(), None::<String>)
        .map_err(|error| CaptureErrorResponse {
            code: "io_error".to_string(),
            message: format!(
                "Failed to open native capture debug log {target_kind} '{}': {error}",
                target.display()
            ),
        })?;

    Ok(status_for_path(enabled, &path))
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
    fn record_allowed_always_passes_info_and_more_severe_levels() {
        use tauri_plugin_log::log::Level;

        for level in [Level::Error, Level::Warn, Level::Info] {
            // Severe records pass even in a release build with the gate off.
            assert!(record_allowed(level, false, false));
        }
    }

    #[test]
    fn record_allowed_gates_debug_on_runtime_flag_in_release_builds() {
        use tauri_plugin_log::log::Level;

        // Release build, developer options off: Debug/Trace are suppressed.
        assert!(!record_allowed(Level::Debug, false, false));
        assert!(!record_allowed(Level::Trace, false, false));

        // Release build, developer options on: Debug/Trace flow through.
        assert!(record_allowed(Level::Debug, false, true));
        assert!(record_allowed(Level::Trace, false, true));

        // Debug build: Debug/Trace always flow through, ignoring the flag.
        assert!(record_allowed(Level::Debug, true, false));
        assert!(record_allowed(Level::Trace, true, false));
    }

    #[test]
    fn set_app_debug_logging_enabled_toggles_the_runtime_gate() {
        use tauri_plugin_log::log::Level;

        set_app_debug_logging_enabled(true);
        assert!(app_log_record_allowed(Level::Info));
        // In a debug test build Debug always passes; assert the gate read itself
        // reflects the stored value.
        assert!(APP_DEBUG_LOGGING_ENABLED.load(Ordering::Relaxed));

        set_app_debug_logging_enabled(false);
        assert!(!APP_DEBUG_LOGGING_ENABLED.load(Ordering::Relaxed));
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
    fn open_target_path_picks_the_file_when_present_and_the_folder_otherwise() {
        let dir = TestDir::new("open");
        let log_path = dir.path().join("native-capture-debug.log");

        // Base file missing → reveal the containing folder.
        let folder = open_target_path(&log_path, false).expect("parent directory should resolve");
        assert_eq!(folder, dir.path());

        // Base file present → open the file itself.
        let file = open_target_path(&log_path, true).expect("file target should resolve");
        assert_eq!(file, log_path);
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
