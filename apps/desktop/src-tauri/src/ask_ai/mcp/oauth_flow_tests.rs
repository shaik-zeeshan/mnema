use super::*;

/// A connector flipped Bearer->OAuth without disconnecting leaves a PLAIN bearer
/// secret in the shared, polymorphic keychain slot. That opaque string is NOT an
/// OAuth Token Set, so the connector's `has_token` signal (the input to
/// `derive_oauth_state`) must read false -> *Needs authorization*, never a false
/// *Authorized* -- else Settings paints a green "authorized" badge for a connector
/// whose OAuth connect fails to parse the slot on every turn. A real Token Set
/// (even client_id-only) still reads authorized.
#[test]
fn a_stale_bearer_secret_never_reads_as_oauth_authorized() {
    crate::secret_vault_test_support::install_shared_test_secret_vault();

    let id = "flip-bearer-to-oauth";
    // What the bearer path wrote: an opaque `Authorization: Bearer` token --
    // not JSON, not a StoredCredentials Token Set.
    app_infra::store_mcp_server_secret(id, "lin_bearer_secret_abc")
        .expect("store bearer secret");
    assert!(
        app_infra::has_mcp_server_secret(id).expect("has"),
        "bytes are present in the polymorphic slot"
    );

    // The signal `oauth_statuses` feeds `derive_oauth_state` must reject it.
    let has_token = oauth_token_present(id);
    assert!(!has_token, "a plain bearer secret is not an OAuth Token Set");
    assert_eq!(
        derive_oauth_state(has_token, false, false),
        McpOAuthState::None,
        "a stale bearer secret must read as Needs authorization, not Authorized"
    );

    // A real Token Set (client_id-only is enough to parse) still reads authorized.
    let token_set = serde_json::to_string(&StoredCredentials::new(
        "client-xyz".to_string(),
        None,
        Vec::new(),
        None,
    ))
    .expect("serialize token set");
    app_infra::store_mcp_server_secret(id, &token_set).expect("store token set");
    assert!(oauth_token_present(id), "a real Token Set must read as authorized");

    let _ = app_infra::delete_mcp_server_secret(id);
}

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
    crate::secret_vault_test_support::install_shared_test_secret_vault();

    let id = "oauth-disconnect-test";
    app_infra::store_mcp_server_secret(id, "{\"client_id\":\"x\"}")
        .expect("store the Token Set");
    assert!(
        app_infra::has_mcp_server_secret(id).expect("has"),
        "the Token Set must be present before the drop"
    );

    let inner = Inner::default();
    // Seed the in-memory traces the drop must also clear (a current-generation
    // warm write, the way `warm` sets the flag).
    {
        let mut reconnect = lock_recover(&inner.oauth_reconnect_needed);
        let generation = reconnect.generation(id);
        reconnect.apply_if_current(id, generation, ReconnectFlag::Set);
        assert!(reconnect.contains(id));
    }

    drop_oauth_local(&inner, id).await;

    assert!(
        !app_infra::has_mcp_server_secret(id).expect("has"),
        "local drop must delete the Token Set even with no server-side revoke"
    );
    assert!(
        !lock_recover(&inner.oauth_reconnect_needed).contains(id),
        "local drop must clear the reconnect flag"
    );
}

/// Disconnect's best-effort revoke must NEVER stall local teardown. The comment
/// promises a ~5 s cap, but only the final POST is bounded — a server that
/// ACCEPTS the connection and then stalls during OAuth *discovery* (before the
/// POST) hangs `best_effort_revoke` on rmcp's 30 s per-request timeout, and
/// `disconnect_oauth` awaits the revoke BEFORE dropping the local token. A dead
/// or hostile endpoint must not delay dropping the local token by tens of
/// seconds — the revoke itself must be time-bounded.
#[tokio::test]
async fn best_effort_revoke_is_bounded_against_a_stalling_server() {
    use std::io::Read;
    // A loopback TCP server that ACCEPTS then stalls forever (never writes an
    // HTTP response), so rmcp's discovery GET hangs until its own 30 s timeout.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming() {
            match stream {
                Ok(mut s) => {
                    // Drain the request so the client's write completes, then
                    // hold the socket open without ever replying.
                    let mut buf = [0u8; 64];
                    let _ = s.read(&mut buf);
                    held.push(s);
                }
                Err(_) => break,
            }
        }
    });

    let cfg = McpServerConfig {
        id: "oauth-revoke-stall".to_string(),
        label: "Stall".to_string(),
        enabled: false,
        transport: McpTransport::Http,
        auth_mode: McpAuthMode::OAuth,
        command: None,
        args: Vec::new(),
        env: Vec::new(),
        url: Some(format!("http://127.0.0.1:{port}/")),
        secret_env_name: None,
        enabled_tools: None,
    };

    let outcome =
        tokio::time::timeout(Duration::from_secs(8), best_effort_revoke(&cfg)).await;
    assert!(
        outcome.is_ok(),
        "best_effort_revoke stalled past 8 s against a hung discovery endpoint — a \
         dead server must never block Disconnect's local teardown"
    );
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

