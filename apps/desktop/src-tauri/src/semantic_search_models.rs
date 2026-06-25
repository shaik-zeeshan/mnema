//! Desktop seam for the Semantic Search Model catalog and its model-gating.
//!
//! Mirrors `audio_transcription_models.rs`: the embedding runtime and the
//! pure-filesystem detector live in the `semantic-search` crate; this module is
//! the thin Tauri adapter that resolves the app data directory and reports
//! catalog status to the Settings UI. Download orchestration is owned by the
//! Settings slice; this slice only exposes status + the model-gating check that
//! the **Semantic Index Backfill** worker slice will consume.

use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::Duration,
};

use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use semantic_search::{
    builtin_model_manifest, detect_model_status, list_supported_models, model_install_dir,
    resolve_descriptor, semantic_search_models_dir, write_installed_marker, ModelStatusError,
    ModelStatusKind, SemanticSearchModelDescriptor, SemanticSearchModelTier, SEMANTIC_SEARCH_PROVIDER_ID,
};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

use crate::native_capture::debug_log::{log_error, log_info};

/// The frontend event the Settings UI listens on for live download progress.
/// The bytes/percent in each payload come from the streaming HTTP download of the
/// model's safetensors files (per-chunk), not a CLI progress bool — so the UI
/// receives real programmatic byte progress (ADR 0036 / issue #125).
pub const SEMANTIC_SEARCH_MODEL_DOWNLOAD_PROGRESS_EVENT: &str =
    "semantic_search_model_download_progress";

/// One model download may run at a time, mirroring the OCR/transcription model
/// downloaders. The cancel flag is shared with the running task.
pub type SemanticSearchModelDownloadState = Mutex<Option<ActiveSemanticSearchModelDownload>>;

