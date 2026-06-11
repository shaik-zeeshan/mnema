//! Tauri command surface for the AI "Reasoning Engine".
//!
//! The actual engine — provider clients, structured extraction, reachability —
//! lives in the `ai-runtime` crate, aliased here as `ai_engine` so it does not
//! collide with this `crate::ai_runtime` module. These commands map the
//! provider-centric wire [`AiRuntimeSettings`] (ADR 0034: a flat provider list
//! plus ONE global default model) onto an [`ai_engine::EngineConfig`], read the
//! bring-your-own provider key from the OS keychain (never from settings), and
//! expose status/test round trips plus provider-tagged model discovery to the
//! Settings AI surface.

use capture_types::{AiEngineRef, AiProviderConfig, AiProviderKind, AiRuntimeSettings};
use serde::{Deserialize, Serialize};

use crate::native_capture::{read_recording_settings, RecordingSettingsState};

/// Availability snapshot for the configured engine, mirroring Ask AI's
/// "Ok with available:false + reason" shape for the normal not-ready cases.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRuntimeStatus {
    enabled: bool,
    configured: bool,
    available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_model: Option<AiEngineRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

/// Result of a single structured-extraction round trip against the engine.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRuntimeTestResult {
    ok: bool,
    provider: String,
    model: String,
    message: String,
    raw_json: String,
}

/// One model id discovered from a connected provider's models route, tagged
/// with the provider it came from so the merged pool stays attributable.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRuntimeModel {
    id: String,
    /// Stable provider id ([`AiProviderKind::id`]).
    provider: String,
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

/// Map a cloud provider kind onto the engine crate's provider enum. `None`
/// for the local kinds.
fn cloud_provider_kind(kind: AiProviderKind) -> Option<ai_engine::CloudProvider> {
    match kind {
        AiProviderKind::Anthropic => Some(ai_engine::CloudProvider::Anthropic),
        AiProviderKind::Openai => Some(ai_engine::CloudProvider::Openai),
        AiProviderKind::OpenaiCompatible => Some(ai_engine::CloudProvider::OpenAiCompatible),
        AiProviderKind::Ollama | AiProviderKind::Llamafile => None,
    }
}

/// Map a local provider kind onto the engine crate's local-kind enum. `None`
/// for the cloud kinds.
fn local_kind(kind: AiProviderKind) -> Option<ai_engine::LocalKind> {
    match kind {
        AiProviderKind::Ollama => Some(ai_engine::LocalKind::Ollama),
        AiProviderKind::Llamafile => Some(ai_engine::LocalKind::Llamafile),
        _ => None,
    }
}

/// The connected provider config for a kind, if the user connected one.
fn provider_config(settings: &AiRuntimeSettings, kind: AiProviderKind) -> Option<&AiProviderConfig> {
    settings
        .providers
        .iter()
        .find(|provider| provider.kind == kind)
}

/// The effective endpoint for a local provider config: its `baseUrl`, falling
/// back to the kind's default localhost endpoint when left empty.
fn local_endpoint(provider: &AiProviderConfig) -> String {
    let base_url = provider.base_url.trim();
    if base_url.is_empty() {
        provider
            .kind
            .default_local_endpoint()
            .unwrap_or_default()
            .to_string()
    } else {
        base_url.to_string()
    }
}

/// Build an [`ai_engine::EngineConfig`] for one engine identity
/// `{provider, model}` against the connected provider list, sourcing the cloud
/// credential from the keychain. Returns a reason-code string on failure:
/// `provider_not_connected:<id>`, `no_base_url`, or `no_provider_key:<id>`.
fn engine_config_for_ref(
    settings: &AiRuntimeSettings,
    kind: AiProviderKind,
    model: &str,
) -> Result<ai_engine::EngineConfig, String> {
    let Some(provider) = provider_config(settings, kind) else {
        return Err(format!("provider_not_connected:{}", kind.id()));
    };
    if let Some(local) = local_kind(kind) {
        return Ok(ai_engine::EngineConfig::Local {
            kind: local,
            endpoint: local_endpoint(provider),
            model: model.to_string(),
        });
    }
    let base_url = provider.base_url.trim();
    if matches!(kind, AiProviderKind::OpenaiCompatible) && base_url.is_empty() {
        return Err("no_base_url".to_string());
    }
    let api_key = app_infra::load_ai_provider_key(kind.id())
        .map_err(|error| error.to_string())?
        .filter(|key| !key.is_empty())
        .ok_or_else(|| format!("no_provider_key:{}", kind.id()))?;
    Ok(ai_engine::EngineConfig::Cloud {
        provider: cloud_provider_kind(kind).expect("cloud kind"),
        model: model.to_string(),
        api_key,
        base_url: if base_url.is_empty() {
            None
        } else {
            Some(base_url.to_string())
        },
    })
}

