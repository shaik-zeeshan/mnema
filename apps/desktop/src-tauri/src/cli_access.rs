use std::sync::Arc;

use app_core::cli_access::{self as cli, MnemaCliStatus};
use app_core::AppCore;
use tauri::State;

#[tauri::command]
pub async fn list_cli_access_grants(
    core: State<'_, Arc<AppCore>>,
) -> Result<::app_infra::brokered_access::BrokerGrantFile, String> {
    cli::list_cli_access_grants(&core).await
}

#[tauri::command]
pub async fn block_cli_access_client(
    core: State<'_, Arc<AppCore>>,
    client_name: String,
) -> Result<bool, String> {
    cli::block_cli_access_client(&core, client_name).await
}

#[tauri::command]
pub async fn unblock_cli_access_client(
    core: State<'_, Arc<AppCore>>,
    client_name: String,
) -> Result<bool, String> {
    cli::unblock_cli_access_client(&core, client_name).await
}

#[tauri::command]
pub async fn list_cli_access_history(
    core: State<'_, Arc<AppCore>>,
) -> Result<::app_infra::brokered_access::BrokerAuditFile, String> {
    cli::list_cli_access_history(&core).await
}

#[tauri::command]
pub async fn get_cli_access_status() -> Result<MnemaCliStatus, String> {
    cli::get_cli_status_inner().await
}

#[tauri::command]
pub async fn install_cli() -> Result<MnemaCliStatus, String> {
    cli::install_cli_inner().await
}
