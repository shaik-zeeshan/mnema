//! Provider keys, ChatGPT login, MCP secrets, status, connection test and
//! verification. The Tauri commands are one-liners over these.

use super::*;

// Secret access goes through the in-memory secret vault, but the first access
// in a process performs the vault unlock, which can block on a macOS keychain
// prompt. These run `async` + `spawn_blocking` so that blocking call never
// freezes the Tauri main thread (sync commands run there).
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

pub async fn ai_runtime_clear_provider_key(
    request: AiRuntimeProviderRequest,
) -> Result<(), String> {
    let provider = request.provider.trim().to_string();
    if provider.is_empty() {
        return Err("a provider id is required".to_string());
    }
    // Disconnect is a revocation, so invalidating any ChatGPT device poll or
    // token refresh still in flight and clearing the slot must happen together
    // — `revoke_provider_credential` does both under the one lock a concurrent
    // refresh's compare-and-swap also holds. Clearing the slot beside that lock
    // lets a refresh write the token set straight back into it.
    tokio::task::spawn_blocking(move || crate::chatgpt_auth::revoke_provider_credential(&provider))
        .await
        .map_err(|error| error.to_string())?
}

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

/// The last 4 characters of a provider's stored API key, so Settings can show
/// which key is which (`…4f2a`). This is the ONLY key material that ever
/// crosses IPC; `None` when no key is stored, the key is too short for 4
/// characters to be a small fraction of it, or the slot holds a ChatGPT OAuth
/// token set (JSON) rather than a key.
pub async fn ai_runtime_provider_key_ending(
    request: AiRuntimeProviderRequest,
) -> Result<Option<String>, String> {
    let provider = request.provider.trim().to_string();
    if provider.is_empty() {
        return Err("a provider id is required".to_string());
    }
    let key = tokio::task::spawn_blocking(move || app_infra::load_ai_provider_key(&provider))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    Ok(key.as_deref().and_then(key_ending))
}

pub(super) fn key_ending(key: &str) -> Option<String> {
    let key = key.trim();
    let chars: Vec<char> = key.chars().collect();
    if key.starts_with('{') || chars.len() < 12 {
        return None;
    }
    Some(chars[chars.len() - 4..].iter().collect())
}

/// Start a ChatGPT device-code login for one `chatgpt` provider instance.
/// Returns the user code + verify URL for the UI to show; the terminal
/// outcome arrives as a `chatgpt_login_update` event (see `chatgpt_auth`).
/// Disconnect reuses [`ai_runtime_clear_provider_key`] — the token set lives
/// in the same vault slot — and "is connected" is [`ai_runtime_has_provider_key`].
pub async fn ai_runtime_chatgpt_begin_login(
    core: &std::sync::Arc<AppCore>,
    request: AiRuntimeProviderRequest,
) -> Result<crate::chatgpt_auth::ChatgptLoginPrompt, String> {
    let provider = request.provider.trim().to_string();
    if provider.is_empty() {
        return Err("a provider id is required".to_string());
    }
    crate::chatgpt_auth::begin_login(core, provider).await
}

/// Abandon a ChatGPT device login the user walked away from.
///
/// Without this, dismissing the code UI only hides it: the detached poll keeps
/// hitting `auth.openai.com` for the rest of its 15 minutes, and this endpoint
/// treats a rate limit as terminal — so an abandoned login can be what kills
/// the next real one. Leaves any existing token set alone; disconnecting is
/// [`ai_runtime_clear_provider_key`].
pub async fn ai_runtime_chatgpt_cancel_login(
    request: AiRuntimeProviderRequest,
) -> Result<(), String> {
    let provider = request.provider.trim();
    if provider.is_empty() {
        return Err("a provider id is required".to_string());
    }
    crate::chatgpt_auth::cancel_login(provider);
    Ok(())
}

// MCP connector secrets: same keychain-off-the-main-thread pattern as the
// provider keys above, keyed by the MCP server instance id (which also keys the
// `mcp__<id>__` tool prefix a later slice parses).
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

/// A denied keychain/vault surfaces as `AppInfraError::SecretVaultDenied`'s
/// sentence; the status hands the UI a stable `vault_denied` code instead so
/// nothing has to match on prose. Every other reason is already a code.
pub(super) fn status_reason_code(reason: String) -> String {
    if classify_listing_failure(&reason) == "keychain access denied" {
        "vault_denied".to_string()
    } else {
        reason
    }
}

