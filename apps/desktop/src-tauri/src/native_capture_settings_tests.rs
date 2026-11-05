use super::*;
use capture_types::default_inactivity_activity_mode;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(label: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("desktop-settings-{label}-{unique}"));

        fs::create_dir_all(&path).expect("test directory should be created");

        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn load_recording_settings_from_path_returns_none_for_missing_file() {
    let dir = TestDir::new("missing");

    assert!(load_recording_settings_from_path(&dir.path().join("missing.json")).is_none());
}

#[test]
fn load_recording_settings_from_path_returns_none_for_invalid_file() {
    let dir = TestDir::new("invalid");
    let path = dir.path().join("recording-settings.json");
    fs::write(&path, "not valid json").expect("invalid file should write");

    assert!(load_recording_settings_from_path(&path).is_none());
}

#[test]
fn load_recording_settings_from_path_or_default_uses_defaults_for_missing_file() {
    let dir = TestDir::new("missing-default");

    assert_eq!(
        load_recording_settings_from_path_or_default(&dir.path().join("missing.json"))
            .save_directory,
        default_recording_settings().save_directory
    );
}

#[test]
fn load_recording_settings_from_path_or_default_uses_defaults_for_invalid_file() {
    let dir = TestDir::new("invalid-default");
    let path = dir.path().join("recording-settings.json");
    fs::write(&path, "not valid json").expect("invalid file should write");

    assert_eq!(
        load_recording_settings_from_path_or_default(&path).save_directory,
        default_recording_settings().save_directory
    );
}

#[test]
fn default_recording_settings_disable_ask_ai_access() {
    assert!(!default_recording_settings().access.ask_ai_enabled);
    assert!(!default_recording_settings().access.ask_ai_web_fetch_enabled);
}

#[test]
fn default_recording_settings_disable_developer_options() {
    assert!(!default_recording_settings().developer_options_enabled);
}

#[test]
fn default_recording_settings_use_default_microphone_vad_adapter() {
    assert_eq!(
        default_recording_settings().microphone_vad_adapter,
        capture_types::default_microphone_vad_adapter()
    );
}

#[test]
fn load_recording_settings_from_path_preserves_developer_options_flag() {
    let dir = TestDir::new("developer-options-enabled");
    let path = dir.path().join("recording-settings.json");
    let mut settings = default_recording_settings();
    settings.developer_options_enabled = true;

    fs::write(
        &path,
        serde_json::to_string_pretty(&settings).expect("settings should serialize"),
    )
    .expect("settings file should write");

    let loaded =
        load_recording_settings_from_path(&path).expect("settings should load from disk");

    assert!(loaded.developer_options_enabled);
}

#[test]
fn load_recording_settings_from_path_preserves_ask_ai_access_flag() {
    let dir = TestDir::new("ask-ai-enabled");
    let path = dir.path().join("recording-settings.json");
    let mut settings = default_recording_settings();
    settings.access.ask_ai_enabled = true;

    fs::write(
        &path,
        serde_json::to_string_pretty(&settings).expect("settings should serialize"),
    )
    .expect("settings file should write");

    let loaded =
        load_recording_settings_from_path(&path).expect("settings should load from disk");

    assert!(loaded.access.ask_ai_enabled);
}

#[test]
fn load_recording_settings_from_path_migrates_legacy_microphone_vad_adapter_to_shared_detector()
{
    let dir = TestDir::new("microphone-vad-adapter");
    let path = dir.path().join("recording-settings.json");

    fs::write(
        &path,
        r#"{
            "captureScreen": true,
            "captureMicrophone": true,
            "captureSystemAudio": false,
            "segmentDurationSeconds": 60,
            "screenFrameRate": 1,
            "screenResolution": { "mode": "preset", "preset": "original" },
            "videoBitrate": { "mode": "preset", "preset": "medium" },
            "saveDirectory": "/tmp",
            "autoStart": false,
            "pauseCaptureOnInactivity": true,
            "idleTimeoutSeconds": 10,
            "microphoneActivitySensitivity": 50,
            "systemAudioActivitySensitivity": 50,
            "microphoneVadAdapter": "webrtc",
            "activityMode": "system_input_or_screen"
        }"#,
    )
    .expect("settings file should write");

    let loaded =
        load_recording_settings_from_path(&path).expect("settings should load from disk");

    assert_eq!(
        loaded.audio_speech_detection.detector,
        capture_types::AudioSpeechDetector::Webrtc
    );
    assert_eq!(
        loaded.microphone_vad_adapter,
        capture_types::AudioSpeechDetector::Webrtc
    );
}

