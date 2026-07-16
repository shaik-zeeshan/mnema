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
    /// Provider-reported context-window size in tokens, when the listing route
    /// advertises one (many OpenAI-compatible vendors and the Fireworks catalog
    /// do; Anthropic/OpenAI don't expose it at all).
    context_window: Option<u64>,
}

/// One connected provider that failed to list its models, so the picker can
/// surface it (with a Retry) instead of silently showing a smaller pool.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRuntimeProviderFailure {
    /// The provider instance id that failed ([`AiProviderConfig::id`]).
    provider: String,
    /// A short, human-readable reason (`unreachable`, `missing API key`, …).
    reason: String,
}

/// The result of listing the merged model pool: the discovered models plus the
/// providers that failed to list. Best-effort listing means a failed provider
/// no longer just vanishes from the pool — it rides back here so the UI can
/// show it and offer a retry.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRuntimeModelsResult {
    models: Vec<AiRuntimeModel>,
    failures: Vec<AiRuntimeProviderFailure>,
}

/// Flatten an error and its whole `source()` chain into one string. `reqwest`'s
/// top-level `Display` is just "error sending request for url (…)"; the useful
/// cause ("tcp connect error: Connection refused (os error 61)", a DNS failure,
/// a TLS error) lives in the source chain, so we append each link.
fn describe_error(error: &dyn std::error::Error) -> String {
    let mut out = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        out.push_str(": ");
        out.push_str(&cause.to_string());
        source = cause.source();
    }
    out
}

/// Condense a raw listing error into a short reason for the picker. The raw
/// error still rides the warning log for debugging; this is the at-a-glance
/// label a user sees next to the provider.
fn classify_listing_failure(error: &str) -> String {
    if error.starts_with("no_provider_key") {
        "missing API key".to_string()
    } else if error.contains("keychain") && error.contains("denied") {
        // AppInfraError::SecretVaultDenied Display — a denied key store, NOT a
        // missing key (denied ≠ missing, ADR 0048 amendment).
        "keychain access denied".to_string()
    } else if error.starts_with("no_base_url") {
        "no base URL set".to_string()
    } else if error.starts_with("invalid_base_url") || error.starts_with("base_url_host_mismatch")
    {
        "invalid base URL".to_string()
    } else if error.contains("error sending request")
        || error.contains("connect")
        || error.contains("dns")
        || error.contains("timed out")
    {
        "unreachable".to_string()
    } else if error.contains("status 401")
        || error.contains("status 403")
        || error.contains("invalid_api_key")
    {
        "authentication failed".to_string()
    } else {
        "couldn't list models".to_string()
    }
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
    /// Context-window size, under the names OpenAI-compatible vendors use:
    /// `context_length` (OpenRouter, DeepSeek, …) or `max_model_len` (vLLM).
    #[serde(default)]
    context_length: Option<u64>,
    #[serde(default)]
    max_model_len: Option<u64>,
    /// llama.cpp-style servers (incl. llamafile) nest it as `meta.n_ctx_train`.
    #[serde(default)]
    meta: Option<ModelsListEntryMeta>,
}

#[derive(Debug, Deserialize)]
struct ModelsListEntryMeta {
    #[serde(default)]
    n_ctx_train: Option<u64>,
}

impl ModelsListEntry {
    fn context_window(&self) -> Option<u64> {
        self.context_length
            .or(self.max_model_len)
            .or(self.meta.as_ref().and_then(|meta| meta.n_ctx_train))
            .filter(|&tokens| tokens > 0)
    }
}

/// One discovered model: its id plus the provider-reported context window,
/// when the listing route advertised one.
#[derive(Debug)]
struct DiscoveredModel {
    id: String,
    context_window: Option<u64>,
}

/// One page of Fireworks' proprietary Gateway catalog
/// (`/v1/accounts/{account}/models`). Unlike the OpenAI-compatible
/// `/inference/v1/models` route — which advertises only a small curated set —
/// this lists the full catalog, paginated via `nextPageToken`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FireworksCatalogResponse {
    #[serde(default)]
    models: Vec<FireworksCatalogModel>,
    #[serde(default)]
    next_page_token: Option<String>,
}

