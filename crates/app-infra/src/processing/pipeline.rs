use serde::{Deserialize, Serialize};

use crate::Result;

use super::{FrameProcessingJob, NewFrame, ProcessingStore, OCR_PROCESSOR};

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
}