#[test]
fn load_recording_settings_from_path_preserves_saved_resolution_when_backend_unsupported() {
    let dir = TestDir::new("non-original-resolution");
    let path = dir.path().join("recording-settings.json");
    let mut settings = default_recording_settings();
    settings.save_directory = "/tmp/custom-mnema".to_string();
    settings.auto_start = true;
    settings.screen_resolution = ScreenResolution::Preset {
        preset: ScreenResolutionPreset::P720,
    };

    fs::write(
        &path,
        serde_json::to_string_pretty(&settings).expect("settings should serialize"),
    )
    .expect("settings file should write");

    let loaded =
        load_recording_settings_from_path(&path).expect("settings should load from disk");

    assert_eq!(loaded.save_directory, "/tmp/custom-mnema");
    assert!(loaded.auto_start);
    assert_eq!(
        loaded.screen_resolution,
        ScreenResolution::Preset {
            preset: ScreenResolutionPreset::P720
        }
    );
}

fn apply_domain_patch_for_test(
    mut settings: RecordingSettings,
    patch: RecordingSettingsDomainPatch,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    apply_domain_patch_to_settings(&mut settings, patch)?;
    validate_recording_settings_with_resolution_support(
        recording_settings_request_from_settings(settings),
        true,
    )
}

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

