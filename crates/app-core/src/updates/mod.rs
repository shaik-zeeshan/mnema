//! The GPUI app's own updater (BUILD-PROMPT §9.2), shared with the Tauri app:
//! the channel + settings, the status/state machine payload and its pure
//! decisions (license Update Window gate, `latest.json` size lookup), and the
//! install gate. The Tauri host keeps the `tauri_plugin_updater` I/O.

#[cfg(target_os = "macos")]
pub mod install_gate;
mod settings;
mod status;

#[cfg(target_os = "macos")]
pub use install_gate::{check_update_bundle, GateRefusal, OwnIdentity};
pub use settings::{
    current_settings, initialize, load_app_update_settings_from_path, persist_settings,
    AppUpdateChannel, AppUpdateSettings, AppUpdateSettingsRuntime, AppUpdateSettingsState,
    PREVIEW_UPDATE_ENDPOINT, STABLE_UPDATE_ENDPOINT,
};
pub use status::{
    apply_running_build_window_gate, build_out_of_window, decide_remote_update, emit_status,
    out_of_window_notice, restart_after_update_error, update_size_bytes, user_facing_error_message,
    AppUpdateAppInfo, AppUpdateError, AppUpdateErrorKind, AppUpdateInfo, AppUpdateProgress,
    AppUpdateState, AppUpdateStatus, RemoteUpdateDecision, APP_UPDATE_STATUS_CHANGED_EVENT,
    UPDATE_CHECK_INTERVAL,
};
