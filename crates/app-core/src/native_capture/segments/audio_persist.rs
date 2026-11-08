//! Committed audio segments: their timing windows, processing admission, and
//! persistence; plus closing a stopped screen session's frame batches.

use super::*;

#[cfg(target_os = "macos")]
pub(crate) fn rfc3339_from_unix_ms(unix_ms: u64) -> String {
    time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(unix_ms) * 1_000_000)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

#[cfg(target_os = "macos")]
pub(crate) fn audio_file_duration_ms(file_path: &str) -> Option<u64> {
    use cidre::{av, ns};

    let _autorelease_pool = cidre::objc::autorelease_pool::AutoreleasePoolPage::push();
    let result = {
        let url = ns::Url::with_fs_path_str(file_path, false);
        let asset = av::UrlAsset::with_url(&url, None)?;
        audio_duration_time_to_ms(asset.duration())
    };

    result
}

#[cfg(target_os = "macos")]
pub(crate) fn audio_duration_time_to_ms(duration: cidre::cm::Time) -> Option<u64> {
    if !duration.is_numeric() || duration.value <= 0 || duration.scale <= 0 {
        return None;
    }

    let value_ms = i128::from(duration.value)
        .checked_mul(1_000)?
        .checked_add(i128::from(duration.scale / 2))?
        / i128::from(duration.scale);

    u64::try_from(value_ms).ok()
}

#[cfg(target_os = "macos")]
pub(crate) fn scheduled_audio_segment_started_at_unix_ms(
    source_session: &SourceSessionMeta,
    segment_index: u64,
    schedule: &SegmentSchedule,
) -> u64 {
    let duration_ms = schedule
        .segment_duration()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64;
    let offset_ms = segment_index.saturating_sub(1).saturating_mul(duration_ms);
    source_session.started_at_unix_ms.saturating_add(offset_ms)
}

#[cfg(target_os = "macos")]
pub(crate) fn audio_segment_window_for_file(
    source_session: &SourceSessionMeta,
    segment_index: u64,
    schedule: &SegmentSchedule,
    file_path: &str,
) -> (String, String) {
    let started_at_unix_ms = audio_segment_started_at_unix_ms_for_file(
        source_session,
        segment_index,
        schedule,
        file_path,
    );
    let scheduled_duration_ms = schedule
        .segment_duration()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64;
    let duration_ms = audio_file_duration_ms(file_path).unwrap_or(scheduled_duration_ms);

    audio_segment_window_from_duration_ms(started_at_unix_ms, duration_ms)
}

#[cfg(target_os = "macos")]
pub(crate) fn audio_segment_started_at_unix_ms_for_file(
    source_session: &SourceSessionMeta,
    segment_index: u64,
    schedule: &SegmentSchedule,
    file_path: &str,
) -> u64 {
    parse_audio_restart_started_at_unix_ms(file_path).unwrap_or_else(|| {
        scheduled_audio_segment_started_at_unix_ms(source_session, segment_index, schedule)
    })
}

#[cfg(target_os = "macos")]
pub(crate) fn audio_segment_window_from_duration_ms(
    started_at_unix_ms: u64,
    duration_ms: u64,
) -> (String, String) {
    let ended_at_unix_ms = started_at_unix_ms.saturating_add(duration_ms);

    (
        rfc3339_from_unix_ms(started_at_unix_ms),
        rfc3339_from_unix_ms(ended_at_unix_ms),
    )
}

