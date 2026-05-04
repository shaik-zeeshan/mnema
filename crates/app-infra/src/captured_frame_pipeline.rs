use serde::{Deserialize, Serialize};
use sqlx::{Sqlite, Transaction};

use crate::{
    captured_frame_equivalence::{
        CapturedFrameEquivalenceResolver, CapturedFrameEquivalenceScope,
    },
    frame_batch_store::{FrameBatch, FrameBatchStore},
    processing::{
        Frame, FrameProcessingJob, NewFrame, ProcessingJob, ProcessingStore, OCR_PROCESSOR,
    },
    Result,
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

    pub(crate) async fn capture_frame_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        frame: &NewFrame,
        ocr_payload_json: Option<&str>,
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
        let equivalence_scope = CapturedFrameEquivalenceScope::from_frame(&stored_frame);
        let job = if self
            .equivalence
            .find_nearest_earlier_equivalent_frame_in_transaction(
                transaction,
                &stored_frame,
                &equivalence_scope,
            )
            .await?
            .is_none()
        {
            Some(
                self.processing
                    .enqueue_processor_job_for_frame_in_transaction(
                        transaction,
                        stored_frame.id,
                        OCR_PROCESSOR,
                        ocr_payload_json,
                    )
                    .await?,
            )
        } else {
            None
        };
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

    pub(crate) async fn debug_insert_frame_and_enqueue_processor_job(
        &self,
        frame: &NewFrame,
        processor: &str,
        payload_json: Option<&str>,
    ) -> Result<FrameProcessingJob> {
        let mut transaction = self.processing.begin_transaction().await?;

        let batch = self
            .frame_batches
            .upsert_open_batch_for_frame_in_transaction(
                &mut transaction,
                &frame.session_id,
                &frame.captured_at,
            )
            .await?;
        let stored_frame = self
            .processing
            .insert_frame_in_transaction(&mut transaction, frame)
            .await?;
        self.frame_batches
            .attach_frame_to_batch_in_transaction(
                &mut transaction,
                stored_frame.id,
                batch.id,
                &stored_frame.captured_at,
            )
            .await?;
        let job = self
            .processing
            .enqueue_processor_job_for_frame_in_transaction(
                &mut transaction,
                stored_frame.id,
                processor,
                payload_json,
            )
            .await?;
        self.frame_batches
            .close_and_schedule_completed_batches_for_frame_in_transaction(
                &mut transaction,
                &frame.session_id,
                batch.id,
            )
            .await?;

        transaction.commit().await?;

        Ok(FrameProcessingJob {
            frame: stored_frame,
            job,
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
