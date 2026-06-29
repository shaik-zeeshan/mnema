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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::projection::{insert_search_document, timestamp_plus_ms, NewSearchDocument};
    use crate::search::test_support::*;
    use crate::search::{
        SearchAppRefinement, SearchAppRefinementKind, SearchCaptureRefinements,
        SearchCaptureRequest,
    };
    use crate::{AppInfra, NewAudioSegment, NewFrame, ProcessingJobDraft, ProcessingResultDraft};
    use audio_transcription::{TranscriptionMetadata, TranscriptionSegment};

    #[test]
    fn search_ranks_body_matches_ahead_of_context_matches() {
        run_async_test(async {
            let dir = test_dir("body-context-rank");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let context_match = infra
                .insert_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-context-rank-a.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_metadata_snapshot(
                        capture_metadata::FrameMetadataSnapshot {
                            app_bundle_id: Some("com.example.Roadmap".to_string()),
                            app_name: Some("Roadmap".to_string()),
                            window_title: None,
                            window_id: None,
                            browser_url: None,
                            display_id: Some(1),
                            metadata_redaction_reason: None,
                            metadata_redaction_source_id: None,
                        },
                    ),
                )
                .await
                .expect("context frame should insert");
            let body_match = infra
                .insert_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-context-rank-b.jpg",
                        "2026-05-17T10:00:01Z",
                    )
                    .with_metadata_snapshot(
                        capture_metadata::FrameMetadataSnapshot {
                            app_bundle_id: Some("com.example.Notes".to_string()),
                            app_name: Some("Notes".to_string()),
                            window_title: None,
                            window_id: None,
                            browser_url: None,
                            display_id: Some(1),
                            metadata_redaction_reason: None,
                            metadata_redaction_source_id: None,
                        },
                    ),
                )
                .await
                .expect("body frame should insert");

            let context_job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(context_match.id))
                .await
                .expect("context job should enqueue");
            complete_job(
                &infra,
                context_job,
                ProcessingResultDraft::new().with_result_text("ordinary body text"),
            )
            .await;
            let body_job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(body_match.id))
                .await
                .expect("body job should enqueue");
            complete_job(
                &infra,
                body_job,
                ProcessingResultDraft::new().with_result_text("roadmap appears in captured text"),
            )
            .await;

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "roadmap".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.frames.len(), 2);
            assert_eq!(response.frames[0].representative_frame.id, body_match.id);
            assert_eq!(response.frames[1].representative_frame.id, context_match.id);
        });
    }

    #[test]
    fn search_preserves_short_symbol_qualified_terms() {
        run_async_test(async {
            let dir = test_dir("short-symbol-query");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let frame = infra
                .insert_frame(&NewFrame::new(
                    "screen-session",
                    "/tmp/search-short-symbol.jpg",
                    "2026-05-17T10:00:00Z",
                ))
                .await
                .expect("frame should insert");
            let job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
                .await
                .expect("ocr job should enqueue");
            complete_job(
                &infra,
                job,
                ProcessingResultDraft::new().with_result_text("C# compiler notes"),
            )
            .await;

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "C#".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.frames.len(), 1);
            assert_eq!(response.frames[0].representative_frame.id, frame.id);
        });
    }

    #[test]
    fn search_has_more_uses_grouped_frame_results() {
        run_async_test(async {
            let dir = test_dir("grouped-has-more");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let equivalence = crate::FrameEquivalence {
                hint: Some("same-screen".to_string()),
                proof: Some(vec![11; 1024]),
                version: Some(1),
                status: Some(crate::FrameEquivalenceStatus::Ready),
                error: None,
            };
            for index in 0..260 {
                let frame = infra
                    .insert_frame(
                        &NewFrame::new(
                            "screen-session",
                            &format!("/tmp/search-grouped-has-more-{index}.jpg"),
                            &format!("2026-05-17T10:{:02}:{:02}Z", index / 60, index % 60),
                        )
                        .with_equivalence(equivalence.clone()),
                    )
                    .await
                    .expect("frame should insert");
                let job = infra
                    .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
                    .await
                    .expect("ocr job should enqueue");
                complete_job(
                    &infra,
                    job,
                    ProcessingResultDraft::new().with_result_text("collapsed target phrase"),
                )
                .await;
            }

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "collapsed".to_string(),
                    frame_limit: Some(5),
                    frame_offset: Some(0),
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.frames.len(), 1);
            assert!(!response.has_more_frames);
        });
    }

    #[test]
    fn search_has_more_uses_grouped_audio_results() {
        run_async_test(async {
            let dir = test_dir("grouped-audio-has-more");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "mic-session",
                    1,
                    "/tmp/search-grouped-audio.m4a",
                    "2026-05-17T10:00:00Z",
                    "2026-05-17T10:05:00Z",
                ))
                .await
                .expect("segment should insert");
            let spans = (0..260)
                .map(|index| TranscriptionSegment {
                    start_ms: index * 1_000,
                    end_ms: index * 1_000 + 500,
                    text: "collapsed audio target".to_string(),
                    confidence: None,
                })
                .collect::<Vec<_>>();
            let metadata = TranscriptionMetadata {
                provider: "test".to_string(),
                model_id: None,
                language: "en".to_string(),
                segments: spans,
                words: Vec::new(),
                provenance: Default::default(),
            };
            let job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_audio_segment_transcription(
                    segment.id,
                ))
                .await
                .expect("transcription job should enqueue");
            complete_job(
                &infra,
                job,
                ProcessingResultDraft::new()
                    .with_result_text("collapsed audio target")
                    .with_structured_payload_json(
                        serde_json::to_string(&metadata).expect("metadata should serialize"),
                    ),
            )
            .await;

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "collapsed".to_string(),
                    frame_limit: Some(0),
                    frame_offset: None,
                    audio_limit: Some(5),
                    audio_offset: Some(0),
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.audio.len(), 1);
            assert!(!response.has_more_audio);
        });
    }

    #[test]
    fn frame_search_paginates_beyond_hit_fetch_batch_cap() {
        run_async_test(async {
            let dir = test_dir("frame-beyond-hit-cap");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let mut transaction = infra.pool().begin().await.expect("tx should begin");
            for index in 0..5_006_u64 {
                let captured_at = timestamp_plus_ms("2026-05-17T10:00:00Z", index * 1_000)
                    .expect("timestamp should format");
                let insert = sqlx::query(
                    "INSERT INTO frames (session_id, file_path, captured_at) VALUES (?1, ?2, ?3)",
                )
                .bind("screen-session")
                .bind(format!("/tmp/search-deep-frame-{index}.jpg"))
                .bind(&captured_at)
                .execute(&mut *transaction)
                .await
                .expect("frame should insert");
                let frame_id = insert.last_insert_rowid();
                insert_search_document(
                    &mut transaction,
                    NewSearchDocument {
                        anchor_type: "frame",
                        frame_id: Some(frame_id),
                        audio_segment_id: None,
                        processing_result_id: None,
                        span_start_ms: None,
                        span_end_ms: None,
                        absolute_start_at: &captured_at,
                        absolute_end_at: &captured_at,
                        source_kind: None,
                        session_id: "screen-session",
                        app_bundle_id: None,
                        app_name: None,
                        app_name_search_key: None,
                        window_title: None,
                        group_key: &format!("frame:{frame_id}"),
                        text_source_kind: "direct",
                        body_text: "deepframe target",
                        context_text: "",
                    },
                )
                .await
                .expect("search document should insert");
            }
            transaction.commit().await.expect("tx should commit");

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "deepframe".to_string(),
                    frame_limit: Some(5),
                    frame_offset: Some(5_000),
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.frames.len(), 5);
            assert!(response.has_more_frames);
        });
    }

    #[test]
    fn audio_search_paginates_beyond_hit_fetch_batch_cap() {
        run_async_test(async {
            let dir = test_dir("audio-beyond-hit-cap");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "mic-session",
                    1,
                    "/tmp/search-deep-audio.m4a",
                    "2026-05-17T10:00:00Z",
                    "2026-05-17T14:15:00Z",
                ))
                .await
                .expect("segment should insert");
            let mut transaction = infra.pool().begin().await.expect("tx should begin");
            for index in 0..5_006_u64 {
                let start_ms = index * 3_000;
                let end_ms = start_ms + 500;
                let absolute_start_at = timestamp_plus_ms(&segment.started_at, start_ms)
                    .expect("start timestamp should format");
                let absolute_end_at = timestamp_plus_ms(&segment.started_at, end_ms)
                    .expect("end timestamp should format");
                insert_search_document(
                    &mut transaction,
                    NewSearchDocument {
                        anchor_type: "audio",
                        frame_id: None,
                        audio_segment_id: Some(segment.id),
                        processing_result_id: None,
                        span_start_ms: Some(start_ms as i64),
                        span_end_ms: Some(end_ms as i64),
                        absolute_start_at: &absolute_start_at,
                        absolute_end_at: &absolute_end_at,
                        source_kind: Some(segment.source_kind.as_str()),
                        session_id: &segment.source_session_id,
                        app_bundle_id: None,
                        app_name: None,
                        app_name_search_key: None,
                        window_title: None,
                        group_key: &format!("audio:{}:{index}", segment.id),
                        text_source_kind: "direct",
                        body_text: "deepaudio target",
                        context_text: "",
                    },
                )
                .await
                .expect("search document should insert");
            }
            transaction.commit().await.expect("tx should commit");

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "deepaudio".to_string(),
                    frame_limit: Some(0),
                    frame_offset: None,
                    audio_limit: Some(5),
                    audio_offset: Some(5_000),
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.audio.len(), 5);
            assert!(response.has_more_audio);
        });
    }

    #[test]
    fn grouped_audio_search_drains_lower_ranked_bridge_hits_before_paging() {
        run_async_test(async {
            let dir = test_dir("audio-bridge-drain");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "mic-session",
                    1,
                    "/tmp/search-bridged-audio.m4a",
                    "2026-05-17T10:00:00Z",
                    "2026-05-17T10:30:00Z",
                ))
                .await
                .expect("segment should insert");
            let mut transaction = infra.pool().begin().await.expect("tx should begin");

            for (start_ms, end_ms, body_text, context_text) in [
                (1_000_u64, 1_500_u64, "bridgeword bridgeword bridgeword", ""),
                (6_000_u64, 6_500_u64, "bridgeword bridgeword bridgeword", ""),
                (3_500_u64, 4_000_u64, "lower relevance bridge", "bridgeword"),
            ] {
                let absolute_start_at = timestamp_plus_ms(&segment.started_at, start_ms)
                    .expect("start timestamp should format");
                let absolute_end_at = timestamp_plus_ms(&segment.started_at, end_ms)
                    .expect("end timestamp should format");
                insert_search_document(
                    &mut transaction,
                    NewSearchDocument {
                        anchor_type: "audio",
                        frame_id: None,
                        audio_segment_id: Some(segment.id),
                        processing_result_id: None,
                        span_start_ms: Some(start_ms as i64),
                        span_end_ms: Some(end_ms as i64),
                        absolute_start_at: &absolute_start_at,
                        absolute_end_at: &absolute_end_at,
                        source_kind: Some(segment.source_kind.as_str()),
                        session_id: &segment.source_session_id,
                        app_bundle_id: None,
                        app_name: None,
                        app_name_search_key: None,
                        window_title: None,
                        group_key: &format!("audio:{}:{start_ms}", segment.id),
                        text_source_kind: "direct",
                        body_text,
                        context_text,
                    },
                )
                .await
                .expect("bridged search document should insert");
            }

            for index in 0..248_u64 {
                let start_ms = 60_000 + index * 3_000;
                let end_ms = start_ms + 500;
                let absolute_start_at = timestamp_plus_ms(&segment.started_at, start_ms)
                    .expect("start timestamp should format");
                let absolute_end_at = timestamp_plus_ms(&segment.started_at, end_ms)
                    .expect("end timestamp should format");
                insert_search_document(
                    &mut transaction,
                    NewSearchDocument {
                        anchor_type: "audio",
                        frame_id: None,
                        audio_segment_id: Some(segment.id),
                        processing_result_id: None,
                        span_start_ms: Some(start_ms as i64),
                        span_end_ms: Some(end_ms as i64),
                        absolute_start_at: &absolute_start_at,
                        absolute_end_at: &absolute_end_at,
                        source_kind: Some(segment.source_kind.as_str()),
                        session_id: &segment.source_session_id,
                        app_bundle_id: None,
                        app_name: None,
                        app_name_search_key: None,
                        window_title: None,
                        group_key: &format!("audio:{}:filler-{index}", segment.id),
                        text_source_kind: "direct",
                        body_text: "bridgeword",
                        context_text: "",
                    },
                )
                .await
                .expect("filler search document should insert");
            }
            transaction.commit().await.expect("tx should commit");

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "bridgeword".to_string(),
                    frame_limit: Some(0),
                    frame_offset: None,
                    audio_limit: Some(2),
                    audio_offset: Some(0),
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.audio.len(), 2);
            assert_eq!(response.audio[0].span_start_ms, 1_000);
            assert_eq!(response.audio[0].span_end_ms, 6_500);
            assert_eq!(response.audio[0].match_count, 3);
            assert!(response.has_more_audio);
        });
    }

    #[test]
    fn search_pagination_uses_snapshot_document_high_water_mark() {
        run_async_test(async {
            let dir = test_dir("pagination-snapshot");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");

            for (path, captured_at) in [
                ("/tmp/search-page-a.jpg", "2026-05-17T10:00:00Z"),
                ("/tmp/search-page-b.jpg", "2026-05-17T10:00:01Z"),
            ] {
                let frame = infra
                    .insert_frame(&NewFrame::new("screen-session", path, captured_at))
                    .await
                    .expect("frame should insert");
                let job = infra
                    .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
                    .await
                    .expect("ocr job should enqueue");
                complete_job(
                    &infra,
                    job,
                    ProcessingResultDraft::new().with_result_text("snapshot target phrase"),
                )
                .await;
            }

            let first_page = infra
                .search_capture(SearchCaptureRequest {
                    query: "snapshot".to_string(),
                    frame_limit: Some(1),
                    frame_offset: Some(0),
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("first page search should succeed");
            let first_frame_id = first_page.frames[0].representative_frame.id;

            let newer_frame = infra
                .insert_frame(&NewFrame::new(
                    "screen-session",
                    "/tmp/search-page-c.jpg",
                    "2026-05-17T10:00:02Z",
                ))
                .await
                .expect("newer frame should insert");
            let job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(newer_frame.id))
                .await
                .expect("ocr job should enqueue");
            complete_job(
                &infra,
                job,
                ProcessingResultDraft::new().with_result_text("snapshot target phrase"),
            )
            .await;

            let second_page = infra
                .search_capture(SearchCaptureRequest {
                    query: "snapshot".to_string(),
                    frame_limit: Some(1),
                    frame_offset: Some(1),
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: Some(first_page.snapshot_document_id),
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("second page search should succeed");

            assert_eq!(second_page.frames.len(), 1);
            assert_ne!(
                second_page.frames[0].representative_frame.id,
                first_frame_id
            );
            assert_ne!(
                second_page.frames[0].representative_frame.id,
                newer_frame.id
            );
        });
    }

    #[test]
    fn meaning_snippet_collapses_whitespace_and_bounds_length() {
        let short = meaning_snippet("  hello   world  ");
        assert_eq!(short, "hello world", "whitespace collapses, no truncation");

        let long_word = "lorem ".repeat(60);
        let bounded = meaning_snippet(&long_word);
        assert!(
            bounded.chars().count() <= MEANING_SNIPPET_CHAR_BUDGET + 1,
            "excerpt is char-bounded (+1 for the ellipsis)"
        );
        assert!(
            bounded.ends_with('…'),
            "a truncated excerpt ends with an ellipsis"
        );
    }

    #[test]
    fn rrf_fuses_rank_only_and_dedups_anchors_keeping_the_text_row() {
        // A: text-only (FTS rank 0). B: in both lists. C: semantic-only.
        let text = vec![
            frame_hit_for_fusion(1, "alpha snippet", false),
            frame_hit_for_fusion(2, "<mark>bravo</mark> keyword", false),
        ];
        let semantic = vec![
            frame_hit_for_fusion(2, "bravo meaning excerpt", true),
            frame_hit_for_fusion(3, "charlie meaning excerpt", true),
        ];

        let fused = rrf_fuse_frame_hits(&text, &semantic);

        // Three distinct anchors after dedup, none duplicated.
        let ids: Vec<i64> = fused.iter().map(|hit| hit.anchor_id).collect();
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&1) && ids.contains(&2) && ids.contains(&3));

        // Anchor 2 surfaced in both lists, so it keeps the Text Search row (the
        // highlighted snippet), not the meaning excerpt.
        let anchor_two = fused.iter().find(|hit| hit.anchor_id == 2).unwrap();
        assert!(!anchor_two.found_by_meaning);
        assert_eq!(anchor_two.snippet, "<mark>bravo</mark> keyword");

        // RRF is rank-only: anchor 2 (in both lists, both near the head) outscores
        // the single-list anchors, and the fused `rank` is negated so lower wins.
        let rank_two = anchor_two.rank;
        let rank_one = fused.iter().find(|hit| hit.anchor_id == 1).unwrap().rank;
        let rank_three = fused.iter().find(|hit| hit.anchor_id == 3).unwrap().rank;
        assert!(rank_two < rank_one && rank_two < rank_three);
    }

    #[test]
    fn meaning_only_hit_is_fused_with_keyword_hits_and_tagged_found_by_meaning() {
        run_async_test(async {
            let dir = test_dir("hybrid-meaning-fused");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");

            // One anchor contains the literal keyword; one is only related by
            // meaning (no shared term). Both get a vector.
            let keyword_id =
                seed_frame_anchor(&infra, "2026-05-17T10:00:00Z", "quarterly budget keyword").await;
            let meaning_id =
                seed_frame_anchor(&infra, "2026-05-17T10:05:00Z", "fiscal spending plan").await;
            infra
                .semantic_search()
                .store_vector(keyword_id, &seeded_vector(1))
                .await
                .expect("keyword vector stores");
            infra
                .semantic_search()
                .store_vector(meaning_id, &seeded_vector(2))
                .await
                .expect("meaning vector stores");

            // The query vector is closest to the meaning anchor; the FTS term
            // "keyword" only matches the keyword anchor.
            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "keyword".to_string(),
                    frame_limit: Some(10),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: Some(seeded_vector(2)),
                })
                .await
                .expect("search should succeed");

            // Both surface: the keyword hit via Text Search, the related anchor
            // via Semantic Search — fused into one list.
            let frame_ids: Vec<i64> = response
                .frames
                .iter()
                .map(|frame| frame.representative_frame.id)
                .collect();
            assert_eq!(response.frames.len(), 2, "keyword + meaning hits fuse");

            // The keyword anchor matched a term, so it is NOT tagged found_by_meaning.
            let keyword_result = response
                .frames
                .iter()
                .find(|frame| frame.snippet.contains("keyword"))
                .expect("keyword hit present");
            assert!(!keyword_result.found_by_meaning);

            // The meaning-only anchor carries a leading body_text excerpt tagged
            // found_by_meaning (no FTS <mark> to highlight).
            let meaning_result = response
                .frames
                .iter()
                .find(|frame| frame.found_by_meaning)
                .expect("a meaning-only hit is present");
            assert!(meaning_result.snippet.contains("fiscal spending plan"));
            assert!(!meaning_result.snippet.contains("<mark>"));
            assert!(frame_ids
                .iter()
                .any(|&id| id == keyword_result.thumbnail_frame_id));
        });
    }

    #[test]
    fn meaning_only_audio_hit_is_fused_and_tagged_found_by_meaning() {
        run_async_test(async {
            let dir = test_dir("hybrid-audio-meaning");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");

            // One audio anchor contains the literal keyword; one is only related
            // by meaning (no shared term). Both get a vector.
            let keyword_id = seed_audio_anchor(
                &infra,
                "2026-05-17T10:00:00Z",
                "2026-05-17T10:00:02Z",
                "quarterly budget keyword",
            )
            .await;
            let meaning_id = seed_audio_anchor(
                &infra,
                "2026-05-17T10:05:00Z",
                "2026-05-17T10:05:02Z",
                "fiscal spending plan",
            )
            .await;
            infra
                .semantic_search()
                .store_vector(keyword_id, &seeded_vector(1))
                .await
                .expect("keyword vector stores");
            infra
                .semantic_search()
                .store_vector(meaning_id, &seeded_vector(2))
                .await
                .expect("meaning vector stores");

            // The query vector is closest to the meaning anchor; the FTS term
            // "keyword" only matches the keyword anchor. With `audio_limit > 0`
            // this drives `fetch_semantic_audio_hits` — the C1 panic path.
            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "keyword".to_string(),
                    frame_limit: Some(0),
                    frame_offset: None,
                    audio_limit: Some(10),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: Some(seeded_vector(2)),
                })
                .await
                .expect("search should succeed without panicking");

            // Both surface: the keyword hit via Text Search, the related anchor
            // via Semantic Search — fused into one audio list.
            assert_eq!(response.audio.len(), 2, "keyword + meaning audio hits fuse");

            // The keyword anchor matched a term, so it is NOT found_by_meaning.
            let keyword_result = response
                .audio
                .iter()
                .find(|audio| audio.snippet.contains("keyword"))
                .expect("keyword audio hit present");
            assert!(!keyword_result.found_by_meaning);

            // The meaning-only anchor carries a leading body_text excerpt tagged
            // found_by_meaning (no FTS <mark> to highlight).
            let meaning_result = response
                .audio
                .iter()
                .find(|audio| audio.found_by_meaning)
                .expect("a meaning-only audio hit is present");
            assert!(meaning_result.snippet.contains("fiscal spending plan"));
            assert!(!meaning_result.snippet.contains("<mark>"));
        });
    }

    #[test]
    fn a_dimension_mismatched_query_degrades_to_keyword_only_not_an_error() {
        run_async_test(async {
            let dir = test_dir("hybrid-degrade-keyword");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");

            // A keyword anchor with a correctly-sized stored vector (the live table
            // is float[768]).
            let keyword_id =
                seed_frame_anchor(&infra, "2026-05-17T10:00:00Z", "quarterly budget keyword").await;
            infra
                .semantic_search()
                .store_vector(keyword_id, &seeded_vector(1))
                .await
                .expect("keyword vector stores");

            // The query embedding is the WRONG width for the live table (4 dims vs
            // 768) — exactly the shape an embedder reloaded at a new model emits
            // before the table is rebuilt, or permanently after a failed rebuild.
            // The live-dimension guard skips the KNN, and the degrade wrapper fuses
            // an empty semantic list, so the search stays keyword-only.
            let wrong_dimension_query = vec![1.0_f32, 0.0, 0.0, 0.0];
            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "keyword".to_string(),
                    frame_limit: Some(10),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: Some(wrong_dimension_query),
                })
                .await
                .expect("a dimension mismatch degrades to keyword-only, never an Err");

            // The keyword hit still surfaces (degrade, not fail), and nothing is
            // tagged found_by_meaning because the semantic path was skipped.
            assert_eq!(response.frames.len(), 1, "the keyword hit still returns");
            assert!(response.frames[0].snippet.contains("keyword"));
            assert!(
                !response.frames.iter().any(|frame| frame.found_by_meaning),
                "no meaning hits when the semantic fetch is skipped"
            );
        });
    }

    #[test]
    fn refined_semantic_query_pre_filters_to_the_refinement_scope() {
        run_async_test(async {
            let dir = test_dir("hybrid-prefilter");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");

            // The whole point of this test is to distinguish a PRE-filter (rowid IN
            // scope, applied inside the KNN) from a naive post-filter (rank the top-k
            // first, drop out-of-scope rows second). With only a couple of anchors,
            // a post-filter would also pass — the in-scope answer fits inside the
            // top-`k` window either way. So we crowd the KNN window past
            // `SEMANTIC_KNN_LIMIT` with out-of-scope anchors that sit *exactly* on
            // the query vector (L2 distance 0, nearer than anything else). A
            // post-filter's top-`k` would then be entirely out-of-scope rows and the
            // in-scope answer would never survive the post-drop. Only the pre-filter
            // — which excludes those rows before ranking — keeps the in-scope anchor.
            let out_of_scope_count = (SEMANTIC_KNN_LIMIT as usize) + 5;

            // Seed the in-scope answer first, at a vector slightly off the query
            // (distance √2). Under a correct pre-filter it is the *only* candidate;
            // under a post-filter it is rank `out_of_scope_count + 1` and falls
            // outside the top-`k`, so it would be lost. Its OCR text deliberately
            // does NOT contain the FTS query term ("meaning"), so it can only surface
            // via the semantic tier — otherwise FTS would mask a post-filter bug by
            // matching it on keyword regardless of the KNN.
            let in_scope_id = seed_frame_anchor_with_app(
                &infra,
                "2026-05-17T10:00:00Z",
                "kept by the refinement scope",
                "com.example.Keep",
                "Keep",
            )
            .await;
            infra
                .semantic_search()
                .store_vector(in_scope_id, &seeded_vector(6))
                .await
                .expect("in-scope vector stores");

            // Seed > SEMANTIC_KNN_LIMIT out-of-scope anchors, every one of them sitting
            // on the query vector (seed 5, distance 0) so they fully occupy the
            // KNN's top-`k` window. A post-filter would rank these ahead of the
            // in-scope anchor and then discard them all, returning nothing in scope.
            for offset in 0..out_of_scope_count {
                let captured_at = format!("2026-05-17T11:{:02}:{:02}Z", offset / 60, offset % 60);
                let out_scope_id = seed_frame_anchor_with_app(
                    &infra,
                    &captured_at,
                    "dropped by scope",
                    "com.example.Drop",
                    "Drop",
                )
                .await;
                infra
                    .semantic_search()
                    .store_vector(out_scope_id, &seeded_vector(5))
                    .await
                    .expect("out-of-scope vector stores");
            }

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "meaning".to_string(),
                    frame_limit: Some(10),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: Some(SearchCaptureRefinements {
                        date_range: None,
                        apps: vec![SearchAppRefinement {
                            kind: SearchAppRefinementKind::BundleId,
                            value: "com.example.Keep".to_string(),
                            display_name: "Keep".to_string(),
                        }],
                        window_title: None,
                        audio_sources: Vec::new(),
                        screen_source: false,
                    }),
                    // Query vector exactly on every out-of-scope anchor's vector, so
                    // a post-filter's top-`k` is all out-of-scope rows.
                    query_embedding: Some(seeded_vector(5)),
                })
                .await
                .expect("search should succeed");

            // The in-scope anchor survives — it can only be present if the scope was
            // applied as a PRE-filter, since a post-filter's top-`k` window was
            // entirely consumed by the out-of-scope anchors crowding the query vector.
            let ids: Vec<i64> = response
                .frames
                .iter()
                .map(|frame| frame.representative_frame.id)
                .collect();
            assert!(
                !ids.is_empty(),
                "the in-scope meaning answer survives even though > SEMANTIC_KNN_LIMIT \
                 out-of-scope anchors crowd the query vector — only a pre-filter keeps it"
            );
            for frame in &response.frames {
                assert_eq!(
                    frame.app_bundle_id.as_deref(),
                    Some("com.example.Keep"),
                    "no out-of-scope anchor leaks past the pre-filter"
                );
            }
        });
    }

    #[test]
    fn no_vectors_degrades_to_keyword_only_with_no_regression() {
        run_async_test(async {
            let dir = test_dir("hybrid-keyword-only");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");

            seed_frame_anchor(&infra, "2026-05-17T10:00:00Z", "alpha keyword document").await;
            seed_frame_anchor(&infra, "2026-05-17T10:05:00Z", "unrelated meaning text").await;

            // No vectors backfilled, no query embedding: pure Text Search.
            let baseline = infra
                .search_capture(SearchCaptureRequest {
                    query: "keyword".to_string(),
                    frame_limit: Some(10),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            // Exactly the keyword anchor, no found_by_meaning tagging anywhere.
            assert_eq!(baseline.frames.len(), 1);
            assert!(baseline.frames[0].snippet.contains("keyword"));
            assert!(!baseline.frames[0].found_by_meaning);

            // A query embedding present but NO vectors stored: the KNN returns
            // nothing, so the result is identical to the keyword-only baseline.
            let with_embedding = infra
                .search_capture(SearchCaptureRequest {
                    query: "keyword".to_string(),
                    frame_limit: Some(10),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: Some(seeded_vector(9)),
                })
                .await
                .expect("search should succeed");
            assert_eq!(with_embedding.frames.len(), 1);
            assert!(!with_embedding.frames[0].found_by_meaning);
            assert_eq!(
                with_embedding.frames[0].representative_frame.id,
                baseline.frames[0].representative_frame.id,
                "no vectors => identical to keyword-only ranking"
            );
        });
    }
}
