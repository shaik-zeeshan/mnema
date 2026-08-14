//! ChatGPT-subscription OAuth for the `chatgpt` provider kind (ADR 0058).
//!
//! The credential for a `chatgpt` provider is not a pasted API key but an OAuth
//! token set (access + refresh token) acquired through OpenAI's device-code
//! flow and stored — as one JSON blob — in the secret vault under the provider
//! instance id, exactly where other providers keep their API key. This module
//! owns the whole token lifecycle:
//!
//! - [`begin_login`] runs the device-code flow: fetch a user code, hand it to
//!   the UI, poll until the user approves at `auth.openai.com/codex/device`,
//!   exchange for tokens, and persist the set.
//! - [`fresh_access_token`] is the per-call gate: load the set, refresh it via
//!   `oauth/token` when the access token is expired or about to expire, persist
//!   the rotated set, and hand back a ready access token.
//!
//! rig's own `providers::chatgpt` OAuth support is deliberately unused for
//! persistence: it only caches tokens in a plaintext `auth.json` file (with
//! `auth_file: None` it re-runs the login on every call), and plaintext
//! secrets on disk violate the vault invariant. Completions still go through
//! rig — the engine crate receives the access token as a per-call
//! `ChatGPTAuth::AccessToken`.
//!
//! Endpoints, client id, and poll semantics mirror rig 0.41's implementation
//! (which mirrors the Codex CLI): device-code poll answers 403/404 while the
//! user has not approved yet, and the refresh grant preserves the previous
//! refresh token when the response omits a rotated one. There is no unattended
//! login anywhere: a token set that cannot be refreshed surfaces as a
//! `needs_reconnect:<id>` reason code, never as a device-code prompt.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use tauri::Emitter;

/// OpenAI's public Codex app registration — the client id the ChatGPT
/// subscription backend accepts (same one the Codex CLI and rig use).
const CHATGPT_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const DEVICE_CODE_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/usercode";
const DEVICE_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";
const OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
/// Where the user types the code. Returned to the UI so it can open the page.
pub const DEVICE_VERIFY_URL: &str = "https://auth.openai.com/codex/device";

/// Refresh when the access token has less than this long left. Matches rig's
/// 60-second skew.
const TOKEN_EXPIRY_SKEW_SECONDS: i64 = 60;
/// Give up on a device-code login the user never approves. Matches rig/Codex.
const DEVICE_CODE_TIMEOUT_SECONDS: u64 = 15 * 60;
const DEVICE_CODE_POLL_SLEEP_SECONDS: u64 = 5;
/// Bound on one `auth.openai.com` round trip. Every AI call awaits a refresh
/// before the engine runs — and holds the provider's refresh lock while it
/// does — so a stalled connection must fail the call, not hang the feature.
const OAUTH_REQUEST_TIMEOUT_SECONDS: u64 = 30;

/// The models the ChatGPT subscription backend serves. The picker list is
/// static — owned here (not re-exported from rig) so updating it never waits
/// on a rig release; rig's constants proved stale live (`gpt-5.4` 400s with
/// "not supported when using Codex with a ChatGPT account"). Source of truth:
/// the `visibility: "list"` entries in openai/codex
/// `codex-rs/models-manager/models.json` (retired ids like `gpt-5.4` carry an
/// `upgrade` pointer there). A plan-tier mismatch surfaces as a provider
/// error at call time.
pub const CHATGPT_MODEL_IDS: &[&str] = &[
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-5.6-luna",
    "gpt-5.5",
    "gpt-5.2",
];

/// The vault-persisted OAuth token set for one `chatgpt` provider instance.
/// Serialized as JSON into the same vault slot other providers use for their
/// API key (`app_infra` key store, keyed by provider instance id).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatgptTokenSet {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Unix seconds the access token expires at, read off the JWT `exp` claim.
    /// `None` when the claim can't be read — treated as expired so every use
    /// goes through a refresh attempt.
    #[serde(default)]
    pub expires_at: Option<i64>,
}

impl ChatgptTokenSet {
    fn expires_within_skew(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        match self.expires_at {
            Some(expires_at) => expires_at - now <= TOKEN_EXPIRY_SKEW_SECONDS,
            None => true,
        }
    }
}

/// Read the `exp` claim (unix seconds) off a JWT access token without
/// verifying it — we only need a refresh timing hint, not trust.
fn jwt_expiration_seconds(token: &str) -> Option<i64> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::prelude::BASE64_URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    claims.get("exp")?.as_i64()
}

/// Load the persisted token set for a provider instance. `Ok(None)` when the
/// provider was never connected; `Err` carries a vault/parse failure.
pub fn load_token_set(provider_id: &str) -> Result<Option<ChatgptTokenSet>, String> {
    let Some(raw) = app_infra::load_ai_provider_key(provider_id).map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    if raw.trim().is_empty() {
        return Ok(None);
    }
    serde_json::from_str(&raw).map(Some).map_err(|e| e.to_string())
}

