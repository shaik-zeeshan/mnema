//! Tauri command surface for the AI "Reasoning Engine".
//!
//! The actual engine — provider clients, structured extraction, reachability —
//! lives in the `ai-runtime` crate, aliased here as `ai_engine` so it does not
//! collide with this `crate::ai_runtime` module. These commands map the wire
//! [`AiRuntimeSettings`] onto an [`ai_engine::EngineConfig`], read the
//! bring-your-own provider key from the OS keychain (never from settings), and
//! expose status/test round trips to the Settings → Access "Reasoning Engine"
//! card.

use capture_types::{
    AiCloudProvider, AiEngineKind, AiEngineProfile, AiLocalKind, AiRuntimeSettings,
};
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

// NOTE: the per-thread engine-pin selection seam below (`*_profile*`,
// `select_profile_for_pin`, `resolve_engine_config_for_pin`) is the settings
// contract this slice provides for the later conversation-pin / ask_ai.rs slices
// to consume. It is exercised by this module's tests but has no production
// caller yet, so it is `#[allow(dead_code)]` until those slices wire it.

/// The per-thread **engine-pin provider string** for a local kind
/// ("ollama" / "llamafile"). Cloud engines use [`cloud_provider_id`]; together
/// these are the stable provider strings the conversation-pin slice + frontend
/// pass to [`resolve_engine_config_for_pin`].
#[allow(dead_code)]
fn local_kind_id(kind: AiLocalKind) -> &'static str {
    match kind {
        AiLocalKind::Ollama => "ollama",
        AiLocalKind::Llamafile => "llamafile",
    }
}

/// The per-thread engine-pin provider string for one profile: the keychain
/// provider id for a cloud engine, the local-kind name for a local engine.
#[allow(dead_code)]
fn profile_provider_id(profile: &AiEngineProfile) -> &'static str {
    match profile.engine_kind {
        AiEngineKind::Cloud => cloud_provider_id(profile.cloud_provider),
        AiEngineKind::Local => local_kind_id(profile.local_kind),
    }
}

/// The model string a profile resolves to for its engine kind (cloud model /
/// local model), trimmed. Used for the pin (provider, model) match key.
#[allow(dead_code)]
fn profile_model(profile: &AiEngineProfile) -> &str {
    match profile.engine_kind {
        AiEngineKind::Cloud => profile.cloud_model.trim(),
        AiEngineKind::Local => profile.local_model.trim(),
    }
}

/// The [`AiEngineProfile`] described by the flat default/global fields of the
/// settings (the default engine).
fn default_engine_profile(settings: &AiRuntimeSettings) -> AiEngineProfile {
    AiEngineProfile {
        engine_kind: settings.engine_kind,
        cloud_provider: settings.cloud_provider,
        cloud_model: settings.cloud_model.clone(),
        cloud_base_url: settings.cloud_base_url.clone(),
        local_kind: settings.local_kind,
        local_endpoint: settings.local_endpoint.clone(),
        local_model: settings.local_model.clone(),
    }
}

/// Every engine the user has configured: the default/global engine (from the
/// flat fields) first, then the `additional_engines` catalog, de-duplicated by
/// the pin identity `(engine_kind, provider/kind id, model)` — so the default
/// engine appearing again in the additional list is not listed twice.
#[allow(dead_code)]
pub(crate) fn configured_engine_profiles(settings: &AiRuntimeSettings) -> Vec<AiEngineProfile> {
    let mut profiles = Vec::with_capacity(1 + settings.additional_engines.len());
    let mut seen: Vec<(AiEngineKind, &'static str, String)> = Vec::new();
    for profile in std::iter::once(default_engine_profile(settings))
        .chain(settings.additional_engines.iter().cloned())
    {
        let key = (
            profile.engine_kind,
            profile_provider_id(&profile),
            profile_model(&profile).to_string(),
        );
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        profiles.push(profile);
    }
    profiles
}

/// Find the profile a thread is pinned to, matching by the engine-pin
/// `provider` string ([`cloud_provider_id`] for cloud, `"ollama"`/`"llamafile"`
/// for local) AND the model id. `None` when either is absent or no profile
/// matches — the caller then falls back to the default engine. Pure (no
/// keychain), so it is unit-testable.
#[allow(dead_code)]
pub(crate) fn select_profile_for_pin<'a>(
    profiles: &'a [AiEngineProfile],
    provider: Option<&str>,
    model: Option<&str>,
) -> Option<&'a AiEngineProfile> {
    let provider = provider?.trim();
    let model = model?.trim();
    if provider.is_empty() || model.is_empty() {
        return None;
    }
    profiles
        .iter()
        .find(|profile| profile_provider_id(profile) == provider && profile_model(profile) == model)
}

