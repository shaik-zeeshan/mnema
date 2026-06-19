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

/// A loaded **Semantic Search Model** ready to derive **Semantic Search
/// Vectors**. Holds the backend (the raw forward) plus a non-truncating split
/// tokenizer used only to detect and split overflowing text above the backend.
pub struct SemanticSearchEmbedder {
    backend: Box<dyn SemanticSearchBackend>,
    /// Non-truncating tokenizer over the same vocab, for overflow detection and
    /// token-window splitting (the backend's own tokenizer truncates).
    split_tokenizer: Tokenizer,
    max_tokens: usize,
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
        Self::from_backend(model_dir, Box::new(backend), descriptor.max_tokens)
    }

    /// Construct the wrapper over an already-built backend (the seam tests and a
    /// future non-candle backend share). Loads the split tokenizer from
    /// `tokenizer.json` under `model_dir` and clamps the window.
    fn from_backend(
        model_dir: &Path,
        backend: Box<dyn SemanticSearchBackend>,
        descriptor_max_tokens: usize,
    ) -> Result<Self, EmbeddingError> {
        // GPU tensor bound: clamp the window before it sizes the chunk splits.
        let max_tokens = descriptor_max_tokens.min(MAX_EMBED_WINDOW_TOKENS);

        let tokenizer_path = model_dir.join(TOKENIZER_FILE_NAME);
        let tokenizer_bytes =
            std::fs::read(&tokenizer_path).map_err(|source| EmbeddingError::ReadModelFile {
                path: tokenizer_path,
                source,
            })?;
        let split_tokenizer = Tokenizer::from_bytes(tokenizer_bytes)
            .map_err(|error| EmbeddingError::LoadTokenizer(error.to_string()))?;

        Ok(Self {
            backend,
            split_tokenizer,
            max_tokens,
        })
    }

    /// The vector dimension the loaded model produces (delegates to the backend).
    pub fn dimension(&self) -> usize {
        self.backend.dimension()
    }

    /// Derive a single **Semantic Search Vector** (f32) for a UTF-8 string.
    ///
    /// Text within the window is embedded directly; text that overflows is
    /// auto-split into token-window chunks (never silently truncated), each chunk
    /// embedded, and the chunk vectors mean-pooled and L2-normalized into one
    /// vector. Delegates to [`embed_texts`](Self::embed_texts) so the single-text
    /// query path and the batched backfill path share one set of semantics.
    pub fn embed_text(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        self.embed_texts(&[text])
            .into_iter()
            .next()
            .unwrap_or(Err(EmbeddingError::EmptyEmbedding))
    }

    /// Derive a **Semantic Search Vector** for each input text. Returns exactly one
    /// result per input, in input order.
    ///
    /// Each text is split into token-window chunks up front; all chunks are fanned
    /// into one flat list, embedded in bounded length-sorted sub-batches, then
    /// regrouped per text by [`fan_in_chunk_vectors`] (single chunk passes through
    /// unchanged; multiple chunks mean-pool).
    pub fn embed_texts(&self, texts: &[&str]) -> Vec<Result<Vec<f32>, EmbeddingError>> {
        if texts.is_empty() {
            return Vec::new();
        }

        // Split every text up front. A split failure is recorded as that text's
        // result slot and contributes ZERO chunks to the batch; a success records
        // `None` here and pushes its chunks (tracking the count) so the fan-in can
        // slice the flat vectors back per text.
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

        // Embed the flat chunk list in bounded, length-sorted sub-batches so one
        // long chunk can't balloon the whole batch's padded tensor. Returns vectors
        // in the same flat order the chunks were pushed.
        let embed_result = self.embed_chunks_bounded(all_chunks);

        // Fan the flat vectors back into one result per successfully-split text. On
        // an embed error every successfully-split text fails identically; texts that
        // already failed splitting keep their own split error below.
        let mut fanned_in = match embed_result {
            Ok(vectors) => fan_in_chunk_vectors(&chunk_counts, vectors).into_iter(),
            Err(error) => {
                // One `Err` per successfully-split text. `EmbeddingError` is not
                // `Clone`, so the message is rebuilt per slot.
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
                    .unwrap_or(Err(EmbeddingError::EmptyEmbedding)),
            })
            .collect()
    }

    /// Embed every chunk in `chunks`, returning one vector per chunk **in input
    /// order**, while bounding the work shape so one long chunk can't blow up the
    /// whole batch.
    ///
    /// The backend pads each batch to its longest sequence, so handing it a
    /// backfill batch's chunks all at once would let a single long chunk drag every
    /// sibling up to its width. Here chunks are sorted by length and embedded in
    /// [`EMBED_SUB_BATCH_SIZE`]-wide sub-batches, so each sub-batch pads close to
    /// its own members' width and the peak per forward pass is bounded by the
    /// sub-batch. Vectors are scattered back to input order so the caller's
    /// chunk→text fan-in is unchanged.
    fn embed_chunks_bounded(&self, chunks: Vec<String>) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        let total = chunks.len();
        if total == 0 {
            return Ok(Vec::new());
        }

        // Sort *indices* by chunk byte length (a cheap proxy for token length) so
        // similar-length chunks share a sub-batch. `order` is a permutation.
        let mut order: Vec<usize> = (0..total).collect();
        order.sort_unstable_by_key(|&index| chunks[index].len());

        let mut out: Vec<Option<Vec<f32>>> = (0..total).map(|_| None).collect();

        for window in order.chunks(EMBED_SUB_BATCH_SIZE) {
            let sub_batch: Vec<&str> = window.iter().map(|&index| chunks[index].as_str()).collect();
            let vectors = self.backend.embed_batch(&sub_batch)?;
            if vectors.len() != window.len() {
                return Err(EmbeddingError::Embed(format!(
                    "backend returned {} vectors for {} chunks",
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

/// Regroup the flat list of chunk vectors a batched embed produced back into one
/// vector per text, given each (successfully-split) text's chunk count in order.
///
/// `chunk_counts[i]` is text `i`'s number of chunks; the helper walks `vectors` in
/// lockstep, taking each text's contiguous slice. A text with exactly one chunk
/// passes its single vector through unchanged (byte-for-byte parity with
/// `embed_text`'s single-chunk path — the backend already L2-normalized it); a
/// text with more than one chunk is `mean_pool_l2`'d (→ `EmptyEmbedding` if pooling
/// returns `None`, e.g. a ragged/empty slice). An under-delivered batch clamps its
/// slice instead of panicking.
fn fan_in_chunk_vectors(
    chunk_counts: &[usize],
    vectors: Vec<Vec<f32>>,
) -> Vec<Result<Vec<f32>, EmbeddingError>> {
    let mut results = Vec::with_capacity(chunk_counts.len());
    let mut cursor = 0usize;
    for &count in chunk_counts {
        let end = (cursor + count).min(vectors.len());
        let slice = &vectors[cursor.min(vectors.len())..end];
        cursor += count;
        if count == 1 && slice.len() == 1 {
            results.push(Ok(slice[0].clone()));
        } else {
            results.push(mean_pool_l2(slice).ok_or(EmbeddingError::EmptyEmbedding));
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
        let results = fan_in_chunk_vectors(&[2, 1], vec![vec![1.0, 2.0], vec![1.0], vec![9.0, 9.0]]);
        assert_eq!(results.len(), 2);
        assert!(matches!(results[0], Err(EmbeddingError::EmptyEmbedding)));
        assert_eq!(results[1].as_ref().expect("text1"), &vec![9.0, 9.0]);

        // An under-delivered batch (fewer vectors than chunk_counts.sum()) clamps
        // its slice instead of panicking. A 2-chunk text handed only 1 vector still
        // pools that one survivor — no panic, no dropped text.
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
}
