//! Provider-neutral local speaker diarization and recognition contracts.
//!
//! The crate intentionally keeps app storage and Tauri download orchestration
//! out of the provider boundary. V1 providers receive local audio plus optional
//! local person embeddings and return anonymous speaker clusters, turns, and
//! cautious recognition suggestions.

mod core;
mod macos_audio_decode;
pub mod providers;

pub use core::{
    PersonEnrollment, PersonRecognitionRejection, RecognitionConfidence, SpeakerAnalysisError,
    SpeakerAnalysisMetadata, SpeakerAnalysisOutput, SpeakerAnalysisProvider,
    SpeakerAnalysisRequest, SpeakerAnalysisResult, SpeakerCluster, SpeakerRecognitionSuggestion,
    SpeakerTurn,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};
use thiserror::Error;

pub const MODEL_STORE_DIR_NAME: &str = "speaker-analysis-models";
pub const INSTALLED_MARKER_FILE_NAME: &str = ".installed.json";
pub const FAILED_MARKER_FILE_NAME: &str = ".failed.json";
pub const DOWNLOADING_MARKER_FILE_NAME: &str = ".download-in-progress";
pub const SHERPA_ONNX_PROVIDER_ID: &str = "sherpa_onnx";
pub const DEFAULT_SHERPA_ONNX_MODEL_ID: &str = "pyannote-3.0-nemo-titanet-small";
/// Multilingual (English + Mandarin) preset: pyannote segmentation 3.0 plus
/// 3D-Speaker CAM++ zh/en embeddings.
pub const MULTILINGUAL_SHERPA_ONNX_MODEL_ID: &str = "pyannote-3.0-campplus-zh-en";
/// High-accuracy preset: reverb-diarization-v1 segmentation (robust in
/// noise/reverb) plus NeMo Titanet Large English embeddings.
pub const HIGH_ACCURACY_SHERPA_ONNX_MODEL_ID: &str = "reverb-v1-nemo-titanet-large";

const MANIFEST_VERSION: u32 = 1;

/// Default per-model clustering similarity threshold for the original Balanced
/// preset. Historically the global `DEFAULT_CLUSTERING_THRESHOLD` const in the
/// sherpa provider; now a per-descriptor field so new presets can tune it.
pub const DEFAULT_CLUSTERING_THRESHOLD: f32 = 0.65;
/// Conservative cross-chunk cluster similarity threshold used by presets whose
/// embedding model has not been empirically calibrated. Historically the global
/// `CROSS_CHUNK_CLUSTER_SIMILARITY_THRESHOLD` const (0.60); now only the two
/// newer presets (campplus / titanet-large) inherit it, since their embedding
/// similarity scales differ from titanet-small and have not been measured.
pub const DEFAULT_CROSS_CHUNK_THRESHOLD: f32 = 0.60;
/// Cross-chunk cluster similarity threshold for the Balanced (titanet-small)
/// preset. A brief experiment lowered this from the historical 0.60 to 0.50,
/// calibrated on a single 3-speaker clip. The DER benchmark
/// (`scripts/diarization_bench/`, VoxConverse 10-clip subset) showed that
/// over-fit: 0.50 wrongly merges distinct speakers on harder multi-speaker audio
/// (confusion-dominated DER), and 0.60 is the empirical optimum of a clean
/// U-curve (DER 10.89% -> 9.71%). Restored to 0.60, matching the other presets.
pub const BALANCED_CROSS_CHUNK_THRESHOLD: f32 = 0.60;
/// Default minimum speaker-turn duration (milliseconds) below which a turn is
/// skipped when forming per-chunk cluster embeddings (accuracy improvement #2).
pub const DEFAULT_MIN_TURN_MS: u64 = 500;