/// Provider-centric AI runtime settings with the master switch ON, for the
/// wipe-flips-switch and AiRuntime patch tests.
fn enabled_ai_runtime_settings() -> AiRuntimeSettings {
    AiRuntimeSettings {
        enabled: true,
        providers: vec![capture_types::AiProviderConfig {
            id: "anthropic".to_string(),
            kind: capture_types::AiProviderKind::Anthropic,
            label: String::new(),
            base_url: String::new(),
        }],
        default_model: Some(capture_types::AiEngineRef {
            provider: "anthropic".to_string(),
            model: "claude-haiku-4-5".to_string(),
        }),
        mcp_servers: Vec::new(),
        favorite_models: Vec::new(),
        recent_models: Vec::new(),
    }
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
    base.ai_runtime.providers.push(capture_types::AiProviderConfig {
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
    assert_eq!(after.ai_runtime.favorite_models, vec![engine("anthropic", "x")]);
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
fn load_recording_settings_from_path_migrates_legacy_engine_centric_ai_runtime() {
    // ADR 0034: an existing recording-settings.json with the old
    // engine-centric aiRuntime block loads into the provider-centric shape
    // (deserialization-level migration; the next save writes only the new
    // shape).
    let dir = TestDir::new("legacy-ai-runtime");
    let path = dir.path().join("recording-settings.json");

    fs::write(
        &path,
        r#"{
            "captureScreen": true,
            "captureMicrophone": false,
            "captureSystemAudio": false,
            "segmentDurationSeconds": 60,
            "screenFrameRate": 1,
            "screenResolution": { "mode": "preset", "preset": "original" },
            "videoBitrate": { "mode": "preset", "preset": "medium" },
            "saveDirectory": "/tmp",
            "autoStart": false,
            "pauseCaptureOnInactivity": true,
            "idleTimeoutSeconds": 10,
            "microphoneActivitySensitivity": 50,
            "systemAudioActivitySensitivity": 50,
            "activityMode": "system_input_or_screen",
            "aiRuntime": {
                "enabled": true,
                "engineKind": "cloud",
                "cloudProvider": "anthropic",
                "cloudModel": "claude-haiku-4-5",
                "cloudBaseUrl": "",
                "localKind": "ollama",
                "localEndpoint": "http://localhost:11434",
                "localModel": "",
                "additionalEngines": [
                    {
                        "engineKind": "local",
                        "cloudProvider": "openai",
                        "cloudModel": "",
                        "cloudBaseUrl": "",
                        "localKind": "ollama",
                        "localEndpoint": "http://localhost:11434",
                        "localModel": "llama3.2"
                    }
                ]
            }
        }"#,
    )
    .expect("settings file should write");

    let loaded =
        load_recording_settings_from_path(&path).expect("settings should load from disk");

    assert!(loaded.ai_runtime.enabled);
    assert_eq!(
        loaded.ai_runtime.providers,
        vec![
            capture_types::AiProviderConfig {
                id: "anthropic".to_string(),
                kind: capture_types::AiProviderKind::Anthropic,
                label: String::new(),
                base_url: String::new(),
            },
            capture_types::AiProviderConfig {
                id: "ollama".to_string(),
                kind: capture_types::AiProviderKind::Ollama,
                label: String::new(),
                base_url: "http://localhost:11434".to_string(),
            },
        ]
    );
    assert_eq!(
        loaded.ai_runtime.default_model,
        Some(capture_types::AiEngineRef {
            provider: "anthropic".to_string(),
            model: "claude-haiku-4-5".to_string(),
        })
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
        RecordingSettingsDomainPatch::CaptureSources(
            UpdateCaptureSourceSettingsRequest::default(),
        ),
    )
    .expect_err("empty patch should be rejected");

    assert_eq!(error.code, "empty_settings_patch");
}

// A sources save that changes nothing never asks the capture lock, which a
// stop can hold for seconds while these saves run on the main thread.
#[test]
fn only_a_sources_patch_that_changes_them_consults_the_capture() {
    let current = default_recording_settings();
    let patch = |microphone: bool| {
        RecordingSettingsDomainPatch::CaptureSources(UpdateCaptureSourceSettingsRequest {
            capture_screen: Some(current.capture_screen),
            capture_microphone: Some(microphone),
            capture_system_audio: None,
        })
    };

    assert!(!domain_patch_changes_sources(
        &current,
        &patch(current.capture_microphone)
    ));
    assert!(domain_patch_changes_sources(
        &current,
        &patch(!current.capture_microphone)
    ));
    assert!(!domain_patch_changes_sources(
        &current,
        &RecordingSettingsDomainPatch::Developer(Default::default())
    ));
}

#[test]
fn capture_sources_change_is_refused_only_while_capture_is_in_use() {
    let previous = default_recording_settings();
    let mut next = previous.clone();
    next.capture_microphone = !next.capture_microphone;

    let refused = ensure_capture_sources_unlocked(&previous, &next, true)
        .expect_err("a source change mid-session never reaches the capture");
    assert_eq!(refused.code, CAPTURE_SOURCES_LOCKED);
    assert!(ensure_capture_sources_unlocked(&previous, &next, false).is_ok());
    // Re-saving the same sources mid-session is harmless.
    assert!(ensure_capture_sources_unlocked(&previous, &previous, true).is_ok());
}

// The start/save counter protocol end to end, through the full save's
// check. One test, because the counters are process-wide.
#[test]
fn a_full_save_refuses_a_sources_change_while_a_start_overlaps_it() {
    let state = RecordingSettingsState::default();
    let previous = default_recording_settings();
    let mut next = previous.clone();
    next.capture_microphone = !next.capture_microphone;
    let check = |next: &RecordingSettings, capture_use: &CaptureUse| {
        check_full_recording_settings_update(&previous, next, &previous.privacy, capture_use)
    };

    // A start in flight when the save begins.
    let start = recording_settings_for_start(&state);
    let capture_use =
        CaptureUse::before_settings_lock(|| panic!("a start in flight answers first"));
    let refused =
        check(&next, &capture_use).expect_err("the start may have read the old sources");
    assert_eq!(refused.code, CAPTURE_SOURCES_LOCKED);
    assert!(
        check(&previous, &capture_use).is_ok(),
        "unchanged sources always pass"
    );
    drop(start);

    // A start begun between the two halves read the old sources.
    let capture_use = CaptureUse::before_settings_lock(|| false);
    let start = recording_settings_for_start(&state);
    assert!(check(&next, &capture_use).is_err());
    drop(start);

    // A running session.
    assert!(check(&next, &CaptureUse::before_settings_lock(|| true)).is_err());

    // Nothing overlaps: the change saves.
    assert!(check(&next, &CaptureUse::before_settings_lock(|| false)).is_ok());
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

#[test]
fn validate_privacy_settings_preserves_app_bundle_id_casing_and_dedupes_exact_ids() {
    let mut privacy = default_privacy_settings();
    privacy.excluded_apps = vec![
        capture_metadata::ExcludedAppEntry {
            id: "app-a".to_string(),
            enabled: true,
            bundle_id: " com.apple.Safari ".to_string(),
            display_name: "Safari".to_string(),
        },
        capture_metadata::ExcludedAppEntry {
            id: "app-b".to_string(),
            enabled: true,
            bundle_id: "com.apple.Safari".to_string(),
            display_name: "Safari duplicate exact".to_string(),
        },
        capture_metadata::ExcludedAppEntry {
            id: "app-c".to_string(),
            enabled: true,
            bundle_id: "com.apple.safari".to_string(),
            display_name: "Different-case bundle ID".to_string(),
        },
    ];

    let normalized = validate_privacy_settings(privacy).expect("privacy should validate");

    assert_eq!(normalized.excluded_apps.len(), 2);
    assert_eq!(normalized.excluded_apps[0].id, "app-a");
    assert_eq!(normalized.excluded_apps[0].bundle_id, "com.apple.Safari");
    assert_eq!(normalized.excluded_apps[1].id, "app-c");
    assert_eq!(normalized.excluded_apps[1].bundle_id, "com.apple.safari");
}

// A rule for an app the user has not installed yet has no bundle id, only a
// typed name. Validation runs on every settings write and on load, so
// dropping it here would silently delete the rule the user just made.
#[test]
fn validate_privacy_settings_keeps_a_pending_rule_and_dedupes_it_by_display_name() {
    let mut privacy = default_privacy_settings();
    privacy.excluded_apps = vec![
        capture_metadata::ExcludedAppEntry {
            id: "app-pending".to_string(),
            enabled: false,
            bundle_id: "  ".to_string(),
            display_name: " Figma ".to_string(),
        },
        capture_metadata::ExcludedAppEntry {
            id: "app-pending-duplicate".to_string(),
            enabled: true,
            bundle_id: String::new(),
            display_name: "figma".to_string(),
        },
        capture_metadata::ExcludedAppEntry {
            id: "app-nameless".to_string(),
            enabled: true,
            bundle_id: String::new(),
            display_name: "   ".to_string(),
        },
    ];

    let normalized = validate_privacy_settings(privacy).expect("privacy should validate");

    assert_eq!(normalized.excluded_apps.len(), 1);
    assert_eq!(normalized.excluded_apps[0].id, "app-pending");
    assert!(normalized.excluded_apps[0].bundle_id.is_empty());
    assert_eq!(normalized.excluded_apps[0].display_name, " Figma ");
    assert!(!normalized.excluded_apps[0].enabled);
}

#[test]
fn validate_recording_settings_normalizes_microphone_vad_adapter_from_shared_detector() {
    let settings = validate_recording_settings_with_resolution_support(
        UpdateRecordingSettingsRequest {
            capture_screen: true,
            capture_microphone: true,
            capture_system_audio: false,
            segment_duration_seconds: 60,
            screen_frame_rate: 1.0,
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::Original,
            },
            video_bitrate: default_video_bitrate(),
            save_directory: "/tmp".to_string(),
            auto_start: false,
            developer_options_enabled: false,
            preview_cache_ttl_seconds: default_preview_cache_ttl_seconds(),
            follow_timeline_live: false,
            retention_policy: RetentionPolicy::Never,
            appearance: default_appearance(),
            time_format: Default::default(),
            ocr: default_ocr_settings(),
            transcription: default_audio_transcription_settings(),
            speaker_analysis: default_speaker_analysis_settings(),
            audio_speech_detection: capture_types::AudioSpeechDetectionSettings {
                detector: capture_types::AudioSpeechDetector::Off,
            },
            metadata: default_metadata_settings(),
            privacy: default_privacy_settings(),
            access: AccessSettings::default(),
            ai_runtime: AiRuntimeSettings::default(),
            user_context: UserContextSettings::default(),
            semantic_search: capture_types::default_semantic_search_settings(),
            pause_capture_on_inactivity: true,
            idle_timeout_seconds: 10,
            microphone_activity_sensitivity: 50,
            system_audio_activity_sensitivity: 50,
            microphone_vad_adapter: capture_types::MicrophoneVadAdapter::Webrtc,
            inactivity_activity_mode: default_inactivity_activity_mode(),
        },
        true,
    )
    .expect("settings should validate");

    assert_eq!(
        settings.microphone_vad_adapter,
        capture_types::AudioSpeechDetector::Off
    );
}

#[test]
fn validate_recording_settings_preserves_ocr_provider_specific_defaults() {
    let mut ocr = default_ocr_settings();
    ocr.provider = capture_types::OcrProvider::Tesseract;
    ocr.model_id = None;
    ocr.language = Some(" eng ".to_string());
    ocr.recognition_mode = capture_types::OcrRecognitionMode::Accurate;
    ocr.language_correction = true;
    ocr.tesseract_upscale_factor = 2;
    ocr.tesseract_char_whitelist = Some(" 0123456789 ".to_string());
    ocr.tesseract_page_segmentation_mode =
        capture_types::OcrTesseractPageSegmentationMode::SparseText;
    ocr.tesseract_preprocess_mode = capture_types::OcrTesseractPreprocessMode::Thresholded;

    let settings = validate_recording_settings_with_resolution_support(
        UpdateRecordingSettingsRequest {
            capture_screen: true,
            capture_microphone: false,
            capture_system_audio: false,
            segment_duration_seconds: 60,
            screen_frame_rate: 1.0,
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::Original,
            },
            video_bitrate: default_video_bitrate(),
            save_directory: "/tmp".to_string(),
            auto_start: false,
            developer_options_enabled: false,
            preview_cache_ttl_seconds: default_preview_cache_ttl_seconds(),
            follow_timeline_live: false,
            retention_policy: RetentionPolicy::Never,
            appearance: default_appearance(),
            time_format: Default::default(),
            ocr,
            transcription: default_audio_transcription_settings(),
            speaker_analysis: default_speaker_analysis_settings(),
            audio_speech_detection: default_audio_speech_detection_settings(),
            metadata: default_metadata_settings(),
            privacy: default_privacy_settings(),
            access: AccessSettings::default(),
            ai_runtime: AiRuntimeSettings::default(),
            user_context: UserContextSettings::default(),
            semantic_search: capture_types::default_semantic_search_settings(),
            pause_capture_on_inactivity: true,
            idle_timeout_seconds: 10,
            microphone_activity_sensitivity: 50,
            system_audio_activity_sensitivity: 50,
            microphone_vad_adapter: capture_types::default_microphone_vad_adapter(),
            inactivity_activity_mode: default_inactivity_activity_mode(),
        },
        true,
    )
    .expect("settings should validate");

    assert_eq!(settings.ocr.provider, capture_types::OcrProvider::Tesseract);
    assert_eq!(
        settings.ocr.model_id.as_deref(),
        Some(ocr::DEFAULT_TESSERACT_MODEL_ID)
    );
    assert_eq!(
        settings.ocr.language.as_deref(),
        Some(ocr::DEFAULT_TESSERACT_LANGUAGE)
    );
    assert_eq!(
        settings.ocr.recognition_mode,
        capture_types::OcrRecognitionMode::Fast
    );
    assert!(!settings.ocr.language_correction);
    assert_eq!(
        settings.ocr.tesseract_page_segmentation_mode,
        capture_types::OcrTesseractPageSegmentationMode::SparseText
    );
    assert_eq!(
        settings.ocr.tesseract_preprocess_mode,
        capture_types::OcrTesseractPreprocessMode::Thresholded
    );
    assert_eq!(settings.ocr.tesseract_upscale_factor, 2);
    assert_eq!(
        settings.ocr.tesseract_char_whitelist.as_deref(),
        Some("0123456789")
    );
}

#[test]
fn validate_recording_settings_normalizes_legacy_paddle_ocr_to_default_provider() {
    let mut ocr_settings = default_ocr_settings();
    ocr_settings.provider = capture_types::OcrProvider::PaddleOcr;
    ocr_settings.model_id = Some(ocr::DEFAULT_PADDLE_OCR_MODEL_ID.to_string());
    ocr_settings.language = Some(ocr::DEFAULT_PADDLE_OCR_LANGUAGE.to_string());

    let settings = validate_recording_settings_with_resolution_support(
        UpdateRecordingSettingsRequest {
            capture_screen: true,
            capture_microphone: false,
            capture_system_audio: false,
            segment_duration_seconds: 60,
            screen_frame_rate: 1.0,
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::Original,
            },
            video_bitrate: default_video_bitrate(),
            save_directory: "/tmp".to_string(),
            auto_start: false,
            developer_options_enabled: false,
            preview_cache_ttl_seconds: default_preview_cache_ttl_seconds(),
            follow_timeline_live: false,
            retention_policy: RetentionPolicy::Never,
            appearance: default_appearance(),
            time_format: Default::default(),
            ocr: ocr_settings,
            transcription: default_audio_transcription_settings(),
            speaker_analysis: default_speaker_analysis_settings(),
            audio_speech_detection: default_audio_speech_detection_settings(),
            metadata: default_metadata_settings(),
            privacy: default_privacy_settings(),
            access: AccessSettings::default(),
            ai_runtime: AiRuntimeSettings::default(),
            user_context: UserContextSettings::default(),
            semantic_search: capture_types::default_semantic_search_settings(),
            pause_capture_on_inactivity: true,
            idle_timeout_seconds: 10,
            microphone_activity_sensitivity: 50,
            system_audio_activity_sensitivity: 50,
            microphone_vad_adapter: capture_types::default_microphone_vad_adapter(),
            inactivity_activity_mode: default_inactivity_activity_mode(),
        },
        true,
    )
    .expect("legacy PaddleOCR settings should normalize");

    assert_eq!(settings.ocr, default_ocr_settings());
}

