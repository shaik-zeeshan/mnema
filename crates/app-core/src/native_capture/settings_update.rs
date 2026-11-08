//! Saving recording settings: the full save and every per-domain save, and what
//! a landed save sets off (debug logging, the exclusion list, OCR job cleanup,
//! absent-model locks, Whisper unload, MCP secret clearing and connector
//! reconcile, the retention wake, the change events, the privacy refresh, the
//! tray).

use std::sync::Arc;

use capture_types::{
    AudioTranscriptionProvider, CaptureErrorResponse, CaptureSources, RecordingSettings,
    RecordingSettingsDomainUpdateResponse, SettingsOwnershipDomain, UpdateAccessSettingsRequest,
    UpdateAiRuntimeSettingsRequest, UpdateCaptureSourceSettingsRequest,
    UpdateCaptureTimingSettingsRequest, UpdateDeveloperSettingsRequest,
    UpdateDisplaySettingsRequest, UpdateInactivitySettingsRequest, UpdateMetadataSettingsRequest,
    UpdateProcessingSettingsRequest, UpdateRecordingSettingsRequest, UpdateStorageSettingsRequest,
    UpdateUserContextSettingsRequest, UpdateVideoSettingsRequest,
};

use crate::debug_log;
use crate::native_capture::log_format::log_recording_settings_changes;
use crate::native_capture::settings::{
    apply_recording_settings_domain_mutation, apply_recording_settings_domain_patch,
    apply_recording_settings_update,
};
use crate::recording_settings::{
    current_recording_settings, RecordingSettingsDomainPatch, RecordingSettingsState,
};
use crate::{AppCore, CoreEvent};

pub fn emit_recording_settings_changed(core: &AppCore, settings: &RecordingSettings) {
    core.host().emit(CoreEvent::RecordingSettingsChanged(
        serde_json::to_value(settings).unwrap_or(serde_json::Value::Null),
    ));
}

pub fn emit_recording_settings_domain_changed(
    core: &AppCore,
    response: &RecordingSettingsDomainUpdateResponse,
) {
    core.host().emit(CoreEvent::RecordingSettingsDomainChanged(
        serde_json::to_value(response).unwrap_or(serde_json::Value::Null),
    ));
}

/// Whether this save moved any model-backed subsystem's provider or model id.
/// Those are exactly the fields the absent-model lock keys off, so a change here
/// means the locks must be re-derived from what is on disk.
pub fn model_selection_changed(previous: &RecordingSettings, next: &RecordingSettings) -> bool {
    previous.ocr.provider != next.ocr.provider
        || previous.ocr.model_id != next.ocr.model_id
        || previous.transcription.provider != next.transcription.provider
        || previous.transcription.model_id != next.transcription.model_id
        || previous.speaker_analysis.provider != next.speaker_analysis.provider
        || previous.speaker_analysis.model_id != next.speaker_analysis.model_id
}

