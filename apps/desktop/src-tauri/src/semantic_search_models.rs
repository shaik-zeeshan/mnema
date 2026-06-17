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
        Arc, Mutex,
    },
};

use futures_util::StreamExt;
use semantic_search::{
    builtin_model_manifest, detect_model_status, list_fastembed_supported_models, model_install_dir,
    resolve_descriptor, semantic_search_models_dir, write_installed_marker, ModelStatusError,
    ModelStatusKind,
    SemanticSearchModelDescriptor, SemanticSearchModelTier, CONFIG_FILE_NAME, FASTEMBED_PROVIDER_ID,
    SPECIAL_TOKENS_MAP_FILE_NAME, TOKENIZER_CONFIG_FILE_NAME, TOKENIZER_FILE_NAME,
};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

use crate::native_capture::debug_log::{log_error, log_info};

/// The frontend event the Settings UI listens on for live download progress.
/// The bytes/percent in each payload come from the streaming HTTP download of
/// fastembed's actual model files (per-chunk), not a CLI progress bool — so the
/// UI receives real programmatic byte progress (ADR 0036 / issue #125).
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
    /// The fastembed/HuggingFace repo id the model is downloaded from.
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

/// One fastembed-supported text-embedding model the **Custom** picker can offer,
/// distilled to the fields the Settings UI needs (serde camelCase to match the
/// other DTOs). `model_id` is a stable slug from the HF `model_code`'s last
/// segment, so a Custom pick installs under the same `{provider}/{model_id}`
/// layout as the guided tiers. `approx_download_bytes` is omitted (None) because
/// fastembed's `ModelInfo` carries no size; the UI shows the disk cost only once
/// known.
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

