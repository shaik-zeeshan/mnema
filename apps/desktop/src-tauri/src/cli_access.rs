use std::path::PathBuf;

use tauri::Manager;

fn access_config_dir(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    app_handle
        .path()
        .app_config_dir()
        .map_err(|error| format!("failed to resolve app config dir: {error}"))
}

fn access(
    app_handle: &tauri::AppHandle,
) -> Result<::app_infra::brokered_access::BrokeredCaptureAccess, String> {
    Ok(
        ::app_infra::brokered_access::BrokeredCaptureAccess::from_config_dir(access_config_dir(
            app_handle,
        )?),
    )
}

#[tauri::command]
pub async fn list_cli_access_grants(
    app_handle: tauri::AppHandle,
) -> Result<::app_infra::brokered_access::BrokerGrantFile, String> {
    access(&app_handle)?
        .list_grants()
        .map_err(|error| format!("failed to load CLI Access grants: {error}"))
}

/// Block a tool: a standing rejection. The row stays visible in Settings, is
/// denied without ever opening the approval window, and never idle-expires
/// (ADR 0059). Returns whether a row actually changed.
#[tauri::command]
pub async fn block_cli_access_client(
    app_handle: tauri::AppHandle,
    client_name: String,
) -> Result<bool, String> {
    access(&app_handle)?
        .block_client(&client_name)
        .map_err(|error| format!("failed to block CLI Access for this tool: {error}"))
}

#[tauri::command]
pub async fn unblock_cli_access_client(
    app_handle: tauri::AppHandle,
    client_name: String,
) -> Result<bool, String> {
    access(&app_handle)?
        .unblock_client(&client_name)
        .map_err(|error| format!("failed to re-enable CLI Access for this tool: {error}"))
}

#[tauri::command]
pub async fn list_cli_access_history(
    app_handle: tauri::AppHandle,
) -> Result<::app_infra::brokered_access::BrokerAuditFile, String> {
    access(&app_handle)?
        .list_history()
        .map_err(|error| format!("failed to load CLI Access history: {error}"))
}

#[tauri::command]
pub async fn get_cli_access_status(
    app_handle: tauri::AppHandle,
) -> Result<crate::app_infra::MnemaCliStatus, String> {
    crate::app_infra::get_cli_status_inner(app_handle).await
}

#[tauri::command]
pub async fn install_cli(
    app_handle: tauri::AppHandle,
) -> Result<crate::app_infra::MnemaCliStatus, String> {
    crate::app_infra::install_cli_inner(app_handle).await
}
