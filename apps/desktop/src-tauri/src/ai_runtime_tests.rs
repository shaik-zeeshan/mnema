use super::*;

#[test]
fn models_listing_reads_each_vendors_context_window_field() {
    let windows = |body: &str| -> Vec<Option<u64>> {
        serde_json::from_str::<ModelsListResponse>(body)
            .unwrap()
            .into_entries()
            .iter()
            .map(ModelsListEntry::context_window)
            .collect()
    };
    // Anthropic, Groq/DeepSeek, Mistral, OpenAI (none).
    assert_eq!(
        windows(
            r#"{"data":[{"id":"a","max_input_tokens":1000000},{"id":"g","context_window":131072},{"id":"m","max_context_length":32768},{"id":"o"}]}"#
        ),
        vec![Some(1_000_000), Some(131_072), Some(32_768), None]
    );
    // llama.cpp prefers the runtime window, skips router mode's 0 placeholder.
    assert_eq!(
        windows(
            r#"{"data":[{"id":"l","meta":{"n_ctx":4096,"n_ctx_train":131072}},{"id":"r","meta":{"n_ctx":0,"n_ctx_train":8192}}]}"#
        ),
        vec![Some(4096), Some(8192)]
    );
    // Together: bare top-level array.
    assert_eq!(
        windows(r#"[{"id":"t","context_length":8192}]"#),
        vec![Some(8192)]
    );
    assert_eq!(windows("[]"), Vec::<Option<u64>>::new());
    // vLLM, and an entry carrying two names still parses (no duplicate-field error).
    assert_eq!(
        windows(r#"{"data":[{"id":"v","max_model_len":16384},{"id":"x","context_length":200000,"max_input_tokens":100000}]}"#),
        vec![Some(16_384), Some(200_000)]
    );
}

/// Settings with local-only providers so resolution never touches the OS
/// keychain (cloud key loads are exercised by the app-infra store tests).
fn local_settings() -> AiRuntimeSettings {
    AiRuntimeSettings {
        enabled: true,
        providers: vec![
            AiProviderConfig {
                id: "ollama".to_string(),
                kind: AiProviderKind::Ollama,
                label: String::new(),
                base_url: String::new(),
            },
            AiProviderConfig {
                id: "llamafile".to_string(),
                kind: AiProviderKind::Llamafile,
                label: String::new(),
                base_url: "http://localhost:9090".to_string(),
            },
        ],
        default_model: Some(AiEngineRef {
            provider: "ollama".to_string(),
            model: "llama-default".to_string(),
        }),
        mcp_servers: Vec::new(),
        favorite_models: Vec::new(),
        recent_models: Vec::new(),
    }
}

#[test]
fn ai_runtime_status_round_trips_with_has_providers_and_kind() {
    let settings = local_settings();
    let status = AiRuntimeStatus {
        enabled: true,
        has_providers: true,
        configured: true,
        available: false,
        default_model: settings.default_model.clone(),
        default_provider_kind: crate::user_context::worker::default_provider_kind(&settings),
        reason: Some("local_endpoint_unreachable".to_string()),
    };
    let value = serde_json::to_value(&status).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "enabled": true, "hasProviders": true, "configured": true, "available": false,
            "defaultModel": { "provider": "ollama", "model": "llama-default" },
            "defaultProviderKind": "ollama",
            "reason": "local_endpoint_unreachable"
        })
    );
    assert_eq!(serde_json::from_value::<AiRuntimeStatus>(value).unwrap(), status);

    // No default model → no kind key at all.
    let bare = AiRuntimeSettings { default_model: None, ..settings };
    assert_eq!(crate::user_context::worker::default_provider_kind(&bare), None);
}

fn expect_local(config: ai_engine::EngineConfig) -> (ai_engine::LocalKind, String, String) {
    match config {
        ai_engine::EngineConfig::Local {
            kind,
            endpoint,
            model,
        } => (kind, endpoint, model),
        other => panic!("expected Local, got {other:?}"),
    }
}

