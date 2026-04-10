mod native_capture;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(native_capture::NativeCaptureState::default())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            native_capture::get_capture_support,
            native_capture::get_capture_permissions,
            native_capture::start_native_capture,
            native_capture::stop_native_capture,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
