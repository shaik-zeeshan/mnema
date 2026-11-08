//! ChatGPT credentials, vault denial, first-party host binding, MCP secrets and
//! local model listing.

use super::*;

#[test]
fn a_rejected_chatgpt_login_reports_needs_reconnect_without_a_send() {
    let rejected = crate::chatgpt_auth::ChatgptTokenSet {
        access_token: "a".to_string(),
        refresh_token: Some("r".to_string()),
        expires_at: Some(4_000_000_000),
        rejected: true,
    };
    assert_eq!(
        chatgpt_engine_config("chatgpt", "gpt-5.5", Ok(Some(rejected.clone()))).err(),
        Some("needs_reconnect:chatgpt".to_string())
    );
    assert_eq!(
        chatgpt_static_models("chatgpt", Ok(Some(rejected))).err(),
        Some("needs_reconnect:chatgpt".to_string())
    );
}

#[test]
fn a_denied_vault_is_a_stable_status_code() {
    let denied =
        app_infra::AppInfraError::SecretVaultDenied("user denied prompt".to_string()).to_string();
    assert_eq!(status_reason_code(denied), "vault_denied");
    assert_eq!(status_reason_code("no_base_url".to_string()), "no_base_url");
}

#[test]
fn chatgpt_token_set_maps_to_an_access_token_config_or_needs_reconnect() {
    // The whole mapping, with the vault result supplied rather than read —
    // the previous version of this test resolved the real provider id
    // "chatgpt" against the developer's actual vault, so it passed through
    // whichever arm the machine happened to take and would fail outright on
    // a machine with ChatGPT connected.
    let connected = chatgpt_engine_config(
        "chatgpt",
        "gpt-5.6-terra",
        Ok(Some(crate::chatgpt_auth::ChatgptTokenSet {
            access_token: "the-access-token".to_string(),
            refresh_token: Some("r".to_string()),
            expires_at: Some(4_000_000_000),
            rejected: false,
        })),
    );
    match connected {
        Ok(ai_engine::EngineConfig::Cloud {
            provider,
            model,
            api_key,
            base_url,
        }) => {
            assert!(matches!(provider, ai_engine::CloudProvider::Chatgpt));
            assert_eq!(model, "gpt-5.6-terra");
            assert_eq!(
                api_key, "the-access-token",
                "the stored access token is the credential"
            );
            assert_eq!(
                base_url, None,
                "the chatgpt kind never carries a caller host"
            );
        }
        other => panic!("expected a chatgpt cloud config, got {other:?}"),
    }

    // Never connected, and the vault refusing to answer, are both the
    // reconnect code — NOT `no_provider_key`, whose UI copy says to paste a
    // key, which is not the fix for a login.
    assert_eq!(
        chatgpt_engine_config("chatgpt", "gpt-5.6-terra", Ok(None)).map(|_| ()),
        Err("needs_reconnect:chatgpt".to_string())
    );
    // A vault that refused to answer still fails closed, but keeps its own
    // error: it is not a verdict on the login (see
    // `a_denied_vault_is_not_a_verdict_on_the_chatgpt_login`).
    assert_eq!(
        chatgpt_engine_config("chatgpt", "gpt-5.6-terra", Err("vault locked".to_string()))
            .map(|_| ()),
        Err("vault locked".to_string())
    );
}

