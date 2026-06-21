//! The backend-neutral embedding wrapper: derives a **Semantic Search Vector**
//! from raw text, ABOVE the [`SemanticSearchBackend`] trait.
//!
//! The chunking / length-bucketed sub-batch / cross-chunk fan-in + mean-pool that
//! turns arbitrary `body_text` into one stored vector per anchor lives HERE, so it
//! is shared by every backend (candle today; Ollama / cloud later). The raw model
//! forward — tokenize, run the architecture, pool, L2-normalize — lives BELOW the
//! trait in [`crate::backend::candle`].
//!
//! Overflow handling: the model's tokenizer truncates text past its window. To
//! honor "auto-split on overflow, never silently truncated/dropped", this wrapper
//! counts tokens up front with a non-truncating split tokenizer; when the text
//! overflows the window it is split into token-window chunks, each chunk is
//! embedded by the backend, and the chunk vectors are mean-pooled and
//! L2-normalized into one vector.

use std::path::Path;

use tokenizers::Tokenizer;

use crate::backend::candle::CandleBackend;
use crate::backend::{EmbeddingError, SemanticSearchBackend};
use crate::models::{SemanticSearchModelDescriptor, TOKENIZER_FILE_NAME};

/// Special-token budget reserved per chunk (e.g. `[CLS]`/`[SEP]`) so a split
/// chunk plus its special tokens still fits the model window.
const SPECIAL_TOKEN_HEADROOM: usize = 2;

/// Hard GPU batch ceiling: the most chunks handed to one backend `embed_batch`.
///
/// The backend pads every row in a batch to the *longest* sequence in that batch,
/// so a length-sorted sub-batch keeps the padded `(B, L)` tensor — and the peak
/// per forward pass — bounded. 8 is the measured ceiling: the GPU is
/// compute-bound, and larger padded `(B, 256)` batches blow RAM to ~8.3 GB
/// (ADR 0037). See [`SemanticSearchEmbedder::embed_chunks_bounded`].
const EMBED_SUB_BATCH_SIZE: usize = 8;

/// Hard cap on the embedding window, regardless of a model's advertised window
/// (nomic = 8192). This bounds the padded `(B, 256)` GPU tensor the backend
/// allocates per forward pass.
///
/// This is **not** the retired ORT CPU-arena leak (that runtime is gone with
/// fastembed/`ort`). candle has no process-global arena: each forward pass's
/// tensors are allocated and freed normally. The cap survives as a **GPU tensor
/// bound** — capping the sequence dim bounds every padded `(B, L)` tensor the
/// backend mints, which together with [`EMBED_SUB_BATCH_SIZE`] keeps the per-pass
/// footprint tight. Text longer than this is split into capped chunks and
/// mean-pooled (the overflow path below), so short anchors embed identically and
/// only long ones are chunked more finely.
const MAX_EMBED_WINDOW_TOKENS: usize = 256;

/// Whether a text is being embedded as a search **query** or a stored
/// **document** (anchor body). Some models ship asymmetric input prompts — a
/// per-side instruction string prepended before the model forward — so the same
/// text yields a query-side vs document-side vector. The embedder selects the
/// prompt by this kind (see [`SemanticSearchEmbedder::embed_texts`]); a model
/// with no prompt for the side embeds the bare text identically for both.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EmbedKind {
    /// The text is a search query (uses the descriptor's `query_prompt`).
    Query,
    /// The text is a stored document/anchor body (uses `document_prompt`).
    Document,
}

/// A loaded **Semantic Search Model** ready to derive **Semantic Search
/// Vectors**. Holds the backend (the raw forward) plus a non-truncating split
/// tokenizer used only to detect and split overflowing text above the backend.
pub struct SemanticSearchEmbedder {
    backend: Box<dyn SemanticSearchBackend>,
    /// Non-truncating tokenizer over the same vocab, for overflow detection and
    /// token-window splitting (the backend's own tokenizer truncates).
    split_tokenizer: Tokenizer,
    max_tokens: usize,
    /// Instruction prepended to a **query** before the backend forward (e.g.
    /// `"query: "`), or `None`/empty for a model that embeds bare query text.
    query_prompt: Option<String>,
    /// Instruction prepended to a **document/anchor body** before the backend
    /// forward, or `None`/empty for a model that embeds bare document text.
    document_prompt: Option<String>,
    /// Matryoshka stored width: when `Some(d)`, each native backend vector is
    /// truncated to its first `d` elements and L2-renormalized above the trait
    /// (the backend still produces native-width vectors). `None` ⇒ pass through.
    mrl_truncate_dim: Option<usize>,
    /// The vector width this model STORES — the MRL-truncated width when
    /// truncating, else the native width. The backend's `dimension()` is the
    /// native produced width, which differs from this when `mrl_truncate_dim` is
    /// set, so [`Self::dimension`] reports this stored width instead.
    stored_dimension: usize,
}

