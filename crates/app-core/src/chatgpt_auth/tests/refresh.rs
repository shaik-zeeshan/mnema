use super::*;

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
    store_token_set(
        provider,
        &token_set(&jwt_with_exp(now - 10), "refresh-0", now - 10),
    )
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

    let held = LOGIN_GENERATIONS
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
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
        load_token_set(provider)
            .expect("load")
            .map(|set| set.access_token),
        None,
        "once it does run, the revocation clears the slot"
    );
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
    store_token_set(rejected, &token_set(&jwt_with_exp(now - 10), "r", now - 10)).expect("seed");
    assert_eq!(
        fresh_access_token_with(rejected, rejecting_grant).await,
        Err(format!("needs_reconnect:{rejected}")),
        "a spent refresh token is the one case that really does need a new login"
    );
    // The dead set stays put: nothing here should clear the slot behind the
    // user's back.
    let marked = load_token_set(rejected).expect("load").expect("set");
    assert!(marked.rejected, "invalid_grant persists the rejected mark");
    // The mark short-circuits: no second round trip replays a spent token.
    assert_eq!(
        fresh_access_token_with(rejected, rotating_refresh_grant).await,
        Err(format!("needs_reconnect:{rejected}"))
    );
    // A successful login writes a fresh set, which clears the mark.
    let generation = bump_login_generation(rejected);
    device_login::persist_token_set_if_current(
        rejected,
        generation,
        &token_set(&jwt_with_exp(now + 3600), "r2", now + 3600),
    )
    .expect("login");
    assert!(
        !load_token_set(rejected)
            .expect("load")
            .expect("set")
            .rejected
    );
    let _ = app_infra::delete_ai_provider_key(rejected);

    let offline = "chatgpt-grant-unreachable";
    store_token_set(offline, &token_set(&jwt_with_exp(now - 10), "r", now - 10)).expect("seed");
    assert_eq!(
        fresh_access_token_with(offline, unreachable_grant).await,
        Err(format!("provider_unreachable:{offline}")),
        "an unreachable endpoint must not be reported as a signed-out account"
    );
    assert!(
        !load_token_set(offline)
            .expect("load")
            .expect("set")
            .rejected,
        "a transport error never marks the login rejected"
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

    let stored = load_token_set(provider)
        .expect("load")
        .expect("still connected");
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
        load_token_set(provider)
            .expect("load")
            .and_then(|s| s.refresh_token),
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
    *WEDGED_VAULT_PROVIDER
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(provider.to_string());

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

    *WEDGED_VAULT_PROVIDER
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = None;
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
        load_token_set(provider)
            .expect("load")
            .map(|s| s.access_token),
        None,
        "a disconnected provider must stay disconnected"
    );
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
        load_token_set(provider)
            .expect("load")
            .map(|set| set.access_token),
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
    // Taken in the module's own order (generations before rotations, as
    // `revoke_provider_credential` does), or this deadlocks a parallel revoke.
    let _ = std::thread::spawn(|| {
        let _generations = LOGIN_GENERATIONS.lock().expect("login generations");
        let _rotations = PENDING_ROTATIONS.lock().expect("pending rotations");
        let _refresh = REFRESH_LOCKS.lock().expect("refresh locks");
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
        load_token_set(provider)
            .expect("load")
            .map(|set| set.access_token),
        None,
        "a poisoned lock must not make a provider impossible to disconnect"
    );
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
