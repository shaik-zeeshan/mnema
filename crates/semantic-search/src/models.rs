//! Semantic Search Model catalog, on-disk layout, and model-gating detector.
//!
//! The catalog is **hand-maintained** (ADR 0037): candle has no model registry,
//! so each model is a hand-coded [`SemanticSearchModelDescriptor`]
//! (`{ architecture, dimension, pooling, max_tokens, hf_repo, expected_layout }`).
//! This reverses commit `524975e`'s "synthesize from fastembed, never hand-restate"
//! overlay — there is no fastembed catalog left to overlay. A `config.json`
//! cross-check (`tests::descriptor_dimension_matches_config_json`) is the drift
//! guard that replaces the old `ort` pin-lockstep and fastembed-synthesis guards.
//!
//! The detector mirrors the audio-transcription model detector: a model is
//! Installed when an `.installed.json` marker for that exact provider/model id
//! sits in `semantic_search_models/{provider}/{model_id}/` alongside every
//! required file (the safetensors weights + `config.json` + `tokenizer.json`).
//! Anything else is Missing — and a Missing model makes **Semantic Search** a
//! silent no-op, never an error.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// App-data subdirectory that holds installed Semantic Search Models, laid out
/// as `semantic_search_models/{provider}/{model_id}/` (ADR 0036).
pub const MODEL_STORE_DIR_NAME: &str = "semantic_search_models";

/// Marker file written into a model directory once every required file is
/// present, mirroring the transcription installer's `.installed.json`.
pub const INSTALLED_MARKER_FILE_NAME: &str = ".installed.json";

/// The catalog namespace / on-disk provider segment: the `{provider}/{model_id}`
/// namespace under which all locally-run candle models install.
///
/// `"local"` is backend-neutral on purpose — candle is just today's backend (ADR
/// 0037 made it pluggable), so the namespace must not name a runtime (the old
/// `"fastembed"` value did, and was wrong the moment fastembed was dropped). The
/// persisted user setting `semantic_search.provider` defaults to this value, and a
/// capture-types serde test pins it.
pub const SEMANTIC_SEARCH_PROVIDER_ID: &str = "local";

/// The files a candle model loads from disk. A Semantic Search Model is only
/// Installed when all three exist alongside the marker. (The retired ONNX layout's
/// `model.onnx`, external-data siblings, `tokenizer_config.json`, and
/// `special_tokens_map.json` are gone — candle needs only these three.)
pub const MODEL_SAFETENSORS_FILE_NAME: &str = "model.safetensors";
pub const TOKENIZER_FILE_NAME: &str = "tokenizer.json";
pub const CONFIG_FILE_NAME: &str = "config.json";

/// Bumped 1 → 2 for the candle cutover: an older ONNX-shaped `.installed.json`
/// marker (manifest_version 1) no longer matches, so a model installed in the
/// retired ONNX layout is reported Missing and re-downloads in the safetensors
/// layout (ADR 0037).
pub(crate) const MANIFEST_VERSION: u32 = 2;

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

/// The candle architecture a **Semantic Search Model** runs through. Dispatched in
/// the candle backend (`backend/candle.rs`) to the matching
/// `candle_transformers::models::*` module: `NomicBert` for the English default
/// (`nomic-embed-text-v1.5`), `XlmRoberta` for the multilingual-e5 family and
/// `bge-m3`, and `StellaEnV5` for `stella_en_400M_v5` (dispatches to
/// `candle_transformers::models::stella_en_v5`, whose dense head pools internally).
/// Hand-coded per model — never inferred from an id.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SemanticSearchArchitecture {
    NomicBert,
    XlmRoberta,
    StellaEnV5,
}

/// The sentence-pooling strategy a **Semantic Search Model** reads its single
/// vector with: `Mean`-pool the token hidden states over the attention mask
/// (nomic / e5) or take the `[CLS]` token hidden state (bge-m3). Getting this
/// wrong silently mean-pools a CLS-trained model — a wrong, lower-quality vector —
/// so pooling is a **declared descriptor field**, hand-coded per model, never
/// guessed from the id.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SemanticSearchPooling {
    Mean,
    Cls,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SemanticSearchModelManifest {
    pub version: u32,
    pub models: Vec<SemanticSearchModelDescriptor>,
}

/// A hand-coded **Semantic Search Model** descriptor (ADR 0037). Every fact is
/// stated explicitly here — there is no catalog to synthesize from — and guarded
/// against the model's own `config.json` by the drift test.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SemanticSearchModelDescriptor {
    pub provider: String,
    pub model_id: String,
    pub display_name: String,
    pub description: String,
    pub tier: SemanticSearchModelTier,
    /// The candle architecture this model runs through (dispatched in the backend).
    pub architecture: SemanticSearchArchitecture,
    /// The HuggingFace repo id (e.g. `nomic-ai/nomic-embed-text-v1.5`) the Settings
    /// slice downloads the model from.
    pub hf_repo: String,
    /// The immutable HuggingFace commit SHA the model is pinned to. Downloads
    /// resolve `…/resolve/{hf_revision}/{path}` instead of the mutable `main`
    /// branch — this kills the force-push / mutable-ref surface (an upstream
    /// rewrite of `main` can no longer swap the bytes under us) and makes every
    /// install reproducible (the same revision always yields the same files).
    pub hf_revision: String,
    pub license_label: Option<String>,
    /// Vector dimension this model produces. The default English tier is 768 to
    /// match the `search_document_vectors vec0(embedding float[768])` table.
    pub dimension: usize,
    /// The model's token window. Text overflowing this is auto-split on overflow
    /// (never silently truncated) before embedding.
    pub max_tokens: usize,
    /// Approximate on-disk footprint of the downloaded safetensors model, in bytes.
    /// Surfaced in Settings as the disk-cost disclosure (ADR 0036).
    pub approx_download_bytes: u64,
    /// The sentence-pooling strategy the backend pools this model with — Mean for
    /// nomic / e5, Cls for bge-m3. Hand-coded, never guessed from the id.
    pub pooling: SemanticSearchPooling,
    /// The instruction prefix prepended to a **query** before embedding (e.g.
    /// nomic's `search_query: `, e5's `query: `). `None` when the model takes a
    /// bare query with no instruction (bge-m3's dense path). Hand-coded per model.
    #[serde(default)]
    pub query_prompt: Option<String>,
    /// The instruction prefix prepended to a **document/passage** before embedding
    /// (e.g. nomic's `search_document: `, e5's `passage: `). `None` when the model
    /// embeds the bare text. Hand-coded per model.
    #[serde(default)]
    pub document_prompt: Option<String>,
    /// When set, the stored vector width when **Matryoshka-truncating** the model's
    /// native vector: the backend produces its native dimension, the embedder
    /// truncates each vector to this many leading elements and renormalizes, and the
    /// truncated width (equal to `dimension`) is what is stored. `None` when the
    /// model is stored at its native width (no truncation).
    #[serde(default)]
    pub mrl_truncate_dim: Option<usize>,
    pub expected_layout: InstalledModelLayout,
}