impl SemanticSearchEmbedder {
    /// Load a Semantic Search Model from a
    /// `semantic_search_models/{provider}/{model_id}/` directory and its catalog
    /// descriptor.
    ///
    /// Loads the non-truncating split tokenizer from `tokenizer.json`, constructs
    /// the candle [`CandleBackend`] from the descriptor + model dir (which dispatches
    /// the architecture, picks the device, and loads its own forward-pass
    /// tokenizer), and clamps the embedding window to
    /// `descriptor.max_tokens.min(256)` — the GPU tensor bound
    /// ([`MAX_EMBED_WINDOW_TOKENS`]).
    ///
    /// This signature replaces the old fastembed `load_from_dir(model_dir,
    /// max_tokens, pooling, layout, output_key, intra_threads)`: the architecture,
    /// pooling, dimension, window, and layout now all come from the one
    /// `descriptor`, and there is no ONNX thread cap. Wave 2 adapts the desktop
    /// call site to this shape.
    pub fn load_from_dir(
        model_dir: impl AsRef<Path>,
        descriptor: &SemanticSearchModelDescriptor,
    ) -> Result<Self, EmbeddingError> {
        let model_dir = model_dir.as_ref();
        let backend = CandleBackend::load_from_dir(model_dir, descriptor)?;
        Self::from_backend(model_dir, Box::new(backend), descriptor)
    }

    /// Construct the wrapper over an already-built backend (the seam tests and a
    /// future non-candle backend share). Loads the split tokenizer from
    /// `tokenizer.json` under `model_dir`, clamps the window, and pulls the
    /// per-model prompts, MRL stored width, and stored dimension straight from the
    /// descriptor (the backend stays prompt-/MRL-agnostic; those live above it).
    fn from_backend(
        model_dir: &Path,
        backend: Box<dyn SemanticSearchBackend>,
        descriptor: &SemanticSearchModelDescriptor,
    ) -> Result<Self, EmbeddingError> {
        // GPU tensor bound: clamp the window before it sizes the chunk splits.
        let max_tokens = descriptor.max_tokens.min(MAX_EMBED_WINDOW_TOKENS);

        let tokenizer_path = model_dir.join(TOKENIZER_FILE_NAME);
        let tokenizer_bytes =
            std::fs::read(&tokenizer_path).map_err(|source| EmbeddingError::ReadModelFile {
                path: tokenizer_path,
                source,
            })?;
        let mut split_tokenizer = Tokenizer::from_bytes(tokenizer_bytes)
            .map_err(|error| EmbeddingError::LoadTokenizer(error.to_string()))?;
        // Explicitly disable truncation regardless of what `tokenizer.json` declares:
        // overflow is handled by this wrapper's own token-window chunking
        // (`split_on_overflow`), so the tokenizer must never silently truncate the
        // up-front token count it produces.
        split_tokenizer
            .with_truncation(None)
            .map_err(|error| EmbeddingError::LoadTokenizer(error.to_string()))?;

        Ok(Self {
            backend,
            split_tokenizer,
            max_tokens,
            query_prompt: descriptor.query_prompt.clone(),
            document_prompt: descriptor.document_prompt.clone(),
            mrl_truncate_dim: descriptor.mrl_truncate_dim,
            stored_dimension: descriptor.dimension,
        })
    }

    /// The vector dimension the loaded model STORES — the MRL-truncated width when
    /// truncating, else the native width. This is the descriptor's `dimension`,
    /// NOT `backend.dimension()` (the native produced width): the two differ when
    /// `mrl_truncate_dim` is set, and storage/query both index the stored width.
    pub fn dimension(&self) -> usize {
        self.stored_dimension
    }

