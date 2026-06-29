use sqlx::{Executor, QueryBuilder, Row, Sqlite, SqlitePool};

use super::grouping::{
    group_audio_hits, group_frame_hits, map_audio_hit, map_audio_segment_for_search,
};
use super::types::{NormalizedAppRefinement, NormalizedSearchRefinements};
use super::{
    AudioSearchResult, FrameSearchResult, MAX_HIT_FETCH_LIMIT, MEANING_SNIPPET_CHAR_BUDGET,
    SEMANTIC_KNN_LIMIT,
};
use crate::processing::{map_frame_for_search, Frame};
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

pub(super) async fn fetch_search_document_high_water_mark(pool: &SqlitePool) -> Result<i64> {
    let row =
        sqlx::query("SELECT COALESCE(MAX(id), 0) AS snapshot_document_id FROM search_documents")
            .fetch_one(pool)
            .await?;
    Ok(row.get("snapshot_document_id"))
}

async fn fetch_frame_hits(
    pool: &SqlitePool,
    fts_query: &str,
    snapshot_document_id: i64,
    hit_offset: i64,
    hit_limit: i64,
    refinements: &NormalizedSearchRefinements,
) -> Result<Vec<FrameHit>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT search_documents.id AS document_id, \
                search_documents.group_key, search_documents.app_bundle_id, search_documents.app_name, search_documents.window_title, \
                search_documents.text_source_kind, \
                CASE \
                    WHEN instr(snippet(search_documents_fts, 0, '<mark>', '</mark>', '...', 12), '<mark>') > 0 \
                    THEN snippet(search_documents_fts, 0, '<mark>', '</mark>', '...', 12) \
                    ELSE snippet(search_documents_fts, 1, '<mark>', '</mark>', '...', 12) \
                END AS snippet, \
                bm25(search_documents_fts, 5.0, 1.0) AS rank, \
                frames.id, frames.session_id, frames.file_path, frames.captured_at, frames.width, frames.height, \
                frames.equivalence_hint, frames.equivalence_proof, frames.equivalence_version, \
                frames.equivalence_status, frames.equivalence_error, \
                frame_metadata_snapshots.snapshot_json AS metadata_snapshot_json, \
                frames.created_at, frames.updated_at, \
                COALESCE((\
                    SELECT COUNT(*) FROM secret_redactions \
                    WHERE secret_redactions.anchor_type = 'frame' \
                      AND (\
                        (search_documents.text_source_kind = 'equivalent_reuse' \
                         AND search_documents.processing_result_id IS NOT NULL \
                         AND secret_redactions.processing_result_id = search_documents.processing_result_id) \
                        OR ((search_documents.text_source_kind != 'equivalent_reuse' \
                             OR search_documents.processing_result_id IS NULL) \
                            AND secret_redactions.frame_id = frames.id)\
                      )\
                ), 0) AS secret_redaction_count \
         FROM search_documents_fts \
         JOIN search_documents ON search_documents.id = search_documents_fts.rowid \
         JOIN frames ON frames.id = search_documents.frame_id \
         LEFT JOIN frame_metadata_snapshots ON frame_metadata_snapshots.id = frames.metadata_snapshot_id \
         WHERE search_documents_fts MATCH ",
    );
    query.push_bind(fts_query);
    query.push(
        " \
           AND search_documents.anchor_type = 'frame' \
           AND search_documents.id <= ",
    );
    query.push_bind(snapshot_document_id);
    push_search_refinement_predicates(&mut query, refinements);
    query.push(
        " ORDER BY rank ASC, search_documents.absolute_start_at DESC, search_documents.id DESC LIMIT ",
    );
    query.push_bind(hit_limit);
    query.push(" OFFSET ");
    query.push_bind(hit_offset);

    let rows = query.build().fetch_all(pool).await?;

    rows.into_iter()
        .map(|row| {
            Ok(FrameHit {
                anchor_id: row.get("document_id"),
                group_key: row.get("group_key"),
                app_bundle_id: row.get("app_bundle_id"),
                app_name: row.get("app_name"),
                window_title: row.get("window_title"),
                text_source_kind: row.get("text_source_kind"),
                snippet: row.get("snippet"),
                rank: row.get("rank"),
                secret_redaction_count: u32::try_from(row.get::<i64, _>("secret_redaction_count"))
                    .unwrap_or(u32::MAX),
                // A `MATCH` hit is by definition a **Text Search** match, never
                // meaning-only.
                found_by_meaning: false,
                frame: map_frame_for_search(row)?,
            })
        })
        .collect()
}

