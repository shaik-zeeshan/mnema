use std::{
    collections::BTreeMap,
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
use tauri::{Emitter, Manager};

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

#[tauri::command]
pub fn get_ocr_model_status(
    app_handle: tauri::AppHandle,
) -> Result<OcrModelStatusResponseDto, String> {
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|error| format!("failed to resolve app data directory: {error}"))?;
    build_ocr_model_status_response(&app_data_dir)
        .map_err(|error| format!("failed to inspect OCR models: {error}"))
}

#[tauri::command]
pub fn start_ocr_model_download(
    app_handle: tauri::AppHandle,
    request: OcrModelDownloadRequestDto,
    download_state: tauri::State<'_, OcrModelDownloadState>,
) -> Result<OcrModelDownloadProgressDto, String> {
    let plan = build_download_plan(&app_handle, &request).map_err(|error| error.to_string())?;
    let cancel_requested = Arc::new(AtomicBool::new(false));

    claim_model_download(
        download_state.inner(),
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
    emit_download_progress(&app_handle, &starting);

    let app_for_task = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        run_model_download_task(app_for_task, plan, cancel_requested).await;
    });

    Ok(starting)
}

#[tauri::command]
pub fn cancel_ocr_model_download(
    download_state: tauri::State<'_, OcrModelDownloadState>,
) -> Result<(), String> {
    let active = download_state
        .lock()
        .map_err(|_| "ocr model download state poisoned".to_string())?;
    let Some(active) = active.as_ref() else {
        return Err(ModelDownloadError::NoActiveDownload.to_string());
    };
    active.cancel_requested.store(true, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
pub fn delete_unused_ocr_models(
    app_handle: tauri::AppHandle,
    download_state: tauri::State<'_, OcrModelDownloadState>,
    settings: tauri::State<'_, crate::native_capture::RecordingSettingsState>,
) -> Result<DeleteUnusedOcrModelsResponseDto, String> {
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|error| format!("failed to resolve app data directory: {error}"))?;
    let settings = crate::native_capture::read_recording_settings(settings.inner());
    let selected_provider = provider_id_for_settings(settings.ocr.provider);
    let selected_model_id = resolved_model_id_for_settings(&settings.ocr);
    let active_download = active_download_key(download_state.inner())?;

    delete_unused_ocr_models_inner(
        &app_data_dir,
        selected_provider,
        selected_model_id.as_deref(),
        active_download.as_deref(),
    )
    .map_err(|error| format!("failed to delete unused OCR models: {error}"))
}

pub(crate) fn selected_ocr_model_available(
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

pub(crate) fn provider_id_for_settings(provider: OcrProvider) -> &'static str {
    match provider {
        OcrProvider::AppleVision => ocr::APPLE_VISION_PROVIDER_ID,
        OcrProvider::Tesseract => ocr::TESSERACT_PROVIDER_ID,
        OcrProvider::PaddleOcr => ocr::PADDLE_OCR_PROVIDER_ID,
    }
}

pub(crate) fn resolved_model_id_for_settings(settings: &OcrSettings) -> Option<String> {
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
) -> Result<DeleteUnusedOcrModelsResponseDto, DeleteUnusedOcrModelsError> {
    let models_dir = ocr_models_dir(app_data_dir);
    let manifest = builtin_model_manifest();
    let mut deleted = Vec::new();
    let mut skipped_active_downloads = Vec::new();

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

        remove_model_dir_if_exists(&install_dir)?;
        deleted.push(candidate);
    }

    Ok(DeleteUnusedOcrModelsResponseDto {
        deleted,
        skipped_active_downloads,
    })
}

fn clear_active_download(app_handle: &tauri::AppHandle, provider: &str, model_id: &str) {
    if let Ok(mut active) = app_handle.state::<OcrModelDownloadState>().lock() {
        if active
            .as_ref()
            .is_some_and(|download| download.provider == provider && download.model_id == model_id)
        {
            *active = None;
        }
    }
}