#[derive(Debug, Clone)]
pub struct ActiveSemanticSearchModelDownload {
    provider: String,
    model_id: String,
    cancel_requested: Arc<AtomicBool>,
    /// Wakes a stalled download future the instant Cancel is requested. The
    /// `cancel_requested` flag alone is only observed by the per-chunk branch in
    /// the receive loop, which a half-open TCP / wedged CDN never reaches (the
    /// body simply stops arriving and `stream.next()` blocks forever). The
    /// download future also `select!`s on this notify, so Cancel works even with
    /// zero inbound bytes — the future wakes, sees the flag, and releases the
    /// single download slot rather than holding it for the process lifetime.
    cancel_notify: Arc<tokio::sync::Notify>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SemanticSearchModelStatusResponseDto {
    pub models_directory: String,
    pub models: Vec<SemanticSearchModelStatusDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SemanticSearchModelStatusDto {
    pub provider: String,
    pub model_id: String,
    pub display_name: String,
    pub description: String,
    pub tier: SemanticSearchModelTier,
    pub dimension: usize,
    pub max_tokens: usize,
    /// The HuggingFace repo id the model is downloaded from (sourced from the
    /// descriptor's `hf_repo`). The DTO field name stays `modelCode` for the stable
    /// frontend contract.
    pub model_code: String,
    /// Approximate on-disk footprint in bytes — the Settings disk-cost disclosure.
    pub approx_download_bytes: u64,
    pub license_label: Option<String>,
    pub status: ModelStatusKind,
    pub available: bool,
    pub install_path: String,
    pub missing_files: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SemanticSearchModelDownloadRequestDto {
    pub provider: String,
    pub model_id: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SemanticSearchModelDownloadStatusDto {
    Starting,
    Downloading,
    Installing,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SemanticSearchModelDownloadProgressDto {
    pub provider: String,
    pub model_id: String,
    pub status: SemanticSearchModelDownloadStatusDto,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub message: Option<String>,
}

/// One curated candle-supported text-embedding model the **Custom** picker can
/// offer, distilled to the fields the Settings UI needs (serde camelCase to match
/// the other DTOs). The open "any ONNX model" picker is gone (ADR 0037): this list
/// is exactly the curated catalog, sourced from the crate's hand-coded
/// descriptors. `model_id` is the catalog slug, so a Custom pick installs under the
/// same `{provider}/{model_id}` layout as the guided tiers. The DTO field name
/// stays `modelCode` for the stable frontend contract (sourced from the
/// descriptor's `hf_repo`). `approx_download_bytes` is omitted (None) here; the UI
/// shows the disk cost from the status DTO once known.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SupportedModelDto {
    pub model_id: String,
    pub display_name: String,
    pub model_code: String,
    pub dimension: usize,
    pub description: String,
    pub multilingual: bool,
    pub approx_download_bytes: Option<u64>,
}

impl SemanticSearchModelDownloadProgressDto {
    fn new(
        provider: impl Into<String>,
        model_id: impl Into<String>,
        status: SemanticSearchModelDownloadStatusDto,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
        message: Option<String>,
    ) -> Self {
        Self {
            provider: provider.into(),
            model_id: model_id.into(),
            status,
            downloaded_bytes,
            total_bytes,
            message,
        }
    }
}

/// One file the candle backend loads from disk and the HuggingFace path it is
/// fetched from. Both are the **same repo-relative path** (e.g. `model.safetensors`):
/// the file is fetched from `…/resolve/main/<path>` and written to
/// `<install>/<path>`. The safetensors layout puts all three required files
/// (`model.safetensors`, `config.json`, `tokenizer.json`) at the repo root, so the
/// path preservation is trivial — there is no `onnx/` subdirectory or external-data
/// sibling anymore (ADR 0037). The detector requires this same relative path, so
/// the download, on-disk, and completeness views all agree.
#[derive(Debug, Clone)]
struct ModelFileSpec {
    relative_path: String,
    /// The pinned SHA256 of this file, when known. Mirrors the
    /// **speaker-analysis** downloader's `ModelArtifactFile.sha256: Option<String>`
    /// policy: a `Some(_)` hash is verified against the downloaded bytes and a
    /// mismatch fails the install (a tampered/truncated/corrupt file is never
    /// marked Installed); a `None` hash means the file is **not yet pinned** —
    /// it installs but is logged as "integrity unverified" so the supply-chain gap
    /// is visible rather than silently trusted.
    ///
    /// Guided-tier hashes are pinned in [`pinned_file_sha256`]. **Custom**
    /// (user-picked) models have no pinned digest and always download unverified.
    expected_sha256: Option<String>,
}

#[derive(Debug, thiserror::Error)]
enum ModelDownloadError {
    #[error("download for {provider}/{model_id} is already running")]
    AlreadyRunning { provider: String, model_id: String },
    #[error("no active semantic search model download")]
    NoActiveDownload,
    #[error("semantic search model not found for provider={provider}, modelId={model_id}")]
    ModelNotFound { provider: String, model_id: String },
    #[error("failed to inspect semantic search model status: {0}")]
    Status(#[from] ModelStatusError),
    #[error("download failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("failed to create directory {path}: {source}")]
    CreateDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to create file {path}: {source}")]
    CreateFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write file {path}: {source}")]
    WriteFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("downloaded model is missing required files: {0:?}")]
    IncompleteLayout(Vec<String>),
    #[error("refusing to install guided-tier model {provider}/{model_id}: its weights file {weights_relative_path} has no pinned SHA256 to verify against (fail-closed)")]
    GuidedTierWeightsUnpinned {
        provider: String,
        model_id: String,
        weights_relative_path: String,
    },
    #[error("downloaded file {relative_path} checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch {
        relative_path: String,
        expected: String,
        actual: String,
    },
    #[error("server advertised {expected} bytes for {relative_path} but {actual} were written (truncated download)")]
    ContentLengthMismatch {
        relative_path: String,
        expected: u64,
        actual: u64,
    },
    #[error("downloaded file {relative_path} is empty (zero bytes) with no Content-Length to check against (dropped/truncated download)")]
    EmptyDownload { relative_path: String },
    #[error("download of {relative_path} stalled: no bytes received for {idle_secs}s (half-open connection or wedged CDN); aborting so the slot is released")]
    Stalled {
        relative_path: String,
        idle_secs: u64,
    },
    #[error("unpinned file {relative_path} was served with no Content-Length: refusing to install an unverifiable large file with no length and no pinned SHA256 to bound truncation against (fail-closed)")]
    MissingContentLength { relative_path: String },
    #[error("download of {relative_path} overran the advertised {advertised} bytes (received {received}) — aborting before it fills the volume")]
    ContentLengthOverrun {
        relative_path: String,
        advertised: u64,
        received: u64,
    },
    #[error("failed to read file {path} for checksum: {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("insufficient disk space for {provider}/{model_id}: need ~{needed_bytes} bytes, have ~{available_bytes} free on the models volume")]
    InsufficientDiskSpace {
        provider: String,
        model_id: String,
        needed_bytes: u64,
        available_bytes: u64,
    },
}

#[derive(Debug, thiserror::Error)]
enum ModelDownloadTaskError {
    #[error("download cancelled")]
    Cancelled,
    #[error(transparent)]
    Failed(#[from] ModelDownloadError),
}

struct DownloadPlan {
    provider: String,
    model_id: String,
    /// The HuggingFace repo the files are fetched from (e.g.
    /// `nomic-ai/nomic-embed-text-v1.5`).
    hf_repo: String,
    /// The immutable HuggingFace commit SHA every file is fetched from, so the
    /// download pins `…/resolve/{hf_revision}/{path}` rather than the mutable
    /// `main` branch (kills the force-push/mutable-ref surface). Sourced from the
    /// descriptor's `hf_revision`.
    hf_revision: String,
    /// The user-facing model tier. The install is gated fail-closed for the guided
    /// tiers (`English`/`Multilingual`): their weights MUST verify against a pinned
    /// digest before the model is marked Installed.
    tier: SemanticSearchModelTier,
    /// The repo-relative path of the safetensors weights, used by the guided-tier
    /// fail-closed gate to confirm the weights file carries a pinned digest.
    weights_relative_path: String,
    /// The repo-relative path of an **auxiliary head** weights file, when the model
    /// loads a second safetensors alongside the base backbone (e.g. Stella's dense
    /// projection head `2_Dense_2048/model.safetensors`). `None` for every
    /// single-backbone model. The guided-tier gate requires this file to carry a
    /// pinned digest too when present (F14) — the "guided tiers can never install
    /// unverified" contract covers EVERY required file, not just the base weights.
    aux_weights_relative_path: Option<String>,
    install_dir: PathBuf,
    files: Vec<ModelFileSpec>,
    total_bytes: u64,
}

#[tauri::command]
pub fn get_semantic_search_model_status(
    app_handle: tauri::AppHandle,
) -> Result<SemanticSearchModelStatusResponseDto, String> {
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|error| format!("failed to resolve app data directory: {error}"))?;
    // Surface the currently-persisted selection even when it is a Custom model
    // outside the 3 guided tiers, so the Settings UI can show that Custom pick's
    // installed/selected state. We read the effective settings here (not an extra
    // command argument) so the frontend keeps calling this with no parameters and
    // the "selected" row stays authoritative with the persisted selection.
    let selected =
        crate::semantic_search_worker::effective_semantic_search_settings(&app_handle).model_id;
    build_semantic_search_model_status_response(&app_data_dir, selected.as_deref())
        .map_err(|error| format!("failed to inspect semantic search models: {error}"))
}

/// List the curated candle-supported text-embedding models for the **Custom**
/// picker.
///
/// Drives off the crate's hand-coded catalog (ADR 0037) — the open "any ONNX
/// model" picker is gone, so this is exactly the curated list. The frontend Custom
/// picker consumes these to let a user pick a supported model; downloading one then
/// reuses [`start_semantic_search_model_download`] once the selection is persisted.
#[tauri::command]
pub fn list_semantic_search_supported_models() -> Result<Vec<SupportedModelDto>, String> {
    let models = list_supported_models()
        .into_iter()
        .map(|model| SupportedModelDto {
            model_id: model.model_id,
            display_name: model.display_name,
            // The DTO field name stays `modelCode` (stable frontend contract); it is
            // sourced from the descriptor's `hf_repo`.
            model_code: model.hf_repo,
            dimension: model.dimension,
            description: model.description,
            multilingual: model.multilingual,
            // The catalog descriptor's size rides the status DTO; left None here.
            approx_download_bytes: None,
        })
        .collect();
    Ok(models)
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
    let plan = build_download_plan(&app_handle, &request).map_err(|error| {
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
    emit_download_progress(&app_handle, &starting);

    let app_for_task = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        run_model_download_task(app_for_task, plan, cancel_requested, cancel_notify).await;
    });

    Ok(starting)
}

/// Cancel the in-flight **Semantic Search Model** download, if any.
#[tauri::command]
pub fn cancel_semantic_search_model_download(
    download_state: tauri::State<'_, SemanticSearchModelDownloadState>,
) -> Result<(), String> {
    let active = download_state
        .lock()
        .map_err(|_| "semantic search model download state poisoned".to_string())?;
    let Some(active) = active.as_ref() else {
        return Err(ModelDownloadError::NoActiveDownload.to_string());
    };
    active.cancel_requested.store(true, Ordering::SeqCst);
    // Wake the download future immediately so Cancel works even when the body has
    // stalled with zero inbound bytes (the per-chunk flag check would never be
    // reached). The future `select!`s on this notify; `notify_waiters` wakes the
    // currently-parked branch. (Set the flag FIRST so the woken future observes it.)
    active.cancel_notify.notify_waiters();
    Ok(())
}

/// The vec0 column dimension migration 0039 ships with — the English default
/// tier (`nomic-embed-text-v1.5`). Used as the rebuild fallback when no model is
/// selected, so the table matches a fresh-migration DB.
const DEFAULT_SEMANTIC_SEARCH_DIMENSION: usize = 768;

/// Process-wide serialization for [`select_semantic_search_model`]'s
/// rebuild-then-persist pair. Mirrors the single-slot policy the download path
/// enforces via [`claim_model_download`]: only one model switch runs at a time, so
/// two concurrent invocations can never interleave recreate + persist and leave
/// the live table dimension disagreeing with the persisted `model_id`. A
/// module-level lock (rather than Tauri-managed state) keeps this self-contained —
/// the guard is held across both writes inside the command.
static SELECT_MODEL_LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();

fn select_model_lock() -> &'static tokio::sync::Mutex<()> {
    SELECT_MODEL_LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
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
    infra: tauri::State<'_, crate::app_infra::AppInfraState>,
    settings_state: tauri::State<'_, crate::native_capture::RecordingSettingsState>,
) -> Result<u64, String> {
    // Serialize the whole rebuild+persist against any concurrent switch, so two
    // invocations cannot interleave recreate + persist and leave the table
    // dimension disagreeing with the persisted id. Held until the command returns.
    let _switch_guard = select_model_lock().lock().await;

    // Resolve the target model's dimension up front. An unresolvable id (legacy /
    // not enumerated) aborts BEFORE touching the table or the persisted selection,
    // so a bad pick is a clean no-op.
    let current = crate::semantic_search_worker::effective_semantic_search_settings(&app_handle);
    let descriptor = resolve_descriptor(&current.provider, &model_id)
        .ok_or_else(|| format!("unknown semantic search model '{model_id}'"))?;
    let dimension = descriptor.dimension;

    // Rebuild the table FIRST (the step that can fail under the worker's
    // concurrent writes). Only on success do we persist the new selection, so the
    // persisted `model_id` and the live table dimension stay in lockstep.
    let cleared = infra
        .semantic_search()
        .recreate_vectors_table(dimension)
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
    // re-align a half-applied switch. At minimum, confirm the live vec0 column width
    // now matches the selected descriptor's dimension here, so a half-applied switch
    // (a concurrent worker DROP+CREATE landing between, or a recreate that did not
    // take) is DETECTED and logged loudly rather than silently degrading search to
    // keyword-only. The selection is already persisted, so a mismatch is reported but
    // not rolled back — the next startup reconciliation self-heals it. (The deeper
    // model-identity epoch stamp remains out of scope for this pass; see ADR 0036.)
    match infra.semantic_search().live_vector_dimension().await {
        Ok(Some(live)) if live == dimension => {}
        Ok(other) => {
            crate::native_capture::debug_log::log_error(format!(
                "semantic search model switch to '{model_id}': live vec0 column width {other:?} does NOT match the selected model dimension {dimension} after rebuild+persist (half-applied switch — startup reconciliation will re-align it; search stays keyword-only until then)"
            ));
        }
        Err(error) => {
            crate::native_capture::debug_log::log_error(format!(
                "semantic search model switch to '{model_id}': could not read the live vec0 column width to confirm it matches the selected dimension {dimension} after rebuild+persist: {error}"
            ));
        }
    }

    crate::native_capture::debug_log::log_info(format!(
        "semantic search model switched to '{model_id}': rebuilt vector table at {dimension} dims, discarded {cleared} vector(s); the backfill worker will re-derive every anchor under the new model"
    ));
    Ok(cleared)
}

/// The vec0 table dimension reconciliation should expect for a settings snapshot,
/// or `None` when reconciliation must be **skipped** because the dimension cannot
/// be determined safely — resolved from the persisted `model_id` **regardless of
/// the `enabled` flag**.
///
/// Reconciliation keys off the *selected model*, NOT the *active feature*: a user
/// on a non-768 tier (Multilingual e5 = 384, Custom bge-m3 = 1024) who toggles
/// Semantic Search OFF still has that `model_id` persisted (disabling never clears
/// it), and their vec0 table is at the model's width, not 768. If we resolved the
/// dimension through the worker's `resolve_selected_descriptor` (which returns
/// `None` the moment `enabled == false`, BEFORE it even reads `model_id`), startup
/// would fall back to 768 and `reconcile_vectors_table(768)` would DROP+recreate
/// their table — wiping the entire vector index and forcing a full re-embed. So we
/// resolve straight from `model_id` here, and the `enabled` flag is deliberately
/// ignored.
///
/// Two `None`-shaped inputs must NOT be conflated:
///   - `model_id == None` (a fresh/never-selected profile) => `Some(768)`: the
///     migration default is correct, a fresh DB is already a `float[768]` table.
///   - `model_id == Some(unresolvable)` (catalog/config drift — the id no longer
///     resolves to a descriptor) => `None`: we do NOT know the table's true
///     dimension, so falling back to 768 here would DROP a populated 384/1024
///     table and force a full re-embed over a transient resolve failure. Returning
///     `None` makes the caller SKIP reconciliation and leave the existing table
///     untouched; the next time the id resolves (or the real selection re-runs the
///     switch), the table re-aligns.
fn reconcile_expected_dimension(settings: &capture_types::SemanticSearchSettings) -> Option<usize> {
    match settings.model_id.as_deref() {
        None => Some(DEFAULT_SEMANTIC_SEARCH_DIMENSION),
        Some(model_id) => resolve_descriptor(&settings.provider, model_id)
            .map(|descriptor| descriptor.dimension),
    }
}

/// Reconcile the `vec0` table dimension against the selected model's expected
/// dimension on startup — the **self-heal** for a permanently-stuck switch.
///
/// If a model switch ever left the table at the old dimension while the selection
/// named a new-dimension model (e.g. a rebuild that failed under DB contention in
/// an older build, or a hand-edited config), every search degrades to
/// keyword-only and the worker idles forever — recovery cannot come from
/// re-selecting the same model (the UI early-returns on an unchanged pick). Run
/// once on the deferred-startup seam, this rebuilds the table to the selected
/// model's dimension so the worker can backfill under it again. Idempotent: a
/// table that already matches is left untouched (the common case — no rebuild, no
/// vectors discarded).
///
/// The expected dimension is resolved from `model_id` **ignoring `enabled`** (see
/// [`reconcile_expected_dimension`]): a disabled-but-previously-selected non-768
/// model must KEEP its table (disabling never clears `model_id`), so reconciliation
/// only falls back to the migration default 768 when no model was ever selected.
/// Resolving through the worker's enabled-gated `resolve_selected_descriptor` here
/// would wipe a disabled non-768 user's index on every restart (B1).
///
/// A persisted `model_id` that no longer resolves (catalog/config drift) makes
/// [`reconcile_expected_dimension`] return `None`; we then SKIP reconciliation
/// entirely and leave the existing table as-is, so a transient resolve failure can
/// never silently DROP a populated 384/1024 index back to 768. The real selection
/// re-aligns the table the next time the id resolves.
pub(crate) async fn reconcile_semantic_search_index_on_startup(
    infra: &crate::app_infra::AppInfraState,
    settings: &capture_types::SemanticSearchSettings,
) {
    let Some(expected_dimension) = reconcile_expected_dimension(settings) else {
        crate::native_capture::debug_log::log_info(format!(
            "semantic search startup reconciliation skipped: selected model '{}' (provider '{}') does not resolve to a known descriptor; leaving the existing vec0 table untouched rather than wiping it to the {DEFAULT_SEMANTIC_SEARCH_DIMENSION}-dim default (catalog/config drift)",
            settings.model_id.as_deref().unwrap_or("<none>"),
            settings.provider
        ));
        return;
    };
    match infra
        .semantic_search()
        .reconcile_vectors_table(expected_dimension)
        .await
    {
        Ok(Some(discarded)) => crate::native_capture::debug_log::log_info(format!(
            "semantic search startup reconciliation: live vec0 dimension disagreed with the selected model; rebuilt the table at {expected_dimension} dims (discarded {discarded} stale vector(s)); the backfill worker will re-derive every anchor"
        )),
        Ok(None) => {
            // The common case: the table already matches the selected model. No log
            // so a default/Anthropic-only user sees nothing on every launch.
        }
        Err(error) => crate::native_capture::debug_log::log_error(format!(
            "semantic search startup reconciliation failed to read/rebuild the vec0 table (search stays keyword-only until it succeeds): {error}"
        )),
    }
}

/// Build one status DTO for a descriptor by detecting its on-disk state.
fn status_dto_for(
    models_dir: &Path,
    descriptor: &SemanticSearchModelDescriptor,
) -> Result<SemanticSearchModelStatusDto, ModelStatusError> {
    let status = detect_model_status(models_dir, descriptor)?;
    Ok(SemanticSearchModelStatusDto {
        provider: descriptor.provider.clone(),
        model_id: descriptor.model_id.clone(),
        display_name: descriptor.display_name.clone(),
        description: descriptor.description.clone(),
        tier: descriptor.tier,
        dimension: descriptor.dimension,
        max_tokens: descriptor.max_tokens,
        model_code: descriptor.hf_repo.clone(),
        approx_download_bytes: descriptor.approx_download_bytes,
        license_label: descriptor.license_label.clone(),
        status: status.status,
        available: status.is_available(),
        install_path: status.install_path.to_string_lossy().into_owned(),
        missing_files: status.missing_files,
    })
}

/// Report the 3 guided manifest tiers, plus every **Custom** model that is
/// already downloaded on disk, plus the currently-selected Custom model.
///
/// `selected_model_id` is the persisted `RecordingSettings.semantic_search`
/// selection (always the `local` provider namespace in v1).
///
/// A Custom model (a curated catalog model surfaced via the Custom picker, e.g.
/// `bge-m3`) must be able to be **activated after it is downloaded**. The Settings
/// UI's "Use this model"
/// action only appears once the picked model's status row reports `available`, and
/// the picked-model view only carries a real `available` when the model is present
/// in this response (the catalog fallback is structurally unavailable). So a
/// Custom model that was downloaded but not yet selected MUST appear here with its
/// real on-disk status — otherwise it is a dead end (downloadable, never
/// activatable). We enumerate the on-disk install dirs and append any installed
/// Custom model not already listed; the selected model is appended last even if it
/// is not yet installed, so its "Not installed → Download" state stays visible.
/// Unresolvable ids (legacy, no longer enumerated) are simply omitted, never an
/// error.
fn build_semantic_search_model_status_response(
    app_data_dir: &Path,
    selected_model_id: Option<&str>,
) -> Result<SemanticSearchModelStatusResponseDto, ModelStatusError> {
    let models_dir = semantic_search_models_dir(app_data_dir);
    let manifest = builtin_model_manifest();
    let mut models = Vec::with_capacity(manifest.models.len() + 1);

    for descriptor in &manifest.models {
        models.push(status_dto_for(&models_dir, descriptor)?);
    }

    // Append every Custom model already downloaded on disk (not just the selected
    // one), so a downloaded-but-unselected Custom pick carries a real `available`
    // and the UI's "Use this model" activation becomes reachable.
    for model_id in installed_custom_model_ids(&models_dir) {
        if models.iter().any(|model| model.model_id == model_id) {
            continue;
        }
        if let Some(descriptor) = resolve_descriptor(SEMANTIC_SEARCH_PROVIDER_ID, &model_id) {
            models.push(status_dto_for(&models_dir, &descriptor)?);
        }
    }

    // Append the selected Custom model even when it is NOT yet installed, so its
    // Download state is visible. (An installed selection was already covered by the
    // disk scan above; this `already_listed` guard prevents a duplicate.)
    if let Some(model_id) = selected_model_id {
        let already_listed = models.iter().any(|model| model.model_id == model_id);
        if !already_listed {
            if let Some(descriptor) = resolve_descriptor(SEMANTIC_SEARCH_PROVIDER_ID, model_id) {
                models.push(status_dto_for(&models_dir, &descriptor)?);
            }
        }
    }

    Ok(SemanticSearchModelStatusResponseDto {
        models_directory: models_dir.to_string_lossy().into_owned(),
        models,
    })
}

/// Enumerate the `model_id`s of every **local-provider** model directory that
/// exists on disk under `semantic_search_models/local/`. This is a cheap
/// directory listing (no per-model status detection yet): each id is a candidate
/// Custom model the caller resolves + status-checks. A model that has a directory
/// but is incomplete (no marker / missing files) still surfaces — its
/// `detect_model_status` will report it Missing, which is correct (it shows as
/// "Not installed" rather than vanishing). Returns an empty list when the install
/// directory does not exist or cannot be read (a fresh profile), never an error.
fn installed_custom_model_ids(models_dir: &Path) -> Vec<String> {
    let provider_dir = models_dir.join(SEMANTIC_SEARCH_PROVIDER_ID);
    let Ok(entries) = std::fs::read_dir(&provider_dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect()
}

/// The list of files to download for a model, as **repo-relative paths**.
///
/// **Single file-list authority** (ADR 0036 deepening #4): download and
/// completeness now read the SAME list — `descriptor.expected_layout.required_files`
/// — so a model can never download "successfully" yet report broken. That list is
/// the hand-coded safetensors layout (ADR 0037): the three repo-root files
/// `model.safetensors`, `config.json`, `tokenizer.json`. The old ONNX graph +
/// external-data siblings (e.g. bge-m3's `onnx/model.onnx_data`) are gone, so
/// there is no second derivation here to drift out of sync.
///
/// Each path is zipped with its pinned SHA256 (when known) for the integrity gate.
fn model_file_specs(descriptor: &SemanticSearchModelDescriptor) -> Vec<ModelFileSpec> {
    descriptor
        .expected_layout
        .required_files
        .iter()
        .map(|relative_path| ModelFileSpec {
            expected_sha256: pinned_file_sha256(&descriptor.hf_repo, relative_path)
                .map(str::to_owned),
            relative_path: relative_path.clone(),
        })
        .collect()
}

/// The pinned SHA256 for one repo-relative file of a **guided-tier** model, when
/// known. Returns `None` for any file we have not yet sourced a real hash for —
/// including every **Custom** (user-picked) model, which has no pinned digest by
/// definition. A `None` here is a deliberate "integrity unverified" signal, not a
/// silent trust: [`download_and_install_model`] logs it and `verify_file_checksum`
/// skips verification for it.
///
/// Mirrors the other model downloaders, which pin per-file SHA256 in their
/// manifests (`ModelArtifactFile.sha256`). The semantic-search downloader fetches
/// directly from each model's HuggingFace repo at the pinned revision
/// (`…/resolve/{hf_revision}/<path>`), so the hashes below are keyed by
/// `(hf_repo, repo-relative path)` and verified against the LFS-published digest.
///
/// Integrity is now **fail-CLOSED for the two guided tiers**: the real LFS sha256
/// of each weights file is pinned below for `nomic-embed-text-v1.5` (English) and
/// `multilingual-e5-small` (Multilingual), so a tampered/truncated weights file
/// fails verification and never installs (see also the guided-tier gate in
/// [`download_and_install_model`], which refuses to mark a guided tier Installed
/// unless its weights matched a pinned digest). The **Custom** tier `BAAI/bge-m3`
/// ships only a PyTorch `pytorch_model.bin` (no `model.safetensors`); that `.bin`
/// is its declared weights file, so it now ALSO carries a pinned digest below —
/// Custom stays exempt from the guided-tier MUST-pin gate, but a present pin makes
/// its install fail-closed too. Any file still without a digest falls through to
/// the logged "integrity unverified" path, trusting the revision pin + TLS.
fn pinned_file_sha256(hf_repo: &str, relative_path: &str) -> Option<&'static str> {
    let pinned: &[(&str, &str)] = match hf_repo {
        // English tier — pinned sha256 of every required guided-tier file: the
        // safetensors weights plus the candle-loaded `config.json` and
        // `tokenizer.json`, so the whole guided-tier install is integrity-verified
        // (not just the weights). Hashes computed from the pinned revision.
        "nomic-ai/nomic-embed-text-v1.5" => &[
            (
                "model.safetensors",
                "9e7d262b1fe5ea350782829496efa831901b77486bbde1cea54a4c822d010d5c",
            ),
            (
                "config.json",
                "9ab00bd92cee80a569f708140b7b6c1661a65891ff3765b1519e181ba2f2c92b",
            ),
            (
                "tokenizer.json",
                "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66",
            ),
        ],
        // Multilingual tier — pinned sha256 of every required guided-tier file:
        // weights + `config.json` + `tokenizer.json`. Hashes computed from the
        // pinned revision.
        "intfloat/multilingual-e5-small" => &[
            (
                "model.safetensors",
                "1a55775f53449dac10a2bcbc312469fac40b96d53198c407081a831f81c98477",
            ),
            (
                "config.json",
                "69137736cab8b8903a07fe8afaafdda25aac55415a12a55d1bffa9f581abf959",
            ),
            (
                "tokenizer.json",
                "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39",
            ),
        ],
        // Custom tier — `BAAI/bge-m3` ships only `pytorch_model.bin` (no
        // `model.safetensors`), and that `.bin` is now the descriptor's weights
        // file, so we DO have a pinnable weights digest: the LFS sha256 of the
        // PyTorch checkpoint at the pinned revision. Custom/bge-m3 stays exempt from
        // the guided-tier MUST-pin gate, but pinning here makes its install
        // fail-closed too (a tampered/truncated `.bin` fails verification and never
        // installs).
        "BAAI/bge-m3" => &[(
            "pytorch_model.bin",
            "b5e0ce3470abf5ef3831aa1bd5553b486803e83251590ab7ff35a117cf6aad38",
        )],
        // Custom tier — `NovaSearch/stella_en_400M_v5` loads a base backbone plus a
        // separate dense projection head (`2_Dense_2048/model.safetensors`). Both
        // are multi-GB-scale LFS files; pinning BOTH closes F7's "integrity
        // UNVERIFIED — trusting TLS only" branch, so a redirect to an un-allowlisted
        // CDN serving swapped bytes fails verification and never installs. The
        // digests are the LFS `oid sha256` published at the pinned revision
        // (`ffeb2b7ee715c226d4ffe5e4619f7dbb48624c20`).
        "NovaSearch/stella_en_400M_v5" => &[
            (
                "model.safetensors",
                "17e549d16172a548a3115739b55575968eb6523653daad76c46b0758e9425032",
            ),
            (
                "2_Dense_2048/model.safetensors",
                "a831055e5110e81c03ed6559f4ebf5842630f227ded6b6c18826700d548b990f",
            ),
        ],
        // Custom tier — `Snowflake/snowflake-arctic-embed-l-v2.0` ships a single
        // multi-GB safetensors backbone. Pinning its LFS sha256 (published at the
        // pinned revision `ac6544c8a46e00af67e330e85a9028c66b8cfd9a`) closes the same
        // F7 unverified branch for Arctic.
        "Snowflake/snowflake-arctic-embed-l-v2.0" => &[(
            "model.safetensors",
            "21bf1a120b1c6562aeec379dfa9039b0d360591c784cb1c6786e87256b738ee1",
        )],
        _ => &[],
    };
    pinned
        .iter()
        .find(|(path, _)| *path == relative_path)
        .map(|(_, sha256)| *sha256)
}

/// Verify a downloaded file's SHA256 against its pinned digest.
///
/// Mirrors the **speaker-analysis** `validate_artifact_sha256(path, Option<&str>)`:
/// a `None`/empty pinned hash is a no-op success (the file is **not yet pinned** —
/// integrity unverified, logged by the caller); a present hash is hashed from disk
/// and compared case-insensitively, returning [`ModelDownloadError::ChecksumMismatch`]
/// on disagreement so a tampered/corrupt file fails the install before the
/// `.installed.json` marker is ever written.
fn verify_file_checksum(
    file_path: &Path,
    relative_path: &str,
    expected_sha256: Option<&str>,
) -> Result<(), ModelDownloadError> {
    let Some(expected) = expected_sha256.filter(|value| !value.trim().is_empty()) else {
        return Ok(());
    };
    let actual = sha256_of_file(file_path)?;
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(ModelDownloadError::ChecksumMismatch {
            relative_path: relative_path.to_string(),
            expected: expected.to_string(),
            actual,
        });
    }
    Ok(())
}

/// Stream the file through SHA256 in 8 KiB chunks so a multi-GB safetensors file
/// never loads into memory. Returns the lowercase hex digest.
fn sha256_of_file(file_path: &Path) -> Result<String, ModelDownloadError> {
    let mut file =
        std::fs::File::open(file_path).map_err(|source| ModelDownloadError::ReadFile {
            path: file_path.to_path_buf(),
            source,
        })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = std::io::Read::read(&mut file, &mut buffer).map_err(|source| {
            ModelDownloadError::ReadFile {
                path: file_path.to_path_buf(),
                source,
            }
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Fail-closed install gate for the **guided tiers** (B2 / F14): a model whose
/// `tier` is `English` or `Multilingual` MUST verify EVERY required weights file
/// against a pinned digest, so a guided tier can never install down the unverified
/// "trust TLS only" path. The per-file checksum loop already ran and a mismatch
/// aborted; this only refuses the finalize when a guided tier's weights file
/// carries no pinned digest at all (`pinned_file_sha256` returns `None` for it).
///
/// The check covers BOTH the base `weights_relative_path` AND the auxiliary head
/// `aux_weights_relative_path` when present (F14): the layout supports an aux head
/// (Stella's `2_Dense_2048/model.safetensors`), and the "guided tiers can never
/// install unverified" contract is about every required weights file, not just the
/// base. Today no English/Multilingual tier carries an aux head, so the aux branch
/// is dormant for them — but it is the correct, future-proof contract: a guided
/// tier that ever loaded a second head would be required to pin it too.
///
/// Custom tier (and any unpinned file) is exempt — it keeps the "integrity
/// unverified" path, trusting the revision pin + TLS (and, for Stella/Arctic,
/// happens to be pinned anyway). So this is a no-op for the Custom tier.
fn guard_guided_tier_weights_pinned(plan: &DownloadPlan) -> Result<(), ModelDownloadError> {
    let is_guided = matches!(
        plan.tier,
        SemanticSearchModelTier::English | SemanticSearchModelTier::Multilingual
    );
    if !is_guided {
        return Ok(());
    }
    // Every required weights file of a guided tier must be pinned: the base
    // backbone and, when the model loads a second safetensors head, that head too.
    let required_weights = std::iter::once(plan.weights_relative_path.as_str())
        .chain(plan.aux_weights_relative_path.as_deref());
    for relative_path in required_weights {
        if pinned_file_sha256(&plan.hf_repo, relative_path).is_none() {
            return Err(ModelDownloadError::GuidedTierWeightsUnpinned {
                provider: plan.provider.clone(),
                model_id: plan.model_id.clone(),
                weights_relative_path: relative_path.to_string(),
            });
        }
    }
    Ok(())
}

/// The HuggingFace `resolve/{hf_revision}` URL for one file in a model repo.
///
/// Pins the **immutable commit SHA** (the descriptor's `hf_revision`) rather than
/// the mutable `main` branch, so an upstream force-push / branch rewrite can never
/// swap the bytes under a pinned-digest verification and every install is
/// reproducible.
fn hf_file_url(hf_repo: &str, hf_revision: &str, hf_relative_path: &str) -> String {
    format!("https://huggingface.co/{hf_repo}/resolve/{hf_revision}/{hf_relative_path}")
}

/// How long to wait for the TCP+TLS handshake before giving up. A wedged CDN that
/// never completes the handshake errors here rather than hanging the task (and
/// holding the single download slot) forever. Conservative so a slow-but-live link
/// still connects.
const DOWNLOAD_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum idle gap between received body bytes before the read is declared
/// stalled. A half-open TCP / wedged CDN that returns 200 headers and then stops
/// sending body bytes leaves `stream.next()` blocking forever — the per-chunk
/// cancel branch in the receive loop is never reached and the slot stays claimed
/// for the process lifetime (every later download then returns AlreadyRunning).
/// Wrapping each `stream.next()` in a `tokio::time::timeout` of this length resets
/// the clock on every chunk, so a stalled body errors and releases the slot while a
/// live-but-slow multi-GB transfer (which keeps producing chunks) is never falsely
/// aborted. Mirrors `reqwest`'s per-read idle bound via an explicit timeout we
/// control so it composes with the cancel-notify `select!`.
const DOWNLOAD_READ_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// The shared `reqwest::Client` for every semantic-search model file fetch, built
/// once and reused (mirrors the single-client policy of the AI runtime's model
/// HTTP). Bare `reqwest::get` builds a fresh default client per call with NO
/// connect timeout and NO redirect constraint — F2/F7. This client adds:
///   - a **connect timeout** (`DOWNLOAD_CONNECT_TIMEOUT`) so a wedged CDN that
///     never finishes the handshake errors instead of hanging;
///   - a **read timeout** (`DOWNLOAD_READ_IDLE_TIMEOUT`) as a backstop idle bound
///     beneath the explicit per-`next()` timeout in the receive loop;
///   - an **HF-only redirect allowlist** (`hf_redirect_policy`) so the default
///     "follow up to 10 redirects to ANY host" can never bounce an unpinned (or
///     even pinned) large file to an arbitrary CDN. HF `resolve/` 30x-redirects to
///     its own LFS CDN; anything off the allowlist is refused.
fn download_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(DOWNLOAD_CONNECT_TIMEOUT)
            .read_timeout(DOWNLOAD_READ_IDLE_TIMEOUT)
            .redirect(hf_redirect_policy())
            .build()
            // A client build failure is a programmer error (the rustls backend is
            // always compiled in); fall back to the crate default rather than
            // panicking the download task.
            .unwrap_or_default()
    })
}

/// Whether `host` is on the HuggingFace download allowlist: `huggingface.co`,
/// `hf.co`, or any `*.hf.co` subdomain (HF's LFS CDN resolves under `*.hf.co` /
/// `cdn-lfs*.hf.co`). The match is exact-or-suffix on a dot boundary so
/// `evilhf.co` or `huggingface.co.attacker.test` never pass.
fn is_allowlisted_hf_host(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == "huggingface.co"
        || host == "hf.co"
        || host.ends_with(".hf.co")
        || host.ends_with(".huggingface.co")
}

/// Redirect policy constraining every hop to the HF allowlist (F7). The default
/// policy follows up to 10 redirects to ANY host, so an HF `resolve/` 30x could
/// land bytes from an un-allowlisted CDN that the integrity gate never sees for an
/// unpinned file. This refuses any hop whose host is off the allowlist, and caps
/// the chain length, so a redirect loop or an off-host bounce fails the download
/// instead of silently fetching foreign bytes.
fn hf_redirect_policy() -> reqwest::redirect::Policy {
    const MAX_HF_REDIRECTS: usize = 10;
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() > MAX_HF_REDIRECTS {
            return attempt.error(std::io::Error::other(
                "semantic search model download exceeded the redirect limit",
            ));
        }
        // Resolve the target host to an owned decision BEFORE consuming `attempt`:
        // `attempt.url()` borrows `attempt`, and both `follow()`/`error()` move it.
        let host = attempt.url().host_str().map(str::to_owned);
        match host.as_deref() {
            Some(host) if is_allowlisted_hf_host(host) => attempt.follow(),
            other => attempt.error(std::io::Error::other(format!(
                "semantic search model download refused a redirect to a non-HuggingFace host: {}",
                other.unwrap_or("<no host>")
            ))),
        }
    })
}

