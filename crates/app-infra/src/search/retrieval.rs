use sqlx::{QueryBuilder, Sqlite};

use super::types::{NormalizedAppRefinement, NormalizedSearchRefinements};
use super::{MAX_HIT_FETCH_LIMIT, MEANING_SNIPPET_CHAR_BUDGET};
use crate::processing::Frame;
use crate::{AudioSegment, AudioSegmentSourceKind, Result};

const DEFAULT_GROUP_LIMIT: u32 = 5;
const MAX_GROUP_LIMIT: u32 = 50;
const MIN_HIT_FETCH_LIMIT: i64 = 250;
const HIT_FETCH_OVERFETCH_PER_GROUP: i64 = 50;

/// Reciprocal rank fusion constant. The textbook `k = 60` from the TREC RRF
/// paper (Cormack et al. 2009): it damps the contribution of low-ranked tail
/// hits so the top of each list dominates the fused order without one list's
/// long tail swamping the other's head. `1 / (k + rank)` per list, summed.
const RRF_K: f64 = 60.0;

#[derive(Debug, Clone)]
pub(super) struct FrameHit {
    /// `search_documents.id` of this **Search Result Anchor** — the fusion key
    /// for RRF and the `vec0` rowid. One anchor can surface from both the
    /// **Text Search** and the **Semantic Search** candidate lists; fusion
    /// dedups on this id before grouping.
    pub(super) anchor_id: i64,
    pub(super) group_key: String,
    pub(super) frame: Frame,
    pub(super) snippet: String,
    /// The hit's ranking score, **lower is better**. For an FTS-only path this
    /// is BM25; once **Hybrid Search** fuses, it is the negated RRF score so the
    /// existing ASC sort keeps the best hit first.
    pub(super) rank: f64,
    pub(super) app_bundle_id: Option<String>,
    pub(super) app_name: Option<String>,
    pub(super) window_title: Option<String>,
    pub(super) text_source_kind: String,
    pub(super) secret_redaction_count: u32,
    /// True when this anchor entered via the `vec0` KNN with no **Text Search**
    /// term to highlight, so `snippet` is a leading `body_text` excerpt.
    pub(super) found_by_meaning: bool,
}

#[derive(Debug, Clone)]
pub(super) struct AudioHit {
    /// `search_documents.id` — the RRF fusion key and `vec0` rowid (see [`FrameHit::anchor_id`]).
    pub(super) anchor_id: i64,
    pub(super) audio_segment: AudioSegment,
    pub(super) source_kind: AudioSegmentSourceKind,
    pub(super) span_start_ms: u64,
    pub(super) span_end_ms: u64,
    pub(super) snippet: String,
    pub(super) rank: f64,
    pub(super) secret_redaction_count: u32,
    /// True for a meaning-only hit (see [`FrameHit::found_by_meaning`]).
    pub(super) found_by_meaning: bool,
}

pub(super) fn clamp_limit(limit: Option<u32>) -> u32 {
    if limit == Some(0) {
        return 0;
    }
    limit
        .unwrap_or(DEFAULT_GROUP_LIMIT)
        .clamp(1, MAX_GROUP_LIMIT)
}

pub(super) fn hit_fetch_limit(offset: usize, limit: u32) -> i64 {
    let requested_groups = offset
        .saturating_add(limit as usize)
        .saturating_add(1)
        .min((MAX_HIT_FETCH_LIMIT / HIT_FETCH_OVERFETCH_PER_GROUP) as usize);
    ((requested_groups as i64) * HIT_FETCH_OVERFETCH_PER_GROUP)
        .max(MIN_HIT_FETCH_LIMIT)
        .min(MAX_HIT_FETCH_LIMIT)
}

pub(super) fn push_search_refinement_predicates(
    query: &mut QueryBuilder<'_, Sqlite>,
    refinements: &NormalizedSearchRefinements,
) {
    if let Some(range) = &refinements.date_range {
        query.push(" AND julianday(search_documents.absolute_end_at) >= julianday(");
        query.push_bind(range.start_at.clone());
        query.push(") AND julianday(search_documents.absolute_start_at) <= julianday(");
        query.push_bind(range.end_at.clone());
        query.push(")");
    }
    if !refinements.apps.is_empty() {
        // Multiple `app:` operators accumulate with OR semantics: a frame
        // matches when its retained identity matches any of the apps.
        query.push(" AND (");
        for (index, app) in refinements.apps.iter().enumerate() {
            if index > 0 {
                query.push(" OR ");
            }
            match app {
                NormalizedAppRefinement::Any { value, search_key } => {
                    query.push(
                        "(LOWER(TRIM(COALESCE(search_documents.app_bundle_id, ''))) = LOWER(",
                    );
                    query.push_bind(value.clone());
                    query.push(") OR search_documents.app_name_search_key = ");
                    query.push_bind(search_key.clone());
                    query.push(")");
                }
                NormalizedAppRefinement::BundleId { value } => {
                    query
                        .push("LOWER(TRIM(COALESCE(search_documents.app_bundle_id, ''))) = LOWER(");
                    query.push_bind(value.clone());
                    query.push(")");
                }
                NormalizedAppRefinement::AppName { search_key, .. } => {
                    query.push(
                        "(LENGTH(TRIM(COALESCE(search_documents.app_bundle_id, ''))) = 0 \
                          AND search_documents.app_name_search_key = ",
                    );
                    query.push_bind(search_key.clone());
                    query.push(")");
                }
            }
        }
        query.push(")");
    }
    if let Some(window_title) = &refinements.window_title {
        query.push(" AND LOWER(COALESCE(search_documents.window_title, '')) LIKE LOWER(");
        query.push_bind(sqlite_contains_like_pattern(window_title));
        query.push(") ESCAPE '\\'");
    }
    if !refinements.audio_sources.is_empty() {
        query.push(" AND search_documents.source_kind IN (");
        for (index, source) in refinements.audio_sources.iter().enumerate() {
            if index > 0 {
                query.push(", ");
            }
            query.push_bind(source.as_str().to_string());
        }
        query.push(")");
    }
}

