use std::path::PathBuf;

use app_infra::brokered_access::{
    BrokerClientIdentity, BrokerClientIdentitySource, BrokerSearchRequest, BrokerTimelineRequest,
    BrokeredCaptureAccess, BrokeredCaptureRequest, BrokeredCaptureResponse,
};
use serde::Deserialize;
use tauri::Manager;

fn access_config_dir(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    app_handle
        .path()
        .app_config_dir()
        .map_err(|error| format!("failed to resolve app config dir: {error}"))
}

fn broker_access(app_handle: &tauri::AppHandle) -> Result<BrokeredCaptureAccess, String> {
    Ok(BrokeredCaptureAccess::from_config_dir(access_config_dir(
        app_handle,
    )?))
}

fn pi_broker_identity() -> Result<BrokerClientIdentity, String> {
    BrokerClientIdentity::new("PI", BrokerClientIdentitySource::Inferred)
        .map_err(|error| error.to_string())
}

async fn execute_pi_broker_request(
    app_handle: tauri::AppHandle,
    request: BrokeredCaptureRequest,
) -> Result<BrokeredCaptureResponse, String> {
    broker_access(&app_handle)?
        .execute_for_identity(pi_broker_identity()?, request)
        .await
        .map_err(|error| format!("failed to execute Ask AI broker request: {error}"))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskAiShowTextRequest {
    opaque_id: String,
}

#[tauri::command]
pub async fn get_pi_runtime_status(
    app_handle: tauri::AppHandle,
) -> Result<crate::app_infra::PiRuntimeStatus, String> {
    crate::app_infra::get_pi_runtime_status_inner(app_handle).await
}

#[tauri::command]
pub async fn ask_ai_broker_search(
    app_handle: tauri::AppHandle,
    request: BrokerSearchRequest,
) -> Result<BrokeredCaptureResponse, String> {
    execute_pi_broker_request(app_handle, BrokeredCaptureRequest::Search(request)).await
}

#[tauri::command]
pub async fn ask_ai_broker_timeline(
    app_handle: tauri::AppHandle,
    request: BrokerTimelineRequest,
) -> Result<BrokeredCaptureResponse, String> {
    execute_pi_broker_request(app_handle, BrokeredCaptureRequest::Timeline(request)).await
}

#[tauri::command]
pub async fn ask_ai_broker_show_text(
    app_handle: tauri::AppHandle,
    request: AskAiShowTextRequest,
) -> Result<BrokeredCaptureResponse, String> {
    execute_pi_broker_request(
        app_handle,
        BrokeredCaptureRequest::ShowText {
            opaque_id: request.opaque_id,
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pi_broker_identity_matches_existing_pi_client_label() {
        let identity = pi_broker_identity().expect("PI identity should be valid");

        assert_eq!(identity.label, "PI");
        assert_eq!(identity.normalized_label, "pi");
        assert_eq!(identity.source, BrokerClientIdentitySource::Inferred);
    }
}