/// The on-disk layout of an installed (safetensors) **Semantic Search Model**.
///
/// Every path is **repo-relative** (the same path used as the HuggingFace
/// `resolve/main/<path>` download path AND as the on-disk path under the model's
/// install dir). For the current catalog every weights file sits at the repo root
/// (`model.safetensors`), but the field is repo-relative so a model whose weights
/// live in a subdirectory still round-trips.
///
/// A model is **Installed** only when every entry in `required_files` (the
/// safetensors weights, `config.json`, `tokenizer.json`) is present alongside the
/// marker.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModelLayout {
    pub marker_file_name: String,
    /// Every required file, repo-relative (safetensors + config + tokenizer).
    pub required_files: Vec<String>,
    /// The repo-relative path of the safetensors weights (e.g. `model.safetensors`).
    /// The candle backend mmaps this from disk.
    pub weights_relative_path: String,
    /// The repo-relative path of an **auxiliary head** weights file, when the model
    /// loads a second safetensors alongside the base backbone (e.g. Stella's dense
    /// projection head `2_Dense_2048/model.safetensors`). `None` for every model
    /// whose single backbone safetensors is the whole model.
    #[serde(default)]
    pub aux_weights_relative_path: Option<String>,
}

impl InstalledModelLayout {
    /// Build a layout from the safetensors weights path plus the two json files.
    /// `required_files` is the union: weights, `config.json`, `tokenizer.json`.
    /// No auxiliary head (single-backbone model).
    pub fn from_weights_path(weights_relative_path: impl Into<String>) -> Self {
        let weights_relative_path = weights_relative_path.into();
        let required_files = vec![
            weights_relative_path.clone(),
            CONFIG_FILE_NAME.to_string(),
            TOKENIZER_FILE_NAME.to_string(),
        ];
        Self {
            marker_file_name: INSTALLED_MARKER_FILE_NAME.to_string(),
            required_files,
            weights_relative_path,
            aux_weights_relative_path: None,
        }
    }

    /// Build a layout for a model that loads a **base backbone plus a separate head**
    /// safetensors (e.g. Stella: `model.safetensors` + `2_Dense_2048/model.safetensors`).
    /// `required_files` is the union: base, head, `config.json`, `tokenizer.json`;
    /// `weights_relative_path` is the base and `aux_weights_relative_path` the head.
    pub fn from_weights_and_head(
        base_relative_path: impl Into<String>,
        head_relative_path: impl Into<String>,
    ) -> Self {
        let base_relative_path = base_relative_path.into();
        let head_relative_path = head_relative_path.into();
        let required_files = vec![
            base_relative_path.clone(),
            head_relative_path.clone(),
            CONFIG_FILE_NAME.to_string(),
            TOKENIZER_FILE_NAME.to_string(),
        ];
        Self {
            marker_file_name: INSTALLED_MARKER_FILE_NAME.to_string(),
            required_files,
            weights_relative_path: base_relative_path,
            aux_weights_relative_path: Some(head_relative_path),
        }
    }
}