#[cfg(target_os = "macos")]
pub(crate) fn committed_audio_segments_for_output_files(
    source_sessions: Option<&SourceSessions>,
    schedule: Option<&SegmentSchedule>,
    segment_index: u64,
    output_files: Option<&CaptureOutputFiles>,
) -> Vec<::app_infra::NewAudioSegment> {
    let (Some(source_sessions), Some(schedule), Some(output_files)) =
        (source_sessions, schedule, output_files)
    else {
        return Vec::new();
    };
    let Ok(segment_index_i64) = i64::try_from(segment_index) else {
        return Vec::new();
    };

    let mut segments = Vec::new();
    if let Some(source_session) = source_sessions.microphone.as_ref() {
        segments.extend(
            output_files
                .microphone_files
                .iter()
                .filter_map(|file_path| {
                    if !Path::new(file_path).is_file() {
                        return None;
                    }
                    let (started_at, ended_at) = audio_segment_window_for_file(
                        source_session,
                        segment_index,
                        schedule,
                        file_path,
                    );
                    Some(::app_infra::NewAudioSegment::new(
                        ::app_infra::AudioSegmentSourceKind::Microphone,
                        source_session.session_id.clone(),
                        segment_index_i64,
                        file_path.clone(),
                        started_at.clone(),
                        ended_at.clone(),
                    ))
                }),
        );
    }
    if let Some(source_session) = source_sessions.system_audio.as_ref() {
        segments.extend(
            output_files
                .system_audio_files
                .iter()
                .filter_map(|file_path| {
                    if !Path::new(file_path).is_file() {
                        return None;
                    }
                    let (started_at, ended_at) = audio_segment_window_for_file(
                        source_session,
                        segment_index,
                        schedule,
                        file_path,
                    );
                    Some(::app_infra::NewAudioSegment::new(
                        ::app_infra::AudioSegmentSourceKind::SystemAudio,
                        source_session.session_id.clone(),
                        segment_index_i64,
                        file_path.clone(),
                        started_at.clone(),
                        ended_at.clone(),
                    ))
                }),
        );
    }

    segments
}