fn sqlite_contains_like_pattern(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('%');
    for ch in value.chars() {
        match ch {
            '\\' | '%' | '_' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }
    escaped.push('%');
    escaped
}

/// Fuse a semantic-fetch result into the keyword-only fallback: on `Ok`, return
/// the meaning hits; on `Err`, **log and yield an empty list** so search degrades
/// to keyword-only rather than failing the whole `search_capture`.
///
/// This is the query-side half of the single dimension authority. The dominant
/// failure mode is a `vec0` dimension mismatch — the query embedder emits a
/// vector at the *selected model's* dimension, but the live `search_document_vectors`
/// column only changes when the table is rebuilt; whenever a model switch left
/// the two disagreeing (mid-switch, or permanently after a failed rebuild) the
/// KNN raises a dimension error. ADR 0036 promises "no usable runtime → feature
/// unavailable, keyword-only with no regression"; propagating the error would
/// instead fail keyword results too. So a semantic error here is swallowed to an
/// empty meaning list (the fusion of `[]` semantic with the text hits is exactly
/// the keyword-only ordering), and the failure is surfaced only as a diagnostic.
/// The dimension gate itself lives in the store seam (`knn_in_scope_anchors`),
/// which returns a clean empty candidate set on a mismatch — so a mismatch never
/// reaches this wrapper as an `Err`; only a real DB failure does.
pub(super) fn degrade_to_keyword_only<T>(kind: &str, result: Result<Vec<T>>) -> Vec<T> {
    match result {
        Ok(hits) => hits,
        Err(error) => {
            // Route the degrade through `debug_log!` — the same packaged-app log
            // target the companion query-embed failure path uses on the desktop
            // side — rather than bare stderr, so a persistent semantic-tier
            // outage (a DB failure silently dropping hybrid to keyword-only) is
            // observable even when the packaged app does not capture stderr. It
            // is a degrade signal, never a user-facing error — search proceeds
            // keyword-only.
            capture_runtime::debug_log!(
                "[app-infra][search] semantic {kind} search degraded to keyword-only (semantic fetch failed: {error})"
            );
            Vec::new()
        }
    }
}

/// A leading `body_text` excerpt for a meaning-only **Search Snippet**: the hit
/// matched the query vector but has no **Text Search** term to mark, so we show
/// the start of the captured text rather than an FTS `snippet(...)`. Whitespace
/// is collapsed and the excerpt is char-bounded (never byte-sliced through a
/// multibyte scalar) with an ellipsis when truncated. Redaction is *not* applied
/// here: the `secret_redactions` rollup carried on the result drives the same
/// masking a **Text Search** snippet uses, at the same egress boundary.
pub(super) fn meaning_snippet(body_text: &str) -> String {
    let collapsed = body_text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= MEANING_SNIPPET_CHAR_BUDGET {
        return collapsed;
    }
    let truncated: String = collapsed
        .chars()
        .take(MEANING_SNIPPET_CHAR_BUDGET)
        .collect();
    format!("{}…", truncated.trim_end())
}

/// Re-impose the **Semantic Candidate Set** order (nearest-first) on hydrated
/// hits. The seam returns ordered `anchor_id`s, but the hydration projection
/// (`WHERE id IN (…)`) returns rows in an arbitrary order, so the candidate-set
/// position — the entire rank-only payload **Hybrid Search** RRF fuses on — is
/// restored here from the candidate list, keyed by `anchor_id`. Hits whose anchor
/// somehow fell out between the KNN and the hydration (a delete racing the read)
/// sort to the tail and are harmless.
pub(super) fn order_by_candidate_set<T>(
    mut hits: Vec<T>,
    candidate_set: &[i64],
    key: impl Fn(&T) -> i64,
) -> Vec<T> {
    let position: std::collections::HashMap<i64, usize> = candidate_set
        .iter()
        .enumerate()
        .map(|(index, id)| (*id, index))
        .collect();
    hits.sort_by_key(|hit| position.get(&key(hit)).copied().unwrap_or(usize::MAX));
    hits
}

/// Push the in-scope **Search Result Anchor** rowid sub-select that constrains a
/// `vec0` KNN to the **Search Refinement** scope. This reuses the exact same
/// `push_search_refinement_predicates` `WHERE` that the **Text Search** path
/// builds (date range, app, window-title, source) plus the `anchor_type` and
/// snapshot bound, so the meaning tier scopes identically to the keyword tier.
/// An unrefined query yields the all-anchors-of-this-type set, so the KNN
/// becomes a plain top-k.
pub(super) fn push_in_scope_anchor_rowids(
    query: &mut QueryBuilder<'_, Sqlite>,
    anchor_type: &str,
    snapshot_document_id: i64,
    refinements: &NormalizedSearchRefinements,
) {
    query.push(
        "SELECT search_documents.id FROM search_documents WHERE search_documents.anchor_type = ",
    );
    query.push_bind(anchor_type.to_string());
    query.push(" AND search_documents.id <= ");
    query.push_bind(snapshot_document_id);
    push_search_refinement_predicates(query, refinements);
}

/// Reciprocal rank fusion of the **Text Search** and **Semantic Search** anchor
/// lists into one re-ranked list, **rank-only** (BM25 and vector distance are
/// incomparable scales, so no score is combined — only list positions). Each
/// input list must already be in its own best-first order. The fused score for
/// an anchor is `Σ 1 / (RRF_K + position)` over the lists it appears in; the
/// returned `rank` is the *negated* fused score so the existing ascending sort
/// (lower = better) keeps the strongest hit first.
///
/// Dedup is at the **Search Result Anchor** (`anchor_id`) level: an anchor that
/// surfaced in both lists is emitted once, keeping the **Text Search** row (so a
/// hit that also matched a query term renders its highlighted FTS snippet, not
/// the meaning excerpt) with the fused rank. This fusion runs *before* grouping
/// and pagination, slotting in exactly where the BM25 `rank` sat.
/// RRF-fuse two best-first hit lists into one deduped list, keyed by **Search
/// Result Anchor** id. The frame and audio paths share this identical fusion
/// math; `anchor_id` reads the dedup key and `set_rank` writes the negated fused
/// score, so the one body serves both `FrameHit` and `AudioHit`.
///
/// Keyword-only path: with no meaning tier to fuse, return the **Text Search**
/// list untouched so its raw BM25 `rank` (the grouping tie-break key) is preserved
/// exactly. Overwriting every hit's `rank` with a position-derived RRF score here
/// would change equal-BM25 group tie-break ordering, so skipping fusion keeps the
/// keyword-only path byte-identical to pre-Semantic-Search.
///
/// Dedup prefers the **Text Search** row for an anchor present in both lists, so a
/// keyword-and-meaning hit keeps its highlighted snippet. Both inputs are borrowed
/// and only the deduped hits we keep are cloned — the frame path re-fuses on every
/// pagination page, so cloning the whole inputs per call would be wasteful.
fn rrf_fuse_hits<T: Clone>(
    text_hits: &[T],
    semantic_hits: &[T],
    anchor_id: impl Fn(&T) -> i64,
    set_rank: impl Fn(&mut T, f64),
) -> Vec<T> {
    if semantic_hits.is_empty() {
        return text_hits.to_vec();
    }

    let mut scores: std::collections::HashMap<i64, f64> = std::collections::HashMap::new();
    for (position, hit) in text_hits.iter().enumerate() {
        *scores.entry(anchor_id(hit)).or_insert(0.0) += 1.0 / (RRF_K + position as f64);
    }
    for (position, hit) in semantic_hits.iter().enumerate() {
        *scores.entry(anchor_id(hit)).or_insert(0.0) += 1.0 / (RRF_K + position as f64);
    }

    let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let mut fused = Vec::with_capacity(text_hits.len() + semantic_hits.len());
    for hit in text_hits.iter().chain(semantic_hits.iter()) {
        let id = anchor_id(hit);
        if !seen.insert(id) {
            continue;
        }
        let mut hit = hit.clone();
        // Negate so lower = better, matching the BM25 ASC ordering grouping uses.
        set_rank(&mut hit, -scores.get(&id).copied().unwrap_or(0.0));
        fused.push(hit);
    }
    fused
}

pub(super) fn rrf_fuse_frame_hits(
    text_hits: &[FrameHit],
    semantic_hits: &[FrameHit],
) -> Vec<FrameHit> {
    rrf_fuse_hits(
        text_hits,
        semantic_hits,
        |hit| hit.anchor_id,
        |hit, rank| hit.rank = rank,
    )
}

/// Audio counterpart of [`rrf_fuse_frame_hits`].
pub(super) fn rrf_fuse_audio_hits(
    text_hits: &[AudioHit],
    semantic_hits: &[AudioHit],
) -> Vec<AudioHit> {
    rrf_fuse_hits(
        text_hits,
        semantic_hits,
        |hit| hit.anchor_id,
        |hit, rank| hit.rank = rank,
    )
}
