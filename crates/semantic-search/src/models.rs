//! Semantic Search Model catalog, on-disk layout, and model-gating detector.
//!
//! The detector mirrors the audio-transcription model detector: a model is
//! Installed when an `.installed.json` marker for that exact provider/model id
//! sits in `semantic_search_models/{provider}/{model_id}/` alongside every
//! required file. Anything else is Missing — and a Missing model makes
//! **Semantic Search** a silent no-op, never an error.

use std::path::{Component, Path, PathBuf};

use capture_types::SemanticSearchSettings;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// App-data subdirectory that holds installed Semantic Search Models, laid out
/// as `semantic_search_models/{provider}/{model_id}/` (ADR 0036).
pub const MODEL_STORE_DIR_NAME: &str = "semantic_search_models";

/// Marker file written into a model directory once every required file is
/// present, mirroring the transcription installer's `.installed.json`.
pub const INSTALLED_MARKER_FILE_NAME: &str = ".installed.json";

/// The single embedding provider in v1: fastembed running on the shared `ort`.
pub const FASTEMBED_PROVIDER_ID: &str = "fastembed";

/// The fastembed model files a user-defined ("bring your own") embedder loads
/// from disk. These are the file names fastembed expects, so a Semantic Search
/// Model is only Installed when all of them exist.
pub const MODEL_ONNX_FILE_NAME: &str = "model.onnx";
pub const TOKENIZER_FILE_NAME: &str = "tokenizer.json";
pub const TOKENIZER_CONFIG_FILE_NAME: &str = "tokenizer_config.json";
pub const SPECIAL_TOKENS_MAP_FILE_NAME: &str = "special_tokens_map.json";
pub const CONFIG_FILE_NAME: &str = "config.json";

const MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum ModelStatusError {
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
}

/// A user-facing **Semantic Search Model Tier** (ADR 0036): an English default,
/// a Multilingual option, and a Custom selection over locally supported models.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SemanticSearchModelTier {
    English,
    Multilingual,
    Custom,
}

/// The sentence-pooling strategy a **Semantic Search Model** reads its single
/// vector with: `Mean`-pool `last_hidden_state` (nomic / e5) or read the `[CLS]`
/// token (bge / mxbai / gte / snowflake-arctic). A serde-friendly mirror of
/// fastembed's `Pooling` so the descriptor carries pooling **without** this
/// (non-`fastembed`-feature) module taking a fastembed dependency; the runtime
/// converts to/from `fastembed::Pooling` behind the feature. Getting this wrong
/// silently mean-pools a CLS-trained model — a wrong, lower-quality vector — so a
/// model's pooling is captured from fastembed's own `get_default_pooling_method`,
/// never guessed from the id.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SemanticSearchPooling {
    Mean,
    Cls,
}

