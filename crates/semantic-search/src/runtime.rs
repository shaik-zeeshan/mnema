//! The embedding runtime: derives a **Semantic Search Vector** from raw text.
//!
//! `fastembed` reuses the shared `ort` ONNX runtime (the same one Parakeet
//! transcription ships) — there is no second native runtime. Models are loaded
//! from disk via fastembed's user-defined ("bring your own") path, so nothing is
//! fetched online at embed time (ADR 0036: Mnema never auto-downloads a model).
//!
//! Overflow handling: fastembed's tokenizer silently truncates text past the
//! model's token window. To honor "auto-split on overflow, never silently
//! truncated/dropped", this runtime counts tokens up front with a non-truncating
//! tokenizer; when the text overflows the window it is split into token-window
//! chunks, each chunk is embedded, and the chunk vectors are mean-pooled and
//! L2-normalized into one vector.

use std::path::{Path, PathBuf};

use fastembed::{
    InitOptionsUserDefined, OutputKey, Pooling, QuantizationMode, TextEmbedding, TokenizerFiles,
    UserDefinedEmbeddingModel,
};
use thiserror::Error;
use tokenizers::Tokenizer;

use crate::models::{
    builtin_model_manifest, find_model_descriptor, InstalledModelLayout, SemanticSearchModelDescriptor,
    SemanticSearchModelTier, SemanticSearchOutputKey, SemanticSearchPooling, CONFIG_FILE_NAME,
    FASTEMBED_PROVIDER_ID, SPECIAL_TOKENS_MAP_FILE_NAME, TOKENIZER_CONFIG_FILE_NAME,
    TOKENIZER_FILE_NAME,
};

/// Special-token budget reserved per chunk (e.g. `[CLS]`/`[SEP]`) so a split
/// chunk plus its special tokens still fits the model window.
const SPECIAL_TOKEN_HEADROOM: usize = 2;

/// Map the serde-friendly descriptor pooling onto fastembed's `Pooling`. Lives
/// here (behind the `fastembed` feature) so the descriptor module needs no
/// fastembed dependency, while the runtime still loads each model with the exact
/// pooling fastembed assigns it. Exported so the desktop worker can pass
/// `descriptor.pooling` straight through to [`SemanticSearchEmbedder::load_from_dir`]
/// instead of re-deriving pooling from the model id.
pub fn fastembed_pooling(pooling: SemanticSearchPooling) -> Pooling {
    match pooling {
        SemanticSearchPooling::Mean => Pooling::Mean,
        SemanticSearchPooling::Cls => Pooling::Cls,
    }
}

/// Capture fastembed's own pooling for a model into the serde-friendly mirror.
/// `get_default_pooling_method` is `Option`; every text-embedding model in
/// fastembed 5.17.2 returns `Some`, but we fall back to `Mean` (the BERT-family
/// sentence default) rather than panic if a future model returns `None`.
fn pooling_from_fastembed(pooling: Option<Pooling>) -> SemanticSearchPooling {
    match pooling {
        Some(Pooling::Cls) => SemanticSearchPooling::Cls,
        Some(Pooling::Mean) | None => SemanticSearchPooling::Mean,
    }
}

/// Map the serde-friendly descriptor output key onto fastembed's `OutputKey`.
/// Exported alongside [`fastembed_pooling`] so the desktop worker passes
/// `descriptor.output_key` through unchanged.
pub fn fastembed_output_key(output_key: &Option<SemanticSearchOutputKey>) -> Option<OutputKey> {
    output_key.as_ref().map(|key| match key {
        SemanticSearchOutputKey::OnlyOne => OutputKey::OnlyOne,
        SemanticSearchOutputKey::ByOrder(index) => OutputKey::ByOrder(*index),
        // fastembed's `ByName` is `&'static str`. The only named outputs in the
        // 5.17.2 catalog are the gated EmbeddingGemma variants (excluded from
        // synthesis), so this maps the known static names; an unknown name falls
        // back to the default single output rather than leaking memory.
        SemanticSearchOutputKey::ByName(name) => match name.as_str() {
            "sentence_embedding" => OutputKey::ByName("sentence_embedding"),
            "last_hidden_state" => OutputKey::ByName("last_hidden_state"),
            _ => OutputKey::OnlyOne,
        },
    })
}

