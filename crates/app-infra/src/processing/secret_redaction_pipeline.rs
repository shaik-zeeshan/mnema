use audio_transcription::TranscriptionMetadata;
use ocr::OcrStructuredPayload;
use secret_redaction::{
    plan_redactions, OcrRedactionInput, OcrRedactionObservation, PlannedRedaction,
    RedactionBoundingBox, RedactionBudget, RedactionContext, RedactionRequest, RedactionScope,
    RedactionSurfaceKind, SecretCategory, TranscriptRedactionInput, TranscriptRedactionSegment,
    TranscriptRedactionWord, DETECTOR_VERSION,
};
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
    redaction_detector_version: Option<String>,
    #[cfg(test)]
    redaction_context: Option<RedactionContext>,
}

#[derive(Debug, Clone)]
struct SecretRedactionPersistenceDraft {
    anchor: SecretRedactionAnchor,
    category: SecretCategory,
    surface_kind: RedactionSurfaceKind,
    redaction_scope: RedactionScope,
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
                redaction_detector_version: None,
                #[cfg(test)]
                redaction_context: None,
            });
        };

        let structured_payload = draft
            .structured_payload_json
            .as_deref()
            .map(|payload_json| parse_structured_payload(payload_json, policy.structured_payload))
            .transpose();
        let structured_payload = match structured_payload {
            Ok(payload) => payload,
            Err(_) => {
                return Err(AppInfraError::SecretRedactionGate(
                    "secret redaction gate rejected unparseable structured payload before safe derived-text persistence"
                        .to_string(),
                ));
            }
        };

        let request = RedactionRequest {
            context: policy.context,
            result_text: draft.result_text.clone(),
            ocr: structured_payload
                .as_ref()
                .and_then(ParsedStructuredPayload::ocr_input),
            transcript: structured_payload
                .as_ref()
                .and_then(ParsedStructuredPayload::transcript_input),
            additional_surfaces: Vec::new(),
            budget: RedactionBudget::default(),
        };
        let redaction_plan = plan_redactions(request).map_err(|_| {
            AppInfraError::SecretRedactionGate(
                "secret redaction gate failed before safe derived-text persistence".to_string(),
            )
        })?;

        let structured_payload_json = structured_payload
            .map(|payload| apply_redaction_plan_to_structured_payload(payload, &redaction_plan))
            .transpose()?;

        let secret_redactions = redaction_plan
            .redactions
            .iter()
            .map(|redaction| secret_redaction_persistence_draft(policy.anchor, redaction))
            .collect();

        Ok(ProcessingResultPersistencePlan {
            draft: ProcessingResultDraft {
                result_text: redaction_plan.result_text,
                structured_payload_json,
                processor_version: draft.processor_version.clone(),
            },
            secret_redactions,
            redaction_detector_version: Some(redaction_plan.detector_version),
            #[cfg(test)]
            redaction_context: Some(policy.context),
        })
    }
}

impl ProcessingResultPersistencePlan {
    pub(crate) fn draft(&self) -> &ProcessingResultDraft {
        &self.draft
    }

    pub(crate) fn redaction_detector_version(&self) -> Option<&str> {
        self.redaction_detector_version.as_deref()
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
                    surface_kind, redaction_scope, redacted_start, redacted_end, detector_version\
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )
            .bind(redaction.anchor.anchor_type())
            .bind(redaction.anchor.frame_id())
            .bind(redaction.anchor.audio_segment_id())
            .bind(stored_result.id)
            .bind(redaction.category.as_storage_str())
            .bind(redaction.surface_kind.as_storage_str())
            .bind(redaction.redaction_scope.as_storage_str())
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

enum ParsedStructuredPayload {
    Ocr(OcrStructuredPayload),
    Transcription(TranscriptionMetadata),
}

impl ParsedStructuredPayload {
    fn ocr_input(&self) -> Option<OcrRedactionInput> {
        match self {
            Self::Ocr(payload) => Some(OcrRedactionInput {
                observations: payload
                    .observations
                    .iter()
                    .map(|observation| OcrRedactionObservation {
                        text: observation.text.clone(),
                        confidence: observation.confidence,
                        bounding_box: RedactionBoundingBox {
                            x: observation.bounding_box.x,
                            y: observation.bounding_box.y,
                            width: observation.bounding_box.width,
                            height: observation.bounding_box.height,
                        },
                    })
                    .collect(),
            }),
            Self::Transcription(_) => None,
        }
    }