fn store_token_set(provider_id: &str, set: &ChatgptTokenSet) -> Result<(), String> {
    let raw = serde_json::to_string(set).map_err(|e| e.to_string())?;
    app_infra::store_ai_provider_key(provider_id, &raw).map_err(|e| e.to_string())
}

/// The per-call freshness gate: return an access token ready to authenticate a
/// completion, refreshing (and persisting the rotated set) when the stored one
/// is expired or about to expire. Every failure — no token set, unparseable
/// set, refresh rejected — collapses to the `needs_reconnect:<id>` reason code
/// so features render "Reconnect ChatGPT in Settings" instead of a raw error.
/// Never starts an interactive login.
pub async fn fresh_access_token(provider_id: &str) -> Result<String, String> {
    fresh_access_token_with(provider_id, refresh_grant).await
}

/// The network half of a refresh, named as a plain `fn` pointer so a test can
/// stand in a fake OpenAI without a generic on the call path.
type RefreshCall =
    fn(String) -> Pin<Box<dyn Future<Output = Result<OAuthTokenResponse, String>> + Send>>;

fn refresh_grant(
    refresh_token: String,
) -> Pin<Box<dyn Future<Output = Result<OAuthTokenResponse, String>> + Send>> {
    Box::pin(async move {
        oauth_token_request(&[
            ("client_id", CHATGPT_CLIENT_ID),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token.as_str()),
            ("scope", "openid profile email"),
        ])
        .await
    })
}

/// One in-flight refresh per provider instance. Ask AI turns, conversation
/// titles, the user-context worker, digest and Test Connection each resolve
/// their engine independently, so several can meet the same expiring token set
/// at once. OpenAI rotates the refresh token on use, so a parallel second POST
/// replays a consumed one and comes back `invalid_grant` — exactly what rig
/// 0.41's `should_reauthenticate_after_refresh` classifies as "log in again",
/// i.e. a spurious `needs_reconnect` for a healthy login.
///
/// ponytail: one lock per provider id, never evicted — bounded by the number of
/// configured providers.
static REFRESH_LOCKS: Mutex<Option<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    Mutex::new(None);

fn refresh_lock(provider_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    let mut map = REFRESH_LOCKS.lock().expect("chatgpt refresh locks");
    map.get_or_insert_with(HashMap::new)
        .entry(provider_id.to_string())
        .or_default()
        .clone()
}

async fn load_token_set_off_thread(provider_id: &str) -> Result<Option<ChatgptTokenSet>, String> {
    let id = provider_id.to_string();
    tokio::task::spawn_blocking(move || load_token_set(&id))
        .await
        .map_err(|e| e.to_string())?
}

async fn fresh_access_token_with(
    provider_id: &str,
    refresh: RefreshCall,
) -> Result<String, String> {
    let needs_reconnect = || format!("needs_reconnect:{provider_id}");
    let load = || async move {
        load_token_set_off_thread(provider_id)
            .await
            .map_err(|error| {
                tauri_plugin_log::log::warn!("chatgpt-auth: loading token set failed: {error}");
                needs_reconnect()
            })?
            .ok_or_else(needs_reconnect)
    };

    let set = load().await?;
    if !set.expires_within_skew() {
        return Ok(set.access_token);
    }

    // Serialize the refresh for this provider, then re-read the slot: the
    // caller we queued behind may already have rotated the set, and replaying
    // its consumed refresh token fails this call *and* persists a dead token.
    let lock = refresh_lock(provider_id);
    let _guard = lock.lock().await;
    let set = load().await?;
    if !set.expires_within_skew() {
        return Ok(set.access_token);
    }

    // Captured under the lock so a disconnect landing during the round-trip
    // invalidates the rotated write instead of resurrecting the credential.
    let generation = current_login_generation(provider_id);
    let refresh_token = set.refresh_token.clone().ok_or_else(needs_reconnect)?;
    let tokens = refresh(refresh_token.clone()).await.map_err(|error| {
        tauri_plugin_log::log::warn!("chatgpt-auth: token refresh failed for {provider_id}: {error}");
        needs_reconnect()
    })?;

    // The refresh grant may omit (or blank out) a rotated refresh token; keep
    // the old one in both cases.
    let rotated = token_set_from_grant(tokens, Some(refresh_token));
    let id = provider_id.to_string();
    let persisted = rotated.clone();
    tokio::task::spawn_blocking(move || persist_token_set_if_current(&id, generation, &persisted))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|error| {
            tauri_plugin_log::log::warn!("chatgpt-auth: persisting refreshed token set failed: {error}");
            needs_reconnect()
        })?;

    Ok(rotated.access_token)
}