    /// Derive a single **Semantic Search Vector** (f32) for a UTF-8 string,
    /// embedded as `kind` (query vs document — selects the per-model input prompt).
    ///
    /// Text within the window is embedded directly; text that overflows is
    /// auto-split into token-window chunks (never silently truncated), each chunk
    /// embedded, and the chunk vectors mean-pooled and L2-normalized into one
    /// vector. Delegates to [`embed_texts`](Self::embed_texts) so the single-text
    /// query path and the batched backfill path share one set of semantics.
    pub fn embed_text(&self, text: &str, kind: EmbedKind) -> Result<Vec<f32>, EmbeddingError> {
        self.embed_texts(&[text], kind)
            .into_iter()
            .next()
            .unwrap_or(Err(EmbeddingError::EmptyEmbedding))
    }

    /// Derive a **Semantic Search Vector** for each input text, all embedded as
    /// `kind` (query vs document). Returns exactly one result per input, in input
    /// order.
    ///
    /// Each text is split into token-window chunks up front; all chunks are fanned
    /// into one flat list, embedded in bounded length-sorted sub-batches, then
    /// regrouped per text by [`fan_in_chunk_results`] (single chunk passes through
    /// unchanged; multiple chunks mean-pool). A chunk that fails to embed fails only
    /// its own text, never a sibling in the batch.
    ///
    /// PROMPT APPLICATION (above the backend trait): the per-model `kind` prompt is
    /// prepended to each chunk STRING before the backend forward, and the split
    /// budget is reduced by the prompt's token length so `prompt + chunk + special`
    /// still fits the window. A `None`/empty prompt prepends nothing and reserves
    /// zero budget, so the chunk strings are byte-identical to the no-prompt path.
    pub fn embed_texts(
        &self,
        texts: &[&str],
        kind: EmbedKind,
    ) -> Vec<Result<Vec<f32>, EmbeddingError>> {
        if texts.is_empty() {
            return Vec::new();
        }

        // Select the per-model prompt for this side and measure its token cost up
        // front (once for the whole batch): the prompt reserves that many tokens of
        // the window so each chunk leaves room for `prompt + chunk + special`. A
        // `None`/empty prompt costs zero tokens and prepends nothing (bare-text
        // parity with the no-prompt path).
        let prompt = select_prompt(kind, &self.query_prompt, &self.document_prompt);
        let prompt_token_len = match self.prompt_token_len(prompt) {
            Ok(len) => len,
            // A prompt that fails to tokenize is a load-time misconfiguration, not a
            // per-text fault: fail every input uniformly rather than silently
            // dropping the prompt and producing wrong-side vectors. (`EmbeddingError`
            // isn't `Clone`, so reconstruct the same `Tokenize` error per slot from
            // its message.)
            Err(error) => {
                let message = error.to_string();
                return texts
                    .iter()
                    .map(|_| Err(EmbeddingError::Tokenize(message.clone())))
                    .collect();
            }
        };
        // Reduce the window by the prompt tokens so the split reserves room for the
        // prompt; the split still subtracts `SPECIAL_TOKEN_HEADROOM` internally.
        let chunk_max_tokens = self.max_tokens.saturating_sub(prompt_token_len);

        // Split every text up front. A split failure is recorded as that text's
        // result slot and contributes ZERO chunks to the batch; a success records
        // `None` here and pushes its (prompt-prepended) chunks (tracking the count)
        // so the fan-in can slice the flat vectors back per text.
        let mut split_results: Vec<Option<EmbeddingError>> = Vec::with_capacity(texts.len());
        let mut chunk_counts: Vec<usize> = Vec::new();
        let mut all_chunks: Vec<String> = Vec::new();
        for text in texts {
            match self.split_on_overflow(text, chunk_max_tokens) {
                Ok(chunks) => {
                    split_results.push(None);
                    chunk_counts.push(chunks.len());
                    // Prepend the per-model prompt STRING to each chunk before it
                    // reaches the backend. `prepend_prompt` returns the chunk
                    // unchanged for a `None`/empty prompt, so the bare-text path is
                    // byte-identical to today.
                    all_chunks.extend(chunks.into_iter().map(|chunk| prepend_prompt(prompt, chunk)));
                }
                Err(error) => split_results.push(Some(error)),
            }
        }

        // Embed the flat chunk list in bounded, length-sorted sub-batches so one
        // long chunk can't balloon the whole batch's padded tensor. Returns one
        // result PER CHUNK in the same flat order the chunks were pushed: a chunk
        // that fails to embed fails ONLY itself, so one poison chunk never fails its
        // batch-mates (data-integrity: a single bad/oversized input must not
        // quarantine a whole newest-first window of healthy anchors). A sub-batch
        // backend error is retried chunk-by-chunk to localize the fault — a
        // transient whole-batch fault (e.g. a padding-driven GPU OOM) typically
        // succeeds at the smaller per-chunk shape, while a genuine poison chunk
        // still fails alone.
        let chunk_results = self.embed_chunks_bounded(all_chunks);

        // Fan the flat per-chunk results back into one result per successfully-split
        // text: a text fails only if one of ITS OWN chunks failed, independent of
        // sibling texts in the batch. Texts that already failed splitting keep their
        // own split error below.
        let mut fanned_in = fan_in_chunk_results(&chunk_counts, chunk_results).into_iter();

        // Interleave: a split-failure slot keeps its error; every other slot draws
        // the next fan-in result, in the same order the chunk counts were pushed.
        split_results
            .into_iter()
            .map(|split_error| match split_error {
                Some(error) => Err(error),
                None => fanned_in
                    .next()
                    .unwrap_or(Err(EmbeddingError::EmptyEmbedding)),
            })
            .collect()
    }

