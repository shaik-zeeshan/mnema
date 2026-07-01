use std::{future::Future, pin::Pin};

use audio_transcription::TranscriptionMetadata;
use sqlx::{sqlite::SqliteRow, Row, Sqlite, Transaction};
use time::{format_description::well_known::Rfc3339, Duration, OffsetDateTime};

mod dates;

use crate::db::CaptureDb;
use crate::{
    captured_frame_equivalence::CapturedFrameEquivalenceScope,
    processing::{map_frame_for_search, Frame},
    AppInfraError, AudioSegment, ProcessingResult, Result, AUDIO_SEGMENT_SUBJECT_TYPE,
    AUDIO_TRANSCRIPTION_PROCESSOR, FRAME_SUBJECT_TYPE, OCR_PROCESSOR,
};

pub(super) const MAX_HIT_FETCH_LIMIT: i64 = 5_000;

/// Rows projected per write transaction in the startup search backfills. Bounds
/// how long any one backfill commit holds the writer lock so interactive
/// start/stop writes are not blocked behind the whole backlog.
const PROJECTION_COMMIT_BATCH: usize = 200;

/// How many nearest **Semantic Search Vector**s the `vec0` KNN returns per query.
/// This is the meaning-tier candidate budget that RRF fuses with the **Text
/// Search** list; it is intentionally generous (there is no ANN in v1, so the
/// scan is already a filtered brute force) but bounded so an unrefined query
/// over a large index stays a fixed-cost top-k rather than a full table scan.
pub(super) const SEMANTIC_KNN_LIMIT: i64 = 200;

/// Character budget for a meaning-only **Search Snippet**: the leading
/// `body_text` excerpt rendered when a hit matched the query vector but carries
/// no **Text Search** term to highlight. Sized to roughly match the FTS
/// `snippet(...)` 12-token window so a "found by meaning" card reads at the same
/// length as a keyword card.
pub(super) const MEANING_SNIPPET_CHAR_BUDGET: usize = 120;

mod types;

pub use types::{
    AudioSearchResult, FrameSearchResult, SearchAppRefinement, SearchAppRefinementKind,
    SearchCaptureRefinements, SearchCaptureRequest, SearchCaptureResponse, SearchDateRangeOrigin,
    SearchDateRangeRefinement, SearchParseError, SearchableApp,
};
use types::{
    normalize_app_bundle_id_for_search, normalize_app_name_for_search, EquivalentReuseText,
};

mod query;

pub use query::semantic_search_residual_query;
use query::{
    merge_parsed_field_operators, normalize_query, normalize_search_refinements, parse_search_query,
    ParsedQuery,
};

mod equivalent_reuse;
mod grouping;
mod projection;
mod retrieval;
#[cfg(test)]
mod test_support;

pub(crate) use equivalent_reuse::project_equivalent_frame_reuse_in_transaction;
pub(crate) use projection::project_processing_result_direct_in_transaction;
use equivalent_reuse::*;
use grouping::align_audio_results;
use projection::*;
use retrieval::{
    clamp_limit, fetch_grouped_audio_hits, fetch_grouped_frame_hits,
    fetch_search_document_high_water_mark,
};

#[derive(Clone)]
pub struct SearchStore {
    db: CaptureDb,
}

impl SearchStore {
    pub(crate) fn new(db: CaptureDb) -> Self {
        Self { db }
    }

    pub(crate) async fn equivalent_reuse_text_for_frame(
        &self,
        frame_id: i64,
    ) -> Result<Option<EquivalentReuseText>> {
        let row = sqlx::query(
            "SELECT processing_results.result_text AS result_text, \
                    processing_results.subject_type AS source_subject_type, \
                    processing_results.subject_id AS source_subject_id \
             FROM search_documents \
             JOIN processing_results ON processing_results.id = search_documents.processing_result_id \
             WHERE search_documents.anchor_type = 'frame' \
               AND search_documents.frame_id = ?1 \
               AND search_documents.text_source_kind = 'equivalent_reuse' \
               AND search_documents.processing_result_id IS NOT NULL \
               AND LENGTH(TRIM(COALESCE(processing_results.result_text, ''))) > 0 \
             ORDER BY search_documents.id DESC, processing_results.id DESC \
             LIMIT 1",
        )
        .bind(frame_id)
        .fetch_optional(self.db.read())
        .await?;

        Ok(row.map(|row| EquivalentReuseText {
            result_text: row.get("result_text"),
            source_subject_type: row.get("source_subject_type"),
            source_subject_id: row.get("source_subject_id"),
        }))
    }

