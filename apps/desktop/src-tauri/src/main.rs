// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    mnema_lib::maybe_run_speaker_analysis_helper_and_exit();
    mnema_lib::run()
}