    /// Embed every chunk in `chunks`, returning one **result** per chunk **in input
    /// order**, while bounding the work shape so one long chunk can't blow up the
    /// whole batch AND a single failing chunk fails only itself.
    ///
    /// The backend pads each batch to its longest sequence, so handing it a
    /// backfill batch's chunks all at once would let a single long chunk drag every
    /// sibling up to its width. Here chunks are sorted by length and embedded in
    /// [`EMBED_SUB_BATCH_SIZE`]-wide sub-batches, so each sub-batch pads close to
    /// its own members' width and the peak per forward pass is bounded by the
    /// sub-batch. Results are scattered back to input order so the caller's
    /// chunk→text fan-in is unchanged.
    ///
    /// Per-chunk fault isolation (data-integrity): if a whole sub-batch
    /// `embed_batch` errors, each chunk in that sub-batch is retried ALONE so the
    /// error is localized — a transient padding-driven fault usually succeeds at the
    /// smaller single-chunk shape, while a genuinely-poison chunk fails by itself
    /// and only its own slot carries the `Err`. This keeps one bad input from
    /// failing its healthy batch-mates.
    fn embed_chunks_bounded(&self, chunks: Vec<String>) -> Vec<Result<Vec<f32>, EmbeddingError>> {
        let total = chunks.len();
        if total == 0 {
            return Vec::new();
        }

        // Sort *indices* by chunk byte length (a cheap proxy for token length) so
        // similar-length chunks share a sub-batch. `order` is a permutation.
        let mut order: Vec<usize> = (0..total).collect();
        order.sort_unstable_by_key(|&index| chunks[index].len());

        let mut out: Vec<Option<Result<Vec<f32>, EmbeddingError>>> =
            (0..total).map(|_| None).collect();

        for window in order.chunks(EMBED_SUB_BATCH_SIZE) {
            let sub_batch: Vec<&str> = window.iter().map(|&index| chunks[index].as_str()).collect();
            match self.backend.embed_batch(&sub_batch) {
                Ok(vectors) if vectors.len() == window.len() => {
                    for (&index, vector) in window.iter().zip(vectors) {
                        // MRL: truncate the native-width vector to the stored width
                        // and L2-renormalize BEFORE the cross-chunk fan-in, so the
                        // pool and the single-chunk passthrough both operate on the
                        // stored-width unit vector. A `None` mrl passes through.
                        out[index] = Some(Ok(self.apply_mrl(vector)));
                    }
                }
                // A count mismatch or a backend error for the whole sub-batch:
                // retry each chunk alone to localize the fault to the actual
                // offending chunk(s), so a transient batch fault recovers and only a
                // true poison chunk ends up `Err`.
                Ok(_) | Err(_) => {
                    for &index in window {
                        out[index] = Some(self.embed_single_chunk(&chunks[index]));
                    }
                }
            }
        }

        // Every position is filled because `order` covers `0..total` exactly once.
        out.into_iter()
            .map(|result| result.expect("every chunk position embedded"))
            .collect()
    }