impl Default for InstalledModelLayout {
    /// The common layout: `model.safetensors` at the repo root, plus `config.json`
    /// and `tokenizer.json`.
    fn default() -> Self {
        Self::from_weights_path(MODEL_SAFETENSORS_FILE_NAME)
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

/// A curated **Semantic Search Model** the **Custom** picker can offer, distilled
/// from a catalog descriptor to just the fields the Settings UI needs. The open
/// "any ONNX model" picker is gone (ADR 0037): "I want a different model" is served
/// by a future backend (e.g. local Ollama), not an arbitrary-architecture loader,
/// so this list is exactly the curated catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupportedEmbeddingModel {
    pub model_id: String,
    pub display_name: String,
    pub hf_repo: String,
    pub dimension: usize,
    pub description: String,
    /// Whether the model is multilingual (e5 / bge-m3).
    pub multilingual: bool,
    /// The model's hand-coded pooling strategy.
    pub pooling: SemanticSearchPooling,
}

/// The hand-coded catalog: the curated tiers (ADR 0037).
///
/// - **English (default):** `nomic-embed-text-v1.5` — NomicBert, 768-dim, Mean,
///   8192-token, Apache-2.0, ~250 MB. `model.safetensors` at the repo root.
/// - **Multilingual:** `multilingual-e5-small` — XLM-Roberta, 384-dim, Mean,
///   512-token, MIT, ~470 MB.
/// - **Custom multilingual option:** `bge-m3` — XLM-Roberta, 1024-dim, CLS,
///   8192-token, MIT, ~2.27 GB.
/// - **Custom English option:** `stella_en_400M_v5` — StellaEnV5, 2048-dim
///   (native `2_Dense_2048` head, module-internal mean+dense pool), 8192-token,
///   MIT, ~1.75 GB. Base `model.safetensors` plus the `2_Dense_2048/model.safetensors`
///   head.
/// - **Custom multilingual option:** `snowflake-arctic-embed-l-v2.0` — XLM-Roberta,
///   256-dim stored (Matryoshka-truncated from native 1024), CLS, 8192-token,
///   Apache-2.0, ~2.3 GB.
fn catalog() -> Vec<SemanticSearchModelDescriptor> {
    vec![
        SemanticSearchModelDescriptor {
            provider: SEMANTIC_SEARCH_PROVIDER_ID.to_string(),
            model_id: "nomic-embed-text-v1.5".to_string(),
            display_name: "Nomic Embed Text v1.5 (English)".to_string(),
            description: "Default English tier: long-context (8192 tokens), \
                Apache-2.0, 768-dimensional. Long context makes truncation a \
                non-issue and the permissive license keeps the default path \
                obligation-free."
                .to_string(),
            tier: SemanticSearchModelTier::English,
            architecture: SemanticSearchArchitecture::NomicBert,
            hf_repo: "nomic-ai/nomic-embed-text-v1.5".to_string(),
            hf_revision: "e9b6763023c676ca8431644204f50c2b100d9aab".to_string(),
            license_label: Some("Apache-2.0".to_string()),
            dimension: 768,
            max_tokens: 8192,
            // ~250 MB safetensors (F32 weights).
            approx_download_bytes: 250_000_000,
            pooling: SemanticSearchPooling::Mean,
            // nomic's asymmetric retrieval prefixes (trailing space is significant).
            query_prompt: Some("search_query: ".to_string()),
            document_prompt: Some("search_document: ".to_string()),
            mrl_truncate_dim: None,
            expected_layout: InstalledModelLayout::default(),
        },
        SemanticSearchModelDescriptor {
            provider: SEMANTIC_SEARCH_PROVIDER_ID.to_string(),
            model_id: "multilingual-e5-small".to_string(),
            display_name: "Multilingual E5 Small (Multilingual)".to_string(),
            description: "Multilingual tier: covers 100+ languages, non-gated \
                (MIT), 384-dimensional. A non-English user is guided here rather \
                than silently degraded by the English default, and it serves \
                English well too."
                .to_string(),
            tier: SemanticSearchModelTier::Multilingual,
            architecture: SemanticSearchArchitecture::XlmRoberta,
            hf_repo: "intfloat/multilingual-e5-small".to_string(),
            hf_revision: "614241f622f53c4eeff9890bdc4f31cfecc418b3".to_string(),
            license_label: Some("MIT".to_string()),
            dimension: 384,
            max_tokens: 512,
            // ~470 MB on disk.
            approx_download_bytes: 470_000_000,
            pooling: SemanticSearchPooling::Mean,
            // e5's asymmetric retrieval prefixes (trailing space is significant).
            query_prompt: Some("query: ".to_string()),
            document_prompt: Some("passage: ".to_string()),
            mrl_truncate_dim: None,
            expected_layout: InstalledModelLayout::default(),
        },
        SemanticSearchModelDescriptor {
            provider: SEMANTIC_SEARCH_PROVIDER_ID.to_string(),
            model_id: "bge-m3".to_string(),
            display_name: "BGE-M3 (Multilingual, Custom)".to_string(),
            description: "Custom multilingual option (BAAI/bge-m3), 1024-dimensional, \
                8192-token, CLS-pooled. Available via the Custom picker."
                .to_string(),
            tier: SemanticSearchModelTier::Custom,
            architecture: SemanticSearchArchitecture::XlmRoberta,
            hf_repo: "BAAI/bge-m3".to_string(),
            hf_revision: "5617a9f61b028005a4858fdac845db406aefb181".to_string(),
            license_label: Some("MIT".to_string()),
            dimension: 1024,
            max_tokens: 8192,
            // ~2.27 GB safetensors.
            approx_download_bytes: 2_270_000_000,
            pooling: SemanticSearchPooling::Cls,
            // bge-m3's dense path takes bare text — no instruction prefix.
            query_prompt: None,
            document_prompt: None,
            mrl_truncate_dim: None,
            // bge-m3's repo ships ONLY a PyTorch `pytorch_model.bin` (no
            // `model.safetensors`), so the weights file — and thus the download
            // path AND the on-disk loader input — is the `.bin`. The candle
            // backend branches on this extension and reads it via the pickle path
            // (`VarBuilder::from_pth`) instead of mmaping safetensors. The `.bin`
            // is an `XLMRobertaModel` state-dict whose keys already sit at the
            // VarBuilder root (same as e5), so no key remap is needed.
            expected_layout: InstalledModelLayout::from_weights_path("pytorch_model.bin"),
        },
        SemanticSearchModelDescriptor {
            provider: SEMANTIC_SEARCH_PROVIDER_ID.to_string(),
            model_id: "stella_en_400M_v5".to_string(),
            display_name: "Stella 400M v5 (English, Custom)".to_string(),
            description: "Custom English option (NovaSearch/stella_en_400M_v5), \
                2048-dimensional, MIT. Stronger English retrieval than the default \
                Nomic; larger per-vector storage (2048-dim)."
                .to_string(),
            tier: SemanticSearchModelTier::Custom,
            architecture: SemanticSearchArchitecture::StellaEnV5,
            hf_repo: "NovaSearch/stella_en_400M_v5".to_string(),
            hf_revision: "ffeb2b7ee715c226d4ffe5e4619f7dbb48624c20".to_string(),
            license_label: Some("MIT".to_string()),
            // Stored = the `2_Dense_2048` head's out_features (the backbone hidden is
            // 1024; the dense head projects up to 2048). No truncation.
            dimension: 2048,
            max_tokens: 8192,
            // ~1.75 GB safetensors (~435M F32 backbone params + the 2048 head).
            approx_download_bytes: 1_750_000_000,
            // Stella pools INSIDE the candle module (mean-pool over the mask, then the
            // dense head). The external pool step is bypassed for this architecture, so
            // this `Mean` is the closest-truth label only — it is ignored at load.
            pooling: SemanticSearchPooling::Mean,
            // Stella's asymmetric retrieval instruction for queries (the `\n` is a real
            // newline in the prompt string). Documents are embedded bare.
            query_prompt: Some(
                "Instruct: Given a web search query, retrieve relevant passages that \
                 answer the query.\nQuery: "
                    .to_string(),
            ),
            document_prompt: None,
            mrl_truncate_dim: None,
            // Base backbone `model.safetensors` at the repo root plus the dense
            // projection head `2_Dense_2048/model.safetensors` loaded alongside it.
            expected_layout: InstalledModelLayout::from_weights_and_head(
                "model.safetensors",
                "2_Dense_2048/model.safetensors",
            ),
        },
        SemanticSearchModelDescriptor {
            provider: SEMANTIC_SEARCH_PROVIDER_ID.to_string(),
            model_id: "snowflake-arctic-embed-l-v2.0".to_string(),
            display_name: "Snowflake Arctic Embed L v2.0 (Multilingual, Custom)"
                .to_string(),
            description: "Custom multilingual option \
                (Snowflake/snowflake-arctic-embed-l-v2.0), Matryoshka-truncated to \
                256-dimensional, Apache-2.0, CLS-pooled. bge-m3-class multilingual \
                retrieval at a quarter of the per-vector storage."
                .to_string(),
            tier: SemanticSearchModelTier::Custom,
            architecture: SemanticSearchArchitecture::XlmRoberta,
            hf_repo: "Snowflake/snowflake-arctic-embed-l-v2.0".to_string(),
            hf_revision: "ac6544c8a46e00af67e330e85a9028c66b8cfd9a".to_string(),
            license_label: Some("Apache-2.0".to_string()),
            // Stored width is the Matryoshka-truncated 256 (see `mrl_truncate_dim`),
            // NOT the model's native 1024.
            dimension: 256,
            max_tokens: 8192,
            // ~2.3 GB safetensors (~568M F32 params, XLM-Roberta large).
            approx_download_bytes: 2_300_000_000,
            pooling: SemanticSearchPooling::Cls,
            // Arctic's asymmetric retrieval prefix for queries (trailing space is
            // significant); documents are embedded bare.
            query_prompt: Some("query: ".to_string()),
            document_prompt: None,
            // Matryoshka: the backend produces the native 1024-dim vector; the embedder
            // truncates each vector to the first 256 elements and renormalizes (in
            // `runtime.rs`, above the backend trait) before storage.
            mrl_truncate_dim: Some(256),
            expected_layout: InstalledModelLayout::default(),
        },
    ]
}

/// The hand-maintained guided **Semantic Search Model** manifest (ADR 0037): the
/// three curated tiers, stated explicitly (no fastembed synthesis, no panic).
pub fn builtin_model_manifest() -> SemanticSearchModelManifest {
    SemanticSearchModelManifest {
        version: MANIFEST_VERSION,
        models: catalog(),
    }
}

/// The curated supported-models list for the **Custom** picker: the same three
/// catalog descriptors distilled to the picker fields.
pub fn list_supported_models() -> Vec<SupportedEmbeddingModel> {
    catalog()
        .into_iter()
        .map(|descriptor| SupportedEmbeddingModel {
            multilingual: matches!(
                descriptor.tier,
                SemanticSearchModelTier::Multilingual
            ) || descriptor.architecture == SemanticSearchArchitecture::XlmRoberta,
            model_id: descriptor.model_id,
            display_name: descriptor.display_name,
            hf_repo: descriptor.hf_repo,
            dimension: descriptor.dimension,
            description: descriptor.description,
            pooling: descriptor.pooling,
        })
        .collect()
}

/// Resolve a **Semantic Search Model** descriptor for a `{provider}/{model_id}`
/// selection. Manifest lookup only — there is NO synthesis (ADR 0037): an unknown
/// id returns `None`. The provider must equal the catalog namespace
/// ([`SEMANTIC_SEARCH_PROVIDER_ID`]).
pub fn resolve_descriptor(
    provider: &str,
    model_id: &str,
) -> Option<SemanticSearchModelDescriptor> {
    let manifest = builtin_model_manifest();
    find_model_descriptor(&manifest, provider, model_id).cloned()
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

/// Model-gating: is the user's selected **Semantic Search Model** installed?
///
/// Returns `false` (a silent no-op admission, never an error) when the feature
/// is disabled, no model is selected, the selection is not a resolvable model, or
/// the model is not yet installed. The only `Err` path is a corrupt marker file.
///
/// Resolves the selection through [`resolve_descriptor`] (manifest lookup), then
/// reuses the pure [`detect_model_status`] detector.
pub fn selected_semantic_search_model_available(
    app_data_dir: impl AsRef<Path>,
    settings: &capture_types::SemanticSearchSettings,
) -> Result<bool, ModelStatusError> {
    if !settings.enabled {
        return Ok(false);
    }
    let Some(model_id) = settings.model_id.as_deref() else {
        return Ok(false);
    };
    let Some(descriptor) = resolve_descriptor(&settings.provider, model_id) else {
        return Ok(false);
    };
    let status = detect_model_status(semantic_search_models_dir(app_data_dir), &descriptor)?;
    Ok(status.is_available())
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

/// Write the `.installed.json` marker into a freshly-downloaded model directory,
/// mirroring the transcription installer. The detector only treats a model as
/// Installed once this marker (matching the exact provider/model id and the
/// current `MANIFEST_VERSION`) sits alongside every required file, so the
/// downloader writes it last.
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
    use std::fs;

    /// A self-contained nomic-shaped descriptor (safetensors at the root, no
    /// external data) built inline so the **pure detector** is exercised without
    /// the catalog.
    fn nomic_test_descriptor() -> SemanticSearchModelDescriptor {
        SemanticSearchModelDescriptor {
            provider: SEMANTIC_SEARCH_PROVIDER_ID.to_string(),
            model_id: "nomic-embed-text-v1.5".to_string(),
            display_name: "Nomic Embed Text v1.5 (English)".to_string(),
            description: "Default English tier".to_string(),
            tier: SemanticSearchModelTier::English,
            architecture: SemanticSearchArchitecture::NomicBert,
            hf_repo: "nomic-ai/nomic-embed-text-v1.5".to_string(),
            hf_revision: "e9b6763023c676ca8431644204f50c2b100d9aab".to_string(),
            license_label: Some("Apache-2.0".to_string()),
            dimension: 768,
            max_tokens: 8192,
            approx_download_bytes: 250_000_000,
            pooling: SemanticSearchPooling::Mean,
            query_prompt: Some("search_query: ".to_string()),
            document_prompt: Some("search_document: ".to_string()),
            mrl_truncate_dim: None,
            expected_layout: InstalledModelLayout::default(),
        }
    }

    /// Write a (possibly nested) required file, creating its parent directory so
    /// the repo-relative layout is reproduced on disk.
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
        write_installed_marker(models_dir, &descriptor.provider, &descriptor.model_id)
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
    fn find_model_descriptor_matches_on_provider_and_id() {
        let manifest = builtin_model_manifest();
        let found = find_model_descriptor(&manifest, SEMANTIC_SEARCH_PROVIDER_ID, "bge-m3")
            .expect("bge-m3 present");
        assert_eq!(found.model_id, "bge-m3");
        assert!(find_model_descriptor(&manifest, SEMANTIC_SEARCH_PROVIDER_ID, "nope").is_none());
        assert!(find_model_descriptor(&manifest, "other-provider", "bge-m3").is_none());
    }

    #[test]
    fn missing_model_is_not_available_and_lists_missing_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        let descriptor = nomic_test_descriptor();
        let descriptor = &descriptor;
        let status = detect_model_status(semantic_search_models_dir(temp.path()), descriptor)
            .expect("status");
        assert_eq!(status.status, ModelStatusKind::Missing);
        assert!(!status.is_available());
        // The safetensors weights are required at their repo-relative path.
        assert!(status
            .missing_files
            .contains(&descriptor.expected_layout.weights_relative_path));
        assert!(status.install_path.ends_with("local/nomic-embed-text-v1.5"));
    }

    #[test]
    fn installed_model_requires_marker_and_every_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let models_dir = semantic_search_models_dir(temp.path());
        let descriptor = nomic_test_descriptor();
        let descriptor = &descriptor;

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
        write_installed_marker(&models_dir, &descriptor.provider, &descriptor.model_id)
            .expect("marker");
        let installed = detect_model_status(&models_dir, descriptor).expect("status");
        assert_eq!(installed.status, ModelStatusKind::Installed);
        assert!(installed.is_available());
    }

