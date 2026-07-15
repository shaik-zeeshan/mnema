// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // FIRST: pin ORT_DYLIB_PATH to the bundled, exe-adjacent onnxruntime.dll
    // (Windows dynamic-ORT, #137). This runs before ANY other entry — including
    // `maybe_run_speaker_analysis_helper_and_exit`, which re-invokes THIS exe as
    // the speakrs helper and must have the dylib path set before touching ORT.
    // No-op off Windows (ONNX Runtime is statically linked there).
    mnema_lib::ensure_ort_dylib_path();

    mnema_lib::maybe_run_speaker_analysis_helper_and_exit();
    mnema_lib::maybe_run_windows_inactivity_smoke_and_exit();
    mnema_lib::maybe_run_windows_transient_liveness_smoke_and_exit();
    mnema_lib::maybe_run_windows_browser_url_smoke_and_exit();
    mnema_lib::run()
}
