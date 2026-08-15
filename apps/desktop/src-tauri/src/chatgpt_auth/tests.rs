use super::*;
use super::device_login::*;

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

/// A dropped connection on the CODE EXCHANGE must not throw away an
/// approval the user already gave.
///
/// This is the one hop with no second chance. The poll loop now survives a
/// blip (`a_network_blip_mid_poll_does_not_kill_the_login`), but the
/// exchange right after it discards `OAuthError::transient` entirely
/// (`.map_err(|error| error.message)`) — so a WiFi roam in the second
/// between "the user approved in the browser" and "the vault holds a token
/// set" ends the login with a raw `error sending request for url (…)`. By
/// then the code UI is gone: `ChatgptConnect.svelte` renders that string as
/// the sign-in failure and drops back to idle, and onboarding
/// (`onboarding-ai.svelte.ts`) marks the instance `status: "error"`, which
/// blocks the Finish gate. The whole device flow has to be redone for a
/// request that never rendered a verdict — exactly what this module's own
/// transient/terminal split exists to prevent.
///
/// Retrying is safe in the one direction that matters: if the server DID
/// process the lost request, the code is spent and the retry comes back
/// `invalid_grant`, which is terminal and stops the loop.
static BLIPPED_EXCHANGE_CALLS: AtomicU64 = AtomicU64::new(0);

fn exchange_after_one_dropped_connection(
    code: DeviceTokenResponse,
) -> Pin<Box<dyn Future<Output = Result<OAuthTokenResponse, OAuthError>> + Send>> {
    Box::pin(async move {
        if BLIPPED_EXCHANGE_CALLS.fetch_add(1, Ordering::SeqCst) == 0 {
            return Err(OAuthError::transport(
                "error sending request for url (https://auth.openai.com/oauth/token)",
            ));
        }
        Ok(OAuthTokenResponse {
            access_token: format!("access-for-{}", code.authorization_code),
            refresh_token: Some("refresh-after-the-blip".to_string()),
            expires_in: Some(3600),
        })
    })
}

#[tokio::test]
async fn a_dropped_connection_on_the_code_exchange_does_not_throw_away_an_approval() {
    install_test_vault();
    let provider = "chatgpt-login-exchange-blip";
    let _ = app_infra::delete_ai_provider_key(provider);
    let generation = bump_login_generation(provider);
    BLIPPED_EXCHANGE_CALLS.store(0, Ordering::SeqCst);

    let (poll, _calls) = scripted_poll(vec![(200, APPROVED_BODY)]);
    poll_and_store_with(
        poll,
        exchange_after_one_dropped_connection,
        provider,
        generation,
        std::time::Duration::from_millis(1),
        std::time::Duration::from_secs(30),
    )
    .await
    .expect("a dropped connection is not OpenAI refusing the sign-in the user just approved");

    assert_eq!(
        load_token_set(provider).expect("load").map(|set| set.access_token),
        Some("access-for-the-code".to_string()),
        "the approval must land in the vault, not be thrown away with the connection"
    );
    assert_eq!(
        BLIPPED_EXCHANGE_CALLS.load(Ordering::SeqCst),
        2,
        "the lost request must be retried exactly once here, not abandoned"
    );
    let _ = app_infra::delete_ai_provider_key(provider);
}

/// …but a code OpenAI actually REFUSED is terminal on the first answer:
/// retrying a rejected grant only delays the failure the user has to see.
#[tokio::test]
async fn a_refused_code_exchange_is_not_retried() {
    install_test_vault();
    let provider = "chatgpt-login-exchange-refusal-not-retried";
    let _ = app_infra::delete_ai_provider_key(provider);
    let generation = bump_login_generation(provider);
    REFUSED_EXCHANGE_CALLS.store(0, Ordering::SeqCst);

    let (poll, _calls) = scripted_poll(vec![(200, APPROVED_BODY)]);
    let outcome = poll_and_store_with(
        poll,
        counting_refusing_exchange,
        provider,
        generation,
        std::time::Duration::from_millis(1),
        std::time::Duration::from_secs(30),
    )
    .await;

    assert!(outcome.is_err(), "a refused code is a failed login");
    assert_eq!(
        REFUSED_EXCHANGE_CALLS.load(Ordering::SeqCst),
        1,
        "a verdict OpenAI already rendered must not be asked for again"
    );
}

static REFUSED_EXCHANGE_CALLS: AtomicU64 = AtomicU64::new(0);