    /// Embed a single chunk on its own (the per-chunk fallback path): a one-element
    /// `embed_batch` whose result is unwrapped to exactly one vector, or an `Err` for
    /// just this chunk. Used to localize a sub-batch failure to the offending chunk.
    /// Applies MRL truncation to the native vector (matching the sub-batch path) so
    /// the single-chunk passthrough carries the stored-width unit vector.
    fn embed_single_chunk(&self, chunk: &str) -> Result<Vec<f32>, EmbeddingError> {
        let vectors = self.backend.embed_batch(&[chunk])?;
        vectors
            .into_iter()
            .next()
            .map(|vector| self.apply_mrl(vector))
            .ok_or(EmbeddingError::EmptyEmbedding)
    }

    /// Apply Matryoshka truncation to one native backend vector: when
    /// `mrl_truncate_dim == Some(d)`, truncate to the first `d` elements and
    /// L2-renormalize; otherwise return the native vector unchanged. The backend
    /// only knows the native width — this is the one place the stored width is cut.
    fn apply_mrl(&self, vector: Vec<f32>) -> Vec<f32> {
        match self.mrl_truncate_dim {
            Some(d) => truncate_and_renormalize(&vector, d),
            None => vector,
        }
    }

    /// Split `text` into chunks that each fit `max_tokens` (already reduced by the
    /// prompt's token cost by the caller, so `prompt + chunk + special` fits the
    /// window). Returns a single-element vec when the text already fits.
    fn split_on_overflow(
        &self,
        text: &str,
        max_tokens: usize,
    ) -> Result<Vec<String>, EmbeddingError> {
        split_text_on_token_overflow(&self.split_tokenizer, text, max_tokens)
    }

    /// Token length of `prompt` via the non-truncating split tokenizer (no special
    /// tokens added), so the budget the split reserves matches the actual prompt
    /// tokens the backend prepends. A `None`/empty prompt costs zero tokens.
    fn prompt_token_len(&self, prompt: Option<&str>) -> Result<usize, EmbeddingError> {
        match prompt {
            Some(prompt) if !prompt.is_empty() => {
                let encoding = self
                    .split_tokenizer
                    .encode(prompt, false)
                    .map_err(|error| EmbeddingError::Tokenize(error.to_string()))?;
                Ok(encoding.get_ids().len())
            }
            _ => Ok(0),
        }
    }
}

/// Split `text` so each chunk fits `max_tokens` (minus special-token headroom),
/// mapping token windows back to char spans of the original text so no content is
/// dropped at a chunk boundary. Returns a single-element vec when the text already
/// fits the window.
fn split_text_on_token_overflow(
    tokenizer: &Tokenizer,
    text: &str,
    max_tokens: usize,
) -> Result<Vec<String>, EmbeddingError> {
    // Encode without adding special tokens so the count reflects the raw content
    // tokens; the backend adds its own special tokens at embed time.
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
        // Snap the token byte offsets to valid UTF-8 char boundaries before slicing:
        // a `text.get(a..b)` whose `a`/`b` land mid-codepoint returns `None` and would
        // silently drop the whole chunk's tokens. Floor the start down and ceil the
        // end up to the nearest boundary so the slice always succeeds and no content
        // is lost (worst case it widens the chunk by a few bytes, never narrows it).
        let char_start = floor_char_boundary(text, offsets[start_token].0);
        let char_end = ceil_char_boundary(text, offsets[end_token - 1].1);
        let slice = &text[char_start..char_end];
        if !slice.trim().is_empty() {
            chunks.push(slice.to_string());
        }
        start_token = end_token;
    }

    if chunks.is_empty() {
        chunks.push(text.to_string());
    }
    Ok(chunks)
}

/// Largest valid char-boundary index `<= index` in `text` (stable equivalent of the
/// unstable `str::floor_char_boundary`). Clamps `index` into range first.
fn floor_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// Smallest valid char-boundary index `>= index` in `text` (stable equivalent of the
/// unstable `str::ceil_char_boundary`). Clamps `index` into range first.
fn ceil_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

/// Select the per-model input prompt for an [`EmbedKind`] (query vs document). A
/// pure free helper so the side→prompt mapping is unit-testable without a loaded
/// model. Returns `None` when the model has no prompt for the side.
fn select_prompt<'a>(
    kind: EmbedKind,
    query_prompt: &'a Option<String>,
    document_prompt: &'a Option<String>,
) -> Option<&'a str> {
    match kind {
        EmbedKind::Query => query_prompt.as_deref(),
        EmbedKind::Document => document_prompt.as_deref(),
    }
}

