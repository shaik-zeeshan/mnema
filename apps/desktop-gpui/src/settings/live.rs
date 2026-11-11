//! Settings' live commands: each one calls app-core directly, the way the
//! Svelte page invokes its Tauri command. All are synchronous (`block_on` on
//! AppCore's runtime), so callers run them on the background executor.
//! `Ok(Some(text))` is a line worth showing; `Err` is the backend's own words.

use std::sync::Arc;

use app_core::native_capture::settings_update as su;
use app_core::recording_settings::{RecordingSettingsDomainPatch as Patch, RecordingSettingsState};
use app_core::{AppCore, AppInfraState};
use capture_types::{
    MicrophoneControllerState, RecordingSettings, UpdateAccessSettingsRequest,
    UpdateAiRuntimeSettingsRequest, UpdateCaptureSourceSettingsRequest,
    UpdateCaptureTimingSettingsRequest, UpdateDeveloperSettingsRequest,
    UpdateDisplaySettingsRequest, UpdateInactivitySettingsRequest, UpdateMetadataSettingsRequest,
    UpdateMicrophoneControllerRequest, UpdateProcessingSettingsRequest,
    UpdateStorageSettingsRequest, UpdateUserContextSettingsRequest, UpdateVideoSettingsRequest,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use super::autosave::Unit;

pub type Outcome = Result<Option<String>, String>;

fn infra(core: &AppCore) -> Result<Arc<AppInfraState>, String> {
    core.try_state::<AppInfraState>()
        .ok_or_else(|| "app infrastructure isn't ready".to_string())
}

fn req<T: DeserializeOwned>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| format!("couldn't build the request: {e}"))
}