#[tokio::test]
async fn the_live_resolver_is_a_passthrough_for_every_non_chatgpt_kind() {
    // Five real call sites moved from the sync resolver to the live one.
    // For every kind but chatgpt the two must agree exactly, across the
    // whole precedence chain — a drift here silently breaks Ask AI, the
    // digest, and the derivation worker for the providers that were
    // already working.
    let settings = local_settings();
    let cases: [(Option<(&str, &str)>, Option<&str>); 4] = [
        (None, None),
        (Some(("ollama", "pinned-model")), None),
        (None, Some("override-model")),
        (Some(("does-not-exist", "m")), None),
    ];
    for (pin, feature_override) in cases {
        let sync = resolve_engine_config(&settings, pin, feature_override);
        let live = resolve_engine_config_live(&settings, pin, feature_override).await;
        assert_eq!(
            format!("{sync:?}"),
            format!("{live:?}"),
            "sync and live resolvers disagree for pin={pin:?} override={feature_override:?}"
        );
    }

    // …and the CLOUD kinds, which are the ones the live resolver can
    // actually damage. Local settings alone never produce an
    // `EngineConfig::Cloud`, so they never enter the `if let Cloud {
    // provider: Chatgpt, .. }` arm at all — widening that arm to every
    // cloud provider (which would send Anthropic/OpenAI/compatible keys
    // through the ChatGPT refresh and fail them all with
    // `needs_reconnect`) leaves a local-only comparison completely green.
    crate::test_support::install_shared_test_secret_vault();
    let cloud_id = "anthropic-live-passthrough";
    app_infra::store_ai_provider_key(cloud_id, "sk-live-passthrough").expect("seed a cloud key");
    let cloud = AiRuntimeSettings {
        enabled: true,
        providers: vec![AiProviderConfig {
            id: cloud_id.to_string(),
            kind: AiProviderKind::Anthropic,
            label: String::new(),
            base_url: String::new(),
        }],
        default_model: Some(AiEngineRef {
            provider: cloud_id.to_string(),
            model: "claude-test".to_string(),
        }),
        mcp_servers: Vec::new(),
        favorite_models: Vec::new(),
        recent_models: Vec::new(),
    };
    let sync = resolve_engine_config(&cloud, None, None).expect("sync resolves the cloud key");
    let live = resolve_engine_config_live(&cloud, None, None)
        .await
        .expect("live must resolve it identically, with no network");
    assert_eq!(format!("{sync:?}"), format!("{live:?}"));
    match live {
        ai_engine::EngineConfig::Cloud { api_key, .. } => assert_eq!(
            api_key, "sk-live-passthrough",
            "a non-chatgpt cloud provider must keep its own stored key"
        ),
        other => panic!("expected a cloud config, got {other:?}"),
    }
    let _ = app_infra::delete_ai_provider_key(cloud_id);
}

#[test]
fn listing_failure_keeps_the_chatgpt_reconnect_reason_code() {
    // `list_models_for_provider` answers a never-connected chatgpt instance
    // with `needs_reconnect:<id>`, and both consumers branch on that literal
    // prefix to render the sign-in copy. The code must survive this
    // translation — note "needs_re*connect*" also matches the network arm's
    // `contains("connect")`, which would mislabel it as "unreachable".
    assert_eq!(
        classify_listing_failure("needs_reconnect:chatgpt"),
        "needs_reconnect:chatgpt"
    );
    assert_eq!(
        classify_listing_failure("error sending request"),
        "unreachable"
    );
}

#[test]
fn chatgpt_id_is_bound_like_the_other_first_party_ids() {
    // The `chatgpt` instance id holds an OAuth token set in the very vault
    // slot the pasted-key kinds use, so a `{id:"chatgpt",
    // kind:OpenaiCompatible, baseUrl:"…attacker…"}` record must not be able
    // to ship that token set out as a Bearer credential.
    assert!(validate_provider_base_url("chatgpt", "https://attacker.test/v1").is_err());
    assert!(validate_provider_base_url("chatgpt", "https://chatgpt.com/backend-api/codex").is_ok());

    let settings = AiRuntimeSettings {
        enabled: true,
        providers: vec![AiProviderConfig {
            id: "chatgpt".to_string(),
            kind: AiProviderKind::OpenaiCompatible,
            label: String::new(),
            base_url: "https://attacker.test/v1".to_string(),
        }],
        default_model: Some(AiEngineRef {
            provider: "chatgpt".to_string(),
            model: "gpt-5.6-terra".to_string(),
        }),
        mcp_servers: Vec::new(),
        favorite_models: Vec::new(),
        recent_models: Vec::new(),
    };
    assert_eq!(
        resolve_engine_config(&settings, None, None).map(|_| ()),
        Err("base_url_host_mismatch:chatgpt".to_string())
    );
}

