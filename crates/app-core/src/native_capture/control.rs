//! Starting, stopping, pausing and resuming capture: the preflight checks a
//! start runs (sources, OCR and audio models, the microphone), the stop's
//! session bookkeeping, and the entry points every host surface calls (commands,
//! tray, shortcuts, auto-start, Ask AI's app control).

use std::sync::Arc;

use capture_microphone as microphone_capture;
use capture_types::{
    CaptureErrorResponse, NativeCaptureSessionResponse, RecordingSettings,
    StartNativeCaptureRequest,
};
use capture_vad::configured_adapter_as_str;

use crate::debug_log;
use crate::native_capture::app_notifications::{
    push_app_notification, AppNotification, AppNotificationsState,
};
use crate::native_capture::emit_native_capture_session_changed;
use crate::native_capture::lifecycle::{NativeCaptureState, StartRecordingLifecycleOutcome};
use crate::native_capture::log_format::*;
use crate::native_capture::metadata::{self, CaptureMetadataState};
use crate::native_capture::microphone::{
    resolve_capture_microphone_device_id, should_wait_for_same_microphone_device,
    MicrophoneControllerPreferencesState,
};
use crate::native_capture::model_warnings::*;
use crate::native_capture::permissions::get_capture_support;
use crate::native_capture::runtime::{self, validate_start_request};
use crate::native_capture::settings;
use crate::recording_settings::{
    current_recording_settings, initialize_recording_settings_state_from_disk,
    RecordingSettingsState,
};
use crate::AppCore;

