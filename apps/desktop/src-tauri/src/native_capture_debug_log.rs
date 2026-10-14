use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;

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

#[track_caller]
pub(crate) fn log(message: impl AsRef<str>) {
    log_at(tauri_plugin_log::log::Level::Debug, message.as_ref());
}

#[track_caller]
pub(crate) fn log_info(message: impl AsRef<str>) {
    log_at(tauri_plugin_log::log::Level::Info, message.as_ref());
}

#[track_caller]
pub(crate) fn log_warn(message: impl AsRef<str>) {
    log_at(tauri_plugin_log::log::Level::Warn, message.as_ref());
}

#[track_caller]
pub(crate) fn log_error(message: impl AsRef<str>) {
    log_at(tauri_plugin_log::log::Level::Error, message.as_ref());
}

/// Logs under the caller's module (`mnema_lib::semantic_search_worker`), not this
/// file's, so the app log and the Developer window can tell sources apart.
#[track_caller]
fn log_at(level: tauri_plugin_log::log::Level, message: &str) {
    let target = caller_target(std::panic::Location::caller().file());
    tauri_plugin_log::log::log!(target: &target, level, "{message}");
}

fn caller_target(file: &str) -> String {
    let file = file.replace('\\', "/");
    let module = file
        .rsplit_once("/src/")
        .map(|(_, rest)| rest)
        .or_else(|| file.strip_prefix("src/"))
        .unwrap_or(&file);
    let module = module.strip_suffix(".rs").unwrap_or(module);
    let module = module.strip_suffix("/mod").unwrap_or(module);
    format!("mnema_lib::{}", module.replace('/', "::"))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn caller_target_names_the_calling_module() {
        assert_eq!(
            super::caller_target("apps/desktop/src-tauri/src/semantic_search_worker.rs"),
            "mnema_lib::semantic_search_worker"
        );
        assert_eq!(
            super::caller_target("src/native_capture/lifecycle.rs"),
            "mnema_lib::native_capture::lifecycle"
        );
        assert_eq!(
            super::caller_target("C:\\mnema\\apps\\desktop\\src-tauri\\src\\ask_ai\\mod.rs"),
            "mnema_lib::ask_ai"
        );
    }
}
