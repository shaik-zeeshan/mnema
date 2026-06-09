//! Tauri command surface for the AI "Reasoning Engine".
//!
//! The actual engine — provider clients, structured extraction, reachability —
//! lives in the `ai-runtime` crate, aliased here as `ai_engine` so it does not
//! collide with this `crate::ai_runtime` module. These commands map the wire
//! [`AiRuntimeSettings`] onto an [`ai_engine::EngineConfig`], read the
//! bring-your-own provider key from the OS keychain (never from settings), and
//! expose status/test round trips to the Settings → Access "Reasoning Engine"
//! card.

use capture_types::{AiCloudProvider, AiEngineKind, AiLocalKind, AiRuntimeSettings};
use serde::{Deserialize, Serialize};

use crate::native_capture::{read_recording_settings, RecordingSettingsState};

/// Availability snapshot for the configured engine, mirroring Ask AI's
/// "Ok with available:false + reason" shape for the normal not-ready cases.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRuntimeStatus {
    enabled: bool,
    engine_kind: String,
    configured: bool,
    available: bool,
    has_cloud_key: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

/// Result of a single structured-extraction round trip against the engine.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRuntimeTestResult {
    ok: bool,
    engine_kind: String,
    model: String,
    message: String,
    raw_json: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRuntimeProviderKeyRequest {
    provider: String,
    key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRuntimeProviderRequest {
    provider: String,
}

/// Wire string for an [`AiEngineKind`] (matches the serde snake_case form).
fn engine_kind_str(kind: AiEngineKind) -> &'static str {
    match kind {
        AiEngineKind::Cloud => "cloud",
        AiEngineKind::Local => "local",
    }
}

/// Keychain provider id ("anthropic" / "openai") for the configured cloud provider.
fn cloud_provider_id(provider: AiCloudProvider) -> &'static str {
    match provider {
        AiCloudProvider::Anthropic => "anthropic",
        AiCloudProvider::Openai => "openai",
        AiCloudProvider::OpenaiCompatible => "openai_compatible",
    }
}

/// Map the wire cloud provider onto the engine crate's provider enum.
fn cloud_provider_kind(provider: AiCloudProvider) -> ai_engine::CloudProvider {
    match provider {
        AiCloudProvider::Anthropic => ai_engine::CloudProvider::Anthropic,
        AiCloudProvider::Openai => ai_engine::CloudProvider::Openai,
        AiCloudProvider::OpenaiCompatible => ai_engine::CloudProvider::OpenAiCompatible,
    }
}

/// Map the wire local kind onto the engine crate's local-kind enum.
fn local_kind(kind: AiLocalKind) -> ai_engine::LocalKind {
    match kind {
        AiLocalKind::Ollama => ai_engine::LocalKind::Ollama,
        AiLocalKind::Llamafile => ai_engine::LocalKind::Llamafile,
    }
}

/// Build an [`ai_engine::EngineConfig`] from the current settings, sourcing the
/// cloud credential from the keychain. Returns a human-readable reason string on
/// failure (no model, no key, no endpoint).
pub(crate) fn resolve_engine_config(
    settings: &AiRuntimeSettings,
) -> Result<ai_engine::EngineConfig, String> {
    match settings.engine_kind {
        AiEngineKind::Cloud => {
            let model = settings.cloud_model.trim();
            if model.is_empty() {
                return Err("no_model".to_string());
            }
            let base_url = settings.cloud_base_url.trim();
            if matches!(settings.cloud_provider, AiCloudProvider::OpenaiCompatible)
                && base_url.is_empty()
            {
                return Err("no_base_url".to_string());
            }
            let provider_id = cloud_provider_id(settings.cloud_provider);
            let api_key = app_infra::load_ai_provider_key(provider_id)
                .map_err(|error| error.to_string())?
                .filter(|key| !key.is_empty())
                .ok_or_else(|| "no_cloud_key".to_string())?;
            Ok(ai_engine::EngineConfig::Cloud {
                provider: cloud_provider_kind(settings.cloud_provider),
                model: model.to_string(),
                api_key,
                base_url: if base_url.is_empty() {
                    None
                } else {
                    Some(base_url.to_string())
                },
            })
        }
        AiEngineKind::Local => {
            let endpoint = settings.local_endpoint.trim();
            if endpoint.is_empty() {
                return Err("local_endpoint_unreachable".to_string());
            }
            let model = settings.local_model.trim();
            if model.is_empty() {
                return Err("local_no_model".to_string());
            }
            Ok(ai_engine::EngineConfig::Local {
                kind: local_kind(settings.local_kind),
                endpoint: endpoint.to_string(),
                model: model.to_string(),
            })
        }
    }
}