/// Capture fastembed's `ModelInfo.output_key` into the serde-friendly mirror.
fn output_key_from_fastembed(output_key: Option<OutputKey>) -> Option<SemanticSearchOutputKey> {
    output_key.map(|key| match key {
        OutputKey::OnlyOne => SemanticSearchOutputKey::OnlyOne,
        OutputKey::ByOrder(index) => SemanticSearchOutputKey::ByOrder(index),
        OutputKey::ByName(name) => SemanticSearchOutputKey::ByName(name.to_string()),
    })
}

#[derive(Debug, Error)]
pub enum EmbeddingError {
    #[error("failed to read model file {path}: {source}")]
    ReadModelFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to load fastembed model: {0}")]
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

/// A loaded **Semantic Search Model** ready to derive **Semantic Search
/// Vectors**. Holds the fastembed session plus a non-truncating tokenizer used
/// only to detect and split overflowing text.
pub struct SemanticSearchEmbedder {
    embedder: TextEmbedding,
    /// Non-truncating tokenizer over the same vocab, for overflow detection and
    /// token-window splitting (fastembed's own tokenizer truncates).
    split_tokenizer: Tokenizer,
    max_tokens: usize,
}

impl SemanticSearchEmbedder {
    /// Load a Semantic Search Model from a
    /// `semantic_search_models/{provider}/{model_id}/` directory.
    ///
    /// `max_tokens` is the model's token window (from the catalog descriptor)
    /// and `pooling` its pooling strategy (Mean for nomic/e5, Cls for bge).
    ///
    /// `layout` carries the fastembed-`ModelInfo`-derived on-disk shape: the
    /// repo-relative ONNX path (e.g. `onnx/model.onnx`) and any external-data
    /// siblings (e.g. `onnx/model.onnx_data`). Because the ONNX graph is loaded
    /// **from memory** here (fastembed's "bring your own" path), it cannot resolve
    /// a sibling external-data file by directory the way an on-disk
    /// `commit_from_file` would. So for every external-data file we register an
    /// external initializer keyed by its **basename** — the name the graph's
    /// `external_data.location` field references (e.g. `model.onnx_data`). Without
    /// this, a model like bge-m3 would load but produce no usable weights. Models
    /// with no external data (nomic / e5-small) take the self-contained path.
    ///
    /// `output_key` mirrors fastembed's `ModelInfo.output_key`: most sentence
    /// models (nomic / e5 / bge) use the default mean/CLS-pooled output, but
    /// passing it through keeps any model that names a specific output tensor
    /// correct.
    pub fn load_from_dir(
        model_dir: impl AsRef<Path>,
        max_tokens: usize,
        pooling: Pooling,
        layout: &InstalledModelLayout,
        output_key: Option<OutputKey>,
    ) -> Result<Self, EmbeddingError> {
        let model_dir = model_dir.as_ref();
        let onnx_file = read_file(model_dir, &layout.onnx_relative_path)?;
        let tokenizer_bytes = read_file(model_dir, TOKENIZER_FILE_NAME)?;
        let tokenizer_files = TokenizerFiles {
            tokenizer_file: tokenizer_bytes.clone(),
            config_file: read_file(model_dir, CONFIG_FILE_NAME)?,
            special_tokens_map_file: read_file(model_dir, SPECIAL_TOKENS_MAP_FILE_NAME)?,
            tokenizer_config_file: read_file(model_dir, TOKENIZER_CONFIG_FILE_NAME)?,
        };

        let mut user_model = UserDefinedEmbeddingModel::new(onnx_file, tokenizer_files)
            .with_pooling(pooling)
            .with_quantization(QuantizationMode::None);
        // Register each external-data sibling as an in-memory external initializer,
        // keyed by the basename the ONNX graph references.
        for external_relative in &layout.external_data_files {
            let buffer = read_file(model_dir, external_relative)?;
            let file_name = Path::new(external_relative)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| external_relative.clone());
            user_model = user_model.with_external_initializer(file_name, buffer);
        }
        // `output_key` is a public field on the user-defined model (there is no
        // builder for it in fastembed 5.17.2).
        user_model.output_key = output_key;

        let embedder = TextEmbedding::try_new_from_user_defined(
            user_model,
            InitOptionsUserDefined::new().with_max_length(max_tokens),
        )
        .map_err(|error| EmbeddingError::LoadModel(error.to_string()))?;