fn to_json(v: impl serde::Serialize) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// One autosave unit's fields as its `update_*_settings` request (every field
/// sent, as the Svelte page does). Also used to copy a saved answer's domain
/// back into the draft. `None` for the two units that aren't recording domains.
pub fn patch(unit: Unit, s: &RecordingSettings) -> Option<Patch> {
    Some(match unit {
        Unit::CaptureSources => Patch::CaptureSources(UpdateCaptureSourceSettingsRequest {
            capture_screen: Some(s.capture_screen),
            capture_microphone: Some(s.capture_microphone),
            capture_system_audio: Some(s.capture_system_audio),
        }),
        Unit::CaptureTiming => Patch::CaptureTiming(UpdateCaptureTimingSettingsRequest {
            segment_duration_seconds: Some(s.segment_duration_seconds),
            auto_start: Some(s.auto_start),
        }),
        Unit::Video => Patch::Video(UpdateVideoSettingsRequest {
            screen_frame_rate: Some(s.screen_frame_rate),
            screen_resolution: Some(s.screen_resolution.clone()),
            video_bitrate: Some(s.video_bitrate.clone()),
        }),
        Unit::Storage => Patch::Storage(UpdateStorageSettingsRequest {
            save_directory: Some(s.save_directory.clone()),
            retention_policy: Some(s.retention_policy.clone()),
        }),
        Unit::Display => Patch::Display(UpdateDisplaySettingsRequest {
            appearance: Some(s.appearance.clone()),
            time_format: Some(s.time_format.clone()),
            follow_timeline_live: Some(s.follow_timeline_live),
        }),
        Unit::Metadata => Patch::Metadata(UpdateMetadataSettingsRequest {
            enabled: Some(s.metadata.enabled),
            browser_url_mode: Some(s.metadata.browser_url_mode.clone()),
        }),
        Unit::Inactivity => Patch::Inactivity(UpdateInactivitySettingsRequest {
            pause_capture_on_inactivity: Some(s.pause_capture_on_inactivity),
            idle_timeout_seconds: Some(s.idle_timeout_seconds),
            microphone_activity_sensitivity: Some(s.microphone_activity_sensitivity),
            system_audio_activity_sensitivity: Some(s.system_audio_activity_sensitivity),
            audio_speech_detection: Some(s.audio_speech_detection.clone()),
        }),
        Unit::Processing => Patch::Processing(UpdateProcessingSettingsRequest {
            ocr: Some(s.ocr.clone()),
            transcription: Some(s.transcription.clone()),
            speaker_analysis: Some(s.speaker_analysis.clone()),
            preview_cache_ttl_seconds: Some(s.preview_cache_ttl_seconds),
        }),
        Unit::Developer => Patch::Developer(UpdateDeveloperSettingsRequest {
            developer_options_enabled: Some(s.developer_options_enabled),
        }),
        Unit::Access => Patch::Access(UpdateAccessSettingsRequest {
            ask_ai_enabled: Some(s.access.ask_ai_enabled),
            ask_ai_max_tool_calls: Some(s.access.ask_ai_max_tool_calls),
            ask_ai_web_fetch_enabled: Some(s.access.ask_ai_web_fetch_enabled),
            // An empty model clears the pair back to the default engine's model.
            ask_ai_model: Some(s.access.ask_ai_model.clone().unwrap_or_default()),
            ask_ai_model_provider: s.access.ask_ai_model_provider.clone(),
            quick_recall_auto_answer: Some(s.access.quick_recall_auto_answer.clone()),
        }),
        Unit::AiRuntime => Patch::AiRuntime(UpdateAiRuntimeSettingsRequest {
            enabled: Some(s.ai_runtime.enabled),
            providers: Some(s.ai_runtime.providers.clone()),
            default_model: Some(s.ai_runtime.default_model.clone()),
            mcp_servers: Some(s.ai_runtime.mcp_servers.clone()),
            favorite_models: Some(s.ai_runtime.favorite_models.clone()),
            recent_models: Some(s.ai_runtime.recent_models.clone()),
        }),
        Unit::UserContext => Patch::UserContext(UpdateUserContextSettingsRequest {
            enabled: Some(s.user_context.enabled),
            derivation_budget_tier: Some(s.user_context.derivation_budget_tier.clone()),
            backfill_window_days: Some(s.user_context.backfill_window_days),
            backfill_go_deeper: Some(s.user_context.backfill_go_deeper),
        }),
        Unit::Keyboard | Unit::Microphone => return None,
    })
}

/// `update_<domain>_settings`: validate, persist, apply to the running app
/// (capture runtime, workers, tray) and emit the change events.
pub fn save_domain(
    core: &Arc<AppCore>,
    unit: Unit,
    draft: &RecordingSettings,
) -> Result<RecordingSettings, String> {
    let patch = patch(unit, draft).ok_or("not a recording settings domain")?;
    let saved = match patch {
        // Off the caller's thread too: a sources change asks the capture lock.
        Patch::CaptureSources(r) => core.block_on(su::update_capture_source_settings(r, core)),
        Patch::CaptureTiming(r) => su::update_capture_timing_settings(r, core),
        Patch::Video(r) => su::update_video_settings(r, core),
        Patch::Storage(r) => su::update_storage_settings(r, core),
        Patch::Display(r) => su::update_display_settings(r, core),
        Patch::Metadata(r) => su::update_metadata_settings(r, core),
        Patch::Inactivity(r) => su::update_inactivity_settings(r, core),
        Patch::Processing(r) => su::update_processing_settings(r, core),
        Patch::Developer(r) => su::update_developer_settings(r, core),
        Patch::Access(r) => su::update_access_settings(r, core),
        Patch::AiRuntime(r) => su::update_ai_runtime_settings(r, core),
        Patch::UserContext(r) => su::update_user_context_settings(r, core),
        Patch::SemanticSearch(r) | Patch::SemanticSearchModelSwitch(r) => {
            su::update_semantic_search_settings(r, core)
        }
    };
    saved.map(|r| r.settings).map_err(|e| e.message)
}

