//! Tauri commands for the Semantic Search Model catalog; the behaviour lives in
//! `app_core::semantic_search_models`.
//!
//! Three pieces stay here because they call code still in this crate:
//! the download task (`app_infra::release_model_download_lock`, which re-derives
//! the OCR/transcription/speaker absent-model locks), its start command, and the
//! model switch (`native_capture::persist_semantic_search_settings`).

use std::sync::{atomic::AtomicBool, Arc};

use app_core::debug_log::{log_error, log_info};
use app_core::semantic_search_models::{
    self as models, build_download_plan, claim_model_download, download_and_install_model,
    emit_download_progress, select_model_lock, DownloadPlan, ModelDownloadTaskError,
    SemanticSearchModelDownloadProgressDto, SemanticSearchModelDownloadRequestDto,
    SemanticSearchModelDownloadState, SemanticSearchModelDownloadStatusDto,
    SemanticSearchModelStatusResponseDto, SupportedModelDto, SEMANTIC_SEARCH_MODEL_LOCK_PROCESSOR,
};
use app_core::AppCore;
use semantic_search::resolve_descriptor;
use tauri::{Manager, State};

#[tauri::command]
pub fn get_semantic_search_model_status(
    core: State<'_, Arc<AppCore>>,
) -> Result<SemanticSearchModelStatusResponseDto, String> {
    models::get_semantic_search_model_status(&core)
}

#[tauri::command]
pub fn list_semantic_search_supported_models() -> Result<Vec<SupportedModelDto>, String> {
    models::list_semantic_search_supported_models()
}

#[tauri::command]
pub fn cancel_semantic_search_model_download(
    download_state: State<'_, SemanticSearchModelDownloadState>,
) -> Result<(), String> {
    models::cancel_semantic_search_model_download(&download_state)
}

#[tauri::command]
pub fn delete_semantic_search_model(
    core: State<'_, Arc<AppCore>>,
    request: SemanticSearchModelDownloadRequestDto,
    download_state: State<'_, SemanticSearchModelDownloadState>,
) -> Result<(), String> {
    models::delete_semantic_search_model(&core, request, &download_state)
}

/// Start downloading + installing a **Semantic Search Model**. Mirrors the
/// OCR/transcription model downloaders: claims the single download slot, emits a
/// `Starting` event, spawns the streaming download on the async runtime, and
/// returns immediately. Progress arrives as `Downloading` events with real
/// per-chunk byte counts on [`SEMANTIC_SEARCH_MODEL_DOWNLOAD_PROGRESS_EVENT`].
///
/// Mnema only downloads when the user picks a model here — there is no
/// auto-download path, so nothing is fetched unprompted (ADR 0036).
#[tauri::command]
pub fn start_semantic_search_model_download(
    app_handle: tauri::AppHandle,
    request: SemanticSearchModelDownloadRequestDto,
    download_state: tauri::State<'_, SemanticSearchModelDownloadState>,
) -> Result<SemanticSearchModelDownloadProgressDto, String> {
    let core = app_handle.state::<Arc<AppCore>>();
    let plan = build_download_plan(&core, &request).map_err(|error| {
        log_error(format!(
            "semantic search model download: failed to build plan for {}/{}: {error}",
            request.provider, request.model_id
        ));
        error.to_string()
    })?;
    let cancel_requested = Arc::new(AtomicBool::new(false));
    let cancel_notify = Arc::new(tokio::sync::Notify::new());

    claim_model_download(
        download_state.inner(),
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
    emit_download_progress(&core, &starting);

    let app_for_task = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        run_model_download_task(app_for_task, plan, cancel_requested, cancel_notify).await;
    });

    Ok(starting)
}

