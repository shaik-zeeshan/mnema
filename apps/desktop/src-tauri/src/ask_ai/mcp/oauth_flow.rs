//! The MCP **OAuth flow** for HTTP OAuth connectors (ADR 0051) — the authorize/
//! callback/disconnect/revoke/status lifecycle, split out of [`super::manager`]
//! so that file stays the connection/slot manager.
//!
//! The [`McpManager`] state lives in `manager`; this module reaches into its
//! `Inner` (the pending-flow map, the reconnect-needed set, the slot cache) via
//! `pub(super)` visibility and hangs the OAuth methods off `McpManager` as
//! additional inherent-impl blocks. Nothing here connects a server — it only
//! mints/claims browser flows and drops the cached slot so the next turn redials
//! with the fresh token.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use capture_types::{McpAuthMode, McpServerConfig, McpTransport};
use rmcp::transport::auth::{AuthorizationManager, StoredCredentials};
use tauri::{Emitter, Manager};

use super::manager::{lock_recover, Inner, McpManager};
use super::transport::secret_may_ride_url;
use super::OAuthCredentialStore;

/// How long a browser OAuth flow may sit unclaimed in [`Inner::oauth_pending`]
/// before it is swept: the user opened the connector's authorize page but never
/// finished. Only bounds memory for abandoned flows — a live callback matches by
/// its CSRF `state` long before this. Swept lazily on the next `begin_oauth`.
const OAUTH_PENDING_TTL: Duration = Duration::from_secs(300);

/// The client name Mnema registers itself under via RFC 7591 Dynamic Client
/// Registration (shown on the provider's consent screen).
const OAUTH_CLIENT_NAME: &str = "Mnema";

/// Frontend event fired whenever a connector's authorization state MAY have
/// changed (authorized, denied, exchange-failed, or disconnected). Carries only
/// `{ id }`; Settings (slice 6) re-reads the connector's real status on receipt.
pub(crate) const MCP_AUTHORIZATION_CHANGED_EVENT: &str = "mcp_authorization_changed";

/// The registered OAuth redirect URI, keyed off the active build's deep-link
/// scheme (prod `mnema`, dev `mnema-dev`).
// ponytail: cfg!(debug_assertions) tracks the dev build, whose tauri.dev.conf.json
// registers the `mnema-dev` scheme — one branch, no config plumbing.
fn oauth_redirect_uri() -> &'static str {
    if cfg!(debug_assertions) {
        "mnema-dev://oauth/callback"
    } else {
        "mnema://oauth/callback"
    }
}

/// One in-flight browser OAuth authorization, parked between `begin_oauth` opening
/// the browser and the deep-link callback returning with the code. The `manager`
/// is the very one that minted this flow's PKCE verifier + CSRF token, so the
/// callback MUST exchange the code on IT (a fresh manager would not hold the
/// verifier the token endpoint requires).
pub(super) struct PendingOAuth {
    manager: Arc<AuthorizationManager>,
    pub(super) connector_id: String,
    pub(super) created_at: Instant,
}

/// Drop pending-OAuth entries older than `ttl` relative to `now`. Generic over the
/// value (with a `created_at` projection) so the sweep rule is unit-testable
/// offline — a real [`AuthorizationManager`] needs the network to construct.
fn evict_expired<V>(
    map: &mut HashMap<String, V>,
    created_at: impl Fn(&V) -> Instant,
    now: Instant,
    ttl: Duration,
) {
    map.retain(|_state, entry| now.saturating_duration_since(created_at(entry)) <= ttl);
}

