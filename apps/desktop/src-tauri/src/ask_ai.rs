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

fn validate_ask_ai_access_ready(
    ask_ai_enabled: bool,
    status: &crate::app_infra::PiRuntimeStatus,
) -> Result<(), String> {
    if !ask_ai_enabled {
        return Err("Ask AI access is disabled in settings".to_string());
    }
    if !status.ready {
        let reason = status
            .reason
            .as_deref()
            .unwrap_or("pi_unavailable");
        return Err(format!("Ask AI requires a ready PI runtime ({reason})"));
    }

    Ok(())
}

async fn ensure_ask_ai_access_ready(app_handle: &tauri::AppHandle) -> Result<(), String> {
    let Some(settings_state) = app_handle.try_state::<crate::native_capture::RecordingSettingsState>() else {
        return Err("Ask AI settings are unavailable".to_string());
    };
    let ask_ai_enabled = settings_state
        .lock()
        .map_err(|_| "Ask AI settings are unavailable".to_string())?
        .settings
        .access
        .ask_ai_enabled;
    let status = crate::app_infra::get_pi_runtime_status_inner(app_handle.clone()).await?;
    validate_ask_ai_access_ready(ask_ai_enabled, &status)?;

    Ok(())
}

async fn execute_pi_broker_request(
    app_handle: tauri::AppHandle,
    request: BrokeredCaptureRequest,
) -> Result<BrokeredCaptureResponse, String> {
    ensure_ask_ai_access_ready(&app_handle).await?;
    broker_access(&app_handle)?
        .execute_for_ask_ai(pi_broker_identity()?, request)
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

    fn ready_pi_status() -> crate::app_infra::PiRuntimeStatus {
        crate::app_infra::PiRuntimeStatus {
            source: crate::app_infra::PiRuntimeSource::Path,
            executable_path: Some("/usr/local/bin/pi".to_string()),
            version: Some("0.65.0".to_string()),
            minimum_version: "0.65.0".to_string(),
            version_ok: true,
            auth_json_path: "/Users/tester/.pi/agent/auth.json".to_string(),
            auth_json_exists: true,
            provider_configured: true,
            provider_count: 1,
            ready: true,
            reason: None,
        }
    }

    #[test]
    fn pi_broker_identity_matches_existing_pi_client_label() {
        let identity = pi_broker_identity().expect("PI identity should be valid");

        assert_eq!(identity.label, "PI");
        assert_eq!(identity.normalized_label, "pi");
        assert_eq!(identity.source, BrokerClientIdentitySource::Inferred);
    }

    #[test]
    fn ask_ai_access_ready_rejects_disabled_setting() {
        let error = validate_ask_ai_access_ready(false, &ready_pi_status())
            .expect_err("disabled Ask AI should be rejected");

        assert_eq!(error, "Ask AI access is disabled in settings");
    }

    #[test]
    fn ask_ai_access_ready_rejects_unready_pi() {
        let mut status = ready_pi_status();
        status.ready = false;
        status.reason = Some("pi_auth_missing".to_string());

        let error = validate_ask_ai_access_ready(true, &status)
            .expect_err("unready PI should be rejected");

        assert_eq!(error, "Ask AI requires a ready PI runtime (pi_auth_missing)");
    }

    #[test]
    fn ask_ai_access_ready_accepts_enabled_setting_and_ready_pi() {
        validate_ask_ai_access_ready(true, &ready_pi_status())
            .expect("enabled Ask AI with ready PI should be accepted");
    }
}
