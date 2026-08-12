//! ChatGPT-subscription OAuth for the `chatgpt` provider kind (ADR pending).
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
//! Endpooints, client id, and poll semantics mirror rig 0.41's implementation
//! (which mirrors the Codex CLI): device-code poll answers 403/404 while the
//! user has not approved yet, and the refresh grant preserves the previous
//! refresh token when the response omits a rotated one. There is no unattended
//! login anywhere: a token set that cannot be refreshed surfaces as a
//! `needs_reconnect:<id>` reason code, never as a device-code prompt.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

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

/// The models the ChatGPT subscription backend serves. There is no discovery
/// endpoint on this backend, so the picker list is static — owned here (not
/// re-exported from rig) so updating it never waits on a rig release. The
/// backend gates some ids by plan tier (e.g. `gpt-5.4-pro` needs Pro); a
/// mismatch surfaces as a provider error at call time.
pub const CHATGPT_MODEL_IDS: &[&str] = &[
    "gpt-5.4",
    "gpt-5.4-pro",
    "gpt-5.3-codex",
    "gpt-5.3-codex-spark",
    "gpt-5.3-instant",
    "gpt-5.3-chat-latest",
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
    let needs_reconnect = || format!("needs_reconnect:{provider_id}");

    let id = provider_id.to_string();
    let set = tokio::task::spawn_blocking(move || load_token_set(&id))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|error| {
            tauri_plugin_log::log::warn!("chatgpt-auth: loading token set failed: {error}");
            needs_reconnect()
        })?
        .ok_or_else(needs_reconnect)?;

    if !set.expires_within_skew() {
        return Ok(set.access_token);
    }

    let refresh_token = set.refresh_token.clone().ok_or_else(needs_reconnect)?;
    let tokens = oauth_token_request(&[
        ("client_id", CHATGPT_CLIENT_ID),
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token.as_str()),
        ("scope", "openid profile email"),
    ])
    .await
    .map_err(|error| {
        tauri_plugin_log::log::warn!("chatgpt-auth: token refresh failed for {provider_id}: {error}");
        needs_reconnect()
    })?;

    let rotated = ChatgptTokenSet {
        expires_at: jwt_expiration_seconds(&tokens.access_token),
        access_token: tokens.access_token,
        // The refresh grant may omit a rotated refresh token; keep the old one.
        refresh_token: tokens.refresh_token.or(Some(refresh_token)),
    };
    let id = provider_id.to_string();
    let persisted = rotated.clone();
    tokio::task::spawn_blocking(move || store_token_set(&id, &persisted))
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
}

/// POST an `application/x-www-form-urlencoded` grant to `oauth/token`.
async fn oauth_token_request(form: &[(&str, &str)]) -> Result<OAuthTokenResponse, String> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(form)
        .finish();
    let response = reqwest::Client::new()
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
    #[serde(default)]
    interval: Option<u64>,
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
    let client = reqwest::Client::new();
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
        let outcome = poll_and_store(&client, &device, &provider_id).await;
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

/// The background half of the device flow: poll for user approval, exchange
/// the authorization code for tokens, persist the set.
async fn poll_and_store(
    client: &reqwest::Client,
    device: &DeviceCodeResponse,
    provider_id: &str,
) -> Result<(), String> {
    let interval = device.interval.unwrap_or(DEVICE_CODE_POLL_SLEEP_SECONDS);
    let poll_body = serde_json::json!({
        "device_auth_id": device.device_auth_id,
        "user_code": device.user_code,
    })
    .to_string();

    let start = std::time::Instant::now();
    let code = loop {
        if start.elapsed().as_secs() >= DEVICE_CODE_TIMEOUT_SECONDS {
            return Err("timed out waiting for ChatGPT device authorization".to_string());
        }

        let response = client
            .post(DEVICE_TOKEN_URL)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(poll_body.clone())
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let status = response.status();
        let text = response.text().await.map_err(|e| e.to_string())?;

        if status.is_success() {
            let token: DeviceTokenResponse =
                serde_json::from_str(&text).map_err(|e| e.to_string())?;
            break token;
        }
        // 403/404 mean "not approved yet" on this endpoint; keep polling.
        if status.as_u16() == 403 || status.as_u16() == 404 {
            tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
            continue;
        }
        return Err(format!(
            "device authorization failed with status {status}: {}",
            text.trim()
        ));
    };

    let redirect_uri = "https://auth.openai.com/deviceauth/callback";
    let tokens = oauth_token_request(&[
        ("grant_type", "authorization_code"),
        ("code", code.authorization_code.as_str()),
        ("redirect_uri", redirect_uri),
        ("client_id", CHATGPT_CLIENT_ID),
        ("code_verifier", code.code_verifier.as_str()),
    ])
    .await?;

    let set = ChatgptTokenSet {
        expires_at: jwt_expiration_seconds(&tokens.access_token),
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
    };
    let id = provider_id.to_string();
    tokio::task::spawn_blocking(move || store_token_set(&id, &set))
        .await
        .map_err(|e| e.to_string())?
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
    fn token_set_round_trips_through_json() {
        let set = ChatgptTokenSet {
            access_token: "access".into(),
            refresh_token: Some("refresh".into()),
            expires_at: Some(42),
        };
        let raw = serde_json::to_string(&set).unwrap();
        let back: ChatgptTokenSet = serde_json::from_str(&raw).unwrap();
        assert_eq!(back.access_token, "access");
        assert_eq!(back.refresh_token.as_deref(), Some("refresh"));
        assert_eq!(back.expires_at, Some(42));
    }
}
