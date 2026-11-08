use super::*;

#[test]
fn domain_update_preserves_unrelated_settings_fields() {
    let mut base = default_recording_settings();
    base.capture_microphone = true;
    base.ocr.enabled = false;
    base.appearance = capture_types::AppearanceSetting::Dark;
    base.save_directory = "/tmp/mnema-before".to_string();

    let updated = apply_domain_patch_for_test(
        base.clone(),
        RecordingSettingsDomainPatch::CaptureSources(UpdateCaptureSourceSettingsRequest {
            capture_screen: None,
            capture_microphone: Some(false),
            capture_system_audio: None,
        }),
    )
    .expect("capture source patch should validate");

    assert!(!updated.capture_microphone);
    assert_eq!(updated.capture_screen, base.capture_screen);
    assert_eq!(updated.ocr, base.ocr);
    assert_eq!(updated.appearance, base.appearance);
    assert_eq!(updated.save_directory, base.save_directory);
}

#[test]
fn display_update_sets_time_format_and_preserves_appearance() {
    let mut base = default_recording_settings();
    base.appearance = capture_types::AppearanceSetting::Dark;

    let updated = apply_domain_patch_for_test(
        base.clone(),
        RecordingSettingsDomainPatch::Display(capture_types::UpdateDisplaySettingsRequest {
            time_format: Some(capture_types::TimeFormatSetting::H24),
            ..Default::default()
        }),
    )
    .expect("display patch should validate");

    assert_eq!(updated.time_format, capture_types::TimeFormatSetting::H24);
    assert_eq!(updated.appearance, base.appearance);
}

#[test]
fn semantic_search_model_switch_patch_switches_model_and_preserves_other_fields() {
    let mut base = default_recording_settings();
    base.capture_microphone = true;
    base.save_directory = "/tmp/mnema-before".to_string();
    assert_eq!(
        base.semantic_search.model_id.as_deref(),
        Some("nomic-embed-text-v1.5")
    );

    // Switch to the Multilingual tier via the trusted atomic-switch variant
    // (the path `select_semantic_search_model` uses AFTER it has rebuilt the
    // vec0 table to the new model's dimension).
    let updated = apply_domain_patch_for_test(
        base.clone(),
        RecordingSettingsDomainPatch::SemanticSearchModelSwitch(
            capture_types::UpdateSemanticSearchSettingsRequest {
                enabled: None,
                provider: None,
                model_id: Some(Some("multilingual-e5-small".to_string())),
            },
        ),
    )
    .expect("semantic search model switch should validate");

    assert_eq!(
        updated.semantic_search.model_id.as_deref(),
        Some("multilingual-e5-small")
    );
    // Unrelated fields are untouched.
    assert!(updated.capture_microphone);
    assert_eq!(updated.save_directory, base.save_directory);
    assert_eq!(updated.ocr, base.ocr);
}

#[test]
fn generic_semantic_search_patch_ignores_model_id_and_provider() {
    let mut base = default_recording_settings();
    base.semantic_search.enabled = false;
    let original_model = base.semantic_search.model_id.clone();
    let original_provider = base.semantic_search.provider.clone();
    assert!(original_model.is_some());

    // A generic IPC patch that carries `model_id`/`provider` must NOT change
    // the persisted model or provider (changing them here would re-open the
    // non-atomic dimension split). The honored fields (here `enabled`) still
    // apply, so the patch is non-empty and validates.
    let updated = apply_domain_patch_for_test(
        base.clone(),
        RecordingSettingsDomainPatch::SemanticSearch(
            capture_types::UpdateSemanticSearchSettingsRequest {
                enabled: Some(true),
                provider: Some("some-other-provider".to_string()),
                model_id: Some(Some("bge-m3".to_string())),
            },
        ),
    )
    .expect("generic semantic search patch should validate");

    assert!(updated.semantic_search.enabled, "enabled toggle is honored");
    assert_eq!(
        updated.semantic_search.model_id, original_model,
        "model_id must be ignored by the generic IPC patch"
    );
    assert_eq!(
        updated.semantic_search.provider, original_provider,
        "provider must be ignored by the generic IPC patch"
    );
}