#[test]
fn resolver_pin_beats_override_beats_default() {
    let settings = local_settings();
    // Pin wins over both the feature override and the default model.
    let (kind, endpoint, model) = expect_local(
        resolve_engine_config(
            &settings,
            Some(("llamafile", "pinned-model")),
            Some("override-model"),
        )
        .expect("pin should resolve"),
    );
    assert!(matches!(kind, ai_engine::LocalKind::Llamafile));
    assert_eq!(endpoint, "http://localhost:9090");
    assert_eq!(model, "pinned-model");

    // No pin: the override rides on the default model's provider.
    let (kind, endpoint, model) = expect_local(
        resolve_engine_config(&settings, None, Some("override-model"))
            .expect("override should resolve"),
    );
    assert!(matches!(kind, ai_engine::LocalKind::Ollama));
    assert_eq!(endpoint, "http://localhost:11434");
    assert_eq!(model, "override-model");

    // Neither: the global default model.
    let (_, _, model) = expect_local(
        resolve_engine_config(&settings, None, None).expect("default should resolve"),
    );
    assert_eq!(model, "llama-default");
}

#[test]
fn ask_ai_override_with_provider_runs_on_that_provider() {
    let settings = local_settings();
    let access = |model: Option<&str>, provider: Option<&str>| AccessSettings {
        ask_ai_model: model.map(str::to_string),
        ask_ai_model_provider: provider.map(str::to_string),
        ..AccessSettings::default()
    };
    let resolve = |access: &AccessSettings, pin: Option<(&str, &str)>| {
        let (pin, model) = ask_ai_resolver_inputs(&settings, access, pin);
        expect_local(resolve_engine_config(&settings, pin, model).expect("resolves"))
    };

    // Saved with its provider: runs there, not on the default's provider.
    let with_provider = access(Some("lf-model"), Some("llamafile"));
    let (kind, _, model) = resolve(&with_provider, None);
    assert!(matches!(kind, ai_engine::LocalKind::Llamafile));
    assert_eq!(model, "lf-model");

    // A usable thread pin still wins over it.
    let (kind, _, model) = resolve(&with_provider, Some(("ollama", "pinned")));
    assert!(matches!(kind, ai_engine::LocalKind::Ollama));
    assert_eq!(model, "pinned");

    // A pin to a removed provider falls through to the override, not past it.
    let (kind, _, model) = resolve(&with_provider, Some(("gone", "pinned")));
    assert!(matches!(kind, ai_engine::LocalKind::Llamafile));
    assert_eq!(model, "lf-model");

    // The override's own provider removed: skip it (never send its id to
    // another provider) and use the global default model.
    let orphaned = access(Some("lf-model"), Some("gone"));
    let (kind, _, model) = resolve(&orphaned, None);
    assert!(matches!(kind, ai_engine::LocalKind::Ollama));
    assert_eq!(model, "llama-default");

    // No provider saved: not an override (every writer saves the pair; a
    // bare id is only the pre-pair on-disk format) — the global default.
    let bare = access(Some("bare-model"), None);
    let (kind, _, model) = resolve(&bare, None);
    assert!(matches!(kind, ai_engine::LocalKind::Ollama));
    assert_eq!(model, "llama-default");

    // No override at all: the default.
    let (_, _, model) = resolve(&AccessSettings::default(), None);
    assert_eq!(model, "llama-default");
}

#[tokio::test]
async fn ask_ai_gate_checks_the_engine_ask_ai_runs_on() {
    // A live local server for one provider; nothing listens on port 1.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let live = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let dead = "http://127.0.0.1:1".to_string();
    let ask_on_llamafile = AccessSettings {
        ask_ai_model: Some("lf-model".to_string()),
        ask_ai_model_provider: Some("llamafile".to_string()),
        ..AccessSettings::default()
    };
    let with_endpoints = |ollama: &str, llamafile: &str| {
        let mut settings = local_settings();
        settings.providers[0].base_url = ollama.to_string();
        settings.providers[1].base_url = llamafile.to_string();
        settings
    };

    // The global default's (Ollama) server is down, but Ask AI runs on
    // Llamafile, which is up: Ask AI is ready.
    assert_eq!(
        ask_ai_engine_prerequisite(&with_endpoints(&dead, &live), &ask_on_llamafile).await,
        Ok(())
    );
    // And the converse: the default is up, Ask AI's own server is down.
    assert_eq!(
        ask_ai_engine_prerequisite(&with_endpoints(&live, &dead), &ask_on_llamafile).await,
        Err("local_endpoint_unreachable".to_string())
    );
    // No Ask AI model: the default engine decides, as before.
    assert_eq!(
        ask_ai_engine_prerequisite(&with_endpoints(&dead, &live), &AccessSettings::default())
            .await,
        Err("local_endpoint_unreachable".to_string())
    );
}

