//! The capture and processing pipeline adapter: a captured screen frame becomes
//! a frame row (with its segment and OCR admission), and the reprocess / debug
//! commands enqueue jobs with the payloads the current settings imply.

use super::*;

pub(super) fn audio_segment_mime_type(file_path: &Path) -> &'static str {
    match file_path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("m4a") => "audio/mp4; codecs=mp4a.40.2",
        Some("mp4") => "audio/mp4",
        Some("aac") => "audio/aac",
        Some("wav") => "audio/wav",
        _ => "application/octet-stream",
    }
}

pub(super) async fn get_audio_segment_media_inner(
    infra: &::app_infra::AppInfra,
    audio_segment_id: i64,
) -> ::app_infra::Result<Option<AudioSegmentMediaDto>> {
    let Some(segment) = infra.get_audio_segment(audio_segment_id).await? else {
        return Ok(None);
    };

    let file_path = PathBuf::from(&segment.file_path);
    let bytes = fs::read(&file_path).map_err(|error| {
        ::app_infra::AppInfraError::Io(std::io::Error::new(
            error.kind(),
            format!(
                "failed to read persisted audio segment {} at {}: {error}",
                segment.id,
                file_path.display()
            ),
        ))
    })?;

    Ok(Some(AudioSegmentMediaDto {
        mime_type: audio_segment_mime_type(&file_path).to_string(),
        data_base64: BASE64_STANDARD.encode(bytes),
    }))
}

pub(super) fn normalized_ocr_language_for_settings(settings: &OcrSettings) -> Option<String> {
    let language = settings
        .language
        .as_deref()
        .map(str::trim)
        .filter(|language| !language.is_empty());

    match settings.provider {
        OcrProvider::AppleVision => language.map(ToOwned::to_owned),
        OcrProvider::Tesseract => Some(
            language
                .unwrap_or(ocr::DEFAULT_TESSERACT_LANGUAGE)
                .to_string(),
        ),
        OcrProvider::PaddleOcr => Some(
            language
                .unwrap_or(ocr::DEFAULT_PADDLE_OCR_LANGUAGE)
                .to_string(),
        ),
    }
}

pub(super) fn normalized_ocr_model_id_for_settings(settings: &OcrSettings) -> Option<String> {
    let model_id = settings
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|model_id| !model_id.is_empty());

    match settings.provider {
        OcrProvider::AppleVision => None,
        OcrProvider::Tesseract => Some(
            model_id
                .unwrap_or(ocr::DEFAULT_TESSERACT_MODEL_ID)
                .to_string(),
        ),
        OcrProvider::PaddleOcr => Some(
            model_id
                .unwrap_or(ocr::DEFAULT_PADDLE_OCR_MODEL_ID)
                .to_string(),
        ),
    }
}

pub(super) fn provider_id_for_ocr_settings(provider: OcrProvider) -> &'static str {
    match provider {
        OcrProvider::AppleVision => ocr::APPLE_VISION_PROVIDER_ID,
        OcrProvider::Tesseract => ocr::TESSERACT_PROVIDER_ID,
        OcrProvider::PaddleOcr => ocr::PADDLE_OCR_PROVIDER_ID,
    }
}