#[test]
fn generic_semantic_search_patch_with_only_model_id_is_rejected_as_empty() {
    // A generic patch whose ONLY fields are the now-ignored `model_id` /
    // `provider` leaves nothing honored, so it is rejected as an empty patch
    // rather than silently no-op'ing the persist.
    let mut base = default_recording_settings();
    let error = apply_domain_patch_to_settings(
        &mut base,
        RecordingSettingsDomainPatch::SemanticSearch(
            capture_types::UpdateSemanticSearchSettingsRequest {
                enabled: None,
                provider: Some("some-other-provider".to_string()),
                model_id: Some(Some("bge-m3".to_string())),
            },
        ),
    )
    .expect_err("a patch carrying only ignored fields must be rejected");
    assert_eq!(error.code, "empty_settings_patch");
    // The persisted model is left untouched by the rejected patch.
    assert_eq!(
        base.semantic_search.model_id.as_deref(),
        Some("nomic-embed-text-v1.5")
    );
}

#[test]
fn semantic_search_model_switch_can_clear_and_toggle() {
    let base = default_recording_settings();

    // An explicit null clears the selected model; disabling the feature flips
    // `enabled`. This is the trusted atomic-switch variant, so model_id is
    // honored.
    let updated = apply_domain_patch_for_test(
        base.clone(),
        RecordingSettingsDomainPatch::SemanticSearchModelSwitch(
            capture_types::UpdateSemanticSearchSettingsRequest {
                enabled: Some(false),
                provider: None,
                model_id: Some(None),
            },
        ),
    )
    .expect("semantic search model switch should validate");

    assert!(!updated.semantic_search.enabled);
    assert_eq!(updated.semantic_search.model_id, None);
}

#[test]
fn empty_semantic_search_patch_is_rejected() {
    let mut base = default_recording_settings();
    let error = apply_domain_patch_to_settings(
        &mut base,
        RecordingSettingsDomainPatch::SemanticSearch(
            capture_types::UpdateSemanticSearchSettingsRequest::default(),
        ),
    )
    .expect_err("an empty patch must be rejected");
    assert_eq!(error.code, "empty_settings_patch");
}

#[test]
fn access_patch_writes_ask_ai_model_and_provider_together() {
    let patch = |model: Option<&str>, provider: Option<&str>| {
        RecordingSettingsDomainPatch::Access(UpdateAccessSettingsRequest {
            ask_ai_model: model.map(str::to_string),
            ask_ai_model_provider: provider.map(str::to_string),
            ..Default::default()
        })
    };
    let set = apply_domain_patch_for_test(
        default_recording_settings(),
        patch(Some(" qwen3:8b "), Some(" ollama ")),
    )
    .expect("access patch should validate");
    assert_eq!(set.access.ask_ai_model.as_deref(), Some("qwen3:8b"));
    assert_eq!(set.access.ask_ai_model_provider.as_deref(), Some("ollama"));

    // A new model without a provider clears the old provider (bare id).
    let bare = apply_domain_patch_for_test(set.clone(), patch(Some("gpt-5"), None))
        .expect("access patch should validate");
    assert_eq!(bare.access.ask_ai_model.as_deref(), Some("gpt-5"));
    assert_eq!(bare.access.ask_ai_model_provider, None);

    // Clearing the model clears its provider too.
    let cleared = apply_domain_patch_for_test(set.clone(), patch(Some(""), Some("ollama")))
        .expect("access patch should validate");
    assert_eq!(cleared.access.ask_ai_model, None);
    assert_eq!(cleared.access.ask_ai_model_provider, None);

    // A patch that doesn't touch the model leaves the pair alone.
    let untouched = apply_domain_patch_for_test(
        set,
        RecordingSettingsDomainPatch::Access(UpdateAccessSettingsRequest {
            ask_ai_enabled: Some(true),
            ..Default::default()
        }),
    )
    .expect("access patch should validate");
    assert_eq!(
        untouched.access.ask_ai_model_provider.as_deref(),
        Some("ollama")
    );
}