impl McpManager {
    /// Begin (or re-begin) the browser OAuth flow for one HTTP OAuth connector.
    /// Backs both "Connect" and "Reconnect" in Settings — a reconnect is simply a
    /// fresh authorize, so there is deliberately no separate reconnect path (ADR
    /// 0051): reach the server, run RFC 7591 Dynamic Client Registration, open the
    /// system browser at the authorization URL, and park a [`PendingOAuth`] the
    /// deep-link callback claims by its CSRF `state`. Every failure surfaces as
    /// readable text for the Settings surface; nothing is persisted until the
    /// callback exchanges the code.
    pub(crate) async fn begin_oauth(
        &self,
        app_handle: &tauri::AppHandle,
        id: &str,
    ) -> Result<(), String> {
        // Resolve from the FULL settings list, not just enabled: authorizing is a
        // Settings action that must work independent of the enabled toggle.
        let cfg = super::super::read_ai_runtime_settings(app_handle)
            .mcp_servers
            .into_iter()
            .find(|cfg| cfg.id == id)
            .ok_or_else(|| format!("MCP connector \"{id}\" was not found in Settings"))?;

        if cfg.transport != McpTransport::Http {
            return Err(format!(
                "connector \"{}\" is not an HTTP connector — OAuth applies only to HTTP transports",
                cfg.label
            ));
        }
        if cfg.auth_mode != McpAuthMode::OAuth {
            return Err(format!(
                "connector \"{}\" is not configured for OAuth (set its auth mode to OAuth first)",
                cfg.label
            ));
        }
        let url = cfg
            .url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .ok_or_else(|| format!("oauth connector \"{}\" has no url", cfg.label))?;

        // Discovery, registration, token exchange, and refresh ALL ride this base
        // URL, so gate it through the same TLS-or-loopback guard the bearer secret
        // gets — refuse to run the OAuth flow against a cleartext remote endpoint.
        if !secret_may_ride_url(url) {
            return Err(format!(
                "oauth connector \"{}\" has a non-HTTPS (and non-loopback) URL; refusing to run \
                 the OAuth flow in cleartext — use an https:// endpoint",
                cfg.label
            ));
        }

        let mut manager = AuthorizationManager::new(url)
            .await
            .map_err(|error| format!("failed to reach \"{}\": {error}", cfg.label))?;
        manager.set_credential_store(OAuthCredentialStore::new(cfg.id.clone()));
        let md = manager.discover_metadata().await.map_err(|error| {
            format!("\"{}\" did not advertise OAuth support: {error}", cfg.label)
        })?;

        // Read everything off `&md` BEFORE `set_metadata` moves it.
        let scopes: Vec<String> = md.scopes_supported.clone().unwrap_or_default();
        let has_registration = md.registration_endpoint.is_some();
        manager.set_metadata(md);

        if !has_registration {
            // ponytail: the manual-`client_id` escape hatch (AuthorizationManager::
            // configure_client_id) belongs HERE for a real connector that lacks DCR
            // — deferred per ADR 0051 §2, so surface a readable error for now.
            return Err(format!(
                "\"{}\" does not support automatic app registration (DCR); a manual client id is \
                 not yet supported for this connector",
                cfg.label
            ));
        }

        let scope_refs: Vec<&str> = scopes.iter().map(String::as_str).collect();
        manager
            .register_client(OAUTH_CLIENT_NAME, oauth_redirect_uri(), &scope_refs)
            .await
            .map_err(|error| format!("could not register with \"{}\": {error}", cfg.label))?;
        let auth_url = manager
            .get_authorization_url(&scope_refs)
            .await
            .map_err(|error| {
                format!("could not start authorization for \"{}\": {error}", cfg.label)
            })?;

        // The CSRF token rmcp saved in its (in-memory) state store IS the `state`
        // query param of the returned URL; it keys the pending map so the callback
        // exchanges the code on THIS manager.
        let state = url::Url::parse(&auth_url)
            .ok()
            .and_then(|parsed| {
                parsed
                    .query_pairs()
                    .find(|(key, _)| key == "state")
                    .map(|(_, value)| value.into_owned())
            })
            .ok_or_else(|| {
                format!("authorization URL for \"{}\" carried no state parameter", cfg.label)
            })?;

        {
            use tauri_plugin_opener::OpenerExt;
            app_handle
                .opener()
                .open_url(auth_url, None::<String>)
                .map_err(|error| format!("could not open your browser: {error}"))?;
        }

        {
            let mut pending = lock_recover(&self.inner().oauth_pending);
            evict_expired(
                &mut pending,
                |entry| entry.created_at,
                Instant::now(),
                OAUTH_PENDING_TTL,
            );
            pending.insert(
                state,
                PendingOAuth {
                    manager: Arc::new(manager),
                    connector_id: id.to_string(),
                    created_at: Instant::now(),
                },
            );
        }
        // Starting a fresh authorize clears any stale *Needs reconnect* flag.
        lock_recover(&self.inner().oauth_reconnect_needed).remove(id);
        tauri_plugin_log::log::info!(
            "Ask AI MCP OAuth authorization began for \"{}\"",
            cfg.label
        );
        Ok(())
    }

