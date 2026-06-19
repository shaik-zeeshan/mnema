//! The pluggable **Semantic Search Backend** seam: a raw model forward.
//!
//! A backend embeds **already-chunked, in-window** text fragments into one
//! L2-normalized, per-model-pooled vector each — nothing more. The
//! split / length-bucketed sub-batch / cross-chunk fan-in + mean-pool that turns
//! arbitrary `body_text` into one stored vector per anchor lives ABOVE the trait,
//! in [`crate::SemanticSearchEmbedder`], so it is shared by every backend.
//!
//! candle is the only v1 implementation (Apple GPU via Metal, or CPU). The trait
//! exists so a future local Ollama backend — and, opt-in, a cloud backend — can
//! plug in without touching the storage or query layers (ADR 0037).

pub mod candle;

use std::path::PathBuf;

use thiserror::Error;

/// Backend-neutral embedding error.
///
/// No variant names a specific runtime: the same shape covers candle (and any
/// future backend). `ReadModelFile` / `LoadModel` / `LoadTokenizer` / `Tokenize`
/// / `Embed` / `EmptyEmbedding` are carried over unchanged from the fastembed era;
/// `LoadConfig` and `Device` were added for candle (parsing `config.json` and
/// acquiring a compute device).
#[derive(Debug, Error)]
pub enum EmbeddingError {
    #[error("failed to read model file {path}: {source}")]
    ReadModelFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to load model config: {0}")]
    LoadConfig(String),
    #[error("failed to acquire compute device: {0}")]
    Device(String),
    #[error("failed to load model: {0}")]
    LoadModel(String),
    #[error("failed to load tokenizer: {0}")]
    LoadTokenizer(String),
    #[error("failed to tokenize text: {0}")]
    Tokenize(String),
    #[error("failed to embed text: {0}")]
    Embed(String),
    #[error("model produced an empty embedding")]
    EmptyEmbedding,
}

/// The raw model forward, the one seam every embedding runtime implements.
///
/// Object-safe (held as `Box<dyn SemanticSearchBackend>` by the embedder
/// wrapper). It embeds pre-chunked, in-window fragments; chunking and cross-chunk
/// pooling are NOT its concern — see the module docs.
pub trait SemanticSearchBackend: Send {
    /// Embed already-chunked, in-window text fragments → one L2-normalized,
    /// per-model-pooled (Mean or CLS) vector per input, in input order.
    ///
    /// The caller guarantees each fragment fits the model window and the batch is
    /// within the backend's batch ceiling.
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError>;

    /// The vector dimension this backend's model produces.
    fn dimension(&self) -> usize;

    /// The model's token window (used by the wrapper to size its chunk windows).
    fn max_tokens(&self) -> usize;
}