#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    /// Seconds until the access token expires. Only consulted when the access
    /// token is not a readable JWT — see [`token_set_from_grant`].
    #[serde(default)]
    expires_in: Option<i64>,
}

/// Build the persisted token set from a grant response.
///
/// Two things the raw response cannot be trusted to give us directly:
/// `expires_at` (the access token is usually a JWT, but an opaque one still
/// comes with the standard `expires_in`, and treating an unknown expiry as
/// "expired" forces a full refresh — and a refresh-token rotation — before
/// *every* completion), and a *blank* rotated refresh token, which is not a
/// rotation: persisting it kills the next refresh and silently signs the user
/// out one access-token lifetime later.
fn token_set_from_grant(
    tokens: OAuthTokenResponse,
    previous_refresh_token: Option<String>,
) -> ChatgptTokenSet {
    let expires_at = jwt_expiration_seconds(&tokens.access_token).or_else(|| {
        let expires_in = tokens.expires_in?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs() as i64;
        Some(now + expires_in)
    });
    ChatgptTokenSet {
        expires_at,
        access_token: tokens.access_token,
        refresh_token: tokens
            .refresh_token
            .filter(|token| !token.trim().is_empty())
            .or(previous_refresh_token),
    }
}

/// The client for every `auth.openai.com` round trip.
fn oauth_http_client() -> reqwest::Client {
    http_client_with_timeout(std::time::Duration::from_secs(OAUTH_REQUEST_TIMEOUT_SECONDS))
}

fn http_client_with_timeout(timeout: std::time::Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// POST an `application/x-www-form-urlencoded` grant to `oauth/token`.
async fn oauth_token_request(form: &[(&str, &str)]) -> Result<OAuthTokenResponse, String> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(form)
        .finish();
    let response = oauth_http_client()
        .post(OAUTH_TOKEN_URL)
        .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status();
    let body = response.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("token request failed with status {status}: {}", body.trim()));
    }
    serde_json::from_str(&body).map_err(|e| e.to_string())
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_auth_id: String,
    #[serde(alias = "usercode")]
    user_code: String,
    #[serde(default, deserialize_with = "deserialize_optional_u64")]
    interval: Option<u64>,
}

/// OpenAI returns `interval` as either a number or a string ("5"); mirror rig
/// 0.41's lenient parse.
fn deserialize_optional_u64<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    // `interval` is an optional poll hint with a default, so ANY shape we do
    // not recognise (a float, a negative, a bool) falls back to the default
    // rather than failing the response that carries the mandatory user code.
    Ok(
        match Option::<serde_json::Value>::deserialize(deserializer)? {
            Some(serde_json::Value::Number(value)) => value.as_u64(),
            Some(serde_json::Value::String(value)) => value.trim().parse::<u64>().ok(),
            _ => None,
        },
    )
}

#[derive(Debug, Deserialize)]
struct DeviceTokenResponse {
    authorization_code: String,
    code_verifier: String,
}

/// What the connect UI needs to show after starting a login.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatgptLoginPrompt {
    pub user_code: String,
    pub verify_url: String,
}

/// Terminal login outcome, emitted as the `chatgpt_login_update` event once the
/// background poll finishes.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatgptLoginUpdate {
    provider_id: String,
    connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Monotonic generation per provider id, bumped by every `begin_login`, so a
/// superseded poll (user clicked Connect again) exits silently instead of
/// racing the newer one's outcome event.
static LOGIN_GENERATIONS: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);
static LOGIN_GENERATION_COUNTER: AtomicU64 = AtomicU64::new(1);

fn bump_login_generation(provider_id: &str) -> u64 {
    let generation = LOGIN_GENERATION_COUNTER.fetch_add(1, Ordering::SeqCst);
    let mut map = LOGIN_GENERATIONS.lock().expect("login generations lock");
    map.get_or_insert_with(HashMap::new)
        .insert(provider_id.to_string(), generation);
    generation
}

fn login_generation_is_current(provider_id: &str, generation: u64) -> bool {
    let map = LOGIN_GENERATIONS.lock().expect("login generations lock");
    map.as_ref().and_then(|m| m.get(provider_id)) == Some(&generation)
}