fn start_native_capture_inner(
    origin: &str,
    request: StartNativeCaptureRequest,
    core: &Arc<AppCore>,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    let state = core.state::<NativeCaptureState>();
    let microphone_controller_preferences_state =
        core.state::<MicrophoneControllerPreferencesState>();
    let recording_settings_state = core.state::<RecordingSettingsState>();
    let app_notifications_state = core.state::<AppNotificationsState>();
    let incoming_sources = capture_sources_from_start_request(&request);
    // Held to the end of the start, so a sources save racing it is refused.
    let (settings, _sources_read) =
        settings::recording_settings_for_start(&recording_settings_state);

    let resolved_request = StartNativeCaptureRequest {
        capture_screen: settings.capture_screen,
        capture_microphone: settings.capture_microphone,
        capture_system_audio: settings.capture_system_audio,
    };
    let resolved_sources = capture_sources_from_start_request(&resolved_request);

    debug_log::log_info(format!(
        "attempting native capture {origin} start (incoming_sources={}, resolved_sources={}, save_directory='{}')",
        format_capture_source_flags(&incoming_sources),
        format_capture_source_flags(&resolved_sources),
        settings.save_directory
    ));

    let support = get_capture_support();
    let sources = match validate_start_request(&resolved_request, &support) {
        Ok(sources) => sources,
        Err(error) => {
            debug_log::log_warn(format!(
                "rejected native capture {origin} start during source validation (resolved_sources={}, supported_sources={}): [{}] {}",
                format_capture_source_flags(&resolved_sources),
                format_capture_source_flags(&support.supported_sources),
                error.code,
                error.message
            ));
            return Err(error);
        }
    };

    if resolved_request.capture_screen && settings.ocr.enabled {
        let app_data_dir = core.paths().config.clone();
        match crate::ocr_models::selected_ocr_model_available(&app_data_dir, &settings.ocr) {
            Ok(true) => {}
            Ok(false) => {
                maybe_push_ocr_unavailable_start_warning(core, &app_notifications_state, &settings);
                let error = CaptureErrorResponse {
                    code: "ocr_model_unavailable".to_string(),
                    message: format!(
                        "{} is unavailable. Install or choose an available OCR engine before recording screen capture.",
                        ocr_selection_label(&settings.ocr)
                    ),
                };
                debug_log::log_warn(format!(
                    "rejected native capture {origin} start because OCR is unavailable: [{}] {}",
                    error.code, error.message
                ));
                return Err(error);
            }
            Err(status_error) => {
                maybe_push_ocr_unavailable_start_warning(core, &app_notifications_state, &settings);
                let error = CaptureErrorResponse {
                    code: "ocr_model_unavailable".to_string(),
                    message: format!(
                        "failed to verify OCR availability for {}: {status_error}",
                        ocr_selection_label(&settings.ocr)
                    ),
                };
                debug_log::log_warn(format!(
                    "rejected native capture {origin} start because OCR availability check failed: [{}] {}",
                    error.code, error.message
                ));
                return Err(error);
            }
        }
    }

    if recording_requires_speech_detector(&settings) {
        match selected_speech_detector_available(&settings) {
            Ok(true) => {}
            Ok(false) => {
                push_app_notification(
                    core,
                    &app_notifications_state,
                    speech_detector_unavailable_notification(runtime::now_unix_ms()),
                );
                return Err(CaptureErrorResponse {
                    code: "speech_detector_unavailable".to_string(),
                    message: "Selected speech detector is unavailable for the requested recording sources.".to_string(),
                });
            }
            Err(error) => {
                push_app_notification(
                    core,
                    &app_notifications_state,
                    speech_detector_unavailable_notification(runtime::now_unix_ms()),
                );
                return Err(CaptureErrorResponse {
                    code: "speech_detector_unavailable".to_string(),
                    message: format!("Failed to verify selected speech detector: {error}"),
                });
            }
        }
    }

    let app_data_dir_for_processing = if recording_requires_transcription_model(&settings)
        || recording_requires_speaker_analysis_model(&settings)
    {
        Some(core.paths().config.clone())
    } else {
        None
    };

    if recording_requires_transcription_model(&settings) {
        let app_data_dir = app_data_dir_for_processing
            .as_deref()
            .expect("processing dir should exist");
        match crate::audio_transcription_models::selected_audio_transcription_model_available(
            app_data_dir,
            &settings.transcription,
        ) {
            Ok(true) => {}
            Ok(false) => {
                maybe_push_audio_transcription_unavailable_start_warning(
                    core,
                    &app_notifications_state,
                    &settings,
                );
                return Err(CaptureErrorResponse {
                    code: "audio_transcription_model_unavailable".to_string(),
                    message: format!(
                        "{} is unavailable. Install or choose an available transcription model before recording requested audio.",
                        audio_transcription_selection_label(&settings.transcription)
                    ),
                });
            }
            Err(error) => {
                maybe_push_audio_transcription_unavailable_start_warning(
                    core,
                    &app_notifications_state,
                    &settings,
                );
                return Err(CaptureErrorResponse {
                    code: "audio_transcription_model_unavailable".to_string(),
                    message: format!("failed to verify transcription model availability: {error}"),
                });
            }
        }
    }

    if recording_requires_speaker_analysis_model(&settings) {
        let app_data_dir = app_data_dir_for_processing
            .as_deref()
            .expect("processing dir should exist");
        match selected_speaker_analysis_model_available(app_data_dir, &settings) {
            Ok(true) => {}
            Ok(false) => {
                push_app_notification(
                    core,
                    &app_notifications_state,
                    speaker_analysis_unavailable_notification(runtime::now_unix_ms()),
                );
                return Err(CaptureErrorResponse {
                    code: "speaker_analysis_model_unavailable".to_string(),
                    message: "Selected speaker analysis model is unavailable. Install or choose an available model before recording requested audio.".to_string(),
                });
            }
            Err(error) => {
                push_app_notification(
                    core,
                    &app_notifications_state,
                    speaker_analysis_unavailable_notification(runtime::now_unix_ms()),
                );
                return Err(CaptureErrorResponse {
                    code: "speaker_analysis_model_unavailable".to_string(),
                    message: format!(
                        "failed to verify speaker analysis model availability: {error}"
                    ),
                });
            }
        }
    }

    let microphone_device_id_for_capture = if resolved_request.capture_microphone {
        let preferences_runtime = microphone_controller_preferences_state
            .lock()
            .expect("microphone controller preferences state poisoned");
        let controller_state = match microphone_capture::microphone_controller_state(
            preferences_runtime.preference.clone(),
            preferences_runtime.disconnect_policy.clone(),
        ) {
            Ok(state) => state,
            Err(error) => {
                debug_log::log_error(format!(
                    "failed to resolve microphone controller state for native capture {origin} start: [{}] {}",
                    error.code, error.message
                ));
                return Err(error);
            }
        };

        if should_wait_for_same_microphone_device(&controller_state) {
            let error = CaptureErrorResponse {
                code: "microphone_device_unavailable_waiting_for_selected_device".to_string(),
                message: "The selected microphone is unavailable. Reconnect the same device or change microphone preference."
                    .to_string(),
            };
            debug_log::log_warn(format!(
                "rejected native capture {origin} start because the selected microphone is unavailable and wait-for-same-device is active: [{}] {}",
                error.code, error.message
            ));
            return Err(error);
        }

        resolve_capture_microphone_device_id(&controller_state)
    } else {
        None
    };

    let mut runtime = state.lock().expect("native capture state poisoned");
    if runtime.runtime().is_running {
        let existing_sources =
            format_optional_capture_source_flags(runtime.runtime().requested_sources.as_ref());
        let session_id = runtime_log_session_id(runtime.runtime());

        if runtime.runtime().requested_sources.as_ref() != Some(&sources) {
            let error = CaptureErrorResponse {
                code: "capture_session_already_running".to_string(),
                message: "A native capture session is already running with different sources"
                    .to_string(),
            };
            debug_log::log_warn(format!(
                "rejected native capture {origin} start because another session is already running (session_id='{}', existing_sources={}, requested_sources={}): [{}] {}",
                session_id,
                existing_sources,
                format_capture_source_flags(&sources),
                error.code,
                error.message
            ));
            return Err(error);
        }

        debug_log::log_info(format!(
            "native capture {origin} start requested while session is already running; returning existing session (session_id='{}', requested_sources={})",
            session_id, existing_sources
        ));

        return Ok(NativeCaptureSessionResponse {
            session: runtime.session(),
        });
    }

    let requested_sources_for_log = sources.clone();
    let started_session = match runtime.start(
        core.clone(),
        &settings,
        sources,
        microphone_device_id_for_capture,
    ) {
        Ok(StartRecordingLifecycleOutcome::Started(session)) => session,
        Ok(StartRecordingLifecycleOutcome::AlreadyRunning(session)) => {
            debug_log::log_info(format!(
                "native capture {origin} start requested while session is already running; returning existing session (session_id='{}', requested_sources={})",
                session_log_session_id(&session),
                format_optional_capture_source_flags(session.requested_sources.as_ref())
            ));

            return Ok(NativeCaptureSessionResponse { session });
        }
        Err(error) => {
            let message = format!(
                "failed to start native capture ({origin}, requested_sources={}): [{}] {}",
                format_capture_source_flags(&requested_sources_for_log),
                error.code,
                error.message
            );
            // Auto-start waits out a missing display (ADR 0021): not a give-up.
            if origin.starts_with("auto-start")
                && crate::native_capture::auto_start::is_display_unavailable(&error)
            {
                debug_log::log_warn(message);
            } else {
                debug_log::log_error(message);
            }
            return Err(error);
        }
    };

    debug_log::log_info(format!(
        "started native capture successfully ({origin}, session_id='{}', requested_sources={}, segment_index={}, save_directory='{}')",
        runtime_log_session_id(runtime.runtime()),
        format_optional_capture_source_flags(runtime.runtime().requested_sources.as_ref()),
        runtime.runtime().current_segment_index,
        settings.save_directory
    ));

    // Slice 6a: the Trial clock starts at first *successful* Capture, not launch.
    // `ensure_trial_started` writes once (idempotent), so firing it on every
    // successful start is safe; skip the async hop when already Licensed —
    // there is nothing to stamp. Fire-and-forget off the DB pool, matching the
    // spawn pattern used elsewhere in this file.
    if !matches!(
        crate::licensing::cached_status(core),
        Some(capture_types::LicenseStatus::Licensed { .. })
    ) {
        if let Some(infra) = core.try_state::<crate::AppInfraState>() {
            let infra = std::sync::Arc::clone(&*infra);
            let trial_core = core.clone();
            core.spawn(async move {
                crate::licensing::ensure_trial_started(
                    infra.pool(),
                    &trial_core,
                    runtime::now_unix_ms() as i64,
                )
                .await;
            });
        }
    }

    maybe_push_audio_transcription_unavailable_start_warning(
        core,
        &app_notifications_state,
        &settings,
    );
    if let Some(notice) = runtime.take_microphone_vad_fallback_notification() {
        let message = format!(
            "Configured microphone VAD '{}' could not run. Using '{}' for this recording session.",
            configured_adapter_as_str(notice.configured_adapter),
            notice.effective_adapter.as_str(),
        );
        debug_log::log_warn(format!(
            "microphone VAD fallback active: configured_adapter={}, effective_adapter={}, reason={}",
            configured_adapter_as_str(notice.configured_adapter),
            notice.effective_adapter.as_str(),
            notice.reason
        ));
        push_app_notification(
            core,
            &app_notifications_state,
            AppNotification {
                id: format!(
                    "microphone-vad-fallback-{}",
                    configured_adapter_as_str(notice.configured_adapter)
                ),
                severity: "warning".to_string(),
                title: "Microphone VAD fallback".to_string(),
                message,
                created_at_unix_ms: runtime::now_unix_ms(),
                action: None,
            },
        );
    }

    Ok(NativeCaptureSessionResponse {
        session: started_session,
    })
}