        // A separate, untruncated tokenizer so we can see the *full* token count
        // and slice the original text on token boundaries.
        let split_tokenizer = Tokenizer::from_bytes(tokenizer_bytes)
            .map_err(|error| EmbeddingError::LoadTokenizer(error.to_string()))?;

        Ok(Self {
            embedder,
            split_tokenizer,
            max_tokens,
        })
    }

    /// Derive a single **Semantic Search Vector** (f32) for a UTF-8 string.
    ///
    /// Text within the model's token window is embedded directly. Text that
    /// overflows the window is auto-split into token-window chunks (never
    /// silently truncated), each chunk embedded, and the chunk vectors mean-
    /// pooled and L2-normalized into one vector.
    pub fn embed_text(&mut self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let chunks = self.split_on_overflow(text)?;
        if chunks.len() <= 1 {
            let single = chunks.into_iter().next().unwrap_or_default();
            let mut vectors = self
                .embedder
                .embed(vec![single], None)
                .map_err(|error| EmbeddingError::Embed(error.to_string()))?;
            return vectors.pop().ok_or(EmbeddingError::EmptyEmbedding);
        }

        let vectors = self
            .embedder
            .embed(chunks, None)
            .map_err(|error| EmbeddingError::Embed(error.to_string()))?;
        mean_pool_l2(&vectors).ok_or(EmbeddingError::EmptyEmbedding)
    }

    /// Split `text` into chunks that each fit the model's token window. Returns a
    /// single-element vec when the text already fits.
    fn split_on_overflow(&self, text: &str) -> Result<Vec<String>, EmbeddingError> {
        split_text_on_token_overflow(&self.split_tokenizer, text, self.max_tokens)
    }
}

/// Split `text` so each chunk fits `max_tokens` (minus special-token headroom),
/// mapping token windows back to char spans of the original text so no content
/// is dropped at a chunk boundary. Returns a single-element vec when the text
/// already fits the window.
fn split_text_on_token_overflow(
    tokenizer: &Tokenizer,
    text: &str,
    max_tokens: usize,
) -> Result<Vec<String>, EmbeddingError> {
    // Encode without adding special tokens so the count reflects the raw content
    // tokens; fastembed adds its own special tokens at embed time.
    let encoding = tokenizer
        .encode(text, false)
        .map_err(|error| EmbeddingError::Tokenize(error.to_string()))?;
    let offsets = encoding.get_offsets();

    let budget = max_tokens.saturating_sub(SPECIAL_TOKEN_HEADROOM).max(1);
    if offsets.len() <= budget {
        return Ok(vec![text.to_string()]);
    }

    let mut chunks = Vec::new();
    let mut start_token = 0usize;
    while start_token < offsets.len() {
        let end_token = (start_token + budget).min(offsets.len());
        let char_start = offsets[start_token].0;
        let char_end = offsets[end_token - 1].1;
        if let Some(slice) = text.get(char_start..char_end) {
            if !slice.trim().is_empty() {
                chunks.push(slice.to_string());
            }
        }
        start_token = end_token;
    }

    if chunks.is_empty() {
        chunks.push(text.to_string());
    }
    Ok(chunks)
}

fn read_file(model_dir: &Path, file_name: &str) -> Result<Vec<u8>, EmbeddingError> {
    let path = model_dir.join(file_name);
    std::fs::read(&path).map_err(|source| EmbeddingError::ReadModelFile { path, source })
}

/// A fastembed text-embedding model the **Custom** picker can offer, distilled
/// from fastembed's `ModelInfo` to just the fields the Settings UI needs. The
/// `model_id` is a stable slug derived from the HuggingFace `model_code`'s last
/// path segment, so it round-trips through the same `{provider}/{model_id}`
/// install layout as the guided tiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupportedEmbeddingModel {
    pub model_id: String,
    pub display_name: String,
    pub model_code: String,
    pub dimension: usize,
    pub description: String,
    /// On-disk path of the primary ONNX graph (e.g. `onnx/model.onnx`).
    pub onnx_relative_path: String,
    /// External-data siblings the model references (e.g. `onnx/model.onnx_data`).
    pub external_data_files: Vec<String>,
    /// Cheap multilingual heuristic from the model code (e5 / bge-m3 / "multilingual").
    pub multilingual: bool,
    /// fastembed's own pooling for this model (`get_default_pooling_method`), so a
    /// Custom pick carries the right strategy through the same path the guided
    /// tiers do — CLS for mxbai / gte / snowflake-arctic, Mean for nomic / e5.
    pub pooling: SemanticSearchPooling,
    /// fastembed's `ModelInfo.output_key` for this model (`None` for the default
    /// single output).
    pub output_key: Option<SemanticSearchOutputKey>,
}

