use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use serde::{Deserialize, Serialize};
use speaker_analysis::{
    builtin_model_manifest, detect_model_status, find_model_descriptor, install_model_file,
    model_install_dir, remove_model_dir_if_exists, remove_model_file_if_exists,
    speaker_analysis_models_dir, validate_artifact_sha256, write_downloading_marker,
    write_failed_marker, write_installed_marker, ModelArtifactFile, ModelArtifactShape,
    ModelManagement, ModelStatusKind, SpeakerAnalysisModelDescriptor,
};

use crate::{AppCore, AppInfraState, CoreEvent};

mod download;
use download::*;

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

#[derive(Debug, Clone, PartialEq)]
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
    #[error(
        "download for {relative_path} exceeded its expected size of {byte_size} bytes (limit {limit} bytes); aborting"
    )]
    OversizedDownload {
        relative_path: String,
        byte_size: u64,
        limit: u64,
    },
    #[error("speaker model artifact {relative_path} is missing its integrity checksum; refusing to install")]
    MissingChecksum { relative_path: String },
}

pub fn get_speaker_analysis_model_status(
    core: &AppCore,
) -> Result<SpeakerAnalysisModelStatusResponseDto, String> {
    build_speaker_analysis_model_status_response(&core.paths().config)
        .map_err(|error| format!("failed to inspect speaker analysis model: {error}"))
}

pub fn start_speaker_analysis_model_download(
    core: &Arc<AppCore>,
    request: SpeakerAnalysisModelDownloadRequestDto,
) -> Result<SpeakerAnalysisModelDownloadProgressDto, String> {
    let plan = build_download_plan(core, &request).map_err(|error| error.to_string())?;
    let cancel_requested = Arc::new(AtomicBool::new(false));
    claim_model_download(
        &core.state::<SpeakerAnalysisModelDownloadState>(),
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
    emit_download_progress(core, &starting);

    let core_for_task = Arc::clone(core);
    core.spawn(async move {
        run_model_download_task(core_for_task, plan, cancel_requested).await;
    });

    Ok(starting)
}

pub fn cancel_speaker_analysis_model_download(core: &AppCore) -> Result<(), String> {
    let download_state = core.state::<SpeakerAnalysisModelDownloadState>();
    let active = download_state
        .lock()
        .map_err(|_| "speaker analysis model download state poisoned".to_string())?;
    let Some(active) = active.as_ref() else {
        return Err(ModelDownloadError::NoActiveDownload.to_string());
    };
    active.cancel_requested.store(true, Ordering::SeqCst);
    Ok(())
}

pub async fn delete_speaker_analysis_model(
    core: &AppCore,
    request: SpeakerAnalysisModelDownloadRequestDto,
    infra: &AppInfraState,
) -> Result<(), String> {
    if active_download_key(&core.state::<SpeakerAnalysisModelDownloadState>())?
        == Some(model_key(&request.provider, &request.model_id))
    {
        return Err("cannot delete a speaker analysis model while it is downloading".to_string());
    }
    let app_data_dir = core.paths().config.clone();
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
        .release_processing_model_locks(&cleanup_lock)
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

/// Human-facing label for a speaker-analysis provider group. Falls back to the
/// raw provider id for any future provider that lacks a curated label here.
fn speaker_provider_display_name(provider: &str) -> String {
    match provider {
        speaker_analysis::SPEAKRS_PROVIDER_ID => "On-device (CoreML)".to_string(),
        other => other.to_string(),
    }
}

fn build_speaker_analysis_model_status_response(
    app_data_dir: &Path,
) -> Result<SpeakerAnalysisModelStatusResponseDto, speaker_analysis::ModelStatusError> {
    let models_dir = speaker_analysis_models_dir(app_data_dir);
    let manifest = builtin_model_manifest();

    // Group descriptors by their ACTUAL provider (preserving manifest order) so
    // every provider — speakrs today (and any future provider) — gets its own
    // selectable group surfaced to the settings UI.
    let mut providers: Vec<SpeakerAnalysisProviderStatusDto> = Vec::new();

    for descriptor in &manifest.models {
        let status = detect_model_status(&models_dir, descriptor)?;
        let ModelManagement::AppManaged { artifact, .. } = &descriptor.management;
        let model = SpeakerAnalysisModelStatusDto {
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
        };

        match providers
            .iter_mut()
            .find(|group| group.provider == descriptor.provider)
        {
            Some(group) => group.models.push(model),
            None => providers.push(SpeakerAnalysisProviderStatusDto {
                provider: descriptor.provider.clone(),
                display_name: speaker_provider_display_name(&descriptor.provider),
                models: vec![model],
            }),
        }
    }

    Ok(SpeakerAnalysisModelStatusResponseDto {
        models_directory: models_dir.to_string_lossy().to_string(),
        providers,
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

/// The model key of the selected speaker-analysis model when its files are **not** on disk, so its
/// queued jobs can be parked (claim-time model gating) instead of burning their failure attempts
/// against a model that is not there. The selection is normalized onto speakrs first, matching the
/// key every other seam derives for a speaker_analysis job (including the claim SQL).
pub fn absent_selected_speaker_analysis_model_key(
    app_data_dir: &Path,
    settings: &capture_types::SpeakerAnalysisSettings,
) -> Option<String> {
    let mut payload = ::app_infra::SpeakerAnalysisJobPayload::new(
        settings.provider.clone(),
        settings.model_id.clone(),
    );
    payload.normalize_model_selection();
    let model_id = payload
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|model_id| !model_id.is_empty())?;
    let manifest = builtin_model_manifest();
    let installed = find_model_descriptor(&manifest, &payload.provider, Some(model_id))
        .and_then(|descriptor| {
            detect_model_status(speaker_analysis_models_dir(app_data_dir), descriptor).ok()
        })
        .is_some_and(|status| status.status == ModelStatusKind::Installed);
    (!installed).then(|| model_key(&payload.provider, model_id))
}

fn clear_active_download(core: &AppCore, provider: &str, model_id: &str) {
    if let Ok(mut active) = core.state::<SpeakerAnalysisModelDownloadState>().lock() {
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