#[test]
fn validate_recording_settings_preserves_transcription_update() {
    let mut transcription = default_audio_transcription_settings();
    transcription.provider = capture_types::AudioTranscriptionProvider::Parakeet;
    transcription.model_id = Some("parakeet-tdt-0.6b-v3-onnx".to_string());
    transcription.language = " en ".to_string();

    let settings = validate_recording_settings_with_resolution_support(
        UpdateRecordingSettingsRequest {
            capture_screen: true,
            capture_microphone: true,
            capture_system_audio: false,
            segment_duration_seconds: 60,
            screen_frame_rate: 1.0,
            screen_resolution: ScreenResolution::Preset {
                preset: ScreenResolutionPreset::Original,
            },
            video_bitrate: default_video_bitrate(),
            save_directory: "/tmp".to_string(),
            auto_start: false,
            developer_options_enabled: false,
            preview_cache_ttl_seconds: default_preview_cache_ttl_seconds(),
            follow_timeline_live: false,
            retention_policy: RetentionPolicy::Never,
            appearance: default_appearance(),
            time_format: Default::default(),
            ocr: default_ocr_settings(),
            transcription,
            speaker_analysis: default_speaker_analysis_settings(),
            audio_speech_detection: default_audio_speech_detection_settings(),
            metadata: default_metadata_settings(),
            privacy: default_privacy_settings(),
            access: AccessSettings::default(),
            ai_runtime: AiRuntimeSettings::default(),
            user_context: UserContextSettings::default(),
            semantic_search: capture_types::default_semantic_search_settings(),
            pause_capture_on_inactivity: true,
            idle_timeout_seconds: 10,
            microphone_activity_sensitivity: 50,
            system_audio_activity_sensitivity: 50,
            microphone_vad_adapter: capture_types::default_microphone_vad_adapter(),
            inactivity_activity_mode: default_inactivity_activity_mode(),
        },
        true,
    )
    .expect("settings should validate");

    assert_eq!(
        settings.transcription.provider,
        capture_types::AudioTranscriptionProvider::Parakeet
    );
    assert_eq!(
        settings.transcription.model_id.as_deref(),
        Some("parakeet-tdt-0.6b-v3-onnx")
    );
    assert_eq!(settings.transcription.language, "en");
}