#[test]
fn the_chatgpt_model_gate_is_the_provider_verification_signal() {
    // "Models listed" IS the proof this provider works — onboarding
    // readiness, the Settings lamps and the Finish gate all read it. So a
    // provider instance that exists but was never signed in has to fail,
    // and it has to fail with the reconnect code specifically: the copy for
    // `no_provider_key` says to paste a key, which is not the fix here.
    let never_connected = chatgpt_static_models("chatgpt", Ok(None));
    assert_eq!(
        never_connected.map(|_| ()),
        Err("needs_reconnect:chatgpt".to_string())
    );
    // A vault that refuses to answer is not a licence to report "live"
    // either — fail closed. It keeps its own error rather than becoming the
    // reconnect code, because it is not a verdict on the login (see
    // `a_denied_vault_is_not_a_verdict_on_the_chatgpt_login`).
    assert_eq!(
        chatgpt_static_models("chatgpt", Err("vault locked".to_string())).map(|_| ()),
        Err("vault locked".to_string())
    );

    let connected = chatgpt_static_models(
        "chatgpt",
        Ok(Some(crate::chatgpt_auth::ChatgptTokenSet {
            access_token: "a".to_string(),
            refresh_token: Some("r".to_string()),
            expires_at: Some(4_000_000_000),
            rejected: false,
        })),
    )
    .expect("a connected instance lists its catalog");
    assert_eq!(
        connected.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        crate::chatgpt_auth::CHATGPT_MODEL_IDS.to_vec(),
        "the picker gets the static catalog verbatim"
    );
    assert!(
        !connected.is_empty(),
        "an empty list would read as connected-but-no-models, which is the \
         one outcome the gate exists to prevent"
    );
    assert!(
        connected.iter().all(|m| m.context_window.is_none()),
        "this backend reports no context-window metadata"
    );
}

#[test]
fn a_denied_vault_is_not_a_verdict_on_the_chatgpt_login() {
    // "Denied ≠ missing" (the ADR 0048 amendment `classify_listing_failure`
    // already implements for every other kind): a vault that REFUSED to
    // answer says nothing about the credential inside it. The user denied
    // the keychain prompt, or the vault is cached-denied for this process
    // lifetime — the ChatGPT login is untouched and still healthy.
    //
    // Collapsing that into `needs_reconnect` renders "Reconnect ChatGPT in
    // Settings", and the button next to that copy is Disconnect — which
    // destroys the OAuth token set (access AND the long-lived refresh
    // token) the vault merely declined to hand over. That is precisely the
    // harm ADR 0058 splits `provider_unreachable` out to avoid: never tell
    // a user to sign in again over something that never rendered a verdict
    // on their grant.
    let denied =
        app_infra::AppInfraError::SecretVaultDenied("user denied prompt".to_string()).to_string();

    // Still fails closed — a denied vault never reads as "live".
    let listing = chatgpt_static_models("chatgpt", Err(denied.clone()))
        .map(|_| ())
        .expect_err("a denied vault must not list models");
    assert_ne!(
        listing, "needs_reconnect:chatgpt",
        "a denied vault is not a bad login; this copy invites Disconnect"
    );
    assert_eq!(
        classify_listing_failure(&listing),
        "keychain access denied",
        "the picker row must read the same for chatgpt as for every other kind"
    );

    // Same on the egress path: the engine must not resolve, and must not
    // blame the login for it.
    let engine = chatgpt_engine_config("chatgpt", "gpt-5.6-terra", Err(denied))
        .map(|_| ())
        .expect_err("a denied vault must not resolve an engine");
    assert_ne!(
        engine, "needs_reconnect:chatgpt",
        "a denied vault is not a bad login"
    );

    // The genuine credential problems keep the reconnect code.
    assert_eq!(
        chatgpt_static_models("chatgpt", Ok(None)).map(|_| ()),
        Err("needs_reconnect:chatgpt".to_string())
    );
    assert_eq!(
        chatgpt_engine_config("chatgpt", "gpt-5.6-terra", Ok(None)).map(|_| ()),
        Err("needs_reconnect:chatgpt".to_string())
    );

    // …including the OTHER thing `load_token_set` can fail on: slot content
    // this module did not write. That IS a broken credential, so signing in
    // again really is the fix — and the split has to happen where the two
    // are still distinguishable (inside `load_token_set`), not by handing
    // both callers one indistinguishable `Err`.
    crate::test_support::install_shared_test_secret_vault();
    let corrupt_id = "chatgpt-corrupt-slot";
    app_infra::store_ai_provider_key(corrupt_id, "sk-not-a-token-set")
        .expect("seed an unparseable token set");
    let settings = AiRuntimeSettings {
        enabled: true,
        providers: vec![AiProviderConfig {
            id: corrupt_id.to_string(),
            kind: AiProviderKind::Chatgpt,
            label: String::new(),
            base_url: String::new(),
        }],
        default_model: Some(AiEngineRef {
            provider: corrupt_id.to_string(),
            model: "gpt-5.6-terra".to_string(),
        }),
        mcp_servers: Vec::new(),
        favorite_models: Vec::new(),
        recent_models: Vec::new(),
    };
    assert_eq!(
        resolve_engine_config(&settings, None, None).map(|_| ()),
        Err(format!("needs_reconnect:{corrupt_id}")),
        "an unreadable token SET is a login problem, and must not leak a \
         serde message the surfaces cannot classify"
    );
    let _ = app_infra::delete_ai_provider_key(corrupt_id);
}