/// One catalog entry. `name` is the fully-qualified id the inference API
/// expects (e.g. `accounts/fireworks/models/deepseek-v4-flash`).
/// `supportsServerless` marks the models actually callable without a dedicated
/// deployment — the only ones worth surfacing in the picker.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FireworksCatalogModel {
    #[serde(default)]
    name: String,
    #[serde(default)]
    supports_serverless: bool,
    /// The model's context-window size in tokens (`contextLength`).
    #[serde(default)]
    context_length: Option<u64>,
}

/// One page of `GET /v1/accounts` — the accounts the API key belongs to. Used
/// to discover the caller's own account so their fine-tunes get listed
/// alongside the public `fireworks` catalog.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FireworksAccountsResponse {
    #[serde(default)]
    accounts: Vec<FireworksAccount>,
    #[serde(default)]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FireworksAccount {
    /// Fully-qualified resource name, e.g. `accounts/shaikzeeshan999-yo15`.
    #[serde(default)]
    name: String,
}

/// How long the `/models` request waits before giving up.
const MODELS_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Hard ceiling on paginated `/models` (and `/accounts`) fetches. The per-request
/// timeout bounds a single page, but a server (or proxy) echoing the same
/// non-empty `nextPageToken` forever would loop indefinitely and hang the model
/// picker; this caps the page walk far above any realistic catalog size.
const MODELS_MAX_PAGES: usize = 50;

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

/// The lowercased host of an OpenAI-style base URL, with scheme, userinfo,
/// port, and path stripped. `https://api.fireworks.ai/inference/v1` →
/// `api.fireworks.ai`.
fn base_host(base: &str) -> String {
    let after_scheme = base.trim().split("://").nth(1).unwrap_or(base.trim());
    after_scheme
        .split('/')
        .next()
        .unwrap_or("")
        .rsplit('@')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// Whether an OpenAI-compatible base URL points at Fireworks, whose
/// `/inference/v1/models` route is curated down to a handful — see
/// [`list_fireworks_serverless_models`].
fn is_fireworks_host(base: &str) -> bool {
    base_host(base) == "api.fireworks.ai"
}

/// The fixed host a well-known first-party provider id must talk to. `None` for
/// ids that legitimately carry a caller-chosen host (the `openai_compatible`
/// custom-endpoint flow, local runtimes, or same-kind instances with derived
/// ids). Used to stop a request from pairing a first-party keychain key with an
/// arbitrary `base_url`.
fn fixed_host_for_provider_id(provider_id: &str) -> Option<&'static str> {
    match provider_id {
        "anthropic" => Some("api.anthropic.com"),
        "openai" => Some("api.openai.com"),
        _ => None,
    }
}

/// Reject a caller-supplied `base_url` that would send a key somewhere it must
/// not go. Two guards: (1) the scheme must be `http`/`https` (a `file:`,
/// `gopher:`, etc. URL never reaches a model endpoint); (2) a well-known
/// first-party provider id (`anthropic`/`openai`) may only point at its fixed
/// host, so a `{id:"anthropic", kind:OpenaiCompatible, baseUrl:"…attacker…"}`
/// request can't POST the Anthropic key to an arbitrary host. Custom
/// `openai_compatible` instances (whose ids are not the reserved first-party
/// ids) keep their caller-chosen host.
fn validate_provider_base_url(provider_id: &str, base_url: &str) -> Result<(), String> {
    let trimmed = base_url.trim();
    let parsed = url::Url::parse(trimmed).map_err(|_| "invalid_base_url".to_string())?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err("invalid_base_url_scheme".to_string());
    }
    if let Some(fixed_host) = fixed_host_for_provider_id(provider_id) {
        if base_host(trimmed) != fixed_host {
            return Err(format!("base_url_host_mismatch:{provider_id}"));
        }
    }
    Ok(())
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