/// Prepend the per-model `prompt` STRING to a chunk before the backend forward. A
/// `None`/empty prompt returns the chunk unchanged (no allocation difference in
/// content), so the bare-text path is byte-identical to the no-prompt path.
fn prepend_prompt(prompt: Option<&str>, chunk: String) -> String {
    match prompt {
        Some(prompt) if !prompt.is_empty() => {
            let mut prefixed = String::with_capacity(prompt.len() + chunk.len());
            prefixed.push_str(prompt);
            prefixed.push_str(&chunk);
            prefixed
        }
        _ => chunk,
    }
}

/// Matryoshka truncation: keep the first `d` elements of a native embedding and
/// L2-renormalize so the truncated prefix is again a unit vector. A pure helper —
/// the model produces native-width vectors and this is the one place the stored
/// width is cut (Arctic 1024 → 256). `d` is clamped to the vector length so a
/// short vector is renormalized whole rather than over-read.
fn truncate_and_renormalize(v: &[f32], d: usize) -> Vec<f32> {
    let keep = d.min(v.len());
    let mut truncated = v[..keep].to_vec();
    let norm = truncated.iter().map(|value| value * value).sum::<f32>().sqrt();
    let epsilon = 1e-12f32;
    for value in truncated.iter_mut() {
        *value /= norm + epsilon;
    }
    truncated
}

/// Regroup the flat list of per-chunk **results** a batched embed produced back
/// into one result per text, given each (successfully-split) text's chunk count in
/// order.
///
/// `chunk_counts[i]` is text `i`'s number of chunks; the helper walks
/// `chunk_results` in lockstep, taking each text's contiguous slice. A text fails
/// (carrying the first chunk error, in chunk order) iff one of ITS OWN chunks
/// failed — independent of sibling texts in the batch, so one poison chunk never
/// fails a healthy neighbor. Otherwise a text with exactly one chunk passes its
/// single vector through unchanged (byte-for-byte parity with `embed_text`'s
/// single-chunk path — the backend already L2-normalized it); a text with more than
/// one chunk is `mean_pool_l2`'d (→ `EmptyEmbedding` if pooling returns `None`, e.g.
/// a ragged/empty slice). An under-delivered batch clamps its slice instead of
/// panicking.
fn fan_in_chunk_results(
    chunk_counts: &[usize],
    chunk_results: Vec<Result<Vec<f32>, EmbeddingError>>,
) -> Vec<Result<Vec<f32>, EmbeddingError>> {
    let mut results = Vec::with_capacity(chunk_counts.len());
    let mut cursor = 0usize;
    for &count in chunk_counts {
        let end = (cursor + count).min(chunk_results.len());
        let slice = &chunk_results[cursor.min(chunk_results.len())..end];
        cursor += count;

        // This text fails iff one of its own chunks failed. Surface that chunk's
        // error (first in chunk order) so the failure is per-text, not per-batch.
        if let Some(error_message) = slice.iter().find_map(|result| match result {
            Err(error) => Some(error.to_string()),
            Ok(_) => None,
        }) {
            results.push(Err(EmbeddingError::Embed(error_message)));
            continue;
        }

        // All of this text's chunks succeeded: collect their vectors and pool.
        let vectors: Vec<Vec<f32>> = slice
            .iter()
            .filter_map(|result| result.as_ref().ok().cloned())
            .collect();
        if count == 1 && vectors.len() == 1 {
            results.push(Ok(vectors[0].clone()));
        } else {
            results.push(mean_pool_l2(&vectors).ok_or(EmbeddingError::EmptyEmbedding));
        }
    }
    results
}