#[test]
fn resolver_falls_back_when_pin_provider_is_not_connected() {
    let mut settings = local_settings();
    settings.providers.retain(|p| p.kind == AiProviderKind::Ollama);
    // Pinned to a provider no longer in the list → fall back through the
    // chain (override layer first).
    let (kind, _, model) = expect_local(
        resolve_engine_config(
            &settings,
            Some(("llamafile", "pinned-model")),
            Some("override-model"),
        )
        .expect("fallback should resolve"),
    );
    assert!(matches!(kind, ai_engine::LocalKind::Ollama));
    assert_eq!(model, "override-model");

    // An unknown pin provider id behaves the same.
    let (_, _, model) = expect_local(
        resolve_engine_config(&settings, Some(("not-a-provider", "x")), None)
            .expect("fallback should resolve"),
    );
    assert_eq!(model, "llama-default");

    // Blank pin halves are treated as no pin.
    let (_, _, model) = expect_local(
        resolve_engine_config(&settings, Some(("", "pinned-model")), None)
            .expect("fallback should resolve"),
    );
    assert_eq!(model, "llama-default");
}

#[test]
fn resolver_requires_a_default_model_without_pin() {
    let mut settings = local_settings();
    settings.default_model = None;
    // No default → the override has no provider to ride on, and the plain
    // resolution fails with the dedicated reason.
    assert_eq!(
        resolve_engine_config(&settings, None, Some("override-model")).map(|_| ()),
        Err("no_default_model".to_string())
    );
    assert_eq!(
        resolve_engine_config(&settings, None, None).map(|_| ()),
        Err("no_default_model".to_string())
    );
    // A pin still resolves on its own (it names its provider explicitly).
    let (_, _, model) = expect_local(
        resolve_engine_config(&settings, Some(("ollama", "pinned-model")), None)
            .expect("pin should resolve without a default"),
    );
    assert_eq!(model, "pinned-model");
}

#[test]
fn resolver_reports_disconnected_default_provider() {
    let mut settings = local_settings();
    settings.default_model = Some(AiEngineRef {
        provider: "anthropic".to_string(),
        model: "claude-haiku-4-5".to_string(),
    });
    assert_eq!(
        resolve_engine_config(&settings, None, None).map(|_| ()),
        Err("provider_not_connected:anthropic".to_string())
    );
}

#[test]
fn resolver_distinguishes_two_instances_of_the_same_kind() {
    // Two Ollama instances with distinct ids and endpoints coexist; a pin
    // to each resolves to that instance's endpoint, and the default model
    // resolves to its own instance — identity is the instance id, not kind.
    let settings = AiRuntimeSettings {
        enabled: true,
        providers: vec![
            AiProviderConfig {
                id: "ollama".to_string(),
                kind: AiProviderKind::Ollama,
                label: String::new(),
                base_url: "http://box-a:11434".to_string(),
            },
            AiProviderConfig {
                id: "ollama-2".to_string(),
                kind: AiProviderKind::Ollama,
                label: "Box B".to_string(),
                base_url: "http://box-b:11434".to_string(),
            },
        ],
        default_model: Some(AiEngineRef {
            provider: "ollama".to_string(),
            model: "default-model".to_string(),
        }),
        mcp_servers: Vec::new(),
        favorite_models: Vec::new(),
        recent_models: Vec::new(),
    };

    // Pin to the second instance resolves to ITS endpoint.
    let (_, endpoint, model) = expect_local(
        resolve_engine_config(&settings, Some(("ollama-2", "pinned")), None)
            .expect("second-instance pin should resolve"),
    );
    assert_eq!(endpoint, "http://box-b:11434");
    assert_eq!(model, "pinned");

    // The default model resolves to the first instance's endpoint.
    let (_, endpoint, _) = expect_local(
        resolve_engine_config(&settings, None, None).expect("default should resolve"),
    );
    assert_eq!(endpoint, "http://box-a:11434");
}

