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

#[tauri::command]
pub async fn revoke_cli_access_grant(
    app_handle: tauri::AppHandle,
    grant_id: String,
) -> Result<bool, String> {
    access(&app_handle)?
        .revoke_grant(&grant_id)
        .map_err(|error| format!("failed to revoke CLI Access grant: {error}"))
}

#[tauri::command]
pub async fn revoke_cli_access_for_client(
    app_handle: tauri::AppHandle,
    client_name: String,
) -> Result<u32, String> {
    access(&app_handle)?
        .revoke_grants_for_client(&client_name)
        .map_err(|error| format!("failed to revoke CLI Access grants: {error}"))
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

#[tauri::command]
pub async fn reinstall_cli(
    app_handle: tauri::AppHandle,
) -> Result<crate::app_infra::MnemaCliStatus, String> {
    crate::app_infra::install_cli_inner(app_handle).await
}