#[cfg(target_os = "macos")]
pub(crate) fn transcription_admission_for_app_handle(
    core: &Arc<AppCore>,
) -> ::app_infra::AudioSegmentTranscriptionAdmission {
    let transcription_settings = match core
        .state::<crate::recording_settings::RecordingSettingsState>()
        .lock()
    {
        Ok(settings) => settings.settings.transcription.clone(),
        Err(_) => {
            crate::debug_log::log_error(
                "failed to read recording settings for audio transcription admission".to_string(),
            );
            return ::app_infra::AudioSegmentTranscriptionAdmission::disabled();
        }
    };

    if !transcription_settings.enabled || !transcription_settings.microphone_enabled {
        return ::app_infra::AudioSegmentTranscriptionAdmission::disabled();
    }

    let app_data_dir = core.paths().data.clone();
    let available = crate::audio_transcription_models::selected_audio_transcription_model_available(
        &app_data_dir,
        &transcription_settings,
    );

    match available {
        Ok(true) => {
            let provider = crate::audio_transcription_models::provider_id_for_settings(
                transcription_settings.provider,
            );
            let mut payload = ::app_infra::AudioTranscriptionJobPayload::new(
                provider,
                transcription_settings.model_id.clone(),
                transcription_settings.language.clone(),
            );
            payload.options =
                crate::audio_transcription_models::transcription_request_options_for_settings(
                    &transcription_settings,
                );
            let speaker_admission = speaker_analysis_admission_for_app_handle(core);
            if speaker_admission.enabled && speaker_admission.provider_available {
                if let Some(payload_json) = speaker_admission.payload_json.as_deref() {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(payload_json) {
                        payload.options.insert(
                            ::app_infra::SPEAKER_ANALYSIS_PAYLOAD_OPTION_KEY.to_string(),
                            value,
                        );
                    }
                }
            }
            match serde_json::to_string(&payload) {
                Ok(payload_json) => {
                    ::app_infra::AudioSegmentTranscriptionAdmission::available(payload_json)
                }
                Err(error) => {
                    crate::debug_log::log_error(format!(
                        "failed to serialize audio transcription admission payload: {error}"
                    ));
                    ::app_infra::AudioSegmentTranscriptionAdmission::unavailable()
                }
            }
        }
        Ok(false) => ::app_infra::AudioSegmentTranscriptionAdmission::unavailable(),
        Err(error) => {
            crate::debug_log::log_warn(format!(
                "failed to inspect selected audio transcription model: {error}"
            ));
            ::app_infra::AudioSegmentTranscriptionAdmission::unavailable()
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn system_audio_speech_admission_for_app_handle(
    core: &Arc<AppCore>,
) -> ::app_infra::SystemAudioSpeechActivityAdmission {
    let settings = match core
        .state::<crate::recording_settings::RecordingSettingsState>()
        .lock()
    {
        Ok(settings) => settings.settings.clone(),
        Err(_) => {
            crate::debug_log::log_error(
                "failed to read recording settings for system-audio speech admission".to_string(),
            );
            return ::app_infra::SystemAudioSpeechActivityAdmission::disabled();
        }
    };

    if !settings.transcription.enabled || !settings.transcription.system_audio_enabled {
        return ::app_infra::SystemAudioSpeechActivityAdmission::disabled();
    }
    if settings.audio_speech_detection.detector == capture_types::AudioSpeechDetector::Off {
        return ::app_infra::SystemAudioSpeechActivityAdmission::unavailable();
    }

    let transcription_admission = {
        let app_data_dir = core.paths().data.clone();
        let available =
            crate::audio_transcription_models::selected_audio_transcription_model_available(
                &app_data_dir,
                &settings.transcription,
            );
        match available {
            Ok(true) => {
                let provider = crate::audio_transcription_models::provider_id_for_settings(
                    settings.transcription.provider,
                );
                let mut payload = ::app_infra::AudioTranscriptionJobPayload::new(
                    provider,
                    settings.transcription.model_id.clone(),
                    settings.transcription.language.clone(),
                );
                payload.options =
                    crate::audio_transcription_models::transcription_request_options_for_settings(
                        &settings.transcription,
                    );
                let speaker_admission = speaker_analysis_admission_for_app_handle(core);
                if speaker_admission.enabled && speaker_admission.provider_available {
                    if let Some(payload_json) = speaker_admission.payload_json.as_deref() {
                        if let Ok(value) = serde_json::from_str::<serde_json::Value>(payload_json) {
                            payload.options.insert(
                                ::app_infra::SPEAKER_ANALYSIS_PAYLOAD_OPTION_KEY.to_string(),
                                value,
                            );
                        }
                    }
                }
                serde_json::to_string(&payload).ok()
            }
            Ok(false) | Err(_) => None,
        }
    };

    let Some(transcription_payload) = transcription_admission else {
        return ::app_infra::SystemAudioSpeechActivityAdmission::unavailable();
    };
    let payload = ::app_infra::SystemAudioSpeechActivityJobPayload {
        detector: settings.audio_speech_detection.detector,
        transcription_payload,
        speaker_analysis_payload: speaker_analysis_admission_for_app_handle(core).payload_json,
    };
    match serde_json::to_string(&payload) {
        Ok(payload_json) => {
            ::app_infra::SystemAudioSpeechActivityAdmission::available(payload_json)
        }
        Err(error) => {
            crate::debug_log::log_error(format!(
                "failed to serialize system-audio speech admission payload: {error}"
            ));
            ::app_infra::SystemAudioSpeechActivityAdmission::unavailable()
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn speaker_analysis_admission_for_app_handle(
    core: &Arc<AppCore>,
) -> ::app_infra::AudioSegmentSpeakerAnalysisAdmission {
    let speaker_settings = match core
        .state::<crate::recording_settings::RecordingSettingsState>()
        .lock()
    {
        Ok(settings) => settings.settings.speaker_analysis.clone(),
        Err(_) => {
            crate::debug_log::log_error(
                "failed to read recording settings for speaker analysis admission".to_string(),
            );
            return ::app_infra::AudioSegmentSpeakerAnalysisAdmission::disabled();
        }
    };
    if !speaker_settings.separate_speakers {
        return ::app_infra::AudioSegmentSpeakerAnalysisAdmission::disabled();
    }
    let app_data_dir = core.paths().data.clone();
    let models_dir = speaker_analysis::speaker_analysis_models_dir(&app_data_dir);
    let manifest = speaker_analysis::builtin_model_manifest();
    let Some(descriptor) = speaker_analysis::find_model_descriptor(
        &manifest,
        &speaker_settings.provider,
        speaker_settings.model_id.as_deref(),
    ) else {
        return ::app_infra::AudioSegmentSpeakerAnalysisAdmission::unavailable();
    };
    match speaker_analysis::detect_model_status(&models_dir, descriptor) {
        Ok(status) if status.status == speaker_analysis::ModelStatusKind::Installed => {
            let mut payload = ::app_infra::SpeakerAnalysisJobPayload::new(
                speaker_settings.provider.clone(),
                speaker_settings.model_id.clone(),
            );
            payload.normalize_model_selection();
            payload.recognize_people = speaker_settings.recognize_saved_people;
            payload.auto_label_owner = speaker_settings.auto_label_owner;
            match serde_json::to_string(&payload) {
                Ok(payload_json) => {
                    ::app_infra::AudioSegmentSpeakerAnalysisAdmission::available(payload_json)
                }
                Err(error) => {
                    crate::debug_log::log_error(format!(
                        "failed to serialize speaker analysis admission payload: {error}"
                    ));
                    ::app_infra::AudioSegmentSpeakerAnalysisAdmission::unavailable()
                }
            }
        }
        Ok(_) => ::app_infra::AudioSegmentSpeakerAnalysisAdmission::unavailable(),
        Err(error) => {
            crate::debug_log::log_warn(format!(
                "failed to inspect selected speaker analysis model: {error}"
            ));
            ::app_infra::AudioSegmentSpeakerAnalysisAdmission::unavailable()
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn persist_committed_audio_segments(
    core: Option<&Arc<AppCore>>,
    source_sessions: Option<&SourceSessions>,
    schedule: Option<&SegmentSchedule>,
    segment_index: u64,
    output_files: Option<&CaptureOutputFiles>,
) {
    let (Some(core), Some(source_sessions), Some(schedule), Some(output_files)) =
        (core, source_sessions, schedule, output_files)
    else {
        return;
    };
    let segments = committed_audio_segments_for_output_files(
        Some(source_sessions),
        Some(schedule),
        segment_index,
        Some(output_files),
    );

    if segments.is_empty() {
        return;
    }

    let infra = Arc::clone(&*core.state::<crate::AppInfraState>());
    let core = core.clone();
    let persistence = run_native_capture_async("audio-segment-persistence", async move {
        let mut persisted_any = false;
        let transcription_admission = transcription_admission_for_app_handle(&core);
        let speaker_admission = speaker_analysis_admission_for_app_handle(&core);
        let system_audio_speech_admission = system_audio_speech_admission_for_app_handle(&core);
        for segment in segments {
            match infra
                .upsert_audio_segment_and_maybe_enqueue_processing(
                    &segment,
                    &transcription_admission,
                    &speaker_admission,
                    &system_audio_speech_admission,
                )
                .await
            {
                Ok(_) => {
                    persisted_any = true;
                }
                Err(error) => {
                    crate::debug_log::log_error(format!(
                        "failed to persist native audio segment {}: {}",
                        segment.file_path, error
                    ));
                }
            }
        }

        if persisted_any {
            core.host().emit(CoreEvent::AudioSegmentsChanged);
        }
    });

    if let Err(error) = persistence {
        crate::debug_log::log_error(format!("native audio segment persistence failed: {error}"));
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn warm_scrub_previews_for_committed_screen_outputs(
    core: Option<&Arc<AppCore>>,
    output_files: Option<&CaptureOutputFiles>,
) {
    let (Some(core), Some(output_files)) = (core, output_files) else {
        return;
    };
    let screen_files = if output_files.screen_files.is_empty() {
        output_files.screen_file.iter().cloned().collect::<Vec<_>>()
    } else {
        output_files.screen_files.clone()
    };
    if screen_files.is_empty() {
        return;
    }

    match crate::app_infra::frame_preview::enqueue_scrub_preview_generation_for_screen_files(
        core,
        &screen_files,
    ) {
        Ok(_) => {}
        Err(error) => {
            crate::debug_log::log_warn(format!(
                "failed to enqueue scrub previews for finalized screen segment: {error}"
            ));
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) async fn close_frame_batches_for_stopped_screen_session_id_async(
    infra: &crate::AppInfraState,
    session_id: &str,
) -> Result<(), CaptureErrorResponse> {
    let session_id = session_id.to_string();
    let infra = Arc::clone(infra);

    infra
        .close_and_schedule_all_frame_batches_for_session(&session_id)
        .await
        .map(|_| ())
        .map_err(|error| {
            crate::debug_log::log_warn(format!(
                "failed to close frame batches for stopped screen session {session_id}: {error}"
            ));
            CaptureErrorResponse {
                code: "frame_batch_close_failed".to_string(),
                message: format!(
                    "Failed to close frame batches for stopped screen session {session_id}: {error}"
                ),
            }
        })
}

#[cfg(target_os = "macos")]
pub(crate) fn close_frame_batches_for_stopped_screen_session_id(
    infra: &crate::AppInfraState,
    session_id: &str,
) -> Result<(), CaptureErrorResponse> {
    let infra = Arc::clone(infra);
    let session_id = session_id.to_string();

    match run_native_capture_async("frame-batch-close", async move {
        close_frame_batches_for_stopped_screen_session_id_async(&infra, &session_id).await
    }) {
        Ok(result) => result,
        Err(error) => Err(CaptureErrorResponse {
            code: "frame_batch_close_failed".to_string(),
            message: format!("Failed to close frame batches for stopped screen session: {error}"),
        }),
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn close_frame_batches_for_stopped_screen_session(
    core: Option<&Arc<AppCore>>,
    source_sessions: Option<&SourceSessions>,
) -> Result<(), CaptureErrorResponse> {
    let Some(screen_session) = source_sessions.and_then(|sessions| sessions.screen.as_ref()) else {
        return Ok(());
    };

    let Some(core) = core else {
        return Ok(());
    };

    let infra = Arc::clone(&*core.state::<crate::AppInfraState>());
    close_frame_batches_for_stopped_screen_session_id(&infra, &screen_session.session_id)
}

#[cfg(target_os = "macos")]
pub(crate) fn persist_committed_microphone_segments(
    core: Option<&Arc<AppCore>>,
    source_sessions: Option<&SourceSessions>,
    schedule: Option<&SegmentSchedule>,
    segment_index: u64,
    output_files: Option<&CaptureOutputFiles>,
) {
    let Some(output_files) = output_files else {
        return;
    };
    let mut microphone_only = empty_output_files();
    microphone_only.microphone_file = output_files.microphone_file.clone();
    microphone_only.microphone_files = output_files.microphone_files.clone();
    persist_committed_audio_segments(
        core,
        source_sessions,
        schedule,
        segment_index,
        Some(&microphone_only),
    );
}

#[cfg(target_os = "macos")]
pub(crate) fn persist_committed_system_audio_segments(
    core: Option<&Arc<AppCore>>,
    source_sessions: Option<&SourceSessions>,
    schedule: Option<&SegmentSchedule>,
    segment_index: u64,
    output_files: Option<&CaptureOutputFiles>,
) {
    let Some(output_files) = output_files else {
        return;
    };
    let mut system_audio_only = empty_output_files();
    system_audio_only.system_audio_file = output_files.system_audio_file.clone();
    system_audio_only.system_audio_files = output_files.system_audio_files.clone();
    persist_committed_audio_segments(
        core,
        source_sessions,
        schedule,
        segment_index,
        Some(&system_audio_only),
    );
}
