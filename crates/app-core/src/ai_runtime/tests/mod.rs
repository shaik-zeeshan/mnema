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
        windows(
            r#"{"data":[{"id":"v","max_model_len":16384},{"id":"x","context_length":200000,"max_input_tokens":100000}]}"#
        ),
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
    assert_eq!(
        serde_json::from_value::<AiRuntimeStatus>(value).unwrap(),
        status
    );

    // No default model → no kind key at all.
    let bare = AiRuntimeSettings {
        default_model: None,
        ..settings
    };
    assert_eq!(
        crate::user_context::worker::default_provider_kind(&bare),
        None
    );
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
    let (_, _, model) =
        expect_local(resolve_engine_config(&settings, None, None).expect("default should resolve"));
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
        ask_ai_engine_prerequisite(&with_endpoints(&dead, &live), &AccessSettings::default()).await,
        Err("local_endpoint_unreachable".to_string())
    );
}

#[test]
fn resolver_falls_back_when_pin_provider_is_not_connected() {
    let mut settings = local_settings();
    settings
        .providers
        .retain(|p| p.kind == AiProviderKind::Ollama);
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
    let (_, endpoint, _) =
        expect_local(resolve_engine_config(&settings, None, None).expect("default should resolve"));
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
        classify_listing_failure(
            "error sending request for url (http://192.168.0.9:8080/v1/models)"
        ),
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

/// Runs a command future to completion the way the Tauri runtime did.
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Runtime::new()
        .expect("test runtime")
        .block_on(future)
}

mod chatgpt;
