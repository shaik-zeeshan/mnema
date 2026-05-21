use audio_transcription::TranscriptionMetadata;
use ocr::OcrStructuredPayload;
use secret_redaction::{redact_searchable_text, RedactionContext, SecretCategory};
use sqlx::{Sqlite, Transaction};

use crate::{AppInfraError, AudioSegmentSourceKind, Result};

use super::{
    ProcessingJob, ProcessingResult, ProcessingResultDraft, AUDIO_SEGMENT_SUBJECT_TYPE,
    AUDIO_TRANSCRIPTION_PROCESSOR, FRAME_SUBJECT_TYPE, OCR_PROCESSOR,
};

pub(crate) struct SecretRedactionPipeline;

#[derive(Debug, Clone)]
pub(crate) struct ProcessingResultPersistencePlan {
    draft: ProcessingResultDraft,
    secret_redactions: Vec<SecretRedactionPersistenceDraft>,
    #[cfg(test)]
    redaction_context: Option<RedactionContext>,
}

#[derive(Debug, Clone)]
struct SecretRedactionPersistenceDraft {
    anchor: SecretRedactionAnchor,
    category: SecretCategory,
    redacted_start: i64,
    redacted_end: i64,
    detector_version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SecretRedactionAnchor {
    Frame { frame_id: i64 },
    Audio { audio_segment_id: i64 },
}

#[derive(Debug, Clone, Copy)]
struct ProcessingResultRedactionPolicy {
    context: RedactionContext,
    anchor: SecretRedactionAnchor,
    structured_payload: StructuredPayloadRedaction,
}

#[derive(Debug, Clone, Copy)]
enum StructuredPayloadRedaction {
    Ocr,
    Transcription,
}

impl SecretRedactionPipeline {
    pub(crate) fn needs_audio_segment_source(job: &ProcessingJob) -> bool {
        job.subject_type == AUDIO_SEGMENT_SUBJECT_TYPE
            && job.processor == AUDIO_TRANSCRIPTION_PROCESSOR
    }