pub fn initialize_recording_settings_from_disk(core: &AppCore) {
    reset_capture_log_snapshots();
    let settings_state = core.state::<RecordingSettingsState>();
    let loaded = initialize_recording_settings_state_from_disk(core, &settings_state);

    debug_log::set_app_debug_logging_enabled(loaded.settings.developer_options_enabled);
    log_loaded_recording_settings(loaded.source, &loaded.settings);
}

pub use crate::native_capture::auto_start::maybe_auto_start_native_capture;

pub fn current_native_capture_session(core: &AppCore) -> capture_types::NativeCaptureSession {
    let state = core.state::<NativeCaptureState>();
    let runtime = state.lock().expect("native capture state poisoned");
    runtime.session()
}

pub fn current_recording_settings_from_core(core: &AppCore) -> RecordingSettings {
    let state = core.state::<RecordingSettingsState>();
    current_recording_settings(&state)
}

pub fn start_native_capture_from_core(
    origin: &str,
    core: &Arc<AppCore>,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    let response = start_native_capture_inner(
        origin,
        StartNativeCaptureRequest {
            capture_screen: false,
            capture_microphone: false,
            capture_system_audio: false,
        },
        core,
    )?;
    emit_native_capture_session_changed(core, &response.session);
    core.host().refresh_tray();
    Ok(response)
}

