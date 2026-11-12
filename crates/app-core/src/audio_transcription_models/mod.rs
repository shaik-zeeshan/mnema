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
    providers::{provider_runtime_available, AppleSpeechOnDeviceAvailabilityStatus},
    remove_model_dir_if_exists, remove_model_file_if_exists, validate_artifact_sha256,
    write_downloading_marker, write_failed_marker, AudioTranscriptionModelDescriptor,
    AudioTranscriptionModelStatus, ModelArtifact, ModelArtifactFile, ModelArtifactShape,
    ModelManagement, ModelStatusError, ModelStatusKind, DOWNLOADING_MARKER_FILE_NAME,
};
use capture_types::{AudioTranscriptionProvider, AudioTranscriptionSettings};
use serde::{Deserialize, Serialize};

use crate::{AppCore, AppInfraState, CoreEvent};

mod download;
use download::*;

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

pub fn get_audio_transcription_model_status(
    core: &AppCore,
) -> Result<AudioTranscriptionModelStatusResponseDto, String> {
    build_audio_transcription_model_status_response(&core.paths().config)
        .map_err(|error| format!("failed to inspect audio transcription models: {error}"))
}

pub fn start_audio_transcription_model_download(
    core: &Arc<AppCore>,
    request: AudioTranscriptionModelDownloadRequestDto,
    infra: &AppInfraState,
) -> Result<AudioTranscriptionModelDownloadProgressDto, String> {
    let plan = build_download_plan(core, &request).map_err(|error| error.to_string())?;
    let cancel_requested = Arc::new(AtomicBool::new(false));

    claim_model_download(
        &core.state::<AudioTranscriptionModelDownloadState>(),
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
    emit_download_progress(core, &starting);

    let core_for_task = Arc::clone(core);
    let infra_for_task = Arc::clone(infra);
    core.spawn(async move {
        run_model_download_task(core_for_task, plan, cancel_requested, infra_for_task).await;
    });

    Ok(starting)
}

pub fn cancel_audio_transcription_model_download(core: &AppCore) -> Result<(), String> {
    let download_state = core.state::<AudioTranscriptionModelDownloadState>();
    let active = download_state
        .lock()
        .map_err(|_| "audio transcription model download state poisoned".to_string())?;
    let Some(active) = active.as_ref() else {
        return Err(ModelDownloadError::NoActiveDownload.to_string());
    };
    active.cancel_requested.store(true, Ordering::SeqCst);
    Ok(())
}

pub async fn delete_unused_audio_transcription_models(
    core: &AppCore,
    infra: &AppInfraState,
) -> Result<DeleteUnusedAudioTranscriptionModelsResponseDto, String> {
    let app_data_dir = core.paths().config.clone();
    let settings = crate::recording_settings::current_recording_settings(
        &core.state::<crate::recording_settings::RecordingSettingsState>(),
    );
    let selected_provider = provider_id_for_settings(settings.transcription.provider);
    let selected_model_id = settings.transcription.model_id.as_deref();
    let active_download =
        active_download_key(&core.state::<AudioTranscriptionModelDownloadState>())?;
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
        .release_processing_model_locks(&cleanup_lock)
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

pub fn request_apple_speech_recognition_permission(
    core: &AppCore,
) -> Result<AudioTranscriptionModelStatusResponseDto, String> {
    let availability =
        audio_transcription::providers::AppleSpeechOnDeviceProvider::request_permission();
    if matches!(
        availability.status,
        AppleSpeechOnDeviceAvailabilityStatus::FrameworkUnavailable
    ) {
        return Err(availability.message);
    }
    get_audio_transcription_model_status(core)
}

pub fn open_apple_speech_recognition_privacy_settings(core: &AppCore) -> Result<(), String> {
    core.host()
        .open_url(
            "x-apple.systempreferences:com.apple.preference.security?Privacy_SpeechRecognition",
        )
        .map_err(|error| format!("failed to open Apple Speech privacy settings: {error}"))
}

pub fn transcription_request_options_for_settings(
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

pub fn selected_audio_transcription_model_available(
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
    if !(status.is_available() && provider_runtime_available(provider)) {
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
        return Ok(deepgram_available_for_enqueue(
            app_infra::has_ai_provider_key(crate::transcription_deepgram::DEEPGRAM_KEY_ACCOUNT),
        ));
    }

    Ok(true)
}

/// Deepgram enqueue-time availability from a vault key-presence read.
///
/// ADR 0048 amendment: a vault-denied read is NOT "no key". Report available so
/// segments still enqueue transcription jobs; each job then parks as transient
/// liveness at transcribe time and recovers on a later launch, instead of
/// silently skipping transcription.
fn deepgram_available_for_enqueue(key_presence: Result<bool, app_infra::AppInfraError>) -> bool {
    match key_presence {
        Ok(present) => present,
        Err(app_infra::AppInfraError::SecretVaultDenied(_)) => true,
        Err(_) => false,
    }
}

/// Deepgram status-DTO availability + failure message from a vault key-presence
/// read. Denied ≠ missing (ADR 0048 amendment): show the vault error itself,
/// never the misleading "add a key" prompt.
fn deepgram_status_fields(
    key_presence: Result<bool, app_infra::AppInfraError>,
) -> (bool, Option<String>) {
    match key_presence {
        Ok(true) => (true, None),
        Ok(false) => (
            false,
            Some("Add a Deepgram API key in Settings to enable cloud transcription.".to_string()),
        ),
        Err(error) => (false, Some(error.to_string())),
    }
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

/// The model key of the selected transcription model when its files are **not** on disk, so its
/// queued jobs can be parked (claim-time model gating) instead of burning their three failure
/// attempts in six minutes against a model that is not there. Deliberately files-only: unlike
/// [`selected_audio_transcription_model_available`] it ignores the enabled flag, the Deepgram key
/// (ADR 0048 already parks those jobs as transient liveness) and runtime availability — none of
/// which a download fixes, and none of which should silently park a segment forever.
pub fn absent_selected_audio_transcription_model_key(
    app_data_dir: &Path,
    settings: &AudioTranscriptionSettings,
) -> Option<String> {
    let provider = provider_id_for_settings(settings.provider);
    let model_id = settings
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|model_id| !model_id.is_empty())?;
    let manifest = builtin_model_manifest();
    let installed = find_model_descriptor(&manifest, provider, Some(model_id))
        .and_then(|descriptor| {
            detect_model_status(audio_transcription_models_dir(app_data_dir), descriptor).ok()
        })
        .is_some_and(|status| status.is_available());
    (!installed).then(|| model_key(provider, model_id))
}

pub fn provider_id_for_settings(provider: AudioTranscriptionProvider) -> &'static str {
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
        let (deepgram_available, deepgram_failure) = deepgram_status_fields(
            app_infra::has_ai_provider_key(crate::transcription_deepgram::DEEPGRAM_KEY_ACCOUNT),
        );
        available = deepgram_available;
        if failure_message.is_none() {
            failure_message = deepgram_failure;
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
mod tests;
