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
/// The redirect the device-code grant is registered against. Never navigated
/// to — the exchange just has to echo it back.
const DEVICE_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";

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

/// `pub(crate)` for tests in sibling modules that need a connected provider.
pub(crate) fn store_token_set(provider_id: &str, set: &ChatgptTokenSet) -> Result<(), String> {
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
    fn(String) -> Pin<Box<dyn Future<Output = Result<OAuthTokenResponse, OAuthError>> + Send>>;

fn refresh_grant(
    refresh_token: String,
) -> Pin<Box<dyn Future<Output = Result<OAuthTokenResponse, OAuthError>> + Send>> {
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

/// A rotation that was granted but could not be written to the vault.
///
/// The window is narrow but the damage is total: once the refresh POST returns,
/// OpenAI has *consumed* the old refresh token, so a failed vault write leaves
/// the slot holding a credential that will never be accepted again — a working
/// login silently dead until the user notices and re-runs the whole device
/// flow. This app is a screen recorder with a `LowDisk` capture suspension
/// (ADR 0040), so "the disk filled up mid-write" is a scenario it already
/// expects elsewhere.
///
/// Holding the rotation in memory and re-trying it on the next call turns that
/// into a hiccup. Keyed by provider id; the consumed refresh token rides along
/// so the retry can still compare-and-swap (a disconnect or a newer login in
/// the meantime must still win).
///
/// ponytail: process-global and lost on quit — the same blast radius as the
/// failed write itself. Persisting it would mean writing the secret to the
/// disk that just refused one.
static PENDING_ROTATIONS: Mutex<Option<HashMap<String, (String, ChatgptTokenSet)>>> =
    Mutex::new(None);

fn stash_pending_rotation(provider_id: &str, consumed_refresh_token: &str, set: &ChatgptTokenSet) {
    let mut map = PENDING_ROTATIONS.lock().expect("pending rotations lock");
    map.get_or_insert_with(HashMap::new).insert(
        provider_id.to_string(),
        (consumed_refresh_token.to_string(), set.clone()),
    );
}

fn take_pending_rotation(provider_id: &str) -> Option<(String, ChatgptTokenSet)> {
    let mut map = PENDING_ROTATIONS.lock().expect("pending rotations lock");
    map.as_mut().and_then(|m| m.remove(provider_id))
}

/// Re-try a rotation an earlier call granted but could not persist. Runs under
/// the provider's refresh lock, so it cannot race another refresh; a `false`
/// compare-and-swap means a disconnect or a newer login owns the slot now and
/// the stale rotation is simply dropped.
fn recover_pending_rotation(provider_id: &str) {
    let Some((consumed, set)) = take_pending_rotation(provider_id) else {
        return;
    };
    match persist_refreshed_token_set(provider_id, &consumed, &set) {
        Ok(true) => tauri_plugin_log::log::info!(
            "chatgpt-auth: recovered a rotation for {provider_id} that an earlier write lost"
        ),
        Ok(false) => {}
        Err(error) => {
            // Still failing: keep holding it rather than throwing the only copy
            // of a live credential away.
            tauri_plugin_log::log::warn!(
                "chatgpt-auth: re-persisting a held rotation for {provider_id} failed: {error}"
            );
            stash_pending_rotation(provider_id, &consumed, &set);
        }
    }
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
    {
        // Under the lock, before the re-read: an earlier call may hold a
        // rotation whose vault write failed. Landing it now is what keeps a
        // full disk from costing the user their login.
        let id = provider_id.to_string();
        tokio::task::spawn_blocking(move || recover_pending_rotation(&id))
            .await
            .map_err(|e| e.to_string())?;
    }
    let set = load().await?;
    if !set.expires_within_skew() {
        return Ok(set.access_token);
    }

    let refresh_token = set.refresh_token.clone().ok_or_else(needs_reconnect)?;
    let tokens = refresh(refresh_token.clone()).await.map_err(|error| {
        tauri_plugin_log::log::warn!(
            "chatgpt-auth: token refresh failed for {provider_id}: {}",
            error.message
        );
        // Being offline is not being signed out. Telling a user with a healthy
        // login to re-run the device flow is the worst possible advice: the
        // obvious next step is Disconnect, which destroys the credential that
        // was fine all along. Same reasoning as ADR 0048 for cloud
        // transcription — connectivity failures are transient liveness, not a
        // terminal auth verdict.
        if error.transient {
            format!("provider_unreachable:{provider_id}")
        } else {
            needs_reconnect()
        }
    })?;

    // The refresh grant may omit (or blank out) a rotated refresh token; keep
    // the old one in both cases.
    let rotated = token_set_from_grant(tokens, Some(refresh_token.clone()));
    let id = provider_id.to_string();
    let persisted = rotated.clone();
    let consumed = refresh_token.clone();
    let stored = tokio::task::spawn_blocking(move || {
        persist_refreshed_token_set(&id, &refresh_token, &persisted)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|error| {
        tauri_plugin_log::log::warn!("chatgpt-auth: persisting refreshed token set failed: {error}");
        // OpenAI has already consumed the old refresh token, so the slot now
        // holds a dead credential. Hold the rotation in memory and hand the
        // caller a transient code: the next call retries the write, and the
        // user is not told to re-login over a disk hiccup.
        stash_pending_rotation(provider_id, &consumed, &rotated);
        format!("provider_unreachable:{provider_id}")
    })?;

    if !stored {
        // A disconnect or a newer login owns the slot now, so this rotated set
        // is not the credential any more — and handing it back would run the
        // call against the account the user just left. Whatever is in the slot
        // is the truth: use it when it is usable, reconnect when it is gone.
        let current = load().await?;
        if current.expires_within_skew() {
            return Err(needs_reconnect());
        }
        return Ok(current.access_token);
    }

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

/// A failed `oauth/token` round trip, split by whether re-running the whole
/// device-code login is actually the fix.
#[derive(Debug)]
pub struct OAuthError {
    /// The endpoint never rendered a verdict on the credential — no route to
    /// the host, a timeout, a 5xx, a rate limit. The stored token set is
    /// untouched and the same call will likely succeed later.
    pub transient: bool,
    pub message: String,
}

impl OAuthError {
    fn transport(error: impl std::fmt::Display) -> Self {
        Self { transient: true, message: error.to_string() }
    }
}

/// Did OpenAI actually reject the *grant*, or just fail to answer?
///
/// Mirrors rig 0.41's `should_reauthenticate_after_refresh`: only a 400/401
/// carrying `invalid_grant` means the refresh token is spent and the user has
/// to log in again. A 429 or any 5xx is the server declining to answer, and
/// treating those as "signed out" is what turns a rate limit into a lost login.
fn refresh_rejection_is_terminal(status: reqwest::StatusCode, body: &str) -> bool {
    matches!(status.as_u16(), 400 | 401) && body.contains("invalid_grant")
}

/// POST an `application/x-www-form-urlencoded` grant to `oauth/token`.
async fn oauth_token_request(form: &[(&str, &str)]) -> Result<OAuthTokenResponse, OAuthError> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(form)
        .finish();
    let response = oauth_http_client()
        .post(OAUTH_TOKEN_URL)
        .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(OAuthError::transport)?;
    let status = response.status();
    let body = response.text().await.map_err(OAuthError::transport)?;
    if !status.is_success() {
        return Err(OAuthError {
            transient: !refresh_rejection_is_terminal(status, &body),
            message: format!("token request failed with status {status}: {}", body.trim()),
        });
    }
    // A 200 whose body will not parse is not a credential verdict either.
    serde_json::from_str(&body).map_err(OAuthError::transport)
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

/// Poll until the user approves, the endpoint refuses, `is_current` goes false,
/// or `timeout` elapses.
///
/// `is_current` is the cancellation path for the detached task `begin_login`
/// spawns: clicking Connect again (or disconnecting) only bumps the login
/// generation, and the generation is otherwise not consulted until the poll has
/// already returned — so without this check every click leaves another loop
/// hitting the device endpoint for the full 15 minutes, and a non-403/404
/// status (429) from that pile-up is terminal for the login the user is
/// actually waiting on.
///
/// The HTTP call is injected so the loop's decisions — 403/404 keeps polling,
/// anything else stops, and the deadline actually bounds the wait — are
/// testable without a network or a 15-minute test.
async fn await_authorization<C, P, Fut>(
    is_current: C,
    poll: P,
    interval: std::time::Duration,
    timeout: std::time::Duration,
) -> Result<DeviceTokenResponse, String>
where
    C: Fn() -> bool,
    P: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<(reqwest::StatusCode, String), String>>,
{
    let start = std::time::Instant::now();
    loop {
        if !is_current() {
            return Err("the ChatGPT login was superseded".to_string());
        }
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
/// The code-for-token exchange, injected for the same reason [`RefreshCall`]
/// is: it is the only network hop between "the user approved" and "the vault
/// holds a token set", so without a seam the whole login path is untestable.
type ExchangeCall = fn(
    DeviceTokenResponse,
) -> Pin<Box<dyn Future<Output = Result<OAuthTokenResponse, OAuthError>> + Send>>;

fn exchange_authorization_code(
    code: DeviceTokenResponse,
) -> Pin<Box<dyn Future<Output = Result<OAuthTokenResponse, OAuthError>> + Send>> {
    Box::pin(async move {
        oauth_token_request(&[
            ("grant_type", "authorization_code"),
            ("code", code.authorization_code.as_str()),
            ("redirect_uri", DEVICE_REDIRECT_URI),
            ("client_id", CHATGPT_CLIENT_ID),
            ("code_verifier", code.code_verifier.as_str()),
        ])
        .await
    })
}

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

    poll_and_store_with(
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
        exchange_authorization_code,
        provider_id,
        generation,
        std::time::Duration::from_secs(poll_sleep_seconds(device.interval)),
        std::time::Duration::from_secs(DEVICE_CODE_TIMEOUT_SECONDS),
    )
    .await
}

/// Wait for approval, exchange the code, persist the set — the whole second
/// half of the device flow, with both network hops injected so it is testable
/// as one piece. Without a seam here the login path can only ever be tested in
/// fragments, which is exactly where a wiring bug hides: whether the captured
/// `generation` is the one the persist is guarded by, and whether what the
/// exchange returned is what lands in the vault.
async fn poll_and_store_with<P, Fut>(
    poll: P,
    exchange: ExchangeCall,
    provider_id: &str,
    generation: u64,
    interval: std::time::Duration,
    timeout: std::time::Duration,
) -> Result<(), String>
where
    P: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<(reqwest::StatusCode, String), String>>,
{
    let code = await_authorization(
        || login_generation_is_current(provider_id, generation),
        poll,
        interval,
        timeout,
    )
    .await?;

    let tokens = exchange(code).await.map_err(|error| error.message)?;

    let set = token_set_from_grant(tokens, None);
    let id = provider_id.to_string();
    tokio::task::spawn_blocking(move || persist_token_set_if_current(&id, generation, &set))
        .await
        .map_err(|e| e.to_string())?
}

/// Persist a token set produced by the login of `generation` — but only while
/// that login is still the current one for this provider.
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

/// Persist a *refreshed* token set — guarded by the credential it rotated, not
/// by a login generation.
///
/// A generation guard is the wrong instrument here in both directions. It is
/// too coarse: a login that merely *starts* mid-refresh bumps the generation
/// without touching the slot, and dropping the write then strands the refresh
/// token OpenAI has already consumed. And it is too loose: a refresh that
/// starts *after* a login began captures that login's own generation, so the
/// old account's rotated tokens overwrite the set the finished login just
/// stored — the user completes a login and stays silently on the previous
/// account.
///
/// What actually matters is whether the slot still holds the credential this
/// grant rotated. Disconnect (slot cleared) and a newer login (slot rewritten)
/// both fail that compare-and-swap; a login still in flight does not.
///
/// `Ok(false)` means the write was dropped — the credential moved under us.
fn persist_refreshed_token_set(
    provider_id: &str,
    consumed_refresh_token: &str,
    set: &ChatgptTokenSet,
) -> Result<bool, String> {
    // Held across the read-compare-write so a login's own check-then-write
    // cannot interleave into it.
    let _map = LOGIN_GENERATIONS.lock().expect("login generations lock");
    let holds_it = matches!(
        load_token_set(provider_id),
        Ok(Some(stored)) if stored.refresh_token.as_deref() == Some(consumed_refresh_token)
    );
    if !holds_it {
        tauri_plugin_log::log::info!(
            "chatgpt-auth: dropping a refreshed token set for {provider_id}: the stored credential changed under it"
        );
        return Ok(false);
    }
    store_token_set(provider_id, set).map(|_| true)
}

/// Abandon an in-flight device login without touching the stored credential.
///
/// The detached poll checks the login generation at the top of every iteration,
/// so bumping it is what actually stops the loop (rather than leaving it to run
/// out its 15-minute deadline against `auth.openai.com`). `begin_login` also
/// gates its outcome event on the same generation, so a cancelled login emits
/// nothing — no late toast contradicting a UI the user already dismissed.
///
/// Distinct from [`revoke_provider_credential`] on purpose: cancelling a
/// *re*-login must leave the existing sign-in working.
pub fn cancel_login(provider_id: &str) {
    bump_login_generation(provider_id);
}

/// Revoke a provider's stored credential: invalidate any in-flight login poll
/// or token refresh, then clear the vault slot. Called by disconnect
/// (`ai_runtime_clear_provider_key`).
///
/// The bump and the delete happen under ONE hold of the generation lock — the
/// same lock [`persist_refreshed_token_set`] holds across its compare-and-swap.
/// That is what makes the two mutually exclusive: a refresh landing
/// concurrently either completes its read-compare-write *before* the delete
/// (and the delete then clears it), or finds an empty slot and drops its write.
/// The interleaving the shared lock forbids is the resurrection — the CAS reads
/// the old token, the delete lands, the CAS writes the rotated set back, and a
/// provider the user just disconnected is connected again with a live OAuth
/// token set. Bumping the generation without also holding it across the delete
/// reopens exactly that window.
pub fn revoke_provider_credential(provider_id: &str) -> Result<(), String> {
    let mut map = LOGIN_GENERATIONS.lock().expect("login generations lock");
    let generation = LOGIN_GENERATION_COUNTER.fetch_add(1, Ordering::SeqCst);
    map.get_or_insert_with(HashMap::new)
        .insert(provider_id.to_string(), generation);
    app_infra::delete_ai_provider_key(provider_id).map_err(|error| error.to_string())
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
    ) -> Pin<Box<dyn Future<Output = Result<OAuthTokenResponse, OAuthError>> + Send>> {
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
                return Err(OAuthError {
                    transient: false,
                    message:
                        "token request failed with status 400 Bad Request: {\"error\":\"invalid_grant\"}"
                            .to_string(),
                });
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

    /// A newer login being *started* is not a revocation. Clicking Connect on an
    /// already-connected provider (to check the account, or to "start over")
    /// while an AI call is refreshing must not strand the consumed refresh
    /// token in the vault: the refresh POST has already rotated the credential
    /// at OpenAI, so dropping the write leaves the slot holding a refresh token
    /// OpenAI will never accept again — the login dies silently on the next
    /// call.
    static STRAND_CONSUMED_REFRESH_TOKENS: Mutex<Option<std::collections::HashSet<String>>> =
        Mutex::new(None);

    fn refresh_grant_with_a_login_starting_mid_flight(
        refresh_token: String,
    ) -> Pin<Box<dyn Future<Output = Result<OAuthTokenResponse, OAuthError>> + Send>> {
        Box::pin(async move {
            // The user clicks Connect while this POST is in flight.
            bump_login_generation("chatgpt-login-during-refresh");
            let mut consumed = STRAND_CONSUMED_REFRESH_TOKENS
                .lock()
                .expect("consumed refresh tokens");
            let consumed = consumed.get_or_insert_with(std::collections::HashSet::new);
            if !consumed.insert(refresh_token.clone()) {
                return Err(OAuthError {
                    transient: false,
                    message:
                        "token request failed with status 400 Bad Request: {\"error\":\"invalid_grant\"}"
                            .to_string(),
                });
            }
            Ok(OAuthTokenResponse {
                // Short-lived on purpose: the *next* AI call must go through a
                // refresh, which is where a stranded refresh token shows up.
                access_token: jwt_with_exp(unix_now() + 10),
                refresh_token: Some(format!("{refresh_token}-rotated")),
                expires_in: None,
            })
        })
    }

    #[tokio::test]
    async fn a_login_starting_mid_refresh_does_not_strand_a_consumed_refresh_token() {
        install_test_vault();
        let provider = "chatgpt-login-during-refresh";
        let now = unix_now();
        store_token_set(
            provider,
            &token_set(&jwt_with_exp(now - 10), "strand-refresh-0", now - 10),
        )
        .expect("seed an expiring token set");

        let first =
            fresh_access_token_with(provider, refresh_grant_with_a_login_starting_mid_flight).await;
        assert!(first.is_ok(), "the refresh itself succeeds: {first:?}");
        assert_eq!(
            load_token_set(provider)
                .expect("load")
                .and_then(|set| set.refresh_token),
            Some("strand-refresh-0-rotated".to_string()),
            "the rotated refresh token must reach the vault: the previous one is consumed"
        );

        let second =
            fresh_access_token_with(provider, refresh_grant_with_a_login_starting_mid_flight).await;
        assert!(
            second.is_ok(),
            "a working login must survive a Connect click landing mid-refresh: {second:?}"
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

        // What disconnect does: cancel in-flight work and clear the slot.
        let _ = revoke_provider_credential(provider);

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

    fn approved_exchange(
        code: DeviceTokenResponse,
    ) -> Pin<Box<dyn Future<Output = Result<OAuthTokenResponse, OAuthError>> + Send>> {
        Box::pin(async move {
            // Echo the code back through the access token so the test can prove
            // the vault holds what THIS exchange returned, not a fixture.
            Ok(OAuthTokenResponse {
                access_token: format!("access-for-{}", code.authorization_code),
                refresh_token: Some(format!("refresh-for-{}", code.code_verifier)),
                expires_in: Some(3600),
            })
        })
    }

    fn refusing_exchange(
        _code: DeviceTokenResponse,
    ) -> Pin<Box<dyn Future<Output = Result<OAuthTokenResponse, OAuthError>> + Send>> {
        Box::pin(async move {
            Err(OAuthError {
                transient: false,
                message: "token request failed with status 400: bad code".to_string(),
            })
        })
    }

    /// The second half of the device flow as one piece: wait through
    /// "not approved yet", exchange the approved code, persist the set.
    ///
    /// The parts were each tested in isolation; the WIRING was not — and the
    /// wiring is where a login silently stores nothing.
    #[tokio::test]
    async fn an_approved_device_login_exchanges_its_code_and_persists_the_token_set() {
        install_test_vault();
        let provider = "chatgpt-login-end-to-end";
        let _ = app_infra::delete_ai_provider_key(provider);
        let generation = bump_login_generation(provider);

        let (poll, calls) = scripted_poll(vec![(403, "not yet"), (200, APPROVED_BODY)]);
        poll_and_store_with(
            poll,
            approved_exchange,
            provider,
            generation,
            std::time::Duration::from_millis(1),
            std::time::Duration::from_secs(30),
        )
        .await
        .expect("an approved login stores its token set");

        assert_eq!(*calls.lock().unwrap(), 2, "it waited through the pending poll");
        let stored = load_token_set(provider).expect("load").expect("connected");
        assert_eq!(stored.access_token, "access-for-the-code");
        assert_eq!(stored.refresh_token.as_deref(), Some("refresh-for-the-verifier"));
        assert!(
            !stored.expires_within_skew(),
            "the grant's expires_in must survive into the stored set"
        );
        let _ = app_infra::delete_ai_provider_key(provider);
    }

    /// A login the user superseded (Connect again, or Disconnect) must stop at
    /// its next poll and persist NOTHING. `begin_login` suppresses the outcome
    /// event for a superseded generation, so this error never reaches the UI —
    /// it just ends the detached task.
    #[tokio::test]
    async fn a_superseded_login_persists_nothing_even_after_approval() {
        install_test_vault();
        let provider = "chatgpt-login-superseded-persist";
        let _ = app_infra::delete_ai_provider_key(provider);
        let stale = bump_login_generation(provider);
        // The user clicked Connect again while the first login was polling.
        bump_login_generation(provider);

        // Scripted but never consumed: the cancellation is checked before the
        // poll, so a superseded login does not even ask the endpoint again.
        let (poll, calls) = scripted_poll(vec![(200, APPROVED_BODY)]);
        let outcome = poll_and_store_with(
            poll,
            approved_exchange,
            provider,
            stale,
            std::time::Duration::from_millis(1),
            std::time::Duration::from_secs(30),
        )
        .await;

        assert!(outcome.is_err(), "a superseded login is not an approval");
        assert_eq!(*calls.lock().unwrap(), 0, "it stopped before polling again");
        assert_eq!(
            load_token_set(provider).expect("load").map(|set| set.access_token),
            None,
            "a superseded login must not write the vault slot"
        );
    }

    /// A refused exchange fails the login rather than persisting a partial set.
    #[tokio::test]
    async fn a_refused_code_exchange_stores_nothing() {
        install_test_vault();
        let provider = "chatgpt-login-exchange-refused";
        let _ = app_infra::delete_ai_provider_key(provider);
        let generation = bump_login_generation(provider);

        let (poll, _calls) = scripted_poll(vec![(200, APPROVED_BODY)]);
        let outcome = poll_and_store_with(
            poll,
            refusing_exchange,
            provider,
            generation,
            std::time::Duration::from_millis(1),
            std::time::Duration::from_secs(30),
        )
        .await;

        assert!(outcome.is_err(), "a refused exchange is a failed login");
        assert_eq!(load_token_set(provider).expect("load").map(|s| s.access_token), None);
    }

    /// The two shapes that cross the Tauri boundary. Tauri events are untyped
    /// and `bun run check` cannot see across the wire, so a dropped
    /// `rename_all` would break the connect UI with a green build. Three
    /// separately-declared TS interfaces read these keys: `LoginPrompt` and
    /// `LoginUpdate` in ChatgptConnect.svelte, plus the inline shapes in
    /// AiSetup.svelte and Providers.svelte.
    #[test]
    fn the_login_wire_shapes_keep_their_camel_case_keys() {
        let prompt = serde_json::to_value(ChatgptLoginPrompt {
            user_code: "ABCD-1234".to_string(),
            verify_url: DEVICE_VERIFY_URL.to_string(),
        })
        .expect("serialize");
        assert_eq!(prompt["userCode"], "ABCD-1234");
        assert_eq!(prompt["verifyUrl"], DEVICE_VERIFY_URL);

        let failed = serde_json::to_value(ChatgptLoginUpdate {
            provider_id: "chatgpt".to_string(),
            connected: false,
            error: Some("nope".to_string()),
        })
        .expect("serialize");
        assert_eq!(failed["providerId"], "chatgpt");
        assert_eq!(failed["connected"], false);
        assert_eq!(failed["error"], "nope");

        // `skip_serializing_if` is load-bearing: the TS field is optional.
        let ok = serde_json::to_value(ChatgptLoginUpdate {
            provider_id: "chatgpt".to_string(),
            connected: true,
            error: None,
        })
        .expect("serialize");
        assert!(ok.get("error").is_none(), "a success carries no error key: {ok}");
    }

    /// Disconnect must be mutually exclusive with the refresh's
    /// compare-and-swap, not merely ordered before it.
    ///
    /// `persist_refreshed_token_set` holds the generation lock across its
    /// read-compare-write. If revocation only bumped the generation and then
    /// deleted the slot *outside* that lock, a delete landing between the CAS's
    /// read and its write would be undone: the read still sees the credential,
    /// the delete clears it, the write puts the rotated set back — and a
    /// provider the user just disconnected is connected again, holding a live
    /// OAuth access + refresh token.
    ///
    /// Holding the lock here stands in for a refresh mid-CAS: the revocation
    /// must wait, not slip past it.
    #[test]
    fn a_revocation_cannot_land_while_a_refresh_holds_the_credential_lock() {
        install_test_vault();
        let provider = "chatgpt-revoke-under-the-cas-lock";
        store_token_set(provider, &token_set("access", "refresh", 4_000_000_000))
            .expect("seed a connected provider");

        // The shape this guards against: disconnect used to bump the generation
        // on the command thread and only then hand the vault delete to a
        // blocking task, so by the time the delete ran the bump was long done
        // and nothing left in the revocation touched this lock at all.
        bump_login_generation(provider);

        let held = LOGIN_GENERATIONS.lock().expect("login generations lock");
        let id = provider.to_string();
        let revoking = std::thread::spawn(move || revoke_provider_credential(&id));
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(
            load_token_set(provider).expect("load").is_some(),
            "a revocation must contend with an in-flight refresh's compare-and-swap, \
             not delete the slot beside it"
        );

        drop(held);
        revoking.join().expect("revoke thread").expect("revoke");
        assert_eq!(
            load_token_set(provider).expect("load").map(|set| set.access_token),
            None,
            "once it does run, the revocation clears the slot"
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

        // The production wiring, not just the builder: `oauth_http_client` must
        // keep passing a real bound, or every `auth.openai.com` round trip can
        // hang the refresh lock forever.
        assert!(
            (1..=60).contains(&OAUTH_REQUEST_TIMEOUT_SECONDS),
            "the auth round-trip bound must stay a real, sub-minute timeout"
        );

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

    /// Being offline is not being signed out. `needs_reconnect` renders as
    /// "sign in with ChatGPT again", and the obvious next step for a user who
    /// believes that is Disconnect — which destroys a credential that was
    /// healthy all along. So only OpenAI actually rejecting the grant may
    /// produce it; everything else is transient liveness (ADR 0048's rule for
    /// cloud transcription, same reasoning).
    #[test]
    fn only_a_rejected_grant_is_terminal_the_rest_is_transient() {
        use reqwest::StatusCode;
        // The one verdict that means the refresh token is spent.
        assert!(refresh_rejection_is_terminal(
            StatusCode::BAD_REQUEST,
            r#"{"error":"invalid_grant"}"#
        ));
        assert!(refresh_rejection_is_terminal(
            StatusCode::UNAUTHORIZED,
            r#"{"error":"invalid_grant"}"#
        ));

        // The server declining to answer says nothing about the credential.
        assert!(!refresh_rejection_is_terminal(
            StatusCode::TOO_MANY_REQUESTS,
            r#"{"error":"rate_limit"}"#
        ));
        assert!(!refresh_rejection_is_terminal(
            StatusCode::INTERNAL_SERVER_ERROR,
            "upstream boom"
        ));
        assert!(!refresh_rejection_is_terminal(
            StatusCode::BAD_GATEWAY,
            r#"{"error":"invalid_grant"}"#
        ));
        // A 400 that is not about the grant (a malformed request, a changed
        // parameter contract) must not sign the user out either.
        assert!(!refresh_rejection_is_terminal(
            StatusCode::BAD_REQUEST,
            r#"{"error":"invalid_request"}"#
        ));
        // Transport failures never reach the classifier at all.
        assert!(OAuthError::transport("dns failure").transient);
    }

    fn unreachable_grant(
        _refresh_token: String,
    ) -> Pin<Box<dyn Future<Output = Result<OAuthTokenResponse, OAuthError>> + Send>> {
        Box::pin(async move { Err(OAuthError::transport("error sending request for url")) })
    }

    fn rejecting_grant(
        _refresh_token: String,
    ) -> Pin<Box<dyn Future<Output = Result<OAuthTokenResponse, OAuthError>> + Send>> {
        Box::pin(async move {
            Err(OAuthError {
                transient: false,
                message: "token request failed with status 400: invalid_grant".to_string(),
            })
        })
    }

    /// The two failure kinds must reach the caller as two different reason
    /// codes, because the UI copy for each is different advice.
    #[tokio::test]
    async fn a_refresh_failure_surfaces_as_reconnect_only_when_the_grant_was_rejected() {
        install_test_vault();
        let now = unix_now();

        let rejected = "chatgpt-grant-rejected";
        store_token_set(rejected, &token_set(&jwt_with_exp(now - 10), "r", now - 10))
            .expect("seed");
        assert_eq!(
            fresh_access_token_with(rejected, rejecting_grant).await,
            Err(format!("needs_reconnect:{rejected}")),
            "a spent refresh token is the one case that really does need a new login"
        );
        // The dead set stays put: nothing here should clear the slot behind the
        // user's back.
        assert!(load_token_set(rejected).expect("load").is_some());
        let _ = app_infra::delete_ai_provider_key(rejected);

        let offline = "chatgpt-grant-unreachable";
        store_token_set(offline, &token_set(&jwt_with_exp(now - 10), "r", now - 10))
            .expect("seed");
        assert_eq!(
            fresh_access_token_with(offline, unreachable_grant).await,
            Err(format!("provider_unreachable:{offline}")),
            "an unreachable endpoint must not be reported as a signed-out account"
        );
        let _ = app_infra::delete_ai_provider_key(offline);
    }

    /// The rotation OpenAI granted but the vault refused to store.
    ///
    /// By the time the write is attempted the old refresh token is already
    /// consumed, so dropping the rotation leaves the slot holding a credential
    /// that will never be accepted again — a working login silently dead. The
    /// held copy has to land on the next call.
    #[test]
    fn a_rotation_whose_write_failed_is_recovered_on_the_next_call() {
        install_test_vault();
        let provider = "chatgpt-rotation-recovery";
        let now = unix_now();
        // The vault still holds the CONSUMED set: this is the state a failed
        // write leaves behind.
        store_token_set(
            provider,
            &token_set(&jwt_with_exp(now - 10), "consumed-refresh", now - 10),
        )
        .expect("seed the consumed set");
        stash_pending_rotation(
            provider,
            "consumed-refresh",
            &token_set(&jwt_with_exp(now + 3600), "rotated-refresh", now + 3600),
        );

        recover_pending_rotation(provider);

        let stored = load_token_set(provider).expect("load").expect("still connected");
        assert_eq!(
            stored.refresh_token.as_deref(),
            Some("rotated-refresh"),
            "the held rotation must land, or the login is dead"
        );
        assert!(!stored.expires_within_skew());
        // One-shot: a second recovery has nothing left to do and must not
        // resurrect anything.
        recover_pending_rotation(provider);
        assert_eq!(
            load_token_set(provider).expect("load").and_then(|s| s.refresh_token),
            Some("rotated-refresh".to_string())
        );
        let _ = app_infra::delete_ai_provider_key(provider);
    }

    /// …but the held copy is still just a rotation of a credential the user may
    /// have revoked in the meantime. Recovery goes through the same
    /// compare-and-swap, so a disconnect still wins.
    #[test]
    fn a_held_rotation_cannot_resurrect_a_disconnected_provider() {
        install_test_vault();
        let provider = "chatgpt-rotation-recovery-after-disconnect";
        let now = unix_now();
        stash_pending_rotation(
            provider,
            "consumed-refresh",
            &token_set(&jwt_with_exp(now + 3600), "rotated-refresh", now + 3600),
        );
        let _ = revoke_provider_credential(provider);

        recover_pending_rotation(provider);

        assert_eq!(
            load_token_set(provider).expect("load").map(|s| s.access_token),
            None,
            "a disconnected provider must stay disconnected"
        );
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
            || true,
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
            || true,
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
            || true,
            poll,
            std::time::Duration::from_millis(5),
            std::time::Duration::from_millis(12),
        )
        .await
        .expect_err("an unapproved login must time out");

        assert!(error.contains("timed out"), "{error}");
        assert!(*calls.lock().unwrap() >= 1, "it polled at least once");
    }

    /// "Start over" — or a disconnect — while a device login is still polling.
    /// `begin_login` spawns a DETACHED task per click and only checks the login
    /// generation once the poll has already returned, so the superseded loop
    /// keeps hitting `auth.openai.com` for the full 15-minute deadline. N clicks
    /// leave N loops polling the device endpoint at once, and this module treats
    /// anything but 403/404 as terminal — so the rate limit they earn kills the
    /// login the user is actually waiting on.
    #[tokio::test]
    async fn a_superseded_login_stops_polling_the_device_endpoint() {
        let provider = "chatgpt-superseded-poll-loop";
        let generation = bump_login_generation(provider);
        let polls = Arc::new(AtomicU64::new(0));
        let seen = polls.clone();
        let poll = move || {
            let seen = seen.clone();
            async move {
                // On the first poll the user clicks Connect again: a newer
                // login for this provider supersedes this one.
                if seen.fetch_add(1, Ordering::SeqCst) == 0 {
                    bump_login_generation("chatgpt-superseded-poll-loop");
                }
                Ok((reqwest::StatusCode::FORBIDDEN, "not yet".to_string()))
            }
        };

        let outcome = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            await_authorization(
                || login_generation_is_current(provider, generation),
                poll,
                std::time::Duration::from_millis(1),
                std::time::Duration::from_secs(DEVICE_CODE_TIMEOUT_SECONDS),
            ),
        )
        .await
        .expect("a superseded login must stop, not poll on to its 15-minute deadline");

        assert!(
            outcome.is_err(),
            "a superseded login is not an approval: {outcome:?}"
        );
        assert!(
            polls.load(Ordering::SeqCst) <= 2,
            "the stale loop must stop within a poll, not keep hammering the device endpoint: {} polls",
            polls.load(Ordering::SeqCst)
        );
    }

    #[test]
    fn a_malformed_approval_is_terminal_not_a_retry() {
        // A 200 whose body we cannot read is not "not approved yet": retrying
        // would spin against a response that will never parse.
        let step = classify_poll_response(reqwest::StatusCode::OK, "{oops");
        assert!(matches!(step, PollStep::Failed(_)), "{step:?}");
    }

    /// A refresh whose round trip is slow enough to still be in flight while
    /// something else rewrites the slot.
    fn slow_rotating_grant(
        refresh_token: String,
    ) -> Pin<Box<dyn Future<Output = Result<OAuthTokenResponse, OAuthError>> + Send>> {
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            Ok(OAuthTokenResponse {
                access_token: jwt_with_exp(unix_now() + 3600),
                refresh_token: Some(format!("{refresh_token}-rotated")),
                expires_in: None,
            })
        })
    }

    /// Connect a (different) ChatGPT account while an ordinary AI call is
    /// refreshing the old one. The refresh captured the *same* generation the
    /// login runs under — the login had already bumped it before the refresh
    /// started — so the generation guard cannot tell them apart, and the
    /// refresh's write lands last. The user finishes a login and is silently
    /// left on the previous account.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_login_that_lands_mid_refresh_is_not_clobbered_by_the_old_account() {
        install_test_vault();
        let provider = "chatgpt-new-login-during-refresh";
        let now = unix_now();
        store_token_set(
            provider,
            &token_set(&jwt_with_exp(now - 10), "old-account-refresh", now - 10),
        )
        .expect("seed the old account's expiring token set");

        // The user clicked Connect: the login owns the current generation.
        let login = bump_login_generation(provider);
        // ...and an AI call meets the expiring old token while they approve.
        let refreshing = tokio::spawn(fresh_access_token_with(provider, slow_rotating_grant));
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        // The user approves: the login persists the new account's token set.
        persist_token_set_if_current(
            provider,
            login,
            &token_set("new-account-access", "new-account-refresh", now + 3600),
        )
        .expect("the login persists");
        let _ = refreshing.await.expect("the refresh task should not panic");

        let stored = load_token_set(provider)
            .expect("load")
            .expect("the provider stays connected");
        assert_eq!(
            stored.refresh_token.as_deref(),
            Some("new-account-refresh"),
            "the account the user just connected must own the slot"
        );
        assert_eq!(stored.access_token, "new-account-access");
        let _ = app_infra::delete_ai_provider_key(provider);
    }

    /// Disconnect is a revocation, and it is documented as invalidating an
    /// in-flight refresh too. A refresh that finishes after the disconnect has
    /// its write dropped — but it must not hand the caller a live access token
    /// for the account the user just disconnected either.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_disconnect_mid_refresh_fails_the_call_it_was_refreshing_for() {
        install_test_vault();
        let provider = "chatgpt-disconnect-mid-refresh";
        let now = unix_now();
        store_token_set(
            provider,
            &token_set(&jwt_with_exp(now - 10), "refresh-0", now - 10),
        )
        .expect("seed an expiring token set");

        let refreshing = tokio::spawn(fresh_access_token_with(provider, slow_rotating_grant));
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        // What disconnect does, while the refresh round trip is in flight.
        let _ = revoke_provider_credential(provider);

        assert_eq!(
            refreshing.await.expect("the refresh task should not panic"),
            Err(format!("needs_reconnect:{provider}")),
            "a disconnected provider must not answer with a usable access token"
        );
        assert_eq!(
            load_token_set(provider).expect("load").map(|set| set.access_token),
            None,
            "a disconnected chatgpt provider must stay disconnected"
        );
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

    /// Every `resolve_engine_config_live` on an expiring token set is one
    /// network round trip to `auth.openai.com`, serialized behind the
    /// per-provider refresh lock and bounded only by
    /// [`OAUTH_REQUEST_TIMEOUT_SECONDS`] (30s). Nothing memoizes a *failed*
    /// refresh, so a provider whose refresh cannot succeed right now (laptop
    /// offline, endpoint flaky) pays that round trip again on every single
    /// resolve.
    ///
    /// That is the cost `get_or_generate_digest` used to put in FRONT of its
    /// step-4 fingerprint cache hit — the path documented as "an unchanged
    /// input set never re-bills the engine", and the one the Insights Day
    /// Timeline re-invokes on EVERY `user_context_changed` worker beat
    /// (`DayTimeline.svelte:310`). Counting round trips rather than timing
    /// keeps this deterministic in CI.
    static BEAT_REFRESH_ROUND_TRIPS: AtomicU64 = AtomicU64::new(0);

    fn counting_unreachable_grant(
        _refresh_token: String,
    ) -> Pin<Box<dyn Future<Output = Result<OAuthTokenResponse, OAuthError>> + Send>> {
        BEAT_REFRESH_ROUND_TRIPS.fetch_add(1, Ordering::SeqCst);
        // What reqwest hands back with no route to the host.
        Box::pin(async move {
            Err(OAuthError::transport(
                "error sending request for url (…auth.openai.com…)",
            ))
        })
    }

    #[tokio::test]
    async fn each_live_resolve_of_an_expiring_token_is_another_round_trip() {
        install_test_vault();
        let provider = "chatgpt-per-beat-round-trip";
        let now = unix_now();
        store_token_set(
            provider,
            &token_set(&jwt_with_exp(now - 10), "beat-refresh-0", now - 10),
        )
        .expect("seed an expiring token set");

        BEAT_REFRESH_ROUND_TRIPS.store(0, Ordering::SeqCst);
        // Five worker beats' worth of digest reads.
        for _ in 0..5 {
            assert_eq!(
                fresh_access_token_with(provider, counting_unreachable_grant).await,
                Err(format!("provider_unreachable:{provider}")),
                "an unreachable refresh endpoint fails the resolve — transiently, \
                 not as a signed-out verdict"
            );
        }

        assert_eq!(
            BEAT_REFRESH_ROUND_TRIPS.load(Ordering::SeqCst),
            5,
            "every live resolve pays its own auth.openai.com round trip — so any \
             read path that resolves the live engine BEFORE its cache check pays \
             one per invocation"
        );
        let _ = app_infra::delete_ai_provider_key(provider);
    }
}