/// The canonical (full-precision) ONNX file name we standardize on.
///
/// fastembed enumerates several model codes twice — a quantized `model.onnx` and
/// a quantized `model_quantized.onnx` (and `model_q4.onnx`) variant. Mnema loads
/// with `QuantizationMode::None`, the manifest's guided tiers all expect
/// `onnx/model.onnx`, and the picker shows one entry per model — so both the
/// enumeration and the synthesized descriptor must standardize on the
/// full-precision `model.onnx`. Quantized variants are dropped.
const CANONICAL_ONNX_FILE_NAME: &str = "model.onnx";

/// Whether a fastembed `model_file` path is the canonical full-precision graph
/// (`…/model.onnx`), as opposed to a `model_quantized.onnx` / `model_q4.onnx`.
fn is_canonical_onnx_file(model_file: &str) -> bool {
    Path::new(model_file)
        .file_name()
        .map(|name| name == CANONICAL_ONNX_FILE_NAME)
        .unwrap_or(false)
}

/// fastembed's supported text-embedding models, deduped to one entry per
/// `model_code` — the full-precision `model.onnx` variant — and with gated repos
/// (EmbeddingGemma) excluded.
///
/// Deduping is what keeps the picker slug, the resolver, and the download file
/// list in agreement: without it, a code enumerated as both `model.onnx` and
/// `model_quantized.onnx` would slug to the same `model_id` twice and a `.find`
/// could pick the quantized variant, mismatching the manifest layout. The
/// canonical (`model.onnx`) variant is preferred; if a code has no full-precision
/// variant its first entry is kept so the model is still offered.
fn canonical_fastembed_models() -> Vec<fastembed::ModelInfo<fastembed::EmbeddingModel>> {
    let mut chosen: Vec<fastembed::ModelInfo<fastembed::EmbeddingModel>> = Vec::new();
    for info in TextEmbedding::list_supported_models() {
        if is_gated_model_code(&info.model_code) {
            continue;
        }
        match chosen.iter().position(|kept| kept.model_code == info.model_code) {
            Some(index) => {
                // Upgrade a previously-kept quantized variant to the canonical one.
                if !is_canonical_onnx_file(&chosen[index].model_file)
                    && is_canonical_onnx_file(&info.model_file)
                {
                    chosen[index] = info;
                }
            }
            None => chosen.push(info),
        }
    }
    chosen
}

/// Enumerate fastembed's supported text-embedding models for the **Custom**
/// picker. Gated models we cannot ship by default (notably EmbeddingGemma, whose
/// repo is access-gated on HuggingFace) are excluded so the picker only offers
/// models the manual reqwest downloader can actually fetch, and each model_code
/// is offered once (its full-precision variant).
pub fn list_fastembed_supported_models() -> Vec<SupportedEmbeddingModel> {
    canonical_fastembed_models()
        .into_iter()
        .map(|info| {
            let model_id = slug_from_model_code(&info.model_code);
            let pooling = pooling_from_fastembed(
                TextEmbedding::get_default_pooling_method(&info.model),
            );
            let output_key = output_key_from_fastembed(info.output_key.clone());
            SupportedEmbeddingModel {
                display_name: humanize_model_code(&info.model_code),
                multilingual: looks_multilingual(&info.model_code),
                dimension: info.dim,
                description: info.description.clone(),
                onnx_relative_path: info.model_file.clone(),
                external_data_files: info.additional_files.clone(),
                model_code: info.model_code.clone(),
                model_id,
                pooling,
                output_key,
            }
        })
        .collect()
}

/// The token window assumed for a **Custom**-picked fastembed model.
///
/// fastembed's `ModelInfo` carries no token window, so a synthesized descriptor
/// cannot know the model's real limit. 512 is the conservative BERT-family
/// default that every supported encoder honors; overflowing text is auto-split on
/// this window (never silently truncated) by the runtime, so a too-small guess
/// only costs extra (still-correct) chunks, never dropped content.
const CUSTOM_MODEL_DEFAULT_MAX_TOKENS: usize = 512;