pub async fn get_ai_runtime_status(core: &AppCore) -> Result<AiRuntimeStatus, String> {
    let settings = current_recording_settings(&core.state::<RecordingSettingsState>()).ai_runtime;

    // Availability + reason flow through the single shared prerequisite so this
    // status surface and the feature gates can never drift on what "ready"
    // means. `configured` is "the static config is complete" (everything but a
    // local engine that is merely unreachable); `available` is the full
    // prerequisite (including the local reachability ping).
    let has_providers = !settings.providers.is_empty();
    let default_provider_kind = crate::user_context::worker::default_provider_kind(&settings);
    match engine_configured_prerequisite(&settings).await {
        Ok(()) => Ok(AiRuntimeStatus {
            enabled: settings.enabled,
            has_providers,
            configured: true,
            available: true,
            default_model: settings.default_model,
            default_provider_kind,
            reason: None,
        }),
        Err(reason) => Ok(AiRuntimeStatus {
            enabled: settings.enabled,
            has_providers,
            // Only the reachability ping fails AFTER the static config passed,
            // so that reason alone means "configured but currently offline".
            configured: reason == "local_endpoint_unreachable",
            available: false,
            default_model: settings.default_model,
            default_provider_kind,
            reason: Some(status_reason_code(reason)),
        }),
    }
}

pub async fn ai_runtime_test_connection(core: &AppCore) -> Result<AiRuntimeTestResult, String> {
    let settings = current_recording_settings(&core.state::<RecordingSettingsState>()).ai_runtime;

    // Test the global default model's engine. Deliberately NOT gated on the
    // master switch so the Settings card can verify a key/endpoint before the
    // user turns AI features on.
    let default_model = settings
        .default_model
        .clone()
        .filter(|model| !model.model.trim().is_empty())
        .ok_or_else(|| "no_default_model".to_string())?;
    let config = resolve_engine_config_live(&settings, None, None).await?;

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

/// Which provider instance to verify, against which list.
///
/// `providers` is the same in-progress draft `ai_runtime_list_models` accepts:
/// onboarding verifies a provider the user is still typing, long before any of
/// it is persisted. When absent, the persisted list is used. The API key is
/// NEVER on this request — it is loaded from the app vault by instance id
/// (ADR 0035), so the frontend neither sends nor receives it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRuntimeVerifyRequest {
    provider: String,
    #[serde(default)]
    providers: Option<Vec<AiProviderConfig>>,
}

/// Proof that one provider instance answered just now: the chat-capable model
/// ids it listed, and how long the round trip took.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRuntimeVerifyResult {
    provider: String,
    models: Vec<String>,
    latency_ms: u64,
}

/// Verify ONE connected provider instance by listing its models with the key
/// the vault already holds. This is the receipt onboarding's readiness rule is
/// built on: a provider counts as configured only when this returned live and
/// the chosen model id is in `models`. Every kind goes through the one
/// [`list_models_for_provider`] path, so cloud (Anthropic/OpenAI/compatible)
/// and local (Ollama/Llamafile) verify identically — and the local case fails
/// cleanly on [`MODELS_REQUEST_TIMEOUT`] instead of hanging.
///
/// `Err` is the same short reason [`classify_listing_failure`] puts on a picker
/// row (`unreachable`, `authentication failed`, `missing API key`, …).
pub async fn verify_ai_provider(
    core: &AppCore,
    request: AiRuntimeVerifyRequest,
) -> Result<AiRuntimeVerifyResult, String> {
    let provider_id = request.provider.trim().to_string();
    if provider_id.is_empty() {
        return Err("a provider id is required".to_string());
    }
    let providers = request.providers.unwrap_or_else(|| {
        current_recording_settings(&core.state::<RecordingSettingsState>())
            .ai_runtime
            .providers
    });
    let provider = providers
        .iter()
        .find(|candidate| candidate.id == provider_id)
        .ok_or_else(|| format!("provider_not_connected:{provider_id}"))?;

    let started = std::time::Instant::now();
    let discovered = list_models_for_provider(&reqwest::Client::new(), provider)
        .await
        .map_err(|error| {
            let reason = classify_listing_failure(&error);
            log::warn!("provider {provider_id} failed verification: {reason}");
            reason
        })?;
    Ok(AiRuntimeVerifyResult {
        provider: provider_id,
        models: discovered
            .into_iter()
            .map(|model| model.id)
            .filter(|id| is_chat_capable_model(id))
            .collect(),
        latency_ms: started.elapsed().as_millis() as u64,
    })
}
