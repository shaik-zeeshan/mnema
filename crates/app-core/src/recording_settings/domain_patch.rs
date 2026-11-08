use capture_types::{
    CaptureErrorResponse, RecordingSettings, SettingsOwnershipDomain, UpdateAccessSettingsRequest,
    UpdateAiRuntimeSettingsRequest, UpdateCaptureSourceSettingsRequest,
    UpdateCaptureTimingSettingsRequest, UpdateDeveloperSettingsRequest,
    UpdateDisplaySettingsRequest, UpdateInactivitySettingsRequest, UpdateMetadataSettingsRequest,
    UpdateProcessingSettingsRequest, UpdateRecordingSettingsRequest,
    UpdateSemanticSearchSettingsRequest, UpdateStorageSettingsRequest,
    UpdateUserContextSettingsRequest, UpdateVideoSettingsRequest,
};

pub enum RecordingSettingsDomainPatch {
    CaptureSources(UpdateCaptureSourceSettingsRequest),
    CaptureTiming(UpdateCaptureTimingSettingsRequest),
    Video(UpdateVideoSettingsRequest),
    Storage(UpdateStorageSettingsRequest),
    Display(UpdateDisplaySettingsRequest),
    Metadata(UpdateMetadataSettingsRequest),
    Inactivity(UpdateInactivitySettingsRequest),
    Processing(UpdateProcessingSettingsRequest),
    Developer(UpdateDeveloperSettingsRequest),
    Access(UpdateAccessSettingsRequest),
    AiRuntime(UpdateAiRuntimeSettingsRequest),
    UserContext(UpdateUserContextSettingsRequest),
    /// Generic **Semantic Search** settings patch from the untrusted IPC boundary
    /// (`update_semantic_search_settings`). Honors the `enabled` toggle and other
    /// non-dimension-affecting fields, but DELIBERATELY ignores `model_id` /
    /// `provider`: changing the model through a generic patch would re-open the
    /// non-atomic dimension split (the persisted `model_id` moves but the `vec0`
    /// table is not rebuilt to the new model's dimension). Model/provider changes
    /// must go through the dedicated atomic switch (`select_semantic_search_model`
    /// → [`Self::SemanticSearchModelSwitch`]), which rebuilds the table and
    /// persists the selection together. See review finding low #4 (PR #126).
    SemanticSearch(UpdateSemanticSearchSettingsRequest),
    /// Trusted **Semantic Search Model Tier** persist from the atomic switch
    /// (`select_semantic_search_model` via `persist_semantic_search_settings`),
    /// which has ALREADY rebuilt the `vec0` table to the new model's dimension
    /// before persisting. This variant honors `model_id` / `provider` so the
    /// persisted selection lands in lockstep with the live table dimension. It is
    /// internal-only — there is no `#[tauri::command]` that constructs it.
    SemanticSearchModelSwitch(UpdateSemanticSearchSettingsRequest),
}

impl RecordingSettingsDomainPatch {
    pub fn domain(&self) -> SettingsOwnershipDomain {
        match self {
            Self::CaptureSources(_) => SettingsOwnershipDomain::CaptureSources,
            Self::CaptureTiming(_) => SettingsOwnershipDomain::CaptureTiming,
            Self::Video(_) => SettingsOwnershipDomain::Video,
            Self::Storage(_) => SettingsOwnershipDomain::Storage,
            Self::Display(_) => SettingsOwnershipDomain::Display,
            Self::Metadata(_) => SettingsOwnershipDomain::Metadata,
            Self::Inactivity(_) => SettingsOwnershipDomain::Inactivity,
            Self::Processing(_) => SettingsOwnershipDomain::Processing,
            Self::Developer(_) => SettingsOwnershipDomain::Developer,
            Self::Access(_) => SettingsOwnershipDomain::Access,
            Self::AiRuntime(_) => SettingsOwnershipDomain::AiRuntime,
            Self::UserContext(_) => SettingsOwnershipDomain::UserContext,
            Self::SemanticSearch(_) => SettingsOwnershipDomain::SemanticSearch,
            Self::SemanticSearchModelSwitch(_) => SettingsOwnershipDomain::SemanticSearch,
        }
    }
}

pub(super) fn empty_domain_patch_error(domain: SettingsOwnershipDomain) -> CaptureErrorResponse {
    CaptureErrorResponse {
        code: "empty_settings_patch".to_string(),
        message: format!("{domain:?} settings patch must include at least one field"),
    }
}

