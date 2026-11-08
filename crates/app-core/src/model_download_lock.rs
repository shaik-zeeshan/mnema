//! The `downloading` model lock a model download holds, and the absent-model
//! locks its release re-derives from the model catalogues.

use std::collections::BTreeSet;

use crate::{AppCore, AppInfraState};

/// Takes the `downloading` model lock a model download holds for its whole run. A download wipes
/// the install directory before it writes, so without the lock a queued job would be claimed
/// against a model that is not readable and spend a failure attempt on it. Pair with
/// [`release_model_download_lock`], which releases it on success, failure
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

/// Releases a [`acquire_model_download_lock`] lock and re-derives the absent-model locks, so a
/// download that failed or was cancelled leaves the (now missing) model locked rather than letting
/// its jobs run against nothing.
pub async fn release_model_download_lock(
    core: &AppCore,
    lock: Option<::app_infra::ProcessingModelLock>,
) {
    // The re-derive runs FIRST and the release second, so the two locks overlap instead of
    // leaving a gap. A failed or cancelled download left the install directory wiped, and
    // releasing before re-deriving would unlock a model that is not on disk for as long as the
    // re-derive takes (a settings read plus a status stat per subsystem) — long enough for a
    // worker to claim its jobs and spend their failure attempts against nothing. The absent sync
    // takes the row over from the download lock, so the model is never unlocked in between.
    reconcile_absent_model_locks(core).await;
    if let (Some(lock), Some(infra)) = (lock, core.try_state::<AppInfraState>()) {
        if let Err(error) = infra.release_processing_model_locks(&lock).await {
            crate::debug_log::log_warn(format!("failed to release model download lock: {error}"));
        }
    }
}

/// Locks every selected model that is not installed, and unlocks the ones that are. A locked model
/// is skipped by the claim predicate, so its jobs sit `queued` — not running, not failing, not
/// burning an attempt — until the model arrives. Recomputed rather than tracked, because nothing
/// owns an absent model's lifetime: run it at startup and whenever a download settles.
///
/// ponytail: only the *selected* model per subsystem is checked, because that is the model jobs are
/// frozen onto at admission. A job frozen onto some other model that later vanishes is not parked;
/// widen to the distinct model keys of queued jobs if that ever shows up in practice.
pub async fn reconcile_absent_model_locks(core: &AppCore) {
    let Some(infra) = core.try_state::<AppInfraState>() else {
        return;
    };
    let app_data_dir = &core.paths().config;
    let Some(settings_state) =
        core.try_state::<crate::recording_settings::RecordingSettingsState>()
    else {
        return;
    };
    let settings = crate::recording_settings::current_recording_settings(&settings_state);

    let absent = [
        (
            ::app_infra::OCR_PROCESSOR,
            crate::ocr_models::absent_selected_ocr_model_key(app_data_dir, &settings.ocr),
        ),
        (
            ::app_infra::AUDIO_TRANSCRIPTION_PROCESSOR,
            crate::audio_transcription_models::absent_selected_audio_transcription_model_key(
                app_data_dir,
                &settings.transcription,
            ),
        ),
        (
            ::app_infra::SPEAKER_ANALYSIS_PROCESSOR,
            crate::speaker_analysis_models::absent_selected_speaker_analysis_model_key(
                app_data_dir,
                &settings.speaker_analysis,
            ),
        ),
    ];

    for (processor, absent_model_key) in absent {
        let model_keys: BTreeSet<String> = absent_model_key.into_iter().collect();
        if let Err(error) = infra
            .sync_absent_processing_model_locks(processor, &model_keys)
            .await
        {
            crate::debug_log::log_warn(format!(
                "failed to reconcile absent-model locks for {processor}: {error}"
            ));
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
