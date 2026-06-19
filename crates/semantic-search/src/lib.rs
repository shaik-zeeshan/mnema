//! Semantic Search embedding runtime and its model-gating.
//!
//! This crate derives a **Semantic Search Vector** from raw `body_text` using
//! **candle** (`candle-core`/`candle-nn`/`candle-transformers`), running the model
//! on the **Apple GPU via Metal** (when the crate `metal` feature is on and a
//! Metal device is available) or on the **CPU** otherwise — the runtime tries
//! Metal then falls back to CPU. candle is the default and only shipped backend,
//! sitting behind the pluggable [`SemanticSearchBackend`] trait so a future local
//! Ollama backend (and, opt-in, a cloud backend) can plug in without touching the
//! storage or query layers (ADR 0037).
//!
//! The backend is the **raw model forward** (tokenize, run the architecture, pool,
//! L2-normalize). The backend-agnostic chunking / length-bucketed sub-batching /
//! cross-chunk fan-in + mean-pool that turns arbitrary text into one stored vector
//! per anchor lives ABOVE the trait, in [`SemanticSearchEmbedder`].
//!
//! Like local transcription/OCR, **Semantic Search** is default-on but
//! **model-gated**: with no **Semantic Search Model** installed under
//! `semantic_search_models/{provider}/{model_id}/` the feature is a silent
//! no-op — never an error, never blocking capture — and Mnema never
//! auto-downloads a model here.
//!
//! Mirroring `audio-transcription`, this crate intentionally does not depend on
//! `app-infra` or Tauri. The desktop app supplies the app data directory and owns
//! download orchestration.

mod backend;
mod models;
mod runtime;

pub use backend::candle::CandleBackend;
pub use backend::{EmbeddingError, SemanticSearchBackend};

pub use models::{
    builtin_model_manifest, detect_model_status, find_model_descriptor, list_supported_models,
    model_install_dir, resolve_descriptor, selected_semantic_search_model_available,
    semantic_search_models_dir, write_installed_marker, InstalledModelLayout, ModelStatusError,
    ModelStatusKind, SemanticSearchArchitecture, SemanticSearchModelDescriptor,
    SemanticSearchModelManifest, SemanticSearchModelStatus, SemanticSearchModelTier,
    SemanticSearchPooling, SupportedEmbeddingModel, CONFIG_FILE_NAME, FASTEMBED_PROVIDER_ID,
    INSTALLED_MARKER_FILE_NAME, MODEL_SAFETENSORS_FILE_NAME, MODEL_STORE_DIR_NAME,
    TOKENIZER_FILE_NAME,
};

pub use runtime::SemanticSearchEmbedder;