fn build_download_plan(
    app_handle: &tauri::AppHandle,
    request: &SemanticSearchModelDownloadRequestDto,
) -> Result<DownloadPlan, ModelDownloadError> {
    // Resolve through the shared resolver (a pure catalog lookup under candle, ADR
    // 0037) so the selected model downloads with the right safetensors file
    // list/layout. An unknown id resolves to None and fails as ModelNotFound below.
    let descriptor = descriptor_for(&request.provider, &request.model_id).ok_or_else(|| {
        ModelDownloadError::ModelNotFound {
            provider: request.provider.clone(),
            model_id: request.model_id.clone(),
        }
    })?;

    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|error| ModelDownloadError::CreateDir {
            path: PathBuf::from("<app_data_dir>"),
            source: std::io::Error::other(error.to_string()),
        })?;
    let models_dir = semantic_search_models_dir(app_data_dir);
    let install_dir = model_install_dir(&models_dir, &request.provider, &request.model_id)?;

    Ok(DownloadPlan {
        provider: request.provider.clone(),
        model_id: request.model_id.clone(),
        hf_repo: descriptor.hf_repo.clone(),
        hf_revision: descriptor.hf_revision.clone(),
        tier: descriptor.tier,
        weights_relative_path: descriptor.expected_layout.weights_relative_path.clone(),
        aux_weights_relative_path: descriptor.expected_layout.aux_weights_relative_path.clone(),
        install_dir,
        files: model_file_specs(&descriptor),
        total_bytes: descriptor.approx_download_bytes,
    })
}