    /// Complete a browser OAuth flow from the deep-link callback (`mnema://oauth/
    /// callback?...`). Synchronous entry — called straight from `on_open_url` — that
    /// parses the callback params and spawns the async token exchange. Claims the
    /// parked [`PendingOAuth`] by its CSRF `state`; on success `exchange_code_for_
    /// token` AUTO-PERSISTS the Token Set into this connector's keychain slot (rmcp
    /// auth.rs:1602 — we save nothing by hand). Emits [`MCP_AUTHORIZATION_CHANGED_
    /// EVENT`] on every terminal outcome so the Settings row leaves its authorizing
    /// state whether it landed Authorized, denied, or exchange-failed.
    pub(crate) fn complete_oauth_callback(&self, app_handle: &tauri::AppHandle, url: &url::Url) {
        let mut code = None;
        let mut state = None;
        let mut error = None;
        for (key, value) in url.query_pairs() {
            match key.as_ref() {
                "code" => code = Some(value.into_owned()),
                "state" => state = Some(value.into_owned()),
                "error" => error = Some(value.into_owned()),
                _ => {}
            }
        }

        let manager = self.clone();
        let app_handle = app_handle.clone();
        tauri::async_runtime::spawn(async move {
            let Some(state) = state else {
                tauri_plugin_log::log::info!(
                    "Ask AI MCP OAuth callback carried no state parameter; ignoring"
                );
                return;
            };
            // Claim the pending flow by its CSRF state (one-shot removal).
            let pending = { lock_recover(&manager.inner().oauth_pending).remove(&state) };
            let Some(pending) = pending else {
                tauri_plugin_log::log::info!(
                    "Ask AI MCP received an unknown or expired OAuth callback; ignoring"
                );
                return;
            };
            let id = pending.connector_id.clone();

            // Denial (user clicked "Deny") or a malformed callback: no token was
            // issued. Leave the authorizing state (emit) but persist nothing.
            if error.is_some() || code.is_none() {
                tauri_plugin_log::log::info!(
                    "Ask AI MCP OAuth authorization for \"{id}\" was denied or returned no code"
                );
                emit_authorization_changed(&app_handle, &id);
                return;
            }
            let code = code.unwrap_or_default();

            match pending.manager.exchange_code_for_token(&code, &state).await {
                Ok(_token) => {
                    // The Token Set is now in the keychain (auto-persisted). Clear
                    // the reconnect flag and drop any cached (pre-auth, failing)
                    // slot so the next turn dials with the fresh token.
                    lock_recover(&manager.inner().oauth_reconnect_needed).remove(&id);
                    manager.inner().slots.lock().await.remove(&id);
                    tauri_plugin_log::log::info!(
                        "Ask AI MCP OAuth authorization completed for \"{id}\""
                    );
                    emit_authorization_changed(&app_handle, &id);
                }
                Err(error) => {
                    tauri_plugin_log::log::warn!(
                        "Ask AI MCP OAuth token exchange failed for \"{id}\": {error}"
                    );
                    emit_authorization_changed(&app_handle, &id);
                }
            }
        });
    }

