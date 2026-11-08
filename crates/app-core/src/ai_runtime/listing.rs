//! Provider model discovery: `/models` listing, Fireworks catalogs, base-URL guards.

use super::*;

/// Flatten an error and its whole `source()` chain into one string. `reqwest`'s
/// top-level `Display` is just "error sending request for url (…)"; the useful
/// cause ("tcp connect error: Connection refused (os error 61)", a DNS failure,
/// a TLS error) lives in the source chain, so we append each link.
pub(super) fn describe_error(error: &dyn std::error::Error) -> String {
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
pub(super) fn classify_listing_failure(error: &str) -> String {
    if error.starts_with("needs_reconnect:") || error.starts_with("provider_unreachable:") {
        // The `chatgpt` kind's two credential codes: "no usable token set" and
        // "the auth endpoint never answered". Neither is a plain network
        // failure the `contains("connect")` arm below should claim — it would
        // match `needs_re*connect*` on the substring — and each maps to
        // different advice downstream ("sign in again" vs "try again"). Pass
        // them through; the frontend owns the wording.
        error.to_string()
    } else if error.starts_with("no_provider_key") {
        "missing API key".to_string()
    } else if error.contains("keychain") && error.contains("denied") {
        // AppInfraError::SecretVaultDenied Display — a denied key store, NOT a
        // missing key (denied ≠ missing, ADR 0048 amendment).
        "keychain access denied".to_string()
    } else if error.starts_with("no_base_url") {
        "no base URL set".to_string()
    } else if error.starts_with("invalid_base_url") || error.starts_with("base_url_host_mismatch") {
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
/// parse covers every provider/runtime we list. Together returns the entries
/// as a bare top-level array instead, so both shapes are accepted.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum ModelsListResponse {
    Bare(Vec<ModelsListEntry>),
    Wrapped {
        #[serde(default)]
        data: Vec<ModelsListEntry>,
    },
}

impl ModelsListResponse {
    pub(super) fn into_entries(self) -> Vec<ModelsListEntry> {
        match self {
            Self::Bare(entries) | Self::Wrapped { data: entries } => entries,
        }
    }
}

/// Every name is a separate field (not a serde alias) so an entry carrying two
/// of them still parses instead of failing on a duplicate field.
#[derive(Debug, Deserialize)]
pub(super) struct ModelsListEntry {
    id: String,
    /// OpenRouter, xAI, Together.
    #[serde(default)]
    context_length: Option<u64>,
    /// vLLM.
    #[serde(default)]
    max_model_len: Option<u64>,
    /// Anthropic.
    #[serde(default)]
    max_input_tokens: Option<u64>,
    /// Groq, DeepSeek (DeepSeek may omit it).
    #[serde(default)]
    context_window: Option<u64>,
    /// Mistral.
    #[serde(default)]
    max_context_length: Option<u64>,
    /// llama.cpp-style servers (incl. llamafile).
    #[serde(default)]
    meta: Option<ModelsListEntryMeta>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ModelsListEntryMeta {
    /// The window the server actually runs with (recent llama.cpp builds).
    /// Preferred: `--fit` can shrink it far below the trained maximum.
    #[serde(default)]
    n_ctx: Option<u64>,
    /// The model's trained maximum, for builds that don't report `n_ctx`.
    #[serde(default)]
    n_ctx_train: Option<u64>,
}

impl ModelsListEntry {
    pub(super) fn context_window(&self) -> Option<u64> {
        let meta = self.meta.as_ref();
        [
            self.context_length,
            self.max_model_len,
            self.max_input_tokens,
            self.context_window,
            self.max_context_length,
            meta.and_then(|meta| meta.n_ctx),
            meta.and_then(|meta| meta.n_ctx_train),
        ]
        .into_iter()
        .flatten()
        // A 0 (llama.cpp router mode's placeholder) means "unknown", not a size.
        .find(|&tokens| tokens > 0)
    }
}

/// One discovered model: its id plus the provider-reported context window,
/// when the listing route advertised one.
#[derive(Debug)]
pub(super) struct DiscoveredModel {
    pub(super) id: String,
    pub(super) context_window: Option<u64>,
}

/// One page of Fireworks' proprietary Gateway catalog
/// (`/v1/accounts/{account}/models`). Unlike the OpenAI-compatible
/// `/inference/v1/models` route — which advertises only a small curated set —
/// this lists the full catalog, paginated via `nextPageToken`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FireworksCatalogResponse {
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
pub(super) struct FireworksCatalogModel {
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
pub(super) struct FireworksAccountsResponse {
    #[serde(default)]
    accounts: Vec<FireworksAccount>,
    #[serde(default)]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FireworksAccount {
    /// Fully-qualified resource name, e.g. `accounts/shaikzeeshan999-yo15`.
    #[serde(default)]
    name: String,
}

/// How long the `/models` request waits before giving up.
pub(super) const MODELS_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Hard ceiling on paginated `/models` (and `/accounts`) fetches. The per-request
/// timeout bounds a single page, but a server (or proxy) echoing the same
/// non-empty `nextPageToken` forever would loop indefinitely and hang the model
/// picker; this caps the page walk far above any realistic catalog size.
pub(super) const MODELS_MAX_PAGES: usize = 50;

/// `anthropic-version` header required by Anthropic's REST API.
pub(super) const ANTHROPIC_VERSION_HEADER: &str = "2023-06-01";

/// Build the `/models` URL for an OpenAI-style API base.
///
/// Bases that already include the `/v1` API prefix (the OpenAI-compatible and
/// first-party cloud bases, e.g. `https://api.fireworks.ai/inference/v1`) get
/// `/models` appended; bare hosts (a local Ollama/Llamafile endpoint like
/// `http://localhost:11434`) get the full `/v1/models` suffix.
pub(super) fn models_endpoint_url(base: &str) -> String {
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
pub(super) fn base_host(base: &str) -> String {
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
pub(super) fn is_fireworks_host(base: &str) -> bool {
    base_host(base) == "api.fireworks.ai"
}

/// The fixed host a well-known first-party provider id must talk to. `None` for
/// ids that legitimately carry a caller-chosen host (the `openai_compatible`
/// custom-endpoint flow, local runtimes, or same-kind instances with derived
/// ids). Used to stop a request from pairing a first-party keychain key with an
/// arbitrary `base_url`.
pub(super) fn fixed_host_for_provider_id(provider_id: &str) -> Option<&'static str> {
    // Instance ids are allocated by `newAiProviderId` (ai-providers.ts): the
    // first instance of a kind keeps the bare kind id, later ones get
    // `<kind>-2`, `<kind>-3`, … So a SECOND ChatGPT login lives at `chatgpt-2`
    // and a second OpenAI key at `openai-2` — the derived ids hold exactly the
    // same first-party credential and must be bound to the same host. Match on
    // the id root so they are. (`openai_compatible` / `openai_compatible-2`
    // have their own root and stay caller-chosen, as intended.)
    let root = provider_id
        .split_once('-')
        .map(|(root, _)| root)
        .unwrap_or(provider_id);
    match root {
        "anthropic" => Some("api.anthropic.com"),
        "openai" => Some("api.openai.com"),
        // Reserved for the same reason and then some: this slot holds an OAuth
        // token SET (access + long-lived refresh token), and the `chatgpt` kind
        // never reads `base_url` at all — so the only way this id can carry a
        // caller-chosen host is a kind-swapped record.
        "chatgpt" => Some("chatgpt.com"),
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
pub(super) fn validate_provider_base_url(provider_id: &str, base_url: &str) -> Result<(), String> {
    let trimmed = base_url.trim();
    let parsed = url::Url::parse(trimmed).map_err(|_| "invalid_base_url".to_string())?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err("invalid_base_url_scheme".to_string());
    }
    if let Some(fixed_host) = fixed_host_for_provider_id(provider_id) {
        // Compare the host the URL *parser* resolves — the same one reqwest
        // dials — not a hand-split of the string. `base_host` reads
        // `https://attacker.test?@chatgpt.com/v1` as `chatgpt.com` (it splits on
        // `/` then takes the last `@` segment), while WHATWG parsing puts
        // `?@chatgpt.com/v1` in the query and dials `attacker.test`. `#` and `\`
        // do the same. A pin that can be read two ways is not a pin.
        if parsed.host_str().unwrap_or_default().to_ascii_lowercase() != fixed_host {
            return Err(format!("base_url_host_mismatch:{provider_id}"));
        }
    }
    Ok(())
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
pub(super) fn is_chat_capable_model(id: &str) -> bool {
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
pub(super) async fn list_models_for_provider(
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

    // The ChatGPT subscription backend has no model-discovery endpoint; the
    // picker gets the static list (owned in `chatgpt_auth`, not rig, so it can
    // be updated without a rig release). No context-window metadata either.
    // Gate the list on a stored token set: "models listed" doubles as the
    // provider-verification signal (onboarding readiness, Settings lamps), and
    // a never-connected instance must read as needing its login, not as live.
    if provider.kind == AiProviderKind::Chatgpt {
        return chatgpt_static_models(
            &provider.id,
            crate::chatgpt_auth::load_token_set(&provider.id),
        );
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
                    client
                        .get(models_endpoint_url(base_url))
                        .bearer_auth(api_key)
                }
            }
        }
        AiProviderKind::Ollama | AiProviderKind::Llamafile => {
            client.get(models_endpoint_url(&local_endpoint(provider)))
        }
        AiProviderKind::Chatgpt => {
            unreachable!("chatgpt is answered by the static-list early return above")
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
        .into_entries()
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
pub(super) async fn list_fireworks_models(
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
            Err(error) => log::warn!(
                "fireworks personal account {account} model listing skipped: {}",
                classify_listing_failure(&error)
            ),
        }
    }

    Ok(ids)
}

/// The account segments (`accounts/<segment>` → `<segment>`) the API key
/// belongs to, via `GET /v1/accounts`. Best-effort: any error yields an empty
/// list so the public catalog still stands on its own.
pub(super) async fn discover_fireworks_accounts(
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
                    log::warn!(
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
pub(super) async fn list_fireworks_account_models(
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

        let response = request
            .send()
            .await
            .map_err(|error| describe_error(&error))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| describe_error(&error))?;

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
                    log::warn!(
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
pub async fn ai_runtime_list_models(
    core: &AppCore,
    request: Option<AiRuntimeListModelsRequest>,
) -> Result<AiRuntimeModelsResult, String> {
    let providers = request
        .and_then(|request| request.providers)
        .unwrap_or_else(|| {
            current_recording_settings(&core.state::<RecordingSettingsState>())
                .ai_runtime
                .providers
        });

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
                log::warn!(
                    "model listing skipped provider {}: {}",
                    provider.id,
                    classify_listing_failure(&error)
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