#[test]
fn validate_audio_transcription_settings_deepgram_model_rules() {
    let deepgram = |model_id: Option<&str>| {
        let mut settings = default_audio_transcription_settings();
        settings.provider = AudioTranscriptionProvider::Deepgram;
        settings.model_id = model_id.map(str::to_string);
        validate_audio_transcription_settings(settings)
    };

    // Explicit supported model is preserved.
    let ok = deepgram(Some("nova-2")).expect("nova-2 should validate");
    assert_eq!(ok.model_id.as_deref(), Some("nova-2"));

    // Missing model falls back to the nova-3 default.
    let defaulted = deepgram(None).expect("missing model should default");
    assert_eq!(defaulted.model_id.as_deref(), Some("nova-3"));

    // Unsupported model is rejected.
    let err = deepgram(Some("whisper-large")).expect_err("whisper-large should be rejected");
    assert_eq!(err.code, "invalid_recording_settings");
}

#[test]
fn default_recording_settings_include_preview_cache_ttl() {
    assert_eq!(
        default_recording_settings().preview_cache_ttl_seconds,
        default_preview_cache_ttl_seconds()
    );
}

#[test]
fn default_recording_settings_capture_screen_at_half_fps() {
    assert_eq!(default_recording_settings().screen_frame_rate, 0.5);
}