/// Mean-pool a set of chunk vectors and L2-normalize the result, so a split text
/// is comparable to an unsplit one.
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

    /// A tiny whitespace word-level tokenizer (one token per word) so token counts
    /// are predictable and we can assert the split shape exactly.
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
        // chunk_counts [1,2,1] over 4 flat chunk results: text0 = v0 passthrough,
        // text1 = mean_pool_l2(v1,v2), text2 = v3 passthrough.
        let v0 = vec![1.0, 0.0];
        let v1 = vec![3.0, 0.0];
        let v2 = vec![0.0, 3.0];
        let v3 = vec![0.0, 1.0];
        let results = fan_in_chunk_results(
            &[1, 2, 1],
            vec![Ok(v0.clone()), Ok(v1.clone()), Ok(v2.clone()), Ok(v3.clone())],
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
        let results = fan_in_chunk_results(&[], Vec::new());
        assert!(results.is_empty());
    }

    #[test]
    fn fan_in_isolates_a_poison_chunk_to_its_own_text() {
        // Data-integrity: a single failing chunk must fail ONLY its own text, never
        // a healthy neighbor in the same batch. text0 (1 chunk) is fine, text1's one
        // chunk errored, text2 (1 chunk) is fine.
        let results = fan_in_chunk_results(
            &[1, 1, 1],
            vec![
                Ok(vec![1.0, 0.0]),
                Err(EmbeddingError::Embed("poison".to_string())),
                Ok(vec![0.0, 1.0]),
            ],
        );
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].as_ref().expect("text0"), &vec![1.0, 0.0]);
        assert!(
            matches!(results[1], Err(EmbeddingError::Embed(_))),
            "the poison chunk fails only its own text"
        );
        assert_eq!(results[2].as_ref().expect("text2"), &vec![0.0, 1.0]);
    }

    #[test]
    fn fan_in_fails_a_multi_chunk_text_if_any_of_its_chunks_failed() {
        // A 2-chunk text fails if EITHER of its chunks failed, while the single-chunk
        // neighbor still succeeds. text0 = [Ok, Err] => Err; text1 = [Ok] => Ok.
        let results = fan_in_chunk_results(
            &[2, 1],
            vec![
                Ok(vec![1.0, 0.0]),
                Err(EmbeddingError::Embed("poison".to_string())),
                Ok(vec![9.0, 9.0]),
            ],
        );
        assert_eq!(results.len(), 2);
        assert!(matches!(results[0], Err(EmbeddingError::Embed(_))));
        assert_eq!(results[1].as_ref().expect("text1"), &vec![9.0, 9.0]);
    }

    #[test]
    fn fan_in_rejects_ragged_or_under_delivered_slices() {
        // A multi-chunk text whose chunk vectors are ragged (different dims) can't
        // be pooled → EmptyEmbedding, while the well-formed neighbor still passes.
        let results = fan_in_chunk_results(
            &[2, 1],
            vec![Ok(vec![1.0, 2.0]), Ok(vec![1.0]), Ok(vec![9.0, 9.0])],
        );
        assert_eq!(results.len(), 2);
        assert!(matches!(results[0], Err(EmbeddingError::EmptyEmbedding)));
        assert_eq!(results[1].as_ref().expect("text1"), &vec![9.0, 9.0]);

        // An under-delivered batch (fewer results than chunk_counts.sum()) clamps
        // its slice instead of panicking. A 2-chunk text handed only 1 vector still
        // pools that one survivor — no panic, no dropped text.
        let short = fan_in_chunk_results(&[2], vec![Ok(vec![1.0, 0.0])]);
        assert_eq!(short.len(), 1);
        assert!(short[0].is_ok());

        // A FULLY starved text (its whole slice clamped to empty) can't pool and
        // fails cleanly: text0 consumed the only result, leaving text1 nothing.
        let starved = fan_in_chunk_results(&[1, 1], vec![Ok(vec![1.0, 0.0])]);
        assert_eq!(starved.len(), 2);
        assert!(starved[0].is_ok());
        assert!(matches!(starved[1], Err(EmbeddingError::EmptyEmbedding)));
    }

    #[test]
    fn embed_kind_selects_the_matching_side_prompt() {
        // `select_prompt` is the pure side→prompt mapping the embedder uses.
        let query = Some("query: ".to_string());
        let document = Some("passage: ".to_string());
        assert_eq!(select_prompt(EmbedKind::Query, &query, &document), Some("query: "));
        assert_eq!(select_prompt(EmbedKind::Document, &query, &document), Some("passage: "));

        // A model with no prompt for the side yields `None` (bare text).
        let none: Option<String> = None;
        assert_eq!(select_prompt(EmbedKind::Query, &none, &document), None);
        assert_eq!(select_prompt(EmbedKind::Document, &query, &none), None);
    }

    #[test]
    fn prompt_budget_reserves_room_for_prompt_plus_chunk_plus_special() {
        // The window split must leave room for `prompt + chunk + special`. Mirror the
        // embedder's budget math with the predictable whitespace tokenizer: a window
        // of 7 tokens and a 2-token prompt leaves a per-chunk budget of
        // 7 - 2 (prompt) - 2 (SPECIAL_TOKEN_HEADROOM) = 3 content tokens.
        let tokenizer = whitespace_tokenizer();
        let window = 7usize;

        // Prompt "alpha bravo" = 2 tokens (mirrors `prompt_token_len`'s
        // `encode(prompt, false)`).
        let prompt = "alpha bravo";
        let prompt_tokens = tokenizer.encode(prompt, false).expect("encode prompt").get_ids().len();
        assert_eq!(prompt_tokens, 2);

        let chunk_max_tokens = window.saturating_sub(prompt_tokens);
        let text = "alpha bravo charlie delta echo foxtrot";
        let chunks =
            split_text_on_token_overflow(&tokenizer, text, chunk_max_tokens).expect("split");

        // EVERY chunk satisfies prompt_tokens + chunk_tokens + SPECIAL_TOKEN_HEADROOM
        // <= window, so prompt + chunk + special never overflows the model window.
        for chunk in &chunks {
            let chunk_tokens = tokenizer.encode(chunk.as_str(), false).expect("encode").get_ids().len();
            assert!(
                prompt_tokens + chunk_tokens + SPECIAL_TOKEN_HEADROOM <= window,
                "prompt({prompt_tokens}) + chunk({chunk_tokens}) + special({SPECIAL_TOKEN_HEADROOM}) must fit window({window})"
            );
        }
    }

    #[test]
    fn bare_prompt_chunks_are_identical_to_the_no_prompt_path() {
        // A `None`/empty prompt must produce byte-identical chunk strings to the
        // no-prompt path: zero reserved budget, no string prepended.
        let tokenizer = whitespace_tokenizer();
        let window = 4usize; // budget = 4 - 0 (prompt) - 2 (special) = 2 content tokens.
        let text = "alpha bravo charlie delta echo foxtrot";

        // No-prompt path: split at the full window, no prepend.
        let baseline = split_text_on_token_overflow(&tokenizer, text, window).expect("split");

        // Prompt path with a `None` prompt: zero reserved budget + `prepend_prompt`
        // returns each chunk unchanged.
        let none: Option<&str> = None;
        let chunk_max_tokens = window.saturating_sub(0);
        let prompted: Vec<String> = split_text_on_token_overflow(&tokenizer, text, chunk_max_tokens)
            .expect("split")
            .into_iter()
            .map(|chunk| prepend_prompt(none, chunk))
            .collect();
        assert_eq!(prompted, baseline, "None prompt must be byte-identical to no-prompt");

        // An empty-string prompt is treated the same as `None`.
        let empty: Option<&str> = Some("");
        let empty_prompted: Vec<String> =
            split_text_on_token_overflow(&tokenizer, text, window.saturating_sub(0))
                .expect("split")
                .into_iter()
                .map(|chunk| prepend_prompt(empty, chunk))
                .collect();
        assert_eq!(empty_prompted, baseline, "empty prompt must be byte-identical to no-prompt");
    }

    #[test]
    fn prepend_prompt_prefixes_a_nonempty_prompt() {
        assert_eq!(prepend_prompt(Some("query: "), "hello".to_string()), "query: hello");
        // None / empty leave the chunk untouched (bare-text parity).
        assert_eq!(prepend_prompt(None, "hello".to_string()), "hello");
        assert_eq!(prepend_prompt(Some(""), "hello".to_string()), "hello");
    }

    #[test]
    fn truncate_and_renormalize_yields_unit_norm_prefix() {
        // A native vector with a known non-unit prefix. Truncating to d=2 keeps the
        // first 2 elements and renormalizes them to unit length.
        let native = vec![3.0f32, 4.0, 12.0, 0.0];
        let d = 2usize;
        let truncated = truncate_and_renormalize(&native, d);

        // Length == d.
        assert_eq!(truncated.len(), d);

        // Unit-norm.
        let norm = truncated.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "truncated vector must be unit length");

        // Equals the renormalized prefix of the native vector: [3,4] → /5 → [0.6, 0.8].
        let prefix_norm = (native[0] * native[0] + native[1] * native[1]).sqrt();
        let expected = [native[0] / prefix_norm, native[1] / prefix_norm];
        assert!((truncated[0] - expected[0]).abs() < 1e-6);
        assert!((truncated[1] - expected[1]).abs() < 1e-6);

        // `d` larger than the vector clamps to the full length (renormalized whole).
        let whole = truncate_and_renormalize(&native, 99);
        assert_eq!(whole.len(), native.len());
        let whole_norm = whole.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((whole_norm - 1.0).abs() < 1e-5);
    }
}