#[test]
fn access_domain_update_preserves_unrelated_settings_fields() {
    let mut base = default_recording_settings();
    base.capture_microphone = true;
    base.ocr.enabled = false;
    base.appearance = capture_types::AppearanceSetting::Dark;
    base.save_directory = "/tmp/mnema-before".to_string();

    let updated = apply_domain_patch_for_test(
        base.clone(),
        RecordingSettingsDomainPatch::Access(UpdateAccessSettingsRequest {
            ask_ai_enabled: Some(true),
            ask_ai_max_tool_calls: Some(0),
            ask_ai_web_fetch_enabled: Some(true),
            ask_ai_model: Some("anthropic:claude-opus-4".to_string()),
            ask_ai_model_provider: None,
            quick_recall_auto_answer: Some(capture_types::QuickRecallAutoAnswer::Never),
        }),
    )
    .expect("access patch should validate");

    assert_eq!(
        updated.access.quick_recall_auto_answer,
        capture_types::QuickRecallAutoAnswer::Never
    );
    assert!(updated.access.ask_ai_enabled);
    assert!(updated.access.ask_ai_web_fetch_enabled);
    assert_eq!(updated.access.ask_ai_max_tool_calls, 0);
    assert_eq!(
        updated.access.ask_ai_model.as_deref(),
        Some("anthropic:claude-opus-4")
    );
    assert_eq!(updated.capture_microphone, base.capture_microphone);
    assert_eq!(updated.ocr, base.ocr);
    assert_eq!(updated.appearance, base.appearance);
    assert_eq!(updated.save_directory, base.save_directory);
}

#[test]
fn ai_runtime_patch_flips_master_switch_off_for_wipe_user_context() {
    // Wipe User Context turns the master AI switch off through this exact
    // patch (`UpdateAiRuntimeSettingsRequest { enabled: Some(false), .. }`
    // in `wipe_user_context`); the flip must only touch `enabled`, leaving
    // the connected providers and default model intact for a re-opt-in.
    let mut base = default_recording_settings();
    base.ai_runtime = enabled_ai_runtime_settings();

    let updated = apply_domain_patch_for_test(
        base.clone(),
        RecordingSettingsDomainPatch::AiRuntime(UpdateAiRuntimeSettingsRequest {
            enabled: Some(false),
            ..Default::default()
        }),
    )
    .expect("ai runtime patch should validate");

    assert!(!updated.ai_runtime.enabled);
    assert_eq!(updated.ai_runtime.providers, base.ai_runtime.providers);
    assert_eq!(
        updated.ai_runtime.default_model,
        base.ai_runtime.default_model
    );
}

#[test]
fn ai_runtime_patch_cleans_favorite_and_recent_models() {
    let engine = |provider: &str, model: &str| capture_types::AiEngineRef {
        provider: provider.to_string(),
        model: model.to_string(),
    };
    let mut base = default_recording_settings();
    base.ai_runtime = enabled_ai_runtime_settings();

    // Favorites: trimmed, de-duplicated, blank ids and refs to a provider
    // that isn't connected dropped; star order kept. Recents: same, capped.
    let updated = apply_domain_patch_for_test(
        base,
        RecordingSettingsDomainPatch::AiRuntime(UpdateAiRuntimeSettingsRequest {
            favorite_models: Some(vec![
                engine("anthropic", " claude-opus-5-5 "),
                engine("ollama", "qwen3:8b"),
                engine("anthropic", "claude-opus-5-5"),
                engine("anthropic", "  "),
                engine("anthropic", "claude-haiku-4-5"),
            ]),
            recent_models: Some(
                (0..8)
                    .map(|i| engine("anthropic", &format!("m{i}")))
                    .collect(),
            ),
            ..Default::default()
        }),
    )
    .expect("ai runtime patch should validate");
    assert_eq!(
        updated.ai_runtime.favorite_models,
        vec![
            engine("anthropic", "claude-opus-5-5"),
            engine("anthropic", "claude-haiku-4-5")
        ]
    );
    let recents: Vec<&str> = updated
        .ai_runtime
        .recent_models
        .iter()
        .map(|e| e.model.as_str())
        .collect();
    assert_eq!(recents, ["m0", "m1", "m2", "m3", "m4"]);
    // The default model is untouched by a favorites-only patch.
    assert_eq!(
        updated.ai_runtime.default_model,
        Some(engine("anthropic", "claude-haiku-4-5"))
    );

    // Removing the provider drops its favorites and recents too.
    let removed = apply_domain_patch_for_test(
        updated,
        RecordingSettingsDomainPatch::AiRuntime(UpdateAiRuntimeSettingsRequest {
            providers: Some(Vec::new()),
            ..Default::default()
        }),
    )
    .expect("ai runtime patch should validate");
    assert!(removed.ai_runtime.favorite_models.is_empty());
    assert!(removed.ai_runtime.recent_models.is_empty());
}