#[test]
fn load_recording_settings_from_path_defaults_preview_cache_ttl_when_missing() {
    let dir = TestDir::new("preview-cache-ttl-default");
    let path = dir.path().join("recording-settings.json");

    fs::write(
        &path,
        r#"{
            "captureScreen": true,
            "captureMicrophone": false,
            "captureSystemAudio": false,
            "segmentDurationSeconds": 60,
            "screenFrameRate": 1,
            "screenResolution": { "mode": "preset", "preset": "original" },
            "videoBitrate": { "mode": "preset", "preset": "medium" },
            "saveDirectory": "/tmp",
            "autoStart": false,
            "pauseCaptureOnInactivity": true,
            "idleTimeoutSeconds": 10,
            "microphoneActivitySensitivity": 50,
            "systemAudioActivitySensitivity": 50,
            "activityMode": "system_input_or_screen"
        }"#,
    )
    .expect("settings file should write");

    let loaded =
        load_recording_settings_from_path(&path).expect("settings should load from disk");

    assert_eq!(
        loaded.preview_cache_ttl_seconds,
        default_preview_cache_ttl_seconds()
    );
    assert_eq!(loaded.follow_timeline_live, default_follow_timeline_live());
    assert_eq!(loaded.appearance, default_appearance());
    assert_eq!(loaded.ocr, default_ocr_settings());
    assert_eq!(loaded.access, AccessSettings::default());
    assert_eq!(loaded.transcription, default_audio_transcription_settings());
}

#[test]
fn validate_recording_settings_rejects_preview_cache_ttl_above_max() {
    let error = validate_recording_settings(UpdateRecordingSettingsRequest {
        capture_screen: true,
        capture_microphone: false,
        capture_system_audio: false,
        segment_duration_seconds: 60,
        screen_frame_rate: 1.0,
        screen_resolution: ScreenResolution::Preset {
            preset: ScreenResolutionPreset::Original,
        },
        video_bitrate: default_video_bitrate(),
        save_directory: "/tmp".to_string(),
        auto_start: false,
        developer_options_enabled: false,
        preview_cache_ttl_seconds: MAX_PREVIEW_CACHE_TTL_SECONDS + 1,
        follow_timeline_live: false,
        retention_policy: RetentionPolicy::Never,
        appearance: default_appearance(),
        time_format: Default::default(),
        ocr: default_ocr_settings(),
        transcription: default_audio_transcription_settings(),
        speaker_analysis: default_speaker_analysis_settings(),
        audio_speech_detection: default_audio_speech_detection_settings(),
        metadata: default_metadata_settings(),
        privacy: default_privacy_settings(),
        access: AccessSettings::default(),
        ai_runtime: AiRuntimeSettings::default(),
        user_context: UserContextSettings::default(),
        semantic_search: capture_types::default_semantic_search_settings(),
        pause_capture_on_inactivity: true,
        idle_timeout_seconds: 10,
        microphone_activity_sensitivity: 50,
        system_audio_activity_sensitivity: 50,
        microphone_vad_adapter: capture_types::default_microphone_vad_adapter(),
        inactivity_activity_mode: default_inactivity_activity_mode(),
    })
    .expect_err("preview cache ttl above max must be rejected");

    assert_eq!(error.code, "invalid_recording_settings");
    assert_eq!(
        error.message,
        format!("previewCacheTtlSeconds must be between 0 and {MAX_PREVIEW_CACHE_TTL_SECONDS}")
    );
}