pub(super) async fn fetch_grouped_frame_hits(
    pool: &SqlitePool,
    fts_query: &str,
    fts_is_searchable: bool,
    snapshot_document_id: i64,
    offset: usize,
    limit: u32,
    refinements: &NormalizedSearchRefinements,
    query_embedding: Option<&[f32]>,
) -> Result<Vec<FrameSearchResult>> {
    // **Hybrid Search**: when a query vector is present, fetch the meaning-tier
    // candidates once (a bounded top-k `vec0` KNN) and RRF-fuse them into the
    // **Text Search** list *before* grouping/pagination. With no vector the
    // fused list is just the FTS list, so keyword-only behavior is unchanged.
    let semantic_hits = match query_embedding {
        Some(embedding) => degrade_to_keyword_only(
            "frame",
            fetch_semantic_frame_hits(pool, embedding, snapshot_document_id, refinements).await,
        ),
        None => Vec::new(),
    };

    // A meaning-only query (no FTS-searchable body) skips the **Text Search**
    // loop entirely — an empty `MATCH` would error — and groups the semantic-only
    // fused list. `fts_is_searchable` is false only when there is a usable query
    // vector (the empty-everything case short-circuits earlier in `search_capture`).
    if !fts_is_searchable {
        let fused = rrf_fuse_frame_hits(&[], &semantic_hits);
        return Ok(group_frame_hits(&fused));
    }

    let needed_groups = offset.saturating_add(limit as usize).saturating_add(1);
    let mut hit_limit = hit_fetch_limit(offset, limit);
    let mut hit_offset = 0_i64;
    let mut text_hits = Vec::new();
    loop {
        let hits = fetch_frame_hits(
            pool,
            fts_query,
            snapshot_document_id,
            hit_offset,
            hit_limit,
            refinements,
        )
        .await?;
        let hit_count = hits.len() as i64;
        text_hits.extend(hits);
        let fused = rrf_fuse_frame_hits(&text_hits, &semantic_hits);
        let groups = group_frame_hits(&fused);
        // Only **Text Search**-derived groups count toward the pagination
        // termination: the semantic snapshot is fetched whole up front, so the
        // meaning-only groups are already all present and never grow across
        // pages. Counting them toward `needed_groups` could stop the **Text
        // Search** loop a page early, starving a later text hit of its keyword
        // RRF contribution (it would rank ~1 low on its semantic term alone).
        // Drain text until enough text groups exist, exactly as the audio path
        // drains its snapshot before fusing.
        let text_group_count = groups
            .iter()
            .filter(|group| !group.found_by_meaning)
            .count();
        if text_group_count >= needed_groups || hit_count < hit_limit {
            return Ok(groups);
        }
        hit_offset = hit_offset.saturating_add(hit_count);
        hit_limit = MAX_HIT_FETCH_LIMIT;
    }
}

