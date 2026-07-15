use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use audio_transcription::{
    audio_transcription_models_dir, builtin_model_manifest, detect_model_status,
    find_model_descriptor, install_downloaded_model_artifact, model_install_dir,
    providers::AppleSpeechOnDeviceAvailabilityStatus, remove_model_dir_if_exists,
    remove_model_file_if_exists, validate_artifact_sha256, write_downloading_marker,
    write_failed_marker, AudioTranscriptionModelDescriptor, AudioTranscriptionModelStatus,
    ModelArtifact, ModelArtifactFile, ModelArtifactShape, ModelManagement, ModelStatusError,
    ModelStatusKind, DOWNLOADING_MARKER_FILE_NAME,
};
use capture_types::{AudioTranscriptionProvider, AudioTranscriptionSettings};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};
use tauri_plugin_opener::OpenerExt;

pub const AUDIO_TRANSCRIPTION_MODEL_DOWNLOAD_PROGRESS_EVENT: &str =
    "audio_transcription_model_download_progress";

pub type AudioTranscriptionModelDownloadState =
    Mutex<Option<ActiveAudioTranscriptionModelDownload>>;

#[derive(Debug, Clone)]
pub struct ActiveAudioTranscriptionModelDownload {
    provider: String,
    model_id: String,
    cancel_requested: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioTranscriptionModelDownloadRequestDto {
    pub provider: String,
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeletedAudioTranscriptionModelDto {
    pub provider: String,
    pub model_id: String,
    pub display_name: String,
    pub install_path: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeleteUnusedAudioTranscriptionModelsResponseDto {
    pub deleted: Vec<DeletedAudioTranscriptionModelDto>,
    pub skipped_active_downloads: Vec<DeletedAudioTranscriptionModelDto>,
    pub skipped_processing_jobs: Vec<DeletedAudioTranscriptionModelDto>,
    pub retargeted_processing_jobs: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AudioTranscriptionModelDownloadStatusDto {
    Starting,
    Downloading,
    Installing,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioTranscriptionModelDownloadProgressDto {
    pub provider: String,
    pub model_id: String,
    pub status: AudioTranscriptionModelDownloadStatusDto,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DownloadPlan {
    provider: String,
    model_id: String,
    models_dir: PathBuf,
    install_dir: PathBuf,
    temp_file_path: PathBuf,
    descriptor: AudioTranscriptionModelDescriptor,
    artifact: ModelArtifact,
}

#[derive(Debug, thiserror::Error)]
enum ModelDownloadError {
    #[error("download for {provider}/{model_id} is already running")]
    AlreadyRunning { provider: String, model_id: String },
    #[error("no active audio transcription model download")]
    NoActiveDownload,
    #[error("audio transcription model not found for provider={provider}, modelId={model_id}")]
    ModelNotFound { provider: String, model_id: String },
    #[error("model {provider}/{model_id} is OS-managed and cannot be downloaded by the app")]
    OsManaged { provider: String, model_id: String },
    #[error("model {provider}/{model_id} has no app-managed download artifact")]
    MissingArtifact { provider: String, model_id: String },
    #[error("failed to inspect audio transcription model status: {0}")]
    Status(#[from] audio_transcription::ModelStatusError),
    #[error("failed to install audio transcription model: {0}")]
    Install(#[from] audio_transcription::ModelInstallError),
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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioTranscriptionModelStatusResponseDto {
    pub models_directory: String,
    pub providers: Vec<AudioTranscriptionProviderStatusDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioTranscriptionProviderStatusDto {
    pub provider: String,
    pub display_name: String,
    pub models: Vec<AudioTranscriptionModelStatusDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioTranscriptionModelStatusDto {
    pub provider: String,
    pub model_id: Option<String>,
    pub display_name: String,
    pub description: String,
    pub management: AudioTranscriptionModelManagementDto,
    pub status: ModelStatusKind,
    pub available: bool,
    pub availability_status: Option<AppleSpeechOnDeviceAvailabilityStatus>,
    pub install_path: Option<String>,
    pub missing_files: Vec<String>,
    pub failure_message: Option<String>,
    pub license_label: Option<String>,
    pub source_url: Option<String>,
    pub download: Option<AudioTranscriptionModelDownloadDto>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AudioTranscriptionModelManagementDto {
    AppManaged,
    OsManaged,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioTranscriptionModelDownloadDto {
    pub url: String,
    pub byte_size: u64,
    pub sha256: String,
    pub shape: serde_json::Value,
}

#[tauri::command]
pub fn get_audio_transcription_model_status(
    app_handle: tauri::AppHandle,
) -> Result<AudioTranscriptionModelStatusResponseDto, String> {
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|error| format!("failed to resolve app data directory: {error}"))?;

    build_audio_transcription_model_status_response(&app_data_dir)
        .map_err(|error| format!("failed to inspect audio transcription models: {error}"))
}

#[tauri::command]
pub fn start_audio_transcription_model_download(
    app_handle: tauri::AppHandle,
    request: AudioTranscriptionModelDownloadRequestDto,
    download_state: tauri::State<'_, AudioTranscriptionModelDownloadState>,
    infra: tauri::State<'_, crate::app_infra::AppInfraState>,
) -> Result<AudioTranscriptionModelDownloadProgressDto, String> {
    let plan = build_download_plan(&app_handle, &request).map_err(|error| error.to_string())?;
    let cancel_requested = Arc::new(AtomicBool::new(false));

    claim_model_download(
        download_state.inner(),
        &plan.provider,
        &plan.model_id,
        Arc::clone(&cancel_requested),
    )
    .map_err(|error| error.to_string())?;

    let starting = AudioTranscriptionModelDownloadProgressDto::new(
        &plan.provider,
        &plan.model_id,
        AudioTranscriptionModelDownloadStatusDto::Starting,
        0,
        Some(plan.artifact.byte_size),
        None,
    );
    emit_download_progress(&app_handle, &starting);

    let app_for_task = app_handle.clone();
    let infra_for_task = Arc::clone(&infra);
    tauri::async_runtime::spawn(async move {
        run_model_download_task(app_for_task, plan, cancel_requested, infra_for_task).await;
    });

    Ok(starting)
}

#[tauri::command]
pub fn cancel_audio_transcription_model_download(
    download_state: tauri::State<'_, AudioTranscriptionModelDownloadState>,
) -> Result<(), String> {
    let active = download_state
        .lock()
        .map_err(|_| "audio transcription model download state poisoned".to_string())?;
    let Some(active) = active.as_ref() else {
        return Err(ModelDownloadError::NoActiveDownload.to_string());
    };
    active.cancel_requested.store(true, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
pub async fn delete_unused_audio_transcription_models(
    app_handle: tauri::AppHandle,
    download_state: tauri::State<'_, AudioTranscriptionModelDownloadState>,
    settings: tauri::State<'_, crate::native_capture::RecordingSettingsState>,
    infra: tauri::State<'_, crate::app_infra::AppInfraState>,
) -> Result<DeleteUnusedAudioTranscriptionModelsResponseDto, String> {
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|error| format!("failed to resolve app data directory: {error}"))?;
    let settings = crate::native_capture::read_recording_settings(settings.inner());
    let selected_provider = provider_id_for_settings(settings.transcription.provider);
    let selected_model_id = settings.transcription.model_id.as_deref();
    let active_download = active_download_key(download_state.inner())?;
    let deletion_candidate_model_keys = unused_installed_audio_transcription_model_keys(
        &app_data_dir,
        selected_provider,
        selected_model_id,
        active_download.as_deref(),
    )
    .map_err(|error| format!("failed to inspect unused transcription models: {error}"))?;
    let running_job_model_keys = infra
        .list_running_audio_transcription_model_keys()
        .await
        .map_err(|error| format!("failed to inspect running transcription jobs: {error}"))?;
    let retarget_candidate_model_keys =
        retargetable_deletion_model_keys(&deletion_candidate_model_keys, &running_job_model_keys);
    let cleanup_lock = infra
        .acquire_audio_transcription_model_cleanup_locks(&retarget_candidate_model_keys)
        .await
        .map_err(|error| format!("failed to reserve transcription models for cleanup: {error}"))?;
    let result = async {
        let retargeted_processing_jobs = infra
            .retarget_audio_transcription_jobs_referencing_model_keys(
                &cleanup_lock.acquired_model_keys,
                selected_provider,
                selected_model_id,
            )
            .await
            .map_err(|error| format!("failed to retarget queued transcription jobs: {error}"))?;
        let mut protected_model_keys = infra
            .list_running_audio_transcription_model_keys()
            .await
            .map_err(|error| {
                format!(
                    "failed to re-check running transcription jobs before deleting models: {error}"
                )
            })?;
        protected_model_keys.extend(
            deletion_candidate_model_keys
                .difference(&cleanup_lock.acquired_model_keys)
                .cloned(),
        );

        delete_unused_audio_transcription_models_inner(
            &app_data_dir,
            selected_provider,
            selected_model_id,
            active_download.as_deref(),
            &protected_model_keys,
            retargeted_processing_jobs,
        )
        .map_err(|error| format!("failed to delete unused audio transcription models: {error}"))
    }
    .await;
    let release_result = infra
        .release_processing_model_cleanup_locks(&cleanup_lock)
        .await
        .map_err(|error| {
            format!("failed to release transcription model cleanup reservation: {error}")
        });

    match (result, release_result) {
        (Ok(response), Ok(_)) => Ok(response),
        (Err(error), Ok(_)) => Err(error),
        (Ok(_), Err(error)) | (Err(_), Err(error)) => Err(error),
    }
}

#[tauri::command]
pub fn request_apple_speech_recognition_permission(
    app_handle: tauri::AppHandle,
) -> Result<AudioTranscriptionModelStatusResponseDto, String> {
    let availability =
        audio_transcription::providers::AppleSpeechOnDeviceProvider::request_permission();
    if matches!(
        availability.status,
        AppleSpeechOnDeviceAvailabilityStatus::FrameworkUnavailable
    ) {
        return Err(availability.message);
    }
    get_audio_transcription_model_status(app_handle)
}

#[tauri::command]
pub fn open_apple_speech_recognition_privacy_settings(
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    app_handle
        .opener()
        .open_url(
            "x-apple.systempreferences:com.apple.preference.security?Privacy_SpeechRecognition",
            None::<String>,
        )
        .map_err(|error| format!("failed to open Apple Speech privacy settings: {error}"))
}

pub(crate) fn transcription_request_options_for_settings(
    settings: &AudioTranscriptionSettings,
) -> serde_json::Map<String, serde_json::Value> {
    let mut options = serde_json::Map::new();
    if settings.provider == AudioTranscriptionProvider::Parakeet {
        options.insert(
            "parakeetOnnxMemoryMode".to_string(),
            serde_json::Value::String(
                match settings.memory_mode {
                    capture_types::AudioTranscriptionMemoryMode::Balanced => "balanced",
                    capture_types::AudioTranscriptionMemoryMode::LowMemory => "low_memory",
                    capture_types::AudioTranscriptionMemoryMode::Performance => "performance",
                }
                .to_string(),
            ),
        );
        options.insert(
            "parakeetOnnxIdleUnloadSeconds".to_string(),
            serde_json::Value::Number(settings.idle_unload_seconds.into()),
        );
        options.insert(
            "parakeetOnnxChunkSeconds".to_string(),
            serde_json::Value::Number(settings.chunk_seconds.into()),
        );
    }
    options
}

pub(crate) fn selected_audio_transcription_model_available(
    app_data_dir: &Path,
    settings: &AudioTranscriptionSettings,
) -> Result<bool, ModelStatusError> {
    if !settings.enabled {
        return Ok(false);
    }

    let provider = provider_id_for_settings(settings.provider);
    let manifest = builtin_model_manifest();
    let Some(descriptor) = manifest.models.iter().find(|descriptor| {
        descriptor.provider == provider
            && descriptor.model_id.as_deref() == settings.model_id.as_deref()
    }) else {
        return Ok(false);
    };

    let status = detect_model_status(audio_transcription_models_dir(app_data_dir), descriptor)?;
    if !status.is_available() {
        return Ok(false);
    }
    if descriptor.provider == audio_transcription::APPLE_SPEECH_ON_DEVICE_PROVIDER_ID {
        return Ok(
            audio_transcription::providers::AppleSpeechOnDeviceProvider::availability_for_language(
                &settings.language,
            )
            .available,
        );
    }
    if descriptor.provider == audio_transcription::PARAKEET_PROVIDER_ID {
        let Some(model_path) = model_file_path_from_status(descriptor, &status) else {
            return Ok(false);
        };
        return Ok(
            audio_transcription::providers::ParakeetProvider::availability_for_model_path_and_id(
                model_path,
                descriptor.model_id.as_deref().unwrap_or_default(),
            )
            .available,
        );
    }
    if descriptor.provider == audio_transcription::DEEPGRAM_PROVIDER_ID {
        // ADR 0047: Deepgram availability = an API key is present (the manifest
        // entry is OsManaged, so `detect_model_status` reports it always installed).
        return Ok(
            match app_infra::has_ai_provider_key(
                crate::transcription_deepgram::DEEPGRAM_KEY_ACCOUNT,
            ) {
                Ok(present) => present,
                // ADR 0048 amendment: a vault-denied read is NOT "no key". Report
                // available so segments still enqueue transcription jobs; each job
                // then parks as transient liveness at transcribe time and recovers
                // on a later launch, instead of silently skipping transcription.
                Err(app_infra::AppInfraError::SecretVaultDenied(_)) => true,
                Err(_) => false,
            },
        );
    }

    Ok(true)
}

fn claim_model_download(
    download_state: &AudioTranscriptionModelDownloadState,
    provider: &str,
    model_id: &str,
    cancel_requested: Arc<AtomicBool>,
) -> Result<(), ModelDownloadError> {
    let mut active = download_state
        .lock()
        .map_err(|_| ModelDownloadError::NoActiveDownload)?;
    if let Some(existing) = active.as_ref() {
        return Err(ModelDownloadError::AlreadyRunning {
            provider: existing.provider.clone(),
            model_id: existing.model_id.clone(),
        });
    }
    *active = Some(ActiveAudioTranscriptionModelDownload {
        provider: provider.to_string(),
        model_id: model_id.to_string(),
        cancel_requested,
    });
    Ok(())
}

fn active_download_key(
    download_state: &AudioTranscriptionModelDownloadState,
) -> Result<Option<String>, String> {
    let active = download_state
        .lock()
        .map_err(|_| "audio transcription model download state poisoned".to_string())?;
    Ok(active
        .as_ref()
        .map(|download| model_key(&download.provider, &download.model_id)))
}

fn model_key(provider: &str, model_id: &str) -> String {
    format!("{provider}/{model_id}")
}

#[derive(Debug, thiserror::Error)]
enum DeleteUnusedAudioTranscriptionModelsError {
    #[error(transparent)]
    Status(#[from] ModelStatusError),
    #[error(transparent)]
    Install(#[from] audio_transcription::ModelInstallError),
}

fn delete_unused_audio_transcription_models_inner(
    app_data_dir: &Path,
    selected_provider: &str,
    selected_model_id: Option<&str>,
    active_download: Option<&str>,
    running_job_model_keys: &BTreeSet<String>,
    retargeted_processing_jobs: u64,
) -> Result<
    DeleteUnusedAudioTranscriptionModelsResponseDto,
    DeleteUnusedAudioTranscriptionModelsError,
> {
    let models_dir = audio_transcription_models_dir(app_data_dir);
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

        let candidate = DeletedAudioTranscriptionModelDto {
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

    Ok(DeleteUnusedAudioTranscriptionModelsResponseDto {
        deleted,
        skipped_active_downloads,
        skipped_processing_jobs,
        retargeted_processing_jobs,
    })
}

fn unused_installed_audio_transcription_model_keys(
    app_data_dir: &Path,
    selected_provider: &str,
    selected_model_id: Option<&str>,
    active_download: Option<&str>,
) -> Result<BTreeSet<String>, DeleteUnusedAudioTranscriptionModelsError> {
    let models_dir = audio_transcription_models_dir(app_data_dir);
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

impl AudioTranscriptionModelDownloadProgressDto {
    fn new(
        provider: &str,
        model_id: &str,
        status: AudioTranscriptionModelDownloadStatusDto,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
        message: Option<String>,
    ) -> Self {
        Self {
            provider: provider.to_string(),
            model_id: model_id.to_string(),
            status,
            downloaded_bytes,
            total_bytes,
            message,
        }
    }
}

fn build_download_plan(
    app_handle: &tauri::AppHandle,
    request: &AudioTranscriptionModelDownloadRequestDto,
) -> Result<DownloadPlan, ModelDownloadError> {
    let app_data_dir =
        app_handle
            .path()
            .app_data_dir()
            .map_err(|source| ModelDownloadError::CreateDir {
                path: PathBuf::from("<app_data_dir>"),
                source: std::io::Error::other(source.to_string()),
            })?;
    let models_dir = audio_transcription_models_dir(&app_data_dir);
    let manifest = builtin_model_manifest();
    let descriptor = find_model_descriptor(
        &manifest,
        &request.provider,
        Some(request.model_id.as_str()),
    )
    .ok_or_else(|| ModelDownloadError::ModelNotFound {
        provider: request.provider.clone(),
        model_id: request.model_id.clone(),
    })?
    .clone();

    let artifact = match &descriptor.management {
        ModelManagement::AppManaged {
            artifact: Some(artifact),
            ..
        } => artifact.clone(),
        ModelManagement::AppManaged { artifact: None, .. } => {
            return Err(ModelDownloadError::MissingArtifact {
                provider: request.provider.clone(),
                model_id: request.model_id.clone(),
            });
        }
        ModelManagement::OsManaged => {
            return Err(ModelDownloadError::OsManaged {
                provider: request.provider.clone(),
                model_id: request.model_id.clone(),
            });
        }
    };

    let install_dir = model_install_dir(&models_dir, &request.provider, &request.model_id)?;
    let temp_file_path = install_dir.join(format!(".download-{}.tmp", std::process::id()));

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

async fn run_model_download_task(
    app_handle: tauri::AppHandle,
    plan: DownloadPlan,
    cancel_requested: Arc<AtomicBool>,
    infra: crate::app_infra::AppInfraState,
) {
    let result = download_and_install_model(&app_handle, &plan, &cancel_requested).await;
    clear_active_download(&app_handle, &plan.provider, &plan.model_id);

    match result {
        Ok(()) => {
            let progress = AudioTranscriptionModelDownloadProgressDto::new(
                &plan.provider,
                &plan.model_id,
                AudioTranscriptionModelDownloadStatusDto::Completed,
                plan.artifact.byte_size,
                Some(plan.artifact.byte_size),
                None,
            );
            emit_download_progress(&app_handle, &progress);
            crate::app_infra::run_audio_transcription_backfill_after_model_install(
                &infra,
                &app_handle,
            )
            .await;
        }
        Err(ModelDownloadTaskError::Cancelled) => {
            let _ = remove_model_file_if_exists(&plan.temp_file_path);
            let _ =
                remove_model_file_if_exists(plan.install_dir.join(DOWNLOADING_MARKER_FILE_NAME));
            let progress = AudioTranscriptionModelDownloadProgressDto::new(
                &plan.provider,
                &plan.model_id,
                AudioTranscriptionModelDownloadStatusDto::Cancelled,
                0,
                Some(plan.artifact.byte_size),
                Some("download cancelled".to_string()),
            );
            emit_download_progress(&app_handle, &progress);
        }
        Err(ModelDownloadTaskError::Failed(error)) => {
            let _ = remove_model_file_if_exists(&plan.temp_file_path);
            let _ =
                remove_model_file_if_exists(plan.install_dir.join(DOWNLOADING_MARKER_FILE_NAME));
            let _ = write_failed_marker(
                &plan.models_dir,
                &plan.provider,
                &plan.model_id,
                error.to_string(),
            );
            let progress = AudioTranscriptionModelDownloadProgressDto::new(
                &plan.provider,
                &plan.model_id,
                AudioTranscriptionModelDownloadStatusDto::Failed,
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
        &AudioTranscriptionModelDownloadProgressDto::new(
            &plan.provider,
            &plan.model_id,
            AudioTranscriptionModelDownloadStatusDto::Installing,
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
        &AudioTranscriptionModelDownloadProgressDto::new(
            &plan.provider,
            &plan.model_id,
            AudioTranscriptionModelDownloadStatusDto::Installing,
            downloaded_total,
            Some(total_bytes),
            Some("installing ONNX bundle".to_string()),
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
            audio_transcription::ModelInstallError::IncompleteInstalledLayout {
                missing_files: missing,
            },
        )
        .into());
    }
    remove_model_file_if_exists(plan.install_dir.join(DOWNLOADING_MARKER_FILE_NAME))
        .map_err(ModelDownloadError::Install)?;
    audio_transcription::write_installed_marker(&plan.models_dir, &plan.provider, &plan.model_id)
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
            &AudioTranscriptionModelDownloadProgressDto::new(
                &plan.provider,
                &plan.model_id,
                AudioTranscriptionModelDownloadStatusDto::Downloading,
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
            &AudioTranscriptionModelDownloadProgressDto::new(
                &plan.provider,
                &plan.model_id,
                AudioTranscriptionModelDownloadStatusDto::Downloading,
                downloaded_bytes,
                total_bytes,
                None,
            ),
        );
    }

    Ok(())
}

fn clear_active_download(app_handle: &tauri::AppHandle, provider: &str, model_id: &str) {
    if let Some(state) = app_handle.try_state::<AudioTranscriptionModelDownloadState>() {
        if let Ok(mut active) = state.lock() {
            if active.as_ref().is_some_and(|download| {
                download.provider == provider && download.model_id == model_id
            }) {
                *active = None;
            }
        }
    }
}

fn emit_download_progress(
    app_handle: &tauri::AppHandle,
    progress: &AudioTranscriptionModelDownloadProgressDto,
) {
    let _ = app_handle.emit(
        AUDIO_TRANSCRIPTION_MODEL_DOWNLOAD_PROGRESS_EVENT,
        progress.clone(),
    );
}

pub(crate) fn provider_id_for_settings(provider: AudioTranscriptionProvider) -> &'static str {
    match provider {
        AudioTranscriptionProvider::LocalWhisper => audio_transcription::LOCAL_WHISPER_PROVIDER_ID,
        AudioTranscriptionProvider::AppleSpeechOnDevice => {
            audio_transcription::APPLE_SPEECH_ON_DEVICE_PROVIDER_ID
        }
        AudioTranscriptionProvider::Parakeet => audio_transcription::PARAKEET_PROVIDER_ID,
        AudioTranscriptionProvider::Deepgram => audio_transcription::DEEPGRAM_PROVIDER_ID,
    }
}

fn build_audio_transcription_model_status_response(
    app_data_dir: &Path,
) -> Result<AudioTranscriptionModelStatusResponseDto, ModelStatusError> {
    let models_dir = audio_transcription_models_dir(app_data_dir);
    let manifest = builtin_model_manifest();
    let mut grouped: BTreeMap<String, Vec<AudioTranscriptionModelStatusDto>> = BTreeMap::new();

    for descriptor in manifest.models {
        let status = detect_model_status(&models_dir, &descriptor)?;
        grouped
            .entry(descriptor.provider.clone())
            .or_default()
            .push(model_status_dto(descriptor, status));
    }

    let providers = grouped
        .into_iter()
        .map(|(provider, models)| AudioTranscriptionProviderStatusDto {
            display_name: provider_display_name(&provider).to_string(),
            provider,
            models,
        })
        .collect();

    Ok(AudioTranscriptionModelStatusResponseDto {
        models_directory: path_to_string(&models_dir),
        providers,
    })
}

fn model_status_dto(
    descriptor: AudioTranscriptionModelDescriptor,
    status: AudioTranscriptionModelStatus,
) -> AudioTranscriptionModelStatusDto {
    let (management, download) = match &descriptor.management {
        ModelManagement::AppManaged { artifact, .. } => (
            AudioTranscriptionModelManagementDto::AppManaged,
            artifact
                .as_ref()
                .map(|artifact| AudioTranscriptionModelDownloadDto {
                    url: artifact.url.clone(),
                    byte_size: artifact.byte_size,
                    sha256: artifact.sha256.clone(),
                    shape: serde_json::to_value(&artifact.shape)
                        .unwrap_or_else(|_| serde_json::Value::Null),
                }),
        ),
        ModelManagement::OsManaged => (AudioTranscriptionModelManagementDto::OsManaged, None),
    };

    let mut available = status.is_available();
    let mut failure_message = status.failure_message.clone();
    let mut availability_status = None;
    if descriptor.provider == audio_transcription::APPLE_SPEECH_ON_DEVICE_PROVIDER_ID {
        let availability =
            audio_transcription::providers::AppleSpeechOnDeviceProvider::availability();
        availability_status = Some(availability.status);
        available = available && availability.available;
        if !availability.available && failure_message.is_none() {
            failure_message = Some(availability.message);
        }
    }
    if available && descriptor.provider == audio_transcription::PARAKEET_PROVIDER_ID {
        if let Some(model_path) = model_file_path_from_status(&descriptor, &status) {
            if let Some(model_id) = descriptor.model_id.as_deref() {
                let availability = audio_transcription::providers::ParakeetProvider::availability_for_model_path_and_id(model_path, model_id);
                available = availability.available;
                if !availability.available && failure_message.is_none() {
                    failure_message = Some(availability.message);
                }
            } else {
                available = false;
                if failure_message.is_none() {
                    failure_message = Some("Parakeet model id is unavailable".to_string());
                }
            }
        } else {
            available = false;
            if failure_message.is_none() {
                failure_message = Some("Parakeet model install path is unavailable".to_string());
            }
        }
    }
    if descriptor.provider == audio_transcription::DEEPGRAM_PROVIDER_ID {
        // ADR 0047: availability = API key present. The OsManaged entry reports
        // always-installed, so key presence is the real gate.
        match app_infra::has_ai_provider_key(crate::transcription_deepgram::DEEPGRAM_KEY_ACCOUNT) {
            Ok(present) => {
                available = present;
                if !available && failure_message.is_none() {
                    failure_message = Some(
                        "Add a Deepgram API key in Settings to enable cloud transcription."
                            .to_string(),
                    );
                }
            }
            // Denied ≠ missing (ADR 0048 amendment): show the vault error itself,
            // never the misleading "add a key" prompt.
            Err(error) => {
                available = false;
                if failure_message.is_none() {
                    failure_message = Some(error.to_string());
                }
            }
        }
    }

    AudioTranscriptionModelStatusDto {
        provider: status.provider,
        model_id: status.model_id,
        display_name: descriptor.display_name,
        description: descriptor.description,
        management,
        status: status.status,
        available,
        availability_status,
        install_path: status.install_path.as_deref().map(path_to_string),
        missing_files: status.missing_files,
        failure_message,
        license_label: descriptor.license_label,
        source_url: descriptor.source_url,
        download,
    }
}

fn model_file_path_from_status(
    descriptor: &AudioTranscriptionModelDescriptor,
    status: &AudioTranscriptionModelStatus,
) -> Option<PathBuf> {
    let install_path = status.install_path.as_ref()?;
    match &descriptor.management {
        ModelManagement::AppManaged {
            expected_layout, ..
        } => expected_layout
            .required_files
            .first()
            .map(|file_name| install_path.join(file_name)),
        ModelManagement::OsManaged => None,
    }
}

fn provider_display_name(provider: &str) -> &'static str {
    match provider {
        audio_transcription::LOCAL_WHISPER_PROVIDER_ID => "Local Whisper",
        audio_transcription::APPLE_SPEECH_ON_DEVICE_PROVIDER_ID => "Apple Speech (on-device)",
        audio_transcription::PARAKEET_PROVIDER_ID => "Parakeet",
        audio_transcription::DEEPGRAM_PROVIDER_ID => "Deepgram (cloud)",
        _ => "Unknown provider",
    }
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use audio_transcription::{
        model_install_dir, write_failed_marker, write_installed_marker,
        DOWNLOADING_MARKER_FILE_NAME, LOCAL_WHISPER_PROVIDER_ID,
    };
    use std::fs;

    fn find_model<'a>(
        response: &'a AudioTranscriptionModelStatusResponseDto,
        provider: &str,
        model_id: Option<&str>,
    ) -> &'a AudioTranscriptionModelStatusDto {
        response
            .providers
            .iter()
            .flat_map(|provider_status| provider_status.models.iter())
            .find(|model| model.provider == provider && model.model_id.as_deref() == model_id)
            .expect("model status")
    }

    #[test]
    fn status_response_uses_app_data_models_directory_not_save_directory() {
        let temp = tempfile::tempdir().expect("tempdir");
        let app_data_dir = temp.path().join("app-data");
        let save_directory = temp.path().join("capture-save-directory");

        let response = build_audio_transcription_model_status_response(&app_data_dir)
            .expect("status response");

        assert_eq!(
            response.models_directory,
            path_to_string(&app_data_dir.join(audio_transcription::MODEL_STORE_DIR_NAME))
        );
        assert!(!Path::new(&response.models_directory).starts_with(&save_directory));
    }

    #[test]
    fn status_response_includes_provider_and_model_options() {
        let temp = tempfile::tempdir().expect("tempdir");
        let response =
            build_audio_transcription_model_status_response(temp.path()).expect("status response");

        let local_whisper = response
            .providers
            .iter()
            .find(|provider| provider.provider == audio_transcription::LOCAL_WHISPER_PROVIDER_ID)
            .expect("local whisper provider");
        let model_ids: Vec<_> = local_whisper
            .models
            .iter()
            .map(|model| model.model_id.as_deref())
            .collect();
        assert_eq!(
            model_ids,
            vec![Some("tiny"), Some("base"), Some("small"), Some("medium")]
        );

        let base = find_model(
            &response,
            audio_transcription::LOCAL_WHISPER_PROVIDER_ID,
            Some("base"),
        );
        let download = base.download.as_ref().expect("base download artifact");
        assert_eq!(download.byte_size, 147_951_465);
        assert_eq!(
            download.sha256,
            "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe"
        );
        assert_eq!(
            download.url,
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin"
        );

        let parakeet = find_model(
            &response,
            audio_transcription::PARAKEET_PROVIDER_ID,
            Some("parakeet-tdt-0.6b-v3-onnx"),
        );
        let parakeet_download = parakeet
            .download
            .as_ref()
            .expect("parakeet download artifact");
        assert_eq!(parakeet_download.byte_size, 2_549_945_719);
        assert_eq!(
            parakeet_download.url,
            "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx"
        );

        let apple = find_model(
            &response,
            audio_transcription::APPLE_SPEECH_ON_DEVICE_PROVIDER_ID,
            None,
        );
        assert_eq!(
            apple.management,
            AudioTranscriptionModelManagementDto::OsManaged
        );
        assert_eq!(apple.status, ModelStatusKind::OsManaged);
        assert_eq!(apple.install_path, None);
        assert_eq!(
            apple.availability_status,
            Some(
                audio_transcription::providers::AppleSpeechOnDeviceProvider::availability().status
            )
        );
    }

    #[test]
    fn selected_apple_speech_model_tracks_provider_availability() {
        let temp = tempfile::tempdir().expect("tempdir");
        let settings = capture_types::AudioTranscriptionSettings {
            enabled: true,
            microphone_enabled: capture_types::default_audio_transcription_microphone_enabled(),
            system_audio_enabled: capture_types::default_audio_transcription_system_audio_enabled(),
            provider: capture_types::AudioTranscriptionProvider::AppleSpeechOnDevice,
            model_id: None,
            language: "auto".to_string(),
            memory_mode: capture_types::default_audio_transcription_memory_mode(),
            idle_unload_seconds: capture_types::default_audio_transcription_idle_unload_seconds(),
            chunk_seconds: capture_types::default_audio_transcription_chunk_seconds(),
        };

        assert_eq!(
            selected_audio_transcription_model_available(temp.path(), &settings)
                .expect("apple speech availability should inspect provider status"),
            audio_transcription::providers::AppleSpeechOnDeviceProvider::availability_for_language(
                &settings.language,
            )
            .available
        );
    }

    #[test]
    fn only_one_model_download_can_be_claimed_at_a_time() {
        let state = AudioTranscriptionModelDownloadState::default();
        let first_cancel = Arc::new(AtomicBool::new(false));
        claim_model_download(
            &state,
            LOCAL_WHISPER_PROVIDER_ID,
            "base",
            Arc::clone(&first_cancel),
        )
        .expect("first claim");

        let second = claim_model_download(
            &state,
            LOCAL_WHISPER_PROVIDER_ID,
            "small",
            Arc::new(AtomicBool::new(false)),
        )
        .expect_err("second claim should fail");

        assert!(matches!(second, ModelDownloadError::AlreadyRunning { .. }));
        let active = state.lock().expect("state");
        assert_eq!(active.as_ref().expect("active").model_id, "base");
    }

    #[test]
    fn staged_download_temp_paths_are_unique_for_same_file_names() {
        let temp = tempfile::tempdir().expect("tempdir");
        let encoder = safe_relative_model_file_path("encoder/model.onnx").expect("encoder path");
        let decoder = safe_relative_model_file_path("decoder/model.onnx").expect("decoder path");

        let encoder_temp = staged_download_temp_path(temp.path(), &encoder, 0);
        let decoder_temp = staged_download_temp_path(temp.path(), &decoder, 1);

        assert_ne!(encoder_temp, decoder_temp);
    }

    #[test]
    fn status_response_marks_installed_parakeet_int8_model_available() {
        let temp = tempfile::tempdir().expect("tempdir");
        let app_data_dir = temp.path();
        let models_dir = audio_transcription_models_dir(app_data_dir);
        let install_dir = model_install_dir(
            &models_dir,
            audio_transcription::PARAKEET_PROVIDER_ID,
            "parakeet-tdt-0.6b-v3-onnx-int8",
        )
        .expect("parakeet int8 dir");
        fs::create_dir_all(&install_dir).expect("install dir");
        for file_name in [
            "config.json",
            "nemo128.onnx",
            "encoder-model.int8.onnx",
            "decoder_joint-model.int8.onnx",
            "vocab.txt",
        ] {
            fs::write(install_dir.join(file_name), b"model").expect("model file");
        }
        write_installed_marker(
            &models_dir,
            audio_transcription::PARAKEET_PROVIDER_ID,
            "parakeet-tdt-0.6b-v3-onnx-int8",
        )
        .expect("installed marker");

        let response =
            build_audio_transcription_model_status_response(app_data_dir).expect("status response");
        let model = find_model(
            &response,
            audio_transcription::PARAKEET_PROVIDER_ID,
            Some("parakeet-tdt-0.6b-v3-onnx-int8"),
        );

        assert_eq!(model.status, ModelStatusKind::Installed);
        assert!(model.available);
        assert_eq!(model.failure_message, None);
    }

    #[test]
    fn status_response_reports_app_managed_statuses() {
        let temp = tempfile::tempdir().expect("tempdir");
        let app_data_dir = temp.path();
        let models_dir = audio_transcription_models_dir(app_data_dir);

        write_failed_marker(
            &models_dir,
            LOCAL_WHISPER_PROVIDER_ID,
            "tiny",
            "checksum mismatch",
        )
        .expect("failed marker");

        let base_dir =
            model_install_dir(&models_dir, LOCAL_WHISPER_PROVIDER_ID, "base").expect("base dir");
        fs::create_dir_all(&base_dir).expect("base dir");
        fs::write(base_dir.join(DOWNLOADING_MARKER_FILE_NAME), b"").expect("downloading marker");

        let small_dir =
            model_install_dir(&models_dir, LOCAL_WHISPER_PROVIDER_ID, "small").expect("small dir");
        fs::create_dir_all(&small_dir).expect("small dir");
        fs::write(small_dir.join("ggml-small.bin"), b"model").expect("model file");
        write_installed_marker(&models_dir, LOCAL_WHISPER_PROVIDER_ID, "small")
            .expect("installed marker");

        let response =
            build_audio_transcription_model_status_response(app_data_dir).expect("status response");

        assert_eq!(
            find_model(&response, LOCAL_WHISPER_PROVIDER_ID, Some("tiny")).status,
            ModelStatusKind::Failed
        );
        assert_eq!(
            find_model(&response, LOCAL_WHISPER_PROVIDER_ID, Some("tiny"))
                .failure_message
                .as_deref(),
            Some("checksum mismatch")
        );
        assert_eq!(
            find_model(&response, LOCAL_WHISPER_PROVIDER_ID, Some("base")).status,
            ModelStatusKind::Downloading
        );
        assert_eq!(
            find_model(&response, LOCAL_WHISPER_PROVIDER_ID, Some("small")).status,
            ModelStatusKind::Installed
        );
        assert_eq!(
            find_model(&response, LOCAL_WHISPER_PROVIDER_ID, Some("medium")).status,
            ModelStatusKind::Missing
        );
    }

    #[test]
    fn delete_unused_audio_transcription_models_preserves_models_referenced_by_processing_jobs() {
        let temp = tempfile::tempdir().expect("tempdir");
        let app_data_dir = temp.path();
        let models_dir = audio_transcription_models_dir(app_data_dir);
        let install_dir =
            model_install_dir(&models_dir, LOCAL_WHISPER_PROVIDER_ID, "base").expect("base dir");
        std::fs::create_dir_all(&install_dir).expect("install dir should be created");
        std::fs::write(install_dir.join("ggml-base.bin"), b"model").expect("model file");
        let protected_models = BTreeSet::from([model_key(LOCAL_WHISPER_PROVIDER_ID, "base")]);

        let response = delete_unused_audio_transcription_models_inner(
            app_data_dir,
            audio_transcription::APPLE_SPEECH_ON_DEVICE_PROVIDER_ID,
            None,
            None,
            &protected_models,
            0,
        )
        .expect("unused model deletion should succeed");

        assert!(install_dir.exists());
        assert!(response.deleted.is_empty());
        assert_eq!(response.skipped_processing_jobs.len(), 1);
        assert_eq!(
            response.skipped_processing_jobs[0].provider,
            LOCAL_WHISPER_PROVIDER_ID
        );
        assert_eq!(response.skipped_processing_jobs[0].model_id, "base");
    }

    #[test]
    fn retargetable_audio_transcription_deletion_model_keys_exclude_running_job_models() {
        let deletion_candidates = BTreeSet::from([
            model_key(LOCAL_WHISPER_PROVIDER_ID, "base"),
            model_key(LOCAL_WHISPER_PROVIDER_ID, "small"),
        ]);
        let running_job_models = BTreeSet::from([model_key(LOCAL_WHISPER_PROVIDER_ID, "base")]);

        let retargetable =
            retargetable_deletion_model_keys(&deletion_candidates, &running_job_models);

        assert_eq!(
            retargetable,
            BTreeSet::from([model_key(LOCAL_WHISPER_PROVIDER_ID, "small")])
        );
    }
}