#[test]
fn validate_speaker_analysis_settings_keeps_default_model() {
    let settings = SpeakerAnalysisSettings {
        model_id: Some(speaker_analysis::SPEAKRS_DEFAULT_MODEL_ID.to_string()),
        ..default_speaker_analysis_settings()
    };

    let validated = validate_speaker_analysis_settings(settings);

    assert_eq!(validated.provider, speaker_analysis::SPEAKRS_PROVIDER_ID);
    assert_eq!(
        validated.model_id.as_deref(),
        Some(speaker_analysis::SPEAKRS_DEFAULT_MODEL_ID)
    );
}

#[test]
fn validate_speaker_analysis_settings_falls_back_for_unknown_model() {
    let settings = SpeakerAnalysisSettings {
        model_id: Some("bogus-model-xyz".to_string()),
        ..default_speaker_analysis_settings()
    };

    let validated = validate_speaker_analysis_settings(settings);

    // speakrs is the global default provider, so its unknown-model fallback
    // also coincides with the global default model.
    assert_eq!(validated.provider, speaker_analysis::SPEAKRS_PROVIDER_ID);
    assert_eq!(validated.model_id, default_speaker_analysis_model_id());
    assert_eq!(
        validated.model_id.as_deref(),
        Some(speaker_analysis::SPEAKRS_DEFAULT_MODEL_ID)
    );
}

#[test]
fn validate_speaker_analysis_settings_keeps_speakrs_default_model() {
    let settings = SpeakerAnalysisSettings {
        provider: speaker_analysis::SPEAKRS_PROVIDER_ID.to_string(),
        model_id: Some(speaker_analysis::SPEAKRS_DEFAULT_MODEL_ID.to_string()),
        ..default_speaker_analysis_settings()
    };

    let validated = validate_speaker_analysis_settings(settings);

    assert_eq!(validated.provider, speaker_analysis::SPEAKRS_PROVIDER_ID);
    assert_eq!(
        validated.model_id.as_deref(),
        Some(speaker_analysis::SPEAKRS_DEFAULT_MODEL_ID)
    );
}

#[test]
fn validate_speaker_analysis_settings_speakrs_unknown_model_resets_to_speakrs_default() {
    // Unknown model_id for a KNOWN provider resets to THAT provider's default
    // model — the provider choice (speakrs) is preserved, not dropped to
    // sherpa.
    let settings = SpeakerAnalysisSettings {
        provider: speaker_analysis::SPEAKRS_PROVIDER_ID.to_string(),
        model_id: Some("bogus-speakrs-model".to_string()),
        ..default_speaker_analysis_settings()
    };

    let validated = validate_speaker_analysis_settings(settings);

    assert_eq!(validated.provider, speaker_analysis::SPEAKRS_PROVIDER_ID);
    assert_eq!(
        validated.model_id.as_deref(),
        Some(speaker_analysis::SPEAKRS_DEFAULT_MODEL_ID)
    );
}

/// MIGRATION: a settings file persisted before sherpa removal carries
/// `provider = "sherpa_onnx"` (plus a sherpa model id). Because sherpa is no
/// longer a known provider, validation must remap it to speakrs + the speakrs
/// default model — it must be impossible to leave a user pinned on sherpa.
#[test]
fn validate_speaker_analysis_settings_remaps_legacy_sherpa_provider_to_speakrs() {
    let settings = SpeakerAnalysisSettings {
        provider: "sherpa_onnx".to_string(),
        model_id: Some("pyannote-3.0-nemo-titanet-small".to_string()),
        ..default_speaker_analysis_settings()
    };

    let validated = validate_speaker_analysis_settings(settings);

    assert_eq!(validated.provider, speaker_analysis::SPEAKRS_PROVIDER_ID);
    assert_eq!(
        validated.model_id.as_deref(),
        Some(speaker_analysis::SPEAKRS_DEFAULT_MODEL_ID)
    );
}

#[test]
fn validate_speaker_analysis_settings_unknown_provider_resets_to_speakrs_default() {
    let settings = SpeakerAnalysisSettings {
        provider: "totally-unknown-provider".to_string(),
        model_id: Some(speaker_analysis::SPEAKRS_DEFAULT_MODEL_ID.to_string()),
        ..default_speaker_analysis_settings()
    };

    let validated = validate_speaker_analysis_settings(settings);

    let defaults = default_speaker_analysis_settings();
    assert_eq!(validated.provider, defaults.provider);
    assert_eq!(validated.provider, speaker_analysis::SPEAKRS_PROVIDER_ID);
    assert_eq!(validated.model_id, defaults.model_id);
}