/// The privacy commands' apply (`add_ / set_ / remove_privacy_excluded_app`,
/// `set_privacy_filter_system_audio`): `next.privacy` goes through the
/// app-exclusion domain, which validates it and refreshes the live filter.
pub fn save_privacy(
    core: &Arc<AppCore>,
    next: &RecordingSettings,
) -> Result<RecordingSettings, String> {
    let wanted = next.privacy.clone();
    su::apply_recording_settings_domain_mutation_from_core(
        core,
        capture_types::SettingsOwnershipDomain::AppPrivacyExclusion,
        move |s| {
            s.privacy = wanted;
            Ok(())
        },
    )
    .map(|r| r.settings)
    .map_err(|e| e.message)
}

pub fn save_mic(
    core: &Arc<AppCore>,
    state: &MicrophoneControllerState,
) -> Result<MicrophoneControllerState, String> {
    let request = UpdateMicrophoneControllerRequest {
        preference: state.preference.clone(),
        disconnect_policy: state.disconnect_policy.clone(),
    };
    core.block_on(
        app_core::native_capture::microphone::controller::update_microphone_controller_async(
            request, core,
        ),
    )
    .map_err(|e| e.message)
}

/// `update_semantic_search_settings { enabled }` (its own domain, not autosaved).
pub fn set_semantic_enabled(core: &Arc<AppCore>, on: bool) -> Result<RecordingSettings, String> {
    su::update_semantic_search_settings(req(json!({ "enabled": on }))?, core)
        .map(|r| r.settings)
        .map_err(|e| e.message)
}

// ── one-shot commands ───────────────────────────────────────────────────────

pub fn install_cli(core: &Arc<AppCore>) -> Outcome {
    let status = to_json(core.block_on(app_core::cli_access::install_cli_inner())?);
    Ok(Some(format!(
        "Installed the mnema command at {}.",
        status["installPath"].as_str().unwrap_or("~/.local/bin")
    )))
}

pub fn derive_now(core: &Arc<AppCore>) -> Outcome {
    let infra = infra(core)?;
    let run = core.block_on(
        app_core::user_context::commands::user_context_run_derivation_now(core, &infra),
    )?;
    Ok(Some(run.message))
}

/// Stops the engine first (AI features go off), then clears what was derived.
pub fn wipe_user_context(core: &Arc<AppCore>) -> Outcome {
    let infra = infra(core)?;
    core.block_on(app_core::user_context::commands::wipe_user_context(
        core, &infra,
    ))?;
    // Svelte says nothing on success: the emptied rows and the switched-off
    // master switch are the answer.
    Ok(None)
}

/// `preview_retention_cleanup` for a policy not saved yet: what it would delete,
/// as `(segments, frames, audio segments, cutoff)`.
pub fn retention_preview(
    core: &Arc<AppCore>,
    policy: capture_types::RetentionPolicy,
) -> Result<(i64, i64, i64, Option<String>), String> {
    let infra = infra(core)?;
    let request = app_core::app_infra::PreviewRetentionCleanupRequest {
        policy: Some(policy),
    };
    let s = core.block_on(app_core::app_infra::preview_retention_cleanup(
        Some(request),
        core,
        &infra,
        &core.state::<RecordingSettingsState>(),
    ))?;
    Ok((
        s.eligible_capture_segments,
        s.deleted_frames,
        s.deleted_audio_segments,
        s.cutoff_ended_before,
    ))
}

/// `run_retention_cleanup_now`: `(segments, frames, audio segments)` deleted.
pub fn retention_run(core: &Arc<AppCore>) -> Result<(i64, i64, i64), String> {
    let infra = infra(core)?;
    let s = core.block_on(app_core::app_infra::run_retention_cleanup_now(
        core,
        &infra,
        &core.state::<RecordingSettingsState>(),
    ))?;
    Ok((
        s.deleted_capture_segments,
        s.deleted_frames,
        s.deleted_audio_segments,
    ))
}