fn finish_recording_settings_update(
    core: &Arc<AppCore>,
    update: crate::recording_settings::AppliedRecordingSettingsUpdate,
    domain: Option<SettingsOwnershipDomain>,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    let settings = update.settings;
    let previous_settings = update.previous_settings;
    let previous_save_directory = update.previous_save_directory;
    let save_directory_changed = update.save_directory_changed;

    debug_log::set_app_debug_logging_enabled(settings.developer_options_enabled);
    if previous_settings.developer_options_enabled != settings.developer_options_enabled {
        core.host()
            .window_signal(crate::WindowSignal::CaptureExclusionChanged);
    }
    log_recording_settings_changes(&previous_settings, &settings);

    if save_directory_changed {
        debug_log::log_info(format!(
            "recording save directory changed from '{}' to '{}'; app infrastructure database location will update on next app start",
            previous_save_directory, settings.save_directory
        ));
    }

    if previous_settings.ocr.enabled && !settings.ocr.enabled {
        if let Some(infra) = core.try_state::<crate::AppInfraState>() {
            // Fire-and-forget off the main thread: this only marks already-queued
            // OCR jobs failed now that OCR is disabled (cleanup whose result is
            // merely logged). Settings commands run on the main thread, so we must
            // not block the UI awaiting this capture-index write.
            let infra = std::sync::Arc::clone(&*infra);
            core.spawn(async move {
                match infra.fail_queued_ocr_jobs_because_disabled().await {
                    Ok(failed_count) => debug_log::log_info(format!(
                        "marked queued OCR jobs failed because OCR was disabled (count={failed_count})"
                    )),
                    Err(error) => debug_log::log_error(format!(
                        "failed to mark queued OCR jobs failed after disabling OCR: {error}"
                    )),
                }
            });
        } else {
            debug_log::log_warn(
                "app infrastructure state unavailable while disabling OCR; queued OCR jobs were not updated",
            );
        }
    }

    // A model SELECTION change can make the selected model absent without any
    // download or deletion running, and neither of the absent lane's two owners
    // (startup, download-settle) fires here. Without this, every job admitted
    // against the newly selected but not-yet-installed model is claimed and burns
    // its three failure attempts — the exact loss claim-time gating exists to
    // prevent, and the non-blocking setup flow makes "selected but not installed"
    // the normal case. Fire-and-forget for the same reason as the OCR pass above:
    // settings commands run on the main thread.
    if model_selection_changed(&previous_settings, &settings) {
        let task_core = core.clone();
        core.spawn(async move {
            crate::model_download_lock::reconcile_absent_model_locks(&task_core).await;
        });
    }

    // Local Whisper contexts have no idle-unload (unlike Parakeet) and otherwise
    // live cached until app quit, so switching away from Whisper stranded ~1GB of
    // model weights in memory. Free them at the save that lands the switch.
    let whisper_was_active = previous_settings.transcription.enabled
        && previous_settings.transcription.provider == AudioTranscriptionProvider::LocalWhisper;
    let whisper_now_active = settings.transcription.enabled
        && settings.transcription.provider == AudioTranscriptionProvider::LocalWhisper;
    if whisper_was_active && !whisper_now_active {
        match audio_transcription::providers::local_whisper::unload_all_cached_contexts() {
            Ok(unloaded) => debug_log::log_info(format!(
                "unloaded {unloaded} cached Local Whisper context(s) after switching transcription away from Whisper"
            )),
            Err(error) => debug_log::log_warn(format!(
                "failed to unload cached Local Whisper contexts after switching away from Whisper: {error}"
            )),
        }
    }

    // An MCP connector whose auth mode flipped (Bearer↔OAuth) leaves the OLD
    // mode's payload in its polymorphic keychain slot — worst case the OAuth
    // Token Set JSON attached verbatim as an `Authorization: Bearer` header.
    // Clear the slot at the save that lands the flip, so neither path can read
    // the other mode's leftover (ADR 0051; the transport also refuses a
    // mismatched payload as a backstop).
    for id in crate::ask_ai::mcp::auth_mode_flipped_ids(
        &previous_settings.ai_runtime.mcp_servers,
        &settings.ai_runtime.mcp_servers,
    ) {
        match app_infra::delete_mcp_server_secret(&id) {
            Ok(()) => debug_log::log_info(format!(
                "cleared the keychain secret for MCP connector \"{id}\" after its auth mode changed"
            )),
            Err(error) => debug_log::log_error(format!(
                "failed to clear the keychain secret for MCP connector \"{id}\" after its auth mode changed: {error}"
            )),
        }
    }

    if previous_settings.retention_policy != settings.retention_policy {
        if let Some(background_workers) =
            core.try_state::<crate::app_infra::BackgroundWorkersState>()
        {
            background_workers.notify_retention_schedule_changed();
        } else {
            debug_log::log_warn(
                "background workers state unavailable while updating retention policy; retention cleanup schedule was not woken",
            );
        }
    }

    emit_recording_settings_changed(core, &settings);
    if let Some(domain) = domain {
        emit_recording_settings_domain_changed(
            core,
            &RecordingSettingsDomainUpdateResponse {
                domain,
                settings: settings.clone(),
            },
        );
    }
    let privacy_changed = previous_settings.privacy != settings.privacy;
    let metadata_changed = previous_settings.metadata != settings.metadata;
    if metadata_changed {
        crate::native_capture::privacy::refresh::request_privacy_filter_refresh(
            core,
            crate::native_capture::privacy::PrivacyRefreshReason::MetadataSettingsMutation,
        );
    } else if privacy_changed {
        crate::native_capture::privacy::refresh::request_privacy_filter_refresh(
            core,
            crate::native_capture::privacy::PrivacyRefreshReason::StaticAppRuleMutation,
        );
    }
    core.host().refresh_tray();

    Ok(settings)
}