/// Invariant #1 (ADR 0051): the TLS-or-loopback guard must cover the endpoints
/// DISCOVERED at runtime, not just the user-typed base URL. rmcp does not enforce
/// https on the DCR auth-code authorization/token/registration endpoints, so a
/// hostile discovery document that advertises a cleartext http token endpoint
/// would otherwise POST the authorization code + PKCE verifier and receive the
/// access/refresh tokens over http — readable by any on-path attacker. The flow
/// must refuse before opening the browser / exchanging the code.
#[test]
fn a_discovered_cleartext_endpoint_is_refused() {
    fn md_with(
        authz: &str,
        token: &str,
        reg: Option<&str>,
        revoke: Option<&str>,
    ) -> AuthorizationMetadata {
        let mut md = AuthorizationMetadata::default();
        md.authorization_endpoint = authz.to_string();
        md.token_endpoint = token.to_string();
        md.registration_endpoint = reg.map(str::to_string);
        if let Some(revoke) = revoke {
            md.additional_fields.insert(
                "revocation_endpoint".to_string(),
                serde_json::Value::String(revoke.to_string()),
            );
        }
        md
    }

    // Cleartext token endpoint (where the code + PKCE verifier are exchanged and
    // the tokens come back) must be refused even though base/authorize are TLS.
    let hostile_token = md_with(
        "https://auth.example.com/authorize",
        "http://auth.example.com/token",
        Some("https://auth.example.com/register"),
        None,
    );
    assert!(
        discovered_endpoints_secure(&hostile_token).is_err(),
        "a cleartext discovered token endpoint must be refused"
    );

    // A cleartext registration endpoint (the DCR POST) is refused too.
    let hostile_reg = md_with(
        "https://auth.example.com/authorize",
        "https://auth.example.com/token",
        Some("http://auth.example.com/register"),
        None,
    );
    assert!(discovered_endpoints_secure(&hostile_reg).is_err());

    // A cleartext revocation endpoint (RFC 7009 carries the refresh token) too.
    let hostile_revoke = md_with(
        "https://auth.example.com/authorize",
        "https://auth.example.com/token",
        None,
        Some("http://auth.example.com/revoke"),
    );
    assert!(discovered_endpoints_secure(&hostile_revoke).is_err());

    // All-TLS metadata is accepted; a loopback auth server (never leaves the
    // host) is allowed, matching `secret_may_ride_url`.
    let clean = md_with(
        "https://auth.example.com/authorize",
        "https://auth.example.com/token",
        Some("https://auth.example.com/register"),
        Some("https://auth.example.com/revoke"),
    );
    assert!(discovered_endpoints_secure(&clean).is_ok());
    let loopback = md_with(
        "http://127.0.0.1:9000/authorize",
        "http://127.0.0.1:9000/token",
        None,
        None,
    );
    assert!(discovered_endpoints_secure(&loopback).is_ok());
}

/// Build a real [`PendingOAuth`] for one connector at `created_at`.
/// `AuthorizationManager::new` is network-free (it only builds an HTTP client
/// and parses the base URL), so the pending-map rules are testable offline.
async fn pending(connector_id: &str, created_at: Instant) -> PendingOAuth {
    let manager = AuthorizationManager::new("https://mcp.example.com/")
        .await
        .expect("AuthorizationManager::new is network-free");
    PendingOAuth {
        manager: Arc::new(manager),
        connector_id: connector_id.to_string(),
        created_at,
    }
}

