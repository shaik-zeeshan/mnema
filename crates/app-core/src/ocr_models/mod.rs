use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use capture_types::{OcrProvider, OcrSettings};
use futures_util::StreamExt;
use ocr::{
    builtin_model_manifest, detect_model_status, find_model_descriptor,
    install_downloaded_model_artifact, model_install_dir, ocr_models_dir,
    provider_runtime_available, remove_model_dir_if_exists, remove_model_file_if_exists,
    validate_artifact_sha256, write_downloading_marker, write_failed_marker, ModelArtifact,
    ModelArtifactFile, ModelArtifactShape, ModelInstallError, ModelManagement, ModelStatusError,
    ModelStatusKind, OcrModelDescriptor,
};
use serde::{Deserialize, Serialize};

use crate::{AppCore, AppInfraState, CoreEvent};

mod download;
use download::*;

pub const OCR_MODEL_DOWNLOAD_PROGRESS_EVENT: &str = "ocr_model_download_progress";

pub type OcrModelDownloadState = Mutex<Option<ActiveOcrModelDownload>>;

#[derive(Debug, Clone)]
pub struct ActiveOcrModelDownload {
    provider: String,
    model_id: String,
    cancel_requested: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OcrModelDownloadRequestDto {
    pub provider: String,
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeletedOcrModelDto {
    pub provider: String,
    pub model_id: String,
    pub display_name: String,
    pub install_path: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeleteUnusedOcrModelsResponseDto {
    pub deleted: Vec<DeletedOcrModelDto>,
    pub skipped_active_downloads: Vec<DeletedOcrModelDto>,
    pub skipped_processing_jobs: Vec<DeletedOcrModelDto>,
    pub retargeted_processing_jobs: u64,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OcrModelDownloadStatusDto {
    Starting,
    Downloading,
    Installing,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OcrModelDownloadProgressDto {
    pub provider: String,
    pub model_id: String,
    pub status: OcrModelDownloadStatusDto,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub message: Option<String>,
}

impl OcrModelDownloadProgressDto {
    fn new(
        provider: impl Into<String>,
        model_id: impl Into<String>,
        status: OcrModelDownloadStatusDto,
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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OcrModelStatusResponseDto {
    pub models_directory: String,
    pub providers: Vec<OcrProviderStatusDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OcrProviderStatusDto {
    pub provider: String,
    pub display_name: String,
    pub models: Vec<OcrModelStatusDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OcrModelStatusDto {
    pub provider: String,
    pub model_id: Option<String>,
    pub display_name: String,
    pub description: String,
    pub management: OcrModelManagementDto,
    pub status: ModelStatusKind,
    pub available: bool,
    pub install_path: Option<String>,
    pub missing_files: Vec<String>,
    pub failure_message: Option<String>,
    pub license_label: Option<String>,
    pub source_url: Option<String>,
    pub download: Option<OcrModelDownloadDto>,
    pub runtime_message: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OcrModelManagementDto {
    AppManaged,
    OsManaged,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OcrModelDownloadDto {
    pub url: String,
    pub byte_size: u64,
    pub sha256: String,
    pub shape: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DownloadPlan {
    provider: String,
    model_id: String,
    models_dir: PathBuf,
    install_dir: PathBuf,
    temp_file_path: PathBuf,
    descriptor: OcrModelDescriptor,
    artifact: ModelArtifact,
}

#[derive(Debug, thiserror::Error)]
enum ModelDownloadError {
    #[error("download for {provider}/{model_id} is already running")]
    AlreadyRunning { provider: String, model_id: String },
    #[error("no active OCR model download")]
    NoActiveDownload,
    #[error("ocr model not found for provider={provider}, modelId={model_id}")]
    ModelNotFound { provider: String, model_id: String },
    #[error("model {provider}/{model_id} is OS-managed and cannot be downloaded by the app")]
    OsManaged { provider: String, model_id: String },
    #[error("model {provider}/{model_id} has no app-managed download artifact")]
    MissingArtifact { provider: String, model_id: String },
    #[error("failed to inspect OCR model status: {0}")]
    Status(#[from] ModelStatusError),
    #[error("failed to install OCR model: {0}")]
    Install(#[from] ModelInstallError),
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
}

pub fn get_ocr_model_status(core: &AppCore) -> Result<OcrModelStatusResponseDto, String> {
    build_ocr_model_status_response(&core.paths().config)
        .map_err(|error| format!("failed to inspect OCR models: {error}"))
}

pub fn start_ocr_model_download(
    core: &Arc<AppCore>,
    request: OcrModelDownloadRequestDto,
) -> Result<OcrModelDownloadProgressDto, String> {
    let plan = build_download_plan(core, &request).map_err(|error| error.to_string())?;
    let cancel_requested = Arc::new(AtomicBool::new(false));

    claim_model_download(
        &core.state::<OcrModelDownloadState>(),
        &plan.provider,
        &plan.model_id,
        Arc::clone(&cancel_requested),
    )
    .map_err(|error| error.to_string())?;

    let starting = OcrModelDownloadProgressDto::new(
        &plan.provider,
        &plan.model_id,
        OcrModelDownloadStatusDto::Starting,
        0,
        Some(plan.artifact.byte_size),
        None,
    );
    emit_download_progress(core, &starting);

    let core_for_task = Arc::clone(core);
    core.spawn(async move {
        run_model_download_task(core_for_task, plan, cancel_requested).await;
    });

    Ok(starting)
}

pub fn cancel_ocr_model_download(core: &AppCore) -> Result<(), String> {
    let download_state = core.state::<OcrModelDownloadState>();
    let active = download_state
        .lock()
        .map_err(|_| "ocr model download state poisoned".to_string())?;
    let Some(active) = active.as_ref() else {
        return Err(ModelDownloadError::NoActiveDownload.to_string());
    };
    active.cancel_requested.store(true, Ordering::SeqCst);
    Ok(())
}

pub async fn delete_unused_ocr_models(
    core: &AppCore,
    infra: &AppInfraState,
) -> Result<DeleteUnusedOcrModelsResponseDto, String> {
    let app_data_dir = core.paths().config.clone();
    let settings = crate::recording_settings::current_recording_settings(
        &core.state::<crate::recording_settings::RecordingSettingsState>(),
    );
    let selected_provider = provider_id_for_settings(settings.ocr.provider);
    let selected_model_id = resolved_model_id_for_settings(&settings.ocr);
    let active_download = active_download_key(&core.state::<OcrModelDownloadState>())?;
    let deletion_candidate_model_keys = unused_installed_ocr_model_keys(
        &app_data_dir,
        selected_provider,
        selected_model_id.as_deref(),
        active_download.as_deref(),
    )
    .map_err(|error| format!("failed to inspect unused OCR models: {error}"))?;
    let running_job_model_keys = infra
        .list_running_ocr_model_keys()
        .await
        .map_err(|error| format!("failed to inspect running OCR jobs: {error}"))?;
    let retarget_candidate_model_keys =
        retargetable_deletion_model_keys(&deletion_candidate_model_keys, &running_job_model_keys);
    let cleanup_lock = infra
        .acquire_ocr_model_cleanup_locks(&retarget_candidate_model_keys)
        .await
        .map_err(|error| format!("failed to reserve OCR models for cleanup: {error}"))?;
    let result = async {
        let retargeted_processing_jobs = infra
            .retarget_ocr_jobs_referencing_model_keys(
                &cleanup_lock.acquired_model_keys,
                selected_provider,
                selected_model_id.as_deref(),
            )
            .await
            .map_err(|error| format!("failed to retarget queued OCR jobs: {error}"))?;
        let mut protected_model_keys =
            infra.list_running_ocr_model_keys().await.map_err(|error| {
                format!("failed to re-check running OCR jobs before deleting models: {error}")
            })?;
        protected_model_keys.extend(
            deletion_candidate_model_keys
                .difference(&cleanup_lock.acquired_model_keys)
                .cloned(),
        );

        delete_unused_ocr_models_inner(
            &app_data_dir,
            selected_provider,
            selected_model_id.as_deref(),
            active_download.as_deref(),
            &protected_model_keys,
            retargeted_processing_jobs,
        )
        .map_err(|error| format!("failed to delete unused OCR models: {error}"))
    }
    .await;
    let release_result = infra
        .release_processing_model_locks(&cleanup_lock)
        .await
        .map_err(|error| format!("failed to release OCR model cleanup reservation: {error}"));

    match (result, release_result) {
        (Ok(response), Ok(_)) => Ok(response),
        (Err(error), Ok(_)) => Err(error),
        (Ok(_), Err(error)) | (Err(_), Err(error)) => Err(error),
    }
}

pub fn selected_ocr_model_available(
    app_data_dir: &Path,
    settings: &OcrSettings,
) -> Result<bool, ModelStatusError> {
    let provider = provider_id_for_settings(settings.provider);
    let model_id = resolved_model_id_for_settings(settings);
    let manifest = builtin_model_manifest();
    let Some(descriptor) = find_model_descriptor(&manifest, provider, model_id.as_deref()) else {
        return Ok(false);
    };
    let status = detect_model_status(ocr_models_dir(app_data_dir), descriptor)?;
    if !status.is_available() {
        return Ok(false);
    }
    Ok(provider_runtime_available(provider))
}

/// The model key of the selected OCR model when its files are **not** on disk, so its queued jobs
/// can be parked (claim-time model gating) instead of burning their failure attempts against a
/// model that is not there. `None` when the selection needs no model files (Apple Vision) or the
/// model is installed. Deliberately files-only: unlike [`selected_ocr_model_available`] it ignores
/// runtime availability, which is not something a download can fix.
pub fn absent_selected_ocr_model_key(
    app_data_dir: &Path,
    settings: &OcrSettings,
) -> Option<String> {
    let provider = provider_id_for_settings(settings.provider);
    let model_id = resolved_model_id_for_settings(settings)?;
    let manifest = builtin_model_manifest();
    let installed = find_model_descriptor(&manifest, provider, Some(&model_id))
        .and_then(|descriptor| detect_model_status(ocr_models_dir(app_data_dir), descriptor).ok())
        .is_some_and(|status| status.is_available());
    (!installed).then(|| model_key(provider, &model_id))
}

pub fn provider_id_for_settings(provider: OcrProvider) -> &'static str {
    match provider {
        OcrProvider::AppleVision => ocr::APPLE_VISION_PROVIDER_ID,
        OcrProvider::Tesseract => ocr::TESSERACT_PROVIDER_ID,
        OcrProvider::PaddleOcr => ocr::PADDLE_OCR_PROVIDER_ID,
    }
}

pub fn resolved_model_id_for_settings(settings: &OcrSettings) -> Option<String> {
    let model_id = settings
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|model_id| !model_id.is_empty());
    match settings.provider {
        OcrProvider::AppleVision => None,
        OcrProvider::Tesseract => Some(
            model_id
                .unwrap_or(ocr::DEFAULT_TESSERACT_MODEL_ID)
                .to_string(),
        ),
        OcrProvider::PaddleOcr => Some(
            model_id
                .unwrap_or(ocr::DEFAULT_PADDLE_OCR_MODEL_ID)
                .to_string(),
        ),
    }
}

fn provider_display_name(provider: &str) -> &'static str {
    match provider {
        ocr::APPLE_VISION_PROVIDER_ID => "Apple Vision",
        ocr::TESSERACT_PROVIDER_ID => "Tesseract",
        ocr::PADDLE_OCR_PROVIDER_ID => "PaddleOCR",
        _ => "Unknown provider",
    }
}

fn runtime_message(provider: &str) -> Option<String> {
    if provider_runtime_available(provider) {
        None
    } else if provider == ocr::APPLE_VISION_PROVIDER_ID {
        Some("Apple Vision OCR is only available on macOS.".to_string())
    } else if provider == ocr::TESSERACT_PROVIDER_ID {
        Some("Tesseract runtime execution is not enabled in this build yet.".to_string())
    } else if provider == ocr::PADDLE_OCR_PROVIDER_ID {
        Some("PaddleOCR runtime execution is not enabled in this build yet.".to_string())
    } else {
        Some("OCR provider runtime is unavailable.".to_string())
    }
}

fn map_model_status(
    descriptor: &OcrModelDescriptor,
    status: ocr::OcrModelStatus,
) -> OcrModelStatusDto {
    let runtime_message = runtime_message(&descriptor.provider);
    let available = status.is_available() && runtime_message.is_none();
    OcrModelStatusDto {
        provider: descriptor.provider.clone(),
        model_id: descriptor.model_id.clone(),
        display_name: descriptor.display_name.clone(),
        description: descriptor.description.clone(),
        management: match descriptor.management {
            ModelManagement::AppManaged { .. } => OcrModelManagementDto::AppManaged,
            ModelManagement::OsManaged => OcrModelManagementDto::OsManaged,
        },
        status: status.status,
        available,
        install_path: status.install_path.map(|path| path.display().to_string()),
        missing_files: status.missing_files,
        failure_message: status.failure_message,
        license_label: descriptor.license_label.clone(),
        source_url: descriptor.source_url.clone(),
        download: match &descriptor.management {
            ModelManagement::AppManaged {
                artifact: Some(artifact),
                ..
            } => Some(OcrModelDownloadDto {
                url: artifact.url.clone(),
                byte_size: artifact.byte_size,
                sha256: artifact.sha256.clone(),
                shape: serde_json::to_value(&artifact.shape).unwrap_or(serde_json::Value::Null),
            }),
            _ => None,
        },
        runtime_message,
    }
}

fn build_ocr_model_status_response(
    app_data_dir: &Path,
) -> Result<OcrModelStatusResponseDto, ModelStatusError> {
    let manifest = builtin_model_manifest();
    let models_dir = ocr_models_dir(app_data_dir);
    let mut grouped = BTreeMap::<String, Vec<OcrModelStatusDto>>::new();

    for descriptor in &manifest.models {
        if !is_desktop_selectable_ocr_provider(&descriptor.provider) {
            continue;
        }
        let status = detect_model_status(&models_dir, descriptor)?;
        grouped
            .entry(descriptor.provider.clone())
            .or_default()
            .push(map_model_status(descriptor, status));
    }

    let providers = grouped
        .into_iter()
        .map(|(provider, models)| OcrProviderStatusDto {
            provider: provider.clone(),
            display_name: provider_display_name(&provider).to_string(),
            models,
        })
        .collect();

    Ok(OcrModelStatusResponseDto {
        models_directory: models_dir.display().to_string(),
        providers,
    })
}

fn is_desktop_selectable_ocr_provider(provider: &str) -> bool {
    matches!(
        provider,
        ocr::APPLE_VISION_PROVIDER_ID | ocr::TESSERACT_PROVIDER_ID
    )
}

fn claim_model_download(
    state: &OcrModelDownloadState,
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
    *active = Some(ActiveOcrModelDownload {
        provider: provider.to_string(),
        model_id: model_id.to_string(),
        cancel_requested,
    });
    Ok(())
}

fn active_download_key(state: &OcrModelDownloadState) -> Result<Option<String>, String> {
    let active = state
        .lock()
        .map_err(|_| "ocr model download state poisoned".to_string())?;
    Ok(active
        .as_ref()
        .map(|download| model_key(&download.provider, &download.model_id)))
}

fn model_key(provider: &str, model_id: &str) -> String {
    format!("{provider}/{model_id}")
}

#[derive(Debug, thiserror::Error)]
enum DeleteUnusedOcrModelsError {
    #[error(transparent)]
    Status(#[from] ModelStatusError),
    #[error(transparent)]
    Install(#[from] ModelInstallError),
}

fn delete_unused_ocr_models_inner(
    app_data_dir: &Path,
    selected_provider: &str,
    selected_model_id: Option<&str>,
    active_download: Option<&str>,
    running_job_model_keys: &BTreeSet<String>,
    retargeted_processing_jobs: u64,
) -> Result<DeleteUnusedOcrModelsResponseDto, DeleteUnusedOcrModelsError> {
    let models_dir = ocr_models_dir(app_data_dir);
    let manifest = builtin_model_manifest();
    let mut deleted = Vec::new();
    let mut skipped_active_downloads = Vec::new();
    let mut skipped_processing_jobs = Vec::new();

    for descriptor in &manifest.models {
        let ModelManagement::AppManaged { .. } = &descriptor.management else {
            continue;
        };
        let Some(model_id) = descriptor.model_id.as_deref() else {
            continue;
        };
        if descriptor.provider == selected_provider && Some(model_id) == selected_model_id {
            continue;
        }
        let install_dir = model_install_dir(&models_dir, &descriptor.provider, model_id)?;
        if !install_dir.exists() {
            continue;
        }

        let candidate = DeletedOcrModelDto {
            provider: descriptor.provider.clone(),
            model_id: model_id.to_string(),
            display_name: descriptor.display_name.clone(),
            install_path: install_dir.display().to_string(),
        };
        if active_download == Some(model_key(&descriptor.provider, model_id).as_str()) {
            skipped_active_downloads.push(candidate);
            continue;
        }
        if running_job_model_keys.contains(&model_key(&descriptor.provider, model_id)) {
            skipped_processing_jobs.push(candidate);
            continue;
        }

        remove_model_dir_if_exists(&install_dir)?;
        deleted.push(candidate);
    }

    Ok(DeleteUnusedOcrModelsResponseDto {
        deleted,
        skipped_active_downloads,
        skipped_processing_jobs,
        retargeted_processing_jobs,
    })
}

fn unused_installed_ocr_model_keys(
    app_data_dir: &Path,
    selected_provider: &str,
    selected_model_id: Option<&str>,
    active_download: Option<&str>,
) -> Result<BTreeSet<String>, DeleteUnusedOcrModelsError> {
    let models_dir = ocr_models_dir(app_data_dir);
    let manifest = builtin_model_manifest();
    let mut keys = BTreeSet::new();

    for descriptor in &manifest.models {
        let ModelManagement::AppManaged { .. } = &descriptor.management else {
            continue;
        };
        let Some(model_id) = descriptor.model_id.as_deref() else {
            continue;
        };
        if descriptor.provider == selected_provider && Some(model_id) == selected_model_id {
            continue;
        }
        let key = model_key(&descriptor.provider, model_id);
        if active_download == Some(key.as_str()) {
            continue;
        }
        let install_dir = model_install_dir(&models_dir, &descriptor.provider, model_id)?;
        if install_dir.exists() {
            keys.insert(key);
        }
    }

    Ok(keys)
}

fn retargetable_deletion_model_keys(
    deletion_candidate_model_keys: &BTreeSet<String>,
    running_job_model_keys: &BTreeSet<String>,
) -> BTreeSet<String> {
    deletion_candidate_model_keys
        .difference(running_job_model_keys)
        .cloned()
        .collect()
}

fn clear_active_download(core: &AppCore, provider: &str, model_id: &str) {
    if let Ok(mut active) = core.state::<OcrModelDownloadState>().lock() {
        if active
            .as_ref()
            .is_some_and(|download| download.provider == provider && download.model_id == model_id)
        {
            *active = None;
        }
    }
}

#[cfg(test)]
mod tests;