/// THE single model resolver (ADR 0034). Every model decision resolves through
/// one precedence chain: **thread pin → feature override → global default
/// model**.
///
/// - `pin` is a thread's persisted engine pin `(provider, model)` — the
///   provider is a stable id string ([`AiProviderKind::id`]), the model a
///   rig-core model id. A pin whose provider is unknown or no longer connected
///   falls through to the next layer instead of failing the thread.
/// - `feature_override_model` is a feature's optional bare model-id override
///   (e.g. `access.askAiModel` for Ask AI); it rides on the global default
///   model's provider.
/// - The global default model anchors the chain; without one the resolution
///   fails with `no_default_model`.
///
/// Returns a reason-code string on failure (`no_default_model`,
/// `provider_not_connected:<id>`, `no_base_url`, `no_provider_key:<id>`).
pub(crate) fn resolve_engine_config(
    settings: &AiRuntimeSettings,
    pin: Option<(&str, &str)>,
    feature_override_model: Option<&str>,
) -> Result<ai_engine::EngineConfig, String> {
    // 1. Thread pin.
    if let Some((provider, model)) = pin {
        let provider = provider.trim();
        let model = model.trim();
        if !provider.is_empty() && !model.is_empty() {
            if let Some(kind) = AiProviderKind::from_id(provider) {
                if provider_config(settings, kind).is_some() {
                    return engine_config_for_ref(settings, kind, model);
                }
            }
            // Pinned to a provider that is unknown or no longer connected →
            // fall back to the override/default layers below.
        }
    }

    // 2./3. The global default model, with the feature override replacing only
    // the model id when present.
    let default_model = settings
        .default_model
        .as_ref()
        .filter(|model| !model.model.trim().is_empty())
        .ok_or_else(|| "no_default_model".to_string())?;
    let model = feature_override_model
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .unwrap_or_else(|| default_model.model.trim());
    engine_config_for_ref(settings, default_model.provider, model)
}

/// The static (no-network) half of the engine-configured prerequisite: master
/// switch on, at least one connected provider, a default model chosen, and the
/// default model's engine resolvable (key present / base URL set). Returns the
/// resolved default engine config so the async half can ping a local endpoint.
pub(crate) fn engine_prerequisite_static(
    settings: &AiRuntimeSettings,
) -> Result<ai_engine::EngineConfig, String> {
    if !settings.enabled {
        return Err("ai_runtime_disabled".to_string());
    }
    if settings.providers.is_empty() {
        return Err("no_providers".to_string());
    }
    resolve_engine_config(settings, None, None)
}

/// The shared engine-configured prerequisite beneath BOTH feature opt-ins
/// (interactive Ask AI and continuous User-Context derivation). `Ok(())` means
/// a usable engine exists; `Err(reason_code)` is one of `"ai_runtime_disabled"`,
/// `"no_providers"`, `"no_default_model"`, `"provider_not_connected:<id>"`,
/// `"no_base_url"`, `"no_provider_key:<id>"`, `"local_endpoint_unreachable"`.
/// Per ADR 0034 this is "master switch on + at least one usable provider + a
/// default model chosen"; a local default additionally needs its endpoint to be
/// reachable right now, matching `get_ai_runtime_status`.
pub(crate) async fn engine_configured_prerequisite(
    settings: &AiRuntimeSettings,
) -> Result<(), String> {
    let config = engine_prerequisite_static(settings)?;
    if let ai_engine::EngineConfig::Local { endpoint, .. } = &config {
        if !ai_engine::ping_endpoint(endpoint).await {
            return Err("local_endpoint_unreachable".to_string());
        }
    }
    Ok(())
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

    // Availability + reason flow through the single shared prerequisite so this
    // status surface and the feature gates can never drift on what "ready"
    // means. `configured` is "the static config is complete" (everything but a
    // local engine that is merely unreachable); `available` is the full
    // prerequisite (including the local reachability ping).
    match engine_configured_prerequisite(&settings).await {
        Ok(()) => Ok(AiRuntimeStatus {
            enabled: settings.enabled,
            configured: true,
            available: true,
            default_model: settings.default_model,
            reason: None,
        }),
        Err(reason) => Ok(AiRuntimeStatus {
            enabled: settings.enabled,
            // Only the reachability ping fails AFTER the static config passed,
            // so that reason alone means "configured but currently offline".
            configured: reason == "local_endpoint_unreachable",
            available: false,
            default_model: settings.default_model,
            reason: Some(reason),
        }),
    }
}