#[test]
fn ai_runtime_patch_keeps_favorites_unless_sent_and_keys_them_by_provider() {
    let engine = |provider: &str, model: &str| capture_types::AiEngineRef {
        provider: provider.to_string(),
        model: model.to_string(),
    };
    let mut base = default_recording_settings();
    base.ai_runtime = enabled_ai_runtime_settings();
    base.ai_runtime
        .providers
        .push(capture_types::AiProviderConfig {
            id: "openrouter".to_string(),
            kind: capture_types::AiProviderKind::OpenaiCompatible,
            label: String::new(),
            base_url: String::new(),
        });
    // The same model id under two providers is two favorites, not a duplicate.
    let starred = apply_domain_patch_for_test(
        base,
        RecordingSettingsDomainPatch::AiRuntime(UpdateAiRuntimeSettingsRequest {
            favorite_models: Some(vec![engine("anthropic", "x"), engine("openrouter", "x")]),
            recent_models: Some(vec![engine("openrouter", "x")]),
            ..Default::default()
        }),
    )
    .expect("ai runtime patch should validate");
    assert_eq!(
        starred.ai_runtime.favorite_models,
        vec![engine("anthropic", "x"), engine("openrouter", "x")]
    );

    // Settings' own autosave (providers + default, no favorites/recents)
    // leaves both lists alone; removing ONE provider drops only its refs.
    let mut settings_save = starred.ai_runtime.clone();
    settings_save.providers.retain(|p| p.id == "anthropic");
    let after = apply_domain_patch_for_test(
        starred,
        RecordingSettingsDomainPatch::AiRuntime(UpdateAiRuntimeSettingsRequest {
            enabled: Some(true),
            providers: Some(settings_save.providers),
            default_model: Some(settings_save.default_model),
            ..Default::default()
        }),
    )
    .expect("ai runtime patch should validate");
    assert_eq!(
        after.ai_runtime.favorite_models,
        vec![engine("anthropic", "x")]
    );
    assert!(after.ai_runtime.recent_models.is_empty());
}

#[test]
fn ai_runtime_patch_replaces_providers_and_clears_default_model() {
    let mut base = default_recording_settings();
    base.ai_runtime = enabled_ai_runtime_settings();

    let updated = apply_domain_patch_for_test(
        base,
        RecordingSettingsDomainPatch::AiRuntime(UpdateAiRuntimeSettingsRequest {
            enabled: None,
            providers: Some(vec![
                capture_types::AiProviderConfig {
                    id: "ollama".to_string(),
                    kind: capture_types::AiProviderKind::Ollama,
                    label: String::new(),
                    base_url: " http://localhost:11434 ".to_string(),
                },
                // A second instance of the SAME kind with a distinct id is
                // kept — same-kind providers coexist (e.g. two local boxes).
                capture_types::AiProviderConfig {
                    id: "ollama-2".to_string(),
                    kind: capture_types::AiProviderKind::Ollama,
                    label: "Other box".to_string(),
                    base_url: "http://other:11434".to_string(),
                },
                // A duplicate *instance id* is dropped (first wins).
                capture_types::AiProviderConfig {
                    id: "ollama".to_string(),
                    kind: capture_types::AiProviderKind::Ollama,
                    label: String::new(),
                    base_url: "http://dupe:11434".to_string(),
                },
            ]),
            // Explicit `null` over the wire clears the default model.
            default_model: Some(None),
            mcp_servers: None,
            favorite_models: None,
            recent_models: None,
        }),
    )
    .expect("ai runtime patch should validate");

    assert!(updated.ai_runtime.enabled);
    assert_eq!(
        updated.ai_runtime.providers,
        vec![
            capture_types::AiProviderConfig {
                id: "ollama".to_string(),
                kind: capture_types::AiProviderKind::Ollama,
                label: String::new(),
                base_url: "http://localhost:11434".to_string(),
            },
            capture_types::AiProviderConfig {
                id: "ollama-2".to_string(),
                kind: capture_types::AiProviderKind::Ollama,
                label: "Other box".to_string(),
                base_url: "http://other:11434".to_string(),
            },
        ]
    );
    assert_eq!(updated.ai_runtime.default_model, None);
}