#[test]
fn a_chatgpt_kind_never_falls_through_to_the_pasted_key_path() {
    // Without the kind check in `engine_config_for_ref`, a chatgpt provider
    // reads the same vault slot as every other kind — and finds the token
    // SET, a JSON blob, which would be handed to rig as a raw Bearer key.
    // Every call would then 401 with a reason code that tells the user to
    // paste an API key they do not have.
    let settings = AiRuntimeSettings {
        enabled: true,
        providers: vec![AiProviderConfig {
            id: "chatgpt".to_string(),
            kind: AiProviderKind::Chatgpt,
            label: String::new(),
            base_url: String::new(),
        }],
        default_model: Some(AiEngineRef {
            provider: "chatgpt".to_string(),
            model: "gpt-5.6-terra".to_string(),
        }),
        mcp_servers: Vec::new(),
        favorite_models: Vec::new(),
        recent_models: Vec::new(),
    };
    crate::test_support::install_shared_test_secret_vault();
    let _ = app_infra::delete_ai_provider_key("chatgpt");
    assert_eq!(
        resolve_engine_config(&settings, None, None).map(|_| ()),
        Err("needs_reconnect:chatgpt".to_string()),
        "a never-connected chatgpt provider is a login problem, not a key problem"
    );
}

#[tokio::test]
async fn the_live_resolver_refreshes_the_pinned_instance_not_the_default() {
    // `resolve_engine_ref` hands the live resolver the provider id whose
    // vault slot the token lives in. If it returned the DEFAULT model's id
    // instead, a thread pinned to a second ChatGPT account would refresh —
    // and rotate — the wrong account's credential.
    crate::test_support::install_shared_test_secret_vault();
    let settings = AiRuntimeSettings {
        enabled: true,
        providers: vec![
            AiProviderConfig {
                id: "chatgpt".to_string(),
                kind: AiProviderKind::Chatgpt,
                label: String::new(),
                base_url: String::new(),
            },
            AiProviderConfig {
                id: "chatgpt-2".to_string(),
                kind: AiProviderKind::Chatgpt,
                label: String::new(),
                base_url: String::new(),
            },
        ],
        default_model: Some(AiEngineRef {
            provider: "chatgpt".to_string(),
            model: "gpt-5.6-terra".to_string(),
        }),
        mcp_servers: Vec::new(),
        favorite_models: Vec::new(),
        recent_models: Vec::new(),
    };
    // Only the SECOND instance is connected, and with a long-lived token so
    // resolving it needs no network.
    let _ = app_infra::delete_ai_provider_key("chatgpt");
    crate::chatgpt_auth::store_token_set(
        "chatgpt-2",
        &crate::chatgpt_auth::ChatgptTokenSet {
            access_token: "second-account-token".to_string(),
            refresh_token: Some("r".to_string()),
            expires_at: Some(4_000_000_000),
            rejected: false,
        },
    )
    .expect("seed the second account");

    let pinned = resolve_engine_config_live(&settings, Some(("chatgpt-2", "gpt-5.5")), None)
        .await
        .expect("the pin resolves against its own instance");
    match pinned {
        ai_engine::EngineConfig::Cloud { api_key, model, .. } => {
            assert_eq!(api_key, "second-account-token");
            assert_eq!(model, "gpt-5.5");
        }
        other => panic!("expected a cloud config, got {other:?}"),
    }

    // …and the unpinned default really is the other, unconnected instance.
    assert_eq!(
        resolve_engine_config_live(&settings, None, None)
            .await
            .map(|_| ()),
        Err("needs_reconnect:chatgpt".to_string())
    );
    let _ = app_infra::delete_ai_provider_key("chatgpt-2");
}