#[tauri::command]
pub async fn ai_runtime_test_connection(
    state: tauri::State<'_, RecordingSettingsState>,
) -> Result<AiRuntimeTestResult, String> {
    let settings = read_recording_settings(state.inner()).ai_runtime;

    // Test the global default model's engine. Deliberately NOT gated on the
    // master switch so the Settings card can verify a key/endpoint before the
    // user turns AI features on.
    let default_model = settings
        .default_model
        .clone()
        .filter(|model| !model.model.trim().is_empty())
        .ok_or_else(|| "no_default_model".to_string())?;
    let config = resolve_engine_config(&settings, None, None)?;

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
        provider: default_model.provider.id().to_string(),
        model: default_model.model.trim().to_string(),
        message,
        raw_json,
    })
}

/// The live provider list the Settings card is currently editing.
///
/// `providers` is the in-progress draft, not the persisted settings: model
/// discovery is triggered while the user is still editing the card, so reading
/// the autosaved settings would race the debounce and list models for a stale
/// provider/base URL. When absent (e.g. the Chat picker), the persisted
/// provider list is used. The bring-your-own key still comes from the keychain
/// (by provider id), never over the wire.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRuntimeListModelsRequest {
    #[serde(default)]
    providers: Option<Vec<AiProviderConfig>>,
}

/// Pragmatic id-pattern filter that keeps only chat-capable models out of the
/// merged pool. This is deliberately a substring deny-list over the model id,
/// NOT a model-capability metadata pipeline (out of scope per the plan): some
/// providers advertise image-generation, embedding, TTS/whisper, rerank, and
/// moderation models alongside chat models, and we only want chat-capable ids
/// in the pickers. Free-form custom-model entry stays allowed elsewhere, so a
/// user can still type a hidden (filtered) model id by hand.
fn is_chat_capable_model(id: &str) -> bool {
    let lowered = id.to_lowercase();
    const NON_CHAT_SUBSTRINGS: &[&str] = &[
        "embed",
        "embedding",
        "flux",
        "stable-diffusion",
        "sdxl",
        "dall-e",
        "dalle",
        "whisper",
        "tts",
        "text-to-speech",
        "rerank",
        "reranker",
        "moderation",
        "guardrail",
        "clip-",
        "bge-",
        "-image",
        "image-",
        "schnell",
        "flux.1",
        "imagen",
        "stable-video",
        "kontext",
        "upscale",
        "inpaint",
    ];
    !NON_CHAT_SUBSTRINGS
        .iter()
        .any(|needle| lowered.contains(needle))
}