    pub(crate) async fn backfill_missing_projections(&self) -> Result<()> {
        // Commit projections in batches so the writer lock is released between
        // chunks instead of being held across the whole (potentially large)
        // backlog. A single `BEGIN IMMEDIATE` over every un-indexed row would
        // block interactive start/stop writes for the backfill's full duration —
        // the startup-freeze regression this guards against. The candidate scan
        // runs on the Reader Pool so no write lock is held while reading.
        let rows = sqlx::query(
            "SELECT processing_results.id, processing_results.job_id, \
                    processing_results.subject_type, processing_results.subject_id, \
                    processing_results.processor, processing_results.result_text, \
                    processing_results.structured_payload_json, \
                    processing_results.processor_version, processing_results.redaction_detector_version, \
                    processing_results.redaction_checked_at, processing_results.created_at \
             FROM processing_results \
             JOIN (\
                SELECT subject_type, subject_id, processor, MAX(id) AS id \
                FROM processing_results \
                WHERE (subject_type = ?1 AND processor = ?2) \
                   OR (subject_type = ?3 AND processor = ?4) \
                GROUP BY subject_type, subject_id, processor\
             ) latest_results ON latest_results.id = processing_results.id \
             LEFT JOIN search_documents AS existing_direct \
                    ON existing_direct.processing_result_id = processing_results.id \
                   AND existing_direct.text_source_kind = 'direct' \
             WHERE existing_direct.id IS NULL \
               AND LENGTH(TRIM(COALESCE(processing_results.result_text, ''))) > 0 \
             ORDER BY processing_results.id ASC",
        )
        .bind(FRAME_SUBJECT_TYPE)
        .bind(OCR_PROCESSOR)
        .bind(AUDIO_SEGMENT_SUBJECT_TYPE)
        .bind(AUDIO_TRANSCRIPTION_PROCESSOR)
        .fetch_all(self.db.read())
        .await?;

        let results = rows
            .into_iter()
            .map(map_processing_result_for_search)
            .collect::<Result<Vec<_>>>()?;

        commit_in_batches(&self.db, &results, |transaction, result| {
            Box::pin(project_processing_result_in_transaction(
                transaction,
                result,
            ))
        })
        .await?;

        // The equivalence / app-id backfills are O(all-rows) loops (a costly
        // nested-EXISTS scan plus per-row UPDATEs); each self-manages a read-pool
        // scan + batched commits so none holds the writer lock across its whole
        // backlog. (Sharing one transaction here once held the lock ~11s.)
        backfill_missing_equivalent_reuse_projections(&self.db).await?;
        backfill_missing_app_bundle_id_projection(&self.db).await?;
        backfill_missing_app_name_search_key_projection(&self.db).await?;
        Ok(())
    }