/// Start a device-code login for one provider instance.
///
/// Fetches the user code synchronously (so the UI can render it from the
/// command's return value), then polls for approval in a background task. The
/// terminal outcome — token set persisted, or failure/timeout — is emitted as
/// one `chatgpt_login_update` event; a login superseded by a newer
/// `begin_login` for the same provider emits nothing.
pub async fn begin_login(
    app: tauri::AppHandle,
    provider_id: String,
) -> Result<ChatgptLoginPrompt, String> {
    let client = oauth_http_client();
    let body = serde_json::json!({ "client_id": CHATGPT_CLIENT_ID }).to_string();
    let response = client
        .post(DEVICE_CODE_URL)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status();
    let text = response.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!(
            "device code request failed with status {status}: {}",
            text.trim()
        ));
    }
    let device: DeviceCodeResponse = serde_json::from_str(&text).map_err(|e| e.to_string())?;

    let prompt = ChatgptLoginPrompt {
        user_code: device.user_code.clone(),
        verify_url: DEVICE_VERIFY_URL.to_string(),
    };

    let generation = bump_login_generation(&provider_id);
    tauri::async_runtime::spawn(async move {
        let outcome = poll_and_store(&client, &device, &provider_id, generation).await;
        if !login_generation_is_current(&provider_id, generation) {
            return;
        }
        let update = match outcome {
            Ok(()) => {
                tauri_plugin_log::log::info!("chatgpt-auth: {provider_id} connected");
                ChatgptLoginUpdate {
                    provider_id: provider_id.clone(),
                    connected: true,
                    error: None,
                }
            }
            Err(error) => {
                tauri_plugin_log::log::warn!("chatgpt-auth: login for {provider_id} failed: {error}");
                ChatgptLoginUpdate {
                    provider_id: provider_id.clone(),
                    connected: false,
                    error: Some(error),
                }
            }
        };
        if let Err(error) = app.emit("chatgpt_login_update", &update) {
            tauri_plugin_log::log::warn!("chatgpt-auth: emitting login update failed: {error}");
        }
    });

    Ok(prompt)
}

/// How long to wait between device-code polls.
/// The server's hint is advisory: `0` would turn the detached poll into a hot
/// loop against the device endpoint, and an oversized value would park the task
/// in one sleep long past the 15-minute deadline (only checked at the top of
/// the loop).
fn poll_sleep_seconds(interval: Option<u64>) -> u64 {
    interval
        .unwrap_or(DEVICE_CODE_POLL_SLEEP_SECONDS)
        .clamp(1, 60)
}

/// What one device-token poll response means. Split out from the loop so the
/// endpoint's three-way answer is decided in one pure place.
#[derive(Debug)]
enum PollStep {
    /// The user approved: the response carries the authorization code.
    Approved(Box<DeviceTokenResponse>),
    /// Not approved yet — sleep and ask again.
    Pending,
    /// Terminal failure; the login is over.
    Failed(String),
}

fn classify_poll_response(status: reqwest::StatusCode, text: &str) -> PollStep {
    if status.is_success() {
        return match serde_json::from_str::<DeviceTokenResponse>(text) {
            Ok(token) => PollStep::Approved(Box::new(token)),
            Err(error) => PollStep::Failed(error.to_string()),
        };
    }
    // 403/404 mean "not approved yet" on this endpoint; keep polling. Every
    // other status is terminal — notably 429, where continuing to poll would
    // deepen the rate limit we just hit.
    if status.as_u16() == 403 || status.as_u16() == 404 {
        return PollStep::Pending;
    }
    PollStep::Failed(format!(
        "device authorization failed with status {status}: {}",
        text.trim()
    ))
}

/// Poll until the user approves, the endpoint refuses, or `timeout` elapses.
///
/// The HTTP call is injected so the loop's decisions — 403/404 keeps polling,
/// anything else stops, and the deadline actually bounds the wait — are
/// testable without a network or a 15-minute test.
async fn await_authorization<P, Fut>(
    poll: P,
    interval: std::time::Duration,
    timeout: std::time::Duration,
) -> Result<DeviceTokenResponse, String>
where
    P: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<(reqwest::StatusCode, String), String>>,
{
    let start = std::time::Instant::now();
    loop {
        if start.elapsed() >= timeout {
            return Err("timed out waiting for ChatGPT device authorization".to_string());
        }
        let (status, text) = poll().await?;
        match classify_poll_response(status, &text) {
            PollStep::Approved(token) => return Ok(*token),
            PollStep::Failed(error) => return Err(error),
            // Never sleep past the deadline: it is only checked between polls,
            // so a long interval would otherwise park the task well beyond it.
            PollStep::Pending => {
                let left = timeout.saturating_sub(start.elapsed());
                if left.is_zero() {
                    return Err(
                        "timed out waiting for ChatGPT device authorization".to_string()
                    );
                }
                tokio::time::sleep(interval.min(left)).await;
            }
        }
    }
}

