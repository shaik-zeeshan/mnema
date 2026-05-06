use crate::{
    frame_batch_artifact_cleanup, frame_batch_store::FrameBatchStore, AppInfraError,
    FrameBatchFinalizeResult, Result,
};

#[derive(Clone)]
pub struct FrameBatchRuntime {
    store: FrameBatchStore,
}

impl FrameBatchRuntime {
    pub fn new(store: FrameBatchStore) -> Self {
        Self { store }
    }

    pub fn store(&self) -> &FrameBatchStore {
        &self.store
    }

    pub async fn process_next_queued_job(&self) -> Result<Option<FrameBatchFinalizeResult>> {
        let Some(job) = self.store.claim_next_finalize_job().await? else {
            return Ok(None);
        };

        self.process_job(job.id).await.map(Some)
    }

    pub async fn process_job(&self, job_id: i64) -> Result<FrameBatchFinalizeResult> {
        let Some(batch_with_frames) = self.store.batch_with_frames_for_job(job_id).await? else {
            return Err(AppInfraError::JobNotFound(job_id));
        };

        let batch_id = batch_with_frames.batch.id;
        if !self.store.is_batch_ocr_terminal(batch_id).await? {
            self.store.mark_job_back_to_queued(job_id).await?;
            return Err(AppInfraError::FrameBatchOcrPending { batch_id });
        }

        if batch_with_frames.frames.is_empty() {
            let error = AppInfraError::EmptyFrameBatch { batch_id };
            self.store
                .mark_finalize_job_failed(job_id, &error.to_string())
                .await?;
            self.store
                .mark_batch_failed(batch_id, &error.to_string())
                .await?;
            return Err(error);
        }

        self.store.mark_batch_processing(batch_id).await?;

        // Clean up artifacts while the batch is still in "processing" state so
        // a crash during cleanup leaves both the batch and job retryable.
        let cleanup_errors =
            frame_batch_artifact_cleanup::cleanup_frame_artifacts(&batch_with_frames.frames);
        if !cleanup_errors.is_empty() {
            capture_runtime::debug_log!(
                "[app-infra][frame-batches] frame artifact cleanup for batch {batch_id}: {} file(s) failed to delete",
                cleanup_errors.len()
            );
            for (path, error) in &cleanup_errors {
                capture_runtime::debug_log!(
                    "[app-infra][frame-batches] failed to delete {path}: {error}"
                );
            }
        }

        let batch = self.store.mark_batch_completed(batch_id, None).await?;

        let result = FrameBatchFinalizeResult {
            batch: batch.clone(),
        };
        self.store
            .mark_finalize_job_completed(job_id, &result)
            .await?;

        Ok(result)
    }
}
