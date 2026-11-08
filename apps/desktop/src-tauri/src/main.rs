// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    app_core::speaker_analysis_runtime::maybe_run_subprocess_helper_and_exit();
    mnema_lib::run()
}
