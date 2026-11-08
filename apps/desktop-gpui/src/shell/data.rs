//! Fills the shell's models from `AppCore`. Views never call the backend; this is
//! the one place that does, off the main thread.
//!
//! Every input the shell shows is still behind a src-tauri command while S2/S3
//! move the modules, so each one returns empty with a pointer to wire it.

use app_core::AppCore;

use super::model::{ShellModel, StatusBarModel};

/// Runs on the tokio runtime (`Tokio::spawn`); returns what app-core can answer today.
pub async fn load(_core: &AppCore) -> ShellModel {
    ShellModel {
        // shortcut: get_capture_permissions / get_idle_debug / start state not in
        // app-core until S3 (native_capture); wire it then.
        // shortcut: get_recording_settings (selection, local only, developer
        // options) not in app-core until S2 (recording settings); wire it then.
        // shortcut: get_license_status (read-only) not in app-core until S2 (licensing); wire it then.
        // shortcut: get_ai_runtime_status (engine pill) not in app-core until S2 (ai_runtime); wire it then.
        // shortcut: get_usage_charts (today) not in app-core until S2 (app_infra commands); wire it then.
        // shortcut: get_storage_usage not in app-core until S2 (storage); wire it then.
        status: StatusBarModel {
            local_only: true,
            ..Default::default()
        },
        // shortcut: get_app_notifications not in app-core until S3 (native_capture); wire it then.
        notifications: Vec::new(),
        developer_options: false,
    }
}
