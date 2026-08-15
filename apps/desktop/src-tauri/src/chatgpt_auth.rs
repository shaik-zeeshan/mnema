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
use std::sync::{Arc, Mutex, PoisonError};

use base64::Engine as _;
use serde::{Deserialize, Serialize};

mod device_login;

pub use device_login::{begin_login, cancel_login, ChatgptLoginPrompt};

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
/// How many device-code polls in a row may fail to reach `auth.openai.com`
/// before the login gives up. A dropped connection says nothing about the
/// login, but an endpoint that never answers must not hold the code UI open
/// for the full deadline either.
const DEVICE_POLL_DROPPED_REQUEST_LIMIT: u32 = 3;
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
/// provider was never connected.
///
/// The two failures are NOT the same verdict and must not share an error, so
/// the split lives here rather than in every caller: unparseable slot CONTENT
/// is a broken credential (`needs_reconnect:<id>` — signing in again is the
/// fix), while a vault that refused to answer says nothing at all about the
/// credential inside it and rides back as its own `AppInfraError` Display, the
/// same string every pasted-key kind surfaces ("denied ≠ missing", the ADR 0048
/// amendment). Telling a user with a healthy login to reconnect over a denied
/// keychain prompt invites Disconnect, which destroys the access AND long-lived
/// refresh token the vault merely declined to hand over.
pub fn load_token_set(provider_id: &str) -> Result<Option<ChatgptTokenSet>, String> {
    let Some(raw) = app_infra::load_ai_provider_key(provider_id).map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    if raw.trim().is_empty() {
        return Ok(None);
    }
    serde_json::from_str(&raw).map(Some).map_err(|error| {
        tauri_plugin_log::log::warn!(
            "chatgpt-auth: the stored token set for {provider_id} is unreadable: {error}"
        );
        format!("needs_reconnect:{provider_id}")
    })
}

/// Test-only: force the vault write to fail for ONE provider id, standing in
/// for the disk that refuses a rotation (ADR 0040's `LowDisk` world). Keyed by
/// provider so it cannot leak into another test sharing this binary's vault.
#[cfg(test)]
static WEDGED_VAULT_PROVIDER: Mutex<Option<String>> = Mutex::new(None);

/// `pub(crate)` for tests in sibling modules that need a connected provider.
pub(crate) fn store_token_set(provider_id: &str, set: &ChatgptTokenSet) -> Result<(), String> {
    #[cfg(test)]
    if WEDGED_VAULT_PROVIDER
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_deref()
        == Some(provider_id)
    {
        return Err("no space left on device".to_string());
    }
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
    let mut map = PENDING_ROTATIONS.lock().unwrap_or_else(PoisonError::into_inner);
    map.get_or_insert_with(HashMap::new).insert(
        provider_id.to_string(),
        (consumed_refresh_token.to_string(), set.clone()),
    );
}

fn take_pending_rotation(provider_id: &str) -> Option<(String, ChatgptTokenSet)> {
    let mut map = PENDING_ROTATIONS.lock().unwrap_or_else(PoisonError::into_inner);
    map.as_mut().and_then(|m| m.remove(provider_id))
}