#[test]
fn ai_runtime_patch_persists_mcp_servers_on_some() {
    // The MCP connector list is what reconcile/warm/turn all read. The other
    // ai_runtime patch tests pass `mcp_servers: None`, so the wholesale-replace
    // branch (native_capture_settings.rs) never runs — a regression that
    // dropped the write would go unnoticed. Exercise the `Some` branch.
    let mut base = default_recording_settings();
    base.ai_runtime = enabled_ai_runtime_settings();

    let server = capture_types::McpServerConfig {
        id: "connector".to_string(),
        label: "GitHub".to_string(),
        enabled: true,
        transport: capture_types::McpTransport::Stdio,
        auth_mode: capture_types::McpAuthMode::Bearer,
        command: Some("npx".to_string()),
        args: vec![
            "-y".to_string(),
            "@modelcontextprotocol/server-github".to_string(),
        ],
        env: Vec::new(),
        url: None,
        secret_env_name: Some("GITHUB_TOKEN".to_string()),
        enabled_tools: Some(vec!["search".to_string()]),
    };

    let updated = apply_domain_patch_for_test(
        base,
        RecordingSettingsDomainPatch::AiRuntime(UpdateAiRuntimeSettingsRequest {
            enabled: None,
            providers: None,
            default_model: None,
            mcp_servers: Some(vec![server.clone()]),
            favorite_models: None,
            recent_models: None,
        }),
    )
    .expect("ai runtime patch should validate");

    assert_eq!(updated.ai_runtime.mcp_servers, vec![server]);
}

#[test]
fn ai_runtime_patch_mcp_servers_none_leaves_existing_list_unchanged() {
    // `mcp_servers: None` means "leave unchanged", not "clear". This exact
    // partial-update shape is reachable via `wipe_user_context`, whose
    // `enabled: Some(false)` patch must not drop configured connectors.
    let mut base = default_recording_settings();
    base.ai_runtime = enabled_ai_runtime_settings();
    base.ai_runtime.mcp_servers = vec![capture_types::McpServerConfig {
        id: "connector".to_string(),
        label: "GitHub".to_string(),
        enabled: true,
        transport: capture_types::McpTransport::Stdio,
        auth_mode: capture_types::McpAuthMode::Bearer,
        command: Some("npx".to_string()),
        args: vec![
            "-y".to_string(),
            "@modelcontextprotocol/server-github".to_string(),
        ],
        env: Vec::new(),
        url: None,
        secret_env_name: Some("GITHUB_TOKEN".to_string()),
        enabled_tools: Some(vec!["search".to_string()]),
    }];

    let updated = apply_domain_patch_for_test(
        base.clone(),
        RecordingSettingsDomainPatch::AiRuntime(UpdateAiRuntimeSettingsRequest {
            enabled: Some(false),
            ..Default::default()
        }),
    )
    .expect("ai runtime patch should validate");

    assert!(!updated.ai_runtime.enabled);
    assert_eq!(
        updated.ai_runtime.mcp_servers, base.ai_runtime.mcp_servers,
        "mcp_servers: None must leave the existing connector list unchanged"
    );
}

#[test]
fn empty_access_domain_patch_is_rejected() {
    let error = apply_domain_patch_for_test(
        default_recording_settings(),
        RecordingSettingsDomainPatch::Access(UpdateAccessSettingsRequest::default()),
    )
    .expect_err("empty access patch should be rejected");

    assert_eq!(error.code, "empty_settings_patch");
}

#[test]
fn empty_domain_patch_is_rejected() {
    let error = apply_domain_patch_for_test(
        default_recording_settings(),
        RecordingSettingsDomainPatch::CaptureSources(UpdateCaptureSourceSettingsRequest::default()),
    )
    .expect_err("empty patch should be rejected");

    assert_eq!(error.code, "empty_settings_patch");
}