async fn fetch_audio_hits(
    pool: &SqlitePool,
    fts_query: &str,
    snapshot_document_id: i64,
    hit_offset: i64,
    hit_limit: i64,
    refinements: &NormalizedSearchRefinements,
) -> Result<Vec<AudioHit>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT search_documents.id AS document_id, search_documents.group_key, \
                search_documents.span_start_ms, search_documents.span_end_ms, \
                search_documents.absolute_start_at, search_documents.absolute_end_at, \
                CASE \
                    WHEN instr(snippet(search_documents_fts, 0, '<mark>', '</mark>', '...', 12), '<mark>') > 0 \
                    THEN snippet(search_documents_fts, 0, '<mark>', '</mark>', '...', 12) \
                    ELSE snippet(search_documents_fts, 1, '<mark>', '</mark>', '...', 12) \
                END AS snippet, \
                bm25(search_documents_fts, 5.0, 1.0) AS rank, \
                audio_segments.id, audio_segments.source_kind, audio_segments.source_session_id, \
                audio_segments.segment_index, audio_segments.file_path, audio_segments.started_at, \
                audio_segments.ended_at, audio_segments.capture_segment_id, audio_segments.created_at, audio_segments.updated_at, \
                COALESCE((\
                    SELECT COUNT(*) FROM secret_redactions \
                    WHERE secret_redactions.anchor_type = 'audio' \
                      AND secret_redactions.audio_segment_id = audio_segments.id\
                ), 0) AS secret_redaction_count \
         FROM search_documents_fts \
         JOIN search_documents ON search_documents.id = search_documents_fts.rowid \
         JOIN audio_segments ON audio_segments.id = search_documents.audio_segment_id \
         WHERE search_documents_fts MATCH ",
    );
    query.push_bind(fts_query);
    query.push(
        " \
           AND search_documents.anchor_type = 'audio' \
           AND search_documents.id <= ",
    );
    query.push_bind(snapshot_document_id);
    push_search_refinement_predicates(&mut query, refinements);
    query.push(
        " ORDER BY rank ASC, search_documents.absolute_start_at DESC, search_documents.id DESC LIMIT ",
    );
    query.push_bind(hit_limit);
    query.push(" OFFSET ");
    query.push_bind(hit_offset);

    let rows = query.build().fetch_all(pool).await?;

    rows.into_iter().map(map_audio_hit).collect()
}

pub(super) async fn fetch_grouped_audio_hits(
    pool: &SqlitePool,
    fts_query: &str,
    fts_is_searchable: bool,
    snapshot_document_id: i64,
    refinements: &NormalizedSearchRefinements,
    query_embedding: Option<&[f32]>,
) -> Result<Vec<AudioSearchResult>> {
    let semantic_hits = match query_embedding {
        Some(embedding) => degrade_to_keyword_only(
            "audio",
            fetch_semantic_audio_hits(pool, embedding, snapshot_document_id, refinements).await,
        ),
        None => Vec::new(),
    };

    // A meaning-only query (no FTS-searchable body) skips the **Text Search**
    // loop entirely — an empty `MATCH` would error — and groups the semantic-only
    // fused list, exactly as the frame path does.
    if !fts_is_searchable {
        let fused = rrf_fuse_audio_hits(&[], &semantic_hits);
        return group_audio_hits(&fused);
    }

    // Audio grouping is transitive by time adjacency, so a lower-ranked hit can
    // bridge two higher-ranked groups. Drain the snapshot before paginating.
    let mut hit_offset = 0_i64;
    let mut text_hits = Vec::new();
    loop {
        let hits = fetch_audio_hits(
            pool,
            fts_query,
            snapshot_document_id,
            hit_offset,
            MAX_HIT_FETCH_LIMIT,
            refinements,
        )
        .await?;
        let hit_count = hits.len() as i64;
        text_hits.extend(hits);
        if hit_count < MAX_HIT_FETCH_LIMIT {
            // RRF-fuse before grouping, exactly as the frame path does.
            let fused = rrf_fuse_audio_hits(&text_hits, &semantic_hits);
            return group_audio_hits(&fused);
        }
        hit_offset = hit_offset.saturating_add(hit_count);
    }
}