fn finish_recording_settings_domain_update(
    core: &Arc<AppCore>,
    domain: SettingsOwnershipDomain,
    update: crate::recording_settings::AppliedRecordingSettingsUpdate,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    let settings = finish_recording_settings_update(core, update, Some(domain))?;
    Ok(RecordingSettingsDomainUpdateResponse { domain, settings })
}

pub fn apply_recording_settings_domain_mutation_from_core(
    core: &Arc<AppCore>,
    domain: SettingsOwnershipDomain,
    mutate: impl FnOnce(&mut RecordingSettings) -> Result<(), CaptureErrorResponse>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    let state = core.state::<RecordingSettingsState>();
    let update = apply_recording_settings_domain_mutation(core, &state, domain, mutate)?;
    finish_recording_settings_domain_update(core, domain, update)
}

pub fn update_recording_sources_from_core(
    core: &Arc<AppCore>,
    sources: CaptureSources,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    let state = core.state::<RecordingSettingsState>();
    let response = update_recording_settings_domain(
        core,
        &state,
        RecordingSettingsDomainPatch::CaptureSources(UpdateCaptureSourceSettingsRequest {
            capture_screen: Some(sources.screen),
            capture_microphone: Some(sources.microphone),
            capture_system_audio: Some(sources.system_audio),
        }),
    )?;
    Ok(response.settings)
}

pub fn get_recording_settings(core: &AppCore) -> RecordingSettings {
    current_recording_settings(&core.state::<RecordingSettingsState>())
}

pub fn update_recording_settings(
    request: UpdateRecordingSettingsRequest,
    core: &Arc<AppCore>,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    let update =
        apply_recording_settings_update(core, &core.state::<RecordingSettingsState>(), request)?;
    finish_recording_settings_update(core, update, None)
}

fn update_recording_settings_domain(
    core: &Arc<AppCore>,
    state: &RecordingSettingsState,
    patch: RecordingSettingsDomainPatch,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    let (domain, update) = apply_recording_settings_domain_patch(core, state, patch)?;
    finish_recording_settings_domain_update(core, domain, update)
}

/// Off the main thread: a sources change asks the capture lock, which a stop
/// can hold for seconds while frame artifacts flush.
pub async fn update_capture_source_settings(
    request: UpdateCaptureSourceSettingsRequest,
    core: &Arc<AppCore>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    let task_core = core.clone();
    core.spawn_blocking(move || {
        update_recording_settings_domain(
            &task_core,
            &task_core.state::<RecordingSettingsState>(),
            RecordingSettingsDomainPatch::CaptureSources(request),
        )
    })
    .await
    .map_err(|error| CaptureErrorResponse {
        code: "capture_source_settings_update_task_failed".to_string(),
        message: format!("Capture source settings update task failed: {error}"),
    })?
}

pub fn update_capture_timing_settings(
    request: UpdateCaptureTimingSettingsRequest,
    core: &Arc<AppCore>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    update_recording_settings_domain(
        core,
        &core.state::<RecordingSettingsState>(),
        RecordingSettingsDomainPatch::CaptureTiming(request),
    )
}

pub fn update_video_settings(
    request: UpdateVideoSettingsRequest,
    core: &Arc<AppCore>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    update_recording_settings_domain(
        core,
        &core.state::<RecordingSettingsState>(),
        RecordingSettingsDomainPatch::Video(request),
    )
}

pub fn update_storage_settings(
    request: UpdateStorageSettingsRequest,
    core: &Arc<AppCore>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    update_recording_settings_domain(
        core,
        &core.state::<RecordingSettingsState>(),
        RecordingSettingsDomainPatch::Storage(request),
    )
}

pub fn update_display_settings(
    request: UpdateDisplaySettingsRequest,
    core: &Arc<AppCore>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    update_recording_settings_domain(
        core,
        &core.state::<RecordingSettingsState>(),
        RecordingSettingsDomainPatch::Display(request),
    )
}

pub fn update_metadata_settings(
    request: UpdateMetadataSettingsRequest,
    core: &Arc<AppCore>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    update_recording_settings_domain(
        core,
        &core.state::<RecordingSettingsState>(),
        RecordingSettingsDomainPatch::Metadata(request),
    )
}

