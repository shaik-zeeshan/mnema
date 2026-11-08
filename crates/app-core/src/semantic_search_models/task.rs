//! Starting a **Semantic Search Model** download and the task that runs it.

use super::*;

/// Start downloading + installing a **Semantic Search Model**. Mirrors the
/// OCR/transcription model downloaders: claims the single download slot, emits a
/// `Starting` event, spawns the streaming download on the async runtime, and
/// returns immediately. Progress arrives as `Downloading` events with real
/// per-chunk byte counts on [`SEMANTIC_SEARCH_MODEL_DOWNLOAD_PROGRESS_EVENT`].
///
/// Mnema only downloads when the user picks a model here — there is no
/// auto-download path, so nothing is fetched unprompted (ADR 0036).
pub fn start_semantic_search_model_download(
    core: &Arc<AppCore>,
    request: SemanticSearchModelDownloadRequestDto,
) -> Result<SemanticSearchModelDownloadProgressDto, String> {
    let plan = build_download_plan(core, &request).map_err(|error| {
        log_error(format!(
            "semantic search model download: failed to build plan for {}/{}: {error}",
            request.provider, request.model_id
        ));
        error.to_string()
    })?;
    let cancel_requested = Arc::new(AtomicBool::new(false));
    let cancel_notify = Arc::new(tokio::sync::Notify::new());

    claim_model_download(
        &core.state::<SemanticSearchModelDownloadState>(),
        &plan.provider,
        &plan.model_id,
        Arc::clone(&cancel_requested),
        Arc::clone(&cancel_notify),
    )
    .map_err(|error| {
        log_error(format!(
            "semantic search model download: could not claim slot for {}/{}: {error}",
            plan.provider, plan.model_id
        ));
        error.to_string()
    })?;

    let starting = SemanticSearchModelDownloadProgressDto::new(
        &plan.provider,
        &plan.model_id,
        SemanticSearchModelDownloadStatusDto::Starting,
        0,
        Some(plan.total_bytes),
        None,
    );
    emit_download_progress(core, &starting);

    let core_for_task = Arc::clone(core);
    core.spawn(async move {
        run_model_download_task(core_for_task, plan, cancel_requested, cancel_notify).await;
    });

    Ok(starting)
}

fn clear_active_download(core: &AppCore, provider: &str, model_id: &str) {
    if let Some(state) = core.try_state::<SemanticSearchModelDownloadState>() {
        if let Ok(mut active) = state.lock() {
            if active.as_ref().is_some_and(|download| {
                download.provider == provider && download.model_id == model_id
            }) {
                *active = None;
            }
        }
    }
}

async fn run_model_download_task(
    core: Arc<AppCore>,
    plan: DownloadPlan,
    cancel_requested: Arc<AtomicBool>,
    cancel_notify: Arc<tokio::sync::Notify>,
) {
    log_info(format!(
        "semantic search model download started: provider={}, model={}, install_dir={}, files={}, approx_total_bytes={}",
        plan.provider,
        plan.model_id,
        plan.install_dir.display(),
        plan.files.len(),
        plan.total_bytes,
    ));
    // ponytail: semantic search has no `processing_jobs` rows today (its worker is a sweep loop
    // already gated on `selected_model_available`, and it never burns failure attempts), so this
    // lock currently protects nothing. It is taken anyway so the four downloads behave alike the
    // day semantic search grows a job — drop it if that day never comes.
    let download_lock = crate::model_download_lock::acquire_model_download_lock(
        &core,
        SEMANTIC_SEARCH_MODEL_LOCK_PROCESSOR,
        format!("{}/{}", plan.provider, plan.model_id),
    )
    .await;
    let result = download_and_install_model(&core, &plan, &cancel_requested, &cancel_notify).await;
    crate::model_download_lock::release_model_download_lock(&core, download_lock).await;
    clear_active_download(&core, &plan.provider, &plan.model_id);

    match result {
        Ok(downloaded_total) => {
            log_info(format!(
                "semantic search model download completed: {}/{} installed at {} ({downloaded_total} bytes downloaded)",
                plan.provider,
                plan.model_id,
                plan.install_dir.display(),
            ));
            // Report the REAL accumulated download total (not the approximate catalog
            // size) as downloaded_bytes; total_bytes carries the larger of the two so
            // a UI progress bar never shows >100%.
            emit_download_progress(
                &core,
                &SemanticSearchModelDownloadProgressDto::new(
                    &plan.provider,
                    &plan.model_id,
                    SemanticSearchModelDownloadStatusDto::Completed,
                    downloaded_total,
                    Some(plan.total_bytes.max(downloaded_total)),
                    None,
                ),
            );
        }
        Err(ModelDownloadTaskError::Cancelled) => {
            log_info(format!(
                "semantic search model download cancelled: {}/{}; removing partial install at {}",
                plan.provider,
                plan.model_id,
                plan.install_dir.display(),
            ));
            let _ = std::fs::remove_dir_all(&plan.install_dir);
            emit_download_progress(
                &core,
                &SemanticSearchModelDownloadProgressDto::new(
                    &plan.provider,
                    &plan.model_id,
                    SemanticSearchModelDownloadStatusDto::Cancelled,
                    0,
                    Some(plan.total_bytes),
                    Some("download cancelled".to_string()),
                ),
            );
        }
        Err(ModelDownloadTaskError::Failed(error)) => {
            // A partial install must not be detected as Installed: the marker is
            // written last, so removing the dir leaves the model Missing.
            log_error(format!(
                "semantic search model download failed: {}/{}: {error}; removing partial install at {}",
                plan.provider,
                plan.model_id,
                plan.install_dir.display(),
            ));
            let _ = std::fs::remove_dir_all(&plan.install_dir);
            emit_download_progress(
                &core,
                &SemanticSearchModelDownloadProgressDto::new(
                    &plan.provider,
                    &plan.model_id,
                    SemanticSearchModelDownloadStatusDto::Failed,
                    0,
                    Some(plan.total_bytes),
                    Some(error.to_string()),
                ),
            );
        }
    }
}
