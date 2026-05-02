mod app_infra;
mod general_app_log;
mod native_capture;
mod native_capture_debug_log;
mod native_capture_inactivity;
mod native_capture_output;
mod native_capture_settings;
mod native_capture_system_idle;

use tauri_plugin_log::{Target, TargetKind};

pub(crate) const APP_LOG_FILE_NAME: &str = "rust";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(native_capture::NativeCaptureState::default())
        .manage(native_capture::MicrophoneControllerPreferencesState::default())
        .manage(native_capture::MicrophoneDeviceChangeNotifierState::default())
        .manage(native_capture::RecordingSettingsState::default())
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(tauri_plugin_log::log::LevelFilter::Info)
                .level_for("capture_runtime", tauri_plugin_log::log::LevelFilter::Debug)
                .level_for("z_lib", tauri_plugin_log::log::LevelFilter::Debug)
                .targets([
                    Target::new(TargetKind::Stderr),
                    Target::new(TargetKind::LogDir {
                        file_name: Some(APP_LOG_FILE_NAME.to_string()),
                    }),
                ])
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
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
            native_capture::get_recording_settings,
            native_capture::update_recording_settings,
            native_capture::get_native_capture_debug_log_status,
            native_capture::delete_native_capture_debug_log,
            native_capture::get_microphone_controller_state,
            native_capture::update_microphone_controller,
            native_capture::start_native_capture,
            native_capture::stop_native_capture,
        ])
        .setup(|app| {
            native_capture::initialize_recording_settings_from_disk(app.handle());
            native_capture_debug_log::install_panic_hook();
            app_infra::initialize(app).map_err(std::io::Error::other)?;
            native_capture::start_microphone_device_change_notifier(app.handle().clone());
            native_capture::maybe_auto_start_native_capture(app.handle());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