pub fn update_inactivity_settings(
    request: UpdateInactivitySettingsRequest,
    core: &Arc<AppCore>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    update_recording_settings_domain(
        core,
        &core.state::<RecordingSettingsState>(),
        RecordingSettingsDomainPatch::Inactivity(request),
    )
}

pub fn update_processing_settings(
    request: UpdateProcessingSettingsRequest,
    core: &Arc<AppCore>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    update_recording_settings_domain(
        core,
        &core.state::<RecordingSettingsState>(),
        RecordingSettingsDomainPatch::Processing(request),
    )
}

pub fn update_developer_settings(
    request: UpdateDeveloperSettingsRequest,
    core: &Arc<AppCore>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    update_recording_settings_domain(
        core,
        &core.state::<RecordingSettingsState>(),
        RecordingSettingsDomainPatch::Developer(request),
    )
}

pub fn update_access_settings(
    request: UpdateAccessSettingsRequest,
    core: &Arc<AppCore>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    update_recording_settings_domain(
        core,
        &core.state::<RecordingSettingsState>(),
        RecordingSettingsDomainPatch::Access(request),
    )
}

pub fn update_ai_runtime_settings(
    request: UpdateAiRuntimeSettingsRequest,
    core: &Arc<AppCore>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    let response = update_recording_settings_domain(
        core,
        &core.state::<RecordingSettingsState>(),
        RecordingSettingsDomainPatch::AiRuntime(request),
    )?;
    // MCP teardown (ADR 0048): reconcile the connection cache against the
    // just-saved settings so removed/disabled connectors' handles — and their
    // child processes — are dropped. Fire-and-forget: teardown never blocks the
    // save, and an EDITED connector is reaped lazily on next use.
    if let Some(manager) = core.try_state::<crate::ask_ai::mcp::McpManager>() {
        let manager = (*manager).clone();
        let task_core = core.clone();
        core.spawn(async move {
            manager.reconcile(&task_core).await;
        });
    }
    Ok(response)
}

pub fn update_user_context_settings(
    request: UpdateUserContextSettingsRequest,
    core: &Arc<AppCore>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    update_recording_settings_domain(
        core,
        &core.state::<RecordingSettingsState>(),
        RecordingSettingsDomainPatch::UserContext(request),
    )
}

/// Update the **Semantic Search** settings that do NOT change the active vector
/// dimension: the enabled toggle (issue #125). A model *switch* goes through the
/// atomic `select_semantic_search_model` command instead — it rebuilds the `vec0`
/// table at the new model's dimension and persists the selection together, so the
/// persisted model and the live table dimension can never disagree. The
/// **Semantic Index Backfill** worker reloads the embedder on its next pass when
/// the provider/model id changes.
pub fn update_semantic_search_settings(
    request: capture_types::UpdateSemanticSearchSettingsRequest,
    core: &Arc<AppCore>,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    update_recording_settings_domain(
        core,
        &core.state::<RecordingSettingsState>(),
        RecordingSettingsDomainPatch::SemanticSearch(request),
    )
}

/// Persist a **Semantic Search Model Tier** patch from outside the command layer
/// (the atomic `select_semantic_search_model` switch, which rebuilds the `vec0`
/// table *before* persisting so the persisted `model_id` and the live table
/// dimension never disagree). Reuses the same domain-update path as
/// [`update_semantic_search_settings`].
pub fn persist_semantic_search_settings(
    core: &Arc<AppCore>,
    state: &RecordingSettingsState,
    request: capture_types::UpdateSemanticSearchSettingsRequest,
) -> Result<RecordingSettingsDomainUpdateResponse, CaptureErrorResponse> {
    // Trusted path: the atomic switch has already rebuilt the `vec0` table to the
    // new model's dimension before this persist, so use the trusted variant that
    // honors `model_id`/`provider`. The generic IPC command
    // (`update_semantic_search_settings`) uses `SemanticSearch`, which ignores
    // those dimension-affecting fields. See review finding low #4 (PR #126).
    update_recording_settings_domain(
        core,
        state,
        RecordingSettingsDomainPatch::SemanticSearchModelSwitch(request),
    )
}