/// Resolve a **Semantic Search Model** descriptor for a `{provider}/{model_id}`
/// selection, including **Custom**-picked fastembed models outside the guided
/// manifest.
///
/// The manifest is consulted first (the guided English / Multilingual / bge-m3
/// tiers carry hand-tuned dimension, token window, license, and external-data
/// layout). When the id is not a manifest model AND the provider is fastembed, a
/// descriptor is synthesized from fastembed's own `ModelInfo` enumeration: the
/// `ModelInfo` whose derived slug ([`slug_from_model_code`], the SAME slug the
/// **Custom** picker shows) equals `model_id`. This is what lets a Custom pick
/// download, install, gate, and embed under the same `{provider}/{model_id}`
/// layout as the guided tiers — the picker's `modelId` round-trips back to a
/// descriptor here.
///
/// Synthesized descriptors use [`SemanticSearchModelTier::Custom`], a 512-token
/// default window ([`CUSTOM_MODEL_DEFAULT_MAX_TOKENS`]), `license_label = None`
/// (fastembed carries no license), and `approx_download_bytes = 0` (no size in
/// `ModelInfo`; the streaming download reports the real content-length anyway).
/// Gated repos (EmbeddingGemma) are excluded from synthesis too, matching the
/// picker, since the manual reqwest downloader cannot fetch them.
pub fn resolve_descriptor(
    provider: &str,
    model_id: &str,
) -> Option<SemanticSearchModelDescriptor> {
    let manifest = builtin_model_manifest();
    if let Some(descriptor) = find_model_descriptor(&manifest, provider, model_id) {
        return Some(descriptor.clone());
    }
    if provider != FASTEMBED_PROVIDER_ID {
        return None;
    }
    synthesize_fastembed_descriptor(model_id)
}

/// Synthesize a descriptor for a non-manifest fastembed model from its
/// `ModelInfo`, matched by the picker's slug. Returns `None` when no enumerable
/// (non-gated) model slugs to `model_id`.
fn synthesize_fastembed_descriptor(model_id: &str) -> Option<SemanticSearchModelDescriptor> {
    canonical_fastembed_models()
        .into_iter()
        .find(|info| slug_from_model_code(&info.model_code) == model_id)
        .map(|info| {
            let expected_layout = InstalledModelLayout::from_fastembed_files(
                info.model_file.clone(),
                info.additional_files.clone(),
            );
            // Read pooling + output key from fastembed's own metadata for this
            // exact model, NOT a guess from the id: `get_default_pooling_method`
            // assigns CLS to mxbai / gte / snowflake-arctic (none start with
            // "bge"), so guessing by prefix silently mean-pooled them.
            let pooling = pooling_from_fastembed(
                TextEmbedding::get_default_pooling_method(&info.model),
            );
            let output_key = output_key_from_fastembed(info.output_key.clone());
            SemanticSearchModelDescriptor {
                provider: FASTEMBED_PROVIDER_ID.to_string(),
                model_id: model_id.to_string(),
                display_name: humanize_model_code(&info.model_code),
                description: info.description.clone(),
                tier: SemanticSearchModelTier::Custom,
                model_code: info.model_code.clone(),
                // fastembed's ModelInfo carries no license; unknown for a Custom pick.
                license_label: None,
                dimension: info.dim,
                max_tokens: CUSTOM_MODEL_DEFAULT_MAX_TOKENS,
                // Unknown up front; the streaming download reports real content-length.
                approx_download_bytes: 0,
                pooling,
                output_key,
                expected_layout,
            }
        })
}

/// Models whose HuggingFace repo is access-gated and therefore cannot be fetched
/// by Mnema's manual (non-`hf-hub`) reqwest downloader. EmbeddingGemma at minimum.
fn is_gated_model_code(model_code: &str) -> bool {
    let lower = model_code.to_ascii_lowercase();
    lower.contains("gemma")
}

/// A stable slug for a model: the last path segment of its HF `model_code`,
/// lowercased (e.g. `intfloat/multilingual-e5-small` -> `multilingual-e5-small`).
fn slug_from_model_code(model_code: &str) -> String {
    model_code
        .rsplit('/')
        .next()
        .unwrap_or(model_code)
        .to_ascii_lowercase()
}

/// A human-friendly display name derived from the model code's last segment.
fn humanize_model_code(model_code: &str) -> String {
    let last = model_code.rsplit('/').next().unwrap_or(model_code);
    let mut out = String::with_capacity(last.len());
    let mut capitalize_next = true;
    for ch in last.chars() {
        if ch == '-' || ch == '_' {
            out.push(' ');
            capitalize_next = true;
        } else if capitalize_next {
            out.extend(ch.to_uppercase());
            capitalize_next = false;
        } else {
            out.push(ch);
        }
    }
    out
}