/// Resolve a descriptor for a `{provider}/{model_id}` selection via the shared
/// catalog lookup (no synthesis under candle, ADR 0037: an unknown id is `None`).
/// Both the download plan and the install verification go through this so a model
/// installs under the same layout the picker advertised.
fn descriptor_for(provider: &str, model_id: &str) -> Option<SemanticSearchModelDescriptor> {
    resolve_descriptor(provider, model_id)
}

fn claim_model_download(
    state: &SemanticSearchModelDownloadState,
    provider: &str,
    model_id: &str,
    cancel_requested: Arc<AtomicBool>,
    cancel_notify: Arc<tokio::sync::Notify>,
) -> Result<(), ModelDownloadError> {
    let mut active = state
        .lock()
        .map_err(|_| ModelDownloadError::AlreadyRunning {
            provider: provider.to_string(),
            model_id: model_id.to_string(),
        })?;
    if let Some(existing) = active.as_ref() {
        return Err(ModelDownloadError::AlreadyRunning {
            provider: existing.provider.clone(),
            model_id: existing.model_id.clone(),
        });
    }
    *active = Some(ActiveSemanticSearchModelDownload {
        provider: provider.to_string(),
        model_id: model_id.to_string(),
        cancel_requested,
        cancel_notify,
    });
    Ok(())
}

fn clear_active_download(app_handle: &tauri::AppHandle, provider: &str, model_id: &str) {
    if let Some(state) = app_handle.try_state::<SemanticSearchModelDownloadState>() {
        if let Ok(mut active) = state.lock() {
            if active
                .as_ref()
                .is_some_and(|download| download.provider == provider && download.model_id == model_id)
            {
                *active = None;
            }
        }
    }
}

