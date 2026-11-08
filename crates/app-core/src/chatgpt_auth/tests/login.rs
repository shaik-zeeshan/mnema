use super::*;

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

    persist_token_set_if_current(
        provider,
        current,
        &token_set("account-two", "r2", 4_000_000_000),
    )
    .expect("the current login persists");
    let _ = persist_token_set_if_current(
        provider,
        stale,
        &token_set("account-one", "r1", 4_000_000_000),
    );

    assert_eq!(
        load_token_set(provider)
            .expect("load")
            .map(|set| set.access_token),
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
        load_token_set(provider)
            .expect("load")
            .map(|set| set.access_token),
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
        load_token_set(provider)
            .expect("load")
            .map(|set| set.access_token),
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

    assert_eq!(
        *calls.lock().unwrap(),
        2,
        "it waited through the pending poll"
    );
    let stored = load_token_set(provider).expect("load").expect("connected");
    assert_eq!(stored.access_token, "access-for-the-code");
    assert_eq!(
        stored.refresh_token.as_deref(),
        Some("refresh-for-the-verifier")
    );
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
        load_token_set(provider)
            .expect("load")
            .map(|set| set.access_token),
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
    assert_eq!(
        load_token_set(provider)
            .expect("load")
            .map(|s| s.access_token),
        None
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
            let (status, body) = responses.get(*at).copied().unwrap_or_else(|| {
                panic!(
                    "poll called {} times, script has {}",
                    *at + 1,
                    responses.len()
                )
            });
            *at += 1;
            Ok((
                reqwest::StatusCode::from_u16(status).unwrap(),
                body.to_string(),
            ))
        })
            as std::pin::Pin<
                Box<
                    dyn std::future::Future<Output = Result<(reqwest::StatusCode, String), String>>
                        + Send,
                >,
            >
    };
    (poll, calls)
}

const APPROVED_BODY: &str = r#"{"authorization_code":"the-code","code_verifier":"the-verifier"}"#;

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

    assert!(
        error.contains("429"),
        "the status belongs in the message: {error}"
    );
    assert_eq!(
        *calls.lock().unwrap(),
        1,
        "a terminal status is not retried"
    );
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