/// Build an [`ai_engine::EngineConfig`] from one [`AiEngineProfile`], sourcing
/// the cloud credential from the keychain. Returns a reason-code string on
/// failure (`no_model` / `no_base_url` / `no_cloud_key` / `local_no_model` /
/// `local_endpoint_unreachable`). This is the shared engine-building body used
/// by both the default-engine [`resolve_engine_config`] and the pinned-engine
/// [`resolve_engine_config_for_pin`].
pub(crate) fn resolve_engine_config_from_profile(
    profile: &AiEngineProfile,
) -> Result<ai_engine::EngineConfig, String> {
    match profile.engine_kind {
        AiEngineKind::Cloud => {
            let model = profile.cloud_model.trim();
            if model.is_empty() {
                return Err("no_model".to_string());
            }
            let base_url = profile.cloud_base_url.trim();
            if matches!(profile.cloud_provider, AiCloudProvider::OpenaiCompatible)
                && base_url.is_empty()
            {
                return Err("no_base_url".to_string());
            }
            let provider_id = cloud_provider_id(profile.cloud_provider);
            let api_key = app_infra::load_ai_provider_key(provider_id)
                .map_err(|error| error.to_string())?
                .filter(|key| !key.is_empty())
                .ok_or_else(|| "no_cloud_key".to_string())?;
            Ok(ai_engine::EngineConfig::Cloud {
                provider: cloud_provider_kind(profile.cloud_provider),
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
            let endpoint = profile.local_endpoint.trim();
            if endpoint.is_empty() {
                return Err("local_endpoint_unreachable".to_string());
            }
            let model = profile.local_model.trim();
            if model.is_empty() {
                return Err("local_no_model".to_string());
            }
            Ok(ai_engine::EngineConfig::Local {
                kind: local_kind(profile.local_kind),
                endpoint: endpoint.to_string(),
                model: model.to_string(),
            })
        }
    }
}

/// Build an [`ai_engine::EngineConfig`] from the current settings' **default**
/// engine, sourcing the cloud credential from the keychain. Returns a
/// reason-code string on failure (no model, no key, no endpoint). Behaviour is
/// identical to before the profile refactor — it just delegates to
/// [`resolve_engine_config_from_profile`] with the default profile.
pub(crate) fn resolve_engine_config(
    settings: &AiRuntimeSettings,
) -> Result<ai_engine::EngineConfig, String> {
    resolve_engine_config_from_profile(&default_engine_profile(settings))
}

/// Resolve the engine for a Quick Recall / chat thread, honouring an optional
/// per-thread **engine pin** `(provider, model)`. The pin `provider` is the
/// engine-pin provider string: a keychain cloud provider id
/// (`"anthropic"`/`"openai"`/`"openai_compatible"`) or a local kind name
/// (`"ollama"`/`"llamafile"`); `model` is the rig-core model id. When both are
/// present and match one of [`configured_engine_profiles`], that profile's
/// config is built. Otherwise (no pin, or no match) it falls back to the
/// default engine via [`resolve_engine_config`]. The same provider-string
/// convention is shared with the conversation-pin slice and the frontend.
#[allow(dead_code)]
pub(crate) fn resolve_engine_config_for_pin(
    settings: &AiRuntimeSettings,
    provider: Option<&str>,
    model: Option<&str>,
) -> Result<ai_engine::EngineConfig, String> {
    let profiles = configured_engine_profiles(settings);
    match select_profile_for_pin(&profiles, provider, model) {
        Some(profile) => resolve_engine_config_from_profile(profile),
        None => resolve_engine_config(settings),
    }
}

/// The shared engine-configured prerequisite beneath BOTH feature opt-ins
/// (interactive Ask AI and continuous User-Context derivation). `Ok(())` means a
/// usable engine exists; `Err(reason_code)` is one of the existing reason codes
/// (`"ai_runtime_disabled"`, `"no_model"`, `"no_base_url"`, `"no_cloud_key"`,
/// `"local_no_model"`, `"local_endpoint_unreachable"`). For local it does the
/// same ping reachability check `get_ai_runtime_status` does. The old "Reasoning
/// Engine master toggle" `AiRuntimeSettings.enabled` IS this prerequisite's
/// enable bit (`enabled: false → "ai_runtime_disabled"`).
pub(crate) async fn engine_configured_prerequisite(
    settings: &AiRuntimeSettings,
) -> Result<(), String> {
    if !settings.enabled {
        return Err("ai_runtime_disabled".to_string());
    }
    // Static config check (no model / no base url / no key / no endpoint).
    resolve_engine_config(settings)?;
    // Local engines additionally need their endpoint to be reachable right now,
    // matching the availability semantics of `get_ai_runtime_status`.
    if matches!(settings.engine_kind, AiEngineKind::Local)
        && !ai_engine::ping_endpoint(&settings.local_endpoint).await
    {
        return Err("local_endpoint_unreachable".to_string());
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
    let engine_kind = engine_kind_str(settings.engine_kind).to_string();

    let has_cloud_key = app_infra::has_ai_provider_key(cloud_provider_id(settings.cloud_provider))
        .map_err(|error| error.to_string())?;

    // Availability + reason flow through the single shared prerequisite so this
    // status surface and the feature gates can never drift on what "ready"
    // means. `configured` is "the static config is complete" (everything but a
    // local engine that is merely unreachable); `available` is the full
    // prerequisite (including the local reachability ping).
    match engine_configured_prerequisite(&settings).await {
        Ok(()) => Ok(AiRuntimeStatus {
            enabled: settings.enabled,
            engine_kind,
            configured: true,
            available: true,
            has_cloud_key,
            reason: None,
        }),
        Err(reason) => {
            // A local engine that is fully configured but currently unreachable
            // is `configured: true` (static config is complete) yet not
            // available; every other failure means the config is incomplete.
            let configured = reason == "local_endpoint_unreachable"
                && matches!(settings.engine_kind, AiEngineKind::Local)
                && !settings.local_model.trim().is_empty();
            Ok(AiRuntimeStatus {
                enabled: settings.enabled,
                engine_kind,
                configured,
                available: false,
                has_cloud_key,
                reason: Some(reason),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn cloud_profile(provider: AiCloudProvider, model: &str) -> AiEngineProfile {
        AiEngineProfile {
            engine_kind: AiEngineKind::Cloud,
            cloud_provider: provider,
            cloud_model: model.to_string(),
            cloud_base_url: String::new(),
            local_kind: AiLocalKind::Ollama,
            local_endpoint: "http://localhost:11434".to_string(),
            local_model: String::new(),
        }
    }

    fn local_profile(kind: AiLocalKind, model: &str) -> AiEngineProfile {
        AiEngineProfile {
            engine_kind: AiEngineKind::Local,
            cloud_provider: AiCloudProvider::Anthropic,
            cloud_model: String::new(),
            cloud_base_url: String::new(),
            local_kind: kind,
            local_endpoint: "http://localhost:11434".to_string(),
            local_model: model.to_string(),
        }
    }

    #[test]
    fn select_profile_for_pin_matches_cloud_by_provider_and_model() {
        let profiles = vec![
            cloud_profile(AiCloudProvider::Anthropic, "claude-haiku-4-5"),
            cloud_profile(AiCloudProvider::Openai, "gpt-4o-mini"),
        ];
        let matched = select_profile_for_pin(&profiles, Some("openai"), Some("gpt-4o-mini"))
            .expect("a matching profile");
        assert_eq!(matched.cloud_model, "gpt-4o-mini");
        assert!(matches!(matched.cloud_provider, AiCloudProvider::Openai));
    }

    #[test]
    fn select_profile_for_pin_matches_local_by_kind_and_model() {
        let profiles = vec![local_profile(AiLocalKind::Ollama, "llama3.2")];
        let matched = select_profile_for_pin(&profiles, Some("ollama"), Some("llama3.2"))
            .expect("a matching local profile");
        assert_eq!(matched.local_model, "llama3.2");
    }

    #[test]
    fn select_profile_for_pin_falls_back_when_no_pin_or_no_match() {
        let profiles = vec![cloud_profile(AiCloudProvider::Anthropic, "claude-haiku-4-5")];
        // No pin at all → None (caller falls back to the default engine).
        assert!(select_profile_for_pin(&profiles, None, None).is_none());
        // Provider matches but model does not → None.
        assert!(select_profile_for_pin(&profiles, Some("anthropic"), Some("other")).is_none());
        // Model matches but provider does not → None.
        assert!(
            select_profile_for_pin(&profiles, Some("openai"), Some("claude-haiku-4-5")).is_none()
        );
        // Blank pin strings are treated as no pin.
        assert!(select_profile_for_pin(&profiles, Some(""), Some("claude-haiku-4-5")).is_none());
    }

    #[test]
    fn configured_engine_profiles_lists_default_first_and_dedupes() {
        let settings = AiRuntimeSettings {
            engine_kind: AiEngineKind::Cloud,
            cloud_provider: AiCloudProvider::Anthropic,
            cloud_model: "claude-haiku-4-5".to_string(),
            // The default engine is repeated in the additional list (same
            // provider+model) and should be de-duplicated away.
            additional_engines: vec![
                cloud_profile(AiCloudProvider::Anthropic, "claude-haiku-4-5"),
                cloud_profile(AiCloudProvider::Openai, "gpt-4o-mini"),
            ],
            ..AiRuntimeSettings::default()
        };
        let profiles = configured_engine_profiles(&settings);
        assert_eq!(profiles.len(), 2);
        // Default engine is first.
        assert_eq!(profiles[0].cloud_model, "claude-haiku-4-5");
        assert!(matches!(
            profiles[0].cloud_provider,
            AiCloudProvider::Anthropic
        ));
        assert_eq!(profiles[1].cloud_model, "gpt-4o-mini");
    }
}
