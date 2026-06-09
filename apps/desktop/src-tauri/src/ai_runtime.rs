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

/// One model id discovered from the engine's OpenAI-style `/models` route.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRuntimeModel {
    id: String,
}

/// Minimal projection of the OpenAI-style `{ "data": [ { "id": … } ] }` model
/// list. Anthropic's `/v1/models` shares this `data[].id` shape, so the same
/// parse covers every provider/runtime we list.
#[derive(Debug, Deserialize)]
struct ModelsListResponse {
    #[serde(default)]
    data: Vec<ModelsListEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelsListEntry {
    id: String,
}

/// How long the `/models` request waits before giving up.
const MODELS_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// `anthropic-version` header required by Anthropic's REST API.
const ANTHROPIC_VERSION_HEADER: &str = "2023-06-01";

/// Build the `/models` URL for an OpenAI-style API base.
///
/// Bases that already include the `/v1` API prefix (the OpenAI-compatible and
/// first-party cloud bases, e.g. `https://api.fireworks.ai/inference/v1`) get
/// `/models` appended; bare hosts (a local Ollama/Llamafile endpoint like
/// `http://localhost:11434`) get the full `/v1/models` suffix.
fn models_endpoint_url(base: &str) -> String {
    let trimmed = base.trim().trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        format!("{trimmed}/models")
    } else {
        format!("{trimmed}/v1/models")
    }
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

/// The live engine selection the Settings combobox is currently editing.
///
/// This is the in-progress draft, not the persisted settings: model discovery is
/// triggered while the user is still editing the card, so reading the autosaved
/// settings would race the debounce and list models for a stale provider/base
/// URL. The bring-your-own key still comes from the keychain (by `cloudProvider`),
/// never over the wire.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRuntimeListModelsRequest {
    engine_kind: AiEngineKind,
    cloud_provider: AiCloudProvider,
    #[serde(default)]
    cloud_base_url: String,
    #[serde(default)]
    local_endpoint: String,
}

/// List the models the selected engine advertises via its OpenAI-style `/models`
/// route, so the Settings model field can be a discovery combobox rather than a
/// blind text input.
///
/// Takes the live draft engine selection (see [`AiRuntimeListModelsRequest`]),
/// sources the cloud credential from the keychain, and issues a single
/// `GET …/models`: `Authorization: Bearer` for OpenAI / OpenAI-compatible, the
/// `x-api-key` + `anthropic-version` pair for Anthropic, and no credential for a
/// local Ollama/Llamafile endpoint. Returns the sorted, de-duplicated model ids;
/// the caller may still type a model id that the route does not advertise.
#[tauri::command]
pub async fn ai_runtime_list_models(
    request: AiRuntimeListModelsRequest,
) -> Result<Vec<AiRuntimeModel>, String> {
    let client = reqwest::Client::new();
    let http_request = match request.engine_kind {
        AiEngineKind::Cloud => {
            let api_key = app_infra::load_ai_provider_key(cloud_provider_id(request.cloud_provider))
                .map_err(|error| error.to_string())?
                .filter(|key| !key.is_empty())
                .ok_or_else(|| "no_cloud_key".to_string())?;
            match request.cloud_provider {
                AiCloudProvider::Anthropic => client
                    .get(models_endpoint_url("https://api.anthropic.com/v1"))
                    .header("x-api-key", api_key)
                    .header("anthropic-version", ANTHROPIC_VERSION_HEADER),
                AiCloudProvider::Openai => client
                    .get(models_endpoint_url("https://api.openai.com/v1"))
                    .bearer_auth(api_key),
                AiCloudProvider::OpenaiCompatible => {
                    let base = request.cloud_base_url.trim();
                    if base.is_empty() {
                        return Err("no_base_url".to_string());
                    }
                    client.get(models_endpoint_url(base)).bearer_auth(api_key)
                }
            }
        }
        AiEngineKind::Local => {
            let endpoint = request.local_endpoint.trim();
            if endpoint.is_empty() {
                return Err("local_endpoint_unreachable".to_string());
            }
            client.get(models_endpoint_url(endpoint))
        }
    };

    let response = http_request
        .timeout(MODELS_REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|error| error.to_string())?;

    let status = response.status();
    // reqwest is built without the `json` feature here, so read the body and
    // deserialize with the already-present `serde_json`.
    let body = response.text().await.map_err(|error| error.to_string())?;

    if !status.is_success() {
        // Surface the provider's own error body (e.g. an "invalid_api_key"
        // message) so the Settings card can explain why listing failed.
        let detail = body.trim();
        return Err(if detail.is_empty() {
            format!("model listing request failed with status {status}")
        } else {
            format!("model listing request failed with status {status}: {detail}")
        });
    }

    let parsed: ModelsListResponse =
        serde_json::from_str(&body).map_err(|error| error.to_string())?;

    let mut models: Vec<AiRuntimeModel> = parsed
        .data
        .into_iter()
        .map(|entry| AiRuntimeModel {
            id: entry.id.trim().to_string(),
        })
        .filter(|model| !model.id.is_empty())
        .collect();
    models.sort_by(|a, b| a.id.cmp(&b.id));
    models.dedup_by(|a, b| a.id == b.id);
    Ok(models)
}