pub fn recording_settings_request_from_settings(
    settings: RecordingSettings,
) -> UpdateRecordingSettingsRequest {
    UpdateRecordingSettingsRequest {
        capture_screen: settings.capture_screen,
        capture_microphone: settings.capture_microphone,
        capture_system_audio: settings.capture_system_audio,
        segment_duration_seconds: settings.segment_duration_seconds,
        screen_frame_rate: settings.screen_frame_rate,
        screen_resolution: settings.screen_resolution,
        video_bitrate: settings.video_bitrate,
        save_directory: settings.save_directory,
        auto_start: settings.auto_start,
        developer_options_enabled: settings.developer_options_enabled,
        preview_cache_ttl_seconds: settings.preview_cache_ttl_seconds,
        follow_timeline_live: settings.follow_timeline_live,
        retention_policy: settings.retention_policy,
        appearance: settings.appearance,
        time_format: settings.time_format,
        ocr: settings.ocr,
        transcription: settings.transcription,
        speaker_analysis: settings.speaker_analysis,
        audio_speech_detection: settings.audio_speech_detection,
        metadata: settings.metadata,
        privacy: settings.privacy,
        access: settings.access,
        ai_runtime: settings.ai_runtime,
        user_context: settings.user_context,
        semantic_search: settings.semantic_search,
        pause_capture_on_inactivity: settings.pause_capture_on_inactivity,
        idle_timeout_seconds: settings.idle_timeout_seconds,
        microphone_activity_sensitivity: settings.microphone_activity_sensitivity,
        system_audio_activity_sensitivity: settings.system_audio_activity_sensitivity,
        microphone_vad_adapter: settings.microphone_vad_adapter,
        inactivity_activity_mode: settings.inactivity_activity_mode,
    }
}