fn build_download_plan(
    app_handle: &tauri::AppHandle,
    request: &OcrModelDownloadRequestDto,
) -> Result<DownloadPlan, ModelDownloadError> {
    let manifest = builtin_model_manifest();
    let descriptor = find_model_descriptor(&manifest, &request.provider, Some(&request.model_id))
        .cloned()
        .ok_or_else(|| ModelDownloadError::ModelNotFound {
            provider: request.provider.clone(),
            model_id: request.model_id.clone(),
        })?;

    let artifact = match &descriptor.management {
        ModelManagement::OsManaged => {
            return Err(ModelDownloadError::OsManaged {
                provider: request.provider.clone(),
                model_id: request.model_id.clone(),
            })
        }
        ModelManagement::AppManaged { artifact: None, .. } => {
            return Err(ModelDownloadError::MissingArtifact {
                provider: request.provider.clone(),
                model_id: request.model_id.clone(),
            })
        }
        ModelManagement::AppManaged {
            artifact: Some(artifact),
            ..
        } => artifact.clone(),
    };

    let app_data_dir =
        app_handle
            .path()
            .app_data_dir()
            .map_err(|error| ModelDownloadError::CreateDir {
                path: PathBuf::from("<app_data_dir>"),
                source: std::io::Error::other(error.to_string()),
            })?;
    let models_dir = ocr_models_dir(app_data_dir);
    let install_dir = model_install_dir(&models_dir, &request.provider, &request.model_id)?;
    let temp_file_path = install_dir.join(".download.tmp");

    Ok(DownloadPlan {
        provider: request.provider.clone(),
        model_id: request.model_id.clone(),
        models_dir,
        install_dir,
        temp_file_path,
        descriptor,
        artifact,
    })
}

fn emit_download_progress(app_handle: &tauri::AppHandle, progress: &OcrModelDownloadProgressDto) {
    let _ = app_handle.emit(OCR_MODEL_DOWNLOAD_PROGRESS_EVENT, progress);
}