/// Fetch the meaning-tier **Search Result Anchor**s for a frame query. The
/// `vec0` KNN, blob serialization, and the live-dimension gate live in the store
/// seam ([`crate::semantic_search::knn_in_scope_anchors`]); this function passes
/// the in-scope rowid sub-select as the `push_scope` closure (so a refined query
/// never drops an in-scope meaning match a post-filter would have lost — ADR
/// 0036, filter-then-rank), then plainly hydrates the returned **Semantic
/// Candidate Set** into `FrameHit`s with no vec0/KNN/blob of its own, re-imposing
/// the candidate order before RRF. Each hit is `found_by_meaning` with a leading
/// `body_text` excerpt for its snippet.
async fn fetch_semantic_frame_hits(
    pool: &SqlitePool,
    query_embedding: &[f32],
    snapshot_document_id: i64,
    refinements: &NormalizedSearchRefinements,
) -> Result<Vec<FrameHit>> {
    let candidate_set = crate::semantic_search::knn_in_scope_anchors(
        pool,
        query_embedding,
        SEMANTIC_KNN_LIMIT,
        |query| push_in_scope_anchor_rowids(query, "frame", snapshot_document_id, refinements),
    )
    .await?;
    if candidate_set.is_empty() {
        return Ok(Vec::new());
    }

    // Plain hydration — a `search_documents JOIN frames` projection with no
    // vec0/KNN/blob — of the candidate anchors. The candidate set is the single
    // source of order (re-imposed below); the `IN (…)` set returns rows arbitrarily.
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT search_documents.id AS document_id, \
                search_documents.group_key, search_documents.app_bundle_id, search_documents.app_name, search_documents.window_title, \
                search_documents.text_source_kind, search_documents.body_text AS body_text, \
                frames.id, frames.session_id, frames.file_path, frames.captured_at, frames.width, frames.height, \
                frames.equivalence_hint, frames.equivalence_proof, frames.equivalence_version, \
                frames.equivalence_status, frames.equivalence_error, \
                frame_metadata_snapshots.snapshot_json AS metadata_snapshot_json, \
                frames.created_at, frames.updated_at, \
                COALESCE((\
                    SELECT COUNT(*) FROM secret_redactions \
                    WHERE secret_redactions.anchor_type = 'frame' \
                      AND (\
                        (search_documents.text_source_kind = 'equivalent_reuse' \
                         AND search_documents.processing_result_id IS NOT NULL \
                         AND secret_redactions.processing_result_id = search_documents.processing_result_id) \
                        OR ((search_documents.text_source_kind != 'equivalent_reuse' \
                             OR search_documents.processing_result_id IS NULL) \
                            AND secret_redactions.frame_id = frames.id)\
                      )\
                ), 0) AS secret_redaction_count \
         FROM search_documents \
         JOIN frames ON frames.id = search_documents.frame_id \
         LEFT JOIN frame_metadata_snapshots ON frame_metadata_snapshots.id = frames.metadata_snapshot_id \
         WHERE search_documents.id IN (",
    );
    let mut separated = query.separated(", ");
    for anchor_id in &candidate_set {
        separated.push_bind(*anchor_id);
    }
    query.push(")");

    let rows = query.build().fetch_all(pool).await?;

    let mut hits = Vec::with_capacity(rows.len());
    for row in rows {
        let body_text: String = row.get("body_text");
        hits.push(FrameHit {
            anchor_id: row.get("document_id"),
            group_key: row.get("group_key"),
            app_bundle_id: row.get("app_bundle_id"),
            app_name: row.get("app_name"),
            window_title: row.get("window_title"),
            text_source_kind: row.get("text_source_kind"),
            snippet: meaning_snippet(&body_text),
            // Placeholder; RRF overwrites `rank` for every fused hit.
            rank: f64::INFINITY,
            secret_redaction_count: u32::try_from(row.get::<i64, _>("secret_redaction_count"))
                .unwrap_or(u32::MAX),
            found_by_meaning: true,
            frame: map_frame_for_search(row)?,
        });
    }
    Ok(order_by_candidate_set(hits, &candidate_set, |hit| {
        hit.anchor_id
    }))
}