    #[test]
    fn model_is_missing_until_every_required_file_is_present() {
        // A model whose weights file is absent must NOT be Installed, even with the
        // two json files and the marker present.
        let temp = tempfile::tempdir().expect("tempdir");
        let models_dir = semantic_search_models_dir(temp.path());
        let descriptor = nomic_test_descriptor();
        let descriptor = &descriptor;
        let install_dir =
            model_install_dir(&models_dir, &descriptor.provider, &descriptor.model_id).expect("dir");
        fs::create_dir_all(&install_dir).expect("install dir");

        // Write every required file EXCEPT the weights, plus the marker.
        let weights = &descriptor.expected_layout.weights_relative_path;
        for file_name in &descriptor.expected_layout.required_files {
            if file_name == weights {
                continue;
            }
            write_required_file(&install_dir, file_name);
        }
        write_installed_marker(&models_dir, &descriptor.provider, &descriptor.model_id)
            .expect("marker");

        let status = detect_model_status(&models_dir, descriptor).expect("status");
        assert_eq!(
            status.status,
            ModelStatusKind::Missing,
            "weights missing => Missing, never a broken Installed"
        );
        assert!(status.missing_files.contains(weights));

        // Now add the weights => Installed.
        write_required_file(&install_dir, weights);
        assert_eq!(
            detect_model_status(&models_dir, descriptor).expect("status").status,
            ModelStatusKind::Installed
        );
    }