    /// Disconnect an OAuth connector: best-effort server-side revoke, then ALWAYS
    /// drop locally (revocation NEVER blocks teardown — a token we can no longer
    /// use must not linger). Returns the connector to *Needs authorization*; its
    /// config row stays in Settings. `Ok` even if the revoke failed or was skipped.
    pub(crate) async fn disconnect_oauth(
        &self,
        app_handle: &tauri::AppHandle,
        id: &str,
    ) -> Result<(), String> {
        // Best-effort revoke needs the URL from the (still-present) config; a
        // missing config just skips the courtesy revoke — the local drop still runs.
        let cfg = super::super::read_ai_runtime_settings(app_handle)
            .mcp_servers
            .into_iter()
            .find(|cfg| cfg.id == id);
        if let Some(cfg) = cfg.as_ref() {
            best_effort_revoke(cfg).await;
        }
        drop_oauth_local(self.inner(), id).await;
        emit_authorization_changed(app_handle, id);
        Ok(())
    }
}

/// Emit [`MCP_AUTHORIZATION_CHANGED_EVENT`] for one connector id. Fire-and-forget
/// (a failed emit only means no window is listening); Settings re-reads the real
/// status on receipt, so the payload is just the id.
fn emit_authorization_changed(app_handle: &tauri::AppHandle, id: &str) {
    let _ = app_handle.emit(MCP_AUTHORIZATION_CHANGED_EVENT, serde_json::json!({ "id": id }));
}

/// The unconditional LOCAL half of disconnect: forget the Token Set and every
/// cached trace of this connector's authorization (reconnect flag, any pending
/// flow, the cached connection slot). Runs regardless of whether the server-side
/// revoke succeeded, was reachable, or was even attempted. AppHandle-free so a
/// test can drive it against the file-backed secret store.
async fn drop_oauth_local(inner: &Inner, id: &str) {
    if let Err(error) = app_infra::delete_mcp_server_secret(id) {
        tauri_plugin_log::log::warn!(
            "Ask AI MCP failed to delete the OAuth Token Set for \"{id}\": {error}"
        );
    }
    lock_recover(&inner.oauth_reconnect_needed).remove(id);
    lock_recover(&inner.oauth_pending).retain(|_state, entry| entry.connector_id != id);
    inner.slots.lock().await.remove(id);
}

/// Best-effort RFC 7009 token revocation. rmcp exposes no revoke helper, so this
/// hand-rolls it: build a throwaway [`AuthorizationManager`] only to DISCOVER the
/// revocation endpoint, read the stored Token Set, and POST the token form. EVERY
/// failure — no endpoint advertised, discovery error, no stored token, dead
/// network — is swallowed at debug: revocation is a courtesy to the server, never
/// a gate on local teardown (the caller drops the token locally regardless). No
/// token material is logged.
async fn best_effort_revoke(cfg: &McpServerConfig) {
    let Some(url) = cfg
        .url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
    else {
        return;
    };
    if !secret_may_ride_url(url) {
        return;
    }
    let Ok(manager) = AuthorizationManager::new(url).await else {
        return;
    };
    let Ok(md) = manager.discover_metadata().await else {
        return;
    };
    // RFC 8414 advertises the revocation endpoint as a metadata field, which rmcp
    // parks in `additional_fields` (it is not a named field on the struct).
    let Some(endpoint) = md
        .additional_fields
        .get("revocation_endpoint")
        .and_then(serde_json::Value::as_str)
    else {
        return;
    };
    if !secret_may_ride_url(endpoint) {
        return;
    }
    let Ok(Some(json)) = app_infra::load_mcp_server_secret(&cfg.id) else {
        return;
    };
    let Ok(creds) = serde_json::from_str::<StoredCredentials>(&json) else {
        return;
    };
    let Some(token) = revocable_token(&creds) else {
        return;
    };
    // RFC 7009: revoke the refresh token (kills the whole grant) with the type
    // hint; a ~5 s cap so a dead endpoint never stalls the Disconnect action.
    // ponytail: hand-encode the form body via the already-present `url` crate —
    // reqwest13's `.form()` is behind a feature we don't enable, and this is one
    // line either way (no new dep/feature just to percent-encode two pairs).
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("token", &token)
        .append_pair("token_type_hint", "refresh_token")
        .finish();
    let outcome = reqwest13::Client::new()
        .post(endpoint)
        .timeout(Duration::from_secs(5))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await;
    match outcome {
        Ok(_) => tauri_plugin_log::log::debug!(
            "Ask AI MCP best-effort token revoke posted for \"{}\"",
            cfg.label
        ),
        Err(error) => tauri_plugin_log::log::debug!(
            "Ask AI MCP best-effort token revoke for \"{}\" failed (ignored): {error}",
            cfg.label
        ),
    }
}

