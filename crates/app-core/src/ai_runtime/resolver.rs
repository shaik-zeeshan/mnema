//! The engine resolver: settings + pin + override → `EngineConfig`, and the
//! prerequisite gates. Two doors: sync [`resolve_engine_config`] (no network)
//! and async [`resolve_engine_config_live`] (ChatGPT freshness gate).

use super::*;

/// Map a cloud provider kind onto the engine crate's provider enum. `None`
/// for the local kinds.
pub(super) fn cloud_provider_kind(kind: AiProviderKind) -> Option<ai_engine::CloudProvider> {
    match kind {
        AiProviderKind::Anthropic => Some(ai_engine::CloudProvider::Anthropic),
        AiProviderKind::Openai => Some(ai_engine::CloudProvider::Openai),
        AiProviderKind::OpenaiCompatible => Some(ai_engine::CloudProvider::OpenAiCompatible),
        AiProviderKind::Chatgpt => Some(ai_engine::CloudProvider::Chatgpt),
        AiProviderKind::Ollama | AiProviderKind::Llamafile => None,
    }
}

/// Map a local provider kind onto the engine crate's local-kind enum. `None`
/// for the cloud kinds.
pub(super) fn local_kind(kind: AiProviderKind) -> Option<ai_engine::LocalKind> {
    match kind {
        AiProviderKind::Ollama => Some(ai_engine::LocalKind::Ollama),
        AiProviderKind::Llamafile => Some(ai_engine::LocalKind::Llamafile),
        _ => None,
    }
}

/// The Ask AI model override as the resolver's `(pin, feature_override_model)`
/// inputs, keeping the precedence thread pin → Ask AI override → default.
///
/// An override saved with its provider (`ask_ai_model_provider`) is a full
/// engine: it takes the pin slot when the thread has no usable pin, and when
/// its provider is no longer connected it is skipped (falls to the default
/// model) rather than sending its model id to some other provider. A model
/// saved without its provider is not an override (every writer saves the pair).
pub fn ask_ai_resolver_inputs<'a>(
    settings: &AiRuntimeSettings,
    access: &'a AccessSettings,
    pin: Option<(&'a str, &'a str)>,
) -> (Option<(&'a str, &'a str)>, Option<&'a str>) {
    let Some(model) = access
        .ask_ai_model
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
    else {
        return (pin, None);
    };
    let provider = access
        .ask_ai_model_provider
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty());
    let Some(provider) = provider else {
        return (pin, None);
    };
    let usable_pin = pin.filter(|(p, m)| {
        !p.trim().is_empty()
            && !m.trim().is_empty()
            && provider_config(settings, p.trim()).is_some()
    });
    if usable_pin.is_some() || provider_config(settings, provider).is_none() {
        return (usable_pin, None);
    }
    (Some((provider, model)), None)
}