fn emit_download_progress(
    app_handle: &tauri::AppHandle,
    progress: &SemanticSearchModelDownloadProgressDto,
) {
    if let Err(error) = app_handle.emit(SEMANTIC_SEARCH_MODEL_DOWNLOAD_PROGRESS_EVENT, progress) {
        // A swallowed emit means the Settings UI never sees this status (a
        // Failed/Cancelled event in particular) — surface it in the log rather
        // than letting the download silently appear stuck.
        log_error(format!(
            "semantic search model download: failed to emit progress event ({:?}) for {}/{}: {error}",
            progress.status, progress.provider, progress.model_id
        ));
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
    let result =
        download_and_install_model(&app_handle, &plan, &cancel_requested, &cancel_notify).await;
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
                &app_handle,
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
                &app_handle,
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
                &app_handle,
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

/// Best-effort free-disk preflight for a download plan: fail fast when the models
/// volume cannot hold the plan's approximate total bytes (`plan.total_bytes`, the
/// catalog footprint — bge-m3 is ~2.27GB), so a multi-GB download does not fill the
/// disk and error mid-stream.
///
/// "Best-effort" means an inability to MEASURE never blocks the download: if the
/// free-space query itself errors (the path resolves to no existing ancestor, or
/// `fs2::available_space` fails), we log and return `Ok(())` so the download
/// proceeds. Only a *measured* shortfall fails. Uses `fs2::available_space`
/// (already a desktop dependency), which reports the space available to a
/// non-privileged process — the right bound for a user-space download.
/// The free space the disk preflight requires for a plan: the catalog estimate
/// plus 10% headroom (F19). The estimate is a lower bound (the real Content-Length
/// can be larger), so demanding `estimate * 1.1` free turns a modest underestimate
/// into a clean fail-fast instead of a mid-write disk-full error. Saturating so an
/// implausibly huge estimate never overflows.
fn required_free_with_headroom(estimate_bytes: u64) -> u64 {
    estimate_bytes.saturating_add(estimate_bytes / 10)
}

fn preflight_free_disk_space(plan: &DownloadPlan) -> Result<(), ModelDownloadError> {
    // The install dir does not exist yet; probe the nearest existing ancestor on
    // the same volume so `available_space` has a real path to stat.
    let probe_path = plan
        .install_dir
        .ancestors()
        .find(|ancestor| ancestor.exists());
    let Some(probe_path) = probe_path else {
        log_info(format!(
            "semantic search model download disk preflight skipped for {}/{}: no existing ancestor of {} to stat",
            plan.provider,
            plan.model_id,
            plan.install_dir.display()
        ));
        return Ok(());
    };
    match fs2::available_space(probe_path) {
        Ok(available_bytes) => {
            // `plan.total_bytes` is the STATIC catalog estimate (`approx_download_bytes`),
            // not the real Content-Length — an underestimate would pass preflight then
            // fill the volume mid-write (F19). Treat it as a lower bound with 10%
            // headroom so a modest underestimate still fails fast here rather than
            // erroring partway through a multi-GB write. (The in-loop overrun guard in
            // `download_file_to` is the second line of defence against the live size
            // exceeding the advertised Content-Length.)
            let needed_bytes = required_free_with_headroom(plan.total_bytes);
            if available_bytes < needed_bytes {
                return Err(ModelDownloadError::InsufficientDiskSpace {
                    provider: plan.provider.clone(),
                    model_id: plan.model_id.clone(),
                    needed_bytes,
                    available_bytes,
                });
            }
            Ok(())
        }
        Err(error) => {
            // Cannot measure — log and proceed rather than blocking the download.
            log_info(format!(
                "semantic search model download disk preflight could not read free space at {} for {}/{}: {error}; proceeding",
                probe_path.display(),
                plan.provider,
                plan.model_id
            ));
            Ok(())
        }
    }
}

async fn download_and_install_model(
    app_handle: &tauri::AppHandle,
    plan: &DownloadPlan,
    cancel_requested: &AtomicBool,
    cancel_notify: &tokio::sync::Notify,
) -> Result<u64, ModelDownloadTaskError> {
    let descriptor = descriptor_for(&plan.provider, &plan.model_id).ok_or_else(|| {
        ModelDownloadError::ModelNotFound {
            provider: plan.provider.clone(),
            model_id: plan.model_id.clone(),
        }
    })?;
    let models_dir = plan
        .install_dir
        .parent()
        .and_then(|p| p.parent())
        .map(PathBuf::from)
        .ok_or_else(|| ModelDownloadError::CreateDir {
            path: plan.install_dir.clone(),
            source: std::io::Error::other("install dir has no models root"),
        })?;

    // Free-disk preflight: bge-m3 alone is ~2.27GB, so fail fast with a clear
    // message rather than filling the volume and erroring mid-stream. Best-effort —
    // if the free-space query itself fails we log and proceed (an inability to
    // MEASURE must never block a download). The check runs against an existing
    // ancestor of the install dir (the install dir does not exist yet).
    if plan.total_bytes > 0 {
        preflight_free_disk_space(plan)?;
    }

    // A fresh install: clear any partial leftovers so a half-download never lingers.
    let _ = std::fs::remove_dir_all(&plan.install_dir);
    std::fs::create_dir_all(&plan.install_dir).map_err(|source| {
        ModelDownloadError::CreateDir {
            path: plan.install_dir.clone(),
            source,
        }
    })?;

    let mut downloaded_total = 0_u64;
    for spec in &plan.files {
        if cancel_requested.load(Ordering::SeqCst) {
            return Err(ModelDownloadTaskError::Cancelled);
        }
        let url = hf_file_url(&plan.hf_repo, &plan.hf_revision, &spec.relative_path);
        // Preserve the repo-relative path on disk. The safetensors layout keeps all
        // three files at the repo root, but joining the relative path keeps this
        // correct for any future model whose weights live in a subdirectory.
        let destination = plan.install_dir.join(&spec.relative_path);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|source| ModelDownloadError::CreateDir {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        log_info(format!(
            "semantic search model download fetching {}/{} file '{}' from {url}",
            plan.provider, plan.model_id, spec.relative_path
        ));
        let file_bytes = download_file_to(
            app_handle,
            plan,
            &url,
            &spec.relative_path,
            &destination,
            downloaded_total,
            spec.expected_sha256.is_some(),
            cancel_requested,
            cancel_notify,
        )
        .await?;
        downloaded_total = downloaded_total.saturating_add(file_bytes);

        // Integrity gate (ADR 0036 / finding L5): verify the bytes we just wrote
        // BEFORE the `.installed.json` marker, so a tampered/corrupt file fails the
        // install rather than being silently loaded by candle. A pinned hash that
        // mismatches aborts here (the partial install dir is then removed by the
        // task runner); an unpinned file installs but is recorded as "integrity
        // unverified" so the supply-chain gap is visible.
        match spec.expected_sha256.as_deref() {
            Some(_) => {
                verify_file_checksum(&destination, &spec.relative_path, spec.expected_sha256.as_deref())?;
                log_info(format!(
                    "semantic search model download verified '{}' against pinned SHA256 for {}/{}",
                    spec.relative_path, plan.provider, plan.model_id
                ));
            }
            None => {
                // Not an error — just an honest signal that no digest is pinned for
                // this file (every Custom pick, and any guided-tier file not yet
                // sourced). The model still installs.
                log_info(format!(
                    "semantic search model download integrity UNVERIFIED for '{}' ({}/{}): no pinned SHA256 — trusting TLS only",
                    spec.relative_path, plan.provider, plan.model_id
                ));
            }
        }

        log_info(format!(
            "semantic search model download saved '{}' ({file_bytes} bytes) for {}/{}",
            spec.relative_path, plan.provider, plan.model_id
        ));
    }

    // Verify every required file landed before claiming Installed. This covers all
    // three safetensors-layout files (`model.safetensors`, `config.json`,
    // `tokenizer.json`), so a model missing its weights is never marked Installed.
    let missing: Vec<String> = descriptor
        .expected_layout
        .required_files
        .iter()
        .filter(|file| !plan.install_dir.join(file).is_file())
        .cloned()
        .collect();
    if !missing.is_empty() {
        log_error(format!(
            "semantic search model download incomplete for {}/{}: missing required files {missing:?}",
            plan.provider, plan.model_id
        ));
        return Err(ModelDownloadError::IncompleteLayout(missing).into());
    }

    // Fail-closed install policy (B2): a **guided tier** (English / Multilingual)
    // MUST have verified its weights file against a pinned digest before we mark it
    // Installed. The per-file loop above already ran `verify_file_checksum` and a
    // mismatch aborted; here we additionally refuse to finalize if the guided tier's
    // weights have no pinned digest at all (so a guided tier can never install on the
    // unverified "trust TLS only" path). Custom/unpinned files keep that path — they
    // install with the "integrity unverified" log, trusting the revision pin + TLS.
    guard_guided_tier_weights_pinned(plan)?;

    emit_download_progress(
        app_handle,
        &SemanticSearchModelDownloadProgressDto::new(
            &plan.provider,
            &plan.model_id,
            SemanticSearchModelDownloadStatusDto::Installing,
            downloaded_total,
            Some(plan.total_bytes.max(downloaded_total)),
            Some("finalizing".to_string()),
        ),
    );

    // The marker is written last (and only on a complete layout), so the detector
    // never reports a partial download as Installed.
    write_installed_marker(&models_dir, &plan.provider, &plan.model_id)
        .map_err(ModelDownloadError::Status)?;
    // Return the real accumulated byte total so the Completed event reports what was
    // actually downloaded, not the approximate catalog footprint.
    Ok(downloaded_total)
}

#[allow(clippy::too_many_arguments)]
async fn download_file_to(
    app_handle: &tauri::AppHandle,
    plan: &DownloadPlan,
    url: &str,
    relative_path: &str,
    destination: &Path,
    already_downloaded_bytes: u64,
    has_pinned_sha256: bool,
    cancel_requested: &AtomicBool,
    cancel_notify: &tokio::sync::Notify,
) -> Result<u64, ModelDownloadTaskError> {
    // This HTTPS fetch runs over rustls: with fastembed/ort's `native-tls` gone the
    // desktop `reqwest` is built with `rustls-tls` (no default-tls), aligning the
    // downloader with the workspace's other TLS users — sqlx's
    // `runtime-tokio-rustls` (ADR 0037). The shared `download_client` adds a connect
    // timeout, a read-idle timeout, and an HF-only redirect allowlist (F2/F7) — a
    // bare `reqwest::get` had none of these, so a wedged CDN could hang the request
    // forever (holding the single download slot) and a redirect could fetch
    // un-allowlisted bytes the integrity gate never sees.
    let response = download_client().get(url).send().await.map_err(|error| {
        log_error(format!(
            "semantic search model download HTTP request failed for {}/{} at {url}: {error}",
            plan.provider, plan.model_id
        ));
        ModelDownloadError::Http(error)
    })?;
    let status = response.status();
    if !status.is_success() {
        // Non-2xx (404 missing file, 401/403 gated repo, 5xx) — log the status
        // before turning it into the typed error so the failure is never silent.
        log_error(format!(
            "semantic search model download got HTTP {status} for {}/{} at {url}",
            plan.provider, plan.model_id
        ));
    }
    let response = response.error_for_status().map_err(ModelDownloadError::Http)?;
    // Prefer the live content-length sum (the catalog size is only approximate).
    let content_length = response.content_length();
    let total_hint = match content_length {
        Some(len) => Some(already_downloaded_bytes.saturating_add(len).max(plan.total_bytes)),
        None => Some(plan.total_bytes),
    };
    let mut stream = response.bytes_stream();
    let mut output =
        std::fs::File::create(destination).map_err(|source| ModelDownloadError::CreateFile {
            path: destination.to_path_buf(),
            source,
        })?;
    let mut file_downloaded = 0_u64;
    loop {
        // Cancel-first: a Cancel requested between chunks is honoured immediately,
        // even if the next chunk never arrives (a stall would otherwise leave the
        // per-chunk check unreached). The notify wakes the `select!` below the
        // instant Cancel runs.
        if cancel_requested.load(Ordering::SeqCst) {
            return Err(ModelDownloadTaskError::Cancelled);
        }
        // Wait for the next chunk OR a cancel OR an idle timeout, whichever happens
        // first. Wrapping `stream.next()` in `tokio::time::timeout` bounds the gap
        // between chunks: a half-open TCP / wedged CDN that stops sending body bytes
        // makes `next()` block forever, so without this the per-chunk cancel branch
        // is never reached and the single download slot is held for the process
        // lifetime (every later download returns AlreadyRunning). The timeout resets
        // on each chunk, so a live-but-slow multi-GB transfer is never falsely
        // aborted; only a genuinely stalled body errors and releases the slot. Racing
        // it against `cancel_notify.notified()` (via `futures_util::future::select`,
        // since the desktop `tokio` is built without the `macros` feature so
        // `tokio::select!` is unavailable) makes Cancel work even with zero inbound
        // bytes — the notify resolves first and the future returns immediately.
        let timed_next = std::pin::pin!(tokio::time::timeout(
            DOWNLOAD_READ_IDLE_TIMEOUT,
            stream.next()
        ));
        let cancelled = std::pin::pin!(cancel_notify.notified());
        let chunk = match futures_util::future::select(timed_next, cancelled).await {
            // Cancel won the race (notify resolved before the next chunk / timeout).
            futures_util::future::Either::Right(_) => {
                return Err(ModelDownloadTaskError::Cancelled);
            }
            // The stream ended: the body is fully received.
            futures_util::future::Either::Left((Ok(None), _)) => break,
            futures_util::future::Either::Left((Ok(Some(chunk)), _)) => {
                chunk.map_err(ModelDownloadError::Http)?
            }
            // No chunk arrived within the idle window: a stalled body. Abort so the
            // task returns and the slot is released (the partial install dir is then
            // removed by the task runner).
            futures_util::future::Either::Left((Err(_elapsed), _)) => {
                log_error(format!(
                    "semantic search model download stalled for {}/{} at {url}: no bytes for {}s after {file_downloaded} byte(s)",
                    plan.provider,
                    plan.model_id,
                    DOWNLOAD_READ_IDLE_TIMEOUT.as_secs()
                ));
                return Err(ModelDownloadError::Stalled {
                    relative_path: relative_path.to_string(),
                    idle_secs: DOWNLOAD_READ_IDLE_TIMEOUT.as_secs(),
                }
                .into());
            }
        };
        std::io::Write::write_all(&mut output, &chunk).map_err(|source| {
            ModelDownloadError::WriteFile {
                path: destination.to_path_buf(),
                source,
            }
        })?;
        file_downloaded = file_downloaded.saturating_add(chunk.len() as u64);
        // Abort as soon as the written bytes exceed the advertised Content-Length
        // (F19): a server that under-reports its length then keeps streaming would
        // otherwise fill the volume past the size the preflight budgeted for. Failing
        // here removes the partial install instead of writing to disk-full.
        if let Some(expected_len) = content_length {
            if file_downloaded > expected_len {
                log_error(format!(
                    "semantic search model download overran the advertised length for {}/{} at {url}: received {file_downloaded} of {expected_len} advertised bytes",
                    plan.provider, plan.model_id
                ));
                return Err(ModelDownloadError::ContentLengthOverrun {
                    relative_path: relative_path.to_string(),
                    advertised: expected_len,
                    received: file_downloaded,
                }
                .into());
            }
        }
        // Real per-chunk byte progress (not a CLI bool) on every chunk.
        emit_download_progress(
            app_handle,
            &SemanticSearchModelDownloadProgressDto::new(
                &plan.provider,
                &plan.model_id,
                SemanticSearchModelDownloadStatusDto::Downloading,
                already_downloaded_bytes.saturating_add(file_downloaded),
                total_hint,
                destination
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned()),
            ),
        );
    }

    // Truncation guard (finding CT3): if the server advertised a Content-Length,
    // the fully-written byte count MUST equal it. A short stream (dropped
    // connection mid-download) otherwise produces a truncated file that passes the
    // is_file() existence check and gets marked Installed — then fails to load as
    // corrupt safetensors. Failing here removes the partial install instead.
    if let Some(expected_len) = content_length {
        if file_downloaded != expected_len {
            log_error(format!(
                "semantic search model download truncated for {}/{} at {url}: wrote {file_downloaded} of {expected_len} advertised bytes",
                plan.provider, plan.model_id
            ));
            return Err(ModelDownloadError::ContentLengthMismatch {
                relative_path: relative_path.to_string(),
                expected: expected_len,
                actual: file_downloaded,
            }
            .into());
        }
    } else if !has_pinned_sha256 {
        // No Content-Length to bound truncation against AND no pinned SHA256 to catch
        // a corrupt/short body (F8): this file is otherwise unguarded. Previously
        // only a 0-byte stream was rejected, so an unpinned file served with no
        // length whose stream dropped after some non-zero bytes slipped through,
        // passed the is_file() check, was marked Installed, then failed as corrupt
        // safetensors at embed time with no self-heal. We now fail CLOSED: an
        // unpinned large file MUST advertise a Content-Length so we can verify the
        // written size; absent one we refuse the install rather than trust an
        // unverifiable, unboundable body. (A file WITH a pinned digest stays exempt —
        // the post-download checksum fully protects it, length or not.)
        log_error(format!(
            "semantic search model download refused for {}/{} at {url}: {file_downloaded} byte(s) received with no Content-Length and no pinned SHA256 — cannot verify the body is complete (fail-closed)",
            plan.provider, plan.model_id
        ));
        if file_downloaded == 0 {
            // Keep the precise "empty download" signal for the zero-byte case.
            return Err(ModelDownloadError::EmptyDownload {
                relative_path: relative_path.to_string(),
            }
            .into());
        }
        return Err(ModelDownloadError::MissingContentLength {
            relative_path: relative_path.to_string(),
        }
        .into());
    }

    Ok(file_downloaded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use capture_types::default_semantic_search_settings;
    use semantic_search::{find_model_descriptor, selected_semantic_search_model_available};

    #[test]
    fn status_response_lists_catalog_under_app_data_models_dir() {
        let temp = tempfile::tempdir().expect("tempdir");
        let response =
            build_semantic_search_model_status_response(temp.path(), None).expect("status response");
        assert!(response.models_directory.ends_with("semantic_search_models"));
        assert!(response
            .models
            .iter()
            .any(|model| model.model_id == "nomic-embed-text-v1.5"
                && model.tier == SemanticSearchModelTier::English
                && model.dimension == 768));
        // No model installed yet => everything is Missing / unavailable.
        assert!(response.models.iter().all(|model| !model.available));
        // With no selection, exactly the 3 guided manifest tiers are listed.
        assert_eq!(response.models.len(), builtin_model_manifest().models.len());
    }

    /// The curated `Custom`-tier catalog model, surfaced via the Custom picker
    /// (ADR 0037: the open "any model outside the manifest" picker is gone, so the
    /// Custom tier is now an explicit catalog entry — `bge-m3`).
    fn custom_tier_descriptor() -> SemanticSearchModelDescriptor {
        builtin_model_manifest()
            .models
            .into_iter()
            .find(|model| model.tier == SemanticSearchModelTier::Custom)
            .expect("a Custom-tier catalog model")
    }

    #[test]
    fn status_response_surfaces_the_selected_custom_tier_model() {
        // The curated Custom-tier model (bge-m3) appears in the status response when
        // it is the persisted selection, so the Settings UI can show its
        // installed/selected state. It is an explicit catalog entry now, so it is
        // already in the manifest list (no extra appended row) — the selected-append
        // guard must not duplicate it.
        let custom = custom_tier_descriptor();

        let temp = tempfile::tempdir().expect("tempdir");
        let response =
            build_semantic_search_model_status_response(temp.path(), Some(&custom.model_id))
                .expect("status response");

        // The Custom-tier model is one of the 3 guided manifest tiers, so the count
        // is unchanged (no appended row for an in-catalog selection).
        assert_eq!(response.models.len(), builtin_model_manifest().models.len());
        let custom_row = response
            .models
            .iter()
            .find(|model| model.model_id == custom.model_id)
            .expect("selected custom-tier model must be listed");
        assert_eq!(custom_row.tier, SemanticSearchModelTier::Custom);
        assert_eq!(custom_row.model_code, custom.hf_repo);
        // Not installed on disk => Missing / unavailable, but still surfaced.
        assert!(!custom_row.available);
    }

    /// Install a catalog model on disk: every required file from its resolved
    /// safetensors layout plus the `.installed.json` marker, so
    /// `detect_model_status` reports it Installed.
    fn install_custom_model_on_disk(models_dir: &Path, descriptor: &SemanticSearchModelDescriptor) {
        let install_dir =
            model_install_dir(models_dir, &descriptor.provider, &descriptor.model_id)
                .expect("install dir");
        std::fs::create_dir_all(&install_dir).expect("install dir");
        for file_name in &descriptor.expected_layout.required_files {
            let path = install_dir.join(file_name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("parent dir");
            }
            std::fs::write(path, b"x").expect("model file");
        }
        write_installed_marker(models_dir, &descriptor.provider, &descriptor.model_id)
            .expect("marker");
    }

    #[test]
    fn status_response_surfaces_a_downloaded_but_unselected_custom_model_as_available() {
        // H3 regression: a Custom-tier model downloaded on disk but NOT yet the
        // persisted selection must appear in the status response with a real
        // `available` true, so the Settings UI can offer "Use this model". Without
        // this it was a dead end — downloadable, never activatable.
        let descriptor = custom_tier_descriptor();

        let temp = tempfile::tempdir().expect("tempdir");
        let models_dir = semantic_search_models_dir(temp.path());
        install_custom_model_on_disk(&models_dir, &descriptor);

        // No selection at all — the model is merely downloaded.
        let response = build_semantic_search_model_status_response(temp.path(), None)
            .expect("status response");
        let custom_row = response
            .models
            .iter()
            .find(|model| model.model_id == descriptor.model_id)
            .expect("downloaded custom-tier model must be listed even when unselected");
        assert_eq!(custom_row.tier, SemanticSearchModelTier::Custom);
        assert!(
            custom_row.available,
            "a fully-downloaded custom-tier model must report available so it can be activated"
        );
        // The other guided tiers are unaffected (still listed, all unavailable on disk).
        assert!(response
            .models
            .iter()
            .any(|model| model.model_id == "nomic-embed-text-v1.5" && !model.available));
    }

    #[test]
    fn status_response_does_not_duplicate_a_downloaded_custom_model() {
        // A downloaded Custom-tier model that is ALSO the selection must appear
        // exactly once (the manifest list covers it; neither the disk scan nor the
        // selected-append may re-add it).
        let descriptor = custom_tier_descriptor();

        let temp = tempfile::tempdir().expect("tempdir");
        let models_dir = semantic_search_models_dir(temp.path());
        install_custom_model_on_disk(&models_dir, &descriptor);

        let response =
            build_semantic_search_model_status_response(temp.path(), Some(&descriptor.model_id))
                .expect("status response");
        assert_eq!(
            response
                .models
                .iter()
                .filter(|model| model.model_id == descriptor.model_id)
                .count(),
            1,
            "a downloaded + selected custom-tier model must appear exactly once"
        );
    }

    #[test]
    fn status_response_does_not_duplicate_a_selected_manifest_model() {
        // A guided-tier selection is already in the manifest list and must not be
        // appended a second time.
        let temp = tempfile::tempdir().expect("tempdir");
        let response = build_semantic_search_model_status_response(
            temp.path(),
            Some("nomic-embed-text-v1.5"),
        )
        .expect("status response");
        assert_eq!(response.models.len(), builtin_model_manifest().models.len());
        assert_eq!(
            response
                .models
                .iter()
                .filter(|model| model.model_id == "nomic-embed-text-v1.5")
                .count(),
            1
        );
    }

    #[test]
    fn gating_wrapper_flips_once_the_selected_model_is_installed() {
        let temp = tempfile::tempdir().expect("tempdir");
        let settings = default_semantic_search_settings();
        assert!(!selected_semantic_search_model_available(temp.path(), &settings).expect("gating check"));

        // Install the selected model on disk: every required file + marker.
        let models_dir = semantic_search_models_dir(temp.path());
        let manifest = builtin_model_manifest();
        let descriptor = manifest
            .models
            .iter()
            .find(|d| d.model_id == settings.model_id.clone().unwrap())
            .expect("selected descriptor");
        let install_dir =
            model_install_dir(&models_dir, SEMANTIC_SEARCH_PROVIDER_ID, &descriptor.model_id)
                .expect("install dir");
        std::fs::create_dir_all(&install_dir).expect("install dir");
        for file_name in &descriptor.expected_layout.required_files {
            let path = install_dir.join(file_name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("parent dir");
            }
            std::fs::write(path, b"x").expect("model file");
        }
        // Write the marker at the CURRENT manifest version. After the candle
        // cutover (MANIFEST_VERSION 2) an older `manifestVersion: 1` ONNX-shaped
        // marker no longer validates — that staleness is what forces the
        // re-download into the safetensors layout — so a fresh install must stamp
        // the live version to be recognized as Installed.
        write_installed_marker(&models_dir, SEMANTIC_SEARCH_PROVIDER_ID, &descriptor.model_id)
            .expect("marker");

        assert!(selected_semantic_search_model_available(temp.path(), &settings).expect("gating check"));
    }

    #[test]
    fn file_specs_cover_every_required_layout_file() {
        // The download spec list must cover every file the detector requires for that
        // model — the three safetensors-layout files — so a model never installs
        // missing its weights. Both views read `expected_layout.required_files`.
        let manifest = builtin_model_manifest();
        for descriptor in &manifest.models {
            let specs = model_file_specs(descriptor);
            let spec_paths: Vec<&str> = specs.iter().map(|s| s.relative_path.as_str()).collect();
            for required in &descriptor.expected_layout.required_files {
                assert!(
                    spec_paths.contains(&required.as_str()),
                    "model {} is missing a download spec for required file {required}",
                    descriptor.model_id
                );
            }
        }
    }

    #[test]
    fn download_plan_covers_each_models_weights_layout() {
        // ADR 0037: every catalog model downloads its weights file plus the two
        // repo-root JSON files (`config.json`, `tokenizer.json`). The catalog no
        // longer downloads `model.safetensors` for every model — nomic/e5 do, but
        // bge-m3's repo ships no safetensors, so it downloads `pytorch_model.bin`
        // instead (still no ONNX siblings). Each plan must therefore carry ITS OWN
        // weights file. The old ONNX graph + external-data siblings (e.g. bge-m3's
        // `onnx/model.onnx_data`) are gone, so they must NOT appear in the plan.
        let manifest = builtin_model_manifest();
        for descriptor in &manifest.models {
            let specs = model_file_specs(descriptor);
            let paths: Vec<&str> = specs.iter().map(|s| s.relative_path.as_str()).collect();
            assert!(
                paths.contains(&descriptor.expected_layout.weights_relative_path.as_str()),
                "{}",
                descriptor.model_id
            );
            assert!(paths.contains(&"config.json"), "{}", descriptor.model_id);
            assert!(paths.contains(&"tokenizer.json"), "{}", descriptor.model_id);
            assert!(
                paths.iter().all(|p| !p.contains("onnx")),
                "{} must not carry any ONNX file",
                descriptor.model_id
            );
        }
    }

    #[test]
    fn supported_models_list_is_the_curated_catalog() {
        let models = list_semantic_search_supported_models().expect("supported models");
        assert!(!models.is_empty(), "the curated catalog must enumerate models");
        // The open "any ONNX model" picker is gone (ADR 0037); the list is exactly
        // the curated catalog, so gated/arbitrary repos cannot appear.
        assert!(
            models.iter().all(|m| !m.model_code.to_ascii_lowercase().contains("gemma")),
            "gated EmbeddingGemma must never appear in the curated list"
        );
        // e5-small carries its catalog slug, hf_repo, dimension, and multilingual flag.
        let e5 = models
            .iter()
            .find(|m| m.model_id == "multilingual-e5-small")
            .expect("e5-small must be in the curated catalog");
        assert_eq!(e5.model_code, "intfloat/multilingual-e5-small");
        assert!(e5.multilingual);
        assert_eq!(e5.dimension, 384);
    }

    #[test]
    fn hf_urls_point_at_the_model_repo_pinned_revision() {
        // B2: downloads pin the immutable commit SHA (`hf_revision`), not the mutable
        // `main` branch — so an upstream force-push can never swap the bytes.
        let manifest = builtin_model_manifest();
        let nomic = find_model_descriptor(&manifest, SEMANTIC_SEARCH_PROVIDER_ID, "nomic-embed-text-v1.5")
            .expect("nomic descriptor");
        let weights_url = hf_file_url(&nomic.hf_repo, &nomic.hf_revision, "model.safetensors");
        assert_eq!(
            weights_url,
            "https://huggingface.co/nomic-ai/nomic-embed-text-v1.5/resolve/e9b6763023c676ca8431644204f50c2b100d9aab/model.safetensors"
        );
        // The URL must carry the pinned revision, never the mutable `main` branch.
        assert!(!weights_url.contains("/resolve/main/"));
        let tokenizer_url = hf_file_url(&nomic.hf_repo, &nomic.hf_revision, "tokenizer.json");
        assert!(tokenizer_url.ends_with(&format!("/resolve/{}/tokenizer.json", nomic.hf_revision)));
    }

    #[test]
    fn only_one_download_can_be_claimed_at_a_time() {
        let state = SemanticSearchModelDownloadState::default();
        claim_model_download(
            &state,
            SEMANTIC_SEARCH_PROVIDER_ID,
            "nomic-embed-text-v1.5",
            Arc::new(AtomicBool::new(false)),
            Arc::new(tokio::sync::Notify::new()),
        )
        .expect("first claim");
        let second = claim_model_download(
            &state,
            SEMANTIC_SEARCH_PROVIDER_ID,
            "multilingual-e5-small",
            Arc::new(AtomicBool::new(false)),
            Arc::new(tokio::sync::Notify::new()),
        )
        .expect_err("second claim should fail while one is active");
        assert!(matches!(second, ModelDownloadError::AlreadyRunning { .. }));
        assert_eq!(
            state.lock().expect("state").as_ref().expect("active").model_id,
            "nomic-embed-text-v1.5"
        );
    }

    #[test]
    fn status_response_exposes_model_code_and_disk_cost() {
        let temp = tempfile::tempdir().expect("tempdir");
        let response =
            build_semantic_search_model_status_response(temp.path(), None).expect("status response");
        let nomic = response
            .models
            .iter()
            .find(|m| m.model_id == "nomic-embed-text-v1.5")
            .expect("nomic model");
        assert_eq!(nomic.model_code, "nomic-ai/nomic-embed-text-v1.5");
        assert!(nomic.approx_download_bytes > 0);
    }

    #[test]
    fn semantic_search_checksum_mismatch_fails_before_install_completes() {
        // L5: a file whose bytes do not match a pinned SHA256 must fail verification
        // (the install must NOT complete / the marker must not be written). We hash
        // a file on disk against a deliberately-wrong pinned digest and assert the
        // verifier returns ChecksumMismatch.
        let temp = tempfile::tempdir().expect("tempdir");
        let file_path = temp.path().join("model.safetensors");
        std::fs::write(&file_path, b"the wrong bytes").expect("write file");

        // The matching path: hashing then pinning the real digest verifies cleanly.
        let real_sha256 = sha256_of_file(&file_path).expect("sha256");
        verify_file_checksum(&file_path, "model.safetensors", Some(&real_sha256))
            .expect("a matching pinned hash verifies");

        // The mismatch path: an unrelated pinned hash fails the install.
        let error = verify_file_checksum(&file_path, "model.safetensors", Some(&"0".repeat(64)))
            .expect_err("a mismatched pinned hash must fail verification");
        match error {
            ModelDownloadError::ChecksumMismatch {
                relative_path,
                actual,
                ..
            } => {
                assert_eq!(relative_path, "model.safetensors");
                assert_eq!(actual, real_sha256);
            }
            other => panic!("expected ChecksumMismatch, got {other:?}"),
        }
    }

    #[test]
    fn semantic_search_unpinned_file_installs_but_is_flagged_unverified() {
        // L5: a file with NO pinned hash (every Custom pick, and any guided-tier
        // file not yet sourced) is a no-op success — it installs, integrity
        // unverified. `verify_file_checksum(None)` must return Ok without touching
        // the bytes.
        let temp = tempfile::tempdir().expect("tempdir");
        let file_path = temp.path().join("model.safetensors");
        std::fs::write(&file_path, b"unpinned but trusted").expect("write file");

        verify_file_checksum(&file_path, "model.safetensors", None)
            .expect("an unpinned file is accepted (integrity unverified)");
        // An empty pinned string is treated as unpinned too (mirrors speaker-analysis).
        verify_file_checksum(&file_path, "model.safetensors", Some("  "))
            .expect("a blank pinned hash is treated as unpinned");

        // B2: the two guided tiers (nomic, e5) carry a real pinned weights digest —
        // fail-closed. bge-m3 (Custom) ships only `pytorch_model.bin`, and that
        // weights file is now pinned too (fail-closed) — even though the Custom tier
        // is exempt from the MUST-pin gate, bge-m3 happens to satisfy it. The unpinned
        // no-op path above still exists generically for any not-yet-sourced file.
        let manifest = builtin_model_manifest();
        for descriptor in &manifest.models {
            let weights = &descriptor.expected_layout.weights_relative_path;
            let weights_spec = model_file_specs(descriptor)
                .into_iter()
                .find(|spec| &spec.relative_path == weights)
                .expect("a weights file spec");
            match descriptor.tier {
                SemanticSearchModelTier::English | SemanticSearchModelTier::Multilingual => assert!(
                    weights_spec.expected_sha256.is_some(),
                    "guided-tier {} must pin its weights digest (fail-closed)",
                    descriptor.model_id
                ),
                // Custom is EXEMPT from the MUST-pin gate, but bge-m3's
                // `pytorch_model.bin` is in fact pinned, so its digest is present.
                SemanticSearchModelTier::Custom => assert!(
                    weights_spec.expected_sha256.is_some(),
                    "custom-tier {} pins its `pytorch_model.bin` weights digest (bge-m3 is exempt but happens to be pinned)",
                    descriptor.model_id
                ),
            }
        }
    }

    #[test]
    fn pinned_hash_lookup_resolves_when_a_constant_is_present() {
        // Guards the verification plumbing: `pinned_file_sha256` returns the real
        // pinned LFS digest for a known (hf_repo, path) and None for an unpinned one.
        let nomic = pinned_file_sha256("nomic-ai/nomic-embed-text-v1.5", "model.safetensors")
            .expect("nomic weights are pinned (fail-closed)");
        assert_eq!(
            nomic,
            "9e7d262b1fe5ea350782829496efa831901b77486bbde1cea54a4c822d010d5c"
        );
        let e5 = pinned_file_sha256("intfloat/multilingual-e5-small", "model.safetensors")
            .expect("e5 weights are pinned (fail-closed)");
        assert_eq!(
            e5,
            "1a55775f53449dac10a2bcbc312469fac40b96d53198c407081a831f81c98477"
        );
        // bge-m3 (Custom) ships no `model.safetensors` — its weights live in
        // `pytorch_model.bin`, which IS pinned. Looking up the nonexistent
        // `model.safetensors` is what returns `None`.
        assert!(
            pinned_file_sha256("BAAI/bge-m3", "model.safetensors").is_none(),
            "bge-m3 has no `model.safetensors` (its weights are `pytorch_model.bin`)"
        );
        let bge_m3 = pinned_file_sha256("BAAI/bge-m3", "pytorch_model.bin")
            .expect("bge-m3 weights live in `pytorch_model.bin` and are pinned");
        assert_eq!(
            bge_m3,
            "b5e0ce3470abf5ef3831aa1bd5553b486803e83251590ab7ff35a117cf6aad38"
        );
        assert!(
            pinned_file_sha256("some/unknown-model", "model.safetensors").is_none(),
            "an unknown hf_repo (not in the catalog) is never pinned"
        );
    }

    /// B1 regression: startup reconciliation resolves the expected vec0 dimension
    /// from `model_id` IGNORING `enabled`, so a disabled-but-previously-selected
    /// non-768 model keeps its table instead of being wiped back to `float[768]`.
    #[test]
    fn reconcile_dimension_ignores_enabled_and_keys_off_model_id() {
        // A user on the Multilingual tier (e5-small, 384-dim) who toggled the feature
        // OFF: `model_id` is still persisted (disabling never clears it), so the
        // expected dimension must stay 384 — NOT fall back to the 768 default that
        // would DROP+recreate their vector index.
        let disabled_e5 = capture_types::SemanticSearchSettings {
            enabled: false,
            provider: SEMANTIC_SEARCH_PROVIDER_ID.to_string(),
            model_id: Some("multilingual-e5-small".to_string()),
        };
        assert_eq!(
            reconcile_expected_dimension(&disabled_e5),
            Some(384),
            "a disabled non-768 model must keep its table dimension (B1: never wipe)"
        );

        // Only a genuinely never-selected profile (model_id == None) falls back to
        // the migration default 768, so a fresh DB stays at `float[768]`.
        let never_selected = capture_types::SemanticSearchSettings {
            enabled: false,
            provider: SEMANTIC_SEARCH_PROVIDER_ID.to_string(),
            model_id: None,
        };
        assert_eq!(
            reconcile_expected_dimension(&never_selected),
            Some(DEFAULT_SEMANTIC_SEARCH_DIMENSION),
            "no model ever selected => the migration default 768"
        );
        assert_eq!(DEFAULT_SEMANTIC_SEARCH_DIMENSION, 768);

        // An enabled non-768 model resolves the same way (enabled is irrelevant here).
        let enabled_bge = capture_types::SemanticSearchSettings {
            enabled: true,
            provider: SEMANTIC_SEARCH_PROVIDER_ID.to_string(),
            model_id: Some("bge-m3".to_string()),
        };
        assert_eq!(reconcile_expected_dimension(&enabled_bge), Some(1024));

        // FIX 4: a persisted model_id that no longer RESOLVES (catalog/config
        // drift) must return None so the caller SKIPS reconciliation and leaves the
        // populated table untouched — never the 768 fallback that would DROP it.
        let unresolvable = capture_types::SemanticSearchSettings {
            enabled: true,
            provider: SEMANTIC_SEARCH_PROVIDER_ID.to_string(),
            model_id: Some("a-model-that-no-longer-exists".to_string()),
        };
        assert_eq!(
            reconcile_expected_dimension(&unresolvable),
            None,
            "an unresolvable model_id must skip reconciliation, not wipe to 768"
        );
    }

    /// B2: the guided-tier fail-closed gate refuses to finalize a guided tier whose
    /// weights have no pinned digest, while leaving the Custom tier on the unpinned
    /// path.
    #[test]
    fn guided_tier_gate_requires_a_pinned_weights_digest() {
        fn plan_for(
            hf_repo: &str,
            tier: SemanticSearchModelTier,
            weights_relative_path: &str,
        ) -> DownloadPlan {
            DownloadPlan {
                provider: SEMANTIC_SEARCH_PROVIDER_ID.to_string(),
                model_id: "m".to_string(),
                hf_repo: hf_repo.to_string(),
                hf_revision: "rev".to_string(),
                tier,
                weights_relative_path: weights_relative_path.to_string(),
                aux_weights_relative_path: None,
                install_dir: PathBuf::from("/tmp/x"),
                files: Vec::new(),
                total_bytes: 0,
            }
        }

        // Guided tiers with real pinned weights pass the gate.
        guard_guided_tier_weights_pinned(&plan_for(
            "nomic-ai/nomic-embed-text-v1.5",
            SemanticSearchModelTier::English,
            "model.safetensors",
        ))
        .expect("nomic weights are pinned => gate passes");
        guard_guided_tier_weights_pinned(&plan_for(
            "intfloat/multilingual-e5-small",
            SemanticSearchModelTier::Multilingual,
            "model.safetensors",
        ))
        .expect("e5 weights are pinned => gate passes");

        // A guided tier whose weights are NOT pinned is refused (fail-closed).
        let error = guard_guided_tier_weights_pinned(&plan_for(
            "BAAI/bge-m3",
            SemanticSearchModelTier::English,
            "model.safetensors",
        ))
        .expect_err("an unpinned guided tier must be refused");
        assert!(matches!(
            error,
            ModelDownloadError::GuidedTierWeightsUnpinned { .. }
        ));

        // The Custom tier is exempt: it installs on the unpinned path even though its
        // weights carry no pinned digest.
        guard_guided_tier_weights_pinned(&plan_for(
            "BAAI/bge-m3",
            SemanticSearchModelTier::Custom,
            "model.safetensors",
        ))
        .expect("custom tier is exempt from the fail-closed gate");
    }

    /// F14: the guided-tier gate requires a pinned digest for EVERY required weights
    /// file — both the base backbone AND the auxiliary head when present. A guided
    /// tier with an aux head whose head is unpinned must be refused even when the
    /// base weights are pinned, so the "guided tiers can never install unverified"
    /// contract covers the whole model.
    #[test]
    fn guided_tier_gate_requires_a_pinned_aux_head_digest() {
        // A synthetic guided tier whose base weights ARE pinned but whose aux head is
        // NOT must be refused (the head would otherwise install unverified).
        let plan_unpinned_head = DownloadPlan {
            provider: SEMANTIC_SEARCH_PROVIDER_ID.to_string(),
            model_id: "m".to_string(),
            // nomic's base `model.safetensors` is pinned, so the base passes; the aux
            // head path below is NOT pinned for this repo, so the gate must fail on it.
            hf_repo: "nomic-ai/nomic-embed-text-v1.5".to_string(),
            hf_revision: "rev".to_string(),
            tier: SemanticSearchModelTier::English,
            weights_relative_path: "model.safetensors".to_string(),
            aux_weights_relative_path: Some("2_Dense_2048/model.safetensors".to_string()),
            install_dir: PathBuf::from("/tmp/x"),
            files: Vec::new(),
            total_bytes: 0,
        };
        let error = guard_guided_tier_weights_pinned(&plan_unpinned_head)
            .expect_err("a guided tier with an unpinned aux head must be refused");
        match error {
            ModelDownloadError::GuidedTierWeightsUnpinned {
                weights_relative_path,
                ..
            } => assert_eq!(weights_relative_path, "2_Dense_2048/model.safetensors"),
            other => panic!("expected GuidedTierWeightsUnpinned for the aux head, got {other:?}"),
        }
    }

    /// F7: Stella's base + dense head and Arctic's backbone now carry pinned LFS
    /// digests, so their multi-GB safetensors install fail-closed instead of on the
    /// "integrity UNVERIFIED — trusting TLS only" branch.
    #[test]
    fn stella_and_arctic_weights_are_pinned() {
        assert_eq!(
            pinned_file_sha256("NovaSearch/stella_en_400M_v5", "model.safetensors")
                .expect("Stella base weights are pinned (F7)"),
            "17e549d16172a548a3115739b55575968eb6523653daad76c46b0758e9425032"
        );
        assert_eq!(
            pinned_file_sha256(
                "NovaSearch/stella_en_400M_v5",
                "2_Dense_2048/model.safetensors"
            )
            .expect("Stella dense head is pinned (F7)"),
            "a831055e5110e81c03ed6559f4ebf5842630f227ded6b6c18826700d548b990f"
        );
        assert_eq!(
            pinned_file_sha256(
                "Snowflake/snowflake-arctic-embed-l-v2.0",
                "model.safetensors"
            )
            .expect("Arctic backbone is pinned (F7)"),
            "21bf1a120b1c6562aeec379dfa9039b0d360591c784cb1c6786e87256b738ee1"
        );

        // Every Custom-tier descriptor that loads safetensors weights must now carry a
        // pinned digest for each weights file (base + aux head), so no Custom model
        // installs its multi-GB weights on the unverified branch.
        let manifest = builtin_model_manifest();
        for descriptor in &manifest.models {
            if descriptor.tier != SemanticSearchModelTier::Custom {
                continue;
            }
            // bge-m3's weights are `pytorch_model.bin` (already pinned); the
            // safetensors-weighted Custom models (Stella, Arctic) must pin every
            // weights file.
            if descriptor
                .expected_layout
                .weights_relative_path
                .ends_with(".safetensors")
            {
                assert!(
                    pinned_file_sha256(
                        &descriptor.hf_repo,
                        &descriptor.expected_layout.weights_relative_path
                    )
                    .is_some(),
                    "custom-tier {} base weights must be pinned (F7)",
                    descriptor.model_id
                );
                if let Some(aux) = &descriptor.expected_layout.aux_weights_relative_path {
                    assert!(
                        pinned_file_sha256(&descriptor.hf_repo, aux).is_some(),
                        "custom-tier {} aux head must be pinned (F7)",
                        descriptor.model_id
                    );
                }
            }
        }
    }

    /// F7: the redirect allowlist accepts HuggingFace hosts and its LFS CDN, and
    /// refuses any off-host (or lookalike) redirect target.
    #[test]
    fn redirect_allowlist_accepts_hf_hosts_and_refuses_others() {
        assert!(is_allowlisted_hf_host("huggingface.co"));
        assert!(is_allowlisted_hf_host("hf.co"));
        assert!(is_allowlisted_hf_host("cdn-lfs.hf.co"));
        assert!(is_allowlisted_hf_host("cdn-lfs-us-1.hf.co"));
        assert!(is_allowlisted_hf_host("CDN-LFS.HF.CO")); // case-insensitive
        assert!(is_allowlisted_hf_host("cas-bridge.xethub.hf.co"));

        // Lookalikes / arbitrary CDNs are refused.
        assert!(!is_allowlisted_hf_host("evilhf.co"));
        assert!(!is_allowlisted_hf_host("huggingface.co.attacker.test"));
        assert!(!is_allowlisted_hf_host("hf.co.evil.test"));
        assert!(!is_allowlisted_hf_host("example.com"));
        assert!(!is_allowlisted_hf_host(""));
    }

    /// F19: the disk preflight treats the catalog estimate as a lower bound with 10%
    /// headroom, so a modest underestimate fails fast rather than mid-write.
    #[test]
    fn disk_preflight_requires_ten_percent_headroom() {
        assert_eq!(required_free_with_headroom(0), 0);
        assert_eq!(required_free_with_headroom(1_000), 1_100);
        // ~2.27GB bge-m3 estimate gains ~227MB of headroom.
        assert_eq!(
            required_free_with_headroom(2_270_000_000),
            2_270_000_000 + 227_000_000
        );
        // Saturating: an implausibly huge estimate never overflows.
        assert_eq!(required_free_with_headroom(u64::MAX), u64::MAX);
    }
}