    pub async fn search_capture(
        &self,
        request: SearchCaptureRequest,
    ) -> Result<SearchCaptureResponse> {
        // Opt-in query syntax: field operators are lifted into refinements and
        // body operators shape the residual that drives FTS matching. A query
        // with no operators leaves the residual equal to the plain text, so
        // plain-text search behaves exactly as before.
        let parsed = parse_search_query(&request.query);

        // Merge typed field operators into any caller-supplied refinements:
        // apps/sources accumulate, the date slot overwrites (last write wins).
        let merged_refinements = merge_parsed_field_operators(request.refinements, &parsed);

        let ParsedQuery {
            fts_body,
            residual_query,
            errors: mut parse_errors,
            ..
        } = parsed;

        // The residual body query drives FTS; it is normalized the same way the
        // plain-text path always was.
        let normalized_query = normalize_query(&residual_query);

        let (refinements, applied_refinements) =
            match normalize_search_refinements(Some(merged_refinements))? {
                Ok(refinements) => {
                    let applied = refinements.applied.clone();
                    (Some(refinements), applied)
                }
                Err(mut refinement_errors) => {
                    parse_errors.append(&mut refinement_errors);
                    (None, SearchCaptureRefinements::default())
                }
            };

        // Strict validation: when there are parse errors we suppress results
        // instead of running a misleading search, but still surface the parsed
        // refinements/residual/errors in an Ok response.
        let empty_response = |snapshot_document_id: i64| SearchCaptureResponse {
            normalized_query: normalized_query.clone(),
            snapshot_document_id,
            frames: Vec::new(),
            audio: Vec::new(),
            has_more_frames: false,
            has_more_audio: false,
            applied_refinements: applied_refinements.clone(),
            residual_query: residual_query.clone(),
            parse_errors: parse_errors.clone(),
        };

        let Some(refinements) = refinements else {
            return Ok(empty_response(0));
        };
        if !parse_errors.is_empty() {
            return Ok(empty_response(0));
        }

        // **Hybrid Search** runs only when the caller supplied a non-empty query
        // vector (the desktop layer embedded the query with the installed
        // **Semantic Search Model**). An absent or empty vector — no model, no
        // backfilled vectors — leaves search keyword-only with no regression.
        // Resolved before the short/empty-FTS gates below so a meaning-only query
        // (e.g. a single CJK concept char, or a residual with no FTS-indexable
        // term) can still reach the semantic path on the strength of its vector.
        let query_embedding = request
            .query_embedding
            .as_deref()
            .filter(|embedding| !embedding.is_empty());

        let fts_query = fts_body;
        // The FTS body is too thin to run **Text Search** when it is under two
        // chars or has no indexable term. With a usable query vector the
        // **Semantic Search** tier can still answer, so only short-circuit when
        // there is *also* no embedding to fall back on.
        let fts_is_searchable = normalized_query.chars().count() >= 2 && !fts_query.is_empty();
        if !fts_is_searchable && query_embedding.is_none() {
            return Ok(empty_response(0));
        }

        let frame_limit = clamp_limit(request.frame_limit);
        let frame_offset = request.frame_offset.unwrap_or(0) as usize;
        let audio_limit = clamp_limit(request.audio_limit);
        let audio_offset = request.audio_offset.unwrap_or(0) as usize;
        let snapshot_document_id = match request.snapshot_document_id {
            Some(id) => id.max(0),
            None => fetch_search_document_high_water_mark(self.db.read()).await?,
        };

        let frame_end = frame_offset.saturating_add(frame_limit as usize);
        let audio_end = audio_offset.saturating_add(audio_limit as usize);
        let all_frame_groups = if frame_limit == 0 || !refinements.audio_sources.is_empty() {
            Vec::new()
        } else {
            fetch_grouped_frame_hits(
                self.db.read(),
                &fts_query,
                fts_is_searchable,
                snapshot_document_id,
                frame_offset,
                frame_limit,
                &refinements,
                query_embedding,
            )
            .await?
        };
        let all_audio_groups = if audio_limit == 0
            || !refinements.apps.is_empty()
            || refinements.window_title.is_some()
            || refinements.screen_source
        {
            Vec::new()
        } else {
            fetch_grouped_audio_hits(
                self.db.read(),
                &fts_query,
                fts_is_searchable,
                snapshot_document_id,
                &refinements,
                query_embedding,
            )
            .await?
        };
        let frames = all_frame_groups
            .iter()
            .skip(frame_offset)
            .take(frame_limit as usize)
            .cloned()
            .collect::<Vec<_>>();
        let mut audio = all_audio_groups
            .iter()
            .skip(audio_offset)
            .take(audio_limit as usize)
            .cloned()
            .collect::<Vec<_>>();
        align_audio_results(self.db.read(), &mut audio).await?;

        Ok(SearchCaptureResponse {
            normalized_query,
            snapshot_document_id,
            frames,
            audio,
            has_more_frames: all_frame_groups.len() > frame_end,
            has_more_audio: all_audio_groups.len() > audio_end,
            applied_refinements,
            residual_query,
            parse_errors,
        })
    }