#[derive(Debug, Error)]
pub enum ModelStatusError {
    #[error("model descriptor for provider {provider} is missing an app-managed model id")]
    MissingAppManagedModelId { provider: String },
    #[error("unsafe path component in {field}: {value}")]
    UnsafePathComponent { field: &'static str, value: String },
    #[error("failed to read marker {path}: {source}")]
    ReadMarker {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse marker {path}: {source}")]
    ParseMarker {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("failed to create directory {path}: {source}")]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to write marker {path}: {source}")]
    WriteMarker {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[derive(Debug, Error)]
pub enum ModelInstallError {
    #[error("failed to remove model path {path}: {source}")]
    RemovePath {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to create directory {path}: {source}")]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to move downloaded model file to {path}: {source}")]
    MoveFile {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("downloaded model checksum mismatch for {path}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    #[error("failed to read downloaded model file {path}: {source}")]
    ReadFile {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("installed model layout is incomplete; missing files: {missing_files:?}")]
    IncompleteInstalledLayout { missing_files: Vec<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelArtifactShape {
    MultiFile { files: Vec<ModelArtifactFile> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelArtifactFile {
    pub relative_path: String,
    pub url: String,
    pub byte_size: u64,
    pub sha256: Option<String>,
}

// NOTE: `SpeakerAnalysisModelManifest`, `SpeakerAnalysisModelDescriptor`, and
// `SpeakerAnalysisModelStatus` intentionally drop `Eq` (keeping `PartialEq`)
// because the descriptor now transitively holds f32 clustering thresholds via
// `SherpaModelParams`, and f32 cannot implement `Eq`. No code compares these
// whole structs for `Eq`/hashing; `ModelStatusKind` and the artifact/layout
// structs keep `Eq` since they hold no floats.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerAnalysisModelManifest {
    pub version: u32,
    pub models: Vec<SpeakerAnalysisModelDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerAnalysisModelDescriptor {
    pub provider: String,
    pub model_id: Option<String>,
    pub display_name: String,
    pub description: String,
    pub license_label: Option<String>,
    pub source_url: Option<String>,
    pub management: ModelManagement,
    /// Sherpa-onnx provider parameters (segmentation/embedding relative paths
    /// plus per-model clustering/cross-chunk thresholds and minimum turn
    /// duration). Optional so the shared descriptor stays forward-compatible
    /// with a future non-sherpa provider (e.g. FluidAudio/ANE) that carries no
    /// sherpa params.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sherpa_params: Option<SherpaModelParams>,
}

/// Sherpa-onnx-specific tuning carried per **Speaker Model Preset** descriptor.
///
/// Holds the segmentation/embedding model relative paths (previously module
/// consts) and the per-model accuracy thresholds settled in ADR 0001 so new
/// presets are not stuck with values tuned for the original combo. Does not
/// derive `Eq` because `clustering_threshold`/`cross_chunk_threshold` are f32.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SherpaModelParams {
    /// Relative path (under the model install dir) to the segmentation
    /// `model.onnx`.
    pub segmentation_relative_path: String,
    /// Relative path (under the model install dir) to the embedding `.onnx`.
    pub embedding_relative_path: String,
    /// Per-model fast-clustering similarity threshold (accuracy #3); a
    /// request-option override still wins at runtime.
    pub clustering_threshold: f32,
    /// Per-model cross-chunk cluster similarity threshold used when stitching
    /// safe-chunked diarization clusters together.
    pub cross_chunk_threshold: f32,
    /// Minimum speaker-turn duration in milliseconds (accuracy #2); turns
    /// shorter than this are skipped when forming per-chunk embeddings.
    pub min_turn_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelManagement {
    AppManaged {
        expected_layout: InstalledModelLayout,
        artifact: Option<ModelArtifact>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModelLayout {
    pub marker_file_name: String,
    pub required_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelArtifact {
    pub url: String,
    pub byte_size: u64,
    pub sha256: Option<String>,
    pub shape: ModelArtifactShape,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelStatusKind {
    NotInstalled,
    Installed,
    Incomplete,
    Failed,
    Downloading,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerAnalysisModelStatus {
    pub descriptor: SpeakerAnalysisModelDescriptor,
    pub status: ModelStatusKind,
    pub install_path: PathBuf,
    pub missing_files: Vec<String>,
    pub failure_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModelMarker {
    pub manifest_version: u32,
    pub provider: String,
    pub model_id: String,
}

pub fn builtin_model_manifest() -> SpeakerAnalysisModelManifest {
    SpeakerAnalysisModelManifest {
        version: MANIFEST_VERSION,
        models: vec![
            // Balanced (default). Paths/thresholds preserved exactly from the
            // historical module consts so this preset behaves identically.
            SpeakerAnalysisModelDescriptor {
                provider: SHERPA_ONNX_PROVIDER_ID.to_string(),
                model_id: Some(DEFAULT_SHERPA_ONNX_MODEL_ID.to_string()),
                display_name: "Balanced (pyannote 3.0 + NeMo Titanet Small)".to_string(),
                description: "Balanced English-first local speaker diarization using pyannote segmentation 3.0 plus NeMo Titanet Small speaker embeddings.".to_string(),
                license_label: None,
                source_url: Some("https://github.com/k2-fsa/sherpa-onnx".to_string()),
                management: ModelManagement::AppManaged {
                    expected_layout: InstalledModelLayout {
                        marker_file_name: INSTALLED_MARKER_FILE_NAME.to_string(),
                        required_files: vec![
                            "pyannote-segmentation-3.0/model.onnx".to_string(),
                            "nemo_en_titanet_small.onnx".to_string(),
                        ],
                    },
                    artifact: Some(ModelArtifact {
                        url: "https://github.com/k2-fsa/sherpa-onnx".to_string(),
                        byte_size: 47_215_727,
                        sha256: None,
                        shape: ModelArtifactShape::MultiFile {
                            files: vec![
                                ModelArtifactFile {
                                    relative_path: "pyannote-segmentation-3.0/model.onnx".to_string(),
                                    url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2".to_string(),
                                    byte_size: 6_958_444,
                                    sha256: Some("24615ee884c897d9d2ba09bb4d30da6bb1b15e685065962db5b02e76e4996488".to_string()),
                                },
                                ModelArtifactFile {
                                    relative_path: "nemo_en_titanet_small.onnx".to_string(),
                                    url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/nemo_en_titanet_small.onnx".to_string(),
                                    byte_size: 40_257_283,
                                    sha256: Some("ad4a1802485d8b34c722d2a9d04249662f2ece5d28a7a039063ca22f515a789e".to_string()),
                                },
                            ],
                        },
                    }),
                },
                sherpa_params: Some(SherpaModelParams {
                    segmentation_relative_path: "pyannote-segmentation-3.0/model.onnx".to_string(),
                    embedding_relative_path: "nemo_en_titanet_small.onnx".to_string(),
                    clustering_threshold: DEFAULT_CLUSTERING_THRESHOLD,
                    cross_chunk_threshold: BALANCED_CROSS_CHUNK_THRESHOLD,
                    min_turn_ms: DEFAULT_MIN_TURN_MS,
                }),
            },
            // Multilingual (English + Mandarin). pyannote segmentation 3.0
            // (shared with Balanced) plus 3D-Speaker CAM++ zh/en embeddings.
            // ~33MB total download.
            SpeakerAnalysisModelDescriptor {
                provider: SHERPA_ONNX_PROVIDER_ID.to_string(),
                model_id: Some(MULTILINGUAL_SHERPA_ONNX_MODEL_ID.to_string()),
                display_name: "Multilingual (pyannote 3.0 + CAM++ zh/en)".to_string(),
                description: "Multilingual (English + Mandarin) local speaker diarization using pyannote segmentation 3.0 plus 3D-Speaker CAM++ zh/en speaker embeddings.".to_string(),
                license_label: None,
                source_url: Some("https://github.com/k2-fsa/sherpa-onnx".to_string()),
                management: ModelManagement::AppManaged {
                    expected_layout: InstalledModelLayout {
                        marker_file_name: INSTALLED_MARKER_FILE_NAME.to_string(),
                        required_files: vec![
                            "pyannote-segmentation-3.0/model.onnx".to_string(),
                            "3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx".to_string(),
                        ],
                    },
                    artifact: Some(ModelArtifact {
                        url: "https://github.com/k2-fsa/sherpa-onnx".to_string(),
                        byte_size: 35_239_608,
                        sha256: None,
                        shape: ModelArtifactShape::MultiFile {
                            files: vec![
                                ModelArtifactFile {
                                    relative_path: "pyannote-segmentation-3.0/model.onnx".to_string(),
                                    url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2".to_string(),
                                    byte_size: 6_958_444,
                                    sha256: Some("24615ee884c897d9d2ba09bb4d30da6bb1b15e685065962db5b02e76e4996488".to_string()),
                                },
                                ModelArtifactFile {
                                    relative_path: "3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx".to_string(),
                                    url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx".to_string(),
                                    byte_size: 28_281_164,
                                    sha256: Some("aa3cfc16963a10586a9393f5035d6d6b57e98d358b347f80c2a30bf4f00ceba2".to_string()),
                                },
                            ],
                        },
                    }),
                },
                sherpa_params: Some(SherpaModelParams {
                    segmentation_relative_path: "pyannote-segmentation-3.0/model.onnx".to_string(),
                    embedding_relative_path: "3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx".to_string(),
                    clustering_threshold: DEFAULT_CLUSTERING_THRESHOLD,
                    cross_chunk_threshold: DEFAULT_CROSS_CHUNK_THRESHOLD,
                    min_turn_ms: DEFAULT_MIN_TURN_MS,
                }),
            },
            // High-accuracy (English). reverb-diarization-v1 segmentation
            // (robust in noise/reverb) plus NeMo Titanet Large embeddings.
            // ~106MB total download. The segmentation tarball extracts its
            // fp32 `model.onnx` into `reverb-diarization-v1/model.onnx`.
            SpeakerAnalysisModelDescriptor {
                provider: SHERPA_ONNX_PROVIDER_ID.to_string(),
                model_id: Some(HIGH_ACCURACY_SHERPA_ONNX_MODEL_ID.to_string()),
                display_name: "High-accuracy (reverb v1 + NeMo Titanet Large)".to_string(),
                description: "High-accuracy English local speaker diarization using reverb-diarization-v1 segmentation (robust in noise/reverb) plus NeMo Titanet Large speaker embeddings.".to_string(),
                license_label: None,
                source_url: Some("https://github.com/k2-fsa/sherpa-onnx".to_string()),
                management: ModelManagement::AppManaged {
                    expected_layout: InstalledModelLayout {
                        marker_file_name: INSTALLED_MARKER_FILE_NAME.to_string(),
                        required_files: vec![
                            "reverb-diarization-v1/model.onnx".to_string(),
                            "nemo_en_titanet_large.onnx".to_string(),
                        ],
                    },
                    artifact: Some(ModelArtifact {
                        url: "https://github.com/k2-fsa/sherpa-onnx".to_string(),
                        byte_size: 112_324_078,
                        sha256: None,
                        shape: ModelArtifactShape::MultiFile {
                            files: vec![
                                ModelArtifactFile {
                                    relative_path: "reverb-diarization-v1/model.onnx".to_string(),
                                    url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-segmentation-models/sherpa-onnx-reverb-diarization-v1.tar.bz2".to_string(),
                                    byte_size: 10_918_585,
                                    sha256: Some("615761e980be1688da0ef81618c056134d63aa55ea0a5f1494c47393b9398eab".to_string()),
                                },
                                ModelArtifactFile {
                                    relative_path: "nemo_en_titanet_large.onnx".to_string(),
                                    url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/nemo_en_titanet_large.onnx".to_string(),
                                    byte_size: 101_405_493,
                                    sha256: Some("d51abcf31717ef28162f26acb9d44dd4127c3d44c9b8624f699f3425daca8e77".to_string()),
                                },
                            ],
                        },
                    }),
                },
                sherpa_params: Some(SherpaModelParams {
                    segmentation_relative_path: "reverb-diarization-v1/model.onnx".to_string(),
                    embedding_relative_path: "nemo_en_titanet_large.onnx".to_string(),
                    clustering_threshold: DEFAULT_CLUSTERING_THRESHOLD,
                    cross_chunk_threshold: DEFAULT_CROSS_CHUNK_THRESHOLD,
                    min_turn_ms: DEFAULT_MIN_TURN_MS,
                }),
            },
        ],
    }
}

pub fn find_model_descriptor<'a>(
    manifest: &'a SpeakerAnalysisModelManifest,
    provider: &str,
    model_id: Option<&str>,
) -> Option<&'a SpeakerAnalysisModelDescriptor> {
    manifest.models.iter().find(|descriptor| {
        descriptor.provider == provider && descriptor.model_id.as_deref() == model_id
    })
}

pub fn write_downloading_marker(
    models_dir: impl AsRef<Path>,
    provider: &str,
    model_id: &str,
) -> Result<(), ModelStatusError> {
    let install_dir = models_dir
        .as_ref()
        .join(safe_path_component("provider", provider)?)
        .join(safe_path_component("modelId", model_id)?);
    fs::create_dir_all(&install_dir).map_err(|source| ModelStatusError::CreateDir {
        path: install_dir.clone(),
        source,
    })?;
    let marker = install_dir.join(DOWNLOADING_MARKER_FILE_NAME);
    fs::write(&marker, "").map_err(|source| ModelStatusError::WriteMarker {
        path: marker,
        source,
    })
}

pub fn write_failed_marker(
    models_dir: impl AsRef<Path>,
    provider: &str,
    model_id: &str,
    message: impl AsRef<str>,
) -> Result<(), ModelStatusError> {
    let install_dir = models_dir
        .as_ref()
        .join(safe_path_component("provider", provider)?)
        .join(safe_path_component("modelId", model_id)?);
    fs::create_dir_all(&install_dir).map_err(|source| ModelStatusError::CreateDir {
        path: install_dir.clone(),
        source,
    })?;
    let marker = install_dir.join(FAILED_MARKER_FILE_NAME);
    fs::write(&marker, message.as_ref()).map_err(|source| ModelStatusError::WriteMarker {
        path: marker,
        source,
    })
}

pub fn write_installed_marker(
    models_dir: impl AsRef<Path>,
    provider: &str,
    model_id: &str,
) -> Result<(), ModelStatusError> {
    let install_dir = models_dir
        .as_ref()
        .join(safe_path_component("provider", provider)?)
        .join(safe_path_component("modelId", model_id)?);
    fs::create_dir_all(&install_dir).map_err(|source| ModelStatusError::CreateDir {
        path: install_dir.clone(),
        source,
    })?;
    let marker = install_dir.join(INSTALLED_MARKER_FILE_NAME);
    let payload = InstalledModelMarker {
        manifest_version: MANIFEST_VERSION,
        provider: provider.to_string(),
        model_id: model_id.to_string(),
    };
    let json =
        serde_json::to_vec_pretty(&payload).map_err(|source| ModelStatusError::ParseMarker {
            path: marker.clone(),
            source,
        })?;
    fs::write(&marker, json).map_err(|source| ModelStatusError::WriteMarker {
        path: marker,
        source,
    })
}

pub fn remove_model_file_if_exists(path: impl AsRef<Path>) -> Result<(), ModelInstallError> {
    let path = path.as_ref();
    if path.exists() {
        fs::remove_file(path).map_err(|source| ModelInstallError::RemovePath {
            path: path.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

pub fn remove_model_dir_if_exists(path: impl AsRef<Path>) -> Result<(), ModelInstallError> {
    let path = path.as_ref();
    if path.exists() {
        fs::remove_dir_all(path).map_err(|source| ModelInstallError::RemovePath {
            path: path.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

pub fn validate_artifact_sha256(
    path: impl AsRef<Path>,
    expected: Option<&str>,
) -> Result<(), ModelInstallError> {
    let Some(expected) = expected.filter(|value| !value.trim().is_empty()) else {
        return Ok(());
    };
    let path = path.as_ref();
    let mut file = fs::File::open(path).map_err(|source| ModelInstallError::ReadFile {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| ModelInstallError::ReadFile {
                path: path.to_path_buf(),
                source,
            })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual != expected {
        return Err(ModelInstallError::ChecksumMismatch {
            path: path.to_path_buf(),
            expected: expected.to_string(),
            actual,
        });
    }
    Ok(())
}

pub fn install_model_file(
    destination: impl AsRef<Path>,
    bytes: &[u8],
) -> Result<(), ModelInstallError> {
    let destination = destination.as_ref();
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|source| ModelInstallError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let mut file = fs::File::create(destination).map_err(|source| ModelInstallError::MoveFile {
        path: destination.to_path_buf(),
        source,
    })?;
    file.write_all(bytes)
        .map_err(|source| ModelInstallError::MoveFile {
            path: destination.to_path_buf(),
            source,
        })
}

pub fn speaker_analysis_models_dir(app_data_dir: impl AsRef<Path>) -> PathBuf {
    app_data_dir.as_ref().join(MODEL_STORE_DIR_NAME)
}

pub fn model_install_dir(
    models_dir: impl AsRef<Path>,
    descriptor: &SpeakerAnalysisModelDescriptor,
) -> Result<PathBuf, ModelStatusError> {
    let model_id = descriptor.model_id.as_deref().ok_or_else(|| {
        ModelStatusError::MissingAppManagedModelId {
            provider: descriptor.provider.clone(),
        }
    })?;
    Ok(models_dir
        .as_ref()
        .join(safe_path_component("provider", &descriptor.provider)?)
        .join(safe_path_component("modelId", model_id)?))
}

pub fn detect_model_status(
    models_dir: impl AsRef<Path>,
    descriptor: &SpeakerAnalysisModelDescriptor,
) -> Result<SpeakerAnalysisModelStatus, ModelStatusError> {
    let install_path = model_install_dir(models_dir, descriptor)?;
    let ModelManagement::AppManaged {
        expected_layout, ..
    } = &descriptor.management;
    let downloading_marker = install_path.join(DOWNLOADING_MARKER_FILE_NAME);
    if downloading_marker.exists() {
        return Ok(status(
            descriptor,
            ModelStatusKind::Downloading,
            install_path,
            vec![],
            None,
        ));
    }

    let missing_files = expected_layout
        .required_files
        .iter()
        .filter(|file| !install_path.join(file).is_file())
        .cloned()
        .collect::<Vec<_>>();
    let installed_marker = install_path.join(&expected_layout.marker_file_name);
    if installed_marker.is_file() && missing_files.is_empty() {
        return Ok(status(
            descriptor,
            ModelStatusKind::Installed,
            install_path,
            vec![],
            None,
        ));
    }
    if install_path.join(FAILED_MARKER_FILE_NAME).is_file() {
        let message =
            fs::read_to_string(install_path.join(FAILED_MARKER_FILE_NAME)).map_err(|source| {
                ModelStatusError::ReadMarker {
                    path: install_path.join(FAILED_MARKER_FILE_NAME),
                    source,
                }
            })?;
        return Ok(status(
            descriptor,
            ModelStatusKind::Failed,
            install_path,
            missing_files,
            Some(message),
        ));
    }
    let kind = if install_path.exists() && !missing_files.is_empty() {
        ModelStatusKind::Incomplete
    } else {
        ModelStatusKind::NotInstalled
    };
    Ok(status(descriptor, kind, install_path, missing_files, None))
}

fn status(
    descriptor: &SpeakerAnalysisModelDescriptor,
    status: ModelStatusKind,
    install_path: PathBuf,
    missing_files: Vec<String>,
    failure_message: Option<String>,
) -> SpeakerAnalysisModelStatus {
    SpeakerAnalysisModelStatus {
        descriptor: descriptor.clone(),
        status,
        install_path,
        missing_files,
        failure_message,
    }
}

fn safe_path_component(field: &'static str, value: &str) -> Result<String, ModelStatusError> {
    let path = Path::new(value);
    let mut components = path.components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(value.to_string()),
        _ => Err(ModelStatusError::UnsafePathComponent {
            field,
            value: value.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_exposes_app_managed_sherpa_model() {
        let manifest = builtin_model_manifest();
        assert_eq!(manifest.version, 1);
        assert_eq!(manifest.models[0].provider, SHERPA_ONNX_PROVIDER_ID);
        assert_eq!(
            manifest.models[0].model_id.as_deref(),
            Some(DEFAULT_SHERPA_ONNX_MODEL_ID)
        );
    }

    fn descriptor_for(model_id: &str) -> SpeakerAnalysisModelDescriptor {
        builtin_model_manifest()
            .models
            .into_iter()
            .find(|model| model.model_id.as_deref() == Some(model_id))
            .unwrap_or_else(|| panic!("manifest is missing model id '{model_id}'"))
    }

    #[test]
    fn manifest_exposes_three_curated_presets() {
        let manifest = builtin_model_manifest();
        let ids: Vec<&str> = manifest
            .models
            .iter()
            .filter_map(|model| model.model_id.as_deref())
            .collect();
        assert_eq!(
            ids,
            vec![
                DEFAULT_SHERPA_ONNX_MODEL_ID,
                MULTILINGUAL_SHERPA_ONNX_MODEL_ID,
                HIGH_ACCURACY_SHERPA_ONNX_MODEL_ID,
            ]
        );
        for model in &manifest.models {
            assert_eq!(model.provider, SHERPA_ONNX_PROVIDER_ID);
            assert!(
                model.sherpa_params.is_some(),
                "preset {:?} must carry sherpa_params",
                model.model_id
            );
        }
    }

    #[test]
    fn default_preset_thresholds_match_calibrated_values() {
        let params = descriptor_for(DEFAULT_SHERPA_ONNX_MODEL_ID)
            .sherpa_params
            .expect("default preset has sherpa_params");
        assert_eq!(params.segmentation_relative_path, "pyannote-segmentation-3.0/model.onnx");
        assert_eq!(params.embedding_relative_path, "nemo_en_titanet_small.onnx");
        // clustering_threshold (fast-clustering, #3) keeps the historical 0.65.
        assert_eq!(params.clustering_threshold, 0.65_f32);
        assert_eq!(params.clustering_threshold, DEFAULT_CLUSTERING_THRESHOLD);
        // cross_chunk_threshold restored to 0.60 (the DER-benchmark optimum;
        // a single-clip calibration had briefly lowered it to 0.50).
        assert_eq!(params.cross_chunk_threshold, 0.60_f32);
        assert_eq!(params.cross_chunk_threshold, BALANCED_CROSS_CHUNK_THRESHOLD);
        assert_eq!(params.min_turn_ms, DEFAULT_MIN_TURN_MS);
    }

    #[test]
    fn multilingual_preset_uses_campplus_zh_en_embedding() {
        let descriptor = descriptor_for(MULTILINGUAL_SHERPA_ONNX_MODEL_ID);
        let params = descriptor.sherpa_params.expect("multilingual sherpa_params");
        assert_eq!(params.segmentation_relative_path, "pyannote-segmentation-3.0/model.onnx");
        assert_eq!(
            params.embedding_relative_path,
            "3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx"
        );
        let ModelManagement::AppManaged { expected_layout, .. } = &descriptor.management;
        assert_eq!(
            expected_layout.required_files,
            vec![
                "pyannote-segmentation-3.0/model.onnx".to_string(),
                "3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx".to_string(),
            ]
        );
    }

    #[test]
    fn high_accuracy_preset_uses_reverb_v1_and_titanet_large() {
        let descriptor = descriptor_for(HIGH_ACCURACY_SHERPA_ONNX_MODEL_ID);
        let params = descriptor.sherpa_params.expect("high-accuracy sherpa_params");
        assert_eq!(params.segmentation_relative_path, "reverb-diarization-v1/model.onnx");
        assert_eq!(params.embedding_relative_path, "nemo_en_titanet_large.onnx");
        let ModelManagement::AppManaged { expected_layout, .. } = &descriptor.management;
        assert_eq!(
            expected_layout.required_files,
            vec![
                "reverb-diarization-v1/model.onnx".to_string(),
                "nemo_en_titanet_large.onnx".to_string(),
            ]
        );
    }

    #[test]
    fn manifest_round_trips_through_json_including_sherpa_params() {
        let manifest = builtin_model_manifest();
        let encoded = serde_json::to_string(&manifest).expect("manifest encodes");
        let decoded: SpeakerAnalysisModelManifest =
            serde_json::from_str(&encoded).expect("manifest decodes");
        assert_eq!(decoded, manifest);
        // sherpa_params serializes camelCase and survives the round trip.
        assert!(encoded.contains("\"sherpaParams\""));
        assert!(encoded.contains("\"segmentationRelativePath\""));
        assert!(encoded.contains("\"clusteringThreshold\""));
        assert!(encoded.contains("\"crossChunkThreshold\""));
        assert!(encoded.contains("\"minTurnMs\""));
    }

    #[test]
    fn descriptor_without_sherpa_params_omits_field_and_round_trips() {
        // Forward-compat: a future non-sherpa provider descriptor has no params.
        let descriptor = SpeakerAnalysisModelDescriptor {
            provider: "future_provider".to_string(),
            model_id: Some("future-model".to_string()),
            display_name: "Future".to_string(),
            description: "Future provider".to_string(),
            license_label: None,
            source_url: None,
            management: ModelManagement::AppManaged {
                expected_layout: InstalledModelLayout {
                    marker_file_name: INSTALLED_MARKER_FILE_NAME.to_string(),
                    required_files: vec!["model.bin".to_string()],
                },
                artifact: None,
            },
            sherpa_params: None,
        };
        let encoded = serde_json::to_string(&descriptor).expect("encodes");
        assert!(!encoded.contains("sherpaParams"));
        let decoded: SpeakerAnalysisModelDescriptor =
            serde_json::from_str(&encoded).expect("decodes");
        assert_eq!(decoded, descriptor);
        assert!(decoded.sherpa_params.is_none());
    }

    #[test]
    fn request_and_output_contract_round_trips_json() {
        let request = SpeakerAnalysisRequest::new(
            "/tmp/audio.wav",
            SHERPA_ONNX_PROVIDER_ID,
            Some(DEFAULT_SHERPA_ONNX_MODEL_ID.to_string()),
            "session-a",
            42,
        );
        let mut output =
            SpeakerAnalysisOutput::new(SpeakerAnalysisMetadata::from_request(&request));
        output.clusters.push(SpeakerCluster {
            provider_cluster_id: "spk0".to_string(),
            stable_label: "Unknown Speaker 1".to_string(),
            embedding: vec![1, 2, 3],
            embedding_model_id: DEFAULT_SHERPA_ONNX_MODEL_ID.to_string(),
            suggestion: None,
        });
        output.turns.push(SpeakerTurn {
            provider_cluster_id: "spk0".to_string(),
            start_ms: 0,
            end_ms: 1000,
            transcript_text: Some("hello".to_string()),
            overlaps: false,
        });

        let encoded = output.structured_payload_json().expect("payload encodes");
        let decoded: SpeakerAnalysisOutput =
            serde_json::from_str(&encoded).expect("payload decodes");
        assert_eq!(decoded.turns[0].provider_cluster_id, "spk0");
        assert_eq!(decoded.metadata.audio_segment_id, 42);
    }
}