#[tauri::command]
pub fn ai_runtime_set_provider_key(request: AiRuntimeProviderKeyRequest) -> Result<(), String> {
    let provider = request.provider.trim();
    if provider.is_empty() {
        return Err("a provider id is required".to_string());
    }
    let key = request.key.trim();
    if key.is_empty() {
        return Err("an API key is required".to_string());
    }
    app_infra::store_ai_provider_key(provider, key).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn ai_runtime_clear_provider_key(request: AiRuntimeProviderRequest) -> Result<(), String> {
    let provider = request.provider.trim();
    if provider.is_empty() {
        return Err("a provider id is required".to_string());
    }
    app_infra::delete_ai_provider_key(provider).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn ai_runtime_has_provider_key(request: AiRuntimeProviderRequest) -> Result<bool, String> {
    let provider = request.provider.trim();
    if provider.is_empty() {
        return Err("a provider id is required".to_string());
    }
    app_infra::has_ai_provider_key(provider).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_ai_runtime_status(
    state: tauri::State<'_, RecordingSettingsState>,
) -> Result<AiRuntimeStatus, String> {
    let settings = read_recording_settings(state.inner()).ai_runtime;
    let engine_kind = engine_kind_str(settings.engine_kind).to_string();

    let has_cloud_key = app_infra::has_ai_provider_key(cloud_provider_id(settings.cloud_provider))
        .map_err(|error| error.to_string())?;

    if !settings.enabled {
        return Ok(AiRuntimeStatus {
            enabled: false,
            engine_kind,
            configured: false,
            available: false,
            has_cloud_key,
            reason: Some("ai_runtime_disabled".to_string()),
        });
    }

    match settings.engine_kind {
        AiEngineKind::Cloud => {
            if settings.cloud_model.trim().is_empty() {
                return Ok(AiRuntimeStatus {
                    enabled: true,
                    engine_kind,
                    configured: false,
                    available: false,
                    has_cloud_key,
                    reason: Some("no_model".to_string()),
                });
            }
            if matches!(settings.cloud_provider, AiCloudProvider::OpenaiCompatible)
                && settings.cloud_base_url.trim().is_empty()
            {
                return Ok(AiRuntimeStatus {
                    enabled: true,
                    engine_kind,
                    configured: false,
                    available: false,
                    has_cloud_key,
                    reason: Some("no_base_url".to_string()),
                });
            }
            if !has_cloud_key {
                return Ok(AiRuntimeStatus {
                    enabled: true,
                    engine_kind,
                    configured: false,
                    available: false,
                    has_cloud_key,
                    reason: Some("no_cloud_key".to_string()),
                });
            }
            Ok(AiRuntimeStatus {
                enabled: true,
                engine_kind,
                configured: true,
                available: true,
                has_cloud_key,
                reason: None,
            })
        }
        AiEngineKind::Local => {
            if settings.local_model.trim().is_empty() {
                return Ok(AiRuntimeStatus {
                    enabled: true,
                    engine_kind,
                    configured: false,
                    available: false,
                    has_cloud_key,
                    reason: Some("local_no_model".to_string()),
                });
            }
            let reachable = ai_engine::ping_endpoint(&settings.local_endpoint).await;
            if !reachable {
                return Ok(AiRuntimeStatus {
                    enabled: true,
                    engine_kind,
                    configured: true,
                    available: false,
                    has_cloud_key,
                    reason: Some("local_endpoint_unreachable".to_string()),
                });
            }
            Ok(AiRuntimeStatus {
                enabled: true,
                engine_kind,
                configured: true,
                available: true,
                has_cloud_key,
                reason: None,
            })
        }
    }
}

#[tauri::command]
pub async fn ai_runtime_test_connection(
    state: tauri::State<'_, RecordingSettingsState>,
) -> Result<AiRuntimeTestResult, String> {
    let settings = read_recording_settings(state.inner()).ai_runtime;
    let engine_kind = engine_kind_str(settings.engine_kind).to_string();

    let model = match settings.engine_kind {
        AiEngineKind::Cloud => settings.cloud_model.trim().to_string(),
        AiEngineKind::Local => settings.local_model.trim().to_string(),
    };

    let config = resolve_engine_config(&settings)?;

    let probe = ai_engine::run_connection_probe(&config)
        .await
        .map_err(|error| error.to_string())?;

    let raw_json = serde_json::to_string(&probe).map_err(|error| error.to_string())?;
    let message = probe
        .message
        .clone()
        .filter(|message| !message.trim().is_empty())
        .unwrap_or_else(|| "Structured output is working.".to_string());

    Ok(AiRuntimeTestResult {
        ok: probe.ok.unwrap_or(true),
        engine_kind,
        model,
        message,
        raw_json,
    })
}