pub(super) fn merged_ocr_payload_json(
    payload_json: Option<&str>,
    ocr_settings: &OcrSettings,
) -> Result<Option<String>, String> {
    let mut payload = ocr::FrozenOcrPayload::from_payload_json(payload_json)
        .map_err(|error| format!("failed to parse OCR payload JSON: {error}"))?;

    payload.provider = provider_id_for_ocr_settings(ocr_settings.provider).to_string();
    payload.model_id = normalized_ocr_model_id_for_settings(ocr_settings);
    payload.language = normalized_ocr_language_for_settings(ocr_settings);

    match ocr_settings.provider {
        OcrProvider::AppleVision => {
            payload.options.insert(
                "recognitionMode".to_string(),
                serde_json::to_value(ocr_settings.recognition_mode.clone()).map_err(|error| {
                    format!("failed to serialize OCR recognition mode: {error}")
                })?,
            );
            payload.options.insert(
                "languageCorrection".to_string(),
                serde_json::Value::Bool(ocr_settings.language_correction),
            );
            payload.options.remove("pageSegmentationMode");
            payload.options.remove("preprocessMode");
            payload.options.remove("upscaleFactor");
            payload.options.remove("charWhitelist");
        }
        OcrProvider::Tesseract => {
            payload.options.remove("recognitionMode");
            payload.options.remove("languageCorrection");
            payload.options.insert(
                "pageSegmentationMode".to_string(),
                serde_json::to_value(ocr_settings.tesseract_page_segmentation_mode).map_err(
                    |error| {
                        format!("failed to serialize Tesseract page segmentation mode: {error}")
                    },
                )?,
            );
            payload.options.insert(
                "preprocessMode".to_string(),
                serde_json::to_value(ocr_settings.tesseract_preprocess_mode).map_err(|error| {
                    format!("failed to serialize Tesseract preprocess mode: {error}")
                })?,
            );
            payload.options.insert(
                "upscaleFactor".to_string(),
                serde_json::Value::Number(serde_json::Number::from(
                    ocr_settings.tesseract_upscale_factor,
                )),
            );
            if let Some(whitelist) = ocr_settings
                .tesseract_char_whitelist
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                payload.options.insert(
                    "charWhitelist".to_string(),
                    serde_json::Value::String(whitelist.to_string()),
                );
            } else {
                payload.options.remove("charWhitelist");
            }
        }
        OcrProvider::PaddleOcr => {
            payload.options.remove("recognitionMode");
            payload.options.remove("languageCorrection");
            payload.options.remove("pageSegmentationMode");
            payload.options.remove("preprocessMode");
            payload.options.remove("upscaleFactor");
            payload.options.remove("charWhitelist");
        }
    }

    serde_json::to_string(&payload)
        .map(Some)
        .map_err(|error| format!("failed to serialize OCR payload JSON: {error}"))
}

pub(super) fn ocr_payload_json_from_settings(
    settings: &crate::recording_settings::RecordingSettingsState,
    payload_json: Option<&str>,
) -> Result<Option<String>, String> {
    let ocr_settings = settings
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .ocr
        .clone();

    merged_ocr_payload_json(payload_json, &ocr_settings)
}

pub(super) fn ocr_enabled_for_settings(
    settings: &crate::recording_settings::RecordingSettingsState,
) -> bool {
    settings
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .ocr
        .enabled
}

pub async fn persist_screen_frame_artifact(
    infra: &::app_infra::AppInfra,
    settings: &crate::recording_settings::RecordingSettingsState,
    metadata_snapshot: Option<capture_metadata::FrameMetadataSnapshot>,
    session_id: &str,
    artifact: ScreenFrameArtifact,
) -> ::app_infra::Result<::app_infra::CapturedFramePipelineResult> {
    let ScreenFrameArtifact {
        file_path,
        captured_at_unix_ms,
        width,
        height,
        captured_frame_equivalence,
    } = artifact;
    let mut frame = ::app_infra::NewFrame::new(
        session_id,
        file_path.clone(),
        captured_at_from_unix_ms(captured_at_unix_ms),
    );
    if let Some(capture_segment_id) =
        ensure_screen_capture_segment_for_frame(infra, session_id, &file_path, &frame.captured_at)
            .await?
    {
        frame = frame.with_capture_segment_id(capture_segment_id);
    }
    if let Some(metadata_snapshot) = metadata_snapshot {
        frame = frame.with_metadata_snapshot(metadata_snapshot);
    }

    if let (Some(width), Some(height)) = (width, height) {
        frame = frame.with_dimensions(i64::from(width), i64::from(height));
    }

    frame = match captured_frame_equivalence {
        capture_screen::CapturedFrameEquivalenceOutcome::Ready(equivalence) => frame
            .with_equivalence(::app_infra::FrameEquivalence::ready(
                equivalence.hint,
                equivalence.proof,
                equivalence.version,
            )),
        capture_screen::CapturedFrameEquivalenceOutcome::Quarantined(error) => {
            crate::debug_log::log_warn(format!(
                "quarantined captured frame equivalence for session {} artifact {}: {}",
                session_id, file_path, error
            ));
            frame.with_equivalence(::app_infra::FrameEquivalence::quarantined(error))
        }
    };

    if !ocr_enabled_for_settings(settings) {
        let decision = ::app_infra::OcrAdmissionDecision::skip(
            ::app_infra::OcrAdmissionReason::SkippedOcrDisabled,
            infra
                .count_queued_or_running_processing_jobs_for_processor(::app_infra::OCR_PROCESSOR)
                .await?,
            true,
        );
        let result = infra
            .capture_frame_skipping_ocr_with_reason(&frame, decision)
            .await?;
        crate::ocr_budget::record_admission_result(infra, &result);
        return Ok(result);
    }

    let payload_json = ocr_payload_json_from_settings(settings, None)
        .map_err(::app_infra::AppInfraError::OcrEngine)?;

    let decision = crate::ocr_budget::decide_admission(infra, &frame, true).await?;
    let result = infra
        .capture_frame_with_ocr_admission(&frame, payload_json.as_deref(), decision)
        .await?;
    crate::ocr_budget::record_admission_result(infra, &result);
    Ok(result)
}