/// Re-try a rotation an earlier call granted but could not persist. Runs under
/// the provider's refresh lock, so it cannot race another refresh; a `false`
/// compare-and-swap means a disconnect or a newer login owns the slot now and
/// the stale rotation is simply dropped.
///
/// Returns the held set when the write failed *again* — the caller must use it
/// rather than the slot, because the slot's refresh token is one OpenAI has
/// already consumed. `None` means the slot is the truth (landed, or dropped by
/// the compare-and-swap, or nothing was held).
fn recover_pending_rotation(provider_id: &str) -> Option<ChatgptTokenSet> {
    let (consumed, set) = take_pending_rotation(provider_id)?;
    match persist_refreshed_token_set(provider_id, &consumed, &set) {
        Ok(true) => {
            tauri_plugin_log::log::info!(
                "chatgpt-auth: recovered a rotation for {provider_id} that an earlier write lost"
            );
            None
        }
        Ok(false) => None,
        Err(error) => {
            // Still failing: keep holding it rather than throwing the only copy
            // of a live credential away.
            tauri_plugin_log::log::warn!(
                "chatgpt-auth: re-persisting a held rotation for {provider_id} failed: {error}"
            );
            stash_pending_rotation(provider_id, &consumed, &set);
            Some(set)
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
    let mut map = REFRESH_LOCKS.lock().unwrap_or_else(PoisonError::into_inner);
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
            .inspect_err(|error| {
                tauri_plugin_log::log::warn!("chatgpt-auth: loading token set failed: {error}");
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
    // Under the lock, before the re-read: an earlier call may hold a rotation
    // whose vault write failed. Landing it now is what keeps a full disk from
    // costing the user their login.
    let id = provider_id.to_string();
    let still_held = tokio::task::spawn_blocking(move || recover_pending_rotation(&id))
        .await
        .map_err(|e| e.to_string())?;
    if let Some(held) = still_held {
        // The write failed a second time, so the slot still holds the token set
        // whose refresh token OpenAI consumed granting this one. Falling
        // through would replay it, earn a real `invalid_grant`, and tell a user
        // whose only problem is a full disk to reconnect a healthy account.
        // The rotation we are holding IS the live credential.
        if !held.expires_within_skew() {
            return Ok(held.access_token);
        }
        // Held long enough to expire and the vault is still refusing writes:
        // there is no refresh we can run whose result we could store (the
        // compare-and-swap keys off a slot that is now stale). Transient, so
        // the user is not sent to Disconnect over a disk problem.
        return Err(format!("provider_unreachable:{provider_id}"));
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

/// Monotonic generation per provider id, bumped by every `begin_login`, so a
/// superseded poll (user clicked Connect again) exits silently instead of
/// racing the newer one's outcome event.
static LOGIN_GENERATIONS: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);
static LOGIN_GENERATION_COUNTER: AtomicU64 = AtomicU64::new(1);

fn bump_login_generation(provider_id: &str) -> u64 {
    let generation = LOGIN_GENERATION_COUNTER.fetch_add(1, Ordering::SeqCst);
    let mut map = LOGIN_GENERATIONS.lock().unwrap_or_else(PoisonError::into_inner);
    map.get_or_insert_with(HashMap::new)
        .insert(provider_id.to_string(), generation);
    generation
}

fn login_generation_is_current(provider_id: &str, generation: u64) -> bool {
    let map = LOGIN_GENERATIONS.lock().unwrap_or_else(PoisonError::into_inner);
    map.as_ref().and_then(|m| m.get(provider_id)) == Some(&generation)
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
    let _map = LOGIN_GENERATIONS.lock().unwrap_or_else(PoisonError::into_inner);
    let holds_it = match load_token_set(provider_id) {
        Ok(Some(stored)) => stored.refresh_token.as_deref() == Some(consumed_refresh_token),
        Ok(None) => false,
        // A slot we could not READ answers neither yes nor no, and "no" is the
        // destructive answer: OpenAI has already consumed the refresh token
        // this set rotated, so dropping the write throws the only live copy of
        // the credential away. Fail instead — the callers hold the rotation and
        // retry it on the next call.
        Err(error) => return Err(error),
    };
    if !holds_it {
        tauri_plugin_log::log::info!(
            "chatgpt-auth: dropping a refreshed token set for {provider_id}: the stored credential changed under it"
        );
        return Ok(false);
    }
    store_token_set(provider_id, set).map(|_| true)
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
    let mut map = LOGIN_GENERATIONS.lock().unwrap_or_else(PoisonError::into_inner);
    let generation = LOGIN_GENERATION_COUNTER.fetch_add(1, Ordering::SeqCst);
    map.get_or_insert_with(HashMap::new)
        .insert(provider_id.to_string(), generation);
    // The held rotation is a live access + refresh token for the account being
    // revoked, kept alive by the one path whose job is writing it back.
    let _ = take_pending_rotation(provider_id);
    app_infra::delete_ai_provider_key(provider_id).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests;