/// The connected provider config for an instance id, if the user connected one.
/// Identity is the per-instance [`AiProviderConfig::id`] (not the kind), so
/// multiple providers of the same kind resolve independently.
pub(super) fn provider_config<'a>(
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
pub(super) fn local_endpoint(provider: &AiProviderConfig) -> String {
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
/// Map a `chatgpt` provider's stored token set onto an engine config.
///
/// The vault slot holds an OAuth token set (JSON), not an API key, so this
/// hands the engine the *stored* access token — possibly stale;
/// [`resolve_engine_config_live`] refreshes it before any real call. Both a
/// missing set and an unreadable one are `needs_reconnect` (the connect flow
/// is a login, not a key paste), which keeps `no_provider_key` meaning
/// "paste a key" for the kinds where that is the fix.
///
/// Split from [`engine_config_for_ref`] so the mapping is testable without a
/// vault: the caller supplies the load result.
pub(super) fn chatgpt_engine_config(
    provider_id: &str,
    model: &str,
    loaded: Result<Option<crate::chatgpt_auth::ChatgptTokenSet>, String>,
) -> Result<ai_engine::EngineConfig, String> {
    let needs_reconnect = || format!("needs_reconnect:{provider_id}");
    // `load_token_set` already reports a broken credential as the reconnect
    // code; anything else it returns is the vault refusing to answer, which is
    // not a verdict on the login. Pass it through unchanged so it reads the
    // same as it does for every pasted-key kind — "denied ≠ missing".
    let token_set = loaded
        .inspect_err(|error| {
            log::warn!("ai-runtime: reading chatgpt token set failed: {error}");
        })?
        .ok_or_else(needs_reconnect)?;
    // OpenAI rejected this set's refresh token (persisted mark): report it
    // without a send having to fail first.
    if token_set.rejected {
        return Err(needs_reconnect());
    }
    Ok(ai_engine::EngineConfig::Cloud {
        provider: ai_engine::CloudProvider::Chatgpt,
        model: model.to_string(),
        api_key: token_set.access_token,
        base_url: None,
    })
}

/// The ChatGPT picker list, gated on a stored token set.
///
/// The subscription backend has no model-discovery endpoint, so the list is
/// static (owned in `chatgpt_auth`, not rig, so updating it never waits on a
/// rig release) and carries no context-window metadata. The gate is the point:
/// "models listed" doubles as the provider-verification proof — onboarding
/// readiness, the Settings lamps, the Finish gate — so a never-connected
/// instance has to read as needing its login, not as live.
///
/// Split from [`list_models_for_provider`] so that gate is testable without a
/// `reqwest::Client`: the caller supplies the load result, exactly as
/// [`chatgpt_engine_config`] does.
pub(super) fn chatgpt_static_models(
    provider_id: &str,
    loaded: Result<Option<crate::chatgpt_auth::ChatgptTokenSet>, String>,
) -> Result<Vec<DiscoveredModel>, String> {
    let needs_reconnect = || format!("needs_reconnect:{provider_id}");
    // Same split as `chatgpt_engine_config`: a broken credential arrives as the
    // reconnect code already, and a vault that refused to answer keeps its own
    // error so the picker row reads "keychain access denied" here exactly as it
    // does for every other kind.
    let connected = loaded
        .inspect_err(|error| {
            log::warn!("ai-runtime: reading chatgpt token set failed: {error}");
        })?
        .is_some_and(|set| !set.rejected);
    if !connected {
        return Err(needs_reconnect());
    }
    Ok(crate::chatgpt_auth::CHATGPT_MODEL_IDS
        .iter()
        .map(|id| DiscoveredModel {
            id: (*id).to_string(),
            context_window: None,
        })
        .collect())
}

pub(super) fn engine_config_for_ref(
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
    // ChatGPT's credential is an OAuth token set, not a pasted key — see
    // `chatgpt_engine_config`.
    if matches!(kind, AiProviderKind::Chatgpt) {
        return chatgpt_engine_config(
            provider_id,
            model,
            crate::chatgpt_auth::load_token_set(provider_id),
        );
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
pub fn resolve_engine_config(
    settings: &AiRuntimeSettings,
    pin: Option<(&str, &str)>,
    feature_override_model: Option<&str>,
) -> Result<ai_engine::EngineConfig, String> {
    resolve_engine_ref(settings, pin, feature_override_model).map(|(_, config)| config)
}

/// [`resolve_engine_config`] plus the resolved provider *instance id* — the
/// internal shape shared by the sync resolver and the freshness-aware
/// [`resolve_engine_config_live`], which needs the id to key the ChatGPT
/// token-set vault slot.
pub(super) fn resolve_engine_ref(
    settings: &AiRuntimeSettings,
    pin: Option<(&str, &str)>,
    feature_override_model: Option<&str>,
) -> Result<(String, ai_engine::EngineConfig), String> {
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
                return engine_config_for_ref(settings, provider, model)
                    .map(|config| (provider.to_string(), config));
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
    let provider = default_model.provider.trim();
    engine_config_for_ref(settings, provider, model).map(|config| (provider.to_string(), config))
}

/// [`resolve_engine_config`] for call sites about to *use* the engine: same
/// precedence chain, plus the ChatGPT freshness gate — a resolved `chatgpt`
/// engine gets its access token refreshed (and the rotated set persisted)
/// when expired or about to expire. Failures surface as
/// `needs_reconnect:<id>`; no interactive login is ever started. Other
/// providers pass through untouched, so this is safe as the default resolver
/// everywhere an async context is available.
pub async fn resolve_engine_config_live(
    settings: &AiRuntimeSettings,
    pin: Option<(&str, &str)>,
    feature_override_model: Option<&str>,
) -> Result<ai_engine::EngineConfig, String> {
    resolve_engine_ref_live(settings, pin, feature_override_model)
        .await
        .map(|(_, config)| config)
}

/// [`resolve_engine_config_live`] plus the resolved provider *instance id*,
/// for a caller that records which engine answered (Ask AI stamps each turn).
pub async fn resolve_engine_ref_live(
    settings: &AiRuntimeSettings,
    pin: Option<(&str, &str)>,
    feature_override_model: Option<&str>,
) -> Result<(String, ai_engine::EngineConfig), String> {
    let (provider_id, mut config) = resolve_engine_ref(settings, pin, feature_override_model)?;
    if let ai_engine::EngineConfig::Cloud {
        provider: ai_engine::CloudProvider::Chatgpt,
        api_key,
        ..
    } = &mut config
    {
        *api_key = crate::chatgpt_auth::fresh_access_token(&provider_id).await?;
    }
    Ok((provider_id, config))
}

/// The static (no-network) half of the engine-configured prerequisite: master
/// switch on, at least one connected provider, a default model chosen, and the
/// default model's engine resolvable (key present / base URL set). Returns the
/// resolved default engine config so the async half can ping a local endpoint.
pub fn engine_prerequisite_static(
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
/// `"needs_reconnect:<id>"` / `"provider_unreachable:<id>"` (the `chatgpt`
/// kind's credential codes — the latter only from the live resolver),
/// `"no_base_url"`, `"no_provider_key:<id>"`, `"local_endpoint_unreachable"`.
/// Per ADR 0034 this is "master switch on + at least one usable provider + a
/// default model chosen"; a local default additionally needs its endpoint to be
/// reachable right now, matching `get_ai_runtime_status`.
pub async fn engine_configured_prerequisite(settings: &AiRuntimeSettings) -> Result<(), String> {
    local_endpoint_reachable(&engine_prerequisite_static(settings)?).await
}

/// [`engine_configured_prerequisite`] for Ask AI: same checks, but on the
/// engine an Ask AI turn without a thread pin runs on — the Ask AI model's own
/// provider when it has one — so a down default server never blocks an Ask AI
/// model on another provider, and a down Ask AI server is never reported ready.
pub async fn ask_ai_engine_prerequisite(
    settings: &AiRuntimeSettings,
    access: &AccessSettings,
) -> Result<(), String> {
    engine_prerequisite_static(settings)?;
    let (pin, model) = ask_ai_resolver_inputs(settings, access, None);
    local_endpoint_reachable(&resolve_engine_config(settings, pin, model)?).await
}

pub(super) async fn local_endpoint_reachable(
    config: &ai_engine::EngineConfig,
) -> Result<(), String> {
    if let ai_engine::EngineConfig::Local { endpoint, .. } = config {
        if !ai_engine::ping_endpoint(endpoint).await {
            return Err("local_endpoint_unreachable".to_string());
        }
    }
    Ok(())
}