pub(super) async fn ensure_screen_capture_segment_for_frame(
    infra: &::app_infra::AppInfra,
    source_session_id: &str,
    file_path: &str,
    captured_at: &str,
) -> ::app_infra::Result<Option<i64>> {
    let Some(paths) =
        ::app_infra::HiddenSegmentWorkspacePaths::from_frame_artifact_path(Path::new(file_path))
    else {
        return Ok(None);
    };
    let Some(segment_index) = segment_index_from_visible_path(&paths.visible_segment_path) else {
        return Ok(None);
    };
    let sidecar_path =
        capture_screen::screen_segment_frame_index_path(Path::new(&paths.visible_segment_path))
            .to_string_lossy()
            .to_string();
    let Some(segment) = infra
        .capture_retention()
        .upsert_screen_segment_for_source_session(
            source_session_id,
            segment_index,
            paths.visible_segment_path,
            paths.workspace_dir,
            paths.frames_dir,
            sidecar_path,
            captured_at.to_string(),
        )
        .await?
    else {
        return Ok(None);
    };
    Ok(Some(segment.id))
}

pub(super) fn segment_index_from_visible_path(path: &str) -> Option<i64> {
    let stem = Path::new(path).file_stem()?.to_str()?;
    let (_, suffix) = stem.rsplit_once("-segment-")?;
    suffix.parse::<i64>().ok()
}

pub fn captured_at_from_unix_ms(unix_ms: u64) -> String {
    time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(unix_ms) * 1_000_000)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

pub fn system_audio_speech_admission_for_current_settings(
    core: &AppCore,
) -> ::app_infra::SystemAudioSpeechActivityAdmission {
    let settings = match core
        .state::<crate::recording_settings::RecordingSettingsState>()
        .lock()
    {
        Ok(settings) => settings.settings.clone(),
        Err(_) => {
            crate::debug_log::log_error(
                "failed to read recording settings for system-audio speech admission",
            );
            return ::app_infra::SystemAudioSpeechActivityAdmission::disabled();
        }
    };

    if !settings.transcription.enabled || !settings.transcription.system_audio_enabled {
        return ::app_infra::SystemAudioSpeechActivityAdmission::disabled();
    }
    if settings.audio_speech_detection.detector == AudioSpeechDetector::Off {
        return ::app_infra::SystemAudioSpeechActivityAdmission::unavailable();
    }

    let app_data_dir = core.paths().data.clone();

    let transcription_admission =
        crate::audio_transcription_backfill::audio_transcription_admission_for_settings(
            &app_data_dir,
            &AudioTranscriptionSettings {
                microphone_enabled: true,
                ..settings.transcription.clone()
            },
            Some(&settings.speaker_analysis),
        );
    if !transcription_admission.enabled || !transcription_admission.provider_available {
        return ::app_infra::SystemAudioSpeechActivityAdmission::unavailable();
    }

    let Some(transcription_payload) = transcription_admission.payload_json else {
        return ::app_infra::SystemAudioSpeechActivityAdmission::unavailable();
    };
    let speaker_admission =
        crate::audio_transcription_backfill::speaker_analysis_admission_for_current_settings(core);
    let payload = ::app_infra::SystemAudioSpeechActivityJobPayload {
        detector: settings.audio_speech_detection.detector,
        transcription_payload,
        speaker_analysis_payload: speaker_admission.payload_json,
    };

    match serde_json::to_string(&payload) {
        Ok(payload_json) => {
            ::app_infra::SystemAudioSpeechActivityAdmission::available(payload_json)
        }
        Err(error) => {
            crate::debug_log::log_error(format!(
                "failed to serialize system-audio speech activity payload: {error}"
            ));
            ::app_infra::SystemAudioSpeechActivityAdmission::unavailable()
        }
    }
}