/// Finalizes the live segment, deletes every overlapping one, resumes capture.
/// Answers how many files couldn't be removed from disk yet.
pub fn delete_recent(core: &Arc<AppCore>, window_seconds: u64) -> Result<i64, String> {
    let s = core.block_on(app_core::app_infra::delete_recent_capture_from_core(
        core,
        window_seconds as i64,
    ))?;
    Ok(s.file_delete_errors)
}

pub fn semantic_download(core: &Arc<AppCore>, model_id: String) -> Outcome {
    let request = app_core::semantic_search_models::SemanticSearchModelDownloadRequestDto {
        provider: "local".into(),
        model_id,
    };
    app_core::semantic_search_models::start_semantic_search_model_download(core, request)?;
    Ok(None)
}

pub fn semantic_delete(core: &Arc<AppCore>, model_id: String) -> Outcome {
    let request = app_core::semantic_search_models::SemanticSearchModelDownloadRequestDto {
        provider: "local".into(),
        model_id,
    };
    app_core::semantic_search_models::delete_semantic_search_model(
        core,
        request,
        &core.state::<app_core::semantic_search_models::SemanticSearchModelDownloadState>(),
    )?;
    Ok(Some("Search model files deleted.".into()))
}

pub fn speaker_model_delete(core: &Arc<AppCore>, provider: String, model_id: String) -> Outcome {
    let infra = infra(core)?;
    let request = app_core::speaker_analysis_models::SpeakerAnalysisModelDownloadRequestDto {
        provider,
        model_id,
    };
    core.block_on(
        app_core::speaker_analysis_models::delete_speaker_analysis_model(core, request, &infra),
    )?;
    Ok(Some("Speaker model files deleted.".into()))
}

/// `ai_runtime_test_connection` for the Developer window's Test AI.
pub fn test_connection(core: &Arc<AppCore>) -> Outcome {
    let r = to_json(core.block_on(app_core::ai_runtime::ai_runtime_test_connection(core))?);
    let line = format!(
        "{} \u{b7} {}: {}",
        r["provider"].as_str().unwrap_or_default(),
        r["model"].as_str().unwrap_or_default(),
        r["message"].as_str().unwrap_or_default()
    );
    if r["ok"].as_bool() == Some(true) {
        Ok(Some(line))
    } else {
        Err(line)
    }
}

pub fn license_refresh(core: &Arc<AppCore>) -> Outcome {
    core.block_on(app_core::licensing::refresh_license_now(core.clone()))?;
    Ok(Some("License checked just now.".into()))
}

pub fn license_activate(core: &Arc<AppCore>, key: String) -> Outcome {
    let infra = infra(core)?;
    core.block_on(app_core::licensing::activate_license(key, core, &infra))?;
    Ok(Some("License activated on this device.".into()))
}

pub fn license_reset_devices(
    core: &Arc<AppCore>,
) -> Result<capture_types::ResetDevicesOutcome, String> {
    core.block_on(app_core::licensing::reset::reset_license_devices(
        core.clone(),
    ))
}

fn deepgram(
    core: &AppCore,
) -> Result<Arc<app_core::transcription_deepgram::DeepgramAuthStatusState>, String> {
    core.try_state()
        .ok_or_else(|| "transcription isn't ready yet".to_string())
}

/// `transcription_test_deepgram`: validates the saved key (no audio, no billing).
pub fn deepgram_test(core: &Arc<AppCore>) -> Outcome {
    let auth = deepgram(core)?;
    let r = to_json(
        core.block_on(app_core::transcription_deepgram::transcription_test_deepgram(&auth))?,
    );
    let message = r["message"].as_str().unwrap_or_default().to_string();
    if r["ok"].as_bool() == Some(true) {
        Ok(Some(message))
    } else {
        Err(message)
    }
}

