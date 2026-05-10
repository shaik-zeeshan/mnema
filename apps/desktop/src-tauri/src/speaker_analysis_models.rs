use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use speaker_analysis::{
    builtin_model_manifest, detect_model_status, find_model_descriptor, install_model_file,
    model_install_dir, remove_model_dir_if_exists, remove_model_file_if_exists,
    speaker_analysis_models_dir, validate_artifact_sha256, write_downloading_marker,
    write_failed_marker, write_installed_marker, ModelArtifactFile, ModelArtifactShape,
    ModelManagement, ModelStatusKind, SpeakerAnalysisModelDescriptor,
};
use tauri::{Emitter, Manager};

pub const SPEAKER_ANALYSIS_MODEL_DOWNLOAD_PROGRESS_EVENT: &str =
    "speaker_analysis_model_download_progress";

pub type SpeakerAnalysisModelDownloadState = Mutex<Option<ActiveSpeakerAnalysisModelDownload>>;

#[derive(Debug, Clone)]
pub struct ActiveSpeakerAnalysisModelDownload {
    provider: String,
    model_id: String,
    cancel_requested: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerAnalysisModelDownloadRequestDto {
    pub provider: String,
    pub model_id: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpeakerAnalysisModelDownloadStatusDto {
    Starting,
    Downloading,
    Installing,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerAnalysisModelDownloadProgressDto {
    pub provider: String,
    pub model_id: String,
    pub status: SpeakerAnalysisModelDownloadStatusDto,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub message: Option<String>,
}

impl SpeakerAnalysisModelDownloadProgressDto {
    fn new(
        provider: impl Into<String>,
        model_id: impl Into<String>,
        status: SpeakerAnalysisModelDownloadStatusDto,
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
pub struct SpeakerAnalysisModelStatusResponseDto {
    pub models_directory: String,
    pub providers: Vec<SpeakerAnalysisProviderStatusDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerAnalysisProviderStatusDto {
    pub provider: String,
    pub display_name: String,
    pub models: Vec<SpeakerAnalysisModelStatusDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerAnalysisModelStatusDto {
    pub provider: String,
    pub model_id: Option<String>,
    pub display_name: String,
    pub description: String,
    pub status: ModelStatusKind,
    pub available: bool,
    pub install_path: String,
    pub missing_files: Vec<String>,
    pub failure_message: Option<String>,
    pub license_label: Option<String>,
    pub source_url: Option<String>,
    pub download: Option<SpeakerAnalysisModelDownloadDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerAnalysisModelDownloadDto {
    pub url: String,
    pub byte_size: u64,
    pub sha256: Option<String>,
    pub shape: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DownloadPlan {
    provider: String,
    model_id: String,
    models_dir: PathBuf,
    install_dir: PathBuf,
    descriptor: SpeakerAnalysisModelDescriptor,
    files: Vec<ModelArtifactFile>,
    total_bytes: u64,
}

#[derive(Debug, thiserror::Error)]
enum ModelDownloadError {
    #[error("download for {provider}/{model_id} is already running")]
    AlreadyRunning { provider: String, model_id: String },
    #[error("no active speaker analysis model download")]
    NoActiveDownload,
    #[error("speaker analysis model not found for provider={provider}, modelId={model_id}")]
    ModelNotFound { provider: String, model_id: String },
    #[error("model {provider}/{model_id} has no app-managed download artifact")]
    MissingArtifact { provider: String, model_id: String },
    #[error("failed to inspect speaker analysis model status: {0}")]
    Status(#[from] speaker_analysis::ModelStatusError),
    #[error("failed to install speaker analysis model: {0}")]
    Install(#[from] speaker_analysis::ModelInstallError),
    #[error("download failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("failed to create directory {path}: {source}")]
    CreateDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read archive: {0}")]
    Archive(std::io::Error),
    #[error("invalid speaker model artifact path: {0}")]
    InvalidArtifactPath(String),
}

#[tauri::command]
pub fn get_speaker_analysis_model_status(
    app_handle: tauri::AppHandle,
) -> Result<SpeakerAnalysisModelStatusResponseDto, String> {
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|error| format!("failed to resolve app data directory: {error}"))?;
    build_speaker_analysis_model_status_response(&app_data_dir)
        .map_err(|error| format!("failed to inspect speaker analysis model: {error}"))
}

#[tauri::command]
pub fn start_speaker_analysis_model_download(
    app_handle: tauri::AppHandle,
    request: SpeakerAnalysisModelDownloadRequestDto,
    download_state: tauri::State<'_, SpeakerAnalysisModelDownloadState>,
) -> Result<SpeakerAnalysisModelDownloadProgressDto, String> {
    let plan = build_download_plan(&app_handle, &request).map_err(|error| error.to_string())?;
    let cancel_requested = Arc::new(AtomicBool::new(false));
    claim_model_download(
        download_state.inner(),
        &plan.provider,
        &plan.model_id,
        Arc::clone(&cancel_requested),
    )
    .map_err(|error| error.to_string())?;

    let starting = SpeakerAnalysisModelDownloadProgressDto::new(
        &plan.provider,
        &plan.model_id,
        SpeakerAnalysisModelDownloadStatusDto::Starting,
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

#[tauri::command]
pub fn cancel_speaker_analysis_model_download(
    download_state: tauri::State<'_, SpeakerAnalysisModelDownloadState>,
) -> Result<(), String> {
    let active = download_state
        .lock()
        .map_err(|_| "speaker analysis model download state poisoned".to_string())?;
    let Some(active) = active.as_ref() else {
        return Err(ModelDownloadError::NoActiveDownload.to_string());
    };
    active.cancel_requested.store(true, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
pub async fn delete_speaker_analysis_model(
    app_handle: tauri::AppHandle,
    request: SpeakerAnalysisModelDownloadRequestDto,
    download_state: tauri::State<'_, SpeakerAnalysisModelDownloadState>,
    infra: tauri::State<'_, crate::app_infra::AppInfraState>,
) -> Result<(), String> {
    if active_download_key(download_state.inner())?
        == Some(model_key(&request.provider, &request.model_id))
    {
        return Err("cannot delete a speaker analysis model while it is downloading".to_string());
    }
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|error| format!("failed to resolve app data directory: {error}"))?;
    let manifest = builtin_model_manifest();
    let descriptor = find_model_descriptor(&manifest, &request.provider, Some(&request.model_id))
        .ok_or_else(|| {
        ModelDownloadError::ModelNotFound {
            provider: request.provider.clone(),
            model_id: request.model_id.clone(),
        }
        .to_string()
    })?;
    let model_key = model_key(&request.provider, &request.model_id);
    let cleanup_lock = infra
        .acquire_speaker_analysis_model_cleanup_locks(&BTreeSet::from([model_key.clone()]))
        .await
        .map_err(|error| {
            format!("failed to reserve speaker analysis model for cleanup: {error}")
        })?;
    let result = async {
        let protected_model_keys = infra
            .list_active_speaker_analysis_model_keys()
            .await
            .map_err(|error| {
                format!(
                    "failed to inspect queued or running speaker analysis jobs before deleting model: {error}"
                )
            })?;
        if protected_model_keys.contains(&model_key) {
            return Err(
                "cannot delete a speaker analysis model while jobs are queued or running for it"
                    .to_string(),
            );
        }

        let install_dir = model_install_dir(speaker_analysis_models_dir(app_data_dir), descriptor)
            .map_err(|error| error.to_string())?;
        remove_model_dir_if_exists(install_dir).map_err(|error| error.to_string())
    }
    .await;
    let release_result = infra
        .release_processing_model_cleanup_locks(&cleanup_lock)
        .await
        .map_err(|error| {
            format!("failed to release speaker analysis model cleanup reservation: {error}")
        });

    match (result, release_result) {
        (Ok(_), Ok(_)) => Ok(()),
        (Err(error), Ok(_)) => Err(error),
        (Ok(_), Err(error)) | (Err(_), Err(error)) => Err(error),
    }
}

fn build_speaker_analysis_model_status_response(
    app_data_dir: &Path,
) -> Result<SpeakerAnalysisModelStatusResponseDto, speaker_analysis::ModelStatusError> {
    let models_dir = speaker_analysis_models_dir(app_data_dir);
    let manifest = builtin_model_manifest();
    let mut models = Vec::new();

    for descriptor in &manifest.models {
        let status = detect_model_status(&models_dir, descriptor)?;
        let ModelManagement::AppManaged { artifact, .. } = &descriptor.management;
        models.push(SpeakerAnalysisModelStatusDto {
            provider: descriptor.provider.clone(),
            model_id: descriptor.model_id.clone(),
            display_name: descriptor.display_name.clone(),
            description: descriptor.description.clone(),
            available: status.status == ModelStatusKind::Installed,
            status: status.status,
            install_path: status.install_path.to_string_lossy().to_string(),
            missing_files: status.missing_files,
            failure_message: status.failure_message,
            license_label: descriptor.license_label.clone(),
            source_url: descriptor.source_url.clone(),
            download: artifact
                .as_ref()
                .map(|artifact| SpeakerAnalysisModelDownloadDto {
                    url: artifact.url.clone(),
                    byte_size: artifact.byte_size,
                    sha256: artifact.sha256.clone(),
                    shape: serde_json::to_value(&artifact.shape).unwrap_or(serde_json::Value::Null),
                }),
        });
    }

    Ok(SpeakerAnalysisModelStatusResponseDto {
        models_directory: models_dir.to_string_lossy().to_string(),
        providers: vec![SpeakerAnalysisProviderStatusDto {
            provider: speaker_analysis::SHERPA_ONNX_PROVIDER_ID.to_string(),
            display_name: "Sherpa ONNX".to_string(),
            models,
        }],
    })
}

fn build_download_plan(
    app_handle: &tauri::AppHandle,
    request: &SpeakerAnalysisModelDownloadRequestDto,
) -> Result<DownloadPlan, ModelDownloadError> {
    let manifest = builtin_model_manifest();
    let descriptor = find_model_descriptor(&manifest, &request.provider, Some(&request.model_id))
        .cloned()
        .ok_or_else(|| ModelDownloadError::ModelNotFound {
            provider: request.provider.clone(),
            model_id: request.model_id.clone(),
        })?;
    let ModelManagement::AppManaged { artifact, .. } = &descriptor.management;
    let artifact = artifact
        .clone()
        .ok_or_else(|| ModelDownloadError::MissingArtifact {
            provider: request.provider.clone(),
            model_id: request.model_id.clone(),
        })?;
    let ModelArtifactShape::MultiFile { files } = artifact.shape;
    let app_data_dir =
        app_handle
            .path()
            .app_data_dir()
            .map_err(|error| ModelDownloadError::CreateDir {
                path: PathBuf::from("<app_data_dir>"),
                source: std::io::Error::other(error.to_string()),
            })?;
    let models_dir = speaker_analysis_models_dir(app_data_dir);
    let install_dir = model_install_dir(&models_dir, &descriptor)?;
    let total_bytes = files.iter().map(|file| file.byte_size).sum();
    Ok(DownloadPlan {
        provider: request.provider.clone(),
        model_id: request.model_id.clone(),
        models_dir,
        install_dir,
        descriptor,
        files,
        total_bytes,
    })
}

fn claim_model_download(
    state: &SpeakerAnalysisModelDownloadState,
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
    *active = Some(ActiveSpeakerAnalysisModelDownload {
        provider: provider.to_string(),
        model_id: model_id.to_string(),
        cancel_requested,
    });
    Ok(())
}

fn active_download_key(
    state: &SpeakerAnalysisModelDownloadState,
) -> Result<Option<String>, String> {
    let active = state
        .lock()
        .map_err(|_| "speaker analysis model download state poisoned".to_string())?;
    Ok(active
        .as_ref()
        .map(|download| model_key(&download.provider, &download.model_id)))
}

fn model_key(provider: &str, model_id: &str) -> String {
    format!("{provider}/{model_id}")
}

fn clear_active_download(app_handle: &tauri::AppHandle, provider: &str, model_id: &str) {
    if let Ok(mut active) = app_handle
        .state::<SpeakerAnalysisModelDownloadState>()
        .lock()
    {
        if active
            .as_ref()
            .is_some_and(|download| download.provider == provider && download.model_id == model_id)
        {
            *active = None;
        }
    }
}

fn emit_download_progress(
    app_handle: &tauri::AppHandle,
    progress: &SpeakerAnalysisModelDownloadProgressDto,
) {
    let _ = app_handle.emit(SPEAKER_ANALYSIS_MODEL_DOWNLOAD_PROGRESS_EVENT, progress);
}

async fn run_model_download_task(
    app_handle: tauri::AppHandle,
    plan: DownloadPlan,
    cancel_requested: Arc<AtomicBool>,
) {
    let result = download_and_install_model(&app_handle, &plan, &cancel_requested).await;
    clear_active_download(&app_handle, &plan.provider, &plan.model_id);
    match result {
        Ok(()) => emit_download_progress(
            &app_handle,
            &SpeakerAnalysisModelDownloadProgressDto::new(
                &plan.provider,
                &plan.model_id,
                SpeakerAnalysisModelDownloadStatusDto::Completed,
                plan.total_bytes,
                Some(plan.total_bytes),
                None,
            ),
        ),
        Err(ModelDownloadTaskError::Cancelled) => {
            let _ = remove_model_file_if_exists(
                plan.install_dir
                    .join(speaker_analysis::DOWNLOADING_MARKER_FILE_NAME),
            );
            emit_download_progress(
                &app_handle,
                &SpeakerAnalysisModelDownloadProgressDto::new(
                    &plan.provider,
                    &plan.model_id,
                    SpeakerAnalysisModelDownloadStatusDto::Cancelled,
                    0,
                    Some(plan.total_bytes),
                    Some("download cancelled".to_string()),
                ),
            );
        }
        Err(ModelDownloadTaskError::Failed(error)) => {
            let _ = remove_model_file_if_exists(
                plan.install_dir
                    .join(speaker_analysis::DOWNLOADING_MARKER_FILE_NAME),
            );
            let _ = write_failed_marker(
                &plan.models_dir,
                &plan.provider,
                &plan.model_id,
                error.to_string(),
            );
            emit_download_progress(
                &app_handle,
                &SpeakerAnalysisModelDownloadProgressDto::new(
                    &plan.provider,
                    &plan.model_id,
                    SpeakerAnalysisModelDownloadStatusDto::Failed,
                    0,
                    Some(plan.total_bytes),
                    Some(error.to_string()),
                ),
            );
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

    let mut downloaded_total = 0_u64;
    for file in &plan.files {
        if cancel_requested.load(Ordering::SeqCst) {
            return Err(ModelDownloadTaskError::Cancelled);
        }
        let bytes =
            download_model_file(app_handle, plan, file, downloaded_total, cancel_requested).await?;
        downloaded_total = downloaded_total.saturating_add(bytes.len() as u64);
        let destination = safe_relative_model_file_path(&file.relative_path)?;
        install_downloaded_model_file(&plan.install_dir, file, &destination, &bytes)?;
    }

    emit_download_progress(
        app_handle,
        &SpeakerAnalysisModelDownloadProgressDto::new(
            &plan.provider,
            &plan.model_id,
            SpeakerAnalysisModelDownloadStatusDto::Installing,
            downloaded_total,
            Some(plan.total_bytes),
            Some("validating speaker analysis model layout".to_string()),
        ),
    );
    validate_installed_layout(plan)?;
    remove_model_file_if_exists(
        plan.install_dir
            .join(speaker_analysis::DOWNLOADING_MARKER_FILE_NAME),
    )
    .map_err(ModelDownloadError::Install)?;
    write_installed_marker(&plan.models_dir, &plan.provider, &plan.model_id)
        .map_err(ModelDownloadError::Status)?;
    Ok(())
}

async fn download_model_file(
    app_handle: &tauri::AppHandle,
    plan: &DownloadPlan,
    file: &ModelArtifactFile,
    already_downloaded_bytes: u64,
    cancel_requested: &AtomicBool,
) -> Result<Vec<u8>, ModelDownloadTaskError> {
    let response = reqwest::get(&file.url)
        .await
        .map_err(ModelDownloadError::Http)?
        .error_for_status()
        .map_err(ModelDownloadError::Http)?;
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        if cancel_requested.load(Ordering::SeqCst) {
            return Err(ModelDownloadTaskError::Cancelled);
        }
        let chunk = chunk.map_err(ModelDownloadError::Http)?;
        bytes.extend_from_slice(&chunk);
        emit_download_progress(
            app_handle,
            &SpeakerAnalysisModelDownloadProgressDto::new(
                &plan.provider,
                &plan.model_id,
                SpeakerAnalysisModelDownloadStatusDto::Downloading,
                already_downloaded_bytes.saturating_add(bytes.len() as u64),
                if plan.total_bytes == 0 {
                    None
                } else {
                    Some(plan.total_bytes)
                },
                Some(file.relative_path.clone()),
            ),
        );
    }
    Ok(bytes)
}

fn install_downloaded_model_file(
    install_dir: &Path,
    file: &ModelArtifactFile,
    destination_relative_path: &Path,
    bytes: &[u8],
) -> Result<(), ModelDownloadError> {
    let temp_path = install_dir.join(".download.tmp");
    install_model_file(&temp_path, bytes).map_err(ModelDownloadError::Install)?;
    validate_artifact_sha256(&temp_path, file.sha256.as_deref())
        .map_err(ModelDownloadError::Install)?;
    if file.url.ends_with(".tar.bz2") {
        install_from_tar_bz2(install_dir, destination_relative_path, bytes)?;
    } else {
        install_model_file(install_dir.join(destination_relative_path), bytes)
            .map_err(ModelDownloadError::Install)?;
    }
    remove_model_file_if_exists(temp_path).map_err(ModelDownloadError::Install)?;
    Ok(())
}

fn install_from_tar_bz2(
    install_dir: &Path,
    destination_relative_path: &Path,
    bytes: &[u8],
) -> Result<(), ModelDownloadError> {
    let decoder = bzip2::read::BzDecoder::new(bytes);
    let mut archive = tar::Archive::new(decoder);
    let entries = archive.entries().map_err(ModelDownloadError::Archive)?;
    for entry in entries {
        let mut entry = entry.map_err(ModelDownloadError::Archive)?;
        let entry_path = entry.path().map_err(ModelDownloadError::Archive)?;
        if !entry_path.ends_with("model.onnx") {
            continue;
        }
        let destination = install_dir.join(destination_relative_path);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|source| ModelDownloadError::CreateDir {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        entry
            .unpack(&destination)
            .map_err(ModelDownloadError::Archive)?;
        return Ok(());
    }
    Err(ModelDownloadError::InvalidArtifactPath(
        destination_relative_path.display().to_string(),
    ))
}

fn validate_installed_layout(plan: &DownloadPlan) -> Result<(), ModelDownloadError> {
    let ModelManagement::AppManaged {
        expected_layout, ..
    } = &plan.descriptor.management;
    let missing = expected_layout
        .required_files
        .iter()
        .filter(|file| !plan.install_dir.join(file).is_file())
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(
            speaker_analysis::ModelInstallError::IncompleteInstalledLayout {
                missing_files: missing,
            }
            .into(),
        );
    }
    Ok(())
}

fn safe_relative_model_file_path(relative_path: &str) -> Result<PathBuf, ModelDownloadError> {
    let path = Path::new(relative_path);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(ModelDownloadError::InvalidArtifactPath(
            relative_path.to_string(),
        ));
    }
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "mnema-speaker-analysis-models-{label}-{}-{unique}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).expect("test dir should be created");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn tar_bz2_install_validates_checksum_before_unpacking() {
        let dir = TestDir::new("archive-checksum");
        let file = ModelArtifactFile {
            relative_path: "segmentation/model.onnx".to_string(),
            url: "https://example.invalid/model.tar.bz2".to_string(),
            byte_size: 15,
            sha256: Some(
                "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            ),
        };

        let result = install_downloaded_model_file(
            dir.path(),
            &file,
            Path::new("segmentation/model.onnx"),
            b"not a tar bz2 archive",
        );

        assert!(matches!(
            result,
            Err(ModelDownloadError::Install(
                speaker_analysis::ModelInstallError::ChecksumMismatch { .. }
            ))
        ));
        assert!(!dir.path().join("segmentation/model.onnx").exists());
    }
}