pub(super) fn processing_subject(
    subject_type: String,
    subject_id: i64,
) -> ::app_infra::ProcessingSubject {
    ::app_infra::ProcessingSubject::new(subject_type, subject_id)
}

pub(super) async fn debug_insert_frame_and_enqueue_processing_job_inner(
    infra: &::app_infra::AppInfra,
    request: DebugInsertFrameAndEnqueueProcessingJobRequest,
    settings: Option<&crate::recording_settings::RecordingSettingsState>,
) -> ::app_infra::Result<FrameProcessingJobDto> {
    let (frame, processor, payload_json) = request.into_parts();
    let payload_json = if processor == ::app_infra::OCR_PROCESSOR {
        if settings.is_some_and(|settings| !ocr_enabled_for_settings(settings)) {
            return Err(::app_infra::AppInfraError::OcrEngine(
                "OCR is disabled".to_string(),
            ));
        }
        settings
            .map(|settings| ocr_payload_json_from_settings(settings, payload_json.as_deref()))
            .transpose()
            .map_err(::app_infra::AppInfraError::OcrEngine)?
            .flatten()
    } else {
        payload_json
    };

    infra
        .debug_insert_frame_and_enqueue_processing_job(&frame, &processor, payload_json.as_deref())
        .await
        .map(FrameProcessingJobDto::from)
}

pub(super) async fn reprocess_captured_frame_ocr_inner(
    infra: &::app_infra::AppInfra,
    request: ReprocessCapturedFrameOcrRequest,
    settings: &crate::recording_settings::RecordingSettingsState,
) -> ::app_infra::Result<CapturedFrameReprocessingResultDto> {
    if !ocr_enabled_for_settings(settings) {
        return Err(::app_infra::AppInfraError::OcrEngine(
            "OCR is disabled".to_string(),
        ));
    }

    let payload_json = ocr_payload_json_from_settings(settings, request.payload_json.as_deref())
        .map_err(::app_infra::AppInfraError::OcrEngine)?;

    infra
        .reprocess_captured_frame_ocr(request.frame_id, payload_json.as_deref())
        .await
        .map(CapturedFrameReprocessingResultDto::from)
}

pub(super) async fn reprocess_audio_segment_transcription_inner(
    infra: &::app_infra::AppInfra,
    request: ReprocessAudioSegmentTranscriptionRequest,
    core: &AppCore,
) -> ::app_infra::Result<AudioSegmentTranscriptionReprocessingResultDto> {
    let admission =
        crate::audio_transcription_backfill::audio_transcription_admission_for_current_settings(
            core,
        );

    infra
        .reprocess_audio_segment_transcription(request.audio_segment_id, &admission)
        .await
        .map(AudioSegmentTranscriptionReprocessingResultDto::from)
}

pub(super) async fn reprocess_audio_segment_speaker_analysis_inner(
    infra: &::app_infra::AppInfra,
    request: ReprocessAudioSegmentTranscriptionRequest,
    core: &AppCore,
) -> ::app_infra::Result<AudioSegmentSpeakerAnalysisReprocessingResultDto> {
    let admission =
        crate::audio_transcription_backfill::speaker_analysis_admission_for_current_settings(core);

    infra
        .reprocess_audio_segment_speaker_analysis(request.audio_segment_id, &admission)
        .await
        .map(AudioSegmentSpeakerAnalysisReprocessingResultDto::from)
}