    #[test]
    fn marker_for_another_model_does_not_count() {
        let temp = tempfile::tempdir().expect("tempdir");
        let models_dir = semantic_search_models_dir(temp.path());
        let descriptor = nomic_test_descriptor();
        let descriptor = &descriptor;
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
    fn stale_onnx_era_marker_invalidates_after_manifest_bump() {
        // An older ONNX-shaped install wrote manifest_version 1; after the candle
        // cutover (MANIFEST_VERSION 2) that marker must no longer match, so the
        // model is reported Missing and re-downloads in the safetensors layout.
        let temp = tempfile::tempdir().expect("tempdir");
        let models_dir = semantic_search_models_dir(temp.path());
        let descriptor = nomic_test_descriptor();
        let descriptor = &descriptor;
        let install_dir =
            model_install_dir(&models_dir, &descriptor.provider, &descriptor.model_id).expect("dir");
        fs::create_dir_all(&install_dir).expect("install dir");
        for file_name in &descriptor.expected_layout.required_files {
            write_required_file(&install_dir, file_name);
        }
        let stale = InstalledModelMarker {
            manifest_version: 1,
            provider: descriptor.provider.clone(),
            model_id: descriptor.model_id.clone(),
        };
        fs::write(
            install_dir.join(INSTALLED_MARKER_FILE_NAME),
            serde_json::to_vec(&stale).expect("marker json"),
        )
        .expect("marker");
        assert_eq!(
            detect_model_status(&models_dir, descriptor).expect("status").status,
            ModelStatusKind::Missing,
            "a manifest_version 1 (ONNX-era) marker must invalidate after the bump to 2"
        );
    }

    #[test]
    fn install_model_then_detect_reports_installed() {
        let temp = tempfile::tempdir().expect("tempdir");
        let models_dir = semantic_search_models_dir(temp.path());
        let descriptor = nomic_test_descriptor();

        assert_eq!(
            detect_model_status(&models_dir, &descriptor).expect("status").status,
            ModelStatusKind::Missing
        );
        install_model(&models_dir, &descriptor);
        assert!(detect_model_status(&models_dir, &descriptor)
            .expect("status")
            .is_available());
    }

    #[test]
    fn rejects_path_traversal_components() {
        let error =
            model_install_dir("/tmp/models", SEMANTIC_SEARCH_PROVIDER_ID, "../escape").expect_err("unsafe");
        assert!(matches!(
            error,
            ModelStatusError::UnsafePathComponent { field: "model_id", .. }
        ));
    }

    // ---- catalog + model-gating wrapper (now dependency-free) ----

    #[test]
    fn resolve_descriptor_returns_manifest_tier() {
        let descriptor =
            resolve_descriptor(SEMANTIC_SEARCH_PROVIDER_ID, "nomic-embed-text-v1.5").expect("nomic");
        assert_eq!(descriptor.tier, SemanticSearchModelTier::English);
        assert_eq!(descriptor.architecture, SemanticSearchArchitecture::NomicBert);
        assert_eq!(descriptor.max_tokens, 8192);
        assert_eq!(descriptor.license_label.as_deref(), Some("Apache-2.0"));
    }

    #[test]
    fn resolve_descriptor_rejects_unknown_and_non_namespace_provider() {
        // No synthesis: an unknown id returns None.
        assert!(resolve_descriptor(SEMANTIC_SEARCH_PROVIDER_ID, "not-a-real-model").is_none());
        // A provider other than the catalog namespace never resolves.
        assert!(resolve_descriptor("some-other-provider", "nomic-embed-text-v1.5").is_none());
    }

    #[test]
    fn default_english_tier_is_nomic_768_dim() {
        let manifest = builtin_model_manifest();
        let default =
            find_model_descriptor(&manifest, SEMANTIC_SEARCH_PROVIDER_ID, "nomic-embed-text-v1.5")
                .expect("english tier");
        assert_eq!(default.tier, SemanticSearchModelTier::English);
        assert_eq!(default.dimension, 768);
        assert_eq!(default.max_tokens, 8192);
        assert_eq!(default.license_label.as_deref(), Some("Apache-2.0"));
    }

    #[test]
    fn pooling_is_a_declared_field_hand_coded_per_model() {
        // Pooling is hand-coded per model, NEVER inferred from an id prefix (the
        // historical silent-drift bug). nomic / e5 = Mean; bge-m3 / Arctic = CLS.
        // Stella's `Mean` is the closest-truth label only (it pools internally via
        // its dense head and the external pool is bypassed at load), but the
        // descriptor field still carries `Mean`, so it is checked here too.
        let mean = ["nomic-embed-text-v1.5", "multilingual-e5-small", "stella_en_400M_v5"];
        let cls = ["bge-m3", "snowflake-arctic-embed-l-v2.0"];
        for id in mean {
            let descriptor =
                resolve_descriptor(SEMANTIC_SEARCH_PROVIDER_ID, id).unwrap_or_else(|| panic!("{id}"));
            assert_eq!(
                descriptor.pooling,
                SemanticSearchPooling::Mean,
                "{id} must be Mean-pooled"
            );
        }
        for id in cls {
            let descriptor =
                resolve_descriptor(SEMANTIC_SEARCH_PROVIDER_ID, id).unwrap_or_else(|| panic!("{id}"));
            assert_eq!(
                descriptor.pooling,
                SemanticSearchPooling::Cls,
                "{id} must be CLS-pooled"
            );
        }
        // The picker rows carry the same pooling as the resolved descriptors.
        let supported = list_supported_models();
        for model in &supported {
            let descriptor = resolve_descriptor(SEMANTIC_SEARCH_PROVIDER_ID, &model.model_id)
                .unwrap_or_else(|| panic!("{}", model.model_id));
            assert_eq!(
                model.pooling, descriptor.pooling,
                "picker row {} pooling must match the descriptor",
                model.model_id
            );
        }
    }

    #[test]
    fn supported_models_lists_the_curated_catalog() {
        let supported = list_supported_models();
        // The two default tiers (nomic / e5) plus the three Custom options
        // (bge-m3, Stella, Arctic).
        assert_eq!(supported.len(), 5, "exactly the five curated models");
        let ids: Vec<&str> = supported.iter().map(|m| m.model_id.as_str()).collect();
        assert!(ids.contains(&"nomic-embed-text-v1.5"));
        assert!(ids.contains(&"multilingual-e5-small"));
        assert!(ids.contains(&"bge-m3"));
        assert!(ids.contains(&"stella_en_400M_v5"));
        assert!(ids.contains(&"snowflake-arctic-embed-l-v2.0"));
        // The English default is not flagged multilingual; the e5/bge tiers are.
        let nomic = supported.iter().find(|m| m.model_id == "nomic-embed-text-v1.5").unwrap();
        assert!(!nomic.multilingual);
        let e5 = supported.iter().find(|m| m.model_id == "multilingual-e5-small").unwrap();
        assert!(e5.multilingual);
        // The multilingual heuristic (tier == Multilingual || architecture ==
        // XlmRoberta) cleaves the two new Custom options correctly: Stella is a
        // StellaEnV5 English model, so it is NOT flagged multilingual; Arctic is an
        // XlmRoberta model, so it IS — even though both share the Custom tier.
        let stella = supported.iter().find(|m| m.model_id == "stella_en_400M_v5").unwrap();
        assert!(!stella.multilingual, "Stella (English, StellaEnV5) is not multilingual");
        let arctic = supported
            .iter()
            .find(|m| m.model_id == "snowflake-arctic-embed-l-v2.0")
            .unwrap();
        assert!(arctic.multilingual, "Arctic (XlmRoberta) is multilingual");
    }

    /// How a descriptor's stored `dimension` relates to the model's *backbone*
    /// hidden size (the `config.json` hidden width). The drift test asserts a
    /// different equality per relation, because for two of the five catalog models
    /// the stored width is deliberately NOT the backbone hidden size:
    ///
    /// - [`DimSource::BackboneHidden`] — the stored vector is the backbone's own
    ///   hidden state (nomic / e5 / bge). `dimension == config hidden`.
    /// - [`DimSource::MrlTruncate`] — the stored vector is a Matryoshka prefix of
    ///   the backbone hidden state, renormalized (Arctic stores 256 of a native
    ///   1024). `mrl_truncate_dim == Some(dimension)` and `dimension <= config
    ///   hidden` (the truncated width can never exceed the native width).
    /// - [`DimSource::DeclaredHead`] — the stored vector is a *dense projection
    ///   head* output, independent of the backbone hidden size (Stella's
    ///   `2_Dense_2048` head projects the 1024 backbone up to 2048). `dimension ==
    ///   head_dim`, `mrl_truncate_dim == None`, and the config hidden is the
    ///   *independent* backbone width (asserted only via `backbone_hidden` below,
    ///   never against `dimension`).
    enum DimSource {
        BackboneHidden,
        MrlTruncate,
        DeclaredHead { head_dim: usize },
    }

    /// Drift guard (replaces the retired `ort` pin-lockstep + fastembed-synthesis
    /// guards): each descriptor's hand-coded `dimension`, `architecture`, and
    /// `max_tokens` must agree with the model's own `config.json`, and the config's
    /// layer count must be sane. Real upstream config fixtures are committed under
    /// `tests/fixtures/{model_id}/config.json`. A hand-coded fact that drifts from
    /// the real config fails HERE. (Pooling is not in `config.json`, so it is guarded
    /// separately by `pooling_is_a_declared_field_hand_coded_per_model`.)
    #[test]
    fn descriptor_dimension_matches_config_json() {
        use serde_json::Value;

        /// The per-model reference facts cross-checked against `config.json`. The
        /// HuggingFace `architectures[0]` class name the descriptor's
        /// [`SemanticSearchArchitecture`] must map to, and the config field the
        /// hand-coded `max_tokens` must equal (nomic names it `n_positions`,
        /// XLM-Roberta `max_position_embeddings`). e5/bge `max_position_embeddings`
        /// carries the +2 offset for the two special tokens, so the descriptor's
        /// usable window is the config value minus that offset.
        ///
        /// `backbone_hidden` is the config's hidden width (always asserted, for
        /// every model). `dim_source` says how the *stored* `dimension` relates to
        /// that backbone — the stored width is the backbone hidden state for three
        /// models, a Matryoshka prefix of it for Arctic, and a dense-head output
        /// independent of it for Stella (see [`DimSource`]).
        struct ConfigReference {
            model_id: &'static str,
            architecture: SemanticSearchArchitecture,
            architectures_class: &'static str,
            max_tokens_field: &'static str,
            max_position_offset: u64,
            backbone_hidden: usize,
            dim_source: DimSource,
        }
        let references = [
            ConfigReference {
                model_id: "nomic-embed-text-v1.5",
                architecture: SemanticSearchArchitecture::NomicBert,
                architectures_class: "NomicBertModel",
                max_tokens_field: "n_positions",
                max_position_offset: 0,
                backbone_hidden: 768,
                dim_source: DimSource::BackboneHidden,
            },
            ConfigReference {
                model_id: "multilingual-e5-small",
                architecture: SemanticSearchArchitecture::XlmRoberta,
                architectures_class: "XLMRobertaModel",
                max_tokens_field: "max_position_embeddings",
                max_position_offset: 2,
                backbone_hidden: 384,
                dim_source: DimSource::BackboneHidden,
            },
            ConfigReference {
                model_id: "bge-m3",
                architecture: SemanticSearchArchitecture::XlmRoberta,
                architectures_class: "XLMRobertaModel",
                max_tokens_field: "max_position_embeddings",
                max_position_offset: 2,
                backbone_hidden: 1024,
                dim_source: DimSource::BackboneHidden,
            },
            ConfigReference {
                // Stella's stored width is its `2_Dense_2048` dense head output
                // (2048), NOT the 1024 backbone hidden size — so the stored
                // `dimension` is cross-checked against the declared head width and
                // the config hidden is asserted independently as the backbone.
                model_id: "stella_en_400M_v5",
                architecture: SemanticSearchArchitecture::StellaEnV5,
                architectures_class: "NewModel",
                max_tokens_field: "max_position_embeddings",
                max_position_offset: 0,
                backbone_hidden: 1024,
                dim_source: DimSource::DeclaredHead { head_dim: 2048 },
            },
            ConfigReference {
                // Arctic stores a Matryoshka-truncated 256 of its native 1024
                // backbone hidden state — so the stored `dimension` is cross-checked
                // against `mrl_truncate_dim` and bounded by (≤) the config hidden.
                model_id: "snowflake-arctic-embed-l-v2.0",
                architecture: SemanticSearchArchitecture::XlmRoberta,
                architectures_class: "XLMRobertaModel",
                max_tokens_field: "max_position_embeddings",
                max_position_offset: 2,
                backbone_hidden: 1024,
                dim_source: DimSource::MrlTruncate,
            },
        ];

        let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures");

        for descriptor in catalog() {
            let reference = references
                .iter()
                .find(|reference| reference.model_id == descriptor.model_id)
                .unwrap_or_else(|| {
                    panic!("{}: no config reference data for descriptor", descriptor.model_id)
                });
            // The hand-coded architecture must match its reference (the reference is
            // in turn cross-checked against config.json's `architectures[0]` below).
            assert_eq!(
                descriptor.architecture, reference.architecture,
                "{}: descriptor.architecture drifted from the config reference",
                descriptor.model_id
            );

            let config_path = fixtures_dir.join(&descriptor.model_id).join(CONFIG_FILE_NAME);
            let bytes = std::fs::read(&config_path).unwrap_or_else(|error| {
                panic!("read fixture config {}: {error}", config_path.display())
            });
            let config: Value = serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                panic!("parse fixture config {}: {error}", config_path.display())
            });

            // nomic config.json names the hidden size `n_embd`; XLM-Roberta and
            // Stella (`model_type: "new"`) configs name it `hidden_size`. Accept
            // either. This is the BACKBONE hidden width — it equals the stored
            // `dimension` only when `dim_source` is `BackboneHidden`; for Arctic it
            // is the native (pre-truncation) width, and for Stella it is the
            // backbone behind the dense head.
            let hidden_size = config
                .get("hidden_size")
                .or_else(|| config.get("n_embd"))
                .and_then(Value::as_u64)
                .unwrap_or_else(|| {
                    panic!("{}: config.json has no hidden_size/n_embd", descriptor.model_id)
                });
            // The reference's backbone hidden must itself agree with the real config
            // (so a wrong fixture/reference pairing fails before the dimension check).
            assert_eq!(
                hidden_size as usize, reference.backbone_hidden,
                "{}: config hidden ({}) drifted from the reference backbone_hidden ({})",
                descriptor.model_id, hidden_size, reference.backbone_hidden
            );

            // The stored `dimension` is cross-checked per its relation to the
            // backbone hidden — equal to it (backbone), a renormalized prefix of it
            // (MRL truncate), or an independent dense-head output (declared head).
            match reference.dim_source {
                DimSource::BackboneHidden => {
                    // nomic / e5 / bge: the stored vector IS the backbone hidden
                    // state, so the stored width must equal the config hidden size.
                    assert_eq!(
                        hidden_size as usize, descriptor.dimension,
                        "{}: descriptor.dimension ({}) drifted from config hidden size ({})",
                        descriptor.model_id, descriptor.dimension, hidden_size
                    );
                    // A backbone-hidden model is stored at native width — no MRL.
                    assert_eq!(
                        descriptor.mrl_truncate_dim, None,
                        "{}: a backbone-hidden model must not declare mrl_truncate_dim",
                        descriptor.model_id
                    );
                }
                DimSource::MrlTruncate => {
                    // Arctic: the backend produces the native backbone hidden state,
                    // and the embedder truncates each vector to `mrl_truncate_dim`
                    // leading elements and renormalizes. So the stored width is the
                    // declared truncate width, and it can never exceed the native
                    // (config hidden) width.
                    assert_eq!(
                        descriptor.mrl_truncate_dim, Some(descriptor.dimension),
                        "{}: an MRL-truncated model's mrl_truncate_dim must equal its stored dimension ({})",
                        descriptor.model_id, descriptor.dimension
                    );
                    assert!(
                        descriptor.dimension <= hidden_size as usize,
                        "{}: truncated dimension ({}) cannot exceed native config hidden ({})",
                        descriptor.model_id, descriptor.dimension, hidden_size
                    );
                }
                DimSource::DeclaredHead { head_dim } => {
                    // Stella: a dense projection head (`2_Dense_2048`) produces the
                    // stored vector, independent of the backbone hidden size — the
                    // candle Config is constructed (not parsed from config.json), so
                    // config hidden is asserted only as the backbone (above), never
                    // against the stored `dimension`. The stored width must equal the
                    // declared head output width, and there is NO truncation.
                    assert_eq!(
                        descriptor.dimension, head_dim,
                        "{}: descriptor.dimension ({}) drifted from the declared head width ({head_dim})",
                        descriptor.model_id, descriptor.dimension
                    );
                    assert_eq!(
                        descriptor.mrl_truncate_dim, None,
                        "{}: a declared-head model must not declare mrl_truncate_dim",
                        descriptor.model_id
                    );
                }
            }

            // The candle architecture is hand-coded, never inferred from an id — so
            // the config's `architectures[0]` class name must match the reference we
            // mapped the descriptor's `SemanticSearchArchitecture` to.
            let architectures_class = config
                .get("architectures")
                .and_then(Value::as_array)
                .and_then(|classes| classes.first())
                .and_then(Value::as_str)
                .unwrap_or_else(|| {
                    panic!("{}: config.json has no architectures[0]", descriptor.model_id)
                });
            assert_eq!(
                architectures_class, reference.architectures_class,
                "{}: config architectures[0] ({architectures_class}) disagrees with the descriptor's architecture",
                descriptor.model_id
            );

            // The hand-coded `max_tokens` window must equal the config's positional
            // limit (minus the special-token offset for the XLM-Roberta tiers). A
            // descriptor that over-states the window would silently feed candle
            // sequences the model cannot encode.
            let max_position = config
                .get(reference.max_tokens_field)
                .and_then(Value::as_u64)
                .unwrap_or_else(|| {
                    panic!(
                        "{}: config.json has no {}",
                        descriptor.model_id, reference.max_tokens_field
                    )
                });
            let usable_window = max_position - reference.max_position_offset;
            assert_eq!(
                usable_window as usize, descriptor.max_tokens,
                "{}: descriptor.max_tokens ({}) drifted from config {} ({} - {} offset)",
                descriptor.model_id,
                descriptor.max_tokens,
                reference.max_tokens_field,
                max_position,
                reference.max_position_offset
            );

            // nomic names the layer count `n_layer`; XLM-Roberta and Stella use
            // `num_hidden_layers`. A sane positive count guards against a wrong
            // fixture/model pairing.
            let layers = config
                .get("num_hidden_layers")
                .or_else(|| config.get("n_layer"))
                .and_then(Value::as_u64)
                .unwrap_or_else(|| {
                    panic!("{}: config.json has no layer count", descriptor.model_id)
                });
            assert!(
                (1..=64).contains(&layers),
                "{}: implausible layer count {layers}",
                descriptor.model_id
            );
        }
    }

    /// **Cross-model contamination guard.** Every catalog model MUST have a
    /// distinct vector dimension. This is a load-bearing invariant for the
    /// `app-infra` vector store: during a model switch, `store_vector_if_dimension_matches`
    /// and `recreate_vectors_table` use the live `vec0` column *width* as the ONLY
    /// discriminator between the old and new embedding spaces (they stamp no model
    /// identity). That is sound only while dimensions are pairwise distinct — the
    /// moment two models share a dimension, an in-flight old-model vector could be
    /// written into the new-model index silently (a different embedding space, no
    /// error, no self-heal). If this test fails, you are adding a colliding-dimension
    /// model: the dimension check no longer guards contamination, and the store seam
    /// needs a stronger model-identity/epoch guard before that model can ship.
    #[test]
    fn catalog_dimensions_are_pairwise_distinct() {
        let descriptors = catalog();
        let mut seen: Vec<(usize, &str)> = Vec::with_capacity(descriptors.len());
        for descriptor in &descriptors {
            if let Some((_, other_id)) =
                seen.iter().find(|(dimension, _)| *dimension == descriptor.dimension)
            {
                panic!(
                    "catalog models {} and {} share dimension {} — distinct dimensions are the \
                     ONLY guard against cross-model contamination in the vector store (see \
                     store_vector_if_dimension_matches / recreate_vectors_table in \
                     app-infra/src/semantic_search.rs); a same-dimension model needs a stronger \
                     model-identity/epoch guard there before it can ship",
                    other_id, descriptor.model_id, descriptor.dimension
                );
            }
            seen.push((descriptor.dimension, &descriptor.model_id));
        }
    }

    #[test]
    fn no_installed_model_makes_feature_a_silent_no_op_not_an_error() {
        use capture_types::default_semantic_search_settings;
        let temp = tempfile::tempdir().expect("tempdir");
        let settings = default_semantic_search_settings();
        assert!(settings.enabled, "default settings are on");
        let available = selected_semantic_search_model_available(temp.path(), &settings)
            .expect("availability check must not error when the model is absent");
        assert!(!available, "no installed model => silent no-op (false)");
    }

    #[test]
    fn selected_model_available_only_once_installed() {
        use capture_types::default_semantic_search_settings;
        let temp = tempfile::tempdir().expect("tempdir");
        let settings = default_semantic_search_settings();
        let models_dir = semantic_search_models_dir(temp.path());
        let descriptor = resolve_descriptor(
            &settings.provider,
            settings.model_id.as_deref().expect("default selects a model"),
        )
        .expect("selected descriptor resolves");

        assert!(!selected_semantic_search_model_available(temp.path(), &settings).expect("check"));
        install_model(&models_dir, &descriptor);
        assert!(selected_semantic_search_model_available(temp.path(), &settings).expect("check"));
    }

    #[test]
    fn disabled_settings_are_never_available_even_with_a_model() {
        use capture_types::default_semantic_search_settings;
        let temp = tempfile::tempdir().expect("tempdir");
        let mut settings = default_semantic_search_settings();
        let models_dir = semantic_search_models_dir(temp.path());
        install_model(&models_dir, &builtin_model_manifest().models[0]);

        settings.enabled = false;
        assert!(!selected_semantic_search_model_available(temp.path(), &settings).expect("check"));
    }

    #[test]
    fn unknown_selected_model_is_not_available() {
        use capture_types::default_semantic_search_settings;
        let temp = tempfile::tempdir().expect("tempdir");
        let mut settings = default_semantic_search_settings();
        settings.model_id = Some("not-a-real-model".to_string());
        assert!(!selected_semantic_search_model_available(temp.path(), &settings).expect("check"));
    }
}