/// A second Connect for the SAME connector must SUPERSEDE the earlier flow: at
/// most one pending entry per connector survives. Otherwise the abandoned first
/// flow keeps `connector_is_authorizing` true even after the second flow is
/// claimed and the connector is Authorized — a stuck *Authorizing* spinner.
#[tokio::test]
async fn a_second_connect_supersedes_the_prior_same_connector_flow() {
    let now = Instant::now();
    let mut map: HashMap<String, PendingOAuth> = HashMap::new();

    park_pending(&mut map, "state-a".to_string(), pending("gh", now).await);
    park_pending(&mut map, "state-b".to_string(), pending("gh", now).await);

    assert_eq!(
        map.len(),
        1,
        "a second Connect for one connector must leave exactly one pending flow"
    );
    assert!(map.contains_key("state-b"), "the newest flow survives");
    assert!(!map.contains_key("state-a"), "the superseded flow is gone");

    // Claiming the surviving flow leaves NO stale same-connector entry, so the
    // connector is no longer read as authorizing.
    assert!(map.remove("state-b").is_some());
    assert!(
        !connector_is_authorizing(&map, "gh", now, OAUTH_PENDING_TTL),
        "after the live flow is claimed, the connector must not read as Authorizing"
    );
}

/// An abandoned Connect (never completed, never swept because no later
/// `begin_oauth` ran) must not read as *Authorizing* forever: once its TTL has
/// lapsed the status read stops counting it.
#[tokio::test]
async fn an_expired_abandoned_flow_no_longer_reads_as_authorizing() {
    let base = Instant::now();
    let mut map: HashMap<String, PendingOAuth> = HashMap::new();
    map.insert("state-old".to_string(), pending("gh", base).await);

    // Just inside the TTL: still authorizing.
    assert!(connector_is_authorizing(&map, "gh", base, OAUTH_PENDING_TTL));
    // Past the TTL (browser abandoned, no sweep ran): no longer authorizing.
    let later = base + OAUTH_PENDING_TTL + Duration::from_millis(1);
    assert!(
        !connector_is_authorizing(&map, "gh", later, OAUTH_PENDING_TTL),
        "an abandoned flow past its TTL must not pin the status to Authorizing"
    );
}

/// The warm-on-open reconnect-flag WRITE decision — the counterpart of the
/// `derive_oauth_state` READ truth table. The load-bearing arm is
/// `(false, false) → Leave`: a connector that never held a token and fails to
/// connect is *Needs authorization*, and must NOT be flagged *Needs reconnect*.
/// Dropping the has-token guard in `warm` would flip every never-authorized
/// connector to "Needs reconnect" on any connect failure; this pins it.
#[test]
fn reconnect_flag_update_flags_a_dead_held_token_but_not_an_unauthorized_connector() {
    assert_eq!(reconnect_flag_update(true, true), ReconnectFlag::Clear);
    assert_eq!(reconnect_flag_update(true, false), ReconnectFlag::Clear);
    assert_eq!(reconnect_flag_update(false, true), ReconnectFlag::Set);
    assert_eq!(reconnect_flag_update(false, false), ReconnectFlag::Leave);
}

/// The warm-race guard (ADR 0051: reconnect state converges under any
/// interleaving of warm tasks / authorize / disconnect). A warm task captures
/// the generation BEFORE its connect; a browser re-authorize that completes
/// while the connect is in flight clears the flag AND bumps the generation, so
/// when the warm task's `Err` (dialed with the now-dead OLD token) resolves
/// late, its `Set` is guarded by a stale generation and dropped — a freshly
/// authorized connector must not flash back to *Needs reconnect*.
#[test]
fn a_stale_warm_write_after_a_newer_authorize_is_dropped() {
    let mut state = ReconnectState::default();
    let id = "gh";

    // warm captures the generation, then starts its (slow) connect…
    let stale_generation = state.generation(id);
    // …the callback lands a fresh token meanwhile: clear + bump…
    state.clear_and_bump(id);
    // …then the warm connect with the OLD token resolves Err and tries to flag.
    state.apply_if_current(id, stale_generation, ReconnectFlag::Set);
    assert!(
        !state.contains(id),
        "a warm write guarded by a stale generation must be dropped after authorize"
    );

    // A warm write against the CURRENT generation still lands (the guard drops
    // stale writes, not the signal itself)…
    let current_generation = state.generation(id);
    state.apply_if_current(id, current_generation, ReconnectFlag::Set);
    assert!(state.contains(id), "an up-to-date warm failure must still flag");

    // …and disconnect bumps again, so THAT write is now stale too.
    state.clear_and_bump(id);
    state.apply_if_current(id, current_generation, ReconnectFlag::Set);
    assert!(!state.contains(id), "disconnect must invalidate in-flight warm writes");
}
