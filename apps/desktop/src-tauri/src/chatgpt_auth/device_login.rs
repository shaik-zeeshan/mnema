//! The device-code login half of ADR 0058's ChatGPT OAuth: fetching a user
//! code, polling `auth.openai.com` until the user approves, exchanging the
//! authorization code for a token set, and persisting it. The per-AI-call
//! refresh half stays in the parent module.

use super::*;
use tauri::Emitter;

#[derive(Debug, Deserialize)]
pub(super) struct DeviceCodeResponse {
    pub(super) device_auth_id: String,
    #[serde(alias = "usercode")]
    pub(super) user_code: String,
    #[serde(default, deserialize_with = "deserialize_optional_u64")]
    pub(super) interval: Option<u64>,
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

#[derive(Debug, Clone, Deserialize)]
pub(super) struct DeviceTokenResponse {
    pub(super) authorization_code: String,
    pub(super) code_verifier: String,
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
pub(super) struct ChatgptLoginUpdate {
    pub(super) provider_id: String,
    pub(super) connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) error: Option<String>,
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
    // Reserve this login's generation BEFORE the device-code round trip, not
    // after. That request takes up to 30s, and Cancel / Disconnect are both
    // reachable while it is in flight — each bumps the generation to invalidate
    // whatever login is running. Bumping afterwards overwrites their bump, so
    // the detached poll below considers itself current: it runs its full 15
    // minutes and, on approval, writes a token set into the slot the user just
    // cleared. Reserve first, then confirm nothing superseded us.
    let generation = bump_login_generation(&provider_id);
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

    // A Cancel or Disconnect that landed while the request above was in flight
    // already bumped past our reservation. Do not start the poll for a login
    // the user has walked away from.
    if !login_generation_is_current(&provider_id, generation) {
        return Err("the ChatGPT sign-in was cancelled".to_string());
    }

    let prompt = ChatgptLoginPrompt {
        user_code: device.user_code.clone(),
        verify_url: DEVICE_VERIFY_URL.to_string(),
    };

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
pub(super) fn poll_sleep_seconds(interval: Option<u64>) -> u64 {
    interval
        .unwrap_or(DEVICE_CODE_POLL_SLEEP_SECONDS)
        .clamp(1, 60)
}

/// What one device-token poll response means. Split out from the loop so the
/// endpoint's three-way answer is decided in one pure place.
#[derive(Debug)]
pub(super) enum PollStep {
    /// The user approved: the response carries the authorization code.
    Approved(Box<DeviceTokenResponse>),
    /// Not approved yet — sleep and ask again.
    Pending,
    /// Terminal failure; the login is over.
    Failed(String),
}

pub(super) fn classify_poll_response(status: reqwest::StatusCode, text: &str) -> PollStep {
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
pub(super) async fn await_authorization<C, P, Fut>(
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
    let mut dropped_in_a_row = 0u32;
    loop {
        if !is_current() {
            return Err("the ChatGPT login was superseded".to_string());
        }
        if start.elapsed() >= timeout {
            return Err("timed out waiting for ChatGPT device authorization".to_string());
        }
        let (status, text) = match poll().await {
            Ok(answer) => {
                dropped_in_a_row = 0;
                answer
            }
            // A request that never reached the endpoint is not a verdict on the
            // login: the code is still on screen and the user is still typing
            // it. A 15-minute wait is ~180 sends, so one WiFi roam, DNS hiccup
            // or sleep/wake would otherwise end a login that was fine. Retry a
            // few times — the deadline and `is_current` still bound the loop —
            // but a network that is simply down still ends it rather than
            // leaving the code UI up against an endpoint that will never
            // answer.
            Err(error) => {
                dropped_in_a_row += 1;
                if dropped_in_a_row >= DEVICE_POLL_DROPPED_REQUEST_LIMIT {
                    return Err(error);
                }
                sleep_until_next_poll(start, interval, timeout).await?;
                continue;
            }
        };
        match classify_poll_response(status, &text) {
            PollStep::Approved(token) => return Ok(*token),
            PollStep::Failed(error) => return Err(error),
            PollStep::Pending => sleep_until_next_poll(start, interval, timeout).await?,
        }
    }
}

/// Wait out one poll interval, or give up when the deadline is already spent.
/// The deadline is only checked between polls, so a long interval must never
/// park the task past it.
async fn sleep_until_next_poll(
    start: std::time::Instant,
    interval: std::time::Duration,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let left = timeout.saturating_sub(start.elapsed());
    if left.is_zero() {
        return Err("timed out waiting for ChatGPT device authorization".to_string());
    }
    tokio::time::sleep(interval.min(left)).await;
    Ok(())
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

/// How many times the approved code may be presented at `oauth/token`.
/// Only the attempts that never rendered a verdict count — a refusal stops on
/// the first answer.
const CODE_EXCHANGE_ATTEMPTS: u32 = 3;

/// Exchange the approved code, retrying the answers that were not answers.
///
/// This is the one hop with no second chance: by the time it runs the user has
/// already approved in the browser and the code UI is gone, so a dropped
/// connection here costs them the whole device flow — surfaced as a raw
/// `error sending request for url (…)` in `ChatgptConnect.svelte` and as a
/// Finish-blocking `status: "error"` in onboarding. The module already splits
/// "OpenAI rejected the grant" from "OpenAI never answered" for the refresh
/// (`OAuthError::transient`); the exchange threw that split away.
///
/// Retrying is safe in the direction that matters: if the lost request *was*
/// processed, the code is spent and the retry comes back `invalid_grant` —
/// terminal, so the loop stops on the real verdict rather than hiding it.
async fn exchange_with_retry(
    exchange: ExchangeCall,
    code: DeviceTokenResponse,
    backoff: std::time::Duration,
) -> Result<OAuthTokenResponse, String> {
    for attempt in 1..CODE_EXCHANGE_ATTEMPTS {
        match exchange(code.clone()).await {
            Ok(tokens) => return Ok(tokens),
            Err(error) if error.transient => {
                tauri_plugin_log::log::warn!(
                    "chatgpt-auth: the code exchange did not reach OpenAI (attempt {attempt}): {}",
                    error.message
                );
                tokio::time::sleep(backoff).await;
            }
            Err(error) => return Err(error.message),
        }
    }
    exchange(code).await.map_err(|error| error.message)
}

pub(super) async fn poll_and_store(
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
pub(super) async fn poll_and_store_with<P, Fut>(
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

    let tokens = exchange_with_retry(exchange, code, interval).await?;

    let set = token_set_from_grant(tokens, None);
    // A set with no refresh token cannot be refreshed: it dies at the first
    // expiry as `needs_reconnect`. Storing it is worse than failing, because
    // Connect is reachable while already connected — the doomed set would
    // overwrite a working credential whose refresh token is then gone for good.
    if set.refresh_token.is_none() {
        return Err("the ChatGPT sign-in returned no refresh token".to_string());
    }
    let id = provider_id.to_string();
    tokio::task::spawn_blocking(move || persist_token_set_if_current(&id, generation, &set))
        .await
        .map_err(|e| e.to_string())?
}

/// Persist a token set produced by the login of `generation` — but only while
/// that login is still the current one for this provider.
pub(super) fn persist_token_set_if_current(
    provider_id: &str,
    generation: u64,
    set: &ChatgptTokenSet,
) -> Result<(), String> {
    // The generation lock is held across the vault write, so a concurrent
    // disconnect either bumps first (we skip) or clears afterwards — it can
    // never interleave into "cleared, then written back".
    let map = LOGIN_GENERATIONS.lock().unwrap_or_else(PoisonError::into_inner);
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