/// The token to send to the revocation endpoint: the refresh token when present
/// (revoking it drops the whole grant per RFC 7009), else the access token. Pulled
/// out of the serialized Token Set so no `oauth2` trait dependency is needed —
/// `StandardTokenResponse` serializes both as bare strings.
fn revocable_token(creds: &StoredCredentials) -> Option<String> {
    let token = serde_json::to_value(creds.token_response.as_ref()?).ok()?;
    token
        .get("refresh_token")
        .and_then(serde_json::Value::as_str)
        .or_else(|| token.get("access_token").and_then(serde_json::Value::as_str))
        .map(str::to_string)
}

/// Begin the browser OAuth flow for an HTTP OAuth connector. Backs BOTH the
/// "Connect" and "Reconnect" Settings buttons — reconnect is just a fresh
/// authorize, so there is deliberately no separate reconnect command. Opens the
/// system browser; the deep-link callback finishes the flow and emits
/// [`MCP_AUTHORIZATION_CHANGED_EVENT`].
#[tauri::command]
pub async fn mcp_oauth_begin(app_handle: tauri::AppHandle, id: String) -> Result<(), String> {
    // Clone the manager out of managed state so the network-bound begin await does
    // not hold the `State` guard — same pattern as `mcp_list_server_tools`.
    let manager = (*app_handle.state::<McpManager>()).clone();
    manager.begin_oauth(&app_handle, &id).await
}

/// Disconnect an OAuth connector: best-effort server-side revoke, then always drop
/// the local Token Set (returning the connector to *Needs authorization*). The
/// config row stays in Settings.
#[tauri::command]
pub async fn mcp_oauth_disconnect(app_handle: tauri::AppHandle, id: String) -> Result<(), String> {
    let manager = (*app_handle.state::<McpManager>()).clone();
    manager.disconnect_oauth(&app_handle, &id).await
}

/// The OAuth authorization lifecycle state of one connector, for the Settings
/// surface (Connector Authorization State, ADR 0051). Only meaningful for an
/// `Http` + `OAuth` connector; a Bearer connector uses the has-secret badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpOAuthState {
    /// No token set — added, never connected (or disconnected). "Needs authorization".
    None,
    /// A browser authorization flow is in flight (a pending entry exists).
    Authorizing,
    /// A token set is held and last known to refresh.
    Authorized,
    /// A held token failed to refresh at warm-on-open. "Needs reconnect".
    Reconnect,
}