pub fn apply_domain_patch_to_settings(
    settings: &mut RecordingSettings,
    patch: RecordingSettingsDomainPatch,
) -> Result<(), CaptureErrorResponse> {
    let domain = patch.domain();
    let mut touched = false;

    match patch {
        RecordingSettingsDomainPatch::CaptureSources(request) => {
            if let Some(value) = request.capture_screen {
                settings.capture_screen = value;
                touched = true;
            }
            if let Some(value) = request.capture_microphone {
                settings.capture_microphone = value;
                touched = true;
            }
            if let Some(value) = request.capture_system_audio {
                settings.capture_system_audio = value;
                touched = true;
            }
        }
        RecordingSettingsDomainPatch::CaptureTiming(request) => {
            if let Some(value) = request.segment_duration_seconds {
                settings.segment_duration_seconds = value;
                touched = true;
            }
            if let Some(value) = request.auto_start {
                settings.auto_start = value;
                touched = true;
            }
        }
        RecordingSettingsDomainPatch::Video(request) => {
            if let Some(value) = request.screen_frame_rate {
                settings.screen_frame_rate = value;
                touched = true;
            }
            if let Some(value) = request.screen_resolution {
                settings.screen_resolution = value;
                touched = true;
            }
            if let Some(value) = request.video_bitrate {
                settings.video_bitrate = value;
                touched = true;
            }
        }
        RecordingSettingsDomainPatch::Storage(request) => {
            if let Some(value) = request.save_directory {
                settings.save_directory = value;
                touched = true;
            }
            if let Some(value) = request.retention_policy {
                settings.retention_policy = value;
                touched = true;
            }
        }
        RecordingSettingsDomainPatch::Display(request) => {
            if let Some(value) = request.appearance {
                settings.appearance = value;
                touched = true;
            }
            if let Some(value) = request.time_format {
                settings.time_format = value;
                touched = true;
            }
            if let Some(value) = request.follow_timeline_live {
                settings.follow_timeline_live = value;
                touched = true;
            }
        }
        RecordingSettingsDomainPatch::Metadata(request) => {
            if let Some(value) = request.enabled {
                settings.metadata.enabled = value;
                touched = true;
            }
            if let Some(value) = request.browser_url_mode {
                settings.metadata.browser_url_mode = value;
                touched = true;
            }
        }
        RecordingSettingsDomainPatch::Inactivity(request) => {
            if let Some(value) = request.pause_capture_on_inactivity {
                settings.pause_capture_on_inactivity = value;
                touched = true;
            }
            if let Some(value) = request.idle_timeout_seconds {
                settings.idle_timeout_seconds = value;
                touched = true;
            }
            if let Some(value) = request.microphone_activity_sensitivity {
                settings.microphone_activity_sensitivity = value;
                touched = true;
            }
            if let Some(value) = request.system_audio_activity_sensitivity {
                settings.system_audio_activity_sensitivity = value;
                touched = true;
            }
            if let Some(value) = request.audio_speech_detection {
                settings.audio_speech_detection = value;
                touched = true;
            }
        }
        RecordingSettingsDomainPatch::Processing(request) => {
            if let Some(value) = request.ocr {
                settings.ocr = value;
                touched = true;
            }
            if let Some(value) = request.transcription {
                settings.transcription = value;
                touched = true;
            }
            if let Some(value) = request.speaker_analysis {
                settings.speaker_analysis = value;
                touched = true;
            }
            if let Some(value) = request.preview_cache_ttl_seconds {
                settings.preview_cache_ttl_seconds = value;
                touched = true;
            }
        }
        RecordingSettingsDomainPatch::Access(request) => {
            if let Some(value) = request.ask_ai_enabled {
                settings.access.ask_ai_enabled = value;
                touched = true;
            }
            if let Some(value) = request.ask_ai_web_fetch_enabled {
                settings.access.ask_ai_web_fetch_enabled = value;
                touched = true;
            }
            if let Some(value) = request.quick_recall_auto_answer {
                settings.access.quick_recall_auto_answer = value;
                touched = true;
            }
            if let Some(value) = request.ask_ai_max_tool_calls {
                settings.access.ask_ai_max_tool_calls = value;
                touched = true;
            }
            if let Some(value) = request.ask_ai_model {
                // The model and its provider are one value: rewriting the model
                // always rewrites (or clears) the provider, so a stale provider
                // never outlives the model it belonged to.
                let non_empty = |v: &str| {
                    let trimmed = v.trim();
                    (!trimmed.is_empty()).then(|| trimmed.to_string())
                };
                settings.access.ask_ai_model = non_empty(&value);
                settings.access.ask_ai_model_provider = settings
                    .access
                    .ask_ai_model
                    .as_ref()
                    .and(request.ask_ai_model_provider.as_deref())
                    .and_then(non_empty);
                touched = true;
            }
        }
        RecordingSettingsDomainPatch::AiRuntime(request) => {
            if let Some(value) = request.enabled {
                settings.ai_runtime.enabled = value;
                touched = true;
            }
            if let Some(value) = request.providers {
                // Replacement provider list (wholesale, like additionalEngines
                // before it); normalization (trim/dedupe) happens in validation.
                settings.ai_runtime.providers = value;
                touched = true;
            }
            if let Some(value) = request.mcp_servers {
                // Replacement MCP connector list (wholesale, like providers).
                settings.ai_runtime.mcp_servers = value;
                touched = true;
            }
            if let Some(value) = request.default_model {
                // Double-Option: an explicit `null` clears the default model.
                settings.ai_runtime.default_model = value;
                touched = true;
            }
            if let Some(value) = request.favorite_models {
                settings.ai_runtime.favorite_models = value;
                touched = true;
            }
            if let Some(value) = request.recent_models {
                settings.ai_runtime.recent_models = value;
                touched = true;
            }
        }
        RecordingSettingsDomainPatch::UserContext(request) => {
            if let Some(value) = request.enabled {
                settings.user_context.enabled = value;
                touched = true;
            }
            if let Some(value) = request.derivation_budget_tier {
                settings.user_context.derivation_budget_tier = value;
                touched = true;
            }
            if let Some(value) = request.backfill_window_days {
                settings.user_context.backfill_window_days = value;
                touched = true;
            }
            if let Some(value) = request.backfill_go_deeper {
                settings.user_context.backfill_go_deeper = value;
                touched = true;
            }
        }
        RecordingSettingsDomainPatch::SemanticSearch(request) => {
            if let Some(value) = request.enabled {
                settings.semantic_search.enabled = value;
                touched = true;
            }
            // Ignore `model_id` / `provider` from this generic IPC patch. Changing
            // the model here would re-open the non-atomic dimension split: the
            // persisted `model_id` would move but the `vec0` table is NOT rebuilt
            // to the new model's dimension, so the index width would disagree with
            // the selection (search degrades to keyword-only until startup
            // reconciliation, and vectors could be discarded). Model/provider
            // changes must go through the dedicated atomic switch
            // (`select_semantic_search_model`), which rebuilds the table and
            // persists the selection together. We ignore-with-log rather than
            // error so existing callers that harmlessly echo the current value
            // don't break.
            if request.model_id.is_some() || request.provider.is_some() {
                crate::debug_log::log_info(
                    "semantic search: ignoring `model_id`/`provider` in a generic settings update; \
                     model/provider changes must go through the atomic switch \
                     (`select_semantic_search_model`) that rebuilds the vec0 table at the new dimension",
                );
            }
        }
        RecordingSettingsDomainPatch::SemanticSearchModelSwitch(request) => {
            // Trusted path: `select_semantic_search_model` has ALREADY rebuilt the
            // `vec0` table to the new model's dimension before persisting, so the
            // selection lands in lockstep with the live table width. Honor every
            // field, including `model_id` / `provider`.
            if let Some(value) = request.enabled {
                settings.semantic_search.enabled = value;
                touched = true;
            }
            if let Some(value) = request.provider {
                settings.semantic_search.provider = value;
                touched = true;
            }
            if let Some(value) = request.model_id {
                // Double-Option: an explicit `null` clears the selected model.
                settings.semantic_search.model_id = value;
                touched = true;
            }
        }
        RecordingSettingsDomainPatch::Developer(request) => {
            if let Some(value) = request.developer_options_enabled {
                settings.developer_options_enabled = value;
                touched = true;
            }
        }
    }

    if !touched {
        return Err(empty_domain_patch_error(domain));
    }

    Ok(())
}