/// Fetch the model ids one connected provider advertises: `GET …/models` with
/// `Authorization: Bearer` for OpenAI / OpenAI-compatible, the `x-api-key` +
/// `anthropic-version` pair for Anthropic, and no credential for a local
/// Ollama/Llamafile endpoint (which serve the OpenAI-compatible `/v1/models`).
async fn list_models_for_provider(
    client: &reqwest::Client,
    provider: &AiProviderConfig,
) -> Result<Vec<String>, String> {
    let base_url = provider.base_url.trim();
    let http_request = match provider.kind {
        AiProviderKind::Anthropic | AiProviderKind::Openai | AiProviderKind::OpenaiCompatible => {
            let api_key = app_infra::load_ai_provider_key(provider.kind.id())
                .map_err(|error| error.to_string())?
                .filter(|key| !key.is_empty())
                .ok_or_else(|| format!("no_provider_key:{}", provider.kind.id()))?;
            match provider.kind {
                AiProviderKind::Anthropic => client
                    .get(models_endpoint_url("https://api.anthropic.com/v1"))
                    .header("x-api-key", api_key)
                    .header("anthropic-version", ANTHROPIC_VERSION_HEADER),
                AiProviderKind::Openai => client
                    .get(models_endpoint_url("https://api.openai.com/v1"))
                    .bearer_auth(api_key),
                _ => {
                    if base_url.is_empty() {
                        return Err("no_base_url".to_string());
                    }
                    client.get(models_endpoint_url(base_url)).bearer_auth(api_key)
                }
            }
        }
        AiProviderKind::Ollama | AiProviderKind::Llamafile => {
            client.get(models_endpoint_url(&local_endpoint(provider)))
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
        // message) so the warning log can explain why listing failed.
        let detail = body.trim();
        return Err(if detail.is_empty() {
            format!("model listing request failed with status {status}")
        } else {
            format!("model listing request failed with status {status}: {detail}")
        });
    }

    let parsed: ModelsListResponse =
        serde_json::from_str(&body).map_err(|error| error.to_string())?;

    Ok(parsed
        .data
        .into_iter()
        .map(|entry| entry.id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect())
}

/// List the merged, provider-tagged model pool across every connected provider
/// (ADR 0034: one pool feeding the default-model picker, the Ask AI override
/// picker, and the Chat thread picker). Best-effort per provider: a provider
/// that fails to list (missing key, unreachable endpoint) is skipped with a
/// warning instead of failing the whole pool, and the caller may still type a
/// model id no provider advertises (free-form entry stays allowed).
#[tauri::command]
pub async fn ai_runtime_list_models(
    state: tauri::State<'_, RecordingSettingsState>,
    request: Option<AiRuntimeListModelsRequest>,
) -> Result<Vec<AiRuntimeModel>, String> {
    let providers = request
        .and_then(|request| request.providers)
        .unwrap_or_else(|| read_recording_settings(state.inner()).ai_runtime.providers);

    let client = reqwest::Client::new();
    let mut models: Vec<AiRuntimeModel> = Vec::new();
    for provider in &providers {
        match list_models_for_provider(&client, provider).await {
            Ok(ids) => models.extend(
                ids.into_iter()
                    // Keep only chat-capable models in the pickers (image-gen,
                    // embeddings, TTS/whisper, rerank, moderation are filtered);
                    // free-form custom-model entry still lets a user pick a
                    // hidden model id by hand.
                    .filter(|id| is_chat_capable_model(id))
                    .map(|id| AiRuntimeModel {
                        id,
                        provider: provider.kind.id().to_string(),
                    }),
            ),
            Err(error) => {
                tauri_plugin_log::log::warn!(
                    "model listing skipped provider {}: {error}",
                    provider.kind.id()
                );
            }
        }
    }
    models.sort_by(|a, b| a.provider.cmp(&b.provider).then_with(|| a.id.cmp(&b.id)));
    models.dedup_by(|a, b| a.id == b.id && a.provider == b.provider);
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Settings with local-only providers so resolution never touches the OS
    /// keychain (cloud key loads are exercised by the app-infra store tests).
    fn local_settings() -> AiRuntimeSettings {
        AiRuntimeSettings {
            enabled: true,
            providers: vec![
                AiProviderConfig {
                    kind: AiProviderKind::Ollama,
                    base_url: String::new(),
                },
                AiProviderConfig {
                    kind: AiProviderKind::Llamafile,
                    base_url: "http://localhost:9090".to_string(),
                },
            ],
            default_model: Some(AiEngineRef {
                provider: AiProviderKind::Ollama,
                model: "llama-default".to_string(),
            }),
        }
    }

    fn expect_local(config: ai_engine::EngineConfig) -> (ai_engine::LocalKind, String, String) {
        match config {
            ai_engine::EngineConfig::Local {
                kind,
                endpoint,
                model,
            } => (kind, endpoint, model),
            other => panic!("expected Local, got {other:?}"),
        }
    }

    #[test]
    fn resolver_pin_beats_override_beats_default() {
        let settings = local_settings();
        // Pin wins over both the feature override and the default model.
        let (kind, endpoint, model) = expect_local(
            resolve_engine_config(
                &settings,
                Some(("llamafile", "pinned-model")),
                Some("override-model"),
            )
            .expect("pin should resolve"),
        );
        assert!(matches!(kind, ai_engine::LocalKind::Llamafile));
        assert_eq!(endpoint, "http://localhost:9090");
        assert_eq!(model, "pinned-model");

        // No pin: the override rides on the default model's provider.
        let (kind, endpoint, model) = expect_local(
            resolve_engine_config(&settings, None, Some("override-model"))
                .expect("override should resolve"),
        );
        assert!(matches!(kind, ai_engine::LocalKind::Ollama));
        assert_eq!(endpoint, "http://localhost:11434");
        assert_eq!(model, "override-model");

        // Neither: the global default model.
        let (_, _, model) = expect_local(
            resolve_engine_config(&settings, None, None).expect("default should resolve"),
        );
        assert_eq!(model, "llama-default");
    }

    #[test]
    fn resolver_falls_back_when_pin_provider_is_not_connected() {
        let mut settings = local_settings();
        settings.providers.retain(|p| p.kind == AiProviderKind::Ollama);
        // Pinned to a provider no longer in the list → fall back through the
        // chain (override layer first).
        let (kind, _, model) = expect_local(
            resolve_engine_config(
                &settings,
                Some(("llamafile", "pinned-model")),
                Some("override-model"),
            )
            .expect("fallback should resolve"),
        );
        assert!(matches!(kind, ai_engine::LocalKind::Ollama));
        assert_eq!(model, "override-model");

        // An unknown pin provider id behaves the same.
        let (_, _, model) = expect_local(
            resolve_engine_config(&settings, Some(("not-a-provider", "x")), None)
                .expect("fallback should resolve"),
        );
        assert_eq!(model, "llama-default");

        // Blank pin halves are treated as no pin.
        let (_, _, model) = expect_local(
            resolve_engine_config(&settings, Some(("", "pinned-model")), None)
                .expect("fallback should resolve"),
        );
        assert_eq!(model, "llama-default");
    }

    #[test]
    fn resolver_requires_a_default_model_without_pin() {
        let mut settings = local_settings();
        settings.default_model = None;
        // No default → the override has no provider to ride on, and the plain
        // resolution fails with the dedicated reason.
        assert_eq!(
            resolve_engine_config(&settings, None, Some("override-model")).map(|_| ()),
            Err("no_default_model".to_string())
        );
        assert_eq!(
            resolve_engine_config(&settings, None, None).map(|_| ()),
            Err("no_default_model".to_string())
        );
        // A pin still resolves on its own (it names its provider explicitly).
        let (_, _, model) = expect_local(
            resolve_engine_config(&settings, Some(("ollama", "pinned-model")), None)
                .expect("pin should resolve without a default"),
        );
        assert_eq!(model, "pinned-model");
    }

    #[test]
    fn resolver_reports_disconnected_default_provider() {
        let mut settings = local_settings();
        settings.default_model = Some(AiEngineRef {
            provider: AiProviderKind::Anthropic,
            model: "claude-haiku-4-5".to_string(),
        });
        assert_eq!(
            resolve_engine_config(&settings, None, None).map(|_| ()),
            Err("provider_not_connected:anthropic".to_string())
        );
    }

    #[test]
    fn resolver_requires_base_url_for_openai_compatible() {
        let settings = AiRuntimeSettings {
            enabled: true,
            providers: vec![AiProviderConfig {
                kind: AiProviderKind::OpenaiCompatible,
                base_url: String::new(),
            }],
            default_model: Some(AiEngineRef {
                provider: AiProviderKind::OpenaiCompatible,
                model: "some-model".to_string(),
            }),
        };
        // The base-URL check fires before any keychain access.
        assert_eq!(
            resolve_engine_config(&settings, None, None).map(|_| ()),
            Err("no_base_url".to_string())
        );
    }

    #[test]
    fn static_prerequisite_reason_codes() {
        // Master switch off.
        let mut settings = local_settings();
        settings.enabled = false;
        assert_eq!(
            engine_prerequisite_static(&settings).map(|_| ()),
            Err("ai_runtime_disabled".to_string())
        );

        // No providers connected.
        let mut settings = local_settings();
        settings.providers.clear();
        assert_eq!(
            engine_prerequisite_static(&settings).map(|_| ()),
            Err("no_providers".to_string())
        );

        // No default model chosen.
        let mut settings = local_settings();
        settings.default_model = None;
        assert_eq!(
            engine_prerequisite_static(&settings).map(|_| ()),
            Err("no_default_model".to_string())
        );

        // Fully configured local default passes the static half.
        assert!(engine_prerequisite_static(&local_settings()).is_ok());
    }

    #[test]
    fn is_chat_capable_model_filters_non_chat_ids() {
        // Non-chat ids (image-gen, embeddings, whisper) are excluded.
        assert!(!is_chat_capable_model("flux-1-schnell"));
        assert!(!is_chat_capable_model("text-embedding-3-large"));
        assert!(!is_chat_capable_model("whisper-1"));
        assert!(!is_chat_capable_model("dall-e-3"));

        // Chat models stay in the pool, including provider-namespaced ids.
        assert!(is_chat_capable_model("deepseek-v4-pro"));
        assert!(is_chat_capable_model("claude-haiku-4-5"));
        assert!(is_chat_capable_model("gpt-4o"));
        assert!(is_chat_capable_model(
            "accounts/fireworks/models/deepseek-v4-pro"
        ));
        assert!(is_chat_capable_model("llama-3.3-70b"));
    }
}
