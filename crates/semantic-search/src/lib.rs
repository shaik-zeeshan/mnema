//! Semantic Search embedding runtime and its model-gating.
//!
//! This crate derives a **Semantic Search Vector** from raw `body_text` using
//! `fastembed`, which reuses the ONNX runtime (`ort`) already shipped for
//! Parakeet transcription — there is no second native runtime (ADR 0036).
//!
//! Like local transcription/OCR, **Semantic Search** is default-on but
//! **model-gated**: with no **Semantic Search Model** installed under
//! `semantic_search_models/{provider}/{model_id}/` the feature is a silent
//! no-op — never an error, never blocking capture — and Mnema never
//! auto-downloads a model here.
//!
//! Mirroring `audio-transcription`, this crate intentionally does not depend on
//! `app-infra` or Tauri. The desktop app supplies the app data directory and
//! owns download orchestration.

mod models;
#[cfg(feature = "fastembed")]
mod runtime;

pub use models::{
    builtin_model_manifest, detect_model_status, find_model_descriptor, model_install_dir,
    selected_semantic_search_model_available, semantic_search_models_dir, write_installed_marker,
    InstalledModelLayout, ModelStatusError, ModelStatusKind, SemanticSearchModelDescriptor,
    SemanticSearchModelManifest, SemanticSearchModelStatus, SemanticSearchModelTier,
    SemanticSearchOutputKey, SemanticSearchPooling, CONFIG_FILE_NAME, FASTEMBED_PROVIDER_ID,
    INSTALLED_MARKER_FILE_NAME, MODEL_ONNX_FILE_NAME, MODEL_STORE_DIR_NAME,
    SPECIAL_TOKENS_MAP_FILE_NAME, TOKENIZER_CONFIG_FILE_NAME, TOKENIZER_FILE_NAME,
};

#[cfg(feature = "fastembed")]
pub use runtime::{
    fastembed_output_key, fastembed_pooling, list_fastembed_supported_models, resolve_descriptor,
    EmbeddingError, SemanticSearchEmbedder, SupportedEmbeddingModel,
};

/// Re-export of fastembed's pooling strategy and named-output key so callers
/// select a model's pooling (Mean for nomic/e5, Cls for bge) and pass through any
/// `ModelInfo.output_key` without taking a direct `fastembed` dependency. The
/// embedding runtime lives behind the `fastembed` feature.
#[cfg(feature = "fastembed")]
pub use fastembed::{OutputKey, Pooling};