    pub async fn list_searchable_apps(&self) -> Result<Vec<SearchableApp>> {
        // Collapse to one row per stable app identity: bundle id when present,
        // otherwise the normalized name. Grouping by (bundle, name) would emit a
        // separate row whenever captures for the same bundle disagree on
        // `app_name` (missing or relabeled), letting one app crowd out distinct
        // apps in the capped result. Recency is ordered by `julianday()` rather
        // than raw TEXT so mixed RFC3339 offsets compare by real instant (the
        // same convention the date-range filter uses). The display name is the
        // newest non-empty label for the identity, chosen deterministically so
        // casing/label variants don't churn the suggestion list across runs.
        let rows = sqlx::query(
            "WITH frame_apps AS ( \
                SELECT \
                    NULLIF(TRIM(COALESCE(app_bundle_id, '')), '') AS bundle_id, \
                    NULLIF(TRIM(COALESCE(app_name, '')), '') AS name, \
                    julianday(absolute_end_at) AS ended_at, \
                    CASE \
                        WHEN LENGTH(TRIM(COALESCE(app_bundle_id, ''))) > 0 \
                            THEN 'bundle:' || LOWER(TRIM(app_bundle_id)) \
                        ELSE 'name:' || LOWER(TRIM(COALESCE(app_name, ''))) \
                    END AS identity_key \
                FROM search_documents \
                WHERE anchor_type = 'frame' \
                  AND ( \
                       LENGTH(TRIM(COALESCE(app_bundle_id, ''))) > 0 \
                       OR LENGTH(TRIM(COALESCE(app_name, ''))) > 0 \
                  ) \
            ), \
            ranked AS ( \
                SELECT \
                    identity_key, \
                    bundle_id, \
                    name, \
                    MAX(ended_at) OVER (PARTITION BY identity_key) AS group_ended_at, \
                    ROW_NUMBER() OVER ( \
                        PARTITION BY identity_key \
                        ORDER BY (name IS NULL), ended_at DESC, name \
                    ) AS name_rank \
                FROM frame_apps \
            ) \
            SELECT bundle_id, name \
            FROM ranked \
            WHERE name_rank = 1 \
            ORDER BY group_ended_at DESC, identity_key \
            LIMIT 50",
        )
        .fetch_all(self.db.read())
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| SearchableApp {
                bundle_id: row.get::<Option<String>, _>("bundle_id"),
                name: row
                    .get::<Option<String>, _>("name")
                    .map(|name| name.trim().to_string())
                    .filter(|name| !name.is_empty()),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use crate::search::test_support::*;
    use crate::AppInfra;

    #[test]
    fn list_searchable_apps_returns_seeded_apps_by_recency() {
        run_async_test(async {
            let dir = test_dir("list-searchable-apps");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            // Oldest first, newest last; expected order is by recency desc.
            seed_frame_with_text(
                &infra,
                "/tmp/list-apps-safari.jpg",
                "2026-05-17T10:00:00Z",
                Some(frame_with_app(Some("com.apple.Safari"), Some("Safari"))),
                "older app text",
            )
            .await;
            seed_frame_with_text(
                &infra,
                "/tmp/list-apps-chrome.jpg",
                "2026-05-17T11:00:00Z",
                Some(frame_with_app(Some("com.google.Chrome"), Some("Chrome"))),
                "newer app text",
            )
            .await;
            // A frame with no app identity should not appear in the list.
            seed_frame_with_text(
                &infra,
                "/tmp/list-apps-anon.jpg",
                "2026-05-17T12:00:00Z",
                None,
                "anonymous app text",
            )
            .await;

            let apps = infra
                .list_searchable_apps()
                .await
                .expect("list_searchable_apps should succeed");

            assert_eq!(apps.len(), 2);
            // Chrome is the most recent app-bearing frame, so it ranks first.
            assert_eq!(apps[0].bundle_id.as_deref(), Some("com.google.Chrome"));
            assert_eq!(apps[0].name.as_deref(), Some("Chrome"));
            assert_eq!(apps[1].bundle_id.as_deref(), Some("com.apple.Safari"));
            assert_eq!(apps[1].name.as_deref(), Some("Safari"));
        });
    }

    #[test]
    fn list_searchable_apps_collapses_one_bundle_id_with_missing_name() {
        // Captures for the same app frequently disagree on `app_name`: some
        // frames carry the label, others have it missing. Grouping by both
        // bundle id and name would emit one row per (bundle, name) variant,
        // letting a single app occupy several of the capped 50 slots. The list
        // must expose a single row per stable identity (bundle id) and surface
        // the best non-empty display name rather than the arbitrary newest one.
        run_async_test(async {
            let dir = test_dir("list-apps-dedupe-identity");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            seed_frame_with_text(
                &infra,
                "/tmp/list-apps-named.jpg",
                "2026-05-17T10:00:00Z",
                Some(frame_with_app(Some("com.apple.Safari"), Some("Safari"))),
                "older named capture",
            )
            .await;
            // Newer capture for the SAME bundle id but with no app name.
            seed_frame_with_text(
                &infra,
                "/tmp/list-apps-unnamed.jpg",
                "2026-05-17T11:00:00Z",
                Some(frame_with_app(Some("com.apple.Safari"), None)),
                "newer unnamed capture",
            )
            .await;

            let apps = infra
                .list_searchable_apps()
                .await
                .expect("list_searchable_apps should succeed");

            assert_eq!(apps.len(), 1, "same bundle id must collapse to one row");
            assert_eq!(apps[0].bundle_id.as_deref(), Some("com.apple.Safari"));
            assert_eq!(
                apps[0].name.as_deref(),
                Some("Safari"),
                "the non-empty name must win over the newer empty one"
            );
        });
    }

    #[test]
    fn list_searchable_apps_orders_by_real_time_across_offsets() {
        // `absolute_end_at` is stored verbatim as RFC3339 text and may carry
        // non-UTC offsets. Ordering on raw TEXT max would compare the strings
        // lexicographically and rank a later instant ("…T23:00:00-08:00",
        // i.e. the next day in UTC) below an earlier UTC string. Recency order
        // must follow the real instant, matching the julianday() comparison the
        // date-range filter already uses.
        run_async_test(async {
            let dir = test_dir("list-apps-offset-recency");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            // Earlier instant, plain UTC text that sorts lexicographically high.
            seed_frame_with_text(
                &infra,
                "/tmp/list-apps-utc.jpg",
                "2026-05-18T05:00:00Z",
                Some(frame_with_app(Some("com.example.Earlier"), Some("Earlier"))),
                "earlier utc capture",
            )
            .await;
            // Later instant (2026-05-18T07:00:00Z) expressed with a -08:00
            // offset, so its raw text sorts lexicographically *below* the UTC
            // string above.
            seed_frame_with_text(
                &infra,
                "/tmp/list-apps-offset.jpg",
                "2026-05-17T23:00:00-08:00",
                Some(frame_with_app(Some("com.example.Later"), Some("Later"))),
                "later offset capture",
            )
            .await;

            let apps = infra
                .list_searchable_apps()
                .await
                .expect("list_searchable_apps should succeed");

            assert_eq!(apps.len(), 2);
            assert_eq!(
                apps[0].bundle_id.as_deref(),
                Some("com.example.Later"),
                "the later real instant must rank first regardless of text offset"
            );
            assert_eq!(apps[1].bundle_id.as_deref(), Some("com.example.Earlier"));
        });
    }
}
