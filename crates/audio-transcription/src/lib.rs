#![allow(unexpected_cfgs)]

//! Provider-neutral audio transcription model manifest and local status primitives.
//!
//! This crate intentionally does not depend on app-infra or Tauri. The desktop
//! app supplies the app data directory and owns download orchestration.

mod core;
mod macos_audio_decode;
pub mod providers;

pub use core::{
    TranscriptionError, TranscriptionMetadata, TranscriptionOutput, TranscriptionProvider,
    TranscriptionRequest, TranscriptionResult, TranscriptionSegment, TranscriptionWord,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read},
    path::{Component, Path, PathBuf},
};
use thiserror::Error;

pub const MODEL_STORE_DIR_NAME: &str = "transcription-models";
pub const INSTALLED_MARKER_FILE_NAME: &str = ".installed.json";
pub const FAILED_MARKER_FILE_NAME: &str = ".failed.json";
pub const DOWNLOADING_MARKER_FILE_NAME: &str = ".download-in-progress";

pub const LOCAL_WHISPER_PROVIDER_ID: &str = "local_whisper";
pub const APPLE_SPEECH_ON_DEVICE_PROVIDER_ID: &str = "apple_speech_on_device";
pub const PARAKEET_PROVIDER_ID: &str = "parakeet";
pub const DEEPGRAM_PROVIDER_ID: &str = "deepgram";

const MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum ModelStatusError {
    #[error("model descriptor for provider {provider} is missing an app-managed model id")]
    MissingAppManagedModelId { provider: String },
    #[error("unsafe path component in {field}: {value}")]
    UnsafePathComponent { field: &'static str, value: String },
    #[error("failed to read marker {path}: {source}")]
    ReadMarker {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse marker {path}: {source}")]
    ParseMarker {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to encode marker {path}: {source}")]
    EncodeMarker {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to write marker {path}: {source}")]
    WriteMarker {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to create model directory {path}: {source}")]
    CreateModelDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Error)]
pub enum ModelInstallError {
    #[error(transparent)]
    Status(#[from] ModelStatusError),
    #[error("app-managed model descriptor for provider {provider} is missing an artifact")]
    MissingArtifact { provider: String },
    #[error("OS-managed model {provider} cannot be installed by the app")]
    OsManagedModel { provider: String },
    #[error("unsafe archive entry path: {path}")]
    UnsafeArchiveEntry { path: String },
    #[error("failed to read archive {path}: {source}")]
    ReadArchive {
        path: PathBuf,
        #[source]
        source: zip::result::ZipError,
    },
    #[error("downloaded artifact checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("installed model layout is incomplete; missing files: {missing_files:?}")]
    IncompleteInstalledLayout { missing_files: Vec<String> },
    #[error("failed to create directory {path}: {source}")]
    CreateDir {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to read file {path}: {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to copy file from {from} to {to}: {source}")]
    CopyFile {
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to create file {path}: {source}")]
    CreateFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to remove file {path}: {source}")]
    RemoveFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to remove directory {path}: {source}")]
    RemoveDir {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioTranscriptionModelManifest {
    pub version: u32,
    pub models: Vec<AudioTranscriptionModelDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioTranscriptionModelDescriptor {
    pub provider: String,
    pub model_id: Option<String>,
    pub display_name: String,
    pub description: String,
    pub license_label: Option<String>,
    pub source_url: Option<String>,
    pub management: ModelManagement,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelManagement {
    AppManaged {
        expected_layout: InstalledModelLayout,
        artifact: Option<ModelArtifact>,
    },
    OsManaged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModelLayout {
    pub marker_file_name: String,
    pub required_files: Vec<String>,
}

impl InstalledModelLayout {
    pub fn single_file(file_name: impl Into<String>) -> Self {
        Self {
            marker_file_name: INSTALLED_MARKER_FILE_NAME.to_string(),
            required_files: vec![file_name.into()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelArtifact {
    pub url: String,
    pub byte_size: u64,
    pub sha256: String,
    pub shape: ModelArtifactShape,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelArtifactFile {
    pub relative_path: String,
    pub url: String,
    pub byte_size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelArtifactShape {
    SingleFile { file_name: String },
    Archive,
    MultiFile { files: Vec<ModelArtifactFile> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModelMarker {
    pub manifest_version: u32,
    pub provider: String,
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FailedModelMarker {
    pub manifest_version: u32,
    pub provider: String,
    pub model_id: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioTranscriptionModelStatus {
    pub provider: String,
    pub model_id: Option<String>,
    pub status: ModelStatusKind,
    pub install_path: Option<PathBuf>,
    pub missing_files: Vec<String>,
    pub failure_message: Option<String>,
}

impl AudioTranscriptionModelStatus {
    pub fn is_available(&self) -> bool {
        matches!(
            self.status,
            ModelStatusKind::Installed | ModelStatusKind::OsManaged
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelStatusKind {
    Installed,
    Missing,
    Downloading,
    Failed,
    OsManaged,
}

pub fn builtin_model_manifest() -> AudioTranscriptionModelManifest {
    AudioTranscriptionModelManifest {
        version: MANIFEST_VERSION,
        models: vec![
            whisper_model(
                "tiny",
                "Whisper Tiny",
                "Smallest local Whisper model; fastest, lowest accuracy.",
                77_691_713,
                "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
            ),
            whisper_model(
                "base",
                "Whisper Base",
                "Default local Whisper model; balanced speed, size, and quality.",
                147_951_465,
                "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
            ),
            whisper_model(
                "small",
                "Whisper Small",
                "Higher quality local Whisper model with larger disk and CPU cost.",
                487_601_967,
                "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
            ),
            whisper_model(
                "medium",
                "Whisper Medium",
                "Largest v1 local Whisper option; best quality, highest resource use.",
                1_533_763_059,
                "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208",
            ),
            AudioTranscriptionModelDescriptor {
                provider: APPLE_SPEECH_ON_DEVICE_PROVIDER_ID.to_string(),
                model_id: None,
                display_name: "Apple Speech (on-device)".to_string(),
                description: "OS-managed on-device Apple Speech recognition executed through Speech.framework. No app-managed download.".to_string(),
                license_label: None,
                source_url: Some("https://developer.apple.com/documentation/speech".to_string()),
                management: ModelManagement::OsManaged,
            },
            AudioTranscriptionModelDescriptor {
                provider: PARAKEET_PROVIDER_ID.to_string(),
                model_id: Some("parakeet-tdt-0.6b-v3-onnx".to_string()),
                display_name: "Parakeet TDT 0.6B v3 ONNX".to_string(),
                description: "Local multilingual Parakeet TDT model executed with a Rust ONNX Runtime adapter. Highest memory use.".to_string(),
                license_label: Some("CC-BY-4.0".to_string()),
                source_url: Some("https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx".to_string()),
                management: ModelManagement::AppManaged {
                    expected_layout: InstalledModelLayout {
                        marker_file_name: INSTALLED_MARKER_FILE_NAME.to_string(),
                        required_files: vec![
                            "config.json".to_string(),
                            "nemo128.onnx".to_string(),
                            "encoder-model.onnx".to_string(),
                            "encoder-model.onnx.data".to_string(),
                            "decoder_joint-model.onnx".to_string(),
                            "vocab.txt".to_string(),
                        ],
                    },
                    artifact: Some(parakeet_v3_onnx_artifact()),
                },
            },
            AudioTranscriptionModelDescriptor {
                provider: PARAKEET_PROVIDER_ID.to_string(),
                model_id: Some("parakeet-tdt-0.6b-v3-onnx-int8".to_string()),
                display_name: "Parakeet TDT 0.6B v3 ONNX int8".to_string(),
                description: "Memory-saving int8 Parakeet ONNX bundle. Smaller download and lower runtime weight memory; accuracy may differ from full precision.".to_string(),
                license_label: Some("CC-BY-4.0".to_string()),
                source_url: Some("https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx".to_string()),
                management: ModelManagement::AppManaged {
                    expected_layout: InstalledModelLayout {
                        marker_file_name: INSTALLED_MARKER_FILE_NAME.to_string(),
                        required_files: vec![
                            "config.json".to_string(),
                            "nemo128.onnx".to_string(),
                            "encoder-model.int8.onnx".to_string(),
                            "decoder_joint-model.int8.onnx".to_string(),
                            "vocab.txt".to_string(),
                        ],
                    },
                    artifact: Some(parakeet_v3_onnx_int8_artifact()),
                },
            },
            AudioTranscriptionModelDescriptor {
                provider: DEEPGRAM_PROVIDER_ID.to_string(),
                model_id: Some("nova-3".to_string()),
                display_name: "Deepgram Nova-3".to_string(),
                description: "Deepgram's latest cloud model. Default. Multilingual detection via `language=multi`. Audio is uploaded to your Deepgram account.".to_string(),
                license_label: None,
                source_url: Some("https://developers.deepgram.com/docs/models-languages-overview".to_string()),
                management: ModelManagement::OsManaged,
            },
            AudioTranscriptionModelDescriptor {
                provider: DEEPGRAM_PROVIDER_ID.to_string(),
                model_id: Some("nova-2".to_string()),
                display_name: "Deepgram Nova-2".to_string(),
                description: "Deepgram cloud model with a broader per-language list (`detect_language`). Audio is uploaded to your Deepgram account.".to_string(),
                license_label: None,
                source_url: Some("https://developers.deepgram.com/docs/models-languages-overview".to_string()),
                management: ModelManagement::OsManaged,
            },
        ],
    }
}

fn parakeet_v3_onnx_artifact() -> ModelArtifact {
    const BASE_URL: &str =
        "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main";
    let files = vec![
        ModelArtifactFile {
            relative_path: "config.json".to_string(),
            url: format!("{BASE_URL}/config.json"),
            byte_size: 97,
            sha256: "666903c76b9798caf2c210afd4f6cd60b08a8dbf9800ec8d7a3bc0d2148ac466".to_string(),
        },
        ModelArtifactFile {
            relative_path: "nemo128.onnx".to_string(),
            url: format!("{BASE_URL}/nemo128.onnx"),
            byte_size: 139_764,
            sha256: "a9fde1486ebfcc08f328d75ad4610c67835fea58c73ba57e3209a6f6cf019e9f".to_string(),
        },
        ModelArtifactFile {
            relative_path: "encoder-model.onnx".to_string(),
            url: format!("{BASE_URL}/encoder-model.onnx"),
            byte_size: 41_770_866,
            sha256: "98a74b21b4cc0017c1e7030319a4a96f4a9506e50f0708f3a516d02a77c96bb1".to_string(),
        },
        ModelArtifactFile {
            relative_path: "encoder-model.onnx.data".to_string(),
            url: format!("{BASE_URL}/encoder-model.onnx.data"),
            byte_size: 2_435_420_160,
            sha256: "9a22d372c51455c34f13405da2520baefb7125bd16981397561423ed32d24f36".to_string(),
        },
        ModelArtifactFile {
            relative_path: "decoder_joint-model.onnx".to_string(),
            url: format!("{BASE_URL}/decoder_joint-model.onnx"),
            byte_size: 72_520_893,
            sha256: "e978ddf6688527182c10fde2eb4b83068421648985ef23f7a86be732be8706c1".to_string(),
        },
        ModelArtifactFile {
            relative_path: "vocab.txt".to_string(),
            url: format!("{BASE_URL}/vocab.txt"),
            byte_size: 93_939,
            sha256: "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d".to_string(),
        },
    ];
    ModelArtifact {
        url: "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx".to_string(),
        byte_size: files.iter().map(|file| file.byte_size).sum(),
        sha256: "".to_string(),
        shape: ModelArtifactShape::MultiFile { files },
    }
}

fn parakeet_v3_onnx_int8_artifact() -> ModelArtifact {
    const BASE_URL: &str =
        "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main";
    // Hugging Face serves these Xet-backed files with an ETag/x-xet-hash that differs from the
    // SHA-256 of the reconstructed bytes we actually download. Keep the checksum aligned with the
    // downloaded file contents (the x-linked-etag header), because install validation hashes the
    // bytes written to disk.
    let files = vec![
        ModelArtifactFile {
            relative_path: "config.json".to_string(),
            url: format!("{BASE_URL}/config.json"),
            byte_size: 97,
            sha256: "666903c76b9798caf2c210afd4f6cd60b08a8dbf9800ec8d7a3bc0d2148ac466".to_string(),
        },
        ModelArtifactFile {
            relative_path: "nemo128.onnx".to_string(),
            url: format!("{BASE_URL}/nemo128.onnx"),
            byte_size: 139_764,
            sha256: "a9fde1486ebfcc08f328d75ad4610c67835fea58c73ba57e3209a6f6cf019e9f".to_string(),
        },
        ModelArtifactFile {
            relative_path: "encoder-model.int8.onnx".to_string(),
            url: format!("{BASE_URL}/encoder-model.int8.onnx"),
            byte_size: 652_183_999,
            sha256: "6139d2fa7e1b086097b277c7149725edbab89cc7c7ae64b23c741be4055aff09".to_string(),
        },
        ModelArtifactFile {
            relative_path: "decoder_joint-model.int8.onnx".to_string(),
            url: format!("{BASE_URL}/decoder_joint-model.int8.onnx"),
            byte_size: 18_202_004,
            sha256: "eea7483ee3d1a30375daedc8ed83e3960c91b098812127a0d99d1c8977667a70".to_string(),
        },
        ModelArtifactFile {
            relative_path: "vocab.txt".to_string(),
            url: format!("{BASE_URL}/vocab.txt"),
            byte_size: 93_939,
            sha256: "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d".to_string(),
        },
    ];
    ModelArtifact {
        url: "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx".to_string(),
        byte_size: files.iter().map(|file| file.byte_size).sum(),
        sha256: "".to_string(),
        shape: ModelArtifactShape::MultiFile { files },
    }
}

fn whisper_model(
    model_id: &str,
    display_name: &str,
    description: &str,
    byte_size: u64,
    sha256: &str,
) -> AudioTranscriptionModelDescriptor {
    AudioTranscriptionModelDescriptor {
        provider: LOCAL_WHISPER_PROVIDER_ID.to_string(),
        model_id: Some(model_id.to_string()),
        display_name: display_name.to_string(),
        description: description.to_string(),
        license_label: Some("MIT".to_string()),
        source_url: Some("https://huggingface.co/ggerganov/whisper.cpp".to_string()),
        management: ModelManagement::AppManaged {
            expected_layout: InstalledModelLayout::single_file(format!("ggml-{model_id}.bin")),
            artifact: Some(ModelArtifact {
                url: format!(
                    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{model_id}.bin"
                ),
                byte_size,
                sha256: sha256.to_string(),
                shape: ModelArtifactShape::SingleFile {
                    file_name: format!("ggml-{model_id}.bin"),
                },
            }),
        },
    }
}

pub fn audio_transcription_models_dir(app_data_dir: impl AsRef<Path>) -> PathBuf {
    app_data_dir.as_ref().join(MODEL_STORE_DIR_NAME)
}

pub fn model_install_dir(
    models_dir: impl AsRef<Path>,
    provider: &str,
    model_id: &str,
) -> Result<PathBuf, ModelStatusError> {
    assert_safe_path_component("provider", provider)?;
    assert_safe_path_component("model_id", model_id)?;
    Ok(models_dir.as_ref().join(provider).join(model_id))
}

pub fn detect_model_status(
    models_dir: impl AsRef<Path>,
    descriptor: &AudioTranscriptionModelDescriptor,
) -> Result<AudioTranscriptionModelStatus, ModelStatusError> {
    match &descriptor.management {
        ModelManagement::OsManaged => Ok(AudioTranscriptionModelStatus {
            provider: descriptor.provider.clone(),
            model_id: descriptor.model_id.clone(),
            status: ModelStatusKind::OsManaged,
            install_path: None,
            missing_files: Vec::new(),
            failure_message: None,
        }),
        ModelManagement::AppManaged {
            expected_layout, ..
        } => {
            let model_id = descriptor.model_id.as_deref().ok_or_else(|| {
                ModelStatusError::MissingAppManagedModelId {
                    provider: descriptor.provider.clone(),
                }
            })?;
            let install_path = model_install_dir(models_dir, &descriptor.provider, model_id)?;
            let missing_files = missing_required_files(&install_path, expected_layout);
            let installed_marker = install_path.join(&expected_layout.marker_file_name);
            let downloading_marker = install_path.join(DOWNLOADING_MARKER_FILE_NAME);
            let failed_marker = install_path.join(FAILED_MARKER_FILE_NAME);

            if installed_marker_matches(&installed_marker, &descriptor.provider, model_id)
                && missing_files.is_empty()
            {
                return Ok(AudioTranscriptionModelStatus {
                    provider: descriptor.provider.clone(),
                    model_id: descriptor.model_id.clone(),
                    status: ModelStatusKind::Installed,
                    install_path: Some(install_path),
                    missing_files,
                    failure_message: None,
                });
            }

            if downloading_marker.exists() {
                return Ok(AudioTranscriptionModelStatus {
                    provider: descriptor.provider.clone(),
                    model_id: descriptor.model_id.clone(),
                    status: ModelStatusKind::Downloading,
                    install_path: Some(install_path),
                    missing_files,
                    failure_message: None,
                });
            }

            if failed_marker.is_file() {
                let message = read_failed_marker(&failed_marker)
                    .map(|marker| marker.message)
                    .unwrap_or_else(|error| error.to_string());
                return Ok(AudioTranscriptionModelStatus {
                    provider: descriptor.provider.clone(),
                    model_id: descriptor.model_id.clone(),
                    status: ModelStatusKind::Failed,
                    install_path: Some(install_path),
                    missing_files,
                    failure_message: Some(message),
                });
            }

            Ok(AudioTranscriptionModelStatus {
                provider: descriptor.provider.clone(),
                model_id: descriptor.model_id.clone(),
                status: ModelStatusKind::Missing,
                install_path: Some(install_path),
                missing_files,
                failure_message: None,
            })
        }
    }
}

pub fn list_model_statuses(
    models_dir: impl AsRef<Path>,
    manifest: &AudioTranscriptionModelManifest,
) -> Result<Vec<AudioTranscriptionModelStatus>, ModelStatusError> {
    manifest
        .models
        .iter()
        .map(|descriptor| detect_model_status(&models_dir, descriptor))
        .collect()
}

pub fn find_model_descriptor<'a>(
    manifest: &'a AudioTranscriptionModelManifest,
    provider: &str,
    model_id: Option<&str>,
) -> Option<&'a AudioTranscriptionModelDescriptor> {
    manifest.models.iter().find(|descriptor| {
        descriptor.provider == provider && descriptor.model_id.as_deref() == model_id
    })
}

pub fn write_downloading_marker(
    models_dir: impl AsRef<Path>,
    provider: &str,
    model_id: &str,
) -> Result<PathBuf, ModelStatusError> {
    let install_path = model_install_dir(models_dir, provider, model_id)?;
    fs::create_dir_all(&install_path).map_err(|source| ModelStatusError::CreateModelDir {
        path: install_path.clone(),
        source,
    })?;
    let path = install_path.join(DOWNLOADING_MARKER_FILE_NAME);
    fs::write(&path, b"").map_err(|source| ModelStatusError::WriteMarker {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

pub fn remove_model_file_if_exists(path: impl AsRef<Path>) -> Result<(), ModelInstallError> {
    let path = path.as_ref();
    if path.exists() {
        fs::remove_file(path).map_err(|source| ModelInstallError::RemoveFile {
            path: path.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

pub fn remove_model_dir_if_exists(path: impl AsRef<Path>) -> Result<(), ModelInstallError> {
    let path = path.as_ref();
    if path.exists() {
        fs::remove_dir_all(path).map_err(|source| ModelInstallError::RemoveDir {
            path: path.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

pub fn sha256_file(path: impl AsRef<Path>) -> Result<String, ModelInstallError> {
    let path = path.as_ref();
    let mut file = fs::File::open(path).map_err(|source| ModelInstallError::ReadFile {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
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
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn validate_artifact_sha256(
    artifact_path: impl AsRef<Path>,
    expected_sha256: &str,
) -> Result<String, ModelInstallError> {
    let actual = sha256_file(artifact_path)?;
    if !actual.eq_ignore_ascii_case(expected_sha256) {
        return Err(ModelInstallError::ChecksumMismatch {
            expected: expected_sha256.to_string(),
            actual,
        });
    }
    Ok(actual)
}

pub fn install_downloaded_model_artifact(
    models_dir: impl AsRef<Path>,
    descriptor: &AudioTranscriptionModelDescriptor,
    artifact_path: impl AsRef<Path>,
) -> Result<PathBuf, ModelInstallError> {
    let (expected_layout, artifact) = match &descriptor.management {
        ModelManagement::AppManaged {
            expected_layout,
            artifact: Some(artifact),
        } => (expected_layout, artifact),
        ModelManagement::AppManaged { artifact: None, .. } => {
            return Err(ModelInstallError::MissingArtifact {
                provider: descriptor.provider.clone(),
            });
        }
        ModelManagement::OsManaged => {
            return Err(ModelInstallError::OsManagedModel {
                provider: descriptor.provider.clone(),
            });
        }
    };

    let model_id = descriptor.model_id.as_deref().ok_or_else(|| {
        ModelStatusError::MissingAppManagedModelId {
            provider: descriptor.provider.clone(),
        }
    })?;
    let models_dir = models_dir.as_ref();
    let install_dir = model_install_dir(models_dir, &descriptor.provider, model_id)?;

    fs::create_dir_all(&install_dir).map_err(|source| ModelInstallError::CreateDir {
        path: install_dir.clone(),
        source,
    })?;
    remove_model_file_if_exists(install_dir.join(INSTALLED_MARKER_FILE_NAME))?;
    remove_model_file_if_exists(install_dir.join(FAILED_MARKER_FILE_NAME))?;

    match &artifact.shape {
        ModelArtifactShape::SingleFile { file_name } => {
            assert_safe_path_component("artifact.file_name", file_name)?;
            let destination = install_dir.join(file_name);
            fs::copy(artifact_path.as_ref(), &destination).map_err(|source| {
                ModelInstallError::CopyFile {
                    from: artifact_path.as_ref().to_path_buf(),
                    to: destination,
                    source,
                }
            })?;
        }
        ModelArtifactShape::Archive => extract_zip_artifact(artifact_path.as_ref(), &install_dir)?,
        ModelArtifactShape::MultiFile { .. } => {
            return Err(ModelInstallError::MissingArtifact {
                provider: descriptor.provider.clone(),
            });
        }
    }

    let missing_files = missing_required_files(&install_dir, expected_layout);
    if !missing_files.is_empty() {
        return Err(ModelInstallError::IncompleteInstalledLayout { missing_files });
    }

    remove_model_file_if_exists(install_dir.join(DOWNLOADING_MARKER_FILE_NAME))?;
    Ok(write_installed_marker(
        models_dir,
        &descriptor.provider,
        model_id,
    )?)
}

pub fn write_installed_marker(
    models_dir: impl AsRef<Path>,
    provider: &str,
    model_id: &str,
) -> Result<PathBuf, ModelStatusError> {
    let install_path = model_install_dir(models_dir, provider, model_id)?;
    fs::create_dir_all(&install_path).map_err(|source| ModelStatusError::CreateModelDir {
        path: install_path.clone(),
        source,
    })?;
    let path = install_path.join(INSTALLED_MARKER_FILE_NAME);
    let marker = InstalledModelMarker {
        manifest_version: MANIFEST_VERSION,
        provider: provider.to_string(),
        model_id: model_id.to_string(),
    };
    let bytes =
        serde_json::to_vec_pretty(&marker).map_err(|source| ModelStatusError::EncodeMarker {
            path: path.clone(),
            source,
        })?;
    fs::write(&path, bytes).map_err(|source| ModelStatusError::WriteMarker {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

pub fn write_failed_marker(
    models_dir: impl AsRef<Path>,
    provider: &str,
    model_id: &str,
    message: impl Into<String>,
) -> Result<PathBuf, ModelStatusError> {
    let install_path = model_install_dir(models_dir, provider, model_id)?;
    fs::create_dir_all(&install_path).map_err(|source| ModelStatusError::CreateModelDir {
        path: install_path.clone(),
        source,
    })?;
    let path = install_path.join(FAILED_MARKER_FILE_NAME);
    let marker = FailedModelMarker {
        manifest_version: MANIFEST_VERSION,
        provider: provider.to_string(),
        model_id: model_id.to_string(),
        message: message.into(),
    };
    let bytes =
        serde_json::to_vec_pretty(&marker).map_err(|source| ModelStatusError::EncodeMarker {
            path: path.clone(),
            source,
        })?;
    fs::write(&path, bytes).map_err(|source| ModelStatusError::WriteMarker {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

fn extract_zip_artifact(artifact_path: &Path, install_dir: &Path) -> Result<(), ModelInstallError> {
    let file = fs::File::open(artifact_path).map_err(|source| ModelInstallError::ReadFile {
        path: artifact_path.to_path_buf(),
        source,
    })?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|source| ModelInstallError::ReadArchive {
            path: artifact_path.to_path_buf(),
            source,
        })?;

    for index in 0..archive.len() {
        let mut entry =
            archive
                .by_index(index)
                .map_err(|source| ModelInstallError::ReadArchive {
                    path: artifact_path.to_path_buf(),
                    source,
                })?;
        let enclosed = entry
            .enclosed_name()
            .ok_or_else(|| ModelInstallError::UnsafeArchiveEntry {
                path: entry.name().to_string(),
            })?
            .to_path_buf();
        let destination = install_dir.join(enclosed);

        if entry.is_dir() {
            fs::create_dir_all(&destination).map_err(|source| ModelInstallError::CreateDir {
                path: destination,
                source,
            })?;
            continue;
        }

        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|source| ModelInstallError::CreateDir {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let mut output =
            fs::File::create(&destination).map_err(|source| ModelInstallError::CreateFile {
                path: destination.clone(),
                source,
            })?;
        io::copy(&mut entry, &mut output).map_err(|source| ModelInstallError::CopyFile {
            from: artifact_path.to_path_buf(),
            to: destination,
            source,
        })?;
    }

    Ok(())
}

fn installed_marker_matches(path: &Path, provider: &str, model_id: &str) -> bool {
    if !path.is_file() {
        return false;
    }
    read_installed_marker(path).is_ok_and(|marker| {
        marker.manifest_version == MANIFEST_VERSION
            && marker.provider == provider
            && marker.model_id == model_id
    })
}

fn read_installed_marker(path: &Path) -> Result<InstalledModelMarker, ModelStatusError> {
    let bytes = fs::read(path).map_err(|source| ModelStatusError::ReadMarker {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(|source| ModelStatusError::ParseMarker {
        path: path.to_path_buf(),
        source,
    })
}

fn read_failed_marker(path: &Path) -> Result<FailedModelMarker, ModelStatusError> {
    let bytes = fs::read(path).map_err(|source| ModelStatusError::ReadMarker {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(|source| ModelStatusError::ParseMarker {
        path: path.to_path_buf(),
        source,
    })
}

fn missing_required_files(model_dir: &Path, expected_layout: &InstalledModelLayout) -> Vec<String> {
    expected_layout
        .required_files
        .iter()
        .filter(|relative_path| !model_dir.join(relative_path).is_file())
        .cloned()
        .collect()
}

fn assert_safe_path_component(field: &'static str, value: &str) -> Result<(), ModelStatusError> {
    if value.is_empty()
        || Path::new(value)
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ModelStatusError::UnsafePathComponent {
            field,
            value: value.to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(model_id: &str) -> AudioTranscriptionModelDescriptor {
        AudioTranscriptionModelDescriptor {
            provider: LOCAL_WHISPER_PROVIDER_ID.to_string(),
            model_id: Some(model_id.to_string()),
            display_name: "Test".to_string(),
            description: "Test model".to_string(),
            license_label: None,
            source_url: None,
            management: ModelManagement::AppManaged {
                expected_layout: InstalledModelLayout::single_file("model.bin"),
                artifact: None,
            },
        }
    }

    fn downloadable_descriptor(
        model_id: &str,
        sha256: String,
    ) -> AudioTranscriptionModelDescriptor {
        AudioTranscriptionModelDescriptor {
            provider: LOCAL_WHISPER_PROVIDER_ID.to_string(),
            model_id: Some(model_id.to_string()),
            display_name: "Test".to_string(),
            description: "Test model".to_string(),
            license_label: None,
            source_url: None,
            management: ModelManagement::AppManaged {
                expected_layout: InstalledModelLayout::single_file("model.bin"),
                artifact: Some(ModelArtifact {
                    url: "https://example.invalid/model.bin".to_string(),
                    byte_size: 5,
                    sha256,
                    shape: ModelArtifactShape::SingleFile {
                        file_name: "model.bin".to_string(),
                    },
                }),
            },
        }
    }

    #[test]
    fn transcription_output_serializes_provider_neutral_metadata() {
        let request = TranscriptionRequest::new(
            "/tmp/audio.m4a",
            LOCAL_WHISPER_PROVIDER_ID,
            Some("base".to_string()),
            "auto",
        );
        let mut metadata = TranscriptionMetadata::from_request(&request);
        metadata.segments.push(TranscriptionSegment {
            start_ms: 120,
            end_ms: 980,
            text: "hello".to_string(),
            confidence: Some(0.91),
        });
        metadata.words.push(TranscriptionWord {
            start_ms: 120,
            end_ms: 420,
            text: "hello".to_string(),
            confidence: None,
        });

        let output = TranscriptionOutput::new("hello", metadata).with_provider_version("mock-1");
        let payload: serde_json::Value = serde_json::from_str(
            &output
                .structured_payload_json()
                .expect("metadata should serialize"),
        )
        .expect("metadata json should parse");

        assert_eq!(payload["provider"], LOCAL_WHISPER_PROVIDER_ID);
        assert_eq!(payload["modelId"], "base");
        assert_eq!(payload["language"], "auto");
        assert_eq!(payload["segments"][0]["startMs"], 120);
        assert_eq!(payload["words"][0]["text"], "hello");
    }

    #[test]
    fn model_store_dir_lives_under_app_data_dir() {
        let app_data = PathBuf::from("/tmp/mnema-app-data");
        assert_eq!(
            audio_transcription_models_dir(&app_data),
            app_data.join(MODEL_STORE_DIR_NAME)
        );
    }

    #[test]
    fn detects_missing_model_without_marker() {
        let temp = tempfile::tempdir().expect("tempdir");
        let status = detect_model_status(temp.path(), &descriptor("base")).expect("status");
        assert_eq!(status.status, ModelStatusKind::Missing);
        assert_eq!(status.missing_files, vec!["model.bin".to_string()]);
        assert!(status
            .install_path
            .expect("install path")
            .ends_with("local_whisper/base"));
    }

    #[test]
    fn installed_marker_requires_expected_layout() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_installed_marker(temp.path(), LOCAL_WHISPER_PROVIDER_ID, "base").expect("marker");

        let missing = detect_model_status(temp.path(), &descriptor("base")).expect("status");
        assert_eq!(missing.status, ModelStatusKind::Missing);
        assert_eq!(missing.missing_files, vec!["model.bin".to_string()]);

        let model_dir =
            model_install_dir(temp.path(), LOCAL_WHISPER_PROVIDER_ID, "base").expect("dir");
        fs::write(model_dir.join("model.bin"), b"model").expect("model file");

        let installed = detect_model_status(temp.path(), &descriptor("base")).expect("status");
        assert_eq!(installed.status, ModelStatusKind::Installed);
        assert!(installed.is_available());
    }

    #[test]
    fn ignores_installed_marker_for_another_model() {
        let temp = tempfile::tempdir().expect("tempdir");
        let model_dir =
            model_install_dir(temp.path(), LOCAL_WHISPER_PROVIDER_ID, "base").expect("dir");
        fs::create_dir_all(&model_dir).expect("model dir");
        fs::write(model_dir.join("model.bin"), b"model").expect("model file");
        fs::write(
            model_dir.join(INSTALLED_MARKER_FILE_NAME),
            serde_json::to_vec(&InstalledModelMarker {
                manifest_version: MANIFEST_VERSION,
                provider: LOCAL_WHISPER_PROVIDER_ID.to_string(),
                model_id: "tiny".to_string(),
            })
            .expect("marker json"),
        )
        .expect("marker");

        let status = detect_model_status(temp.path(), &descriptor("base")).expect("status");
        assert_eq!(status.status, ModelStatusKind::Missing);
    }

    #[test]
    fn detects_downloading_marker() {
        let temp = tempfile::tempdir().expect("tempdir");
        let model_dir =
            model_install_dir(temp.path(), LOCAL_WHISPER_PROVIDER_ID, "base").expect("dir");
        fs::create_dir_all(&model_dir).expect("model dir");
        fs::write(model_dir.join(DOWNLOADING_MARKER_FILE_NAME), b"").expect("downloading marker");

        let status = detect_model_status(temp.path(), &descriptor("base")).expect("status");
        assert_eq!(status.status, ModelStatusKind::Downloading);
    }

    #[test]
    fn detects_failed_marker_with_message() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_failed_marker(
            temp.path(),
            LOCAL_WHISPER_PROVIDER_ID,
            "base",
            "checksum mismatch",
        )
        .expect("failed marker");

        let status = detect_model_status(temp.path(), &descriptor("base")).expect("status");
        assert_eq!(status.status, ModelStatusKind::Failed);
        assert_eq!(status.failure_message.as_deref(), Some("checksum mismatch"));
    }

    #[test]
    fn checksum_validation_reports_mismatch() {
        let temp = tempfile::tempdir().expect("tempdir");
        let artifact = temp.path().join("artifact.bin");
        fs::write(&artifact, b"model").expect("artifact");

        let actual = sha256_file(&artifact).expect("sha256");
        assert_eq!(
            validate_artifact_sha256(&artifact, &actual).expect("matching checksum"),
            actual
        );

        let error =
            validate_artifact_sha256(&artifact, &"0".repeat(64)).expect_err("checksum should fail");
        assert!(matches!(error, ModelInstallError::ChecksumMismatch { .. }));
    }

    #[test]
    fn install_downloaded_single_file_model_writes_marker_after_layout_validation() {
        let temp = tempfile::tempdir().expect("tempdir");
        let artifact = temp.path().join("download.tmp");
        fs::write(&artifact, b"model").expect("artifact");
        let sha256 = sha256_file(&artifact).expect("sha256");
        let descriptor = downloadable_descriptor("base", sha256.clone());

        validate_artifact_sha256(&artifact, &sha256).expect("checksum");
        install_downloaded_model_artifact(temp.path(), &descriptor, &artifact).expect("install");

        let model_dir =
            model_install_dir(temp.path(), LOCAL_WHISPER_PROVIDER_ID, "base").expect("model dir");
        assert_eq!(
            fs::read(model_dir.join("model.bin")).expect("installed"),
            b"model"
        );
        let status = detect_model_status(temp.path(), &descriptor).expect("status");
        assert_eq!(status.status, ModelStatusKind::Installed);
    }

    #[test]
    fn install_downloaded_archive_model_writes_marker_after_layout_validation() {
        let temp = tempfile::tempdir().expect("tempdir");
        let artifact = temp.path().join("download.zip");
        {
            let file = fs::File::create(&artifact).expect("zip file");
            let mut zip = zip::ZipWriter::new(file);
            zip.start_file("model.bin", zip::write::SimpleFileOptions::default())
                .expect("zip entry");
            use std::io::Write as _;
            zip.write_all(b"model").expect("zip bytes");
            zip.finish().expect("finish zip");
        }
        let sha256 = sha256_file(&artifact).expect("sha256");
        let mut descriptor = downloadable_descriptor("base", sha256.clone());
        if let ModelManagement::AppManaged {
            artifact: Some(artifact),
            ..
        } = &mut descriptor.management
        {
            artifact.shape = ModelArtifactShape::Archive;
        }

        validate_artifact_sha256(&artifact, &sha256).expect("checksum");
        install_downloaded_model_artifact(temp.path(), &descriptor, &artifact).expect("install");

        let model_dir =
            model_install_dir(temp.path(), LOCAL_WHISPER_PROVIDER_ID, "base").expect("model dir");
        assert_eq!(
            fs::read(model_dir.join("model.bin")).expect("installed"),
            b"model"
        );
        assert_eq!(
            detect_model_status(temp.path(), &descriptor)
                .expect("status")
                .status,
            ModelStatusKind::Installed
        );
    }

    #[test]
    fn cancel_cleanup_primitives_remove_temp_and_downloading_marker() {
        let temp = tempfile::tempdir().expect("tempdir");
        let model_dir =
            model_install_dir(temp.path(), LOCAL_WHISPER_PROVIDER_ID, "base").expect("dir");
        fs::create_dir_all(&model_dir).expect("model dir");
        let temp_file = model_dir.join(".download.tmp");
        fs::write(&temp_file, b"partial").expect("temp file");
        write_downloading_marker(temp.path(), LOCAL_WHISPER_PROVIDER_ID, "base")
            .expect("downloading marker");

        remove_model_file_if_exists(&temp_file).expect("remove temp");
        remove_model_file_if_exists(model_dir.join(DOWNLOADING_MARKER_FILE_NAME))
            .expect("remove marker");

        assert!(!temp_file.exists());
        let status = detect_model_status(temp.path(), &descriptor("base")).expect("status");
        assert_eq!(status.status, ModelStatusKind::Missing);
    }

    #[test]
    fn builtin_whisper_descriptors_include_download_artifacts() {
        let manifest = builtin_model_manifest();
        let base = manifest
            .models
            .iter()
            .find(|model| {
                model.provider == LOCAL_WHISPER_PROVIDER_ID
                    && model.model_id.as_deref() == Some("base")
            })
            .expect("base whisper model");

        let ModelManagement::AppManaged {
            expected_layout,
            artifact: Some(artifact),
        } = &base.management
        else {
            panic!("whisper base should be app-managed with a download artifact");
        };

        assert_eq!(expected_layout.required_files, vec!["ggml-base.bin"]);
        assert_eq!(artifact.byte_size, 147_951_465);
        assert_eq!(
            artifact.sha256,
            "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe"
        );
        assert_eq!(
            artifact.url,
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin"
        );
        assert_eq!(
            artifact.shape,
            ModelArtifactShape::SingleFile {
                file_name: "ggml-base.bin".to_string()
            }
        );
    }

    #[test]
    fn builtin_parakeet_descriptor_includes_download_artifact() {
        let manifest = builtin_model_manifest();
        let parakeet = manifest
            .models
            .iter()
            .find(|model| {
                model.provider == PARAKEET_PROVIDER_ID
                    && model.model_id.as_deref() == Some("parakeet-tdt-0.6b-v3-onnx")
            })
            .expect("parakeet model");

        let ModelManagement::AppManaged {
            expected_layout,
            artifact: Some(artifact),
        } = &parakeet.management
        else {
            panic!("parakeet should be app-managed with a download artifact");
        };

        assert_eq!(
            expected_layout.required_files,
            vec![
                "config.json",
                "nemo128.onnx",
                "encoder-model.onnx",
                "encoder-model.onnx.data",
                "decoder_joint-model.onnx",
                "vocab.txt",
            ]
        );
        assert_eq!(artifact.byte_size, 2_549_945_719);
        assert_eq!(
            artifact.url,
            "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx"
        );
        let ModelArtifactShape::MultiFile { files } = &artifact.shape else {
            panic!("parakeet ONNX should use multi-file artifact shape");
        };
        assert_eq!(files.len(), 6);
        assert!(files
            .iter()
            .any(|file| file.relative_path == "encoder-model.onnx.data"));

        let parakeet_int8 = manifest
            .models
            .iter()
            .find(|model| {
                model.provider == PARAKEET_PROVIDER_ID
                    && model.model_id.as_deref() == Some("parakeet-tdt-0.6b-v3-onnx-int8")
            })
            .expect("parakeet int8 model");
        let ModelManagement::AppManaged {
            expected_layout,
            artifact: Some(artifact),
        } = &parakeet_int8.management
        else {
            panic!("parakeet int8 should be app-managed with a download artifact");
        };
        assert_eq!(
            expected_layout.required_files,
            vec![
                "config.json",
                "nemo128.onnx",
                "encoder-model.int8.onnx",
                "decoder_joint-model.int8.onnx",
                "vocab.txt",
            ]
        );
        assert_eq!(artifact.byte_size, 670_619_803);
        let ModelArtifactShape::MultiFile { files } = &artifact.shape else {
            panic!("parakeet int8 ONNX should use multi-file artifact shape");
        };
        assert_eq!(files.len(), 5);
        assert!(files.iter().any(|file| {
            file.relative_path == "encoder-model.int8.onnx"
                && file.sha256 == "6139d2fa7e1b086097b277c7149725edbab89cc7c7ae64b23c741be4055aff09"
        }));
        assert!(files.iter().any(|file| {
            file.relative_path == "decoder_joint-model.int8.onnx"
                && file.sha256 == "eea7483ee3d1a30375daedc8ed83e3960c91b098812127a0d99d1c8977667a70"
        }));
    }

    #[test]
    fn apple_speech_descriptor_is_os_managed() {
        let manifest = builtin_model_manifest();
        let apple = manifest
            .models
            .iter()
            .find(|model| model.provider == APPLE_SPEECH_ON_DEVICE_PROVIDER_ID)
            .expect("apple model");
        let status = detect_model_status("/unused", apple).expect("status");

        assert_eq!(status.status, ModelStatusKind::OsManaged);
        assert_eq!(status.install_path, None);
        assert!(status.is_available());
    }

    #[test]
    fn rejects_path_traversal_components() {
        let error = model_install_dir("/tmp/models", "local_whisper", "../base")
            .expect_err("unsafe model id");
        assert!(matches!(
            error,
            ModelStatusError::UnsafePathComponent {
                field: "model_id",
                ..
            }
        ));
    }
}