pub fn stop_native_capture_from_core(
    core: &Arc<AppCore>,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    let response = stop_native_capture_with_state(core)?;
    emit_native_capture_session_changed(core, &response.session);
    core.host().refresh_tray();
    Ok(response)
}

pub fn pause_native_capture_from_core(
    core: &Arc<AppCore>,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    let state = core.state::<NativeCaptureState>();
    let mut runtime = state.lock().expect("native capture state poisoned");
    let session = runtime.pause_user_capture(core)?;
    drop(runtime);
    emit_native_capture_session_changed(core, &session);
    core.host().refresh_tray();
    Ok(NativeCaptureSessionResponse { session })
}

pub fn resume_native_capture_from_core(
    core: &Arc<AppCore>,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    let state = core.state::<NativeCaptureState>();
    let mut runtime = state.lock().expect("native capture state poisoned");
    let session = runtime.resume_user_capture(core)?;
    drop(runtime);
    emit_native_capture_session_changed(core, &session);
    core.host().refresh_tray();
    Ok(NativeCaptureSessionResponse { session })
}

pub fn start_native_capture(
    request: StartNativeCaptureRequest,
    core: &Arc<AppCore>,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    let response = start_native_capture_inner("command", request, core)?;
    emit_native_capture_session_changed(core, &response.session);
    core.host().refresh_tray();
    Ok(response)
}

