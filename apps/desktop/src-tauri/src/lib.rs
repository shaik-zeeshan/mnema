mod app_infra;
mod native_capture;
mod native_capture_inactivity;
mod native_capture_output;
mod native_capture_settings;
mod native_capture_system_idle;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(native_capture::NativeCaptureState::default())
        .manage(native_capture::MicrophoneControllerPreferencesState::default())
        .manage(native_capture::MicrophoneDeviceChangeNotifierState::default())
        .manage(native_capture::RecordingSettingsState::default())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            app_infra::get_app_infra_status,
            app_infra::submit_debug_cpu_job,
            app_infra::list_app_jobs,
            app_infra::get_app_job,
            app_infra::insert_frame_and_enqueue_processing_job,
            app_infra::insert_frame_and_enqueue_ocr,
            app_infra::list_frames,
            app_infra::get_frame,
            app_infra::list_processing_jobs,
            app_infra::get_processing_job,
            app_infra::get_processing_result,
            app_infra::list_processing_results,
            native_capture::get_capture_support,
            native_capture::get_capture_permissions,
            native_capture::get_idle_debug,
            native_capture::get_recording_settings,
            native_capture::update_recording_settings,
            native_capture::get_microphone_controller_state,
            native_capture::update_microphone_controller,
            native_capture::start_native_capture,
            native_capture::stop_native_capture,
        ])
        .setup(|app| {
            app_infra::initialize(app).map_err(std::io::Error::other)?;
            native_capture::initialize_recording_settings_from_disk(app.handle());
            native_capture::start_microphone_device_change_notifier(app.handle().clone());
            native_capture::maybe_auto_start_native_capture(app.handle());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
