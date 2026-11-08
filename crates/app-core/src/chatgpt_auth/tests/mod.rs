use super::device_login::*;
use super::*;

fn jwt_with_exp(exp: i64) -> String {
    let header = base64::prelude::BASE64_URL_SAFE_NO_PAD.encode(b"{\"alg\":\"none\"}");
    let payload = base64::prelude::BASE64_URL_SAFE_NO_PAD
        .encode(serde_json::json!({ "exp": exp }).to_string());
    format!("{header}.{payload}.sig")
}

#[test]
fn jwt_expiration_reads_exp_claim() {
    assert_eq!(
        jwt_expiration_seconds(&jwt_with_exp(1_234_567)),
        Some(1_234_567)
    );
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
        rejected: false,
    };
    let expiring = ChatgptTokenSet {
        access_token: "t".into(),
        refresh_token: None,
        expires_at: Some(now + TOKEN_EXPIRY_SKEW_SECONDS - 5),
        rejected: false,
    };
    let unknown = ChatgptTokenSet {
        access_token: "t".into(),
        refresh_token: None,
        expires_at: None,
        rejected: false,
    };
    assert!(!fresh.expires_within_skew());
    assert!(expiring.expires_within_skew());
    assert!(
        unknown.expires_within_skew(),
        "missing exp must force a refresh attempt"
    );
}

#[test]
fn device_code_response_accepts_string_or_numeric_interval() {
    let as_string: DeviceCodeResponse =
        serde_json::from_str(r#"{"device_auth_id":"d","user_code":"u","interval":"5"}"#).unwrap();
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
    crate::test_support::install_shared_test_secret_vault();
}

fn token_set(access: &str, refresh: &str, expires_at: i64) -> ChatgptTokenSet {
    ChatgptTokenSet {
        access_token: access.to_string(),
        refresh_token: Some(refresh.to_string()),
        expires_at: Some(expires_at),
        rejected: false,
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
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
    assert!(
        ok.get("error").is_none(),
        "a success carries no error key: {ok}"
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
    assert!(
        (expires_at - (now + 3600)).abs() <= 5,
        "expected ~now+3600, got {expires_at}"
    );
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
        let parsed: DeviceCodeResponse = serde_json::from_str(body).unwrap_or_else(|e| {
            panic!("a junk interval hint must not fail the login: {body} ({e})")
        });
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

#[test]
fn oauth_failure_log_kind_drops_the_response_body() {
    assert_eq!(
        oauth_failure_log_kind(
            r#"token request failed with status 401 Unauthorized: {"access_token":"sk-x"}"#
        ),
        "token request failed with status 401 Unauthorized"
    );
    assert_eq!(
        oauth_failure_log_kind("device code request failed with status 500: <html>"),
        "device code request failed with status 500"
    );
    assert_eq!(
        oauth_failure_log_kind(r#"invalid type: string "eyJtoken", expected u64"#),
        "invalid type"
    );
    assert_eq!(oauth_failure_log_kind("timed out"), "timed out");
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
        rejected: false,
    };
    let back: ChatgptTokenSet =
        serde_json::from_str(&serde_json::to_string(&set).unwrap()).unwrap();
    assert_eq!(back.access_token, "access");
    assert_eq!(back.refresh_token.as_deref(), Some("refresh"));
    assert_eq!(back.expires_at, Some(42));
}

mod login;
mod refresh;