fn stop_native_capture_with_state(
    core: &Arc<AppCore>,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    let state = core.state::<NativeCaptureState>();
    let mut runtime = state.lock().expect("native capture state poisoned");
    let session_id = runtime_log_session_id(runtime.runtime()).to_string();
    let requested_sources = runtime.runtime().requested_sources.clone();
    let output_files_before_stop = runtime.runtime().output_files.clone();
    let source_session_ids_before_stop = runtime
        .runtime()
        .source_sessions
        .clone()
        .map(|source_sessions| {
            [
                source_sessions.screen,
                source_sessions.microphone,
                source_sessions.system_audio,
            ]
            .into_iter()
            .flatten()
            .map(|source_session| source_session.session_id)
            .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    debug_log::log_info(format!(
        "received native capture stop request (is_running={}, session_id='{}', requested_sources={}, output_files_before_stop={})",
        runtime.runtime().is_running,
        session_id,
        format_optional_capture_source_flags(requested_sources.as_ref()),
        format_output_file_counts(output_files_before_stop.as_ref())
    ));

    let session = match runtime.stop(core) {
        Ok(session) => session,
        Err(error) => {
            if capture_screen::should_preserve_runtime_on_stop_error(&error) {
                debug_log::log_error(format!(
                    "failed to stop native capture but preserved runtime for recovery (session_id='{}'): [{}] {}",
                    session_id,
                    error.code,
                    error.message
                ));
            } else {
                debug_log::log_error(format!(
                    "failed to stop native capture; runtime marked stopped (session_id='{}'): [{}] {}",
                    session_id, error.code, error.message
                ));
            }

            return Err(error);
        }
    };
    if let Some(metadata_state) = core.try_state::<CaptureMetadataState>() {
        metadata::reset_recording_session_privacy_state(&metadata_state);
    }

    debug_log::log_info(format!(
        "stopped native capture successfully (session_id='{}', requested_sources={}, finalized_outputs={})",
        session_log_session_id(&session),
        format_optional_capture_source_flags(session.requested_sources.as_ref()),
        format_output_file_counts(session.output_files.as_ref())
    ));

    if !source_session_ids_before_stop.is_empty() {
        if let Some(infra) = core.try_state::<crate::AppInfraState>() {
            let stopped_at = time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string());
            let source_session_ids = source_session_ids_before_stop;
            crate::ocr_budget::clear_sessions_for_base_dir(infra.base_dir(), &source_session_ids);
            let infra = std::sync::Arc::clone(&*infra);
            if let Err(error) = core.block_on(async move {
                infra
                    .capture_retention()
                    .complete_capture_sessions_for_source_session_ids(
                        &source_session_ids,
                        &stopped_at,
                        "completed",
                    )
                    .await
            }) {
                debug_log::log_error(format!(
                    "failed to mark capture session completed after stop: {error}"
                ));
            }
        }
    }

    Ok(NativeCaptureSessionResponse { session })
}

pub async fn stop_native_capture(
    core: &Arc<AppCore>,
) -> Result<NativeCaptureSessionResponse, CaptureErrorResponse> {
    // Stopping does a synchronous capture-index write (marking sessions
    // completed). This command runs on the main thread, so run the stop on the
    // blocking pool and await it — keeping the UI responsive while preserving
    // ordering (the completion write finishes before we emit the change event).
    // The tray/relaunch/shortcut entry points keep calling the sync helper
    // directly so they still complete in-line.
    let handle = core.clone();
    let response = core
        .spawn_blocking(move || stop_native_capture_with_state(&handle))
        .await
        .map_err(|error| CaptureErrorResponse {
            code: "stop_native_capture_join".to_string(),
            message: format!("stop native capture task failed: {error}"),
        })??;
    emit_native_capture_session_changed(core, &response.session);
    core.host().refresh_tray();
    Ok(response)
}