#[test]
fn a_second_chatgpt_instance_is_bound_to_its_host_too() {
    // `newAiProviderId` (ai-providers.ts) hands the FIRST instance of a kind
    // the bare kind id and every later one `<kind>-2`, `<kind>-3`, … — so a
    // user with two ChatGPT logins holds an OAuth token set (access AND
    // long-lived refresh token) in the `chatgpt-2` vault slot. The settings
    // record itself is plaintext JSON in the app config dir, i.e. writable
    // by anything running as the user, while the vault is keychain-gated.
    // Binding only the literal id `chatgpt` lets a rewritten
    // `{id:"chatgpt-2", kind:OpenaiCompatible, baseUrl:"…attacker…"}` record
    // make Mnema itself POST that token set to the attacker as a Bearer
    // credential.
    assert!(
        validate_provider_base_url("chatgpt-2", "https://attacker.test/v1").is_err(),
        "a derived chatgpt instance id must stay pinned to chatgpt.com"
    );
    assert!(
        validate_provider_base_url("chatgpt-2", "https://chatgpt.com/backend-api/codex").is_ok()
    );
    // Same shape for the other first-party ids whose slot holds a key that
    // only ever belongs to one host.
    assert!(validate_provider_base_url("openai-2", "https://attacker.test/v1").is_err());
    assert!(validate_provider_base_url("anthropic-3", "https://attacker.test/v1").is_err());
    // Custom OpenAI-compatible instances keep their caller-chosen host.
    assert!(
        validate_provider_base_url(
            "openai_compatible-2",
            "https://api.fireworks.ai/inference/v1"
        )
        .is_ok(),
        "custom compatible instances must keep a caller-chosen host"
    );

    let settings = AiRuntimeSettings {
        enabled: true,
        providers: vec![AiProviderConfig {
            id: "chatgpt-2".to_string(),
            kind: AiProviderKind::OpenaiCompatible,
            label: String::new(),
            base_url: "https://attacker.test/v1".to_string(),
        }],
        default_model: Some(AiEngineRef {
            provider: "chatgpt-2".to_string(),
            model: "gpt-5.6-terra".to_string(),
        }),
        mcp_servers: Vec::new(),
        favorite_models: Vec::new(),
        recent_models: Vec::new(),
    };
    assert_eq!(
        resolve_engine_config(&settings, None, None).map(|_| ()),
        Err("base_url_host_mismatch:chatgpt-2".to_string())
    );
}