#[test]
fn validate_speaker_analysis_settings_known_provider_missing_model_uses_provider_default() {
    // `model_id: None` for a known non-default provider resolves to that
    // provider's default model, not the global default.
    let settings = SpeakerAnalysisSettings {
        provider: speaker_analysis::SPEAKRS_PROVIDER_ID.to_string(),
        model_id: None,
        ..default_speaker_analysis_settings()
    };

    let validated = validate_speaker_analysis_settings(settings);

    assert_eq!(validated.provider, speaker_analysis::SPEAKRS_PROVIDER_ID);
    assert_eq!(
        validated.model_id.as_deref(),
        Some(speaker_analysis::SPEAKRS_DEFAULT_MODEL_ID)
    );
}

#[test]
fn validate_semantic_search_settings_trims_and_keeps_known_model() {
    // L4: an untrimmed but otherwise-known guided-tier provider + model is
    // trimmed and kept (mirrors validate_speaker_analysis_settings keeping a
    // known model). The default model id is a known guided tier.
    let known_model = default_semantic_search_model_id().expect("a default model id");
    let settings = SemanticSearchSettings {
        enabled: true,
        provider: format!("  {}  ", semantic_search::SEMANTIC_SEARCH_PROVIDER_ID),
        model_id: Some(format!("  {known_model}  ")),
        ..Default::default()
    };

    let validated = validate_semantic_search_settings(settings);

    assert_eq!(
        validated.provider,
        semantic_search::SEMANTIC_SEARCH_PROVIDER_ID
    );
    assert_eq!(validated.model_id.as_deref(), Some(known_model.as_str()));
    assert!(validated.enabled, "the enabled flag is carried through");
}

#[test]
fn validate_semantic_search_settings_resets_unknown_provider_to_default() {
    // L4: an unrecognized provider resets to the default ("local"), exactly
    // as the speaker validator resets an unknown provider to "sherpa_onnx".
    let settings = SemanticSearchSettings {
        enabled: false,
        provider: "made-up-provider".to_string(),
        model_id: default_semantic_search_model_id(),
        ..Default::default()
    };

    let validated = validate_semantic_search_settings(settings);

    assert_eq!(validated.provider, default_semantic_search_provider());
    // The model still resolves under the reset default provider, so it survives.
    assert_eq!(validated.model_id, default_semantic_search_model_id());
    assert!(!validated.enabled, "the enabled flag is carried through");
}

#[test]
fn validate_semantic_search_settings_falls_back_for_empty_model() {
    // L4: a PRESENT but empty/whitespace model id (no real selection) falls back
    // to the default model.
    for raw_model in ["   ", ""] {
        let settings = SemanticSearchSettings {
            enabled: true,
            provider: semantic_search::SEMANTIC_SEARCH_PROVIDER_ID.to_string(),
            model_id: Some(raw_model.to_string()),
            ..Default::default()
        };

        let validated = validate_semantic_search_settings(settings);

        assert_eq!(
            validated.model_id,
            default_semantic_search_model_id(),
            "a present empty/whitespace model {raw_model:?} must fall back to the default"
        );
    }
}

#[test]
fn validate_semantic_search_settings_preserves_a_present_unresolvable_model() {
    // An unrelated recording-settings save must NOT silently swap a present-but-
    // unresolvable model id to the (possibly dimension-incompatible) default —
    // that would desync the persisted selection from the vec0 table until the
    // next restart's reconcile. The id is preserved verbatim (trimmed); the
    // live-dimension authority no-ops it into keyword-only and reconciliation
    // re-aligns it. Changing the selected model happens only via the explicit
    // atomic switch path.
    let settings = SemanticSearchSettings {
        enabled: true,
        provider: semantic_search::SEMANTIC_SEARCH_PROVIDER_ID.to_string(),
        model_id: Some("  bogus-model-xyz  ".to_string()),
        ..Default::default()
    };

    let validated = validate_semantic_search_settings(settings);

    assert_eq!(
        validated.model_id.as_deref(),
        Some("bogus-model-xyz"),
        "a present unresolvable model must be preserved (trimmed), not swapped to the default"
    );
}

#[test]
fn validate_semantic_search_settings_keeps_an_explicitly_cleared_model() {
    // L4 boundary: an explicit `None` is the legitimate "no model selected"
    // (keyword-only) sentinel and must NOT be resurrected to the default — this
    // is the one deliberate divergence from the speaker-analysis validator, and
    // it preserves the model-gated clear semantics the domain patch relies on.
    let settings = SemanticSearchSettings {
        enabled: false,
        provider: semantic_search::SEMANTIC_SEARCH_PROVIDER_ID.to_string(),
        model_id: None,
        ..Default::default()
    };

    let validated = validate_semantic_search_settings(settings);

    assert_eq!(validated.model_id, None);
}