#[test]
fn capture_sources_domain_allows_system_audio_without_screen() {
    let settings = apply_domain_patch_for_test(
        default_recording_settings(),
        RecordingSettingsDomainPatch::CaptureSources(UpdateCaptureSourceSettingsRequest {
            capture_screen: Some(false),
            capture_microphone: Some(true),
            capture_system_audio: Some(true),
        }),
    )
    .expect("system audio runs on its own tap, so it needs no screen capture");

    assert!(!settings.capture_screen);
    assert!(settings.capture_system_audio);
}

#[test]
fn inactivity_domain_rejects_detector_off_when_system_audio_transcription_enabled() {
    let mut base = default_recording_settings();
    base.transcription.enabled = true;
    base.transcription.system_audio_enabled = true;

    let error = apply_domain_patch_for_test(
        base,
        RecordingSettingsDomainPatch::Inactivity(UpdateInactivitySettingsRequest {
            audio_speech_detection: Some(capture_types::AudioSpeechDetectionSettings {
                detector: capture_types::AudioSpeechDetector::Off,
            }),
            ..UpdateInactivitySettingsRequest::default()
        }),
    )
    .expect_err("detector off should be rejected for system audio transcription");

    assert_eq!(error.code, "invalid_recording_settings");
    assert_eq!(
        error.message,
        "audioSpeechDetection.detector cannot be off while transcription.systemAudioEnabled is true"
    );
}

#[test]
fn processing_domain_rejects_system_audio_transcription_when_detector_is_off() {
    let mut base = default_recording_settings();
    base.audio_speech_detection.detector = capture_types::AudioSpeechDetector::Off;
    base.microphone_vad_adapter = capture_types::AudioSpeechDetector::Off;
    base.transcription.system_audio_enabled = false;

    let mut transcription = base.transcription.clone();
    transcription.system_audio_enabled = true;

    let error = apply_domain_patch_for_test(
        base,
        RecordingSettingsDomainPatch::Processing(UpdateProcessingSettingsRequest {
            transcription: Some(transcription),
            ..UpdateProcessingSettingsRequest::default()
        }),
    )
    .expect_err("system audio transcription should require a speech detector");

    assert_eq!(error.code, "invalid_recording_settings");
    assert_eq!(
        error.message,
        "audioSpeechDetection.detector cannot be off while transcription.systemAudioEnabled is true"
    );
}

#[test]
fn storage_domain_preserves_non_storage_settings() {
    let mut base = default_recording_settings();
    base.capture_microphone = true;
    base.developer_options_enabled = true;
    base.ocr.enabled = false;

    let updated = apply_domain_patch_for_test(
        base.clone(),
        RecordingSettingsDomainPatch::Storage(UpdateStorageSettingsRequest {
            save_directory: Some("/tmp/mnema-after".to_string()),
            retention_policy: Some(RetentionPolicy::Days7),
        }),
    )
    .expect("storage patch should validate");

    assert_eq!(updated.save_directory, "/tmp/mnema-after");
    assert_eq!(updated.retention_policy, RetentionPolicy::Days7);
    assert_eq!(updated.capture_microphone, base.capture_microphone);
    assert_eq!(
        updated.developer_options_enabled,
        base.developer_options_enabled
    );
    assert_eq!(updated.ocr, base.ocr);
}

#[test]
fn developer_domain_preserves_processing_and_capture_fields() {
    let mut base = default_recording_settings();
    base.capture_microphone = true;
    base.ocr.enabled = false;
    base.preview_cache_ttl_seconds = 120;

    let updated = apply_domain_patch_for_test(
        base.clone(),
        RecordingSettingsDomainPatch::Developer(UpdateDeveloperSettingsRequest {
            developer_options_enabled: Some(true),
        }),
    )
    .expect("developer patch should validate");

    assert!(updated.developer_options_enabled);
    assert_eq!(updated.capture_microphone, base.capture_microphone);
    assert_eq!(updated.ocr, base.ocr);
    assert_eq!(
        updated.preview_cache_ttl_seconds,
        base.preview_cache_ttl_seconds
    );
}