#[test]
fn the_first_party_host_pin_survives_url_authority_tricks() {
    // The pin compares a hand-split of the base-URL string, but the request
    // is dialled by reqwest's real (WHATWG) URL parser. Any string the two
    // read differently turns the pin into a rubber stamp — and the
    // `chatgpt` slot holds an OAuth token SET (access + long-lived refresh
    // token) that a kind-swapped settings record would then ship out as an
    // `Authorization: Bearer` header.
    for hostile in [
        "https://attacker.test?@chatgpt.com/v1",
        "https://attacker.test#@chatgpt.com/v1",
        "https://attacker.test\\@chatgpt.com/v1",
    ] {
        assert_eq!(
            url::Url::parse(hostile).expect("parses").host_str(),
            Some("attacker.test"),
            "the request really does go to the attacker: {hostile}"
        );
        assert_eq!(
            validate_provider_base_url("chatgpt", hostile).map(|_| ()),
            Err("base_url_host_mismatch:chatgpt".to_string()),
            "the chatgpt token set must not be sent to {hostile}"
        );
    }

    // Same instrument, same fix, for the other pinned first-party slots.
    assert!(
        validate_provider_base_url("openai", "https://attacker.test?@api.openai.com/v1").is_err()
    );
    assert!(validate_provider_base_url(
        "anthropic-2",
        "https://attacker.test#@api.anthropic.com/v1"
    )
    .is_err());

    // …and the legitimate hosts still pass.
    assert!(validate_provider_base_url("chatgpt", "https://chatgpt.com/backend-api/codex").is_ok());
    assert!(validate_provider_base_url("openai-2", "https://api.openai.com/v1").is_ok());
}

#[test]
fn mcp_secret_commands_guard_blank_ids_and_secrets() {
    // The guards return BEFORE any spawn_blocking, so these calls never
    // touch the keychain / secret store — safe to run against the real
    // commands with no fixture.
    let err = block_on(mcp_set_server_secret(McpServerSecretRequest {
        id: "  ".to_string(),
        secret: "token".to_string(),
    }))
    .expect_err("blank id must be rejected");
    assert_eq!(err, "a server id is required");

    let err = block_on(mcp_set_server_secret(McpServerSecretRequest {
        id: "github".to_string(),
        secret: "  ".to_string(),
    }))
    .expect_err("blank secret must be rejected");
    assert_eq!(err, "a secret is required");

    let err = block_on(mcp_clear_server_secret(McpServerRequest {
        id: String::new(),
    }))
    .expect_err("blank id must be rejected");
    assert_eq!(err, "a server id is required");

    let err = block_on(mcp_has_server_secret(McpServerRequest {
        id: " ".to_string(),
    }))
    .expect_err("blank id must be rejected");
    assert_eq!(err, "a server id is required");
}

/// A one-shot HTTP server that answers the first request with `body` and
/// exits. Enough to stand in for a local Ollama's `/v1/models`.
fn one_shot_models_server(body: &'static str) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut scratch = [0_u8; 1024];
            let _ = stream.read(&mut scratch);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://127.0.0.1:{port}")
}

/// The local-scan path both ways: an endpoint that answers yields its model
/// ids, and one with nothing listening fails cleanly (an `Err`, promptly —
/// the caller renders "no answer" instead of hanging).
#[test]
fn local_endpoint_lists_models_and_fails_cleanly_when_nothing_answers() {
    let endpoint = one_shot_models_server(
        r#"{"data":[{"id":"llama3.1:8b"},{"id":"qwen2.5:14b"},{"id":"nomic-embed-text"}]}"#,
    );
    let live = AiProviderConfig {
        id: "scan-live".to_string(),
        kind: AiProviderKind::Ollama,
        label: String::new(),
        base_url: endpoint,
    };
    let client = reqwest::Client::new();
    let discovered = block_on(list_models_for_provider(&client, &live)).expect("live");
    let ids: Vec<String> = discovered
        .into_iter()
        .map(|model| model.id)
        .filter(|id| is_chat_capable_model(id))
        .collect();
    // The embedding model is filtered out of the picker pool.
    assert_eq!(ids, vec!["llama3.1:8b", "qwen2.5:14b"]);

    // Nothing listening: a refused connection, classified as `unreachable`.
    let dead = AiProviderConfig {
        id: "scan-dead".to_string(),
        kind: AiProviderKind::Ollama,
        label: String::new(),
        // Port 1 is reserved and never has a listener on macOS.
        base_url: "http://127.0.0.1:1".to_string(),
    };
    let error =
        block_on(list_models_for_provider(&client, &dead)).expect_err("a closed port must fail");
    assert_eq!(classify_listing_failure(&error), "unreachable");
}