    pub(crate) fn plan_processing_result_persistence(
        job: &ProcessingJob,
        audio_segment_source_kind: Option<&AudioSegmentSourceKind>,
        draft: &ProcessingResultDraft,
    ) -> Result<ProcessingResultPersistencePlan> {
        let Some(policy) = redaction_policy_for_job(job, audio_segment_source_kind)? else {
            return Ok(ProcessingResultPersistencePlan {
                draft: draft.clone(),
                secret_redactions: Vec::new(),
                #[cfg(test)]
                redaction_context: None,
            });
        };

        let redaction = draft
            .result_text
            .as_deref()
            .map(|text| redact_searchable_text(text, policy.context));
        let result_text = redaction
            .as_ref()
            .map(|result| result.redacted_text.clone())
            .or_else(|| draft.result_text.clone());

        let structured_payload_json = match draft.structured_payload_json.as_deref() {
            Some(payload_json) => Some(redact_structured_payload_or_keep_if_no_redactions(
                payload_json,
                redaction.as_ref(),
                policy.structured_payload,
                policy.context,
            )?),
            None => None,
        };

        let secret_redactions = redaction
            .as_ref()
            .map(|redaction| {
                redaction
                    .spans
                    .iter()
                    .map(|span| SecretRedactionPersistenceDraft {
                        anchor: policy.anchor,
                        category: span.category,
                        redacted_start: span_position_to_i64(span.start),
                        redacted_end: span_position_to_i64(span.end),
                        detector_version: redaction.detector_version.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(ProcessingResultPersistencePlan {
            draft: ProcessingResultDraft {
                result_text,
                structured_payload_json,
                processor_version: draft.processor_version.clone(),
            },
            secret_redactions,
            #[cfg(test)]
            redaction_context: Some(policy.context),
        })
    }
}

impl ProcessingResultPersistencePlan {
    pub(crate) fn draft(&self) -> &ProcessingResultDraft {
        &self.draft
    }

    pub(crate) async fn persist_secret_redactions_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        stored_result: &ProcessingResult,
    ) -> Result<()> {
        for redaction in &self.secret_redactions {
            sqlx::query(
                "INSERT INTO secret_redactions (\
                    anchor_type, frame_id, audio_segment_id, processing_result_id, category, \
                    redacted_start, redacted_end, detector_version\
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )
            .bind(redaction.anchor.anchor_type())
            .bind(redaction.anchor.frame_id())
            .bind(redaction.anchor.audio_segment_id())
            .bind(stored_result.id)
            .bind(redaction.category.as_storage_str())
            .bind(redaction.redacted_start)
            .bind(redaction.redacted_end)
            .bind(&redaction.detector_version)
            .execute(&mut **transaction)
            .await?;
        }

        Ok(())
    }
}

impl SecretRedactionAnchor {
    fn anchor_type(self) -> &'static str {
        match self {
            Self::Frame { .. } => "frame",
            Self::Audio { .. } => "audio",
        }
    }

    fn frame_id(self) -> Option<i64> {
        match self {
            Self::Frame { frame_id } => Some(frame_id),
            Self::Audio { .. } => None,
        }
    }

    fn audio_segment_id(self) -> Option<i64> {
        match self {
            Self::Frame { .. } => None,
            Self::Audio { audio_segment_id } => Some(audio_segment_id),
        }
    }
}

fn redaction_policy_for_job(
    job: &ProcessingJob,
    audio_segment_source_kind: Option<&AudioSegmentSourceKind>,
) -> Result<Option<ProcessingResultRedactionPolicy>> {
    match (job.subject_type.as_str(), job.processor.as_str()) {
        (FRAME_SUBJECT_TYPE, OCR_PROCESSOR) => Ok(Some(ProcessingResultRedactionPolicy {
            context: RedactionContext::Ocr,
            anchor: SecretRedactionAnchor::Frame {
                frame_id: job.subject_id,
            },
            structured_payload: StructuredPayloadRedaction::Ocr,
        })),
        (AUDIO_SEGMENT_SUBJECT_TYPE, AUDIO_TRANSCRIPTION_PROCESSOR) => {
            let source_kind = audio_segment_source_kind
                .ok_or(AppInfraError::AudioSegmentNotFound(job.subject_id))?;
            let context = match source_kind {
                AudioSegmentSourceKind::Microphone => RedactionContext::MicrophoneTranscript,
                AudioSegmentSourceKind::SystemAudio => RedactionContext::SystemAudioTranscript,
            };
            Ok(Some(ProcessingResultRedactionPolicy {
                context,
                anchor: SecretRedactionAnchor::Audio {
                    audio_segment_id: job.subject_id,
                },
                structured_payload: StructuredPayloadRedaction::Transcription,
            }))
        }
        _ => Ok(None),
    }
}

fn redact_structured_payload_or_keep_if_no_redactions(
    payload_json: &str,
    result_text_redaction: Option<&secret_redaction::RedactionResult>,
    payload_redaction: StructuredPayloadRedaction,
    context: RedactionContext,
) -> Result<String> {
    match redact_structured_payload(payload_json, payload_redaction, context) {
        Ok(redacted) => Ok(redacted),
        Err(error) if result_text_redaction.is_some_and(|redaction| redaction.spans.is_empty()) => {
            let _ = error;
            Ok(payload_json.to_string())
        }
        Err(error) => Err(error),
    }
}

fn redact_structured_payload(
    payload_json: &str,
    payload_redaction: StructuredPayloadRedaction,
    context: RedactionContext,
) -> Result<String> {
    match payload_redaction {
        StructuredPayloadRedaction::Ocr => redact_ocr_structured_payload(payload_json, context),
        StructuredPayloadRedaction::Transcription => {
            redact_transcription_structured_payload(payload_json, context)
        }
    }
}

fn redact_ocr_structured_payload(payload_json: &str, context: RedactionContext) -> Result<String> {
    let mut payload: OcrStructuredPayload = serde_json::from_str(payload_json)?;
    for observation in &mut payload.observations {
        observation.text = redact_searchable_text(&observation.text, context).redacted_text;
    }
    Ok(serde_json::to_string(&payload)?)
}

fn redact_transcription_structured_payload(
    payload_json: &str,
    context: RedactionContext,
) -> Result<String> {
    let mut metadata: TranscriptionMetadata = serde_json::from_str(payload_json)?;
    for segment in &mut metadata.segments {
        segment.text = redact_searchable_text(&segment.text, context).redacted_text;
    }
    for word in &mut metadata.words {
        word.text = redact_searchable_text(&word.text, context).redacted_text;
    }
    Ok(serde_json::to_string(&metadata)?)
}

fn span_position_to_i64(position: usize) -> i64 {
    i64::try_from(position).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use audio_transcription::{TranscriptionSegment, TranscriptionWord};
    use ocr::{OcrBoundingBox, OcrObservation};

    use super::*;
    use crate::ProcessingJobStatus;

    fn job(subject_type: &str, subject_id: i64, processor: &str) -> ProcessingJob {
        ProcessingJob {
            id: 1,
            subject_type: subject_type.to_string(),
            subject_id,
            processor: processor.to_string(),
            status: ProcessingJobStatus::Running,
            attempt_count: 1,
            payload_json: None,
            last_error: None,
            created_at: "2026-05-21T00:00:00Z".to_string(),
            queued_at: "2026-05-21T00:00:00Z".to_string(),
            updated_at: "2026-05-21T00:00:00Z".to_string(),
            started_at: Some("2026-05-21T00:00:00Z".to_string()),
            finished_at: None,
        }
    }

    #[test]
    fn ocr_plan_redacts_result_text_structured_payload_and_metadata() {
        let secret = "sk-abcdefghijklmnopqrstuvwxyz123456";
        let payload = OcrStructuredPayload::new(
            "test",
            None,
            vec![OcrObservation::new(
                format!("OPENAI_API_KEY={secret}"),
                0.98,
                OcrBoundingBox::new(0.0, 0.0, 1.0, 1.0),
            )],
        );
        let plan = SecretRedactionPipeline::plan_processing_result_persistence(
            &job(FRAME_SUBJECT_TYPE, 42, OCR_PROCESSOR),
            None,
            &ProcessingResultDraft::new()
                .with_result_text(format!("OPENAI_API_KEY={secret} nearby context"))
                .with_structured_payload_json(serde_json::to_string(&payload).unwrap()),
        )
        .expect("ocr redaction plan should build");

        let result_text = plan.draft().result_text.as_deref().unwrap();
        assert!(result_text.contains("[REDACTED_SECRET: API_KEY]"));
        assert!(!result_text.contains(secret));

        let payload: OcrStructuredPayload =
            serde_json::from_str(plan.draft().structured_payload_json.as_deref().unwrap())
                .expect("redacted payload should remain valid");
        assert!(payload.observations[0]
            .text
            .contains("[REDACTED_SECRET: API_KEY]"));
        assert!(!payload.observations[0].text.contains(secret));

        assert_eq!(plan.redaction_context, Some(RedactionContext::Ocr));
        assert_eq!(plan.secret_redactions.len(), 1);
        assert_eq!(
            plan.secret_redactions[0].anchor,
            SecretRedactionAnchor::Frame { frame_id: 42 }
        );
        assert_eq!(plan.secret_redactions[0].category, SecretCategory::ApiKey);
    }

    #[test]
    fn audio_plan_uses_source_specific_context_and_audio_anchor() {
        let mic_plan = SecretRedactionPipeline::plan_processing_result_persistence(
            &job(AUDIO_SEGMENT_SUBJECT_TYPE, 7, AUDIO_TRANSCRIPTION_PROCESSOR),
            Some(&AudioSegmentSourceKind::Microphone),
            &ProcessingResultDraft::new(),
        )
        .expect("microphone plan should build");
        assert_eq!(
            mic_plan.redaction_context,
            Some(RedactionContext::MicrophoneTranscript)
        );

        let system_plan = SecretRedactionPipeline::plan_processing_result_persistence(
            &job(AUDIO_SEGMENT_SUBJECT_TYPE, 8, AUDIO_TRANSCRIPTION_PROCESSOR),
            Some(&AudioSegmentSourceKind::SystemAudio),
            &ProcessingResultDraft::new()
                .with_result_text("OPENAI_API_KEY=sk-abcdefghijklmnopqrstuvwxyz123456"),
        )
        .expect("system-audio plan should build");

        assert_eq!(
            system_plan.redaction_context,
            Some(RedactionContext::SystemAudioTranscript)
        );
        assert_eq!(system_plan.secret_redactions.len(), 1);
        assert_eq!(
            system_plan.secret_redactions[0].anchor,
            SecretRedactionAnchor::Audio {
                audio_segment_id: 8
            }
        );
    }

    #[test]
    fn transcription_plan_redacts_segments_and_words() {
        let secret = "sk-abcdefghijklmnopqrstuvwxyz123456";
        let metadata = TranscriptionMetadata {
            provider: "test".to_string(),
            model_id: None,
            language: "en".to_string(),
            segments: vec![TranscriptionSegment {
                start_ms: 0,
                end_ms: 1_000,
                text: format!("OPENAI_API_KEY={secret}"),
                confidence: None,
            }],
            words: vec![TranscriptionWord {
                start_ms: 0,
                end_ms: 500,
                text: format!("OPENAI_API_KEY={secret}"),
                confidence: None,
            }],
            provenance: Default::default(),
        };

        let plan = SecretRedactionPipeline::plan_processing_result_persistence(
            &job(AUDIO_SEGMENT_SUBJECT_TYPE, 9, AUDIO_TRANSCRIPTION_PROCESSOR),
            Some(&AudioSegmentSourceKind::SystemAudio),
            &ProcessingResultDraft::new()
                .with_result_text(format!("OPENAI_API_KEY={secret}"))
                .with_structured_payload_json(serde_json::to_string(&metadata).unwrap()),
        )
        .expect("transcription redaction plan should build");

        let metadata: TranscriptionMetadata =
            serde_json::from_str(plan.draft().structured_payload_json.as_deref().unwrap())
                .expect("redacted metadata should remain valid");
        assert!(!metadata.segments[0].text.contains(secret));
        assert!(!metadata.words[0].text.contains(secret));
        assert!(metadata.segments[0]
            .text
            .contains("[REDACTED_SECRET: API_KEY]"));
        assert!(metadata.words[0]
            .text
            .contains("[REDACTED_SECRET: API_KEY]"));
    }

    #[test]
    fn malformed_structured_payload_fails_closed_when_text_had_redactions() {
        let result = SecretRedactionPipeline::plan_processing_result_persistence(
            &job(FRAME_SUBJECT_TYPE, 42, OCR_PROCESSOR),
            None,
            &ProcessingResultDraft::new()
                .with_result_text("OPENAI_API_KEY=sk-abcdefghijklmnopqrstuvwxyz123456")
                .with_structured_payload_json("{\"blocks\":[]}"),
        );

        assert!(result.is_err());
    }

    #[test]
    fn unknown_structured_payload_is_kept_when_result_text_has_no_redactions() {
        let plan = SecretRedactionPipeline::plan_processing_result_persistence(
            &job(FRAME_SUBJECT_TYPE, 42, OCR_PROCESSOR),
            None,
            &ProcessingResultDraft::new()
                .with_result_text("recognized text")
                .with_structured_payload_json("{\"blocks\":[]}"),
        )
        .expect("non-secret result should keep unknown payload shape");

        assert_eq!(
            plan.draft().structured_payload_json.as_deref(),
            Some("{\"blocks\":[]}")
        );
        assert!(plan.secret_redactions.is_empty());
    }
}