/// Resolve the OAuth state from the three independent signals. Precedence:
/// an in-flight flow (Authorizing) wins; then a held-but-dead token (Reconnect);
/// then a held token (Authorized); else None. `enabled` is deliberately NOT an
/// input — authorization is orthogonal to enablement (ADR 0051).
fn derive_oauth_state(has_token: bool, is_authorizing: bool, needs_reconnect: bool) -> McpOAuthState {
    if is_authorizing {
        return McpOAuthState::Authorizing;
    }
    if has_token {
        if needs_reconnect {
            McpOAuthState::Reconnect
        } else {
            McpOAuthState::Authorized
        }
    } else {
        McpOAuthState::None
    }
}

/// Per-connector OAuth status for the Settings surface. camelCase to match the
/// frontend `McpOAuthStatus` mirror (the `McpToolDescriptor` precedent).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpOAuthStatus {
    pub id: String,
    pub state: McpOAuthState,
}

impl McpManager {
    /// One [`McpOAuthStatus`] per configured **OAuth** connector (Http + OAuth,
    /// non-empty id). Bearer connectors are skipped (they use the has-secret
    /// badge). Reads the FULL settings list, not `enabled_servers` — authorization
    /// is orthogonal to enablement (ADR 0051).
    pub(crate) async fn oauth_statuses(&self, app_handle: &tauri::AppHandle) -> Vec<McpOAuthStatus> {
        super::super::read_ai_runtime_settings(app_handle)
            .mcp_servers
            .into_iter()
            .filter(|cfg| {
                cfg.transport == McpTransport::Http
                    && cfg.auth_mode == McpAuthMode::OAuth
                    && !cfg.id.trim().is_empty()
            })
            .map(|cfg| {
                // ponytail: keychain reads on the command's async task, like
                // `warm` already does at L402 — a handful per Settings open. Move
                // to spawn_blocking only if OAuth connectors ever number enough
                // that a few keychain hits stall the command.
                let has_token = app_infra::has_mcp_server_secret(&cfg.id).unwrap_or(false);
                let is_authorizing = lock_recover(&self.inner().oauth_pending)
                    .values()
                    .any(|pending| pending.connector_id == cfg.id);
                let needs_reconnect =
                    lock_recover(&self.inner().oauth_reconnect_needed).contains(&cfg.id);
                McpOAuthStatus {
                    id: cfg.id,
                    state: derive_oauth_state(has_token, is_authorizing, needs_reconnect),
                }
            })
            .collect()
    }
}