/// One file fastembed loads from disk and the HuggingFace path it is fetched
/// from. Both are the **same repo-relative path** (e.g. `onnx/model.onnx`): the
/// file is fetched from `…/resolve/main/<path>` and written to `<install>/<path>`,
/// preserving the `onnx/` subdirectory so an ONNX graph's external-data sibling
/// (`onnx/model.onnx_data`) stays resolvable. The detector requires this same
/// relative path, so the download, on-disk, and completeness views all agree.
#[derive(Debug, Clone)]
struct ModelFileSpec {
    relative_path: String,
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

/// List fastembed's enumerable text-embedding models for the **Custom** picker.
///
/// Drives entirely off fastembed's `ModelInfo` list, excluding gated repos (at
/// minimum EmbeddingGemma) the manual reqwest downloader cannot fetch. The
/// frontend Custom picker consumes these to let a user pick any locally-supported
/// model; downloading one then reuses [`start_semantic_search_model_download`]
/// once the selection is persisted.
#[tauri::command]
pub fn list_semantic_search_supported_models() -> Result<Vec<SupportedModelDto>, String> {
    let models = list_fastembed_supported_models()
        .into_iter()
        .map(|model| SupportedModelDto {
            model_id: model.model_id,
            display_name: model.display_name,
            model_code: model.model_code,
            dimension: model.dimension,
            description: model.description,
            multilingual: model.multilingual,
            // fastembed's ModelInfo carries no size; left None until known.
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
/// Mnema only downloads when the user picks a model here — fastembed's online
/// fetcher stays disabled, so nothing is auto-downloaded (ADR 0036).
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

    claim_model_download(
        download_state.inner(),
        &plan.provider,
        &plan.model_id,
        Arc::clone(&cancel_requested),
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
        run_model_download_task(app_for_task, plan, cancel_requested).await;
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
    Ok(())
}

/// The vec0 column dimension migration 0039 ships with — the English default
/// tier (`nomic-embed-text-v1.5`). Used as the rebuild fallback when no model is
/// selected, so the table matches a fresh-migration DB.
const DEFAULT_SEMANTIC_SEARCH_DIMENSION: usize = 768;

/// Re-derive the entire Semantic Search index after a confirmed model switch.
///
/// Different **Semantic Search Model Tier**s produce incomparable vectors and
/// `vec0` is a fixed-dim table, so switching the model means re-indexing every
/// **Search Result Anchor** (ADR 0036). Because a switch can also change the
/// vector *dimension* (e.g. 768-dim `nomic` → 1024-dim `bge-m3`), this rebuilds
/// the `vec0` table at the newly-selected model's dimension — not merely clears
/// its rows — so the worker's first store under the new model cannot fail on a
/// length mismatch. The **Semantic Index Backfill** worker then re-derives every
/// `direct` anchor under the new model (newest-first), with progress living
/// entirely in the DB. Returns the number of vectors discarded.
///
/// The caller (Settings UI) persists the new model selection FIRST (behind a
/// `@tauri-apps/plugin-dialog` confirm) and only then invokes this — so both the
/// resolved dimension here and the worker's reloaded embedder match the new
/// model on its next pass.
#[tauri::command]
pub async fn reindex_semantic_search(
    app_handle: tauri::AppHandle,
    infra: tauri::State<'_, crate::app_infra::AppInfraState>,
) -> Result<u64, String> {
    let settings =
        crate::semantic_search_worker::effective_semantic_search_settings(&app_handle);
    let dimension = crate::semantic_search_worker::resolve_selected_descriptor(&settings)
        .map(|descriptor| descriptor.dimension)
        .unwrap_or(DEFAULT_SEMANTIC_SEARCH_DIMENSION);
    let cleared = infra
        .semantic_search()
        .recreate_vectors_table(dimension)
        .await
        .map_err(|error| format!("failed to rebuild semantic search vectors: {error}"))?;
    crate::native_capture::debug_log::log_info(format!(
        "semantic search re-index requested: rebuilt vector table at {dimension} dims, discarded {cleared} vector(s); the backfill worker will re-derive every anchor under the new model"
    ));
    Ok(cleared)
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
        model_code: descriptor.model_code.clone(),
        approx_download_bytes: descriptor.approx_download_bytes,
        license_label: descriptor.license_label.clone(),
        status: status.status,
        available: status.is_available(),
        install_path: status.install_path.to_string_lossy().into_owned(),
        missing_files: status.missing_files,
    })
}

/// Report the 3 guided manifest tiers, plus the currently-selected model when it
/// is a **Custom** pick outside the manifest.
///
/// `selected_model_id` is the persisted `RecordingSettings.semantic_search`
/// selection (always the fastembed provider in v1). When it names a model not in
/// the manifest, the resolver synthesizes its descriptor from fastembed and its
/// detected status is appended — so the Settings UI can show that Custom model's
/// installed/selected state alongside the guided tiers. An unresolvable selection
/// (legacy id no longer enumerated) is simply omitted, never an error.
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

    // Append the selected Custom model (one outside the manifest) so its status is
    // visible. Manifest selections are already covered above.
    if let Some(model_id) = selected_model_id {
        let already_listed = models.iter().any(|model| model.model_id == model_id);
        if !already_listed {
            if let Some(descriptor) = resolve_descriptor(FASTEMBED_PROVIDER_ID, model_id) {
                models.push(status_dto_for(&models_dir, &descriptor)?);
            }
        }
    }

    Ok(SemanticSearchModelStatusResponseDto {
        models_directory: models_dir.to_string_lossy().into_owned(),
        models,
    })
}

/// The list of files to download for a model, as **repo-relative paths**.
///
/// The list is driven by fastembed's own `ModelInfo` (matched by `model_code`):
/// `[model_file] + additional_files + the four root tokenizer files`. This is
/// what fixes external data — a model like bge-m3 lists `onnx/model.onnx_data`
/// (its 2 GB weights) in `additional_files`, so it is fetched and the model
/// actually produces embeddings instead of "installing" but staying empty.
///
/// When the `model_code` is not found in fastembed's list (defensive; should not
/// happen for catalog models), we fall back to the descriptor's
/// `expected_layout.required_files`, which is itself derived from the same
/// fastembed facts.
fn model_file_specs(descriptor: &SemanticSearchModelDescriptor) -> Vec<ModelFileSpec> {
    let relative_paths = match list_fastembed_supported_models()
        .into_iter()
        .find(|model| model.model_code == descriptor.model_code)
    {
        Some(model) => {
            let mut files = Vec::new();
            files.push(model.onnx_relative_path);
            files.extend(model.external_data_files);
            files.extend(root_tokenizer_relative_paths());
            files
        }
        None => {
            log_info(format!(
                "semantic search model '{}' (code '{}') not found in fastembed list; \
                 falling back to descriptor expected_layout for the download file list",
                descriptor.model_id, descriptor.model_code
            ));
            descriptor.expected_layout.required_files.clone()
        }
    };
    relative_paths
        .into_iter()
        .map(|relative_path| ModelFileSpec { relative_path })
        .collect()
}

/// The four tokenizer/config files fastembed always fetches from the repo root.
fn root_tokenizer_relative_paths() -> Vec<String> {
    vec![
        TOKENIZER_FILE_NAME.to_string(),
        TOKENIZER_CONFIG_FILE_NAME.to_string(),
        SPECIAL_TOKENS_MAP_FILE_NAME.to_string(),
        CONFIG_FILE_NAME.to_string(),
    ]
}

/// The HuggingFace `resolve/main` URL for one file in a model repo.
fn hf_file_url(model_code: &str, hf_relative_path: &str) -> String {
    format!("https://huggingface.co/{model_code}/resolve/main/{hf_relative_path}")
}

fn build_download_plan(
    app_handle: &tauri::AppHandle,
    request: &SemanticSearchModelDownloadRequestDto,
) -> Result<DownloadPlan, ModelDownloadError> {
    // Resolve through the shared resolver so a Custom-picked fastembed model
    // (synthesized from fastembed's ModelInfo) downloads with the right file
    // list/layout, not just the 3 guided manifest tiers.
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
        install_dir,
        files: model_file_specs(&descriptor),
        total_bytes: descriptor.approx_download_bytes,
    })
}