/// The background half of the device flow: poll for user approval, exchange
/// the authorization code for tokens, persist the set.
async fn poll_and_store(
    client: &reqwest::Client,
    device: &DeviceCodeResponse,
    provider_id: &str,
    generation: u64,
) -> Result<(), String> {
    let poll_body = serde_json::json!({
        "device_auth_id": device.device_auth_id,
        "user_code": device.user_code,
    })
    .to_string();

    let code = await_authorization(
        || async {
            let response = client
                .post(DEVICE_TOKEN_URL)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(poll_body.clone())
                .send()
                .await
                .map_err(|e| e.to_string())?;
            let status = response.status();
            let text = response.text().await.map_err(|e| e.to_string())?;
            Ok((status, text))
        },
        std::time::Duration::from_secs(poll_sleep_seconds(device.interval)),
        std::time::Duration::from_secs(DEVICE_CODE_TIMEOUT_SECONDS),
    )
    .await?;

    let redirect_uri = "https://auth.openai.com/deviceauth/callback";
    let tokens = oauth_token_request(&[
        ("grant_type", "authorization_code"),
        ("code", code.authorization_code.as_str()),
        ("redirect_uri", redirect_uri),
        ("client_id", CHATGPT_CLIENT_ID),
        ("code_verifier", code.code_verifier.as_str()),
    ])
    .await?;

    let set = token_set_from_grant(tokens, None);
    let id = provider_id.to_string();
    tokio::task::spawn_blocking(move || persist_token_set_if_current(&id, generation, &set))
        .await
        .map_err(|e| e.to_string())?
}

/// Persist a token set produced by the login (or refresh) of `generation` —
/// but only while that login is still the current one for this provider.
fn persist_token_set_if_current(
    provider_id: &str,
    generation: u64,
    set: &ChatgptTokenSet,
) -> Result<(), String> {
    // The generation lock is held across the vault write, so a concurrent
    // disconnect either bumps first (we skip) or clears afterwards — it can
    // never interleave into "cleared, then written back".
    let map = LOGIN_GENERATIONS.lock().expect("login generations lock");
    if map
        .as_ref()
        .and_then(|m| m.get(provider_id))
        .copied()
        .unwrap_or(0)
        != generation
    {
        tauri_plugin_log::log::info!(
            "chatgpt-auth: dropping a token set from a superseded login for {provider_id}"
        );
        return Ok(());
    }
    store_token_set(provider_id, set)
}

/// Invalidate any in-flight login poll or token refresh for a provider. Called
/// by disconnect (`ai_runtime_clear_provider_key`): without it, work still in
/// flight writes the credential back into the vault slot the user just cleared.
pub fn cancel_login(provider_id: &str) {
    bump_login_generation(provider_id);
}

