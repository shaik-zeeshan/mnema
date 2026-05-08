use serde::{Deserialize, Serialize};
use sqlx::{Sqlite, Transaction};

use crate::{
    captured_frame_equivalence::{CapturedFrameEquivalenceResolver, CapturedFrameEquivalenceScope},
    frame_batch_store::{FrameBatch, FrameBatchStore},
    processing::{
        Frame, FrameProcessingJob, NewFrame, ProcessingJob, ProcessingStore, OCR_PROCESSOR,
    },
    AppInfraError, Result,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CapturedFramePipelineResult {
    pub frame: Frame,
    pub active_batch: FrameBatch,
    pub job: Option<ProcessingJob>,
    pub closed_batches: Vec<ClosedFrameBatchSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ClosedFrameBatchSummary {
    pub id: i64,
    pub session_id: String,
    pub batch_key: String,
    pub batch_started_at: String,
    pub batch_ended_at: String,
    pub frame_count: i64,
    pub first_frame_at: Option<String>,
    pub last_frame_at: Option<String>,
    pub closed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapturedFrameReprocessingOutcome {
    Created,
    Ignored,
    Requeued,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CapturedFrameReprocessingResult {
    pub outcome: CapturedFrameReprocessingOutcome,
    pub job: ProcessingJob,
}

impl From<FrameBatch> for ClosedFrameBatchSummary {
    fn from(batch: FrameBatch) -> Self {
        Self {
            id: batch.id,
            session_id: batch.session_id,
            batch_key: batch.batch_key,
            batch_started_at: batch.batch_started_at,
            batch_ended_at: batch.batch_ended_at,
            frame_count: batch.frame_count,
            first_frame_at: batch.first_frame_at,
            last_frame_at: batch.last_frame_at,
            closed_at: batch.closed_at,
        }
    }
}

#[derive(Clone)]
pub struct CapturedFramePipeline {
    processing: ProcessingStore,
    frame_batches: FrameBatchStore,
    equivalence: CapturedFrameEquivalenceResolver,
}

enum PipelineJobMode<'a> {
    AdmitOcrJob {
        ocr_payload_json: Option<&'a str>,
    },
    SkipOcrJob,
    EnqueueProcessorJob {
        processor: &'a str,
        payload_json: Option<&'a str>,
    },
}

impl CapturedFramePipeline {
    pub(crate) fn new(processing: ProcessingStore, frame_batches: FrameBatchStore) -> Self {
        let equivalence = CapturedFrameEquivalenceResolver::new(processing.clone());
        Self {
            processing,
            frame_batches,
            equivalence,
        }
    }

    pub async fn capture_frame(
        &self,
        frame: &NewFrame,
        ocr_payload_json: Option<&str>,
    ) -> Result<CapturedFramePipelineResult> {
        let mut transaction = self.processing.begin_transaction().await?;

        let result = self
            .capture_frame_in_transaction(&mut transaction, frame, ocr_payload_json)
            .await?;

        transaction.commit().await?;

        Ok(result)
    }

    pub async fn capture_frame_without_ocr(
        &self,
        frame: &NewFrame,
    ) -> Result<CapturedFramePipelineResult> {
        let mut transaction = self.processing.begin_transaction().await?;

        let result = self
            .capture_frame_with_mode_in_transaction(
                &mut transaction,
                frame,
                PipelineJobMode::SkipOcrJob,
            )
            .await?;

        transaction.commit().await?;

        Ok(result)
    }

    pub async fn reprocess_captured_frame_ocr(
        &self,
        frame_id: i64,
        payload_json: Option<&str>,
    ) -> Result<CapturedFrameReprocessingResult> {
        let mut transaction = self.processing.begin_transaction().await?;
        let subject = crate::processing::ProcessingSubject::frame(frame_id);

        self.processing
            .get_frame_in_transaction(&mut transaction, frame_id)
            .await?
            .ok_or(AppInfraError::FrameNotFound(frame_id))?;

        let existing_job = self
            .processing
            .get_latest_processing_job_for_subject_and_processor_in_transaction(
                &mut transaction,
                &subject,
                OCR_PROCESSOR,
            )
            .await?;

        let result = match existing_job {
            None => {
                let job = self
                    .processing
                    .enqueue_processor_job_for_frame_in_transaction(
                        &mut transaction,
                        frame_id,
                        OCR_PROCESSOR,
                        payload_json,
                    )
                    .await?;
                CapturedFrameReprocessingResult {
                    outcome: CapturedFrameReprocessingOutcome::Created,
                    job,
                }
            }
            Some(job) if job.status == crate::processing::ProcessingJobStatus::Queued => {
                CapturedFrameReprocessingResult {
                    outcome: CapturedFrameReprocessingOutcome::Ignored,
                    job,
                }
            }
            Some(job) if job.status == crate::processing::ProcessingJobStatus::Running => {
                return Err(AppInfraError::ProcessingJobInvalidTransition {
                    job_id: job.id,
                    from: job.status.as_str().to_string(),
                    to: crate::processing::ProcessingJobStatus::Queued
                        .as_str()
                        .to_string(),
                });
            }
            Some(job) => {
                let job = self
                    .processing
                    .requeue_processing_job_in_transaction(&mut transaction, job.id, payload_json)
                    .await?;
                CapturedFrameReprocessingResult {
                    outcome: CapturedFrameReprocessingOutcome::Requeued,
                    job,
                }
            }
        };

        transaction.commit().await?;

        Ok(result)
    }

    pub(crate) async fn capture_frame_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        frame: &NewFrame,
        ocr_payload_json: Option<&str>,
    ) -> Result<CapturedFramePipelineResult> {
        self.capture_frame_with_mode_in_transaction(
            transaction,
            frame,
            PipelineJobMode::AdmitOcrJob { ocr_payload_json },
        )
        .await
    }

    async fn capture_frame_with_mode_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        frame: &NewFrame,
        job_mode: PipelineJobMode<'_>,
    ) -> Result<CapturedFramePipelineResult> {
        let batch = self
            .frame_batches
            .upsert_open_batch_for_frame_in_transaction(
                transaction,
                &frame.session_id,
                &frame.captured_at,
            )
            .await?;
        let stored_frame = self
            .processing
            .insert_frame_in_transaction(transaction, frame)
            .await?;
        let active_batch = self
            .frame_batches
            .attach_frame_to_batch_in_transaction(
                transaction,
                stored_frame.id,
                batch.id,
                &stored_frame.captured_at,
            )
            .await?;
        let job = self
            .admit_processing_job_in_transaction(transaction, &stored_frame, job_mode)
            .await?;
        let closed_batches = self
            .frame_batches
            .close_and_schedule_completed_batches_for_frame_in_transaction(
                transaction,
                &frame.session_id,
                active_batch.id,
            )
            .await?
            .into_iter()
            .map(ClosedFrameBatchSummary::from)
            .collect();

        Ok(CapturedFramePipelineResult {
            frame: stored_frame,
            active_batch,
            job,
            closed_batches,
        })
    }

    async fn admit_processing_job_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        frame: &Frame,
        job_mode: PipelineJobMode<'_>,
    ) -> Result<Option<ProcessingJob>> {
        match job_mode {
            PipelineJobMode::SkipOcrJob => Ok(None),
            PipelineJobMode::AdmitOcrJob { ocr_payload_json } => {
                let equivalence_scope = CapturedFrameEquivalenceScope::from_frame(frame);
                if self
                    .equivalence
                    .find_nearest_earlier_equivalent_frame_in_transaction(
                        transaction,
                        frame,
                        &equivalence_scope,
                    )
                    .await?
                    .is_some()
                {
                    return Ok(None);
                }

                Ok(Some(
                    self.processing
                        .enqueue_processor_job_for_frame_in_transaction(
                            transaction,
                            frame.id,
                            OCR_PROCESSOR,
                            ocr_payload_json,
                        )
                        .await?,
                ))
            }
            PipelineJobMode::EnqueueProcessorJob {
                processor,
                payload_json,
            } => Ok(Some(
                self.processing
                    .enqueue_processor_job_for_frame_in_transaction(
                        transaction,
                        frame.id,
                        processor,
                        payload_json,
                    )
                    .await?,
            )),
        }
    }

    pub(crate) async fn debug_insert_frame_and_enqueue_processor_job(
        &self,
        frame: &NewFrame,
        processor: &str,
        payload_json: Option<&str>,
    ) -> Result<FrameProcessingJob> {
        let mut transaction = self.processing.begin_transaction().await?;

        let result = self
            .capture_frame_with_mode_in_transaction(
                &mut transaction,
                frame,
                PipelineJobMode::EnqueueProcessorJob {
                    processor,
                    payload_json,
                },
            )
            .await?;

        transaction.commit().await?;

        Ok(FrameProcessingJob {
            frame: result.frame,
            job: result
                .job
                .expect("debug pipeline mode should always enqueue a processing job"),
        })
    }

    pub async fn find_nearest_earlier_equivalent_frame(
        &self,
        frame: &Frame,
        scope: &CapturedFrameEquivalenceScope,
    ) -> Result<Option<Frame>> {
        self.equivalence
            .find_nearest_earlier_equivalent_frame(frame, scope)
            .await
    }
}