async fn run_model_download_task(
    app_handle: tauri::AppHandle,
    plan: DownloadPlan,
    cancel_requested: Arc<AtomicBool>,
) {
    let result = download_and_install_model(&app_handle, &plan, &cancel_requested).await;
    clear_active_download(&app_handle, &plan.provider, &plan.model_id);

    match result {
        Ok(()) => {
            let progress = OcrModelDownloadProgressDto::new(
                &plan.provider,
                &plan.model_id,
                OcrModelDownloadStatusDto::Completed,
                plan.artifact.byte_size,
                Some(plan.artifact.byte_size),
                None,
            );
            emit_download_progress(&app_handle, &progress);
        }
        Err(ModelDownloadTaskError::Cancelled) => {
            let _ = remove_model_file_if_exists(&plan.temp_file_path);
            let _ = remove_model_file_if_exists(
                plan.install_dir.join(ocr::DOWNLOADING_MARKER_FILE_NAME),
            );
            let progress = OcrModelDownloadProgressDto::new(
                &plan.provider,
                &plan.model_id,
                OcrModelDownloadStatusDto::Cancelled,
                0,
                Some(plan.artifact.byte_size),
                Some("download cancelled".to_string()),
            );
            emit_download_progress(&app_handle, &progress);
        }
        Err(ModelDownloadTaskError::Failed(error)) => {
            let _ = remove_model_file_if_exists(&plan.temp_file_path);
            let _ = remove_model_file_if_exists(
                plan.install_dir.join(ocr::DOWNLOADING_MARKER_FILE_NAME),
            );
            let _ = write_failed_marker(
                &plan.models_dir,
                &plan.provider,
                &plan.model_id,
                error.to_string(),
            );
            let progress = OcrModelDownloadProgressDto::new(
                &plan.provider,
                &plan.model_id,
                OcrModelDownloadStatusDto::Failed,
                0,
                Some(plan.artifact.byte_size),
                Some(error.to_string()),
            );
            emit_download_progress(&app_handle, &progress);
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum ModelDownloadTaskError {
    #[error("download cancelled")]
    Cancelled,
    #[error(transparent)]
    Failed(#[from] ModelDownloadError),
}

async fn download_and_install_model(
    app_handle: &tauri::AppHandle,
    plan: &DownloadPlan,
    cancel_requested: &AtomicBool,
) -> Result<(), ModelDownloadTaskError> {
    remove_model_dir_if_exists(&plan.install_dir).map_err(ModelDownloadError::Install)?;
    std::fs::create_dir_all(&plan.install_dir).map_err(|source| ModelDownloadError::CreateDir {
        path: plan.install_dir.clone(),
        source,
    })?;
    write_downloading_marker(&plan.models_dir, &plan.provider, &plan.model_id)
        .map_err(ModelDownloadError::Status)?;

    if let ModelArtifactShape::MultiFile { files } = &plan.artifact.shape {
        download_and_install_multi_file_artifact(app_handle, plan, files, cancel_requested).await?;
        return Ok(());
    }

    download_artifact_to_temp(app_handle, plan, cancel_requested).await?;
    if cancel_requested.load(Ordering::SeqCst) {
        return Err(ModelDownloadTaskError::Cancelled);
    }

    emit_download_progress(
        app_handle,
        &OcrModelDownloadProgressDto::new(
            &plan.provider,
            &plan.model_id,
            OcrModelDownloadStatusDto::Installing,
            plan.artifact.byte_size,
            Some(plan.artifact.byte_size),
            Some("validating checksum".to_string()),
        ),
    );

    validate_artifact_sha256(&plan.temp_file_path, &plan.artifact.sha256)
        .map_err(ModelDownloadError::Install)?;
    install_downloaded_model_artifact(&plan.models_dir, &plan.descriptor, &plan.temp_file_path)
        .map_err(ModelDownloadError::Install)?;
    remove_model_file_if_exists(&plan.temp_file_path).map_err(ModelDownloadError::Install)?;

    Ok(())
}

fn staged_download_temp_path(install_dir: &Path, relative_path: &Path, index: usize) -> PathBuf {
    let label = relative_path.to_string_lossy().replace(['/', '\\'], "__");
    install_dir.join(format!(
        ".download-{}-{}-{}",
        std::process::id(),
        index,
        if label.is_empty() { "file" } else { &label }
    ))
}

async fn download_and_install_multi_file_artifact(
    app_handle: &tauri::AppHandle,
    plan: &DownloadPlan,
    files: &[ModelArtifactFile],
    cancel_requested: &AtomicBool,
) -> Result<(), ModelDownloadTaskError> {
    let total_bytes = plan.artifact.byte_size;
    let mut downloaded_total = 0_u64;
    let mut staged_files = Vec::new();

    for (index, file) in files.iter().enumerate() {
        if cancel_requested.load(Ordering::SeqCst) {
            return Err(ModelDownloadTaskError::Cancelled);
        }
        let relative_path = safe_relative_model_file_path(&file.relative_path)?;
        let temp_path = staged_download_temp_path(&plan.install_dir, &relative_path, index);
        let downloaded = download_model_file_to_temp(
            app_handle,
            plan,
            file,
            &temp_path,
            downloaded_total,
            total_bytes,
            cancel_requested,
        )
        .await?;
        downloaded_total = downloaded_total.saturating_add(downloaded);
        validate_artifact_sha256(&temp_path, &file.sha256).map_err(ModelDownloadError::Install)?;
        staged_files.push((temp_path, plan.install_dir.join(relative_path)));
    }

    emit_download_progress(
        app_handle,
        &OcrModelDownloadProgressDto::new(
            &plan.provider,
            &plan.model_id,
            OcrModelDownloadStatusDto::Installing,
            downloaded_total,
            Some(total_bytes),
            Some("installing OCR bundle".to_string()),
        ),
    );

    for (temp_path, destination) in staged_files {
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|source| ModelDownloadError::CreateDir {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        std::fs::rename(&temp_path, &destination)
            .or_else(|_| {
                std::fs::copy(&temp_path, &destination)?;
                std::fs::remove_file(&temp_path)
            })
            .map_err(|source| ModelDownloadError::WriteFile {
                path: destination,
                source,
            })?;
    }

    let missing = match &plan.descriptor.management {
        ModelManagement::AppManaged {
            expected_layout, ..
        } => expected_layout
            .required_files
            .iter()
            .filter(|file| !plan.install_dir.join(file).is_file())
            .cloned()
            .collect::<Vec<_>>(),
        ModelManagement::OsManaged => Vec::new(),
    };
    if !missing.is_empty() {
        return Err(ModelDownloadError::Install(
            ocr::ModelInstallError::IncompleteInstalledLayout {
                missing_files: missing,
            },
        )
        .into());
    }
    remove_model_file_if_exists(plan.install_dir.join(ocr::DOWNLOADING_MARKER_FILE_NAME))
        .map_err(ModelDownloadError::Install)?;
    ocr::write_installed_marker(&plan.models_dir, &plan.provider, &plan.model_id)
        .map_err(ModelDownloadError::Status)?;
    Ok(())
}

async fn download_model_file_to_temp(
    app_handle: &tauri::AppHandle,
    plan: &DownloadPlan,
    file: &ModelArtifactFile,
    temp_file_path: &Path,
    already_downloaded_bytes: u64,
    total_bytes: u64,
    cancel_requested: &AtomicBool,
) -> Result<u64, ModelDownloadTaskError> {
    let response = reqwest::get(&file.url)
        .await
        .map_err(ModelDownloadError::Http)?
        .error_for_status()
        .map_err(ModelDownloadError::Http)?;
    let mut stream = response.bytes_stream();
    let mut output =
        std::fs::File::create(temp_file_path).map_err(|source| ModelDownloadError::CreateFile {
            path: temp_file_path.to_path_buf(),
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
                path: temp_file_path.to_path_buf(),
                source,
            }
        })?;
        file_downloaded = file_downloaded.saturating_add(chunk.len() as u64);
        emit_download_progress(
            app_handle,
            &OcrModelDownloadProgressDto::new(
                &plan.provider,
                &plan.model_id,
                OcrModelDownloadStatusDto::Downloading,
                already_downloaded_bytes.saturating_add(file_downloaded),
                Some(total_bytes),
                Some(file.relative_path.clone()),
            ),
        );
    }
    Ok(file_downloaded)
}

fn safe_relative_model_file_path(relative_path: &str) -> Result<PathBuf, ModelDownloadError> {
    let path = Path::new(relative_path);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(ModelDownloadError::MissingArtifact {
            provider: "invalid model artifact path".to_string(),
            model_id: relative_path.to_string(),
        });
    }
    Ok(path.to_path_buf())
}

async fn download_artifact_to_temp(
    app_handle: &tauri::AppHandle,
    plan: &DownloadPlan,
    cancel_requested: &AtomicBool,
) -> Result<(), ModelDownloadTaskError> {
    let response = reqwest::get(&plan.artifact.url)
        .await
        .map_err(ModelDownloadError::Http)?
        .error_for_status()
        .map_err(ModelDownloadError::Http)?;
    let total_bytes = response.content_length().or(Some(plan.artifact.byte_size));
    let mut stream = response.bytes_stream();
    let mut file = std::fs::File::create(&plan.temp_file_path).map_err(|source| {
        ModelDownloadError::CreateFile {
            path: plan.temp_file_path.clone(),
            source,
        }
    })?;
    let mut downloaded_bytes = 0_u64;

    while let Some(chunk) = stream.next().await {
        if cancel_requested.load(Ordering::SeqCst) {
            return Err(ModelDownloadTaskError::Cancelled);
        }
        let chunk = chunk.map_err(ModelDownloadError::Http)?;
        std::io::Write::write_all(&mut file, &chunk).map_err(|source| {
            ModelDownloadError::WriteFile {
                path: plan.temp_file_path.clone(),
                source,
            }
        })?;
        downloaded_bytes += chunk.len() as u64;
        emit_download_progress(
            app_handle,
            &OcrModelDownloadProgressDto::new(
                &plan.provider,
                &plan.model_id,
                OcrModelDownloadStatusDto::Downloading,
                downloaded_bytes,
                total_bytes,
                None,
            ),
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find_model<'a>(
        response: &'a OcrModelStatusResponseDto,
        provider: &str,
        model_id: Option<&str>,
    ) -> Option<&'a OcrModelStatusDto> {
        response
            .providers
            .iter()
            .flat_map(|provider_status| provider_status.models.iter())
            .find(|model| model.provider == provider && model.model_id.as_deref() == model_id)
    }

    #[test]
    fn selected_ocr_model_availability_tracks_provider_and_runtime() {
        let temp = tempfile::tempdir().expect("tempdir");
        let settings = OcrSettings {
            provider: OcrProvider::Tesseract,
            model_id: Some(ocr::DEFAULT_TESSERACT_MODEL_ID.to_string()),
            language: Some(ocr::DEFAULT_TESSERACT_LANGUAGE.to_string()),
            ..capture_types::default_ocr_settings()
        };
        let available = selected_ocr_model_available(temp.path(), &settings)
            .expect("availability should inspect status");
        assert!(!available);
    }

    #[test]
    fn staged_download_temp_paths_are_unique_for_same_file_names() {
        let temp = tempfile::tempdir().expect("tempdir");
        let det = safe_relative_model_file_path("det/model.mnn").expect("det path");
        let rec = safe_relative_model_file_path("rec/model.mnn").expect("rec path");

        let det_temp = staged_download_temp_path(temp.path(), &det, 0);
        let rec_temp = staged_download_temp_path(temp.path(), &rec, 1);

        assert_ne!(det_temp, rec_temp);
    }

    #[test]
    fn status_response_includes_only_desktop_selectable_ocr_providers() {
        let temp = tempfile::tempdir().expect("tempdir");
        let response = build_ocr_model_status_response(temp.path()).expect("status response");
        assert!(find_model(&response, ocr::APPLE_VISION_PROVIDER_ID, None).is_some());
        assert!(find_model(
            &response,
            ocr::TESSERACT_PROVIDER_ID,
            Some(ocr::DEFAULT_TESSERACT_MODEL_ID),
        )
        .is_some());
        assert!(find_model(
            &response,
            ocr::PADDLE_OCR_PROVIDER_ID,
            Some(ocr::DEFAULT_PADDLE_OCR_MODEL_ID),
        )
        .is_none());
    }
}