/// The generation an in-flight refresh must still see when it persists. `0`
/// means no login or disconnect was recorded for this provider in this process,
/// so any later bump differs from it and a disconnect mid-refresh still wins.
fn current_login_generation(provider_id: &str) -> u64 {
    let map = LOGIN_GENERATIONS.lock().expect("login generations lock");
    map.as_ref()
        .and_then(|m| m.get(provider_id))
        .copied()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt_with_exp(exp: i64) -> String {
        let header = base64::prelude::BASE64_URL_SAFE_NO_PAD.encode(b"{\"alg\":\"none\"}");
        let payload = base64::prelude::BASE64_URL_SAFE_NO_PAD
            .encode(serde_json::json!({ "exp": exp }).to_string());
        format!("{header}.{payload}.sig")
    }

    #[test]
    fn jwt_expiration_reads_exp_claim() {
        assert_eq!(jwt_expiration_seconds(&jwt_with_exp(1_234_567)), Some(1_234_567));
        assert_eq!(jwt_expiration_seconds("not-a-jwt"), None);
    }

    #[test]
    fn token_set_expiry_honours_skew_and_missing_claim() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let fresh = ChatgptTokenSet {
            access_token: "t".into(),
            refresh_token: None,
            expires_at: Some(now + 3600),
        };
        let expiring = ChatgptTokenSet {
            access_token: "t".into(),
            refresh_token: None,
            expires_at: Some(now + TOKEN_EXPIRY_SKEW_SECONDS - 5),
        };
        let unknown = ChatgptTokenSet {
            access_token: "t".into(),
            refresh_token: None,
            expires_at: None,
        };
        assert!(!fresh.expires_within_skew());
        assert!(expiring.expires_within_skew());
        assert!(unknown.expires_within_skew(), "missing exp must force a refresh attempt");
    }

    #[test]
    fn device_code_response_accepts_string_or_numeric_interval() {
        let as_string: DeviceCodeResponse = serde_json::from_str(
            r#"{"device_auth_id":"d","user_code":"u","interval":"5"}"#,
        )
        .unwrap();
        assert_eq!(as_string.interval, Some(5));
        let as_number: DeviceCodeResponse =
            serde_json::from_str(r#"{"device_auth_id":"d","usercode":"u","interval":5}"#).unwrap();
        assert_eq!(as_number.interval, Some(5));
        let missing: DeviceCodeResponse =
            serde_json::from_str(r#"{"device_auth_id":"d","user_code":"u"}"#).unwrap();
        assert_eq!(missing.interval, None);
    }

    /// One scratch, file-key-backed vault for this test binary: the process
    /// vault slot is global, so vault-touching tests share it and must use ids
    /// unique to themselves.
    fn install_test_vault() {
        crate::secret_vault_test_support::install_shared_test_secret_vault();
    }

    fn token_set(access: &str, refresh: &str, expires_at: i64) -> ChatgptTokenSet {
        ChatgptTokenSet {
            access_token: access.to_string(),
            refresh_token: Some(refresh.to_string()),
            expires_at: Some(expires_at),
        }
    }

    fn unix_now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    /// A fake OpenAI that rotates the refresh token on every grant and answers
    /// a replay with `invalid_grant` — the behaviour rig 0.41 encodes in
    /// `should_reauthenticate_after_refresh` (400/401 + `invalid_grant` means
    /// "log in again").
    static FAKE_CONSUMED_REFRESH_TOKENS: Mutex<Option<std::collections::HashSet<String>>> =
        Mutex::new(None);
    static FAKE_REFRESH_CALLS: AtomicU64 = AtomicU64::new(0);

    fn rotating_refresh_grant(
        refresh_token: String,
    ) -> Pin<Box<dyn Future<Output = Result<OAuthTokenResponse, String>> + Send>> {
        Box::pin(async move {
            // Stand in for the round-trip, so both callers are genuinely in
            // flight at the same time.
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            FAKE_REFRESH_CALLS.fetch_add(1, Ordering::SeqCst);
            let mut consumed = FAKE_CONSUMED_REFRESH_TOKENS
                .lock()
                .expect("consumed refresh tokens");
            let consumed = consumed.get_or_insert_with(std::collections::HashSet::new);
            if !consumed.insert(refresh_token.clone()) {
                return Err(
                    "token request failed with status 400 Bad Request: {\"error\":\"invalid_grant\"}"
                        .to_string(),
                );
            }
            Ok(OAuthTokenResponse {
                access_token: jwt_with_exp(unix_now() + 3600),
                refresh_token: Some(format!("{refresh_token}-rotated")),
                expires_in: None,
            })
        })
    }

    /// Two AI features (say an Ask AI turn and the user-context worker) meet the
    /// same expiring token set at once. Each one runs load -> refresh -> store
    /// on the same vault slot, so without single-flight both POST the *same*
    /// refresh token; OpenAI consumes it on first use, the loser gets
    /// `invalid_grant`, and the user is told to reconnect a healthy login.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_refreshes_never_replay_a_consumed_refresh_token() {
        install_test_vault();
        let provider = "chatgpt-concurrent-refresh";
        let now = unix_now();
        store_token_set(provider, &token_set(&jwt_with_exp(now - 10), "refresh-0", now - 10))
            .expect("seed an expiring token set");

        let first = tokio::spawn(fresh_access_token_with(provider, rotating_refresh_grant));
        let second = tokio::spawn(fresh_access_token_with(provider, rotating_refresh_grant));
        let (first, second) = (first.await.unwrap(), second.await.unwrap());

        assert!(
            first.is_ok() && second.is_ok(),
            "a healthy login must survive two concurrent AI calls: {first:?} / {second:?}"
        );
        assert_eq!(
            FAKE_REFRESH_CALLS.load(Ordering::SeqCst),
            1,
            "one refresh per provider instance, not one per caller"
        );
        assert_eq!(
            load_token_set(provider)
                .expect("load")
                .and_then(|set| set.refresh_token),
            Some("refresh-0-rotated".to_string()),
            "the persisted refresh token must be the live one"
        );
        let _ = app_infra::delete_ai_provider_key(provider);
    }

    /// Connect twice ("Start over", or connecting a second ChatGPT account):
    /// the superseded poll must not clobber the token set the newer login
    /// stored — `poll_and_store` persists *before* `begin_login`'s generation
    /// check ever runs.
    #[tokio::test]
    async fn a_superseded_login_cannot_overwrite_the_newer_token_set() {
        install_test_vault();
        let provider = "chatgpt-superseded-login";
        let stale = bump_login_generation(provider);
        let current = bump_login_generation(provider);

        persist_token_set_if_current(provider, current, &token_set("account-two", "r2", 4_000_000_000))
            .expect("the current login persists");
        let _ = persist_token_set_if_current(
            provider,
            stale,
            &token_set("account-one", "r1", 4_000_000_000),
        );

        assert_eq!(
            load_token_set(provider).expect("load").map(|set| set.access_token),
            Some("account-two".to_string()),
            "a superseded login must not overwrite the newer login's token set"
        );
        let _ = app_infra::delete_ai_provider_key(provider);
    }

    /// Disconnect is a revocation. A device poll (or a refresh) still in flight
    /// when the user disconnects must not write the credential back into the
    /// slot that was just cleared.
    #[tokio::test]
    async fn a_disconnect_mid_login_keeps_the_provider_disconnected() {
        install_test_vault();
        let provider = "chatgpt-disconnect-mid-login";
        let generation = bump_login_generation(provider);

        // What disconnect does: cancel in-flight work, then clear the slot.
        cancel_login(provider);
        let _ = app_infra::delete_ai_provider_key(provider);

        let _ = persist_token_set_if_current(
            provider,
            generation,
            &token_set("resurrected", "r", 4_000_000_000),
        );

        assert_eq!(
            load_token_set(provider).expect("load").map(|set| set.access_token),
            None,
            "a disconnected chatgpt provider must stay disconnected"
        );
    }

    /// The poll interval is whatever `auth.openai.com` puts in the response.
    /// `0` (or a junk `0`-ish hint) turns the background poll into a hot loop
    /// hammering the device endpoint for a full 15 minutes; an oversized hint
    /// parks the detached task in one `sleep` far past the 15-minute bound the
    /// loop believes it enforces (the deadline is only checked at the top).
    #[test]
    fn the_device_poll_sleep_is_bounded_on_both_ends() {
        assert_eq!(poll_sleep_seconds(None), DEVICE_CODE_POLL_SLEEP_SECONDS);
        assert_eq!(poll_sleep_seconds(Some(3)), 3);
        assert!(
            poll_sleep_seconds(Some(0)) >= 1,
            "a zero interval must not turn the poll into a hot loop"
        );
        assert!(
            poll_sleep_seconds(Some(86_400)) < DEVICE_CODE_TIMEOUT_SECONDS,
            "one sleep must not outlive the login timeout"
        );
    }

    /// `fresh_access_token` is awaited by every AI call before the engine runs,
    /// and (with single-flight) it holds the provider's refresh lock while it
    /// waits. An endpoint that accepts the connection and then says nothing
    /// must fail the call, not hang every ChatGPT feature forever. Driven at a
    /// test-sized bound; production wires the same builder to
    /// `OAUTH_REQUEST_TIMEOUT_SECONDS`.
    #[tokio::test]
    async fn an_auth_round_trip_gives_up_on_a_stalled_endpoint() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("addr").port();
        std::thread::spawn(move || {
            // Accept and hold: never write a byte of response.
            let mut held = Vec::new();
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => held.push(stream),
                    Err(_) => break,
                }
            }
        });

        let request = http_client_with_timeout(std::time::Duration::from_millis(200))
            .post(format!("http://127.0.0.1:{port}/oauth/token"))
            .body("grant_type=refresh_token")
            .send();
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(3), request).await;
        assert!(
            matches!(outcome, Ok(Err(_))),
            "a stalled auth endpoint must time out the request, not hang the caller"
        );
    }

    #[test]
    fn refresh_keeps_the_previous_refresh_token_when_the_grant_blanks_it() {
        // A blank rotated refresh token is not a rotation: storing it fails the
        // NEXT refresh, so a working login silently degrades into
        // needs_reconnect one access-token lifetime later.
        let rotated = token_set_from_grant(
            OAuthTokenResponse {
                access_token: jwt_with_exp(1_234_567),
                refresh_token: Some(String::new()),
                expires_in: None,
            },
            Some("previous-refresh".to_string()),
        );
        assert_eq!(
            rotated.refresh_token.as_deref(),
            Some("previous-refresh"),
            "a blank rotated refresh token must not replace a working one"
        );
    }

    #[test]
    fn a_non_jwt_access_token_keeps_its_expiry_from_expires_in() {
        // An opaque access token still comes with the grant's `expires_in`.
        // Dropping it makes every later call treat the set as expired: a
        // refresh, a vault rewrite, and another rotation per completion.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let set = token_set_from_grant(
            OAuthTokenResponse {
                access_token: "opaque-access-token".into(),
                refresh_token: Some("refresh-1".into()),
                expires_in: Some(3600),
            },
            Some("refresh-0".into()),
        );
        let expires_at = set
            .expires_at
            .expect("the grant's expires_in must survive into the stored set");
        assert!((expires_at - (now + 3600)).abs() <= 5, "expected ~now+3600, got {expires_at}");
        assert!(
            !set.expires_within_skew(),
            "a token good for an hour must not force a refresh on the very next call"
        );
    }

    #[test]
    fn device_code_response_tolerates_a_junk_interval() {
        // `interval` is an optional poll hint with a default; a shape drift in
        // it must not fail the response that carries the user code. This
        // endpoint already drifted once (string vs number).
        for body in [
            r#"{"device_auth_id":"d","user_code":"u","interval":5.0}"#,
            r#"{"device_auth_id":"d","user_code":"u","interval":-1}"#,
            r#"{"device_auth_id":"d","user_code":"u","interval":null}"#,
            r#"{"device_auth_id":"d","user_code":"u","interval":true}"#,
        ] {
            let parsed: DeviceCodeResponse = serde_json::from_str(body)
                .unwrap_or_else(|e| panic!("a junk interval hint must not fail the login: {body} ({e})"));
            assert_eq!(parsed.user_code, "u");
        }
    }

    /// Script a device-token endpoint: each call answers the next entry.
    fn scripted_poll(
        responses: Vec<(u16, &'static str)>,
    ) -> (
        impl Fn() -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<(reqwest::StatusCode, String), String>> + Send>,
        >,
        Arc<Mutex<usize>>,
    ) {
        let calls = Arc::new(Mutex::new(0usize));
        let seen = calls.clone();
        let poll = move || {
            let responses = responses.clone();
            let seen = seen.clone();
            Box::pin(async move {
                let mut at = seen.lock().expect("poll counter");
                let (status, body) = responses
                    .get(*at)
                    .copied()
                    .unwrap_or_else(|| panic!("poll called {} times, script has {}", *at + 1, responses.len()));
                *at += 1;
                Ok((reqwest::StatusCode::from_u16(status).unwrap(), body.to_string()))
            })
                as std::pin::Pin<
                    Box<dyn std::future::Future<Output = Result<(reqwest::StatusCode, String), String>> + Send>,
                >
        };
        (poll, calls)
    }

    const APPROVED_BODY: &str =
        r#"{"authorization_code":"the-code","code_verifier":"the-verifier"}"#;

    #[tokio::test]
    async fn the_poll_waits_through_not_approved_yet_and_stops_on_approval() {
        // 403/404 is how this endpoint says "the user hasn't clicked yet" —
        // treating either as a failure would abort the login the moment it
        // starts.
        let (poll, calls) = scripted_poll(vec![
            (403, "not yet"),
            (404, "not yet"),
            (200, APPROVED_BODY),
        ]);
        let code = await_authorization(
            poll,
            std::time::Duration::from_millis(1),
            std::time::Duration::from_secs(30),
        )
        .await
        .expect("approval should land");

        assert_eq!(code.authorization_code, "the-code");
        assert_eq!(code.code_verifier, "the-verifier");
        assert_eq!(*calls.lock().unwrap(), 3);
    }

    #[tokio::test]
    async fn the_poll_gives_up_on_any_other_status() {
        // Anything that is not 403/404 is terminal. 429 especially: polling on
        // would deepen the rate limit we just hit.
        let (poll, calls) = scripted_poll(vec![(429, r#"{"error":"slow_down"}"#)]);
        let error = await_authorization(
            poll,
            std::time::Duration::from_millis(1),
            std::time::Duration::from_secs(30),
        )
        .await
        .expect_err("a rate limit must end the login");

        assert!(error.contains("429"), "the status belongs in the message: {error}");
        assert_eq!(*calls.lock().unwrap(), 1, "a terminal status is not retried");
    }

    #[tokio::test]
    async fn the_poll_is_bounded_by_its_deadline() {
        // The user walked away. The wait must end on its own rather than
        // leaving a detached task polling OpenAI forever.
        let (poll, calls) = scripted_poll(vec![(403, "not yet"); 8]);
        let error = await_authorization(
            poll,
            std::time::Duration::from_millis(5),
            std::time::Duration::from_millis(12),
        )
        .await
        .expect_err("an unapproved login must time out");

        assert!(error.contains("timed out"), "{error}");
        assert!(*calls.lock().unwrap() >= 1, "it polled at least once");
    }

    #[test]
    fn a_malformed_approval_is_terminal_not_a_retry() {
        // A 200 whose body we cannot read is not "not approved yet": retrying
        // would spin against a response that will never parse.
        let step = classify_poll_response(reqwest::StatusCode::OK, "{oops");
        assert!(matches!(step, PollStep::Failed(_)), "{step:?}");
    }

    #[test]
    fn a_token_set_tolerates_absent_optional_fields() {
        // The `#[serde(default)]`s are load-bearing: a grant that omits the
        // refresh token, or an access token with no readable `exp`, still has
        // to parse out of the vault slot. And a set with no known expiry must
        // read as expired, so the next use goes through a refresh attempt
        // rather than presenting a token that may already be dead.
        let minimal: ChatgptTokenSet =
            serde_json::from_str(r#"{"access_token":"a"}"#).expect("a partial set must parse");
        assert_eq!(minimal.refresh_token, None);
        assert_eq!(minimal.expires_at, None);
        assert!(
            minimal.expires_within_skew(),
            "an unknown expiry must force a refresh attempt"
        );

        // And the full shape survives the vault round trip it is stored in.
        let set = ChatgptTokenSet {
            access_token: "access".into(),
            refresh_token: Some("refresh".into()),
            expires_at: Some(42),
        };
        let back: ChatgptTokenSet =
            serde_json::from_str(&serde_json::to_string(&set).unwrap()).unwrap();
        assert_eq!(back.access_token, "access");
        assert_eq!(back.refresh_token.as_deref(), Some("refresh"));
        assert_eq!(back.expires_at, Some(42));
    }
}