    fn transcript_input(&self) -> Option<TranscriptRedactionInput> {
        match self {
            Self::Transcription(metadata) => Some(TranscriptRedactionInput {
                segments: metadata
                    .segments
                    .iter()
                    .map(|segment| TranscriptRedactionSegment {
                        text: segment.text.clone(),
                        start_ms: segment.start_ms,
                        end_ms: segment.end_ms,
                        confidence: segment.confidence,
                    })
                    .collect(),
                words: metadata
                    .words
                    .iter()
                    .map(|word| TranscriptRedactionWord {
                        text: word.text.clone(),
                        start_ms: word.start_ms,
                        end_ms: word.end_ms,
                        confidence: word.confidence,
                    })
                    .collect(),
            }),
            Self::Ocr(_) => None,
        }
    }
}

fn parse_structured_payload(
    payload_json: &str,
    payload_redaction: StructuredPayloadRedaction,
) -> Result<ParsedStructuredPayload> {
    match payload_redaction {
        StructuredPayloadRedaction::Ocr => Ok(ParsedStructuredPayload::Ocr(serde_json::from_str(
            payload_json,
        )?)),
        StructuredPayloadRedaction::Transcription => Ok(ParsedStructuredPayload::Transcription(
            serde_json::from_str(payload_json)?,
        )),
    }
}

fn apply_redaction_plan_to_structured_payload(
    payload: ParsedStructuredPayload,
    plan: &secret_redaction::UnifiedRedactionPlan,
) -> Result<String> {
    match payload {
        ParsedStructuredPayload::Ocr(mut payload) => {
            for (index, text) in &plan.ocr_observation_text {
                if let Some(observation) = payload.observations.get_mut(*index) {
                    observation.text = text.clone();
                }
            }
            Ok(serde_json::to_string(&payload)?)
        }
        ParsedStructuredPayload::Transcription(mut metadata) => {
            for (index, text) in &plan.transcript_segment_text {
                if let Some(segment) = metadata.segments.get_mut(*index) {
                    segment.text = text.clone();
                }
            }
            for (index, text) in &plan.transcript_word_text {
                if let Some(word) = metadata.words.get_mut(*index) {
                    word.text = text.clone();
                }
            }
            Ok(serde_json::to_string(&metadata)?)
        }
    }
}

fn secret_redaction_persistence_draft(
    anchor: SecretRedactionAnchor,
    redaction: &PlannedRedaction,
) -> SecretRedactionPersistenceDraft {
    SecretRedactionPersistenceDraft {
        anchor,
        category: redaction.category,
        surface_kind: redaction.surface_kind,
        redaction_scope: redaction.redaction_scope,
        redacted_start: span_position_to_i64(redaction.redacted_start),
        redacted_end: span_position_to_i64(redaction.redacted_end),
        detector_version: DETECTOR_VERSION.to_string(),
    }
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
        assert!(plan.secret_redactions.len() >= 2);
        assert!(plan.secret_redactions.iter().all(|redaction| {
            redaction.anchor == SecretRedactionAnchor::Frame { frame_id: 42 }
                && redaction.category == SecretCategory::ApiKey
        }));
        assert!(plan.secret_redactions.iter().any(|redaction| {
            redaction.surface_kind == RedactionSurfaceKind::OcrObservation
                && redaction.redaction_scope == RedactionScope::RedactionUnit
        }));
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
    fn malformed_structured_payload_fails_closed_even_when_text_has_no_redactions() {
        let result = SecretRedactionPipeline::plan_processing_result_persistence(
            &job(FRAME_SUBJECT_TYPE, 42, OCR_PROCESSOR),
            None,
            &ProcessingResultDraft::new()
                .with_result_text("recognized text")
                .with_structured_payload_json("{\"blocks\":[]}"),
        );

        assert!(result.is_err());
    }
}
