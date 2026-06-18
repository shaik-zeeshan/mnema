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

/// Maximum chunks handed to one fastembed `embed` call. fastembed pads every row
/// in a batch to the *longest* sequence in that batch (`BatchLongest`), and the
/// default English model has an 8192-token window — so folding a whole backfill
/// batch's chunks into one `embed` let a single long OCR/transcript chunk drag
/// every sibling up to its width, ballooning the transient ONNX tensors and the
/// CPU memory arena `ort` retains at its high-water mark. A small, length-sorted
/// sub-batch keeps the padded width — and the peak per `session.run` — bounded.
/// See [`SemanticSearchEmbedder::embed_chunks_bounded`].
const EMBED_SUB_BATCH_SIZE: usize = 8;

/// Hard cap on the embedding window, regardless of a model's advertised window
/// (nomic = 8192). This is the **primary memory bound** for Semantic Search.
///
/// `ort`'s `memory_pattern` + CPU arena pre-allocate and *retain* one large
/// contiguous buffer per distinct input shape `(batch, padded_seq_len)`. The
/// sequence dim scales with the model window, so a single long anchor at 8192
/// mints a multi-GB arena — and that memory lives in `ort`'s **process-global**
/// allocator, so dropping/reloading the session does not return it to the OS
/// (measured: a long-text batch sat at ~9.9 GB at 8192, flat ~1.7 GB at 256,
/// and the footprint never fell on session drop). fastembed 5.17.2 only disables
/// `memory_pattern` on the DirectML path and exposes no override, so we bound the
/// *shape* instead: capping the window bounds every tensor `ort` can allocate.
///
/// 256 keeps the memory floor low (measured: a worst-case all-long-text backfill
/// holds flat at ~1.7 GB, vs ~2.5 GB at 512 and 9.9 GB-and-climbing at 8192).
/// Text longer than this is split into capped chunks and mean-pooled — the same
/// overflow path as before, just a smaller window — so short anchors (the vast
/// majority of OCR/transcript anchors) embed identically and only long ones are
/// chunked more finely. Raise toward 512 (the standard sentence window the e5
/// tier uses) if long-passage fidelity matters more than the lower floor.
const MAX_EMBED_WINDOW_TOKENS: usize = 256;

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
    ///
    /// `intra_threads` caps the ONNX intra-op thread pool for this session.
    /// `Some(n)` pins it to `n`; `None` leaves fastembed's default, which is to
    /// fan a single embedding across **every** CPU core (the source of the
    /// many-core CPU spikes during backfill). Callers that embed in the
    /// background pass a small cap; tests that don't care pass `None`.
    pub fn load_from_dir(
        model_dir: impl AsRef<Path>,
        max_tokens: usize,
        pooling: Pooling,
        layout: &InstalledModelLayout,
        output_key: Option<OutputKey>,
        intra_threads: Option<usize>,
    ) -> Result<Self, EmbeddingError> {
        let model_dir = model_dir.as_ref();
        // Memory bound: clamp the window before it reaches BOTH the fastembed
        // session (`with_max_length`) and the overflow-split tokenizer (both read
        // the value stored in `self.max_tokens`). This is the load-bearing fix for
        // the embed RSS leak — see [`MAX_EMBED_WINDOW_TOKENS`].
        let max_tokens = max_tokens.min(MAX_EMBED_WINDOW_TOKENS);
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

        // Register the CPU execution provider explicitly with its memory arena
        // **disabled**. Left to fastembed's default (an empty execution-providers
        // vec), `ort` falls back to its implicit CPU EP, which keeps the arena ON:
        // the arena pre-allocates one large buffer per input shape and, by design,
        // *never returns that memory to the OS* while the process lives — so the
        // resident footprint sits at its high-water mark (~1.1 GB measured) and
        // dropping/reloading the session reclaims none of it. With the arena off,
        // each batch's tensors are malloc'd and freed per `session.run`, so the
        // floor collapses toward the model weights alone (ONNX #11627: ~6 GB →
        // ~217 MB). This complements the window cap ([`MAX_EMBED_WINDOW_TOKENS`]),
        // which bounds the *peak* shape; the arena toggle bounds what's *retained*.
        let mut init_options = InitOptionsUserDefined::new()
            .with_max_length(max_tokens)
            .with_execution_providers(vec![
                ort::ep::CPU::default().with_arena_allocator(false).build()
            ]);
        // Without this, fastembed defaults the ONNX intra-op pool to every CPU
        // core, so one embedding fans across all cores — most of it spin-wait on
        // these small encoders. A cap keeps embedding a good background citizen.
        if let Some(threads) = intra_threads {
            init_options = init_options.with_intra_threads(threads.max(1));
        }
        let embedder = TextEmbedding::try_new_from_user_defined(user_model, init_options)
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
    ///
    /// Delegates to [`embed_texts`](Self::embed_texts) so the single-text query
    /// path and the batched backfill path share one set of semantics (overflow
    /// split, single-chunk passthrough, multi-chunk mean-pool). The extra
    /// `String`/`Vec` allocation for a one-element batch is negligible next to a
    /// model forward pass.
    pub fn embed_text(&mut self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        self.embed_texts(&[text])
            .into_iter()
            .next()
            // `embed_texts` returns exactly one result per input, so a 1-element
            // input always yields a 1-element output; this is unreachable.
            .unwrap_or(Err(EmbeddingError::EmptyEmbedding))
    }

    /// Derive a **Semantic Search Vector** for each input text in one batched
    /// fastembed call — the backfill hot path. Returns exactly one result per
    /// input, in input order.
    ///
    /// Why batch: the backfill worker drains a batch of anchors per pass. Calling
    /// [`embed_text`](Self::embed_text) per anchor runs one ONNX forward pass per
    /// text; folding the whole batch into a single `embedder.embed(...)` cuts the
    /// total CPU-seconds (fewer session entries/exits, better internal batching)
    /// so the backlog drains sooner.
    ///
    /// The catch is overflow splitting: a single text can fan into several
    /// token-window chunks (see [`split_on_overflow`](Self::split_on_overflow)),
    /// so a naive `embed(texts)` would mis-align chunks to texts. Instead we fan
    /// every text's chunks into one flat batch, record each text's chunk count,
    /// run one embed, then [`fan_in_chunk_vectors`] regroups the flat vectors back
    /// into one vector per text (single chunk passes through unchanged — byte-for-
    /// byte parity with `embed_text`'s single-chunk path; multiple chunks
    /// mean-pool).
    pub fn embed_texts(&mut self, texts: &[&str]) -> Vec<Result<Vec<f32>, EmbeddingError>> {
        if texts.is_empty() {
            return Vec::new();
        }

        // Split every text up front. A split failure is recorded as that text's
        // result slot (`Some(Err(..))`) and contributes ZERO chunks to the batch;
        // a success records `None` here and pushes its chunks (tracking the count)
        // so the fan-in can slice the flat vectors back per text. `split_results`
        // is one entry per input text, in order, so the slots interleave back
        // around the fan-in output below.
        let mut split_results: Vec<Option<EmbeddingError>> = Vec::with_capacity(texts.len());
        let mut chunk_counts: Vec<usize> = Vec::new();
        let mut all_chunks: Vec<String> = Vec::new();
        for text in texts {
            match self.split_on_overflow(text) {
                Ok(chunks) => {
                    split_results.push(None);
                    chunk_counts.push(chunks.len());
                    all_chunks.extend(chunks);
                }
                Err(error) => split_results.push(Some(error)),
            }
        }

        // Embed the flat chunk list in bounded, length-sorted sub-batches (see
        // `embed_chunks_bounded`) instead of one wide batch: this caps the peak
        // ONNX tensor shape so one long chunk can't balloon the whole batch's
        // memory. Returns vectors in the same flat order the chunks were pushed,
        // so the fan-in below is unaffected.
        let embed_result = self.embed_chunks_bounded(all_chunks);

        // Fan the flat vectors back into one result per successfully-split text.
        // On an embed error every successfully-split text fails identically; texts
        // that already failed splitting keep their own split error below.
        let mut fanned_in = match embed_result {
            Ok(vectors) => fan_in_chunk_vectors(&chunk_counts, vectors).into_iter(),
            Err(error) => {
                // One `Err` per successfully-split text. `EmbeddingError` is not
                // `Clone`, so the message is rebuilt per slot rather than `vec![..; n]`.
                let message = error.to_string();
                chunk_counts
                    .iter()
                    .map(|_| Err(EmbeddingError::Embed(message.clone())))
                    .collect::<Vec<_>>()
                    .into_iter()
            }
        };

        // Interleave: a split-failure slot keeps its error; every other slot draws
        // the next fan-in result, in the same order the chunk counts were pushed.
        split_results
            .into_iter()
            .map(|split_error| match split_error {
                Some(error) => Err(error),
                None => fanned_in
                    .next()
                    // One fan-in result per successfully-split text by construction.
                    .unwrap_or(Err(EmbeddingError::EmptyEmbedding)),
            })
            .collect()
    }

    /// Embed every chunk in `chunks`, returning one vector per chunk **in input
    /// order**, while bounding the work shape so one long chunk can't blow up the
    /// whole batch (the memory-leak guard).
    ///
    /// fastembed pads each batch to its longest sequence (`BatchLongest`) and the
    /// default English model has an 8192-token window, so handing it a backfill
    /// batch's chunks all at once meant a single long chunk dragged every sibling
    /// up to its width — inflating the transient ONNX tensors and the CPU arena
    /// `ort` keeps at its high-water mark (fastembed 5.17.2 hard-codes ort's
    /// `memory_pattern` ON for the CPU path with no override). Here we sort chunks
    /// by length and embed them in [`EMBED_SUB_BATCH_SIZE`]-wide sub-batches, so
    /// each sub-batch pads close to its own members' width and the peak per
    /// `session.run` is bounded by the sub-batch, not the whole batch. Vectors are
    /// scattered back to input order so the caller's chunk→text fan-in is unchanged.
    fn embed_chunks_bounded(
        &mut self,
        chunks: Vec<String>,
    ) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        let total = chunks.len();
        if total == 0 {
            return Ok(Vec::new());
        }

        // Sort *indices* by chunk byte length (a cheap proxy for token length) so
        // similar-length chunks share a sub-batch and short chunks are never padded
        // up to a long one. `order` is a permutation of `0..total`.
        let mut order: Vec<usize> = (0..total).collect();
        order.sort_unstable_by_key(|&index| chunks[index].len());

        // Take each chunk out as it is placed into a sub-batch so fastembed gets
        // owned `String`s without a clone; `order` visits every index exactly once.
        let mut chunks: Vec<Option<String>> = chunks.into_iter().map(Some).collect();
        let mut out: Vec<Option<Vec<f32>>> = (0..total).map(|_| None).collect();

        for window in order.chunks(EMBED_SUB_BATCH_SIZE) {
            let sub_batch: Vec<String> = window
                .iter()
                .map(|&index| {
                    chunks[index]
                        .take()
                        // `order` is a permutation, so each index is taken once.
                        .expect("each chunk index is embedded exactly once")
                })
                .collect();
            let vectors = self
                .embedder
                .embed(sub_batch, None)
                .map_err(|error| EmbeddingError::Embed(error.to_string()))?;
            if vectors.len() != window.len() {
                return Err(EmbeddingError::Embed(format!(
                    "fastembed returned {} vectors for {} chunks",
                    vectors.len(),
                    window.len()
                )));
            }
            for (&index, vector) in window.iter().zip(vectors) {
                out[index] = Some(vector);
            }
        }

        // Every position is filled because `order` covers `0..total` exactly once.
        Ok(out
            .into_iter()
            .map(|vector| vector.expect("every chunk position embedded"))
            .collect())
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

/// Regroup the flat list of chunk vectors a batched embed produced back into one
/// vector per text, given each (successfully-split) text's chunk count in order.
///
/// This is the pure fan-in half of [`SemanticSearchEmbedder::embed_texts`],
/// factored out so the indexing + per-text pooling is unit-testable without a
/// real ONNX model. `chunk_counts[i]` is text `i`'s number of chunks; the helper
/// walks `vectors` in lockstep, taking each text's contiguous slice. A text with
/// exactly one chunk passes its single vector through unchanged (byte-for-byte
/// parity with `embed_text`'s single-chunk path — fastembed already L2-normalized
/// it, so re-pooling would be a no-op at best and a precision drift at worst); a
/// text with more than one chunk is `mean_pool_l2`'d (→ `EmptyEmbedding` if
/// pooling returns `None`, e.g. a ragged/empty slice).
///
/// The vector total is expected to equal `chunk_counts.sum()`; should the batch
/// under-deliver, a slice that runs past the end yields fewer vectors than
/// `count`, which `mean_pool_l2` rejects (→ `EmptyEmbedding`) rather than panic.
fn fan_in_chunk_vectors(
    chunk_counts: &[usize],
    vectors: Vec<Vec<f32>>,
) -> Vec<Result<Vec<f32>, EmbeddingError>> {
    let mut results = Vec::with_capacity(chunk_counts.len());
    let mut cursor = 0usize;
    for &count in chunk_counts {
        // Clamp the slice end so an under-delivered batch can't index out of
        // bounds; the short slice falls through to the pooling check below.
        let end = (cursor + count).min(vectors.len());
        let slice = &vectors[cursor.min(vectors.len())..end];
        cursor += count;
        if count == 1 && slice.len() == 1 {
            // Single chunk: pass the already-normalized vector through untouched.
            results.push(Ok(slice[0].clone()));
        } else {
            results.push(mean_pool_l2(slice).ok_or(EmbeddingError::EmptyEmbedding));
        }
    }
    results
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
    fn fan_in_regroups_single_and_multi_chunk_texts() {
        // chunk_counts [1,2,1] over 4 flat vectors: text0 = v0 passthrough,
        // text1 = mean_pool_l2(v1,v2), text2 = v3 passthrough.
        let v0 = vec![1.0, 0.0];
        let v1 = vec![3.0, 0.0];
        let v2 = vec![0.0, 3.0];
        let v3 = vec![0.0, 1.0];
        let results = fan_in_chunk_vectors(
            &[1, 2, 1],
            vec![v0.clone(), v1.clone(), v2.clone(), v3.clone()],
        );
        assert_eq!(results.len(), 3);

        // Single-chunk slots pass the original vector through byte-for-byte (NOT
        // re-pooled), matching `embed_text`'s single-chunk path exactly.
        assert_eq!(results[0].as_ref().expect("text0"), &v0);
        assert_eq!(results[2].as_ref().expect("text2"), &v3);

        // The 2-chunk slot is mean_pool_l2(v1,v2): the same value the standalone
        // helper produces, so a batched split text matches an unbatched one.
        let pooled = results[1].as_ref().expect("text1");
        assert_eq!(pooled, &mean_pool_l2(&[v1, v2]).expect("pool"));
    }

    #[test]
    fn fan_in_on_empty_input_is_empty() {
        let results = fan_in_chunk_vectors(&[], Vec::new());
        assert!(results.is_empty());
    }

    #[test]
    fn fan_in_rejects_ragged_or_under_delivered_slices() {
        // A multi-chunk text whose chunk vectors are ragged (different dims) can't
        // be pooled → EmptyEmbedding, while the well-formed neighbor still passes.
        let results = fan_in_chunk_vectors(
            &[2, 1],
            vec![vec![1.0, 2.0], vec![1.0], vec![9.0, 9.0]],
        );
        assert_eq!(results.len(), 2);
        assert!(matches!(results[0], Err(EmbeddingError::EmptyEmbedding)));
        assert_eq!(results[1].as_ref().expect("text1"), &vec![9.0, 9.0]);

        // An under-delivered batch (fewer vectors than chunk_counts.sum()) clamps
        // its slice instead of panicking. A 2-chunk text handed only 1 vector still
        // pools that one survivor (mean_pool over a 1-element slice succeeds) — no
        // panic, no dropped text.
        let short = fan_in_chunk_vectors(&[2], vec![vec![1.0, 0.0]]);
        assert_eq!(short.len(), 1);
        assert!(short[0].is_ok());

        // A FULLY starved text (its whole slice clamped to empty) can't pool and
        // fails cleanly: text0 consumed the only vector, leaving text1 nothing.
        let starved = fan_in_chunk_vectors(&[1, 1], vec![vec![1.0, 0.0]]);
        assert_eq!(starved.len(), 2);
        assert!(starved[0].is_ok());
        assert!(matches!(starved[1], Err(EmbeddingError::EmptyEmbedding)));
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