/// **Switch the Semantic Search Model**: rebuild the `vec0` table at the
/// newly-selected model's dimension, THEN persist the selection — two sequential
/// transactions ordered so the failure modes are recoverable rather than
/// permanently stuck.
///
/// This replaces a worse two-step ordering (persist `model_id`, *then* re-index)
/// that was the root of three faces of one bug (H1/H2 + the re-index race): if the
/// re-index failed after the persist, the table stayed at the old dimension while
/// the selection named a new-dimension model — every search hard-failed and the
/// backfill worker error-looped forever, with no recovery by re-selecting the same
/// model (the UI early-returns on an unchanged pick). Here the order is reversed,
/// so each failure leaves a consistent-or-self-healing state:
///   - **A failed rebuild never advances the selection.** The recreate runs in its
///     own transaction and rolls back, so the old model stays selected against the
///     intact old-dim table and the frontend surfaces the error.
///   - **A failed persist leaves a new-dim table that startup reconciliation
///     re-aligns.** The table is already at the new dimension but the selection
///     still names the old model; the error is reported, and the next startup's
///     [`reconcile_semantic_search_index_on_startup`] rebuilds the table back to
///     whatever model remained selected — so there is no permanently-stuck state.
/// The two writes are NOT one atomic operation; the ordering plus the startup
/// reconciler is what keeps table-dim and persisted-id from disagreeing for long.
///
/// The rebuild+persist pair is serialized process-wide by [`SELECT_MODEL_LOCK`] so
/// two concurrent invocations cannot interleave their recreate+persist and leave
/// the table dimension and persisted id disagreeing.
///
/// Different **Semantic Search Model Tier**s produce incomparable vectors and
/// `vec0` is a fixed-dim table, so a switch re-derives every **Search Result
/// Anchor** (ADR 0036); a switch can also change the dimension (768-dim `nomic` →
/// 1024-dim `bge-m3`), so the table is rebuilt, not merely cleared. Recreating it
/// re-exposes every `direct` anchor to the **Semantic Index Backfill** worker,
/// which re-derives each under the new model (newest-first), progress living
/// entirely in the DB. Returns the number of vectors discarded.
///
/// The caller (Settings UI) gates the switch behind a `@tauri-apps/plugin-dialog`
/// confirm; this command owns the persist+rebuild ordering so the frontend no
/// longer makes two separate invokes that could leave state half-applied.
#[tauri::command]
pub async fn select_semantic_search_model(
    model_id: String,
    app_handle: tauri::AppHandle,
    infra: tauri::State<'_, app_core::AppInfraState>,
    settings_state: tauri::State<'_, app_core::recording_settings::RecordingSettingsState>,
) -> Result<u64, String> {
    // Serialize the whole rebuild+persist against any concurrent switch, so two
    // invocations cannot interleave recreate + persist and leave the table
    // dimension disagreeing with the persisted id. Held until the command returns.
    let _switch_guard = select_model_lock().lock().await;

    // Resolve the target model's dimension up front. An unresolvable id (legacy /
    // not enumerated) aborts BEFORE touching the table or the persisted selection,
    // so a bad pick is a clean no-op.
    let current = app_core::semantic_search_worker::effective_semantic_search_settings(
        &app_handle.state::<Arc<AppCore>>(),
    );
    let descriptor = resolve_descriptor(&current.provider, &model_id)
        .ok_or_else(|| format!("unknown semantic search model '{model_id}'"))?;
    let dimension = descriptor.dimension;

    // Rebuild the table FIRST (the step that can fail under the worker's
    // concurrent writes). Only on success do we persist the new selection, so the
    // persisted `model_id` and the live table dimension stay in lockstep.
    let cleared = infra
        .semantic_search()
        .recreate_vectors_table(&model_id, dimension)
        .await
        .map_err(|error| format!("failed to rebuild semantic search vectors: {error}"))?;

    // Persist the selection now that the table matches. A persist failure here is
    // reported to the caller; the table is already at the new dimension, and the
    // next startup reconciliation would re-align it to whatever model remained
    // selected, so there is no permanently-stuck state.
    crate::native_capture::persist_semantic_search_settings(
        &app_handle,
        settings_state.inner(),
        capture_types::UpdateSemanticSearchSettingsRequest {
            enabled: None,
            provider: None,
            model_id: Some(Some(model_id.clone())),
        },
    )
    .map_err(|error| {
        format!(
            "rebuilt the semantic search index but failed to persist the model selection: {}",
            error.message
        )
    })?;

    // Hardening assert (F15): the rebuild + persist are two separate transactions
    // under the in-process `SELECT_MODEL_LOCK`, relying on startup reconciliation to
    // re-align a half-applied switch. Confirm the live vec0 table now names the
    // model we just selected, so a half-applied switch (a concurrent worker
    // DROP+CREATE landing between, or a recreate that did not take) is DETECTED and
    // logged loudly rather than silently degrading search to keyword-only. The
    // selection is already persisted, so a mismatch is reported but not rolled back —
    // the next startup reconciliation self-heals it.
    //
    // The check is on the MODEL STAMP, not the column width: with same-dimension
    // models in the catalog (gte-modernbert / granite-r2 are 768 like nomic,
    // granite-small is 384 like multilingual-e5-small), a width compare would read
    // green on exactly the switches most in need of detection.
    // Compared against the composite INDEX EPOCH (`model_id@recipe`), not the bare
    // id: the stamp also records the embedding recipe, so a stale-recipe table must
    // read as a mismatch here too.
    let expected_epoch = ::app_infra::vectors_index_epoch(&model_id);
    match infra.semantic_search().live_vector_model().await {
        Ok(Some(live)) if live == expected_epoch => {}
        Ok(other) => {
            app_core::debug_log::log_warn(format!(
                "semantic search model switch to '{model_id}': the live vec0 table is stamped {other:?}, NOT the selected model, after rebuild+persist (half-applied switch — startup reconciliation will re-align it; search stays keyword-only until then)"
            ));
        }
        Err(error) => {
            app_core::debug_log::log_warn(format!(
                "semantic search model switch to '{model_id}': could not read the live vec0 model stamp to confirm it matches the selection after rebuild+persist: {error}"
            ));
        }
    }

    app_core::debug_log::log_info(format!(
        "semantic search model switched to '{model_id}': rebuilt vector table at {dimension} dims, discarded {cleared} vector(s); the backfill worker will re-derive every anchor under the new model"
    ));
    Ok(cleared)
}

fn clear_active_download(app_handle: &tauri::AppHandle, provider: &str, model_id: &str) {
    if let Some(state) = app_handle.try_state::<SemanticSearchModelDownloadState>() {
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
    app_handle: tauri::AppHandle,
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
    let core = app_handle.state::<Arc<AppCore>>().inner().clone();
    let download_lock = app_core::model_download_lock::acquire_model_download_lock(
        &core,
        SEMANTIC_SEARCH_MODEL_LOCK_PROCESSOR,
        format!("{}/{}", plan.provider, plan.model_id),
    )
    .await;
    let result = download_and_install_model(&core, &plan, &cancel_requested, &cancel_notify).await;
    app_core::model_download_lock::release_model_download_lock(&core, download_lock).await;
    clear_active_download(&app_handle, &plan.provider, &plan.model_id);

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