#[test]
fn resolver_requires_base_url_for_openai_compatible() {
    let settings = AiRuntimeSettings {
        enabled: true,
        providers: vec![AiProviderConfig {
            id: "openai_compatible".to_string(),
            kind: AiProviderKind::OpenaiCompatible,
            label: String::new(),
            base_url: String::new(),
        }],
        default_model: Some(AiEngineRef {
            provider: "openai_compatible".to_string(),
            model: "some-model".to_string(),
        }),
        mcp_servers: Vec::new(),
        favorite_models: Vec::new(),
        recent_models: Vec::new(),
    };
    // The base-URL check fires before any keychain access.
    assert_eq!(
        resolve_engine_config(&settings, None, None).map(|_| ()),
        Err("no_base_url".to_string())
    );
}

#[test]
fn static_prerequisite_reason_codes() {
    // Master switch off.
    let mut settings = local_settings();
    settings.enabled = false;
    assert_eq!(
        engine_prerequisite_static(&settings).map(|_| ()),
        Err("ai_runtime_disabled".to_string())
    );

    // No providers connected.
    let mut settings = local_settings();
    settings.providers.clear();
    assert_eq!(
        engine_prerequisite_static(&settings).map(|_| ()),
        Err("no_providers".to_string())
    );

    // No default model chosen.
    let mut settings = local_settings();
    settings.default_model = None;
    assert_eq!(
        engine_prerequisite_static(&settings).map(|_| ()),
        Err("no_default_model".to_string())
    );

    // Fully configured local default passes the static half.
    assert!(engine_prerequisite_static(&local_settings()).is_ok());
}

#[test]
fn is_chat_capable_model_filters_non_chat_ids() {
    // Non-chat ids (image-gen, embeddings, whisper) are excluded.
    assert!(!is_chat_capable_model("flux-1-schnell"));
    assert!(!is_chat_capable_model("text-embedding-3-large"));
    assert!(!is_chat_capable_model("whisper-1"));
    assert!(!is_chat_capable_model("dall-e-3"));

    // Chat models stay in the pool, including provider-namespaced ids.
    assert!(is_chat_capable_model("deepseek-v4-pro"));
    assert!(is_chat_capable_model("claude-haiku-4-5"));
    assert!(is_chat_capable_model("gpt-4o"));
    assert!(is_chat_capable_model(
        "accounts/fireworks/models/deepseek-v4-pro"
    ));
    assert!(is_chat_capable_model("llama-3.3-70b"));
}

#[test]
fn base_host_strips_scheme_path_and_port() {
    assert_eq!(
        base_host("https://api.fireworks.ai/inference/v1"),
        "api.fireworks.ai"
    );
    assert_eq!(base_host("http://192.168.0.9:8080/v1"), "192.168.0.9");
    assert_eq!(base_host("https://API.Fireworks.AI/v1"), "api.fireworks.ai");
    assert_eq!(base_host("https://user@host.example/v1"), "host.example");
}

