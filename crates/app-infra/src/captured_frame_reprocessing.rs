use serde::{Deserialize, Serialize};

use crate::{
    processing::{
        ProcessingJob, ProcessingJobStatus, ProcessingStore, ProcessingSubject, OCR_PROCESSOR,
    },
    AppInfraError, Result,
};

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

#[derive(Clone)]
pub struct CapturedFrameReprocessing {
    processing: ProcessingStore,
}

impl CapturedFrameReprocessing {
    pub(crate) fn new(processing: ProcessingStore) -> Self {
        Self { processing }
    }

    pub async fn reprocess_captured_frame_ocr(
        &self,
        frame_id: i64,
        payload_json: Option<&str>,
    ) -> Result<CapturedFrameReprocessingResult> {
        let mut transaction = self.processing.begin_transaction().await?;
        let subject = ProcessingSubject::frame(frame_id);

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
            Some(job) if job.status == ProcessingJobStatus::Queued => {
                CapturedFrameReprocessingResult {
                    outcome: CapturedFrameReprocessingOutcome::Ignored,
                    job,
                }
            }
            Some(job) if job.status == ProcessingJobStatus::Running => {
                return Err(AppInfraError::ProcessingJobInvalidTransition {
                    job_id: job.id,
                    from: job.status.as_str().to_string(),
                    to: ProcessingJobStatus::Queued.as_str().to_string(),
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
}