fn counting_refusing_exchange(
    _code: DeviceTokenResponse,
) -> Pin<Box<dyn Future<Output = Result<OAuthTokenResponse, OAuthError>> + Send>> {
    Box::pin(async move {
        REFUSED_EXCHANGE_CALLS.fetch_add(1, Ordering::SeqCst);
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

    let held = LOGIN_GENERATIONS.lock().unwrap_or_else(PoisonError::into_inner);
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

    assert!(recover_pending_rotation(provider).is_none());

    let stored = load_token_set(provider).expect("load").expect("still connected");
    assert_eq!(
        stored.refresh_token.as_deref(),
        Some("rotated-refresh"),
        "the held rotation must land, or the login is dead"
    );
    assert!(!stored.expires_within_skew());
    // One-shot: a second recovery has nothing left to do and must not
    // resurrect anything.
    assert!(recover_pending_rotation(provider).is_none());
    assert_eq!(
        load_token_set(provider).expect("load").and_then(|s| s.refresh_token),
        Some("rotated-refresh".to_string())
    );
    let _ = app_infra::delete_ai_provider_key(provider);
}

/// A vault that keeps refusing the rotation.
///
/// The held-rotation retry only covers ONE failed write: if the second
/// attempt fails too (the disk is still full — `LowDisk` is a state this
/// app parks in, not a millisecond), the call falls straight through to a
/// refresh using the token set still sitting in the slot — whose refresh
/// token OpenAI consumed on the first call. That replay is the one thing
/// `PENDING_ROTATIONS` exists to prevent: OpenAI answers `invalid_grant`,
/// which this module (correctly) classifies as terminal, and a user whose
/// only problem is a full disk is told to reconnect a perfectly healthy
/// ChatGPT account.
static WEDGED_CONSUMED_REFRESH_TOKENS: Mutex<Option<std::collections::HashSet<String>>> =
    Mutex::new(None);
static WEDGED_GRANT_CALLS: AtomicU64 = AtomicU64::new(0);

fn wedged_rotating_grant(
    refresh_token: String,
) -> Pin<Box<dyn Future<Output = Result<OAuthTokenResponse, OAuthError>> + Send>> {
    Box::pin(async move {
        WEDGED_GRANT_CALLS.fetch_add(1, Ordering::SeqCst);
        let mut consumed = WEDGED_CONSUMED_REFRESH_TOKENS
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

#[tokio::test]
async fn a_vault_that_keeps_refusing_a_rotation_never_replays_the_consumed_refresh_token() {
    install_test_vault();
    let provider = "chatgpt-vault-stays-wedged";
    let now = unix_now();
    store_token_set(
        provider,
        &token_set(&jwt_with_exp(now - 10), "wedged-refresh-0", now - 10),
    )
    .expect("seed an expiring token set");
    *WEDGED_VAULT_PROVIDER.lock().unwrap_or_else(PoisonError::into_inner) =
        Some(provider.to_string());

    // First call: the grant rotates, the write fails, the rotation is held.
    assert_eq!(
        fresh_access_token_with(provider, wedged_rotating_grant).await,
        Err(format!("provider_unreachable:{provider}")),
        "a failed vault write is a hiccup, not a signed-out account"
    );

    // Second call, disk still full: re-persisting the held rotation fails
    // again — and the slot still holds the refresh token OpenAI consumed.
    let second = fresh_access_token_with(provider, wedged_rotating_grant).await;

    assert_eq!(
        WEDGED_GRANT_CALLS.load(Ordering::SeqCst),
        1,
        "the consumed refresh token must never be replayed at OpenAI"
    );
    assert!(
        second.is_ok(),
        "the held rotation is a live credential; a wedged vault must not cost the user their login: {second:?}"
    );

    *WEDGED_VAULT_PROVIDER.lock().unwrap_or_else(PoisonError::into_inner) = None;
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

    assert!(recover_pending_rotation(provider).is_none());

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

/// A 15-minute wait is ~180 `send()` calls against `auth.openai.com` while
/// the user reads the code, switches to a browser, signs in and clears 2FA.
/// One failed send in that window — WiFi roaming, a DNS hiccup, a VPN
/// reconnect, a sleep/wake — must not end the login: nothing about the
/// login changed, the code is still on screen, and this module's own rule
/// is that a request that never rendered a verdict is not a verdict. The
/// documented exits from this loop are approval, a refusal, a superseded
/// generation and the deadline; a dropped connection is none of them.
#[tokio::test]
async fn a_network_blip_mid_poll_does_not_kill_the_login() {
    let calls = Arc::new(AtomicU64::new(0));
    let seen = calls.clone();
    let poll = move || {
        let seen = seen.clone();
        async move {
            match seen.fetch_add(1, Ordering::SeqCst) {
                0 => Ok((reqwest::StatusCode::FORBIDDEN, "not yet".to_string())),
                1 => Err("error sending request for url (https://auth.openai.com/…)".to_string()),
                _ => Ok((reqwest::StatusCode::OK, APPROVED_BODY.to_string())),
            }
        }
    };

    let code = await_authorization(
        || true,
        poll,
        std::time::Duration::from_millis(1),
        std::time::Duration::from_secs(30),
    )
    .await
    .expect("one dropped connection must not end a login the user is still completing");

    assert_eq!(code.authorization_code, "the-code");
}

/// …but a network that is simply down still ends the login, rather than
/// leaving the code UI up against an endpoint that will never answer.
#[tokio::test]
async fn a_dead_network_still_ends_the_login() {
    let calls = Arc::new(AtomicU64::new(0));
    let seen = calls.clone();
    let poll = move || {
        let seen = seen.clone();
        async move {
            seen.fetch_add(1, Ordering::SeqCst);
            Err::<(reqwest::StatusCode, String), String>(
                "error sending request for url (…)".to_string(),
            )
        }
    };

    let error = await_authorization(
        || true,
        poll,
        std::time::Duration::from_millis(1),
        std::time::Duration::from_secs(30),
    )
    .await
    .expect_err("an endpoint that never answers must end the login");

    assert!(error.contains("error sending request"), "{error}");
    assert!(
        calls.load(Ordering::SeqCst) <= 5,
        "a down network must give up quickly, not poll out the whole deadline: {} polls",
        calls.load(Ordering::SeqCst)
    );
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

/// One panic anywhere under this module's std mutexes must not wedge the
/// app for the rest of the process.
///
/// `LOGIN_GENERATIONS` is taken with `.expect()` in five places and held
/// across vault I/O (`persist_refreshed_token_set`,
/// `persist_token_set_if_current`, `revoke_provider_credential`);
/// `REFRESH_LOCKS` and `PENDING_ROTATIONS` the same way. A panic taken
/// while any of them is held poisons that static forever, and every later
/// `.lock().expect(...)` panics with it: Disconnect — which
/// `ai_runtime_clear_provider_key` routes through for EVERY provider kind —
/// panics for good, no ChatGPT login can start, and `fresh_access_token`
/// panics inside its `spawn_blocking`, reaching the UI as a raw
/// "task … panicked" string instead of the `needs_reconnect:` /
/// `provider_unreachable:` reason code this module's whole contract is
/// built on. `app_infra`'s vault handle already treats poison as
/// recoverable (`PoisonError::into_inner`); this module has to as well.
#[tokio::test]
async fn a_panic_under_the_module_locks_does_not_wedge_refresh_or_disconnect() {
    install_test_vault();
    let provider = "chatgpt-poisoned-locks";
    let now = unix_now();
    store_token_set(
        provider,
        &token_set(&jwt_with_exp(now - 10), "poison-refresh-0", now - 10),
    )
    .expect("seed a connected provider");

    // Any panic taken while these are held leaves them poisoned: an
    // `unreachable!` in the vault handle, a panicking blocking task, an
    // assert in a test that holds the generation lock.
    let _ = std::thread::spawn(|| {
        let _refresh = REFRESH_LOCKS.lock().expect("refresh locks");
        let _rotations = PENDING_ROTATIONS.lock().expect("pending rotations");
        let _generations = LOGIN_GENERATIONS.lock().expect("login generations");
        panic!("something panicked while holding the module's locks");
    })
    .join();

    // A refresh must still answer with the module's reason-code contract.
    assert_eq!(
        fresh_access_token_with(provider, rejecting_grant).await,
        Err(format!("needs_reconnect:{provider}")),
        "a poisoned lock must not turn every AI call into a raw panic string"
    );

    // A login must still be startable.
    let generation = bump_login_generation(provider);
    assert!(
        login_generation_is_current(provider, generation),
        "a poisoned lock must not make every login unstartable"
    );

    // And Disconnect must still clear the slot.
    revoke_provider_credential(provider).expect("disconnect must still work");
    assert_eq!(
        load_token_set(provider).expect("load").map(|set| set.access_token),
        None,
        "a poisoned lock must not make a provider impossible to disconnect"
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

/// A grant that carries an access token but no refresh token is not a
/// usable login: the set it produces has nothing to refresh with, so it
/// dies at the first expiry as `needs_reconnect`. Storing it anyway is a
/// silent credential loss, because Connect is reachable while already
/// connected ("start over", checking which account is attached) — the
/// doomed set overwrites the working one and the refresh token it replaced
/// is gone for good.
fn exchange_without_a_refresh_token(
    code: DeviceTokenResponse,
) -> Pin<Box<dyn Future<Output = Result<OAuthTokenResponse, OAuthError>> + Send>> {
    Box::pin(async move {
        Ok(OAuthTokenResponse {
            access_token: format!("access-for-{}", code.authorization_code),
            refresh_token: None,
            expires_in: Some(3600),
        })
    })
}

#[tokio::test]
async fn a_grant_with_no_refresh_token_fails_the_login_instead_of_replacing_a_working_one() {
    install_test_vault();
    let provider = "chatgpt-login-without-a-refresh-token";
    let now = unix_now();
    store_token_set(
        provider,
        &token_set(&jwt_with_exp(now + 3600), "working-refresh", now + 3600),
    )
    .expect("seed a connected provider");
    let generation = bump_login_generation(provider);

    let (poll, _calls) = scripted_poll(vec![(200, APPROVED_BODY)]);
    let outcome = poll_and_store_with(
        poll,
        exchange_without_a_refresh_token,
        provider,
        generation,
        std::time::Duration::from_millis(1),
        std::time::Duration::from_secs(30),
    )
    .await;

    assert!(
        outcome.is_err(),
        "a grant with no refresh token cannot be refreshed: it is a failed login, not a connection"
    );
    let stored = load_token_set(provider)
        .expect("load")
        .expect("the working credential must survive a login that produced nothing usable");
    assert_eq!(
        stored.refresh_token.as_deref(),
        Some("working-refresh"),
        "a doomed grant must not overwrite the refresh token the provider was working with"
    );
    let _ = app_infra::delete_ai_provider_key(provider);
}

/// The compare-and-swap asks "does the slot still hold the credential this
/// grant rotated?". A slot it cannot READ answers neither yes nor no — and
/// reading it as "no" is the destructive answer: by then OpenAI has already
/// consumed the old refresh token, so dropping the rotation is dropping the
/// only live copy of the credential.
#[test]
fn an_unreadable_slot_is_not_evidence_that_the_credential_moved_on() {
    install_test_vault();
    let provider = "chatgpt-cas-unreadable-slot";
    let now = unix_now();
    // What an unreadable slot looks like: a value this module did not write
    // (a pasted API key left behind by a kind swap), a torn write, or a
    // vault read that failed outright.
    app_infra::store_ai_provider_key(provider, "sk-not-a-token-set")
        .expect("seed an unreadable slot");

    let outcome = persist_refreshed_token_set(
        provider,
        "consumed-refresh",
        &token_set(&jwt_with_exp(now + 3600), "rotated-refresh", now + 3600),
    );

    assert!(
        outcome.is_err(),
        "a slot that cannot be read must fail the swap, not report the credential moved on: {outcome:?}"
    );
    let _ = app_infra::delete_ai_provider_key(provider);
}

#[test]
fn a_held_rotation_survives_a_slot_the_swap_cannot_read() {
    install_test_vault();
    let provider = "chatgpt-recovery-unreadable-slot";
    let now = unix_now();
    app_infra::store_ai_provider_key(provider, "sk-not-a-token-set")
        .expect("seed an unreadable slot");
    stash_pending_rotation(
        provider,
        "consumed-refresh",
        &token_set(&jwt_with_exp(now + 3600), "rotated-refresh", now + 3600),
    );

    recover_pending_rotation(provider);

    assert!(
        take_pending_rotation(provider).is_some(),
        "a rotation OpenAI already granted must keep being held when the vault could not be \
         read — dropping it is a login the user has to redo"
    );
    let _ = app_infra::delete_ai_provider_key(provider);
}

/// Disconnect destroys the credential. A rotation still held in memory for
/// that provider is a live access + refresh token for the account the user
/// just revoked, kept alive by the one code path whose whole job is writing
/// it back into the vault.
#[test]
fn disconnecting_drops_a_rotation_still_held_for_that_provider() {
    install_test_vault();
    let provider = "chatgpt-revoke-drops-the-held-rotation";
    let now = unix_now();
    stash_pending_rotation(
        provider,
        "consumed-refresh",
        &token_set(&jwt_with_exp(now + 3600), "rotated-refresh", now + 3600),
    );

    revoke_provider_credential(provider).expect("revoke");

    assert!(
        take_pending_rotation(provider).is_none(),
        "disconnect must not leave a live OAuth token set for the revoked account held in memory"
    );
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
