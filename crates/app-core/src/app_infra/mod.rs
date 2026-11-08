//! The database layer's command surface: app-infra's lifecycle (directory lock,
//! processor registry, initialize), the worker control, the timeline / search /
//! speaker / processing commands, retention and Delete Recent Capture, the frame
//! pipeline adapter and Quick Recall's data. The background loops that read the
//! live capture state still spawn from the host until capture moves (S3).

use std::{
    collections::BTreeSet,
    fs,
    fs::File,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use capture_screen::ScreenFrameArtifact;
use capture_types::{
    AudioSpeechDetector, AudioTranscriptionSettings, NativeCaptureSession, OcrProvider,
    OcrSettings, RetentionPolicy as SettingsRetentionPolicy,
};
use fs2::FileExt;
use futures_util::{
    future::{select, Either},
    pin_mut,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tokio::{sync::watch, task::JoinHandle};

use crate::{AppCore, AppInfraState};

mod commands;
mod delete_recent;
mod dto;
mod frame_pipeline;
pub mod frame_preview;
pub mod quick_recall;
mod requests;
mod retention;
mod speakers;
#[cfg(test)]
mod test_helpers;
mod workers;

pub use commands::*;
pub use delete_recent::*;
pub use dto::*;
pub use frame_pipeline::*;
pub use requests::*;
pub use retention::*;
pub use speakers::*;
pub use workers::*;

pub type BackgroundWorkersState = BackgroundWorkersControl;

pub const TIMELINE_DATA_CHANGED_EVENT: &str = "timeline_data_changed";

const APP_INFRA_LOCK_FILE_NAME: &str = ".app-infra.lock";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppInfraInitializeError {
    AlreadyRunning,
    Other(String),
}

impl std::fmt::Display for AppInfraInitializeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyRunning => write!(f, "app infrastructure is already running"),
            Self::Other(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for AppInfraInitializeError {}

#[derive(Debug)]
enum AppInfraDirectoryLockError {
    Contended {
        path: PathBuf,
        source: std::io::Error,
    },
    Other(String),
}

impl AppInfraDirectoryLockError {
    fn from_try_lock_error(path: PathBuf, source: std::io::Error) -> Self {
        if is_app_infra_lock_contended_error(&source) {
            Self::Contended { path, source }
        } else {
            Self::Other(format!(
                "failed to acquire app infrastructure lock at {}: {source}",
                path.display()
            ))
        }
    }
}

impl std::fmt::Display for AppInfraDirectoryLockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Contended { path, source } => {
                write!(
                    f,
                    "app infrastructure lock is already held at {}: {source}",
                    path.display()
                )
            }
            Self::Other(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for AppInfraDirectoryLockError {}

fn is_app_infra_lock_contended_error(source: &std::io::Error) -> bool {
    let contended = fs2::lock_contended_error();
    match (source.raw_os_error(), contended.raw_os_error()) {
        (Some(source_code), Some(contended_code)) => source_code == contended_code,
        _ => {
            source.kind() == std::io::ErrorKind::WouldBlock
                && contended.kind() == std::io::ErrorKind::WouldBlock
        }
    }
}

#[derive(Debug, Clone)]
struct ResolvedAppInfraBaseDir {
    save_directory: String,
    base_dir: PathBuf,
}

#[derive(Debug)]
struct AppInfraDirectoryLock {
    _file: File,
    path: PathBuf,
}

impl AppInfraDirectoryLock {
    fn acquire(base_dir: &Path) -> Result<Self, AppInfraDirectoryLockError> {
        fs::create_dir_all(base_dir).map_err(|error| {
            AppInfraDirectoryLockError::Other(format!(
                "failed to create app infrastructure base directory {}: {error}",
                base_dir.display()
            ))
        })?;

        let path = base_dir.join(APP_INFRA_LOCK_FILE_NAME);
        let file = File::create(&path).map_err(|error| {
            AppInfraDirectoryLockError::Other(format!(
                "failed to open app infrastructure lock file {}: {error}",
                path.display()
            ))
        })?;

        file.try_lock_exclusive().map_err(|source| {
            AppInfraDirectoryLockError::from_try_lock_error(path.clone(), source)
        })?;

        Ok(Self { _file: file, path })
    }
}

impl Drop for AppInfraDirectoryLock {
    fn drop(&mut self) {
        let _ = self._file.unlock();
        let _ = fs::remove_file(&self.path);
    }
}

fn desktop_processing_registry(
    core: &AppCore,
    deepgram_auth_status: audio_transcription::providers::DeepgramAuthStatus,
) -> Result<::app_infra::ProcessorRegistry, String> {
    let app_data_dir = core.paths().data.clone();
    let models_dir = audio_transcription::audio_transcription_models_dir(&app_data_dir);
    let speaker_models_dir = speaker_analysis::speaker_analysis_models_dir(&app_data_dir);

    let ocr_models_dir = ocr::ocr_models_dir(&app_data_dir);

    Ok(::app_infra::ProcessorRegistry::new()
        .register(::app_infra::OcrProcessorBackend::from_provider_arcs([
            Arc::new(::app_infra::AppleVisionProvider::new()) as Arc<dyn ocr::OcrProvider>,
            Arc::new(::app_infra::TesseractProvider::with_models_dir(
                ocr_models_dir.clone(),
            )),
            Arc::new(::app_infra::PaddleOcrProvider::with_models_dir(
                ocr_models_dir,
            )),
        ]))
        .register(
            ::app_infra::AudioTranscriptionProcessorBackend::from_provider_arcs([
                Arc::new(
                    audio_transcription::providers::LocalWhisperProvider::with_models_dir(
                        models_dir.clone(),
                    ),
                ) as Arc<dyn audio_transcription::TranscriptionProvider>,
                Arc::new(audio_transcription::providers::AppleSpeechOnDeviceProvider),
                Arc::new(
                    audio_transcription::providers::ParakeetProvider::with_models_dir(models_dir),
                ),
                Arc::new(audio_transcription::providers::DeepgramProvider::new(
                    // Denied ≠ missing (ADR 0048 amendment): a vault-denied read
                    // surfaces as Err and parks the job as transient liveness;
                    // only Ok(None) means "no key configured".
                    Arc::new(crate::transcription_deepgram::load_deepgram_key),
                    deepgram_auth_status.clone(),
                )) as Arc<dyn audio_transcription::TranscriptionProvider>,
            ]),
        )
        .register(::app_infra::SpeakerAnalysisProcessorBackend::from_provider_arcs([
            // speakrs is the sole on-device diarization provider; sherpa-onnx is
            // removed. Legacy `sherpa_onnx` job payloads are remapped to speakrs
            // at the normalize/dispatch seam, so only this provider is registered.
            Arc::new(
                crate::speaker_analysis_runtime::SubprocessSpeakerAnalysisProvider::with_provider(
                    speaker_analysis::SPEAKRS_PROVIDER_ID,
                    speaker_models_dir,
                ),
            ) as Arc<dyn speaker_analysis::SpeakerAnalysisProvider>,
        ]))
        .register(::app_infra::SystemAudioSpeechActivityProcessorBackend))
}

pub fn initialize(core: &AppCore) -> Result<(), AppInfraInitializeError> {
    let resolved_base_dir = resolve_base_dir(core).map_err(AppInfraInitializeError::Other)?;
    crate::debug_log::log_info(format!(
        "initializing app infrastructure (save_directory='{}', base_dir='{}')",
        resolved_base_dir.save_directory,
        resolved_base_dir.base_dir.display()
    ));

    let directory_lock =
        AppInfraDirectoryLock::acquire(&resolved_base_dir.base_dir).map_err(|error| {
            crate::debug_log::log_error(format!(
                "failed to acquire app infrastructure directory lock (save_directory='{}', base_dir='{}'): {error}",
                resolved_base_dir.save_directory,
                resolved_base_dir.base_dir.display()
            ));
            match error {
                AppInfraDirectoryLockError::Contended { .. } => {
                    AppInfraInitializeError::AlreadyRunning
                }
                AppInfraDirectoryLockError::Other(message) => AppInfraInitializeError::Other(message),
            }
        })?;

    let deepgram_auth_status: audio_transcription::providers::DeepgramAuthStatus =
        Arc::new(Mutex::new(None));
    let processing_registry = desktop_processing_registry(core, deepgram_auth_status.clone())
        .map_err(AppInfraInitializeError::Other)?;
    let infra = core
        .block_on(
            ::app_infra::AppInfra::initialize_fast_with_processing_registry(
                &resolved_base_dir.base_dir,
                processing_registry,
            ),
        )
        .map_err(|error| {
            crate::debug_log::log_error(format!(
                "failed to initialize app infrastructure (save_directory='{}', base_dir='{}'): {error}",
                resolved_base_dir.save_directory,
                resolved_base_dir.base_dir.display()
            ));

            AppInfraInitializeError::Other(format!(
                "failed to initialize app infrastructure at {}: {error}",
                resolved_base_dir.base_dir.display()
            ))
        })?;
    let infra = Arc::new(infra);
    crate::ocr_budget::reset_for_base_dir(&resolved_base_dir.base_dir);
    let background_workers = BackgroundWorkersState::default();

    if !core.manage(Arc::clone(&infra)) {
        crate::debug_log::log_error(
            "app infrastructure state was already initialized; refusing duplicate setup",
        );
        return Err(AppInfraInitializeError::Other(
            "app infrastructure state was already initialized".to_string(),
        ));
    }

    if !core.manage(Mutex::new(Some(directory_lock))) {
        crate::debug_log::log_error(
            "app infrastructure directory lock state was already initialized; refusing duplicate setup",
        );
        return Err(AppInfraInitializeError::Other(
            "app infrastructure directory lock state was already initialized".to_string(),
        ));
    }

    if !core.manage(frame_preview::FramePreviewCacheState::default()) {
        crate::debug_log::log_error(
            "frame preview cache state was already initialized; refusing duplicate setup",
        );
        return Err(AppInfraInitializeError::Other(
            "frame preview cache state was already initialized".to_string(),
        ));
    }

    if !core.manage(background_workers) {
        crate::debug_log::log_error(
            "background workers state was already initialized; refusing duplicate setup",
        );
        return Err(AppInfraInitializeError::Other(
            "background workers state was already initialized".to_string(),
        ));
    }

    if !core.manage(crate::transcription_deepgram::DeepgramAuthStatusState(
        deepgram_auth_status,
    )) {
        crate::debug_log::log_error(
            "deepgram auth status state was already initialized; refusing duplicate setup",
        );
        return Err(AppInfraInitializeError::Other(
            "deepgram auth status state was already initialized".to_string(),
        ));
    }

    crate::debug_log::log_info(format!(
        "initialized app infrastructure successfully (base_dir='{}')",
        resolved_base_dir.base_dir.display()
    ));

    // The database pool and all stores are now ready, so the window can open and
    // serve queries immediately. The heavy startup maintenance scans (index
    // backfills, orphaned-job reconciliation, hidden-segment workspace repair,
    // frame-index sidecar conversion, audio/speaker backfill) and the background
    // workers are run off the window-open critical path by the host's deferred
    // startup, so they no longer delay the first paint.
    Ok(())
}

fn resolve_base_dir(core: &AppCore) -> Result<ResolvedAppInfraBaseDir, String> {
    let settings = crate::recording_settings::load_recording_settings_or_default(core);
    let base_dir = crate::managed_storage_layout::ManagedStorageLayout::from_save_directory(
        &settings.save_directory,
    )
    .base_dir()
    .clone();

    crate::debug_log::log_info(format!(
        "resolved app infrastructure base directory (save_directory='{}', base_dir='{}')",
        settings.save_directory,
        base_dir.display()
    ));

    Ok(ResolvedAppInfraBaseDir {
        save_directory: settings.save_directory,
        base_dir,
    })
}

/// The resolved on-disk storage root where captures and the database live
/// (downloaded models stay in Mnema's app data, not here). This is the authoritative, env-honoring resolution: when no
/// `save_directory` is persisted it falls back to `default_save_directory()`
/// (`MNEMA_SAVE_DIRECTORY`, else `~/.mnema`). Settings › Storage shows this
/// read-only; the folder is changed by writing `save_directory` through the
/// recording-settings update (Browse), not by editing this string directly.
pub fn get_storage_location(core: &AppCore) -> Result<String, String> {
    Ok(resolve_base_dir(core)?.base_dir.display().to_string())
}

pub async fn get_app_infra_status(
    state: &AppInfraState,
) -> Result<::app_infra::AppInfraStatus, String> {
    let infra = Arc::clone(&*state);
    infra
        .status()
        .await
        .map_err(|error| format!("failed to read app infrastructure status: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_infra::test_helpers::*;

    #[test]
    fn resolve_base_dir_from_save_directory_uses_save_directory_as_base_dir() {
        let layout = crate::managed_storage_layout::ManagedStorageLayout::from_save_directory(
            "/tmp/mnema-recordings",
        );

        assert_eq!(layout.base_dir(), &PathBuf::from("/tmp/mnema-recordings"));
    }

    #[test]
    fn app_infra_directory_lock_rejects_second_owner_for_same_base_dir() {
        let dir = TestDir::new("app-infra-lock");

        let first = AppInfraDirectoryLock::acquire(dir.path())
            .expect("first app infra directory lock should succeed");
        let error = AppInfraDirectoryLock::acquire(dir.path())
            .expect_err("second app infra directory lock should fail");

        assert!(matches!(
            error,
            AppInfraDirectoryLockError::Contended { .. }
        ));

        drop(first);

        AppInfraDirectoryLock::acquire(dir.path())
            .expect("directory lock should be reacquirable after release");
    }

    #[test]
    fn app_infra_directory_lock_contention_maps_to_already_running() {
        let dir = TestDir::new("app-infra-lock-map");
        let _first = AppInfraDirectoryLock::acquire(dir.path())
            .expect("first app infra directory lock should succeed");

        let mapped = AppInfraDirectoryLock::acquire(dir.path()).map_err(|error| match error {
            AppInfraDirectoryLockError::Contended { .. } => AppInfraInitializeError::AlreadyRunning,
            AppInfraDirectoryLockError::Other(message) => AppInfraInitializeError::Other(message),
        });

        assert!(matches!(
            mapped,
            Err(AppInfraInitializeError::AlreadyRunning)
        ));
    }

    #[test]
    fn app_infra_directory_lock_non_contention_error_maps_to_other() {
        let path = PathBuf::from("/tmp/mnema-lock-test/.app-infra.lock");
        let error = AppInfraDirectoryLockError::from_try_lock_error(
            path.clone(),
            std::io::Error::new(std::io::ErrorKind::PermissionDenied, "permission denied"),
        );

        assert!(matches!(
            error,
            AppInfraDirectoryLockError::Other(message)
                if message.contains("failed to acquire app infrastructure lock")
                    && message.contains(&path.display().to_string())
        ));
    }

    #[test]
    fn app_infra_directory_lock_contended_error_maps_to_contended() {
        let path = PathBuf::from("/tmp/mnema-lock-test/.app-infra.lock");
        let error = AppInfraDirectoryLockError::from_try_lock_error(
            path.clone(),
            fs2::lock_contended_error(),
        );

        assert!(matches!(
            error,
            AppInfraDirectoryLockError::Contended { path: error_path, .. }
                if error_path == path
        ));
    }

    #[test]
    fn recordings_root_dir_nests_under_dot_z() {
        let layout = crate::managed_storage_layout::ManagedStorageLayout::from_save_directory(
            "/tmp/mnema-recordings",
        );

        assert_eq!(
            layout.recordings_root(),
            PathBuf::from("/tmp/mnema-recordings").join("recordings")
        );
    }

    #[test]
    fn recordings_root_dir_is_child_of_base_dir() {
        let layout = crate::managed_storage_layout::ManagedStorageLayout::from_save_directory(
            "/tmp/mnema-recordings",
        );
        let base_dir = layout.base_dir().clone();
        let recordings_root = layout.recordings_root();

        assert_eq!(recordings_root.parent(), Some(base_dir.as_path()));
    }
}
