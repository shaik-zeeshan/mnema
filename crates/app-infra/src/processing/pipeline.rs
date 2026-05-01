use serde::{Deserialize, Serialize};
use sqlx::{Sqlite, Transaction};

use crate::Result;

use super::{
    Frame, FrameOcrEnqueueResult, FrameProcessingJob, NewFrame, ProcessingStore, OCR_PROCESSOR,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FramePipelineRequest {
    pub frame: NewFrame,
    pub processor: String,
    pub payload_json: Option<String>,
}

impl FramePipelineRequest {
    pub fn new(frame: NewFrame, processor: impl Into<String>) -> Self {
        Self {
            frame,
            processor: processor.into(),
            payload_json: None,
        }
    }

    pub fn for_ocr(frame: NewFrame) -> Self {
        Self::new(frame, OCR_PROCESSOR)
    }

    pub fn with_payload_json(mut self, payload_json: impl Into<String>) -> Self {
        self.payload_json = Some(payload_json.into());
        self
    }

    pub fn frame(&self) -> &NewFrame {
        &self.frame
    }

    pub fn processor(&self) -> &str {
        &self.processor
    }

    pub fn payload_json(&self) -> Option<&str> {
        self.payload_json.as_deref()
    }
}

#[derive(Clone)]
pub struct FramePipeline {
    processing: ProcessingStore,
}

impl FramePipeline {
    pub(crate) fn new(processing: ProcessingStore) -> Self {
        Self { processing }
    }

    pub async fn enqueue(&self, request: &FramePipelineRequest) -> Result<FrameProcessingJob> {
        self.processing
            .insert_frame_and_enqueue_processor_job(
                request.frame(),
                request.processor(),
                request.payload_json(),
            )
            .await
    }

    pub async fn insert_frame_and_maybe_enqueue_ocr_job(
        &self,
        frame: &NewFrame,
        payload_json: Option<&str>,
    ) -> Result<FrameOcrEnqueueResult> {
        let mut transaction = self.processing.begin_transaction().await?;

        let result = self
            .insert_frame_and_maybe_enqueue_ocr_job_in_transaction(
                &mut transaction,
                frame,
                payload_json,
            )
            .await?;

        transaction.commit().await?;

        Ok(result)
    }

    pub(crate) async fn insert_frame_and_maybe_enqueue_ocr_job_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        frame: &NewFrame,
        payload_json: Option<&str>,
    ) -> Result<FrameOcrEnqueueResult> {
        let stored_frame = self
            .processing
            .insert_frame_in_transaction(transaction, frame)
            .await?;

        let stored_job = if self
            .should_enqueue_ocr_for_frame(transaction, &stored_frame)
            .await?
        {
            Some(
                self.processing
                    .enqueue_processor_job_for_frame_in_transaction(
                        transaction,
                        stored_frame.id,
                        OCR_PROCESSOR,
                        payload_json,
                    )
                    .await?,
            )
        } else {
            None
        };

        Ok(FrameOcrEnqueueResult {
            frame: stored_frame,
            job: stored_job,
        })
    }

    async fn should_enqueue_ocr_for_frame(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        frame: &Frame,
    ) -> Result<bool> {
        let Some(content_fingerprint) = frame.content_fingerprint.as_deref() else {
            return Ok(true);
        };

        let has_previous = self
            .processing
            .has_previous_frame_with_content_fingerprint_in_transaction(
                transaction,
                &frame.session_id,
                frame.id,
                content_fingerprint,
            )
            .await?;

        Ok(!has_previous)
    }
}
