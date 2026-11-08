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
use semantic_search::{
    builtin_model_manifest, detect_model_status, list_supported_models, model_install_dir,
    resolve_descriptor, semantic_search_models_dir, write_installed_marker, ModelStatusError,
    ModelStatusKind, SemanticSearchModelDescriptor, SemanticSearchModelTier,
    SEMANTIC_SEARCH_PROVIDER_ID,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::debug_log::{log, log_error, log_info, log_warn};
use crate::{AppCore, CoreEvent};

mod download;
mod integrity;
mod select;
mod status;
mod task;
#[cfg(test)]
mod tests;

pub use download::{
    build_download_plan, claim_model_download, download_and_install_model, emit_download_progress,
};
use integrity::*;
pub use select::select_semantic_search_model;
use status::*;
pub use task::start_semantic_search_model_download;

/// The frontend event the Settings UI listens on for live download progress.
/// The bytes/percent in each payload come from the streaming HTTP download of the
/// model's safetensors files (per-chunk), not a CLI progress bool — so the UI
/// receives real programmatic byte progress (ADR 0036 / issue #125).
pub const SEMANTIC_SEARCH_MODEL_DOWNLOAD_PROGRESS_EVENT: &str =
    "semantic_search_model_download_progress";

/// Processor name a semantic-search model lock is filed under. Semantic search is not a
/// `processing_jobs` processor, so this only ever namespaces its own lock rows.
pub const SEMANTIC_SEARCH_MODEL_LOCK_PROCESSOR: &str = "semantic_search";

/// One model download may run at a time, mirroring the OCR/transcription model
/// downloaders. The cancel flag is shared with the running task.
pub type SemanticSearchModelDownloadState = Mutex<Option<ActiveSemanticSearchModelDownload>>;

#[derive(Debug, Clone)]
pub struct ActiveSemanticSearchModelDownload {
    pub provider: String,
    pub model_id: String,
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
    pub fn new(
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
pub struct ModelFileSpec {
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
pub enum ModelDownloadError {
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
    #[error(
        "downloaded file {relative_path} checksum mismatch: expected {expected}, got {actual}"
    )]
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
pub enum ModelDownloadTaskError {
    #[error("download cancelled")]
    Cancelled,
    #[error(transparent)]
    Failed(#[from] ModelDownloadError),
}

pub struct DownloadPlan {
    pub provider: String,
    pub model_id: String,
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
    pub install_dir: PathBuf,
    pub files: Vec<ModelFileSpec>,
    pub total_bytes: u64,
}

pub fn get_semantic_search_model_status(
    core: &AppCore,
) -> Result<SemanticSearchModelStatusResponseDto, String> {
    let app_data_dir = &core.paths().config;
    // Surface the currently-persisted selection even when it is a Custom model
    // outside the 3 guided tiers, so the Settings UI can show that Custom pick's
    // installed/selected state. We read the effective settings here (not an extra
    // command argument) so the frontend keeps calling this with no parameters and
    // the "selected" row stays authoritative with the persisted selection.
    let selected = crate::semantic_search_worker::effective_semantic_search_settings(core).model_id;
    build_semantic_search_model_status_response(app_data_dir, selected.as_deref())
        .map_err(|error| format!("failed to inspect semantic search models: {error}"))
}

/// List the curated candle-supported text-embedding models for the **Custom**
/// picker.
///
/// Drives off the crate's hand-coded catalog (ADR 0037) — the open "any ONNX
/// model" picker is gone, so this is exactly the curated list. The frontend Custom
/// picker consumes these to let a user pick a supported model; downloading one then
/// reuses [`start_semantic_search_model_download`] once the selection is persisted.
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

/// Cancel the in-flight **Semantic Search Model** download, if any.
pub fn cancel_semantic_search_model_download(
    download_state: &SemanticSearchModelDownloadState,
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

/// Delete an **installed but unused** Semantic Search Model from disk.
///
/// "Unused" = installed and NOT the currently-selected model. The backfill worker
/// only ever loads the *selected* model's files, so removing any other model's
/// install dir is safe with none of the cleanup-lock dance the speaker/OCR paths
/// need (those run concurrent per-model jobs; this one does not). We refuse to
/// delete the selected model (search depends on it) and refuse while that model is
/// mid-download (the writer still owns the dir). `model_install_dir` is the
/// traversal-safe resolver, so the removed path can never escape the models dir.
pub fn delete_semantic_search_model(
    core: &AppCore,
    request: SemanticSearchModelDownloadRequestDto,
    download_state: &SemanticSearchModelDownloadState,
) -> Result<(), String> {
    {
        let active = download_state
            .lock()
            .map_err(|_| "semantic search model download state poisoned".to_string())?;
        if active
            .as_ref()
            .is_some_and(|d| d.provider == request.provider && d.model_id == request.model_id)
        {
            return Err(
                "cannot delete a semantic search model while it is downloading".to_string(),
            );
        }
    }

    let selected = crate::semantic_search_worker::effective_semantic_search_settings(core).model_id;
    if selected.as_deref() == Some(request.model_id.as_str()) {
        return Err(
            "cannot delete the model that is currently in use — switch to another model first"
                .to_string(),
        );
    }

    let models_dir = semantic_search_models_dir(&core.paths().config);
    let install_dir = model_install_dir(&models_dir, &request.provider, &request.model_id)
        .map_err(|error| error.to_string())?;

    match std::fs::remove_dir_all(&install_dir) {
        Ok(()) => {
            log_info(format!(
                "semantic search model deleted: {}/{} removed from {}",
                request.provider,
                request.model_id,
                install_dir.display()
            ));
            Ok(())
        }
        // Already gone is success — the UI just wanted it off disk.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "failed to delete semantic search model {}/{}: {error}",
            request.provider, request.model_id
        )),
    }
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

pub fn select_model_lock() -> &'static tokio::sync::Mutex<()> {
    SELECT_MODEL_LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// The `(model_id, dimension)` reconciliation should expect the vec0 table to hold
/// for a settings snapshot, or `None` when reconciliation must be **skipped**
/// because the selection cannot be resolved safely — read from the persisted
/// `model_id` **regardless of the `enabled` flag**.
///
/// Reconciliation keys off the *selected model*, NOT the *active feature*: a user
/// on a non-default tier (Multilingual e5 = 384, Custom bge-m3 = 1024) who toggles
/// Semantic Search OFF still has that `model_id` persisted (disabling never clears
/// it), and their vec0 table belongs to that model, not the default. If we resolved
/// through the worker's `resolve_selected_descriptor` (which returns `None` the
/// moment `enabled == false`, BEFORE it even reads `model_id`), startup would fall
/// back to the default and reconciliation would DROP+recreate their table — wiping
/// the entire vector index and forcing a full re-embed. So we resolve straight from
/// `model_id` here, and the `enabled` flag is deliberately ignored.
///
/// Two `None`-shaped inputs must NOT be conflated:
///   - `model_id == None` (a fresh/never-selected profile) => the English default
///     tier, resolved THROUGH the catalog so the id and the dimension cannot
///     disagree. Pairing the default id with a hardcoded
///     [`DEFAULT_SEMANTIC_SEARCH_DIMENSION`] was a live trap: issue #190 is about
///     changing the English default, and the moment that default is a non-768 model
///     startup would build a 768 table stamped with (say) a 384 model. The write
///     gate then passes the stamp check and fails the width check on every anchor
///     forever — no error, no self-heal, a permanently empty index. Resolving the
///     descriptor makes that unrepresentable. If the default id somehow does not
///     resolve, this returns `None` and reconciliation is skipped, which is the same
///     safe behaviour as catalog drift below.
///   - `model_id == Some(unresolvable)` (catalog/config drift — the id no longer
///     resolves to a descriptor) => `None`: we do NOT know what the table holds, so
///     falling back to the default here would DROP a populated 384/1024 table and
///     force a full re-embed over a transient resolve failure. Returning `None`
///     makes the caller SKIP reconciliation and leave the existing table untouched;
///     the next time the id resolves (or the real selection re-runs the switch), the
///     table re-aligns.
fn reconcile_expected_model(
    settings: &capture_types::SemanticSearchSettings,
) -> Option<(String, usize)> {
    // A never-selected profile resolves against the DEFAULT provider, not
    // `settings.provider`: `model_id == None` only happens on a hand-edited or
    // partially-deserialized config, where the provider field may be junk, and the
    // previous revision ignored it entirely on this branch. A selected model keeps
    // using its own provider.
    let (provider, model_id) = match settings.model_id.as_deref() {
        None => (
            capture_types::default_semantic_search_provider(),
            capture_types::default_semantic_search_model_id()?,
        ),
        Some(model_id) => (settings.provider.clone(), model_id.to_string()),
    };
    // One resolution path for both cases, so the dimension always comes from the
    // same descriptor as the id it is paired with.
    resolve_descriptor(&provider, &model_id)
        .map(|descriptor| (descriptor.model_id, descriptor.dimension))
}

/// Reconcile the `vec0` table against the selected model on startup — the
/// **self-heal** for a permanently-stuck switch.
///
/// If a model switch ever left the table holding the old model's vectors while the
/// selection named a new one (e.g. a rebuild that failed under DB contention, or a
/// hand-edited config), every search degrades to keyword-only and the worker idles
/// forever — recovery cannot come from re-selecting the same model (the UI
/// early-returns on an unchanged pick). Run once on the deferred-startup seam, this
/// rebuilds the table under the selected model so the worker can backfill again.
/// Idempotent: a table that already matches is left untouched (the common case — no
/// rebuild, no vectors discarded). A table predating the model stamp is REBUILT, not
/// adopted in place: its embedding recipe is unknowable, so adopting it would stamp
/// the current recipe onto vectors produced by an older one (see
/// `reconcile_vectors_table`).
///
/// The expected model is resolved from `model_id` **ignoring `enabled`** (see
/// [`reconcile_expected_model`]): a disabled-but-previously-selected non-default
/// model must KEEP its table (disabling never clears `model_id`), so reconciliation
/// only falls back to the English default when no model was ever selected.
/// Resolving through the worker's enabled-gated `resolve_selected_descriptor` here
/// would wipe a disabled non-default user's index on every restart (B1).
///
/// A persisted `model_id` that no longer resolves (catalog/config drift) makes
/// [`reconcile_expected_model`] return `None`; we then SKIP reconciliation entirely
/// and leave the existing table as-is, so a transient resolve failure can never
/// silently DROP a populated index. The real selection re-aligns the table the next
/// time the id resolves.
///
/// `read_settings` is a READER, not a snapshot, on purpose: it is called after
/// [`SELECT_MODEL_LOCK`] is held, so the decision is made against the selection as it
/// stands once no switch can be in flight. Passing a value read on the caller's side
/// would reopen the check-then-act window the lock closes.
pub async fn reconcile_semantic_search_index_on_startup(
    infra: &crate::AppInfraState,
    read_settings: impl Fn() -> capture_types::SemanticSearchSettings,
) {
    // Serialize against a Settings model switch. `select_semantic_search_model` holds
    // this same lock across its rebuild+persist pair; this pass runs on the
    // deferred-startup seam — AFTER the window opens — so the user can switch models
    // while it is deciding. Without the lock a switch that commits mid-pass is
    // silently clobbered: `reconcile_vectors_table` DROPs the table the switch just
    // rebuilt and stamps the model from THIS pass's (now stale) settings snapshot
    // onto it, while the persisted selection names the new one. Nothing self-heals
    // that for the rest of the session — the model-stamp write gate skips every
    // batch (the sweep idles rather than errors) and the only other reconciler is the
    // next launch, so search silently stays keyword-only until a restart. Blocking
    // the deferred-startup thread for the length of one switch is the intended cost.
    let _switch_guard = select_model_lock().lock().await;

    // The selection is READ HERE, under the lock — never handed in as a snapshot
    // taken before it. A snapshot read on the way to the lock is stale by exactly the
    // window the lock exists to close: the switch that made us wait is the one that
    // persisted the new `model_id`, so acting on the pre-lock value re-DROPs the table
    // it just rebuilt and stamps the OLD model onto it. Same permanent, silent
    // keyword-only session as the no-lock case, just one step later.
    let settings = read_settings();

    let Some((expected_model_id, expected_dimension)) = reconcile_expected_model(&settings) else {
        crate::debug_log::log_warn(format!(
            "semantic search startup reconciliation skipped: selected model '{}' (provider '{}') does not resolve to a known descriptor; leaving the existing vec0 table untouched rather than wiping it to the {DEFAULT_SEMANTIC_SEARCH_DIMENSION}-dim default (catalog/config drift)",
            settings.model_id.as_deref().unwrap_or("<none>"),
            settings.provider
        ));
        return;
    };
    match infra
        .semantic_search()
        .reconcile_vectors_table(&expected_model_id, expected_dimension)
        .await
    {
        Ok(Some(discarded)) => crate::debug_log::log_info(format!(
            "semantic search startup reconciliation: the live vec0 table disagreed with the selected model; rebuilt it for '{expected_model_id}' at {expected_dimension} dims (discarded {discarded} stale vector(s)); the backfill worker will re-derive every anchor"
        )),
        Ok(None) => {
            // The common case: the table already matches the selected model. No log
            // so a default/Anthropic-only user sees nothing on every launch.
        }
        Err(error) => crate::debug_log::log_warn(format!(
            "semantic search startup reconciliation failed to read/rebuild the vec0 table (search stays keyword-only until it succeeds): {error}"
        )),
    }
}