/// Which session output a model reads its embedding from — a serde-friendly
/// mirror of fastembed's `OutputKey`. Almost every sentence model uses the
/// default (`OnlyOne`); a few name a specific tensor (e.g. `sentence_embedding`).
/// Carried through the descriptor so a model that names its output stays correct,
/// matching fastembed's own `ModelInfo.output_key`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum SemanticSearchOutputKey {
    OnlyOne,
    ByOrder(usize),
    ByName(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SemanticSearchModelManifest {
    pub version: u32,
    pub models: Vec<SemanticSearchModelDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SemanticSearchModelDescriptor {
    pub provider: String,
    pub model_id: String,
    pub display_name: String,
    pub description: String,
    pub tier: SemanticSearchModelTier,
    /// The fastembed model code (HuggingFace repo id) the Settings slice uses to
    /// fetch the model; recorded here so the catalog is the one source of truth.
    pub model_code: String,
    pub license_label: Option<String>,
    /// Vector dimension this model produces. The default English tier is 768 to
    /// match the `search_document_vectors vec0(embedding float[768])` table.
    pub dimension: usize,
    /// The model's token window. Text overflowing this is auto-split on overflow
    /// (never silently truncated) before embedding.
    pub max_tokens: usize,
    /// Approximate on-disk footprint of the downloaded model, in bytes. Surfaced
    /// in Settings as the disk-cost disclosure (ADR 0036) so a user sees the cost
    /// before choosing a tier. Approximate because it is the quantized ONNX size,
    /// not a network-measured total.
    pub approx_download_bytes: u64,
    /// The sentence-pooling strategy the runtime loads this model with. Captured
    /// from fastembed's `get_default_pooling_method` for synthesized Custom picks
    /// and hand-set on the guided tiers — never guessed from the id, so a
    /// CLS-trained model (mxbai / gte / snowflake-arctic) is read at the `[CLS]`
    /// token instead of being silently mean-pooled into a wrong, lower-quality
    /// vector.
    pub pooling: SemanticSearchPooling,
    /// The session output the model reads its embedding from (fastembed's
    /// `ModelInfo.output_key`). `None` for the default single output; carried
    /// through so a model that names a specific output tensor stays correct.
    #[serde(default)]
    pub output_key: Option<SemanticSearchOutputKey>,
    pub expected_layout: InstalledModelLayout,
}

/// The on-disk layout of an installed **Semantic Search Model**.
///
/// Every path here is **repo-relative** (the same path used both as the
/// HuggingFace `resolve/main/<path>` download path AND as the on-disk path under
/// the model's install dir). Preserving the repo-relative subdirectory matters:
/// an ONNX graph with external data (`model.onnx_data`) references its sibling by
/// the relative path stored in the model, so `onnx/model.onnx` and
/// `onnx/model.onnx_data` MUST stay together under `onnx/`.
///
/// A model is **Installed** only when every entry in `required_files` (the ONNX
/// file, all `external_data_files`, and the four tokenizer/config files) is
/// present alongside the marker — so a model whose 2 GB `model.onnx_data` is
/// missing is correctly reported Missing, never a broken "Installed".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModelLayout {
    pub marker_file_name: String,
    /// Every required file, repo-relative (ONNX + external data + tokenizers).
    pub required_files: Vec<String>,
    /// The repo-relative path of the primary ONNX graph (e.g. `onnx/model.onnx`).
    /// The runtime loads this from disk and hands it to fastembed in memory.
    pub onnx_relative_path: String,
    /// Repo-relative paths of the ONNX external-data siblings (e.g.
    /// `onnx/model.onnx_data`). For an in-memory ("bring your own") load these are
    /// passed to fastembed as external initializers, keyed by their **basename**
    /// (the name the graph references), since the graph is loaded from memory and
    /// cannot resolve a sibling file by directory.
    pub external_data_files: Vec<String>,
}

/// The four tokenizer/config files fastembed always loads from the repo root.
fn root_tokenizer_files() -> Vec<String> {
    vec![
        TOKENIZER_FILE_NAME.to_string(),
        TOKENIZER_CONFIG_FILE_NAME.to_string(),
        SPECIAL_TOKENS_MAP_FILE_NAME.to_string(),
        CONFIG_FILE_NAME.to_string(),
    ]
}

impl InstalledModelLayout {
    /// Build a layout from the fastembed `ModelInfo`-derived facts: the ONNX file
    /// path (`onnx/model.onnx`), any external-data siblings, plus the four root
    /// tokenizer files. `required_files` is the union, in download order.
    pub fn from_fastembed_files(
        onnx_relative_path: impl Into<String>,
        external_data_files: Vec<String>,
    ) -> Self {
        let onnx_relative_path = onnx_relative_path.into();
        let mut required_files = Vec::new();
        required_files.push(onnx_relative_path.clone());
        required_files.extend(external_data_files.iter().cloned());
        required_files.extend(root_tokenizer_files());
        Self {
            marker_file_name: INSTALLED_MARKER_FILE_NAME.to_string(),
            required_files,
            onnx_relative_path,
            external_data_files,
        }
    }
}

impl Default for InstalledModelLayout {
    /// The common self-contained layout: `onnx/model.onnx` with no external data,
    /// plus the four root tokenizer files. Used by models (nomic / e5-small) whose
    /// ONNX has no `*.onnx_data` sibling.
    fn default() -> Self {
        Self::from_fastembed_files("onnx/model.onnx", Vec::new())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelStatusKind {
    Installed,
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SemanticSearchModelStatus {
    pub provider: String,
    pub model_id: String,
    pub status: ModelStatusKind,
    pub install_path: PathBuf,
    pub missing_files: Vec<String>,
}

impl SemanticSearchModelStatus {
    pub fn is_available(&self) -> bool {
        matches!(self.status, ModelStatusKind::Installed)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct InstalledModelMarker {
    manifest_version: u32,
    provider: String,
    model_id: String,
}

/// The catalog of guided **Semantic Search Model Tiers** plus the Custom
/// fallback. The default English tier is `nomic-embed-text-v1.5` (768-dim,
/// 8192-token, Apache-2.0) per ADR 0036.
pub fn builtin_model_manifest() -> SemanticSearchModelManifest {
    SemanticSearchModelManifest {
        version: MANIFEST_VERSION,
        models: vec![
            SemanticSearchModelDescriptor {
                provider: FASTEMBED_PROVIDER_ID.to_string(),
                model_id: "nomic-embed-text-v1.5".to_string(),
                display_name: "Nomic Embed Text v1.5 (English)".to_string(),
                description: "Default English tier: long-context (8192 tokens), \
                    Apache-2.0, 768-dimensional. Long context makes truncation a \
                    non-issue and the permissive license keeps the default path \
                    obligation-free."
                    .to_string(),
                tier: SemanticSearchModelTier::English,
                model_code: "nomic-ai/nomic-embed-text-v1.5".to_string(),
                license_label: Some("Apache-2.0".to_string()),
                dimension: 768,
                max_tokens: 8192,
                // ~140 MB quantized ONNX.
                approx_download_bytes: 140_000_000,
                // nomic is mean-pooled (fastembed `get_default_pooling_method`).
                pooling: SemanticSearchPooling::Mean,
                output_key: None,
                expected_layout: InstalledModelLayout::default(),
            },
            SemanticSearchModelDescriptor {
                provider: FASTEMBED_PROVIDER_ID.to_string(),
                model_id: "multilingual-e5-small".to_string(),
                display_name: "Multilingual E5 Small (Multilingual)".to_string(),
                description: "Multilingual tier: covers 100+ languages, non-gated \
                    (MIT), 384-dimensional. A non-English user is guided here rather \
                    than silently degraded by the English default, and it serves \
                    English well too. Self-contained ONNX (no external data)."
                    .to_string(),
                tier: SemanticSearchModelTier::Multilingual,
                model_code: "intfloat/multilingual-e5-small".to_string(),
                license_label: Some("MIT".to_string()),
                dimension: 384,
                max_tokens: 512,
                // ~465 MB on disk.
                approx_download_bytes: 465_000_000,
                // multilingual-e5-small is mean-pooled.
                pooling: SemanticSearchPooling::Mean,
                output_key: None,
                // Self-contained `onnx/model.onnx`, no `*.onnx_data` sibling.
                expected_layout: InstalledModelLayout::default(),
            },
            SemanticSearchModelDescriptor {
                provider: FASTEMBED_PROVIDER_ID.to_string(),
                model_id: "bge-m3".to_string(),
                display_name: "BGE-M3 (Multilingual, Custom)".to_string(),
                description: "Custom multilingual option (BAAI/bge-m3), 1024-dimensional, \
                    8192-token. Available via the Custom picker."
                    .to_string(),
                tier: SemanticSearchModelTier::Custom,
                model_code: "BAAI/bge-m3".to_string(),
                license_label: Some("MIT".to_string()),
                dimension: 1024,
                max_tokens: 8192,
                // ~2.3 GB: the `onnx/model.onnx` graph plus its `onnx/model.onnx_data`
                // external-data sibling and `onnx/Constant_7_attr__value`.
                approx_download_bytes: 2_300_000_000,
                // bge-m3 is CLS-pooled (fastembed `get_default_pooling_method`).
                pooling: SemanticSearchPooling::Cls,
                output_key: None,
                // bge-m3 ships external data: the ONNX graph references
                // `onnx/model.onnx_data` (and `onnx/Constant_7_attr__value`), so they
                // are part of the install layout and the completeness check.
                expected_layout: InstalledModelLayout::from_fastembed_files(
                    "onnx/model.onnx",
                    vec![
                        "onnx/model.onnx_data".to_string(),
                        "onnx/Constant_7_attr__value".to_string(),
                    ],
                ),
            },
        ],
    }
}

pub fn find_model_descriptor<'a>(
    manifest: &'a SemanticSearchModelManifest,
    provider: &str,
    model_id: &str,
) -> Option<&'a SemanticSearchModelDescriptor> {
    manifest
        .models
        .iter()
        .find(|descriptor| descriptor.provider == provider && descriptor.model_id == model_id)
}

/// The app-data directory that holds installed Semantic Search Models.
pub fn semantic_search_models_dir(app_data_dir: impl AsRef<Path>) -> PathBuf {
    app_data_dir.as_ref().join(MODEL_STORE_DIR_NAME)
}

/// `semantic_search_models/{provider}/{model_id}/`, rejecting path traversal.
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
    descriptor: &SemanticSearchModelDescriptor,
) -> Result<SemanticSearchModelStatus, ModelStatusError> {
    let install_path =
        model_install_dir(models_dir, &descriptor.provider, &descriptor.model_id)?;
    let missing_files = missing_required_files(&install_path, &descriptor.expected_layout);
    let installed_marker = install_path.join(&descriptor.expected_layout.marker_file_name);

    let status = if missing_files.is_empty()
        && installed_marker_matches(&installed_marker, &descriptor.provider, &descriptor.model_id)?
    {
        ModelStatusKind::Installed
    } else {
        ModelStatusKind::Missing
    };

    Ok(SemanticSearchModelStatus {
        provider: descriptor.provider.clone(),
        model_id: descriptor.model_id.clone(),
        status,
        install_path,
        missing_files,
    })
}

/// Model-gating: is the user's selected **Semantic Search Model** installed?
///
/// Returns `false` (a silent no-op admission, never an error) when the feature
/// is disabled, no model is selected, the selection is not a known model, or the
/// model is not yet installed. The only `Err` path is a corrupt marker file.
pub fn selected_semantic_search_model_available(
    app_data_dir: impl AsRef<Path>,
    settings: &SemanticSearchSettings,
) -> Result<bool, ModelStatusError> {
    if !settings.enabled {
        return Ok(false);
    }
    let Some(model_id) = settings.model_id.as_deref() else {
        return Ok(false);
    };
    let manifest = builtin_model_manifest();
    let Some(descriptor) = find_model_descriptor(&manifest, &settings.provider, model_id) else {
        return Ok(false);
    };
    let status =
        detect_model_status(semantic_search_models_dir(app_data_dir), descriptor)?;
    Ok(status.is_available())
}

/// Write the `.installed.json` marker into a freshly-downloaded model directory,
/// mirroring the transcription installer. The detector only treats a model as
/// Installed once this marker (matching the exact provider/model id) sits
/// alongside every required file, so the downloader writes it last.
pub fn write_installed_marker(
    models_dir: impl AsRef<Path>,
    provider: &str,
    model_id: &str,
) -> Result<(), ModelStatusError> {
    let install_dir = model_install_dir(models_dir, provider, model_id)?;
    let marker = InstalledModelMarker {
        manifest_version: MANIFEST_VERSION,
        provider: provider.to_string(),
        model_id: model_id.to_string(),
    };
    let path = install_dir.join(INSTALLED_MARKER_FILE_NAME);
    let bytes = serde_json::to_vec(&marker).map_err(|source| ModelStatusError::ParseMarker {
        path: path.clone(),
        source,
    })?;
    std::fs::write(&path, bytes).map_err(|source| ModelStatusError::ReadMarker { path, source })?;
    Ok(())
}

fn missing_required_files(model_dir: &Path, layout: &InstalledModelLayout) -> Vec<String> {
    layout
        .required_files
        .iter()
        .filter(|relative_path| !model_dir.join(relative_path).is_file())
        .cloned()
        .collect()
}

fn installed_marker_matches(
    path: &Path,
    provider: &str,
    model_id: &str,
) -> Result<bool, ModelStatusError> {
    if !path.is_file() {
        return Ok(false);
    }
    let bytes = std::fs::read(path).map_err(|source| ModelStatusError::ReadMarker {
        path: path.to_path_buf(),
        source,
    })?;
    let marker: InstalledModelMarker =
        serde_json::from_slice(&bytes).map_err(|source| ModelStatusError::ParseMarker {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(marker.manifest_version == MANIFEST_VERSION
        && marker.provider == provider
        && marker.model_id == model_id)
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
    use capture_types::default_semantic_search_settings;
    use std::fs;

    /// Write a (possibly nested, e.g. `onnx/model.onnx`) required file, creating
    /// its parent directory so the repo-relative layout is reproduced on disk.
    fn write_required_file(install_dir: &Path, relative_path: &str) {
        let path = install_dir.join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent dir");
        }
        fs::write(path, b"x").expect("model file");
    }

    fn install_model(models_dir: &Path, descriptor: &SemanticSearchModelDescriptor) {
        let install_dir =
            model_install_dir(models_dir, &descriptor.provider, &descriptor.model_id).expect("dir");
        fs::create_dir_all(&install_dir).expect("install dir");
        for file_name in &descriptor.expected_layout.required_files {
            write_required_file(&install_dir, file_name);
        }
        let marker = InstalledModelMarker {
            manifest_version: MANIFEST_VERSION,
            provider: descriptor.provider.clone(),
            model_id: descriptor.model_id.clone(),
        };
        fs::write(
            install_dir.join(INSTALLED_MARKER_FILE_NAME),
            serde_json::to_vec(&marker).expect("marker json"),
        )
        .expect("marker");
    }

    #[test]
    fn models_dir_lives_under_app_data_dir() {
        let app_data = PathBuf::from("/tmp/mnema-app-data");
        assert_eq!(
            semantic_search_models_dir(&app_data),
            app_data.join(MODEL_STORE_DIR_NAME)
        );
        assert!(semantic_search_models_dir(&app_data).ends_with("semantic_search_models"));
    }

    #[test]
    fn default_english_tier_is_nomic_768_dim() {
        let manifest = builtin_model_manifest();
        let default = find_model_descriptor(&manifest, FASTEMBED_PROVIDER_ID, "nomic-embed-text-v1.5")
            .expect("english tier");
        assert_eq!(default.tier, SemanticSearchModelTier::English);
        assert_eq!(default.dimension, 768);
        assert_eq!(default.max_tokens, 8192);
        assert_eq!(default.license_label.as_deref(), Some("Apache-2.0"));
    }

    #[test]
    fn missing_model_is_not_available_and_lists_missing_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        let manifest = builtin_model_manifest();
        let descriptor = &manifest.models[0];
        let status = detect_model_status(semantic_search_models_dir(temp.path()), descriptor)
            .expect("status");
        assert_eq!(status.status, ModelStatusKind::Missing);
        assert!(!status.is_available());
        // The ONNX file is required at its repo-relative path under `onnx/`.
        assert!(status
            .missing_files
            .contains(&descriptor.expected_layout.onnx_relative_path));
        assert!(status.install_path.ends_with("fastembed/nomic-embed-text-v1.5"));
    }

    #[test]
    fn installed_model_requires_marker_and_every_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let models_dir = semantic_search_models_dir(temp.path());
        let manifest = builtin_model_manifest();
        let descriptor = &manifest.models[0];

        // Files present but no marker => still Missing.
        let install_dir =
            model_install_dir(&models_dir, &descriptor.provider, &descriptor.model_id).expect("dir");
        fs::create_dir_all(&install_dir).expect("install dir");
        for file_name in &descriptor.expected_layout.required_files {
            write_required_file(&install_dir, file_name);
        }
        assert_eq!(
            detect_model_status(&models_dir, descriptor).expect("status").status,
            ModelStatusKind::Missing
        );

        // Add the marker => Installed.
        let marker = InstalledModelMarker {
            manifest_version: MANIFEST_VERSION,
            provider: descriptor.provider.clone(),
            model_id: descriptor.model_id.clone(),
        };
        fs::write(
            install_dir.join(INSTALLED_MARKER_FILE_NAME),
            serde_json::to_vec(&marker).expect("marker json"),
        )
        .expect("marker");
        let installed = detect_model_status(&models_dir, descriptor).expect("status");
        assert_eq!(installed.status, ModelStatusKind::Installed);
        assert!(installed.is_available());
    }

    #[test]
    fn model_with_external_data_is_missing_until_onnx_data_is_present() {
        // bge-m3's `onnx/model.onnx_data` (~2 GB) is part of the layout: a model
        // whose external data is absent must NOT be reported Installed, even with
        // the ONNX graph, tokenizers, and marker present.
        let temp = tempfile::tempdir().expect("tempdir");
        let models_dir = semantic_search_models_dir(temp.path());
        let manifest = builtin_model_manifest();
        let descriptor = find_model_descriptor(&manifest, FASTEMBED_PROVIDER_ID, "bge-m3")
            .expect("bge-m3 descriptor");
        assert!(
            !descriptor.expected_layout.external_data_files.is_empty(),
            "bge-m3 must declare external-data files"
        );
        let install_dir =
            model_install_dir(&models_dir, &descriptor.provider, &descriptor.model_id).expect("dir");
        fs::create_dir_all(&install_dir).expect("install dir");

        // Write every required file EXCEPT the external-data siblings, plus marker.
        let external: Vec<&String> = descriptor.expected_layout.external_data_files.iter().collect();
        for file_name in &descriptor.expected_layout.required_files {
            if external.contains(&file_name) {
                continue;
            }
            write_required_file(&install_dir, file_name);
        }
        let marker = InstalledModelMarker {
            manifest_version: MANIFEST_VERSION,
            provider: descriptor.provider.clone(),
            model_id: descriptor.model_id.clone(),
        };
        fs::write(
            install_dir.join(INSTALLED_MARKER_FILE_NAME),
            serde_json::to_vec(&marker).expect("marker json"),
        )
        .expect("marker");

        let status = detect_model_status(&models_dir, descriptor).expect("status");
        assert_eq!(
            status.status,
            ModelStatusKind::Missing,
            "external data missing => Missing, never a broken Installed"
        );
        for external_file in &descriptor.expected_layout.external_data_files {
            assert!(
                status.missing_files.contains(external_file),
                "external-data file {external_file} must be reported missing"
            );
        }

        // Now add the external data => Installed.
        for external_file in &descriptor.expected_layout.external_data_files {
            write_required_file(&install_dir, external_file);
        }
        assert_eq!(
            detect_model_status(&models_dir, descriptor).expect("status").status,
            ModelStatusKind::Installed
        );
    }

    #[test]
    fn marker_for_another_model_does_not_count() {
        let temp = tempfile::tempdir().expect("tempdir");
        let models_dir = semantic_search_models_dir(temp.path());
        let manifest = builtin_model_manifest();
        let descriptor = &manifest.models[0];
        let install_dir =
            model_install_dir(&models_dir, &descriptor.provider, &descriptor.model_id).expect("dir");
        fs::create_dir_all(&install_dir).expect("install dir");
        for file_name in &descriptor.expected_layout.required_files {
            write_required_file(&install_dir, file_name);
        }
        let marker = InstalledModelMarker {
            manifest_version: MANIFEST_VERSION,
            provider: descriptor.provider.clone(),
            model_id: "some-other-model".to_string(),
        };
        fs::write(
            install_dir.join(INSTALLED_MARKER_FILE_NAME),
            serde_json::to_vec(&marker).expect("marker json"),
        )
        .expect("marker");
        assert_eq!(
            detect_model_status(&models_dir, descriptor).expect("status").status,
            ModelStatusKind::Missing
        );
    }

    #[test]
    fn no_installed_model_makes_feature_a_silent_no_op_not_an_error() {
        // Mirrors the transcription backfill skip: default-on settings with no
        // model on disk resolve to "unavailable" (Ok(false)) — never Err, never
        // a capture-blocking failure.
        let temp = tempfile::tempdir().expect("tempdir");
        let settings = default_semantic_search_settings();
        assert!(settings.enabled, "default settings are on");
        let available = selected_semantic_search_model_available(temp.path(), &settings)
            .expect("availability check must not error when the model is absent");
        assert!(!available, "no installed model => silent no-op (false)");
    }

    #[test]
    fn selected_model_available_only_once_installed() {
        let temp = tempfile::tempdir().expect("tempdir");
        let settings = default_semantic_search_settings();
        let models_dir = semantic_search_models_dir(temp.path());
        let manifest = builtin_model_manifest();
        let descriptor =
            find_model_descriptor(&manifest, &settings.provider, settings.model_id.as_deref().unwrap())
                .expect("selected descriptor");

        assert!(!selected_semantic_search_model_available(temp.path(), &settings).expect("check"));
        install_model(&models_dir, descriptor);
        assert!(selected_semantic_search_model_available(temp.path(), &settings).expect("check"));
    }

    #[test]
    fn disabled_settings_are_never_available_even_with_a_model() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut settings = default_semantic_search_settings();
        let models_dir = semantic_search_models_dir(temp.path());
        let manifest = builtin_model_manifest();
        install_model(&models_dir, &manifest.models[0]);

        settings.enabled = false;
        assert!(!selected_semantic_search_model_available(temp.path(), &settings).expect("check"));
    }

    #[test]
    fn unknown_selected_model_is_not_available() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut settings = default_semantic_search_settings();
        settings.model_id = Some("not-a-real-model".to_string());
        assert!(!selected_semantic_search_model_available(temp.path(), &settings).expect("check"));
    }

    #[test]
    fn rejects_path_traversal_components() {
        let error =
            model_install_dir("/tmp/models", FASTEMBED_PROVIDER_ID, "../escape").expect_err("unsafe");
        assert!(matches!(
            error,
            ModelStatusError::UnsafePathComponent { field: "model_id", .. }
        ));
    }

    /// Extract the `version = "..."` pin from a crate's `ort = { ... }` (or
    /// `ort = "..."`) dependency line in a Cargo.toml. fastembed (here) and
    /// Parakeet (`crates/audio-transcription`) must pin the SAME `ort` so the
    /// workspace links exactly one native ONNX runtime — see the lockstep note in
    /// both Cargo.toml files and ADR 0036. Returns `None` if no `ort` pin is found.
    fn ort_version_pin(cargo_toml: &str) -> Option<String> {
        for line in cargo_toml.lines() {
            let trimmed = line.trim_start();
            // The `ort` dependency key, not a substring of some other key
            // (e.g. `ort-sys`): `ort` followed by optional whitespace then `=`.
            let Some(rest) = trimmed.strip_prefix("ort") else {
                continue;
            };
            let rest = rest.trim_start();
            let Some(rest) = rest.strip_prefix('=') else {
                continue;
            };
            let rest = rest.trim_start();
            // Inline-table form: `ort = { version = "=X", ... }`.
            if let Some(after_version) = rest.find("version") {
                let after = &rest[after_version + "version".len()..];
                if let Some(pin) = first_quoted(after) {
                    return Some(pin);
                }
            }
            // Shorthand form: `ort = "=X"`.
            if let Some(pin) = first_quoted(rest) {
                return Some(pin);
            }
        }
        None
    }

    /// The first double-quoted string in `s`, if any.
    fn first_quoted(s: &str) -> Option<String> {
        let start = s.find('"')? + 1;
        let end = s[start..].find('"')? + start;
        Some(s[start..end].to_string())
    }

    #[test]
    fn first_quoted_extracts_version_string() {
        assert_eq!(
            first_quoted(r#" = "=2.0.0-rc.12", optional = true }"#).as_deref(),
            Some("=2.0.0-rc.12")
        );
        assert_eq!(first_quoted("no quotes here"), None);
    }

    #[test]
    fn ort_version_pin_parses_inline_and_shorthand_forms() {
        let inline = r#"ort = { version = "=2.0.0-rc.12", optional = true }"#;
        assert_eq!(ort_version_pin(inline).as_deref(), Some("=2.0.0-rc.12"));

        let shorthand = r#"ort = "=2.0.0-rc.12""#;
        assert_eq!(ort_version_pin(shorthand).as_deref(), Some("=2.0.0-rc.12"));

        // A lookalike key (`ort-sys`) must not be mistaken for the `ort` dependency.
        let lookalike = r#"ort-sys = { version = "=9.9.9" }"#;
        assert_eq!(ort_version_pin(lookalike), None);
    }

    /// Mechanical lockstep guard (cross-cutting Low finding on PR #126): the `ort`
    /// pin in `semantic-search` (used by fastembed) and in `audio-transcription`
    /// (used by Parakeet) MUST stay string-equal so the workspace links exactly one
    /// native ONNX runtime. Today they match and `cargo check` is green, but nothing
    /// previously asserted the two pins stay equal — only matching human comments. A
    /// future bump to one and not the other would diverge silently. This test parses
    /// both Cargo.toml files (located relative to this crate's manifest dir, so it
    /// runs in CI under plain `cargo test -p semantic-search` with no feature gate)
    /// and fails loudly naming both pins if they ever drift.
    #[test]
    fn ort_pin_is_in_lockstep_with_audio_transcription() {
        let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let self_cargo_toml_path = crate_dir.join("Cargo.toml");
        // `crates/semantic-search` -> `crates/audio-transcription`.
        let audio_cargo_toml_path = crate_dir
            .parent()
            .expect("semantic-search crate dir has a parent (crates/)")
            .join("audio-transcription")
            .join("Cargo.toml");

        let self_toml = std::fs::read_to_string(&self_cargo_toml_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", self_cargo_toml_path.display()));
        let audio_toml = std::fs::read_to_string(&audio_cargo_toml_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", audio_cargo_toml_path.display()));

        let self_pin = ort_version_pin(&self_toml).unwrap_or_else(|| {
            panic!(
                "could not find an `ort` version pin in {}",
                self_cargo_toml_path.display()
            )
        });
        let audio_pin = ort_version_pin(&audio_toml).unwrap_or_else(|| {
            panic!(
                "could not find an `ort` version pin in {}",
                audio_cargo_toml_path.display()
            )
        });

        assert_eq!(
            self_pin, audio_pin,
            "`ort` pin drift: semantic-search pins `ort = \"{self_pin}\"` ({}) but \
             audio-transcription pins `ort = \"{audio_pin}\"` ({}). fastembed and \
             Parakeet must share ONE `ort` so the workspace links a single native \
             ONNX runtime — bump both pins together (see ADR 0036).",
            self_cargo_toml_path.display(),
            audio_cargo_toml_path.display(),
        );
    }
}