/// Save into the vault, then check it at once (as the Svelte field does).
pub fn deepgram_save(core: &Arc<AppCore>, key: String) -> Outcome {
    let auth = deepgram(core)?;
    core.block_on(app_core::transcription_deepgram::transcription_set_deepgram_key(key, &auth))?;
    deepgram_test(core)
}

pub fn deepgram_clear(core: &Arc<AppCore>) -> Outcome {
    let auth = deepgram(core)?;
    core.block_on(app_core::transcription_deepgram::transcription_clear_deepgram_key(&auth))?;
    Ok(Some("Deepgram key removed.".into()))
}

/// Raise the macOS prompt for `screen` / `microphone` / `systemAudio`.
pub fn request_permission(core: &Arc<AppCore>, kind: &str) -> Outcome {
    core.block_on(
        app_core::native_capture::permissions::request_capture_permission(kind.into(), core),
    )?;
    Ok(None)
}

pub fn open_privacy_pane(core: &Arc<AppCore>, kind: &str) -> Outcome {
    app_core::native_capture::permissions::open_capture_privacy_settings(kind.into(), core)?;
    Ok(None)
}

/// Raise the Speech Recognition prompt (Apple Speech); a past denial can only
/// be changed in System Settings, so the pane opens when it isn't usable after.
pub fn request_speech_permission(core: &Arc<AppCore>) -> Outcome {
    let status = to_json(
        app_core::audio_transcription_models::request_apple_speech_recognition_permission(core)?,
    );
    let usable = status["providers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["provider"].as_str() == Some("apple_speech_on_device"))
        .flat_map(|p| p["models"].as_array().cloned().unwrap_or_default())
        .any(|m| m["available"].as_bool() == Some(true));
    if usable {
        return Ok(Some("Apple Speech is allowed and ready.".into()));
    }
    open_speech_pane(core)?;
    Ok(Some(
        "Allow Mnema under Privacy & Security \u{2192} Speech Recognition.".into(),
    ))
}

pub fn open_speech_pane(core: &Arc<AppCore>) -> Outcome {
    app_core::audio_transcription_models::open_apple_speech_recognition_privacy_settings(core)?;
    Ok(None)
}

pub fn mcp_oauth_disconnect(core: &Arc<AppCore>, id: String) -> Outcome {
    core.block_on(app_core::ask_ai::mcp::oauth_flow::mcp_oauth_disconnect(
        core, id,
    ))?;
    Ok(Some("Connector disconnected.".into()))
}

pub fn mcp_set_secret(core: &Arc<AppCore>, id: &str, secret: &str) -> Outcome {
    let request = req(json!({ "id": id, "secret": secret }))?;
    core.block_on(app_core::ai_runtime::mcp_set_server_secret(request))?;
    Ok(Some("Connector secret stored encrypted.".into()))
}

pub fn mcp_clear_secret(core: &Arc<AppCore>, id: &str) -> Outcome {
    let request = req(json!({ "id": id }))?;
    core.block_on(app_core::ai_runtime::mcp_clear_server_secret(request))?;
    Ok(Some("Connector secret removed.".into()))
}

pub fn delete_person(core: &Arc<AppCore>, person_id: i64) -> Outcome {
    let infra = infra(core)?;
    let request = req(json!({ "personId": person_id }))?;
    core.block_on(app_core::app_infra::delete_person_profile(request, &infra))?;
    Ok(Some("Voiceprint deleted.".into()))
}

/// `delete_speaker_analysis_model` (Speakers › Delete).
pub fn delete_speaker_model(core: &Arc<AppCore>, provider: String, model_id: String) -> Outcome {
    let infra = infra(core)?;
    core.block_on(
        app_core::speaker_analysis_models::delete_speaker_analysis_model(
            core,
            app_core::speaker_analysis_models::SpeakerAnalysisModelDownloadRequestDto {
                provider,
                model_id,
            },
            &infra,
        ),
    )?;
    Ok(None)
}