/// Fetch the meaning-tier **Search Result Anchor**s for an audio query — the
/// audio counterpart of [`fetch_semantic_frame_hits`]. The KNN/blob/dimension
/// gate live in the same store seam; this passes the audio in-scope rowid
/// sub-select and plainly hydrates the **Semantic Candidate Set** with a
/// `search_documents JOIN audio_segments` projection, re-imposing candidate order.
async fn fetch_semantic_audio_hits(
    pool: &SqlitePool,
    query_embedding: &[f32],
    snapshot_document_id: i64,
    refinements: &NormalizedSearchRefinements,
) -> Result<Vec<AudioHit>> {
    let candidate_set = crate::semantic_search::knn_in_scope_anchors(
        pool,
        query_embedding,
        SEMANTIC_KNN_LIMIT,
        |query| push_in_scope_anchor_rowids(query, "audio", snapshot_document_id, refinements),
    )
    .await?;
    if candidate_set.is_empty() {
        return Ok(Vec::new());
    }

    // Plain hydration — `search_documents JOIN audio_segments`, no vec0/KNN/blob.
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT search_documents.id AS document_id, search_documents.group_key, \
                search_documents.span_start_ms, search_documents.span_end_ms, \
                search_documents.absolute_start_at, search_documents.absolute_end_at, \
                search_documents.body_text AS body_text, \
                audio_segments.id, audio_segments.source_kind, audio_segments.source_session_id, \
                audio_segments.segment_index, audio_segments.file_path, audio_segments.started_at, \
                audio_segments.ended_at, audio_segments.capture_segment_id, audio_segments.created_at, audio_segments.updated_at, \
                COALESCE((\
                    SELECT COUNT(*) FROM secret_redactions \
                    WHERE secret_redactions.anchor_type = 'audio' \
                      AND secret_redactions.audio_segment_id = audio_segments.id\
                ), 0) AS secret_redaction_count \
         FROM search_documents \
         JOIN audio_segments ON audio_segments.id = search_documents.audio_segment_id \
         WHERE search_documents.id IN (",
    );
    let mut separated = query.separated(", ");
    for anchor_id in &candidate_set {
        separated.push_bind(*anchor_id);
    }
    query.push(")");

    let rows = query.build().fetch_all(pool).await?;

    // Build each `AudioHit` field-by-field, mirroring `fetch_semantic_frame_hits`.
    // The semantic hydration does NOT project `snippet`/`rank` columns (the
    // meaning tier has no FTS term to highlight and no BM25 score), so routing
    // through `map_audio_hit` — which `row.get("snippet")`/`row.get("rank")` —
    // would panic with `ColumnNotFound` on the first row (sqlx `Row::get` =
    // `try_get().unwrap()`).
    let mut hits = Vec::with_capacity(rows.len());
    for row in rows {
        let body_text: String = row.get("body_text");
        let source_kind =
            AudioSegmentSourceKind::from_str(row.get::<String, _>("source_kind").as_str());
        let audio_segment = AudioSegment {
            id: row.get("id"),
            source_kind: source_kind.clone(),
            source_session_id: row.get("source_session_id"),
            segment_index: row.get("segment_index"),
            file_path: row.get("file_path"),
            started_at: row.get("started_at"),
            ended_at: row.get("ended_at"),
            capture_segment_id: row.get("capture_segment_id"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        };
        hits.push(AudioHit {
            anchor_id: row.get("document_id"),
            audio_segment,
            source_kind,
            span_start_ms: row
                .get::<Option<i64>, _>("span_start_ms")
                .unwrap_or(0)
                .max(0) as u64,
            span_end_ms: row.get::<Option<i64>, _>("span_end_ms").unwrap_or(0).max(0) as u64,
            snippet: meaning_snippet(&body_text),
            // Placeholder; RRF overwrites `rank` for every fused hit.
            rank: f64::INFINITY,
            secret_redaction_count: u32::try_from(row.get::<i64, _>("secret_redaction_count"))
                .unwrap_or(u32::MAX),
            found_by_meaning: true,
        });
    }
    Ok(order_by_candidate_set(hits, &candidate_set, |hit| {
        hit.anchor_id
    }))
}

pub(super) async fn get_audio_segment_for_search<'e, E>(
    executor: E,
    audio_segment_id: i64,
) -> Result<Option<AudioSegment>>
where
    E: Executor<'e, Database = Sqlite>,
{
    let row = sqlx::query(
        "SELECT id, source_kind, source_session_id, segment_index, file_path, started_at, ended_at, \
                capture_segment_id, created_at, updated_at \
         FROM audio_segments WHERE id = ?1",
    )
    .bind(audio_segment_id)
    .fetch_optional(executor)
    .await?;

    row.map(map_audio_segment_for_search).transpose()
}
