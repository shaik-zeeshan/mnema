mod app_infra;
mod general_app_log;
mod managed_storage_layout;
mod native_capture;
mod windows;

use tauri::Manager;
use tauri_plugin_log::{Target, TargetKind, WEBVIEW_TARGET};

pub(crate) const APP_LOG_FILE_NAME: &str = "rust";

const APP_LOG_TARGET_PREFIXES: &[&str] = &[
    "mnema",
    "mnema_lib",
    "app_infra",
    "capture_runtime",
    "capture_screen",
    "capture_microphone",
    "capture_writers",
    "capture_types",
    WEBVIEW_TARGET,
];

fn is_app_log_target(target: &str) -> bool {
    APP_LOG_TARGET_PREFIXES.iter().any(|prefix| {
        target == *prefix
            || target
                .strip_prefix(*prefix)
                .is_some_and(|suffix| suffix.starts_with("::"))
    })
}

fn should_forward_window_event(event: &tauri::WindowEvent, webview_window_found: bool) -> bool {
    matches!(event, tauri::WindowEvent::Destroyed) || webview_window_found
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(native_capture::NativeCaptureState::default())
        .manage(native_capture::MicrophoneControllerPreferencesState::default())
        .manage(native_capture::MicrophoneDeviceChangeNotifierState::default())
        .manage(native_capture::SystemWakeNotifierState::default())
        .manage(native_capture::RecordingSettingsState::default())
        .manage(native_capture::AppNotificationsState::default())
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(tauri_plugin_log::log::LevelFilter::Info)
                .level_for("capture_runtime", tauri_plugin_log::log::LevelFilter::Debug)
                .level_for("mnema_lib", tauri_plugin_log::log::LevelFilter::Debug)
                .filter(|metadata| is_app_log_target(metadata.target()))
                .targets([
                    Target::new(TargetKind::Stderr),
                    Target::new(TargetKind::LogDir {
                        file_name: Some(APP_LOG_FILE_NAME.to_string()),
                    }),
                ])
                .build(),
        )
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .on_window_event(|window, event| {
            let webview_window = window.get_webview_window(window.label());
            if !should_forward_window_event(event, webview_window.is_some()) {
                return;
            }

            windows::handle_window_event(&window.app_handle(), window.label(), event);
        })
        .invoke_handler(tauri::generate_handler![
            app_infra::get_app_infra_status,
            app_infra::submit_debug_cpu_job,
            app_infra::list_app_jobs,
            app_infra::get_app_job,
            app_infra::debug_insert_frame_and_enqueue_processing_job,
            app_infra::debug_insert_frame_and_enqueue_ocr,
            app_infra::reprocess_captured_frame_ocr,
            app_infra::classify_hidden_segment_workspace,
            app_infra::list_frames,
            app_infra::list_frame_summaries_in_range,
            app_infra::get_latest_frame_in_range,
            app_infra::list_audio_segments,
            app_infra::get_audio_segment_media,
            app_infra::get_frame,
            app_infra::get_earliest_earlier_equivalent_frame,
            app_infra::get_nearest_earlier_equivalent_frame,
            app_infra::get_timeline_window_around_frame,
            app_infra::get_frame_preview,
            app_infra::list_processing_jobs,
            app_infra::get_processing_job,
            app_infra::get_processing_result,
            app_infra::list_processing_results,
            general_app_log::get_general_app_log_status,
            general_app_log::open_general_app_log,
            general_app_log::delete_general_app_log,
            native_capture::get_capture_support,
            native_capture::get_capture_permissions,
            native_capture::get_idle_debug,
            native_capture::get_app_notifications,
            native_capture::clear_app_notification,
            native_capture::clear_app_notifications,
            native_capture::get_recording_settings,
            native_capture::update_recording_settings,
            native_capture::get_native_capture_debug_log_status,
            native_capture::delete_native_capture_debug_log,
            native_capture::get_microphone_controller_state,
            native_capture::update_microphone_controller,
            native_capture::start_native_capture,
            native_capture::stop_native_capture,
            windows::open_settings_window,
            windows::open_debug_window,
            windows::close_current_window,
        ])
        .setup(|app| {
            native_capture::initialize_recording_settings_from_disk(app.handle());
            native_capture::install_panic_hook();
            app_infra::initialize(app).map_err(std::io::Error::other)?;
            native_capture::start_microphone_device_change_notifier(app.handle().clone());
            native_capture::start_system_wake_notifier(app.handle().clone());
            native_capture::maybe_auto_start_native_capture(app.handle());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::{is_app_log_target, should_forward_window_event};

    #[test]
    fn app_log_filter_keeps_only_our_targets() {
        assert!(is_app_log_target("mnema_lib::native_capture"));
        assert!(is_app_log_target("capture_runtime"));
        assert!(is_app_log_target("app_infra::processing::runtime"));
        assert!(is_app_log_target(tauri_plugin_log::WEBVIEW_TARGET));

        assert!(!is_app_log_target("ort::logging"));
        assert!(!is_app_log_target("tauri"));
        assert!(!is_app_log_target("sqlx::query"));
        assert!(!is_app_log_target("capture_runtime_extra"));
    }

    #[test]
    fn destroyed_events_are_forwarded_even_when_manager_lookup_fails() {
        assert!(should_forward_window_event(
            &tauri::WindowEvent::Destroyed,
            false,
        ));
    }

    #[test]
    fn non_destroyed_events_without_a_resolved_webview_window_are_ignored() {
        assert!(!should_forward_window_event(
            &tauri::WindowEvent::Focused(true),
            false,
        ));
    }
}