/// The per-connector OAuth authorization state (ADR 0051), one entry per OAuth
/// connector in Settings — the status contract the Settings surface renders
/// (Bearer connectors are absent; they use the has-secret badge). Re-read on
/// receipt of [`MCP_AUTHORIZATION_CHANGED_EVENT`].
#[tauri::command]
pub async fn mcp_oauth_statuses(app_handle: tauri::AppHandle) -> Result<Vec<McpOAuthStatus>, String> {
    let manager = (*app_handle.state::<McpManager>()).clone();
    Ok(manager.oauth_statuses(&app_handle).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pending-OAuth map: two flows key off distinct CSRF `state` strings with
    /// no cross-talk (each is looked up + removed by its own key), and the TTL
    /// sweep drops a stale flow while a fresh one survives. Keyed off `created_at`
    /// alone (values are plain `Instant`s) since a real `AuthorizationManager`
    /// needs the network to build — the sweep rule is the same either way.
    #[test]
    fn pending_oauth_evicts_only_the_stale_flow_and_keys_by_state() {
        let base = Instant::now();
        let ttl = OAUTH_PENDING_TTL;
        let mut map: HashMap<String, Instant> = HashMap::new();
        map.insert("state-stale".to_string(), base);
        map.insert("state-fresh".to_string(), base + ttl);
        // Distinct keys coexist and resolve independently (no collision).
        assert!(map.contains_key("state-stale") && map.contains_key("state-fresh"));

        // `now` is just past the stale flow's TTL but well within the fresh one's.
        let now = base + ttl + Duration::from_millis(1);
        evict_expired(&mut map, |created_at| *created_at, now, ttl);
        assert!(map.contains_key("state-fresh"), "a fresh flow must survive the sweep");
        assert!(!map.contains_key("state-stale"), "a flow past the TTL must be evicted");

        // Claim-by-state removes exactly the one entry, leaving the map empty.
        assert!(map.remove("state-fresh").is_some());
        assert!(map.remove("state-missing").is_none());
        assert!(map.is_empty());
    }

    /// Disconnect's LOCAL drop is UNCONDITIONAL: it forgets the Token Set (and the
    /// reconnect flag / pending flow / cached slot) with NO server-side revoke in
    /// the picture. A stored token must be gone afterward — local teardown must
    /// never depend on the server being reachable.
    #[tokio::test]
    async fn disconnect_local_drop_forgets_the_token_unconditionally() {
        let dir = fixture_dir("mnema-mcp-oauth-drop");
        std::env::set_var("MNEMA_MCP_SERVER_SECRET_DIR", &dir);

        let id = "oauth-disconnect-test";
        app_infra::store_mcp_server_secret(id, "{\"client_id\":\"x\"}")
            .expect("store the Token Set");
        assert!(
            app_infra::has_mcp_server_secret(id).expect("has"),
            "the Token Set must be present before the drop"
        );

        let inner = Inner::default();
        // Seed the in-memory traces the drop must also clear.
        lock_recover(&inner.oauth_reconnect_needed).insert(id.to_string());

        drop_oauth_local(&inner, id).await;

        assert!(
            !app_infra::has_mcp_server_secret(id).expect("has"),
            "local drop must delete the Token Set even with no server-side revoke"
        );
        assert!(
            !lock_recover(&inner.oauth_reconnect_needed).contains(id),
            "local drop must clear the reconnect flag"
        );

        std::env::remove_var("MNEMA_MCP_SERVER_SECRET_DIR");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The pure OAuth-state derivation (ADR 0051): the whole truth table plus the
    /// two precedence rules the Settings surface depends on — authorizing wins over
    /// a held token, and reconnect requires a held token (a stray flag with no
    /// token is still *Needs authorization*, never *Needs reconnect*).
    #[test]
    fn derive_oauth_state_truth_table() {
        //          has_token, is_authorizing, needs_reconnect
        assert_eq!(derive_oauth_state(false, false, false), McpOAuthState::None);
        assert_eq!(derive_oauth_state(true, false, false), McpOAuthState::Authorized);
        assert_eq!(derive_oauth_state(true, false, true), McpOAuthState::Reconnect);
        assert_eq!(derive_oauth_state(false, true, false), McpOAuthState::Authorizing);
        // Authorizing wins even over a held (and even a dead) token.
        assert_eq!(derive_oauth_state(true, true, false), McpOAuthState::Authorizing);
        assert_eq!(derive_oauth_state(true, true, true), McpOAuthState::Authorizing);
        // Reconnect requires a held token: no token + reconnect flag → None.
        assert_eq!(derive_oauth_state(false, false, true), McpOAuthState::None);
    }

    /// The wire contract the TS mirror depends on: the state serializes snake_case
    /// and `McpOAuthStatus` serializes `{id, state}` camelCase.
    #[test]
    fn oauth_status_serializes_to_the_ts_wire_shape() {
        assert_eq!(
            serde_json::to_value(McpOAuthState::Reconnect).unwrap(),
            serde_json::json!("reconnect")
        );
        assert_eq!(
            serde_json::to_value(McpOAuthState::None).unwrap(),
            serde_json::json!("none")
        );
        let status = McpOAuthStatus {
            id: "x".to_string(),
            state: McpOAuthState::Reconnect,
        };
        assert_eq!(
            serde_json::to_value(&status).unwrap(),
            serde_json::json!({ "id": "x", "state": "reconnect" })
        );
    }

    /// A unique scratch dir for one test's fixture signal files.
    fn fixture_dir(prefix: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create fixture dir");
        dir
    }
}
