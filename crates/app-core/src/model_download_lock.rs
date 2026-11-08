//! The `downloading` model lock a model download holds. Its release half
//! re-derives the absent-model locks from the model catalogues, so it stays in
//! the desktop crate until those move (S2 B3).

use crate::{AppCore, AppInfraState};

/// Takes the `downloading` model lock a model download holds for its whole run. A download wipes
/// the install directory before it writes, so without the lock a queued job would be claimed
/// against a model that is not readable and spend a failure attempt on it. Pair with
/// the desktop crate's `release_model_download_lock`, which releases it on success, failure
/// **and** cancel.
pub async fn acquire_model_download_lock(
    core: &AppCore,
    processor: &str,
    model_key: String,
) -> Option<::app_infra::ProcessingModelLock> {
    let infra = core.try_state::<AppInfraState>()?;
    match infra
        .acquire_processing_model_download_lock(processor, model_key)
        .await
    {
        Ok(lock) => Some(lock),
        Err(error) => {
            crate::debug_log::log_warn(format!(
                "failed to lock {processor} model for download; its queued jobs may fail against the in-flight model: {error}"
            ));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_lock_before_app_infra_is_managed() {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::test_support::core_in(Default::default(), dir.path());
        let lock = core.block_on(acquire_model_download_lock(&core, "ocr", "m".into()));
        assert!(lock.is_none());
    }
}