/// Cheap multilingual heuristic: e5, bge-m3, and any "multilingual" code.
fn looks_multilingual(model_code: &str) -> bool {
    let lower = model_code.to_ascii_lowercase();
    lower.contains("multilingual") || lower.contains("e5") || lower.contains("bge-m3")
}

/// Mean-pool a set of chunk vectors and L2-normalize the result, matching the
/// per-vector normalization fastembed applies, so a split text is comparable to
/// an unsplit one.
fn mean_pool_l2(vectors: &[Vec<f32>]) -> Option<Vec<f32>> {
    let first = vectors.first()?;
    let dim = first.len();
    if dim == 0 || vectors.iter().any(|v| v.len() != dim) {
        return None;
    }
    let mut summed = vec![0f32; dim];
    for vector in vectors {
        for (acc, value) in summed.iter_mut().zip(vector.iter()) {
            *acc += *value;
        }
    }
    let count = vectors.len() as f32;
    for acc in summed.iter_mut() {
        *acc /= count;
    }
    let norm = summed.iter().map(|v| v * v).sum::<f32>().sqrt();
    let epsilon = 1e-12f32;
    for acc in summed.iter_mut() {
        *acc /= norm + epsilon;
    }
    Some(summed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokenizers::models::wordlevel::WordLevel;
    use tokenizers::pre_tokenizers::whitespace::Whitespace;

    /// A tiny whitespace word-level tokenizer (one token per word) so token
    /// counts are predictable and we can assert the split shape exactly.
    fn whitespace_tokenizer() -> Tokenizer {
        let vocab = ["[UNK]", "alpha", "bravo", "charlie", "delta", "echo", "foxtrot"]
            .iter()
            .enumerate()
            .map(|(index, word)| ((*word).to_string(), index as u32))
            .collect();
        let model = WordLevel::builder()
            .vocab(vocab)
            .unk_token("[UNK]".to_string())
            .build()
            .expect("word level model");
        let mut tokenizer = Tokenizer::new(model);
        tokenizer.with_pre_tokenizer(Some(Whitespace {}));
        tokenizer
    }

    #[test]
    fn text_within_window_is_not_split() {
        let tokenizer = whitespace_tokenizer();
        // 3 content tokens, budget = 8 - 2 = 6 => fits.
        let chunks =
            split_text_on_token_overflow(&tokenizer, "alpha bravo charlie", 8).expect("split");
        assert_eq!(chunks, vec!["alpha bravo charlie".to_string()]);
    }

    #[test]
    fn overflowing_text_is_auto_split_not_truncated() {
        let tokenizer = whitespace_tokenizer();
        // max_tokens = 4 => budget = 4 - 2 = 2 content tokens per chunk.
        // 6 content words => 3 chunks of 2 words each, covering every word.
        let text = "alpha bravo charlie delta echo foxtrot";
        let chunks = split_text_on_token_overflow(&tokenizer, text, 4).expect("split");
        assert_eq!(
            chunks,
            vec![
                "alpha bravo".to_string(),
                "charlie delta".to_string(),
                "echo foxtrot".to_string(),
            ],
            "every token must survive the split — nothing truncated or dropped"
        );
        // No content is lost: concatenated chunk words equal the original words.
        let recombined: Vec<&str> = chunks.iter().flat_map(|c| c.split_whitespace()).collect();
        assert_eq!(recombined, text.split_whitespace().collect::<Vec<_>>());
    }

    #[test]
    fn split_covers_remainder_when_not_evenly_divisible() {
        let tokenizer = whitespace_tokenizer();
        // budget = 2, 5 words => chunks of [2,2,1] — the trailing word is kept.
        let text = "alpha bravo charlie delta echo";
        let chunks = split_text_on_token_overflow(&tokenizer, text, 4).expect("split");
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks.last().unwrap(), "echo");
    }

    #[test]
    fn mean_pool_l2_averages_and_normalizes() {
        let pooled = mean_pool_l2(&[vec![3.0, 0.0], vec![0.0, 3.0]]).expect("pooled");
        // Mean is (1.5, 1.5); after L2 normalization each component is ~0.7071.
        assert_eq!(pooled.len(), 2);
        let norm = (pooled[0] * pooled[0] + pooled[1] * pooled[1]).sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "result must be unit length");
        assert!((pooled[0] - pooled[1]).abs() < 1e-6, "symmetric inputs stay symmetric");
    }

    #[test]
    fn mean_pool_rejects_ragged_or_empty_inputs() {
        assert!(mean_pool_l2(&[]).is_none());
        assert!(mean_pool_l2(&[vec![1.0, 2.0], vec![1.0]]).is_none());
        assert!(mean_pool_l2(&[vec![], vec![]]).is_none());
    }

    #[test]
    fn resolve_descriptor_returns_manifest_tier_unchanged() {
        // A guided-tier id resolves to the hand-tuned manifest descriptor (License,
        // long token window, etc.) — NOT a synthesized one.
        let descriptor =
            resolve_descriptor(FASTEMBED_PROVIDER_ID, "nomic-embed-text-v1.5").expect("nomic");
        assert_eq!(descriptor.tier, SemanticSearchModelTier::English);
        assert_eq!(descriptor.max_tokens, 8192);
        assert_eq!(descriptor.license_label.as_deref(), Some("Apache-2.0"));
    }

    #[test]
    fn resolve_descriptor_synthesizes_a_non_manifest_fastembed_model() {
        // Pick any enumerable, non-gated model that is NOT one of the 3 manifest
        // tiers, then prove its picker slug resolves back to a complete descriptor.
        let manifest_ids: Vec<String> = builtin_model_manifest()
            .models
            .into_iter()
            .map(|model| model.model_id)
            .collect();
        let custom = list_fastembed_supported_models()
            .into_iter()
            .find(|model| !manifest_ids.contains(&model.model_id))
            .expect("fastembed should enumerate at least one non-manifest model");

        // The picker's slug (`model_id`) must round-trip back to a descriptor.
        let descriptor =
            resolve_descriptor(FASTEMBED_PROVIDER_ID, &custom.model_id).unwrap_or_else(|| {
                panic!("custom model slug {} must resolve to a descriptor", custom.model_id)
            });

        assert_eq!(descriptor.provider, FASTEMBED_PROVIDER_ID);
        assert_eq!(descriptor.model_id, custom.model_id);
        assert_eq!(descriptor.tier, SemanticSearchModelTier::Custom);
        // The synthesized descriptor carries the same HF repo id as the picker.
        assert_eq!(descriptor.model_code, custom.model_code);
        assert_eq!(descriptor.dimension, custom.dimension);
        assert_eq!(descriptor.license_label, None);
        assert_eq!(descriptor.approx_download_bytes, 0);
        assert_eq!(descriptor.max_tokens, CUSTOM_MODEL_DEFAULT_MAX_TOKENS);

        // The file layout is complete: the ONNX graph + its external data + the four
        // root tokenizer files, all present in required_files.
        assert_eq!(descriptor.expected_layout.onnx_relative_path, custom.onnx_relative_path);
        for external in &custom.external_data_files {
            assert!(
                descriptor.expected_layout.required_files.contains(external),
                "external-data file {external} must be in the required layout"
            );
        }
        for tokenizer in [
            TOKENIZER_FILE_NAME,
            TOKENIZER_CONFIG_FILE_NAME,
            SPECIAL_TOKENS_MAP_FILE_NAME,
            CONFIG_FILE_NAME,
        ] {
            assert!(
                descriptor
                    .expected_layout
                    .required_files
                    .iter()
                    .any(|file| file == tokenizer),
                "tokenizer file {tokenizer} must be in the required layout"
            );
        }
        assert!(descriptor
            .expected_layout
            .required_files
            .contains(&custom.onnx_relative_path));
    }

    #[test]
    fn descriptor_pooling_is_read_from_fastembed_not_guessed_from_the_id() {
        // The retired pooling-by-id-prefix guess (`if id.starts_with("bge")`)
        // silently mean-pooled CLS-trained models that don't start with "bge".
        // This pins the real fastembed assignment per model: across the guided
        // tiers AND the Custom-pickable CLS models the prefix guess got wrong
        // (mxbai / gte / snowflake-arctic). Slugs are the picker's lowercased last
        // path-segment of the HF model_code.
        let cls_models = [
            "bge-m3",                   // guided tier (BAAI/bge-m3)
            "bge-small-en-v1.5",        // BGE small (CLS in fastembed)
            "mxbai-embed-large-v1",     // mixedbread-ai/mxbai-embed-large-v1
            "gte-base-en-v1.5",         // Alibaba-NLP/gte-base-en-v1.5
            "gte-large-en-v1.5",        // Alibaba-NLP/gte-large-en-v1.5
            "snowflake-arctic-embed-m", // Snowflake/snowflake-arctic-embed-m
        ];
        let mean_models = [
            "nomic-embed-text-v1.5", // guided English tier
            "multilingual-e5-small", // guided Multilingual tier
        ];

        for slug in cls_models {
            let descriptor = resolve_descriptor(FASTEMBED_PROVIDER_ID, slug)
                .unwrap_or_else(|| panic!("{slug} must resolve to a descriptor"));
            assert_eq!(
                descriptor.pooling,
                SemanticSearchPooling::Cls,
                "{slug} is CLS-pooled in fastembed and must not be silently mean-pooled"
            );
        }
        for slug in mean_models {
            let descriptor = resolve_descriptor(FASTEMBED_PROVIDER_ID, slug)
                .unwrap_or_else(|| panic!("{slug} must resolve to a descriptor"));
            assert_eq!(
                descriptor.pooling,
                SemanticSearchPooling::Mean,
                "{slug} is mean-pooled in fastembed"
            );
        }

        // The picker catalog rows carry the same pooling as the resolved descriptors.
        let supported = list_fastembed_supported_models();
        for slug in cls_models {
            if let Some(model) = supported.iter().find(|m| m.model_id == slug) {
                assert_eq!(
                    model.pooling,
                    SemanticSearchPooling::Cls,
                    "supported-model row for {slug} must report CLS pooling"
                );
            }
        }

        // The conversion onto fastembed's own Pooling holds for both arms.
        assert_eq!(fastembed_pooling(SemanticSearchPooling::Cls), Pooling::Cls);
        assert_eq!(fastembed_pooling(SemanticSearchPooling::Mean), Pooling::Mean);
    }

    #[test]
    fn resolve_descriptor_rejects_unknown_and_non_fastembed_provider() {
        assert!(resolve_descriptor(FASTEMBED_PROVIDER_ID, "not-a-real-model").is_none());
        // A non-fastembed provider never synthesizes, even for a real slug.
        assert!(resolve_descriptor("some-other-provider", "nomic-embed-text-v1.5").is_none());
    }

    #[test]
    fn slug_from_model_code_is_unique_across_the_fastembed_catalog() {
        // Custom-model identity is keyed off `slug_from_model_code` (the lowercased
        // last path segment of the HF model_code): the picker shows it as `model_id`
        // and `synthesize_fastembed_descriptor` resolves it back via a first-match
        // `.find(...)`. Two enumerable models sharing a final path segment would
        // collide onto one slug and silently resolve to whichever the catalog lists
        // first. The pinned fastembed catalog has no collision today; this guards the
        // next lockstep fastembed bump — a future catalog adding a colliding repo
        // fails here loudly instead of mis-resolving a Custom pick.
        use std::collections::HashMap;

        let mut by_slug: HashMap<String, Vec<String>> = HashMap::new();
        for info in canonical_fastembed_models() {
            by_slug
                .entry(slug_from_model_code(&info.model_code))
                .or_default()
                .push(info.model_code.clone());
        }

        let collisions: Vec<(String, Vec<String>)> = by_slug
            .into_iter()
            .filter(|(_, codes)| codes.len() > 1)
            .map(|(slug, mut codes)| {
                codes.sort();
                (slug, codes)
            })
            .collect();

        assert!(
            collisions.is_empty(),
            "slug_from_model_code must be unique across the fastembed catalog; \
             colliding slugs (slug -> model_codes): {collisions:?}"
        );
    }

    #[test]
    fn resolve_descriptor_excludes_gated_gemma_from_synthesis() {
        // No gated EmbeddingGemma model should ever resolve via synthesis: every
        // enumerable model the picker offers is non-gated, and a gemma slug must not
        // resolve.
        let gated = TextEmbedding::list_supported_models()
            .into_iter()
            .find(|info| is_gated_model_code(&info.model_code));
        if let Some(info) = gated {
            let slug = slug_from_model_code(&info.model_code);
            assert!(
                resolve_descriptor(FASTEMBED_PROVIDER_ID, &slug).is_none(),
                "gated model {slug} must not resolve"
            );
        }
    }
}