/// Resolve a descriptor for a `{provider}/{model_id}` selection — manifest first,
/// then a fastembed-synthesized descriptor for a **Custom**-picked model. Both the
/// download plan and the install verification go through this so a Custom model
/// installs under the same layout the picker advertised.
fn descriptor_for(provider: &str, model_id: &str) -> Option<SemanticSearchModelDescriptor> {
    resolve_descriptor(provider, model_id)
}

fn claim_model_download(
    state: &SemanticSearchModelDownloadState,
    provider: &str,
    model_id: &str,
    cancel_requested: Arc<AtomicBool>,
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
) {
    log_info(format!(
        "semantic search model download started: provider={}, model={}, install_dir={}, files={}, approx_total_bytes={}",
        plan.provider,
        plan.model_id,
        plan.install_dir.display(),
        plan.files.len(),
        plan.total_bytes,
    ));
    let result = download_and_install_model(&app_handle, &plan, &cancel_requested).await;
    clear_active_download(&app_handle, &plan.provider, &plan.model_id);

    match result {
        Ok(()) => {
            log_info(format!(
                "semantic search model download completed: {}/{} installed at {}",
                plan.provider,
                plan.model_id,
                plan.install_dir.display(),
            ));
            emit_download_progress(
                &app_handle,
                &SemanticSearchModelDownloadProgressDto::new(
                    &plan.provider,
                    &plan.model_id,
                    SemanticSearchModelDownloadStatusDto::Completed,
                    plan.total_bytes,
                    Some(plan.total_bytes),
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

async fn download_and_install_model(
    app_handle: &tauri::AppHandle,
    plan: &DownloadPlan,
    cancel_requested: &AtomicBool,
) -> Result<(), ModelDownloadTaskError> {
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
        let url = hf_file_url(&descriptor.model_code, &spec.relative_path);
        // Preserve the repo-relative subdir on disk (e.g. `onnx/model.onnx`) so an
        // ONNX graph's external-data sibling resolves at load time.
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
            &destination,
            downloaded_total,
            cancel_requested,
        )
        .await?;
        downloaded_total = downloaded_total.saturating_add(file_bytes);
        log_info(format!(
            "semantic search model download saved '{}' ({file_bytes} bytes) for {}/{}",
            spec.relative_path, plan.provider, plan.model_id
        ));
    }

    // Verify every required file landed before claiming Installed. This now covers
    // external-data siblings (e.g. bge-m3's `onnx/model.onnx_data`), so a model
    // missing its weights is never marked Installed.
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
    Ok(())
}

async fn download_file_to(
    app_handle: &tauri::AppHandle,
    plan: &DownloadPlan,
    url: &str,
    destination: &Path,
    already_downloaded_bytes: u64,
    cancel_requested: &AtomicBool,
) -> Result<u64, ModelDownloadTaskError> {
    let response = reqwest::get(url).await.map_err(|error| {
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
    while let Some(chunk) = stream.next().await {
        if cancel_requested.load(Ordering::SeqCst) {
            return Err(ModelDownloadTaskError::Cancelled);
        }
        let chunk = chunk.map_err(ModelDownloadError::Http)?;
        std::io::Write::write_all(&mut output, &chunk).map_err(|source| {
            ModelDownloadError::WriteFile {
                path: destination.to_path_buf(),
                source,
            }
        })?;
        file_downloaded = file_downloaded.saturating_add(chunk.len() as u64);
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

    #[test]
    fn status_response_surfaces_a_selected_custom_model_outside_the_manifest() {
        // A Custom-picked fastembed model (not one of the 3 guided tiers) appears in
        // the status response when it is the persisted selection, so the Settings UI
        // can show its installed/selected state.
        let manifest_ids: Vec<String> = builtin_model_manifest()
            .models
            .into_iter()
            .map(|model| model.model_id)
            .collect();
        let custom = list_fastembed_supported_models()
            .into_iter()
            .find(|model| !manifest_ids.contains(&model.model_id))
            .expect("a non-manifest fastembed model");

        let temp = tempfile::tempdir().expect("tempdir");
        let response =
            build_semantic_search_model_status_response(temp.path(), Some(&custom.model_id))
                .expect("status response");

        // The 3 guided tiers plus the selected Custom model.
        assert_eq!(response.models.len(), manifest_ids.len() + 1);
        let custom_row = response
            .models
            .iter()
            .find(|model| model.model_id == custom.model_id)
            .expect("selected custom model must be listed");
        assert_eq!(custom_row.tier, SemanticSearchModelTier::Custom);
        assert_eq!(custom_row.model_code, custom.model_code);
        // Not installed on disk => Missing / unavailable, but still surfaced.
        assert!(!custom_row.available);
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
            model_install_dir(&models_dir, FASTEMBED_PROVIDER_ID, &descriptor.model_id)
                .expect("install dir");
        std::fs::create_dir_all(&install_dir).expect("install dir");
        for file_name in &descriptor.expected_layout.required_files {
            let path = install_dir.join(file_name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("parent dir");
            }
            std::fs::write(path, b"x").expect("model file");
        }
        std::fs::write(
            install_dir.join(".installed.json"),
            serde_json::json!({
                "manifestVersion": 1,
                "provider": FASTEMBED_PROVIDER_ID,
                "modelId": descriptor.model_id,
            })
            .to_string(),
        )
        .expect("marker");

        assert!(selected_semantic_search_model_available(temp.path(), &settings).expect("gating check"));
    }

    #[test]
    fn file_specs_cover_every_required_layout_file() {
        // The download spec list (driven by fastembed's ModelInfo) must cover every
        // file the detector requires for that model — including external-data
        // siblings like bge-m3's `onnx/model.onnx_data`, so a model never installs
        // missing its weights.
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
    fn bge_m3_download_plan_includes_external_data() {
        // Regression: bge-m3's 2 GB `onnx/model.onnx_data` must be in the download
        // list (matched from fastembed's ModelInfo), not silently dropped.
        let manifest = builtin_model_manifest();
        let bge =
            find_model_descriptor(&manifest, FASTEMBED_PROVIDER_ID, "bge-m3").expect("bge-m3");
        let specs = model_file_specs(bge);
        let paths: Vec<&str> = specs.iter().map(|s| s.relative_path.as_str()).collect();
        assert!(paths.contains(&"onnx/model.onnx"));
        assert!(paths.contains(&"onnx/model.onnx_data"));
        assert!(paths.contains(&"tokenizer.json"));
    }

    #[test]
    fn supported_models_list_excludes_gated_gemma_and_slugs_codes() {
        let models = list_semantic_search_supported_models().expect("supported models");
        assert!(!models.is_empty(), "fastembed should enumerate models");
        assert!(
            models.iter().all(|m| !m.model_code.to_ascii_lowercase().contains("gemma")),
            "gated EmbeddingGemma must be excluded from the Custom picker"
        );
        // e5-small enumerates with a slugged id and the multilingual flag set.
        let e5 = models
            .iter()
            .find(|m| m.model_code == "intfloat/multilingual-e5-small");
        if let Some(e5) = e5 {
            assert_eq!(e5.model_id, "multilingual-e5-small");
            assert!(e5.multilingual);
            assert_eq!(e5.dimension, 384);
        }
    }

    #[test]
    fn hf_urls_point_at_the_model_repo_resolve_main() {
        let manifest = builtin_model_manifest();
        let nomic = find_model_descriptor(&manifest, FASTEMBED_PROVIDER_ID, "nomic-embed-text-v1.5")
            .expect("nomic descriptor");
        let onnx_url = hf_file_url(&nomic.model_code, "onnx/model.onnx");
        assert_eq!(
            onnx_url,
            "https://huggingface.co/nomic-ai/nomic-embed-text-v1.5/resolve/main/onnx/model.onnx"
        );
        let tokenizer_url = hf_file_url(&nomic.model_code, "tokenizer.json");
        assert!(tokenizer_url.ends_with("/resolve/main/tokenizer.json"));
    }

    #[test]
    fn only_one_download_can_be_claimed_at_a_time() {
        let state = SemanticSearchModelDownloadState::default();
        claim_model_download(
            &state,
            FASTEMBED_PROVIDER_ID,
            "nomic-embed-text-v1.5",
            Arc::new(AtomicBool::new(false)),
        )
        .expect("first claim");
        let second = claim_model_download(
            &state,
            FASTEMBED_PROVIDER_ID,
            "multilingual-e5-small",
            Arc::new(AtomicBool::new(false)),
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
}