pub(super) async fn reprocess_system_audio_speech_activity_inner(
    infra: &::app_infra::AppInfra,
    request: ReprocessAudioSegmentTranscriptionRequest,
    core: &AppCore,
) -> ::app_infra::Result<SystemAudioSpeechActivityReprocessingResultDto> {
    let admission = system_audio_speech_admission_for_current_settings(core);

    infra
        .reprocess_system_audio_speech_activity(request.audio_segment_id, &admission)
        .await
        .map(SystemAudioSpeechActivityReprocessingResultDto::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_infra::test_helpers::*;

    #[test]
    fn persist_screen_frame_artifact_maps_metadata_for_dedupable_ocr() {
        run_async_test(async {
            let dir = TestDir::new("screen-frame-artifact");
            let infra = ::app_infra::AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let settings = crate::recording_settings::RecordingSettingsState::default();

            let persisted = persist_screen_frame_artifact(
                &infra,
                &settings,
                None,
                "session-artifact",
                ScreenFrameArtifact {
                    file_path: "/tmp/frame-artifact.png".to_string(),
                    captured_at_unix_ms: 1_744_539_600_123,
                    width: Some(1440),
                    height: Some(900),
                    captured_frame_equivalence:
                        capture_screen::CapturedFrameEquivalenceOutcome::Ready(
                            capture_screen::CapturedFrameEquivalence {
                                hint: "feedbeefhint0001".to_string(),
                                proof: b"feedbeef-proof".to_vec(),
                                version: capture_screen::CAPTURED_FRAME_EQUIVALENCE_VERSION,
                            },
                        ),
                },
            )
            .await
            .expect("artifact should persist");

            assert_eq!(persisted.frame.session_id, "session-artifact");
            assert_eq!(persisted.frame.file_path, "/tmp/frame-artifact.png");
            assert_eq!(persisted.frame.width, Some(1440));
            assert_eq!(persisted.frame.height, Some(900));
            assert_eq!(
                persisted.frame.equivalence.hint.as_deref(),
                Some("feedbeefhint0001")
            );
            assert_eq!(
                persisted.job.as_ref().map(|job| job.processor.as_str()),
                Some("ocr")
            );

            let batches = infra
                .list_frame_batches(Some("session-artifact"))
                .await
                .expect("frame batches should list");
            assert_eq!(batches.len(), 1);
            assert_eq!(batches[0].status, ::app_infra::FrameBatchStatus::Open);
            assert_eq!(batches[0].frame_count, 1);
        });
    }

    #[test]
    fn persist_screen_frame_artifact_preserves_quarantined_equivalence() {
        run_async_test(async {
            let dir = TestDir::new("screen-frame-artifact-quarantined");
            let infra = ::app_infra::AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let settings = crate::recording_settings::RecordingSettingsState::default();

            let persisted = persist_screen_frame_artifact(
                &infra,
                &settings,
                None,
                "session-artifact-quarantined",
                ScreenFrameArtifact {
                    file_path: "/tmp/frame-artifact-quarantined.png".to_string(),
                    captured_at_unix_ms: 1_744_539_600_123,
                    width: Some(1440),
                    height: Some(900),
                    captured_frame_equivalence:
                        capture_screen::CapturedFrameEquivalenceOutcome::quarantined(
                            "failed to derive captured frame equivalence from screen sample",
                        ),
                },
            )
            .await
            .expect("artifact should persist even when equivalence is quarantined");

            assert_eq!(
                persisted.frame.equivalence.status,
                Some(::app_infra::FrameEquivalenceStatus::Quarantined)
            );
            assert_eq!(
                persisted.frame.equivalence.error.as_deref(),
                Some("failed to derive captured frame equivalence from screen sample")
            );
            assert!(
                persisted.job.is_some(),
                "quarantined frames must still enqueue OCR"
            );
        });
    }
}