/// Set/clear the single secret of an MCP tool connector, keyed by server id.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerSecretRequest {
    id: String,
    secret: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerRequest {
    id: String,
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

/// The connected provider config for an instance id, if the user connected one.
/// Identity is the per-instance [`AiProviderConfig::id`] (not the kind), so
/// multiple providers of the same kind resolve independently.
fn provider_config<'a>(
    settings: &'a AiRuntimeSettings,
    provider_id: &str,
) -> Option<&'a AiProviderConfig> {
    settings
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)
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
/// credential from the keychain. `provider_id` is the per-instance
/// [`AiProviderConfig::id`]; the kind (cloud vs local, which client) is read off
/// the looked-up instance, and the keychain key lives at that same instance id.
/// Returns a reason-code string on failure: `provider_not_connected:<id>`,
/// `no_base_url`, or `no_provider_key:<id>`.
fn engine_config_for_ref(
    settings: &AiRuntimeSettings,
    provider_id: &str,
    model: &str,
) -> Result<ai_engine::EngineConfig, String> {
    let Some(provider) = provider_config(settings, provider_id) else {
        return Err(format!("provider_not_connected:{provider_id}"));
    };
    let kind = provider.kind;
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
    // This is the egress path for real completions, so apply the same key→host
    // binding as `list_models_for_provider`: an OpenAI-compatible instance with a
    // caller-supplied `base_url` is the only cloud kind that POSTs the keychain
    // key to an arbitrary host (Anthropic/OpenAI hardcode their first-party host
    // in the engine crate). Validate before loading the key, so a
    // {id:"anthropic", kind:OpenaiCompatible, baseUrl:"…attacker…"} record can't
    // forward the real Anthropic key — plus the assembled dossier/captures — to a
    // foreign host. Custom non-first-party ids keep their own host (see
    // `validate_provider_base_url`).
    if matches!(kind, AiProviderKind::OpenaiCompatible) && !base_url.is_empty() {
        validate_provider_base_url(provider_id, base_url)?;
    }
    let api_key = app_infra::load_ai_provider_key(provider_id)
        .map_err(|error| error.to_string())?
        .filter(|key| !key.is_empty())
        .ok_or_else(|| format!("no_provider_key:{provider_id}"))?;
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
            // The pin's provider is an instance id ([`AiProviderConfig::id`]).
            // Resolve only when that instance is still connected; a pin to an
            // unknown or removed provider falls through to the override/default
            // layers below instead of failing the thread.
            if provider_config(settings, provider).is_some() {
                return engine_config_for_ref(settings, provider, model);
            }
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
    engine_config_for_ref(settings, default_model.provider.trim(), model)
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

// Secret access goes through the in-memory secret vault, but the first access
// in a process performs the vault unlock, which can block on a macOS keychain
// prompt. These run `async` + `spawn_blocking` so that blocking call never
// freezes the Tauri main thread (sync commands run there).
#[tauri::command]
pub async fn ai_runtime_set_provider_key(
    request: AiRuntimeProviderKeyRequest,
) -> Result<(), String> {
    let provider = request.provider.trim().to_string();
    if provider.is_empty() {
        return Err("a provider id is required".to_string());
    }
    let key = request.key.trim().to_string();
    if key.is_empty() {
        return Err("an API key is required".to_string());
    }
    tokio::task::spawn_blocking(move || app_infra::store_ai_provider_key(&provider, &key))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn ai_runtime_clear_provider_key(
    request: AiRuntimeProviderRequest,
) -> Result<(), String> {
    let provider = request.provider.trim().to_string();
    if provider.is_empty() {
        return Err("a provider id is required".to_string());
    }
    tokio::task::spawn_blocking(move || app_infra::delete_ai_provider_key(&provider))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn ai_runtime_has_provider_key(
    request: AiRuntimeProviderRequest,
) -> Result<bool, String> {
    let provider = request.provider.trim().to_string();
    if provider.is_empty() {
        return Err("a provider id is required".to_string());
    }
    tokio::task::spawn_blocking(move || app_infra::has_ai_provider_key(&provider))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

// MCP connector secrets: same keychain-off-the-main-thread pattern as the
// provider keys above, keyed by the MCP server instance id (which also keys the
// `mcp__<id>__` tool prefix a later slice parses).
#[tauri::command]
pub async fn mcp_set_server_secret(request: McpServerSecretRequest) -> Result<(), String> {
    let id = request.id.trim().to_string();
    if id.is_empty() {
        return Err("a server id is required".to_string());
    }
    let secret = request.secret.trim().to_string();
    if secret.is_empty() {
        return Err("a secret is required".to_string());
    }
    tokio::task::spawn_blocking(move || app_infra::store_mcp_server_secret(&id, &secret))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn mcp_clear_server_secret(request: McpServerRequest) -> Result<(), String> {
    let id = request.id.trim().to_string();
    if id.is_empty() {
        return Err("a server id is required".to_string());
    }
    tokio::task::spawn_blocking(move || app_infra::delete_mcp_server_secret(&id))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn mcp_has_server_secret(request: McpServerRequest) -> Result<bool, String> {
    let id = request.id.trim().to_string();
    if id.is_empty() {
        return Err("a server id is required".to_string());
    }
    tokio::task::spawn_blocking(move || app_infra::has_mcp_server_secret(&id))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
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
        provider: default_model.provider.trim().to_string(),
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
) -> Result<Vec<DiscoveredModel>, String> {
    let base_url = provider.base_url.trim();

    // OpenAI-compatible is the only kind that POSTs the keychain key to a
    // caller-supplied `base_url` (Anthropic/OpenAI hardcode their first-party
    // host below; local kinds carry no credential). Validate it before any
    // request fires: reject non-http(s) schemes and refuse to pair a well-known
    // first-party id (anthropic/openai) with a host other than its fixed one, so
    // a {id:"anthropic", kind:OpenaiCompatible, baseUrl:"…"} request can't send
    // the Anthropic key to an arbitrary host.
    if provider.kind == AiProviderKind::OpenaiCompatible && !base_url.is_empty() {
        validate_provider_base_url(&provider.id, base_url)?;
    }

    // Fireworks special-case: its OpenAI-compatible `/inference/v1/models`
    // route advertises only a small curated set (≈6), so page the proprietary
    // Gateway catalog instead to surface the full serverless model list. Every
    // other OpenAI-compatible provider (llama-swap, OpenRouter, …) keeps the
    // generic `/v1/models` path below — their `/models` already returns the
    // full catalog.
    if provider.kind == AiProviderKind::OpenaiCompatible && is_fireworks_host(base_url) {
        let api_key = app_infra::load_ai_provider_key(&provider.id)
            .map_err(|error| error.to_string())?
            .filter(|key| !key.is_empty())
            .ok_or_else(|| format!("no_provider_key:{}", provider.id))?;
        return list_fireworks_models(client, &base_host(base_url), &api_key).await;
    }

    let http_request = match provider.kind {
        AiProviderKind::Anthropic | AiProviderKind::Openai | AiProviderKind::OpenaiCompatible => {
            // The key lives in the keychain at the provider's instance id.
            let api_key = app_infra::load_ai_provider_key(&provider.id)
                .map_err(|error| error.to_string())?
                .filter(|key| !key.is_empty())
                .ok_or_else(|| format!("no_provider_key:{}", provider.id))?;
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
        .map_err(|error| describe_error(&error))?;

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
        .map(|entry| DiscoveredModel {
            context_window: entry.context_window(),
            id: entry.id.trim().to_string(),
        })
        .filter(|model| !model.id.is_empty())
        .collect())
}

/// List the Fireworks models worth surfacing in the picker: the public
/// `fireworks` serverless catalog (the models callable without a dedicated
/// deployment) PLUS every model in the API key's own account(s) (the caller's
/// fine-tunes — kept unfiltered since those are rarely "serverless"). The
/// public catalog is the source of truth, so its failure propagates; personal
/// account discovery is best-effort and merely adds to the list. The shared
/// [`is_chat_capable_model`] filter downstream still strips embedding/rerank
/// ids, so the picker ends up with the chat models.
///
/// `host` is the provider's own host (always `api.fireworks.ai` here).
async fn list_fireworks_models(
    client: &reqwest::Client,
    host: &str,
    api_key: &str,
) -> Result<Vec<DiscoveredModel>, String> {
    // Public serverless catalog first — this is the failure that should mark
    // the provider unlisted (e.g. an invalid key surfaces here).
    let mut ids = list_fireworks_account_models(client, host, api_key, "fireworks", true).await?;

    // The caller's own account(s): their fine-tunes, listed unfiltered. Purely
    // additive — a discovery/list failure just means no personal models, never
    // a failed pool.
    for account in discover_fireworks_accounts(client, host, api_key).await {
        if account == "fireworks" {
            continue;
        }
        match list_fireworks_account_models(client, host, api_key, &account, false).await {
            Ok(personal) => ids.extend(personal),
            Err(error) => tauri_plugin_log::log::warn!(
                "fireworks personal account {account} model listing skipped: {error}"
            ),
        }
    }

    Ok(ids)
}

/// The account segments (`accounts/<segment>` → `<segment>`) the API key
/// belongs to, via `GET /v1/accounts`. Best-effort: any error yields an empty
/// list so the public catalog still stands on its own.
async fn discover_fireworks_accounts(
    client: &reqwest::Client,
    host: &str,
    api_key: &str,
) -> Vec<String> {
    let url = format!("https://{host}/v1/accounts");
    let mut accounts: Vec<String> = Vec::new();
    let mut page_token: Option<String> = None;

    for page in 0..MODELS_MAX_PAGES {
        let mut request = client
            .get(&url)
            .query(&[("pageSize", "200")])
            .bearer_auth(api_key)
            .timeout(MODELS_REQUEST_TIMEOUT);
        if let Some(token) = &page_token {
            request = request.query(&[("pageToken", token.as_str())]);
        }

        let Ok(response) = request.send().await else {
            break;
        };
        if !response.status().is_success() {
            break;
        }
        let Ok(body) = response.text().await else {
            break;
        };
        let Ok(parsed) = serde_json::from_str::<FireworksAccountsResponse>(&body) else {
            break;
        };

        accounts.extend(
            parsed
                .accounts
                .into_iter()
                .filter_map(|account| {
                    account
                        .name
                        .trim()
                        .strip_prefix("accounts/")
                        .map(str::to_string)
                })
                .filter(|segment| !segment.is_empty()),
        );

        match parsed.next_page_token {
            Some(token) if !token.is_empty() => {
                if page + 1 == MODELS_MAX_PAGES {
                    tauri_plugin_log::log::warn!(
                        "fireworks /accounts pagination hit the {MODELS_MAX_PAGES}-page cap; truncating account list"
                    );
                    break;
                }
                page_token = Some(token);
            }
            _ => break,
        }
    }

    accounts
}

/// Page one Fireworks account's catalog (`GET …/v1/accounts/<account>/models`)
/// and return the model ids. When `serverless_only` is set, only models
/// callable without a dedicated deployment are kept (used for the public
/// `fireworks` catalog, where the vast majority are non-serverless addons).
async fn list_fireworks_account_models(
    client: &reqwest::Client,
    host: &str,
    api_key: &str,
    account: &str,
    serverless_only: bool,
) -> Result<Vec<DiscoveredModel>, String> {
    let catalog_url = format!("https://{host}/v1/accounts/{account}/models");
    let mut ids: Vec<DiscoveredModel> = Vec::new();
    let mut page_token: Option<String> = None;

    for page in 0..MODELS_MAX_PAGES {
        let mut request = client
            .get(&catalog_url)
            .query(&[("pageSize", "200")])
            .bearer_auth(api_key)
            .timeout(MODELS_REQUEST_TIMEOUT);
        if let Some(token) = &page_token {
            request = request.query(&[("pageToken", token.as_str())]);
        }

        let response = request.send().await.map_err(|error| describe_error(&error))?;
        let status = response.status();
        let body = response.text().await.map_err(|error| describe_error(&error))?;

        if !status.is_success() {
            let detail = body.trim();
            return Err(if detail.is_empty() {
                format!("fireworks catalog request failed with status {status}")
            } else {
                format!("fireworks catalog request failed with status {status}: {detail}")
            });
        }

        let parsed: FireworksCatalogResponse =
            serde_json::from_str(&body).map_err(|error| error.to_string())?;

        ids.extend(
            parsed
                .models
                .into_iter()
                .filter(|model| !serverless_only || model.supports_serverless)
                .map(|model| DiscoveredModel {
                    id: model.name.trim().to_string(),
                    context_window: model.context_length.filter(|&tokens| tokens > 0),
                })
                .filter(|model| !model.id.is_empty()),
        );

        match parsed.next_page_token {
            Some(token) if !token.is_empty() => {
                if page + 1 == MODELS_MAX_PAGES {
                    tauri_plugin_log::log::warn!(
                        "fireworks catalog pagination hit the {MODELS_MAX_PAGES}-page cap; truncating model list"
                    );
                    break;
                }
                page_token = Some(token);
            }
            _ => break,
        }
    }

    Ok(ids)
}

/// List the merged, provider-tagged model pool across every connected provider
/// (ADR 0034: one pool feeding the default-model picker, the Ask AI override
/// picker, and the Chat thread picker). Best-effort per provider: a provider
/// that fails to list (missing key, unreachable endpoint) does NOT fail the
/// whole pool — its successful peers still list, and the failure rides back in
/// `failures` so the UI can surface it and offer a retry (a transiently-down
/// LAN endpoint shouldn't silently vanish from the picker). Free-form entry of
/// a model id no provider advertises stays allowed.
#[tauri::command]
pub async fn ai_runtime_list_models(
    state: tauri::State<'_, RecordingSettingsState>,
    request: Option<AiRuntimeListModelsRequest>,
) -> Result<AiRuntimeModelsResult, String> {
    let providers = request
        .and_then(|request| request.providers)
        .unwrap_or_else(|| read_recording_settings(state.inner()).ai_runtime.providers);

    let client = reqwest::Client::new();
    let mut models: Vec<AiRuntimeModel> = Vec::new();
    let mut failures: Vec<AiRuntimeProviderFailure> = Vec::new();
    for provider in &providers {
        match list_models_for_provider(&client, provider).await {
            Ok(ids) => models.extend(
                ids.into_iter()
                    // Keep only chat-capable models in the pickers (image-gen,
                    // embeddings, TTS/whisper, rerank, moderation are filtered);
                    // free-form custom-model entry still lets a user pick a
                    // hidden model id by hand.
                    .filter(|model| is_chat_capable_model(&model.id))
                    .map(|model| AiRuntimeModel {
                        id: model.id,
                        // Tag with the provider instance id so same-kind
                        // instances stay attributable in the merged pool.
                        provider: provider.id.clone(),
                        context_window: model.context_window,
                    }),
            ),
            Err(error) => {
                tauri_plugin_log::log::warn!(
                    "model listing skipped provider {}: {error}",
                    provider.id
                );
                failures.push(AiRuntimeProviderFailure {
                    provider: provider.id.clone(),
                    reason: classify_listing_failure(&error),
                });
            }
        }
    }
    models.sort_by(|a, b| a.provider.cmp(&b.provider).then_with(|| a.id.cmp(&b.id)));
    models.dedup_by(|a, b| a.id == b.id && a.provider == b.provider);
    Ok(AiRuntimeModelsResult { models, failures })
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
                    id: "ollama".to_string(),
                    kind: AiProviderKind::Ollama,
                    label: String::new(),
                    base_url: String::new(),
                },
                AiProviderConfig {
                    id: "llamafile".to_string(),
                    kind: AiProviderKind::Llamafile,
                    label: String::new(),
                    base_url: "http://localhost:9090".to_string(),
                },
            ],
            default_model: Some(AiEngineRef {
                provider: "ollama".to_string(),
                model: "llama-default".to_string(),
            }),
            mcp_servers: Vec::new(),
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
            provider: "anthropic".to_string(),
            model: "claude-haiku-4-5".to_string(),
        });
        assert_eq!(
            resolve_engine_config(&settings, None, None).map(|_| ()),
            Err("provider_not_connected:anthropic".to_string())
        );
    }

    #[test]
    fn resolver_distinguishes_two_instances_of_the_same_kind() {
        // Two Ollama instances with distinct ids and endpoints coexist; a pin
        // to each resolves to that instance's endpoint, and the default model
        // resolves to its own instance — identity is the instance id, not kind.
        let settings = AiRuntimeSettings {
            enabled: true,
            providers: vec![
                AiProviderConfig {
                    id: "ollama".to_string(),
                    kind: AiProviderKind::Ollama,
                    label: String::new(),
                    base_url: "http://box-a:11434".to_string(),
                },
                AiProviderConfig {
                    id: "ollama-2".to_string(),
                    kind: AiProviderKind::Ollama,
                    label: "Box B".to_string(),
                    base_url: "http://box-b:11434".to_string(),
                },
            ],
            default_model: Some(AiEngineRef {
                provider: "ollama".to_string(),
                model: "default-model".to_string(),
            }),
            mcp_servers: Vec::new(),
        };

        // Pin to the second instance resolves to ITS endpoint.
        let (_, endpoint, model) = expect_local(
            resolve_engine_config(&settings, Some(("ollama-2", "pinned")), None)
                .expect("second-instance pin should resolve"),
        );
        assert_eq!(endpoint, "http://box-b:11434");
        assert_eq!(model, "pinned");

        // The default model resolves to the first instance's endpoint.
        let (_, endpoint, _) = expect_local(
            resolve_engine_config(&settings, None, None).expect("default should resolve"),
        );
        assert_eq!(endpoint, "http://box-a:11434");
    }

    #[test]
    fn resolver_requires_base_url_for_openai_compatible() {
        let settings = AiRuntimeSettings {
            enabled: true,
            providers: vec![AiProviderConfig {
                id: "openai_compatible".to_string(),
                kind: AiProviderKind::OpenaiCompatible,
                label: String::new(),
                base_url: String::new(),
            }],
            default_model: Some(AiEngineRef {
                provider: "openai_compatible".to_string(),
                model: "some-model".to_string(),
            }),
            mcp_servers: Vec::new(),
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

    #[test]
    fn base_host_strips_scheme_path_and_port() {
        assert_eq!(
            base_host("https://api.fireworks.ai/inference/v1"),
            "api.fireworks.ai"
        );
        assert_eq!(base_host("http://192.168.0.9:8080/v1"), "192.168.0.9");
        assert_eq!(base_host("https://API.Fireworks.AI/v1"), "api.fireworks.ai");
        assert_eq!(base_host("https://user@host.example/v1"), "host.example");
    }

    #[test]
    fn classify_listing_failure_maps_common_errors() {
        assert_eq!(
            classify_listing_failure("no_provider_key:openai_compatible-2"),
            "missing API key"
        );
        assert_eq!(classify_listing_failure("no_base_url"), "no base URL set");
        // Denied ≠ missing: the SecretVaultDenied Display must classify as a
        // keychain denial, never as "missing API key".
        assert_eq!(
            classify_listing_failure(
                &app_infra::AppInfraError::SecretVaultDenied("user denied prompt".to_string())
                    .to_string()
            ),
            "keychain access denied"
        );
        assert_eq!(
            classify_listing_failure("error sending request for url (http://192.168.0.9:8080/v1/models)"),
            "unreachable"
        );
        assert_eq!(
            classify_listing_failure("model listing request failed with status 401: invalid_api_key"),
            "authentication failed"
        );
        assert_eq!(
            classify_listing_failure("model listing request failed with status 500"),
            "couldn't list models"
        );
    }

    #[test]
    fn is_fireworks_host_matches_only_fireworks() {
        // Fireworks' OpenAI-compatible base routes to the Gateway catalog.
        assert!(is_fireworks_host("https://api.fireworks.ai/inference/v1"));
        // Other OpenAI-compatible providers keep the generic /v1/models path.
        assert!(!is_fireworks_host("http://192.168.0.9:8080/v1"));
        assert!(!is_fireworks_host("https://openrouter.ai/api/v1"));
        // A path that merely mentions the host must not match (host-anchored).
        assert!(!is_fireworks_host("https://evil.test/api.fireworks.ai/v1"));
    }

    #[test]
    fn validate_provider_base_url_rejects_nonhttp_schemes() {
        // Only http/https may ever carry a key to a model endpoint.
        assert!(validate_provider_base_url("custom-1", "file:///etc/passwd").is_err());
        assert!(validate_provider_base_url("custom-1", "gopher://host/v1").is_err());
        assert!(validate_provider_base_url("custom-1", "not a url").is_err());
        // A legitimate custom OpenAI-compatible endpoint passes.
        assert!(validate_provider_base_url("custom-1", "http://192.168.0.9:8080/v1").is_ok());
        assert!(validate_provider_base_url("custom-1", "https://openrouter.ai/api/v1").is_ok());
    }

    #[test]
    fn validate_provider_base_url_binds_first_party_ids_to_their_host() {
        // The leak: a first-party id paired with an attacker host is refused, so
        // the keychain key for "anthropic"/"openai" can't reach a foreign host.
        assert!(validate_provider_base_url("anthropic", "https://attacker.test/v1").is_err());
        assert!(validate_provider_base_url("openai", "https://attacker.test/v1").is_err());
        // The fixed first-party host stays allowed.
        assert!(validate_provider_base_url("anthropic", "https://api.anthropic.com/v1").is_ok());
        assert!(validate_provider_base_url("openai", "https://api.openai.com/v1").is_ok());
        // Non-reserved ids (same-kind custom instances) keep their own host.
        assert!(
            validate_provider_base_url("openai_compatible-2", "https://my-proxy.example/v1").is_ok()
        );
    }

    #[test]
    fn engine_config_for_ref_binds_first_party_ids_on_the_egress_path() {
        // H2 regression: the EngineConfig::Cloud built for REAL completions must
        // enforce the same key→host binding as model-listing. A first-party id
        // smuggled in as an OpenAI-compatible instance pointed at an attacker host
        // is rejected at config-build time — BEFORE the keychain key is loaded —
        // so the real Anthropic/OpenAI key (and the assembled prompt) never
        // egresses. The error is the same `base_url_host_mismatch` reason code
        // `validate_provider_base_url` returns.
        let settings = AiRuntimeSettings {
            enabled: true,
            providers: vec![AiProviderConfig {
                id: "anthropic".to_string(),
                kind: AiProviderKind::OpenaiCompatible,
                label: String::new(),
                base_url: "https://attacker.test/v1".to_string(),
            }],
            default_model: Some(AiEngineRef {
                provider: "anthropic".to_string(),
                model: "claude-haiku-4-5".to_string(),
            }),
            mcp_servers: Vec::new(),
        };
        assert_eq!(
            resolve_engine_config(&settings, None, None).map(|_| ()),
            Err("base_url_host_mismatch:anthropic".to_string())
        );

        // A custom (non-first-party) OpenAI-compatible instance is unaffected by
        // the host binding: its base_url passes validation and config-building
        // proceeds to the keychain load (which fails here only because no key is
        // stored in the test environment — never on a host-mismatch).
        let custom = AiRuntimeSettings {
            enabled: true,
            providers: vec![AiProviderConfig {
                id: "openai_compatible-2".to_string(),
                kind: AiProviderKind::OpenaiCompatible,
                label: String::new(),
                base_url: "https://my-proxy.example/v1".to_string(),
            }],
            default_model: Some(AiEngineRef {
                provider: "openai_compatible-2".to_string(),
                model: "some-model".to_string(),
            }),
            mcp_servers: Vec::new(),
        };
        let result = resolve_engine_config(&custom, None, None);
        if let Err(reason) = result {
            assert!(
                !reason.starts_with("base_url_host_mismatch"),
                "custom compat host must not be rejected by the binding, got {reason}"
            );
        }
    }

    #[test]
    fn mcp_secret_commands_guard_blank_ids_and_secrets() {
        // The guards return BEFORE any spawn_blocking, so these calls never
        // touch the keychain / secret store — safe to run against the real
        // commands with no fixture.
        let err = tauri::async_runtime::block_on(mcp_set_server_secret(McpServerSecretRequest {
            id: "  ".to_string(),
            secret: "token".to_string(),
        }))
        .expect_err("blank id must be rejected");
        assert_eq!(err, "a server id is required");

        let err = tauri::async_runtime::block_on(mcp_set_server_secret(McpServerSecretRequest {
            id: "github".to_string(),
            secret: "  ".to_string(),
        }))
        .expect_err("blank secret must be rejected");
        assert_eq!(err, "a secret is required");

        let err = tauri::async_runtime::block_on(mcp_clear_server_secret(McpServerRequest {
            id: String::new(),
        }))
        .expect_err("blank id must be rejected");
        assert_eq!(err, "a server id is required");

        let err = tauri::async_runtime::block_on(mcp_has_server_secret(McpServerRequest {
            id: " ".to_string(),
        }))
        .expect_err("blank id must be rejected");
        assert_eq!(err, "a server id is required");
    }
}