#[test]
fn key_ending_never_returns_more_than_four_chars() {
    assert_eq!(
        key_ending("sk-ant-api03-abcdefgh4f2a").as_deref(),
        Some("4f2a")
    );
    assert_eq!(
        key_ending("  sk-proj-0123456789\n").as_deref(),
        Some("6789")
    );
    // Multibyte-safe: 4 characters, not 4 bytes.
    assert_eq!(key_ending("key-ééééééééé✓✓").as_deref(), Some("éé✓✓"));
    // Too short for 4 characters to be a small part of it.
    assert_eq!(key_ending("sk-12345678"), None);
    // A ChatGPT OAuth token set is not a key.
    assert_eq!(key_ending(r#"{"access_token":"abcdefghijklmnop"}"#), None);
    for len in 0..64 {
        let key = "x".repeat(len);
        assert!(key_ending(&key).is_none_or(|end| end.chars().count() == 4));
    }
}

#[test]
fn classify_listing_failure_maps_common_errors() {
    assert_eq!(
        classify_listing_failure("no_provider_key:openai_compatible-2"),
        "missing API key"
    );
    assert_eq!(classify_listing_failure("no_base_url"), "no base URL set");
    // Denied ≠ missing: the SecretVaultDenied Display must classify as a
    // keychain denial, never as "missing API key".
    assert_eq!(
        classify_listing_failure(
            &app_infra::AppInfraError::SecretVaultDenied("user denied prompt".to_string())
                .to_string()
        ),
        "keychain access denied"
    );
    assert_eq!(
        classify_listing_failure("error sending request for url (http://192.168.0.9:8080/v1/models)"),
        "unreachable"
    );
    assert_eq!(
        classify_listing_failure("model listing request failed with status 401: invalid_api_key"),
        "authentication failed"
    );
    assert_eq!(
        classify_listing_failure("model listing request failed with status 500"),
        "couldn't list models"
    );
}

#[test]
fn is_fireworks_host_matches_only_fireworks() {
    // Fireworks' OpenAI-compatible base routes to the Gateway catalog.
    assert!(is_fireworks_host("https://api.fireworks.ai/inference/v1"));
    // Other OpenAI-compatible providers keep the generic /v1/models path.
    assert!(!is_fireworks_host("http://192.168.0.9:8080/v1"));
    assert!(!is_fireworks_host("https://openrouter.ai/api/v1"));
    // A path that merely mentions the host must not match (host-anchored).
    assert!(!is_fireworks_host("https://evil.test/api.fireworks.ai/v1"));
}

#[test]
fn validate_provider_base_url_rejects_nonhttp_schemes() {
    // Only http/https may ever carry a key to a model endpoint.
    assert!(validate_provider_base_url("custom-1", "file:///etc/passwd").is_err());
    assert!(validate_provider_base_url("custom-1", "gopher://host/v1").is_err());
    assert!(validate_provider_base_url("custom-1", "not a url").is_err());
    // A legitimate custom OpenAI-compatible endpoint passes.
    assert!(validate_provider_base_url("custom-1", "http://192.168.0.9:8080/v1").is_ok());
    assert!(validate_provider_base_url("custom-1", "https://openrouter.ai/api/v1").is_ok());
}

#[test]
fn validate_provider_base_url_binds_first_party_ids_to_their_host() {
    // The leak: a first-party id paired with an attacker host is refused, so
    // the keychain key for "anthropic"/"openai" can't reach a foreign host.
    assert!(validate_provider_base_url("anthropic", "https://attacker.test/v1").is_err());
    assert!(validate_provider_base_url("openai", "https://attacker.test/v1").is_err());
    // The fixed first-party host stays allowed.
    assert!(validate_provider_base_url("anthropic", "https://api.anthropic.com/v1").is_ok());
    assert!(validate_provider_base_url("openai", "https://api.openai.com/v1").is_ok());
    // Non-reserved ids (same-kind custom instances) keep their own host.
    assert!(
        validate_provider_base_url("openai_compatible-2", "https://my-proxy.example/v1").is_ok()
    );
}

#[test]
fn engine_config_for_ref_binds_first_party_ids_on_the_egress_path() {
    // H2 regression: the EngineConfig::Cloud built for REAL completions must
    // enforce the same key→host binding as model-listing. A first-party id
    // smuggled in as an OpenAI-compatible instance pointed at an attacker host
    // is rejected at config-build time — BEFORE the keychain key is loaded —
    // so the real Anthropic/OpenAI key (and the assembled prompt) never
    // egresses. The error is the same `base_url_host_mismatch` reason code
    // `validate_provider_base_url` returns.
    let settings = AiRuntimeSettings {
        enabled: true,
        providers: vec![AiProviderConfig {
            id: "anthropic".to_string(),
            kind: AiProviderKind::OpenaiCompatible,
            label: String::new(),
            base_url: "https://attacker.test/v1".to_string(),
        }],
        default_model: Some(AiEngineRef {
            provider: "anthropic".to_string(),
            model: "claude-haiku-4-5".to_string(),
        }),
        mcp_servers: Vec::new(),
        favorite_models: Vec::new(),
        recent_models: Vec::new(),
    };
    assert_eq!(
        resolve_engine_config(&settings, None, None).map(|_| ()),
        Err("base_url_host_mismatch:anthropic".to_string())
    );

    // A custom (non-first-party) OpenAI-compatible instance is unaffected by
    // the host binding: its base_url passes validation and config-building
    // proceeds to the keychain load (which fails here only because no key is
    // stored in the test environment — never on a host-mismatch).
    let custom = AiRuntimeSettings {
        enabled: true,
        providers: vec![AiProviderConfig {
            id: "openai_compatible-2".to_string(),
            kind: AiProviderKind::OpenaiCompatible,
            label: String::new(),
            base_url: "https://my-proxy.example/v1".to_string(),
        }],
        default_model: Some(AiEngineRef {
            provider: "openai_compatible-2".to_string(),
            model: "some-model".to_string(),
        }),
        mcp_servers: Vec::new(),
        favorite_models: Vec::new(),
        recent_models: Vec::new(),
    };
    let result = resolve_engine_config(&custom, None, None);
    if let Err(reason) = result {
        assert!(
            !reason.starts_with("base_url_host_mismatch"),
            "custom compat host must not be rejected by the binding, got {reason}"
        );
    }
}

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
            assert_eq!(api_key, "the-access-token", "the stored access token is the credential");
            assert_eq!(base_url, None, "the chatgpt kind never carries a caller host");
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
    crate::secret_vault_test_support::install_shared_test_secret_vault();
    let cloud_id = "anthropic-live-passthrough";
    app_infra::store_ai_provider_key(cloud_id, "sk-live-passthrough")
        .expect("seed a cloud key");
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
    assert_eq!(classify_listing_failure("error sending request"), "unreachable");
}

#[test]
fn chatgpt_id_is_bound_like_the_other_first_party_ids() {
    // The `chatgpt` instance id holds an OAuth token set in the very vault
    // slot the pasted-key kinds use, so a `{id:"chatgpt",
    // kind:OpenaiCompatible, baseUrl:"…attacker…"}` record must not be able
    // to ship that token set out as a Bearer credential.
    assert!(validate_provider_base_url("chatgpt", "https://attacker.test/v1").is_err());
    assert!(
        validate_provider_base_url("chatgpt", "https://chatgpt.com/backend-api/codex").is_ok()
    );

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
        app_infra::AppInfraError::SecretVaultDenied("user denied prompt".to_string())
            .to_string();

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
    crate::secret_vault_test_support::install_shared_test_secret_vault();
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
    crate::secret_vault_test_support::install_shared_test_secret_vault();
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
    crate::secret_vault_test_support::install_shared_test_secret_vault();
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
        validate_provider_base_url("openai_compatible-2", "https://api.fireworks.ai/inference/v1")
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
    assert!(validate_provider_base_url("openai", "https://attacker.test?@api.openai.com/v1").is_err());
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
    let err = tauri::async_runtime::block_on(mcp_set_server_secret(McpServerSecretRequest {
        id: "  ".to_string(),
        secret: "token".to_string(),
    }))
    .expect_err("blank id must be rejected");
    assert_eq!(err, "a server id is required");

    let err = tauri::async_runtime::block_on(mcp_set_server_secret(McpServerSecretRequest {
        id: "github".to_string(),
        secret: "  ".to_string(),
    }))
    .expect_err("blank secret must be rejected");
    assert_eq!(err, "a secret is required");

    let err = tauri::async_runtime::block_on(mcp_clear_server_secret(McpServerRequest {
        id: String::new(),
    }))
    .expect_err("blank id must be rejected");
    assert_eq!(err, "a server id is required");

    let err = tauri::async_runtime::block_on(mcp_has_server_secret(McpServerRequest {
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
    let discovered =
        tauri::async_runtime::block_on(list_models_for_provider(&client, &live)).expect("live");
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
    let error = tauri::async_runtime::block_on(list_models_for_provider(&client, &dead))
        .expect_err("a closed port must fail");
    assert_eq!(classify_listing_failure(&error), "unreachable");
}
