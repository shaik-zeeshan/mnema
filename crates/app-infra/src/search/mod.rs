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

pub(crate) use equivalent_reuse::project_equivalent_frame_reuse_in_transaction;
pub(crate) use projection::project_processing_result_direct_in_transaction;
use equivalent_reuse::*;
use grouping::align_audio_results;
use projection::*;
use retrieval::{
    clamp_limit, fetch_grouped_audio_hits, fetch_grouped_frame_hits,
    fetch_search_document_high_water_mark,
};

// Referenced only by the test module below (each is exercised through `super::*`);
// the production search entry points reach them via the sibling modules instead.
#[cfg(test)]
use crate::AudioSegmentSourceKind;
#[cfg(test)]
use grouping::{group_audio_hits, group_frame_hits};
#[cfg(test)]
use retrieval::{meaning_snippet, rrf_fuse_frame_hits, AudioHit, FrameHit};

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
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::{
        AppInfra, NewAudioSegment, NewCaptureSession, NewFrame, ProcessingJobDraft,
        ProcessingResultDraft,
    };
    use audio_transcription::{TranscriptionMetadata, TranscriptionSegment};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_dir(name: &str) -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!("mnema-search-{name}-{}-{id}", std::process::id()))
    }

    async fn complete_job(
        infra: &AppInfra,
        job: crate::ProcessingJob,
        result: ProcessingResultDraft,
    ) {
        let running = infra
            .claim_queued_processing_job(job.id)
            .await
            .expect("job should claim")
            .expect("job should exist");
        infra
            .complete_processing_job(running.id, &result)
            .await
            .expect("job should complete");
    }

    fn run_async_test(test: impl std::future::Future<Output = ()>) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(test);
    }

    #[test]
    fn search_rejects_incompatible_app_and_audio_refinements_in_band() {
        let errors = normalize_search_refinements(Some(SearchCaptureRefinements {
            date_range: None,
            apps: vec![SearchAppRefinement {
                kind: SearchAppRefinementKind::BundleId,
                value: "com.example.Linear".to_string(),
                display_name: "Linear".to_string(),
            }],
            window_title: None,
            audio_sources: vec![AudioSegmentSourceKind::Microphone],
            screen_source: false,
        }))
        .expect("conflict should not throw")
        .expect_err("incompatible refinements should surface in-band parse errors");

        assert!(
            errors
                .iter()
                .any(|error| error.kind == "app_source_conflict"
                    && error.message.contains("cannot be combined")),
            "expected an app_source_conflict parse error, got {errors:?}"
        );
    }

    #[test]
    fn search_projects_completed_ocr_and_groups_equivalent_frames() {
        run_async_test(async {
            let dir = test_dir("ocr-groups");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let first = infra
                .insert_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-frame-a.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_equivalence(crate::FrameEquivalence {
                        hint: Some("same-screen".to_string()),
                        proof: Some(vec![0; 1024]),
                        version: Some(1),
                        status: Some(crate::FrameEquivalenceStatus::Ready),
                        error: None,
                    }),
                )
                .await
                .expect("first frame should insert");
            let second = infra
                .insert_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-frame-b.jpg",
                        "2026-05-17T10:00:02Z",
                    )
                    .with_equivalence(crate::FrameEquivalence {
                        hint: Some("same-screen".to_string()),
                        proof: Some(vec![0; 1024]),
                        version: Some(1),
                        status: Some(crate::FrameEquivalenceStatus::Ready),
                        error: None,
                    }),
                )
                .await
                .expect("second frame should insert");

            for frame in [&first, &second] {
                let job = infra
                    .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
                    .await
                    .expect("ocr job should enqueue");
                complete_job(
                    &infra,
                    job,
                    ProcessingResultDraft::new()
                        .with_result_text("quarterly roadmap search target"),
                )
                .await;
            }

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "roadmap".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(5),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.frames.len(), 1);
            assert_eq!(response.frames[0].match_count, 2);
            assert_eq!(response.frames[0].representative_frame.id, second.id);
            assert!(response.audio.is_empty());
        });
    }

    #[test]
    fn startup_backfills_search_projection_for_existing_latest_results() {
        run_async_test(async {
            let dir = test_dir("startup-backfill");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let frame = infra
                .insert_frame(&NewFrame::new(
                    "screen-session",
                    "/tmp/search-startup-backfill.jpg",
                    "2026-05-17T10:00:00Z",
                ))
                .await
                .expect("frame should insert");

            for text in ["old upgraded text", "fresh upgraded text"] {
                let job = infra
                    .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
                    .await
                    .expect("ocr job should enqueue");
                complete_job(
                    &infra,
                    job,
                    ProcessingResultDraft::new().with_result_text(text),
                )
                .await;
            }

            sqlx::query("DELETE FROM search_documents")
                .execute(infra.pool())
                .await
                .expect("search documents should delete");
            drop(infra);

            let reopened = AppInfra::initialize(&dir)
                .await
                .expect("infra should reopen");
            let stale = reopened
                .search_capture(SearchCaptureRequest {
                    query: "old".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("stale search should succeed");
            assert!(stale.frames.is_empty());

            let fresh = reopened
                .search_capture(SearchCaptureRequest {
                    query: "fresh".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("fresh search should succeed");
            assert_eq!(fresh.frames.len(), 1);
            assert_eq!(fresh.frames[0].representative_frame.id, frame.id);
        });
    }

    #[test]
    fn fast_initialize_defers_search_projection_backfill_until_maintenance_runs() {
        run_async_test(async {
            let dir = test_dir("fast-init-defers-backfill");

            // Seed a frame + OCR result (projected on write), then delete the
            // projection so the index needs the startup repair to be searchable.
            let frame_id;
            {
                let infra = AppInfra::initialize(&dir)
                    .await
                    .expect("infra should initialize");
                let frame = infra
                    .insert_frame(&NewFrame::new(
                        "screen-session",
                        "/tmp/fast-init-defers-backfill.jpg",
                        "2026-05-17T10:00:00Z",
                    ))
                    .await
                    .expect("frame should insert");
                frame_id = frame.id;
                let job = infra
                    .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
                    .await
                    .expect("ocr job should enqueue");
                complete_job(
                    &infra,
                    job,
                    ProcessingResultDraft::new().with_result_text("deferred backfill text"),
                )
                .await;
                sqlx::query("DELETE FROM search_documents")
                    .execute(infra.pool())
                    .await
                    .expect("search documents should delete");
            }

            let search_request = || SearchCaptureRequest {
                query: "deferred".to_string(),
                frame_limit: Some(5),
                frame_offset: None,
                audio_limit: Some(0),
                audio_offset: None,
                snapshot_document_id: None,
                refinements: None,
                query_embedding: None,
            };

            // The fast init path opens the index but must NOT run the projection
            // backfill — that is what keeps the expensive scans off the
            // window-open critical path — so the missing projection stays missing.
            let infra = AppInfra::initialize_fast_with_processing_registry(
                &dir,
                crate::default_processing_registry(),
            )
            .await
            .expect("fast infra should initialize");
            let before = infra
                .search_capture(search_request())
                .await
                .expect("search before maintenance should succeed");
            assert!(
                before.frames.is_empty(),
                "fast init should defer the search projection backfill"
            );

            // Running startup maintenance repairs the missing projection.
            infra
                .run_startup_maintenance()
                .await
                .expect("startup maintenance should run");
            let after = infra
                .search_capture(search_request())
                .await
                .expect("search after maintenance should succeed");
            assert_eq!(after.frames.len(), 1);
            assert_eq!(after.frames[0].representative_frame.id, frame_id);
        });
    }

    #[test]
    fn startup_backfill_does_not_double_project_multi_span_audio_result() {
        run_async_test(async {
            let dir = test_dir("backfill-audio-multi-span");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");

            // A single audio transcription result with two segments projects two
            // `direct` search_documents (one per span) for the same
            // processing_result. This is the exact case where the
            // backfill LEFT JOIN would row-multiply if its `IS NULL` anti-join
            // guard regressed to an inner join, so it must be re-projected once.
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "mic-session",
                    1,
                    "/tmp/backfill-audio-multi-span.m4a",
                    "2026-05-17T10:00:00Z",
                    "2026-05-17T10:00:20Z",
                ))
                .await
                .expect("segment should insert");
            let metadata = TranscriptionMetadata {
                provider: "test".to_string(),
                model_id: None,
                language: "en".to_string(),
                segments: vec![
                    TranscriptionSegment {
                        start_ms: 1_000,
                        end_ms: 2_500,
                        text: "deferred backfill alpha".to_string(),
                        confidence: None,
                    },
                    TranscriptionSegment {
                        start_ms: 3_000,
                        end_ms: 4_500,
                        text: "deferred backfill beta".to_string(),
                        confidence: None,
                    },
                ],
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
                    .with_result_text("deferred backfill alpha deferred backfill beta")
                    .with_structured_payload_json(
                        serde_json::to_string(&metadata).expect("metadata should serialize"),
                    ),
            )
            .await;

            // Two `direct` docs were projected on write.
            let direct_count = || async {
                sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM search_documents \
                     WHERE audio_segment_id = ?1 AND text_source_kind = 'direct'",
                )
                .bind(segment.id)
                .fetch_one(infra.pool())
                .await
                .expect("direct doc count should load")
            };
            assert_eq!(
                direct_count().await,
                2,
                "write path projects two direct docs"
            );

            // Drop the projection so the startup backfill must repair it.
            sqlx::query("DELETE FROM search_documents")
                .execute(infra.pool())
                .await
                .expect("search documents should delete");
            assert_eq!(direct_count().await, 0);

            // Backfill must re-project the multi-span result exactly once: two
            // direct docs total, not four.
            infra
                .run_startup_maintenance()
                .await
                .expect("startup maintenance should run");
            assert_eq!(
                direct_count().await,
                2,
                "anti-join must re-project the multi-span audio result exactly once"
            );

            // Re-running the backfill while both direct docs already exist must be
            // a no-op for this result: the anti-join must NOT re-select an
            // already-projected multi-span result and append a second copy of its
            // spans (which an inner join / dropped `IS NULL` guard would do).
            infra
                .run_startup_maintenance()
                .await
                .expect("repeat startup maintenance should run");
            assert_eq!(
                direct_count().await,
                2,
                "anti-join must not double-project an already-projected multi-span result"
            );

            // Search returns the single grouped audio result, not N duplicates.
            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "deferred".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(5),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");
            assert!(response.frames.is_empty());
            assert_eq!(
                response.audio.len(),
                1,
                "the grouped audio result must not be duplicated by the backfill"
            );
        });
    }

    #[test]
    fn startup_backfill_marks_frames_without_app_bundle_id_as_checked() {
        run_async_test(async {
            let dir = test_dir("startup-backfill-empty-app-bundle");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let frame = infra
                .insert_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-empty-app-bundle.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_metadata_snapshot(
                        capture_metadata::FrameMetadataSnapshot {
                            app_bundle_id: None,
                            app_name: Some("Notes".to_string()),
                            window_title: Some("Planning".to_string()),
                            window_id: None,
                            browser_url: None,
                            display_id: Some(1),
                            metadata_redaction_reason: None,
                            metadata_redaction_source_id: None,
                        },
                    ),
                )
                .await
                .expect("frame should insert");
            let frame_without_metadata = infra
                .insert_frame(&NewFrame::new(
                    "screen-session",
                    "/tmp/search-no-metadata.jpg",
                    "2026-05-17T10:00:01Z",
                ))
                .await
                .expect("frame without metadata should insert");
            for frame_id in [frame.id, frame_without_metadata.id] {
                let job = infra
                    .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame_id))
                    .await
                    .expect("ocr job should enqueue");
                complete_job(
                    &infra,
                    job,
                    ProcessingResultDraft::new().with_result_text("empty bundle target"),
                )
                .await;
            }
            let inserted_null_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM search_documents \
                 WHERE frame_id IN (?1, ?2) \
                   AND (app_bundle_id IS NULL OR app_name_search_key IS NULL)",
            )
            .bind(frame.id)
            .bind(frame_without_metadata.id)
            .fetch_one(infra.pool())
            .await
            .expect("inserted null count should load");
            let inserted_checked_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM search_documents \
                 WHERE frame_id IN (?1, ?2) \
                   AND app_bundle_id = '' \
                   AND app_name_search_key IS NOT NULL",
            )
            .bind(frame.id)
            .bind(frame_without_metadata.id)
            .fetch_one(infra.pool())
            .await
            .expect("inserted checked count should load");

            assert_eq!(inserted_null_count, 0);
            assert_eq!(inserted_checked_count, 2);
            drop(infra);

            let reopened = AppInfra::initialize(&dir)
                .await
                .expect("infra should reopen");
            let null_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM search_documents \
                 WHERE frame_id IN (?1, ?2) AND app_bundle_id IS NULL",
            )
            .bind(frame.id)
            .bind(frame_without_metadata.id)
            .fetch_one(reopened.pool())
            .await
            .expect("null count should load");
            let checked_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM search_documents \
                 WHERE frame_id IN (?1, ?2) AND app_bundle_id = ''",
            )
            .bind(frame.id)
            .bind(frame_without_metadata.id)
            .fetch_one(reopened.pool())
            .await
            .expect("checked count should load");

            assert_eq!(null_count, 0);
            assert_eq!(checked_count, 2);
        });
    }

    #[test]
    fn startup_backfills_missing_equivalent_reuse_projection_when_direct_projection_exists() {
        run_async_test(async {
            let dir = test_dir("startup-backfill-equivalent-reuse");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let equivalence = crate::FrameEquivalence {
                hint: Some("same-startup-reuse".to_string()),
                proof: Some(vec![31; 1024]),
                version: Some(1),
                status: Some(crate::FrameEquivalenceStatus::Ready),
                error: None,
            };
            let first = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-startup-reuse-source.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_equivalence(equivalence.clone()),
                    None,
                )
                .await
                .expect("first frame should capture");
            let job = first.job.expect("first frame should enqueue OCR");
            complete_job(
                &infra,
                job,
                ProcessingResultDraft::new().with_result_text("historical reuse target"),
            )
            .await;

            let second = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-startup-reuse-duplicate.jpg",
                        "2026-05-17T10:00:02Z",
                    )
                    .with_equivalence(equivalence),
                    None,
                )
                .await
                .expect("second frame should capture");
            assert!(second.job.is_none());

            sqlx::query(
                "DELETE FROM search_documents \
                 WHERE frame_id = ?1 AND text_source_kind = 'equivalent_reuse'",
            )
            .bind(second.frame.id)
            .execute(infra.pool())
            .await
            .expect("equivalent reuse projection should delete");

            let direct_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM search_documents \
                 WHERE frame_id = ?1 AND text_source_kind = 'direct'",
            )
            .bind(first.frame.id)
            .fetch_one(infra.pool())
            .await
            .expect("direct projection count should load");
            assert_eq!(direct_count, 1);
            drop(infra);

            let reopened = AppInfra::initialize(&dir)
                .await
                .expect("infra should reopen");
            let response = reopened
                .search_capture(SearchCaptureRequest {
                    query: "historical".to_string(),
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
            assert_eq!(response.frames[0].match_count, 2);
            assert_eq!(response.frames[0].representative_frame.id, second.frame.id);
            assert_eq!(response.frames[0].text_source_kind, "equivalent_reuse");
        });
    }

    #[test]
    fn search_projects_transcript_segments_and_sanitizes_plain_query() {
        run_async_test(async {
            let dir = test_dir("audio-segments");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "mic-session",
                    1,
                    "/tmp/search-audio.m4a",
                    "2026-05-17T10:00:00Z",
                    "2026-05-17T10:00:20Z",
                ))
                .await
                .expect("segment should insert");
            let metadata = TranscriptionMetadata {
                provider: "test".to_string(),
                model_id: None,
                language: "en".to_string(),
                segments: vec![TranscriptionSegment {
                    start_ms: 1_000,
                    end_ms: 2_500,
                    text: "search target phrase".to_string(),
                    confidence: None,
                }],
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
                    .with_result_text("search target phrase")
                    .with_structured_payload_json(
                        serde_json::to_string(&metadata).expect("metadata should serialize"),
                    ),
            )
            .await;

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "\"target\"".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(5),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert!(response.frames.is_empty());
            assert_eq!(response.audio.len(), 1);
            assert_eq!(response.audio[0].span_start_ms, 1_000);
            assert_eq!(response.audio[0].span_end_ms, 2_500);
        });
    }

    #[test]
    fn audio_search_alignment_uses_mapped_screen_source_session() {
        run_async_test(async {
            let dir = test_dir("audio-screen-alignment");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            infra
                .capture_retention()
                .create_capture_session(&NewCaptureSession {
                    capture_session_id: "capture-session".to_string(),
                    started_at: "2026-05-17T10:00:00Z".to_string(),
                    requested_screen: true,
                    requested_microphone: true,
                    requested_system_audio: false,
                    screen_source_session_id: Some("screen-session".to_string()),
                    microphone_source_session_id: Some("mic-session".to_string()),
                    system_audio_source_session_id: None,
                    segment_duration_seconds: 300,
                })
                .await
                .expect("capture session should insert");
            let frame = infra
                .insert_frame(&NewFrame::new(
                    "screen-session",
                    "/tmp/search-aligned-screen.jpg",
                    "2026-05-17T10:00:01Z",
                ))
                .await
                .expect("screen frame should insert");
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "mic-session",
                    1,
                    "/tmp/search-aligned-audio.m4a",
                    "2026-05-17T10:00:00Z",
                    "2026-05-17T10:00:20Z",
                ))
                .await
                .expect("segment should insert");
            let metadata = TranscriptionMetadata {
                provider: "test".to_string(),
                model_id: None,
                language: "en".to_string(),
                segments: vec![TranscriptionSegment {
                    start_ms: 1_000,
                    end_ms: 2_000,
                    text: "aligned audio target".to_string(),
                    confidence: None,
                }],
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
                    .with_result_text("aligned audio target")
                    .with_structured_payload_json(
                        serde_json::to_string(&metadata).expect("metadata should serialize"),
                    ),
            )
            .await;

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "aligned".to_string(),
                    frame_limit: Some(0),
                    frame_offset: None,
                    audio_limit: Some(5),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.audio.len(), 1);
            assert_eq!(
                response.audio[0]
                    .aligned_frame
                    .as_ref()
                    .map(|frame| frame.id),
                Some(frame.id)
            );
        });
    }

    #[test]
    fn search_projects_untimed_transcript_fallback_over_full_audio_segment() {
        run_async_test(async {
            let dir = test_dir("audio-untimed-fallback");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "mic-session",
                    1,
                    "/tmp/search-audio-untimed.m4a",
                    "2026-05-17T10:00:00Z",
                    "2026-05-17T10:00:20Z",
                ))
                .await
                .expect("segment should insert");
            let metadata = TranscriptionMetadata {
                provider: "test".to_string(),
                model_id: None,
                language: "en".to_string(),
                segments: Vec::new(),
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
                    .with_result_text("untimed search target phrase")
                    .with_structured_payload_json(
                        serde_json::to_string(&metadata).expect("metadata should serialize"),
                    ),
            )
            .await;

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "untimed".to_string(),
                    frame_limit: Some(0),
                    frame_offset: None,
                    audio_limit: Some(5),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.audio.len(), 1);
            assert_eq!(response.audio[0].span_start_ms, 0);
            assert_eq!(response.audio[0].span_end_ms, 20_000);
            assert_eq!(response.audio[0].absolute_start_at, "2026-05-17T10:00:00Z");
            assert_eq!(response.audio[0].absolute_end_at, "2026-05-17T10:00:20Z");
        });
    }

    #[test]
    fn audio_hits_group_chronologically_before_rank_ordering() {
        let segment = AudioSegment {
            id: 7,
            source_kind: AudioSegmentSourceKind::Microphone,
            source_session_id: "mic-session".to_string(),
            segment_index: 1,
            file_path: "/tmp/audio.m4a".to_string(),
            started_at: "2026-05-17T10:00:00Z".to_string(),
            ended_at: "2026-05-17T10:00:20Z".to_string(),
            capture_segment_id: None,
            created_at: "2026-05-17T10:00:00Z".to_string(),
            updated_at: "2026-05-17T10:00:00Z".to_string(),
        };
        let hit = |span_start_ms, span_end_ms, rank| AudioHit {
            anchor_id: span_start_ms as i64,
            audio_segment: segment.clone(),
            source_kind: AudioSegmentSourceKind::Microphone,
            span_start_ms,
            span_end_ms,
            snippet: format!("hit {span_start_ms}"),
            rank,
            secret_redaction_count: 0,
            found_by_meaning: false,
        };

        let hits = vec![
            hit(4_000, 4_500, -10.0),
            hit(1_000, 1_500, -1.0),
            hit(2_200, 2_500, -5.0),
        ];
        let groups = group_audio_hits(&hits).expect("grouping should succeed");

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].span_start_ms, 1_000);
        assert_eq!(groups[0].span_end_ms, 4_500);
        assert_eq!(groups[0].match_count, 3);
    }

    #[test]
    fn audio_groups_preserve_best_relevance_before_recency() {
        let segment = AudioSegment {
            id: 7,
            source_kind: AudioSegmentSourceKind::Microphone,
            source_session_id: "mic-session".to_string(),
            segment_index: 1,
            file_path: "/tmp/audio.m4a".to_string(),
            started_at: "2026-05-17T10:00:00Z".to_string(),
            ended_at: "2026-05-17T10:00:20Z".to_string(),
            capture_segment_id: None,
            created_at: "2026-05-17T10:00:00Z".to_string(),
            updated_at: "2026-05-17T10:00:00Z".to_string(),
        };
        let hit = |span_start_ms, rank| AudioHit {
            anchor_id: span_start_ms as i64,
            audio_segment: segment.clone(),
            source_kind: AudioSegmentSourceKind::Microphone,
            span_start_ms,
            span_end_ms: span_start_ms + 500,
            snippet: format!("hit {span_start_ms}"),
            rank,
            secret_redaction_count: 0,
            found_by_meaning: false,
        };

        let hits = vec![hit(10_000, -1.0), hit(1_000, -10.0)];
        let groups = group_audio_hits(&hits).expect("grouping should succeed");

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].span_start_ms, 1_000);
        assert_eq!(groups[1].span_start_ms, 10_000);
    }

    #[test]
    fn frame_groups_preserve_best_relevance_before_recency() {
        let frame = |id: i64, captured_at: &str| Frame {
            id,
            session_id: "screen-session".to_string(),
            file_path: format!("/tmp/relevance-{id}.jpg"),
            captured_at: captured_at.to_string(),
            width: None,
            height: None,
            equivalence: crate::FrameEquivalence {
                hint: None,
                proof: None,
                version: None,
                status: None,
                error: None,
            },
            metadata_snapshot: None,
            created_at: captured_at.to_string(),
            updated_at: captured_at.to_string(),
        };
        let hit = |id, captured_at, rank| FrameHit {
            anchor_id: id,
            group_key: format!("frame:{id}"),
            frame: frame(id, captured_at),
            snippet: format!("hit {id}"),
            rank,
            app_bundle_id: None,
            app_name: None,
            window_title: None,
            text_source_kind: "direct".to_string(),
            secret_redaction_count: 0,
            found_by_meaning: false,
        };

        let hits = vec![
            hit(1, "2026-05-17T10:00:00Z", -10.0),
            hit(2, "2026-05-17T10:10:00Z", -1.0),
        ];
        let groups = group_frame_hits(&hits);

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].representative_frame.id, 1);
        assert_eq!(groups[1].representative_frame.id, 2);
    }

    #[test]
    fn frame_group_carries_representative_browser_url_read_time() {
        // Read-time proof: `group_frame_hits` lifts `browser_url` from the
        // SAME representative frame's metadata snapshot whose id becomes the
        // result (and opaque) id — no index column, so any historical frame
        // with a snapshot browser_url is covered for free.
        let frame_with_url = |id: i64, captured_at: &str, browser_url: Option<&str>| Frame {
            id,
            session_id: "screen-session".to_string(),
            file_path: format!("/tmp/url-{id}.jpg"),
            captured_at: captured_at.to_string(),
            width: None,
            height: None,
            equivalence: crate::FrameEquivalence {
                hint: None,
                proof: None,
                version: None,
                status: None,
                error: None,
            },
            metadata_snapshot: browser_url.map(|url| capture_metadata::FrameMetadataSnapshot {
                app_bundle_id: Some("com.google.Chrome".to_string()),
                app_name: Some("Google Chrome".to_string()),
                window_title: Some("Tab".to_string()),
                window_id: None,
                browser_url: Some(url.to_string()),
                display_id: Some(1),
                metadata_redaction_reason: None,
                metadata_redaction_source_id: None,
            }),
            created_at: captured_at.to_string(),
            updated_at: captured_at.to_string(),
        };
        let hit = |id, captured_at, browser_url| FrameHit {
            anchor_id: id,
            group_key: format!("frame:{id}"),
            frame: frame_with_url(id, captured_at, browser_url),
            snippet: format!("hit {id}"),
            rank: -1.0,
            app_bundle_id: None,
            app_name: None,
            window_title: None,
            text_source_kind: "direct".to_string(),
            secret_redaction_count: 0,
            found_by_meaning: false,
        };

        // With no equivalence proof, each distinct frame is its own group; the
        // representative IS the single hit, so its snapshot browser_url surfaces
        // raw on the result (the broker boundary guards it, not search).
        let groups = group_frame_hits(&[
            hit(
                1,
                "2026-05-17T10:10:00Z",
                Some("https://github.com/owner/repo/commit/9fceb02d8f1c"),
            ),
            // A frame with no snapshot browser_url -> result browser_url is None.
            hit(2, "2026-05-17T10:00:00Z", None),
        ]);
        let by_id = |id: i64| {
            groups
                .iter()
                .find(|group| group.representative_frame.id == id)
                .expect("group should exist")
        };
        assert_eq!(
            by_id(1).browser_url.as_deref(),
            Some("https://github.com/owner/repo/commit/9fceb02d8f1c"),
            "browser_url comes from the representative frame's snapshot, raw"
        );
        assert_eq!(by_id(2).browser_url, None);
    }

    #[test]
    fn search_indexes_frame_context_terms() {
        run_async_test(async {
            let dir = test_dir("frame-context");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let frame = infra
                .insert_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-frame-context.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_metadata_snapshot(
                        capture_metadata::FrameMetadataSnapshot {
                            app_bundle_id: Some("com.example.Linear".to_string()),
                            app_name: Some("Linear".to_string()),
                            window_title: Some("Roadmap Grooming".to_string()),
                            window_id: None,
                            browser_url: None,
                            display_id: Some(1),
                            metadata_redaction_reason: None,
                            metadata_redaction_source_id: None,
                        },
                    ),
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
                ProcessingResultDraft::new().with_result_text("ordinary body text"),
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

            assert_eq!(response.frames.len(), 1);
            assert_eq!(response.frames[0].representative_frame.id, frame.id);
            assert_eq!(
                response.frames[0].app_bundle_id.as_deref(),
                Some("com.example.Linear")
            );
            assert_eq!(response.frames[0].app_name.as_deref(), Some("Linear"));
            assert_eq!(
                response.frames[0].window_title.as_deref(),
                Some("Roadmap Grooming")
            );
        });
    }

    #[test]
    fn search_refinements_filter_by_date_app_and_audio_source() {
        run_async_test(async {
            let dir = test_dir("search-refinements");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let linear = infra
                .insert_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-refinement-linear.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_metadata_snapshot(
                        capture_metadata::FrameMetadataSnapshot {
                            app_bundle_id: Some(" com.example.Linear ".to_string()),
                            app_name: Some("Linear".to_string()),
                            window_title: Some("Planning".to_string()),
                            window_id: None,
                            browser_url: None,
                            display_id: Some(1),
                            metadata_redaction_reason: None,
                            metadata_redaction_source_id: None,
                        },
                    ),
                )
                .await
                .expect("linear frame should insert");
            let notes = infra
                .insert_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-refinement-notes.jpg",
                        "2026-05-17T11:00:00Z",
                    )
                    .with_metadata_snapshot(
                        capture_metadata::FrameMetadataSnapshot {
                            app_bundle_id: None,
                            app_name: Some("Notes".to_string()),
                            window_title: Some("Planning".to_string()),
                            window_id: None,
                            browser_url: None,
                            display_id: Some(1),
                            metadata_redaction_reason: None,
                            metadata_redaction_source_id: None,
                        },
                    ),
                )
                .await
                .expect("notes frame should insert");
            for frame in [&linear, &notes] {
                let job = infra
                    .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
                    .await
                    .expect("ocr job should enqueue");
                complete_job(
                    &infra,
                    job,
                    ProcessingResultDraft::new().with_result_text("refined target text"),
                )
                .await;
            }

            let mic = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "mic-session",
                    1,
                    "/tmp/search-refinement-mic.m4a",
                    "2026-05-17T10:00:00Z",
                    "2026-05-17T10:00:10Z",
                ))
                .await
                .expect("mic segment should insert");
            let system = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::SystemAudio,
                    "system-session",
                    1,
                    "/tmp/search-refinement-system.m4a",
                    "2026-05-17T10:00:00Z",
                    "2026-05-17T10:00:10Z",
                ))
                .await
                .expect("system segment should insert");
            for segment in [&mic, &system] {
                let job = infra
                    .enqueue_processing_job(&ProcessingJobDraft::for_audio_segment_transcription(
                        segment.id,
                    ))
                    .await
                    .expect("transcription job should enqueue");
                complete_job(
                    &infra,
                    job,
                    ProcessingResultDraft::new().with_result_text("refined target audio"),
                )
                .await;
            }

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "refined".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(5),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: Some(SearchCaptureRefinements {
                        date_range: Some(SearchDateRangeRefinement {
                            start_at: "2026-05-17T04:59:00-05:00".to_string(),
                            end_at: "2026-05-17T05:30:00-05:00".to_string(),
                            origin: Some(SearchDateRangeOrigin::VisibleTimeline),
                        }),
                        apps: vec![SearchAppRefinement {
                            kind: SearchAppRefinementKind::BundleId,
                            value: " COM.EXAMPLE.LINEAR ".to_string(),
                            display_name: "Linear".to_string(),
                        }],
                        window_title: None,
                        audio_sources: Vec::new(),
                        screen_source: false,
                    }),
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.frames.len(), 1);
            assert_eq!(response.frames[0].representative_frame.id, linear.id);
            assert!(response.audio.is_empty());
            assert_eq!(
                response
                    .applied_refinements
                    .apps
                    .first()
                    .map(|app| app.value.as_str()),
                Some("COM.EXAMPLE.LINEAR")
            );
            let indexed_bundle_id: Option<String> = sqlx::query_scalar(
                "SELECT app_bundle_id FROM search_documents \
                 WHERE frame_id = ?1 AND text_source_kind = 'direct' LIMIT 1",
            )
            .bind(linear.id)
            .fetch_one(infra.pool())
            .await
            .expect("indexed bundle id should load");
            assert_eq!(indexed_bundle_id.as_deref(), Some("com.example.Linear"));
            let plan_rows = sqlx::query(
                "EXPLAIN QUERY PLAN \
                 SELECT id FROM search_documents \
                 WHERE anchor_type = 'frame' \
                   AND LOWER(TRIM(COALESCE(app_bundle_id, ''))) = LOWER(?1)",
            )
            .bind("COM.EXAMPLE.LINEAR")
            .fetch_all(infra.pool())
            .await
            .expect("query plan should load");
            let plan = plan_rows
                .iter()
                .map(|row| row.get::<String, _>("detail"))
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                plan.contains("search_documents_frame_bundle_id_refinement_idx"),
                "bundle-id refinement should use expression index, plan was:\n{plan}"
            );

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "refined".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(5),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: Some(SearchCaptureRefinements {
                        date_range: None,
                        apps: vec![SearchAppRefinement {
                            kind: SearchAppRefinementKind::Any,
                            value: "linear".to_string(),
                            display_name: "linear".to_string(),
                        }],
                        window_title: Some("plan".to_string()),
                        audio_sources: Vec::new(),
                        screen_source: false,
                    }),
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.frames.len(), 1);
            assert_eq!(response.frames[0].representative_frame.id, linear.id);
            assert!(response.audio.is_empty());

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "refined".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(5),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: Some(SearchCaptureRefinements {
                        date_range: None,
                        apps: Vec::new(),
                        window_title: None,
                        audio_sources: vec![AudioSegmentSourceKind::SystemAudio],
                        screen_source: false,
                    }),
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert!(response.frames.is_empty());
            assert_eq!(response.audio.len(), 1);
            assert_eq!(response.audio[0].audio_segment.id, system.id);
            assert_eq!(
                response.applied_refinements.audio_sources,
                vec![AudioSegmentSourceKind::SystemAudio]
            );
        });
    }

    #[test]
    fn date_refinement_compares_mixed_precision_timestamps_chronologically() {
        run_async_test(async {
            let dir = test_dir("search-refinement-fractional-date");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let before_boundary = infra
                .insert_frame(&NewFrame::new(
                    "screen-session",
                    "/tmp/search-fractional-before.jpg",
                    "2026-05-17T10:00:29.900Z",
                ))
                .await
                .expect("before boundary frame should insert");
            let after_boundary = infra
                .insert_frame(&NewFrame::new(
                    "screen-session",
                    "/tmp/search-fractional-after.jpg",
                    "2026-05-17T10:00:30.100Z",
                ))
                .await
                .expect("after boundary frame should insert");
            for frame in [&before_boundary, &after_boundary] {
                let job = infra
                    .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
                    .await
                    .expect("ocr job should enqueue");
                complete_job(
                    &infra,
                    job,
                    ProcessingResultDraft::new().with_result_text("fractional boundary target"),
                )
                .await;
            }

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "fractional".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: Some(SearchCaptureRefinements {
                        date_range: Some(SearchDateRangeRefinement {
                            start_at: "2026-05-17T10:00:29Z".to_string(),
                            end_at: "2026-05-17T10:00:30Z".to_string(),
                            origin: Some(SearchDateRangeOrigin::VisibleTimeline),
                        }),
                        apps: Vec::new(),
                        window_title: None,
                        audio_sources: Vec::new(),
                        screen_source: false,
                    }),
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.frames.len(), 1);
            assert_eq!(
                response.frames[0].representative_frame.id,
                before_boundary.id
            );
        });
    }

    #[test]
    fn app_name_search_refinement_is_unicode_trimmed_bundleless_fallback() {
        run_async_test(async {
            let dir = test_dir("app-name-refinement-fallback");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let fallback = infra
                .insert_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-app-name-fallback.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_metadata_snapshot(
                        capture_metadata::FrameMetadataSnapshot {
                            app_bundle_id: None,
                            app_name: Some(" ÉDITEUR ".to_string()),
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
                .expect("fallback frame should insert");
            let bundled = infra
                .insert_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-app-name-bundled.jpg",
                        "2026-05-17T10:01:00Z",
                    )
                    .with_metadata_snapshot(
                        capture_metadata::FrameMetadataSnapshot {
                            app_bundle_id: Some("com.example.Editor".to_string()),
                            app_name: Some("éditeur".to_string()),
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
                .expect("bundled frame should insert");

            for frame in [&fallback, &bundled] {
                let job = infra
                    .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
                    .await
                    .expect("ocr job should enqueue");
                complete_job(
                    &infra,
                    job,
                    ProcessingResultDraft::new().with_result_text("unicode fallback target"),
                )
                .await;
            }

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "unicode".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: Some(SearchCaptureRefinements {
                        date_range: None,
                        apps: vec![SearchAppRefinement {
                            kind: SearchAppRefinementKind::AppName,
                            value: "éditeur".to_string(),
                            display_name: "éditeur".to_string(),
                        }],
                        window_title: None,
                        audio_sources: Vec::new(),
                        screen_source: false,
                    }),
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.frames.len(), 1);
            assert_eq!(response.frames[0].representative_frame.id, fallback.id);
        });
    }

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
    fn search_projects_ocr_skipped_equivalent_frames() {
        run_async_test(async {
            let dir = test_dir("skipped-equivalent");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let first = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-skip-source.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_equivalence(crate::FrameEquivalence {
                        hint: Some("same-screen".to_string()),
                        proof: Some(vec![7; 1024]),
                        version: Some(1),
                        status: Some(crate::FrameEquivalenceStatus::Ready),
                        error: None,
                    }),
                    None,
                )
                .await
                .expect("first frame should capture");
            let job = first.job.expect("first frame should enqueue OCR");
            complete_job(
                &infra,
                job,
                ProcessingResultDraft::new().with_result_text("duplicate coverage target"),
            )
            .await;

            let second = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-skip-duplicate.jpg",
                        "2026-05-17T10:00:02Z",
                    )
                    .with_equivalence(crate::FrameEquivalence {
                        hint: Some("same-screen".to_string()),
                        proof: Some(vec![7; 1024]),
                        version: Some(1),
                        status: Some(crate::FrameEquivalenceStatus::Ready),
                        error: None,
                    }),
                    None,
                )
                .await
                .expect("second frame should capture");
            assert!(second.job.is_none());
            assert_eq!(
                second
                    .ocr_admission_decision
                    .as_ref()
                    .map(|decision| decision.reason),
                Some(crate::OcrAdmissionReason::SkippedEquivalentFrame)
            );

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "coverage".to_string(),
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
            assert_eq!(response.frames[0].match_count, 2);
            assert_eq!(response.frames[0].representative_frame.id, second.frame.id);
            assert_eq!(response.frames[0].text_source_kind, "equivalent_reuse");
        });
    }

    #[test]
    fn search_projects_equivalent_reuse_through_duplicate_chain() {
        run_async_test(async {
            let dir = test_dir("skipped-equivalent-chain");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let equivalence = crate::FrameEquivalence {
                hint: Some("same-chain".to_string()),
                proof: Some(vec![9; 1024]),
                version: Some(1),
                status: Some(crate::FrameEquivalenceStatus::Ready),
                error: None,
            };
            let first = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-chain-source.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_equivalence(equivalence.clone()),
                    None,
                )
                .await
                .expect("first frame should capture");
            let job = first.job.expect("first frame should enqueue OCR");
            complete_job(
                &infra,
                job,
                ProcessingResultDraft::new().with_result_text("duplicate chain target"),
            )
            .await;

            let second = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-chain-second.jpg",
                        "2026-05-17T10:00:01Z",
                    )
                    .with_equivalence(equivalence.clone()),
                    None,
                )
                .await
                .expect("second frame should capture");
            assert!(second.job.is_none());

            let third = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-chain-third.jpg",
                        "2026-05-17T10:00:02Z",
                    )
                    .with_equivalence(equivalence),
                    None,
                )
                .await
                .expect("third frame should capture");
            assert!(third.job.is_none());

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "chain".to_string(),
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
            assert_eq!(response.frames[0].match_count, 3);
            assert_eq!(response.frames[0].representative_frame.id, third.frame.id);
        });
    }

    #[test]
    fn source_ocr_reprojection_replaces_orphaned_equivalent_reuse_text() {
        run_async_test(async {
            let dir = test_dir("reuse-source-reproject");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let equivalence = crate::FrameEquivalence {
                hint: Some("same-reproject".to_string()),
                proof: Some(vec![19; 1024]),
                version: Some(1),
                status: Some(crate::FrameEquivalenceStatus::Ready),
                error: None,
            };
            let first = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-reproject-source.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_equivalence(equivalence.clone()),
                    None,
                )
                .await
                .expect("first frame should capture");
            let source_job = first.job.expect("first frame should enqueue OCR");
            let source_job_id = source_job.id;
            complete_job(
                &infra,
                source_job,
                ProcessingResultDraft::new().with_result_text("stale reuse target"),
            )
            .await;

            let second = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-reproject-target.jpg",
                        "2026-05-17T10:00:01Z",
                    )
                    .with_equivalence(equivalence),
                    None,
                )
                .await
                .expect("second frame should capture");
            assert!(second.job.is_none());

            sqlx::query("DELETE FROM processing_results WHERE job_id = ?1")
                .bind(source_job_id)
                .execute(infra.pool())
                .await
                .expect("source processing result delete should orphan reuse search");

            let replacement_job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(first.frame.id))
                .await
                .expect("replacement job should enqueue");
            complete_job(
                &infra,
                replacement_job,
                ProcessingResultDraft::new().with_result_text("fresh reuse target"),
            )
            .await;

            let stale = infra
                .search_capture(SearchCaptureRequest {
                    query: "stale".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("stale search should succeed");
            assert!(stale.frames.is_empty());

            let fresh = infra
                .search_capture(SearchCaptureRequest {
                    query: "fresh".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("fresh search should succeed");
            assert_eq!(fresh.frames.len(), 1);
            assert_eq!(fresh.frames[0].match_count, 2);
            assert_eq!(fresh.frames[0].representative_frame.id, second.frame.id);
        });
    }

    #[test]
    fn direct_ocr_reprojection_clears_current_frames_orphaned_equivalent_reuse_text() {
        run_async_test(async {
            let dir = test_dir("reuse-current-frame-reproject");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let equivalence = crate::FrameEquivalence {
                hint: Some("same-current-reproject".to_string()),
                proof: Some(vec![20; 1024]),
                version: Some(1),
                status: Some(crate::FrameEquivalenceStatus::Ready),
                error: None,
            };
            let first = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-current-source.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_equivalence(equivalence.clone()),
                    None,
                )
                .await
                .expect("first frame should capture");
            let source_job = first.job.expect("first frame should enqueue OCR");
            let source_job_id = source_job.id;
            complete_job(
                &infra,
                source_job,
                ProcessingResultDraft::new().with_result_text("old duplicate text"),
            )
            .await;

            let second = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-current-target.jpg",
                        "2026-05-17T10:00:01Z",
                    )
                    .with_equivalence(equivalence),
                    None,
                )
                .await
                .expect("second frame should capture");
            assert!(second.job.is_none());

            sqlx::query("DELETE FROM processing_results WHERE job_id = ?1")
                .bind(source_job_id)
                .execute(infra.pool())
                .await
                .expect("source processing result delete should orphan reuse search");

            let replacement_job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(second.frame.id))
                .await
                .expect("replacement job should enqueue");
            complete_job(
                &infra,
                replacement_job,
                ProcessingResultDraft::new().with_result_text("new direct text"),
            )
            .await;

            let old = infra
                .search_capture(SearchCaptureRequest {
                    query: "old".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("old search should succeed");
            assert!(old.frames.is_empty());

            let equivalent_reuse_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM search_documents \
                 WHERE frame_id = ?1 AND text_source_kind = 'equivalent_reuse'",
            )
            .bind(second.frame.id)
            .fetch_one(infra.pool())
            .await
            .expect("reuse count should load");
            assert_eq!(equivalent_reuse_count, 0);

            let fresh = infra
                .search_capture(SearchCaptureRequest {
                    query: "new".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("fresh search should succeed");
            assert_eq!(fresh.frames.len(), 1);
            assert_eq!(fresh.frames[0].match_count, 2);
            assert_eq!(fresh.frames[0].representative_frame.id, second.frame.id);
        });
    }

    #[test]
    fn source_ocr_reprojection_to_empty_clears_equivalent_reuse_text() {
        run_async_test(async {
            let dir = test_dir("reuse-source-empty");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let equivalence = crate::FrameEquivalence {
                hint: Some("same-empty-reproject".to_string()),
                proof: Some(vec![21; 1024]),
                version: Some(1),
                status: Some(crate::FrameEquivalenceStatus::Ready),
                error: None,
            };
            let first = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-empty-source.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_equivalence(equivalence.clone()),
                    None,
                )
                .await
                .expect("first frame should capture");
            let source_job = first.job.expect("first frame should enqueue OCR");
            complete_job(
                &infra,
                source_job,
                ProcessingResultDraft::new().with_result_text("vanishing reuse target"),
            )
            .await;

            let second = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-empty-target.jpg",
                        "2026-05-17T10:00:01Z",
                    )
                    .with_equivalence(equivalence),
                    None,
                )
                .await
                .expect("second frame should capture");
            assert!(second.job.is_none());

            let replacement_job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(first.frame.id))
                .await
                .expect("replacement job should enqueue");
            complete_job(
                &infra,
                replacement_job,
                ProcessingResultDraft::new().with_result_text("   "),
            )
            .await;

            let stale = infra
                .search_capture(SearchCaptureRequest {
                    query: "vanishing".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("stale search should succeed");
            assert!(stale.frames.is_empty());
        });
    }

    #[test]
    fn equivalent_reuse_projection_uses_raw_source_ocr_text() {
        run_async_test(async {
            let dir = test_dir("reuse-raw-source-text");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let equivalence = crate::FrameEquivalence {
                hint: Some("same-raw-source".to_string()),
                proof: Some(vec![22; 1024]),
                version: Some(1),
                status: Some(crate::FrameEquivalenceStatus::Ready),
                error: None,
            };
            let first = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-raw-source.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_equivalence(equivalence.clone())
                    .with_metadata_snapshot(
                        capture_metadata::FrameMetadataSnapshot {
                            app_bundle_id: Some("com.example.SourceOnly".to_string()),
                            app_name: Some("SourceOnlyApp".to_string()),
                            window_title: Some("Source Window".to_string()),
                            window_id: None,
                            browser_url: None,
                            display_id: Some(1),
                            metadata_redaction_reason: None,
                            metadata_redaction_source_id: None,
                        },
                    ),
                    None,
                )
                .await
                .expect("first frame should capture");
            let source_job = first.job.expect("first frame should enqueue OCR");
            complete_job(
                &infra,
                source_job,
                ProcessingResultDraft::new().with_result_text("shared body target"),
            )
            .await;

            let second = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-raw-target.jpg",
                        "2026-05-17T10:00:01Z",
                    )
                    .with_equivalence(equivalence)
                    .with_metadata_snapshot(
                        capture_metadata::FrameMetadataSnapshot {
                            app_bundle_id: Some("com.example.TargetOnly".to_string()),
                            app_name: Some("TargetOnlyApp".to_string()),
                            window_title: Some("Target Window".to_string()),
                            window_id: None,
                            browser_url: None,
                            display_id: Some(1),
                            metadata_redaction_reason: None,
                            metadata_redaction_source_id: None,
                        },
                    ),
                    None,
                )
                .await
                .expect("second frame should capture");
            assert!(second.job.is_none());

            let source_context = infra
                .search_capture(SearchCaptureRequest {
                    query: "SourceOnlyApp".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("source context search should succeed");
            assert_eq!(source_context.frames.len(), 1);
            assert_eq!(source_context.frames[0].match_count, 1);
            assert_eq!(
                source_context.frames[0].representative_frame.id,
                first.frame.id
            );

            let target_context = infra
                .search_capture(SearchCaptureRequest {
                    query: "TargetOnlyApp".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("target context search should succeed");
            assert_eq!(target_context.frames.len(), 1);
            assert_eq!(target_context.frames[0].match_count, 1);
            assert_eq!(
                target_context.frames[0].representative_frame.id,
                second.frame.id
            );
        });
    }

    #[test]
    fn equivalent_reuse_search_reports_source_result_redactions() {
        run_async_test(async {
            let dir = test_dir("reuse-source-redactions");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let equivalence = crate::FrameEquivalence {
                hint: Some("same-redaction-source".to_string()),
                proof: Some(vec![24; 1024]),
                version: Some(1),
                status: Some(crate::FrameEquivalenceStatus::Ready),
                error: None,
            };
            let source = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-redaction-source.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_equivalence(equivalence.clone()),
                    None,
                )
                .await
                .expect("source frame should capture");
            let source_job = source.job.expect("source frame should enqueue OCR");
            complete_job(
                &infra,
                source_job,
                ProcessingResultDraft::new().with_result_text("redacted shared target"),
            )
            .await;
            let source_result_id: i64 = sqlx::query_scalar(
                "SELECT id FROM processing_results WHERE subject_type = ?1 AND subject_id = ?2 ORDER BY id DESC LIMIT 1",
            )
            .bind(FRAME_SUBJECT_TYPE)
            .bind(source.frame.id)
            .fetch_one(infra.pool())
            .await
            .expect("source result should exist");
            sqlx::query(
                "INSERT INTO secret_redactions \
                    (anchor_type, frame_id, audio_segment_id, processing_result_id, category, redacted_start, redacted_end, detector_version) \
                 VALUES ('frame', ?1, NULL, ?2, 'api_key', 0, 8, 'test')",
            )
            .bind(source.frame.id)
            .bind(source_result_id)
            .execute(infra.pool())
            .await
            .expect("source redaction should insert");

            let target = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-redaction-target.jpg",
                        "2026-05-17T10:00:01Z",
                    )
                    .with_equivalence(equivalence),
                    None,
                )
                .await
                .expect("target frame should capture");
            assert!(target.job.is_none());

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "redacted".to_string(),
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
            let reuse = response
                .frames
                .iter()
                .find(|result| result.text_source_kind == "equivalent_reuse")
                .expect("equivalent reuse result should exist");

            assert_eq!(reuse.representative_frame.id, target.frame.id);
            assert_eq!(reuse.secret_redaction_count, 1);
            assert!(reuse.has_secret_redactions);
        });
    }

    #[test]
    fn source_ocr_projection_respects_hidden_workspace_equivalence_scope() {
        run_async_test(async {
            let dir = test_dir("reuse-hidden-scope");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let equivalence = crate::FrameEquivalence {
                hint: Some("same-scope".to_string()),
                proof: Some(vec![23; 1024]),
                version: Some(1),
                status: Some(crate::FrameEquivalenceStatus::Ready),
                error: None,
            };
            let source = infra
                .insert_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-hidden-scope-source.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_equivalence(equivalence.clone()),
                )
                .await
                .expect("source frame should insert");
            let hidden_frame_path = dir
                .join("recordings/2026/05/17/.screen-session-segment-0001/frames/frame-1.jpg")
                .to_string_lossy()
                .to_string();
            infra
                .insert_frame(
                    &NewFrame::new("screen-session", &hidden_frame_path, "2026-05-17T10:00:01Z")
                        .with_equivalence(equivalence),
                )
                .await
                .expect("hidden frame should insert");

            let job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(source.id))
                .await
                .expect("source job should enqueue");
            complete_job(
                &infra,
                job,
                ProcessingResultDraft::new().with_result_text("scope reuse target"),
            )
            .await;

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "scope".to_string(),
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
            assert_eq!(response.frames[0].match_count, 1);
            assert_eq!(response.frames[0].representative_frame.id, source.id);
            assert_eq!(response.frames[0].text_source_kind, "direct");
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
    fn equivalent_reuse_search_survives_source_result_delete() {
        run_async_test(async {
            let dir = test_dir("reuse-source-delete");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let equivalence = crate::FrameEquivalence {
                hint: Some("same-screen".to_string()),
                proof: Some(vec![17; 1024]),
                version: Some(1),
                status: Some(crate::FrameEquivalenceStatus::Ready),
                error: None,
            };
            let first = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-reuse-source.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_equivalence(equivalence.clone()),
                    None,
                )
                .await
                .expect("first frame should capture");
            let source_job = first.job.expect("first frame should enqueue OCR");
            let source_job_id = source_job.id;
            complete_job(
                &infra,
                source_job,
                ProcessingResultDraft::new().with_result_text("retained reuse target"),
            )
            .await;

            let second = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/search-reuse-target.jpg",
                        "2026-05-17T10:00:02Z",
                    )
                    .with_equivalence(equivalence),
                    None,
                )
                .await
                .expect("second frame should capture");
            assert!(second.job.is_none());

            sqlx::query("DELETE FROM processing_results WHERE job_id = ?1")
                .bind(source_job_id)
                .execute(infra.pool())
                .await
                .expect("source processing result delete should not remove reuse search");

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "retained".to_string(),
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
            assert_eq!(response.frames[0].representative_frame.id, second.frame.id);
            assert_eq!(response.frames[0].text_source_kind, "equivalent_reuse");
        });
    }

    #[test]
    fn audio_search_aligns_to_near_earlier_frame() {
        run_async_test(async {
            let dir = test_dir("audio-alignment-near-earlier");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let frame = infra
                .insert_frame(&NewFrame::new(
                    "shared-session",
                    "/tmp/alignment-near-frame.jpg",
                    "2026-05-17T10:00:56Z",
                ))
                .await
                .expect("frame should insert");
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "shared-session",
                    1,
                    "/tmp/search-audio-alignment-near.m4a",
                    "2026-05-17T10:01:00Z",
                    "2026-05-17T10:01:20Z",
                ))
                .await
                .expect("segment should insert");
            let metadata = TranscriptionMetadata {
                provider: "test".to_string(),
                model_id: None,
                language: "en".to_string(),
                segments: vec![TranscriptionSegment {
                    start_ms: 1_000,
                    end_ms: 2_500,
                    text: "alignment target phrase".to_string(),
                    confidence: None,
                }],
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
                    .with_result_text("alignment target phrase")
                    .with_structured_payload_json(
                        serde_json::to_string(&metadata).expect("metadata should serialize"),
                    ),
            )
            .await;

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "alignment".to_string(),
                    frame_limit: Some(0),
                    frame_offset: None,
                    audio_limit: Some(5),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.audio.len(), 1);
            assert_eq!(
                response.audio[0]
                    .aligned_frame
                    .as_ref()
                    .map(|frame| frame.id),
                Some(frame.id)
            );
        });
    }

    #[test]
    fn audio_search_does_not_align_stale_earlier_frame() {
        run_async_test(async {
            let dir = test_dir("audio-alignment-stale-earlier");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            infra
                .insert_frame(&NewFrame::new(
                    "shared-session",
                    "/tmp/alignment-frame.jpg",
                    "2026-05-17T10:00:00Z",
                ))
                .await
                .expect("frame should insert");
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "shared-session",
                    1,
                    "/tmp/search-audio-alignment.m4a",
                    "2026-05-17T10:01:00Z",
                    "2026-05-17T10:01:20Z",
                ))
                .await
                .expect("segment should insert");
            let metadata = TranscriptionMetadata {
                provider: "test".to_string(),
                model_id: None,
                language: "en".to_string(),
                segments: vec![TranscriptionSegment {
                    start_ms: 1_000,
                    end_ms: 2_500,
                    text: "alignment target phrase".to_string(),
                    confidence: None,
                }],
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
                    .with_result_text("alignment target phrase")
                    .with_structured_payload_json(
                        serde_json::to_string(&metadata).expect("metadata should serialize"),
                    ),
            )
            .await;

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "alignment".to_string(),
                    frame_limit: Some(0),
                    frame_offset: None,
                    audio_limit: Some(5),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert_eq!(response.audio.len(), 1);
            assert_eq!(
                response.audio[0]
                    .aligned_frame
                    .as_ref()
                    .map(|frame| frame.id),
                None
            );
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
    fn cascaded_search_document_deletes_remove_fts_rows() {
        run_async_test(async {
            let dir = test_dir("fts-cascade");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let frame = infra
                .insert_frame(&NewFrame::new(
                    "screen-session",
                    "/tmp/search-fts-cascade.jpg",
                    "2026-05-17T10:00:00Z",
                ))
                .await
                .expect("frame should insert");
            let job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
                .await
                .expect("ocr job should enqueue");
            let job_id = job.id;
            complete_job(
                &infra,
                job,
                ProcessingResultDraft::new().with_result_text("cascade target phrase"),
            )
            .await;

            let count_before: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM search_documents_fts WHERE search_documents_fts MATCH 'cascade'",
            )
            .fetch_one(infra.pool())
            .await
            .expect("fts count should query");
            assert_eq!(count_before, 1);

            sqlx::query("DELETE FROM processing_results WHERE job_id = ?1")
                .bind(job_id)
                .execute(infra.pool())
                .await
                .expect("processing result delete should cascade");

            let count_after: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM search_documents_fts WHERE search_documents_fts MATCH 'cascade'",
            )
            .fetch_one(infra.pool())
            .await
            .expect("fts count should query");
            assert_eq!(count_after, 0);
        });
    }

    #[test]
    fn replacing_search_projection_keeps_fts_delete_trigger_idempotent() {
        run_async_test(async {
            let dir = test_dir("fts-replace");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let frame = infra
                .insert_frame(&NewFrame::new(
                    "screen-session",
                    "/tmp/search-fts-replace.jpg",
                    "2026-05-17T10:00:00Z",
                ))
                .await
                .expect("frame should insert");

            for text in ["first target phrase", "second target phrase"] {
                let job = infra
                    .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
                    .await
                    .expect("ocr job should enqueue");
                complete_job(
                    &infra,
                    job,
                    ProcessingResultDraft::new().with_result_text(text),
                )
                .await;
            }

            let first = infra
                .search_capture(SearchCaptureRequest {
                    query: "first".to_string(),
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
            let second = infra
                .search_capture(SearchCaptureRequest {
                    query: "second".to_string(),
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

            assert!(first.frames.is_empty());
            assert_eq!(second.frames.len(), 1);
        });
    }

    #[test]
    fn frame_search_does_not_group_same_hint_with_different_proofs() {
        run_async_test(async {
            let dir = test_dir("frame-proof-grouping");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");

            for (path, proof) in [
                ("/tmp/search-proof-a.jpg", vec![0; 1024]),
                ("/tmp/search-proof-b.jpg", vec![255; 1024]),
            ] {
                let frame = infra
                    .insert_frame(
                        &NewFrame::new("screen-session", path, "2026-05-17T10:00:00Z")
                            .with_equivalence(crate::FrameEquivalence {
                                hint: Some("same-hint".to_string()),
                                proof: Some(proof),
                                version: Some(1),
                                status: Some(crate::FrameEquivalenceStatus::Ready),
                                error: None,
                            }),
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
                    ProcessingResultDraft::new().with_result_text("proof target phrase"),
                )
                .await;
            }

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "proof".to_string(),
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
            let mut group_keys = response
                .frames
                .iter()
                .map(|frame| frame.group_key.as_str())
                .collect::<Vec<_>>();
            group_keys.sort_unstable();
            group_keys.dedup();
            assert_eq!(group_keys.len(), 2);
            assert_eq!(
                response
                    .frames
                    .iter()
                    .map(|frame| frame.match_count)
                    .collect::<Vec<_>>(),
                vec![1, 1]
            );
        });
    }

    #[test]
    fn frame_search_does_not_group_equivalent_proofs_across_hidden_workspaces() {
        run_async_test(async {
            let dir = test_dir("frame-hidden-workspace-grouping");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let equivalence = crate::FrameEquivalence {
                hint: Some("same-hidden-proof".to_string()),
                proof: Some(vec![31; 1024]),
                version: Some(1),
                status: Some(crate::FrameEquivalenceStatus::Ready),
                error: None,
            };

            for (index, segment) in ["0001", "0002"].into_iter().enumerate() {
                let frame_path = dir
                    .join(format!(
                        "recordings/2026/05/17/.screen-session-segment-{segment}/frames/frame-1.jpg"
                    ))
                    .to_string_lossy()
                    .to_string();
                let frame = infra
                    .insert_frame(
                        &NewFrame::new(
                            "screen-session",
                            &frame_path,
                            &format!("2026-05-17T10:00:0{index}Z"),
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
                    ProcessingResultDraft::new().with_result_text("hidden scope phrase"),
                )
                .await;
            }

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "hidden".to_string(),
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
            let mut group_keys = response
                .frames
                .iter()
                .map(|frame| frame.group_key.as_str())
                .collect::<Vec<_>>();
            group_keys.sort_unstable();
            group_keys.dedup();
            assert_eq!(group_keys.len(), 2);
            assert_eq!(
                response
                    .frames
                    .iter()
                    .map(|frame| frame.match_count)
                    .collect::<Vec<_>>(),
                vec![1, 1]
            );
        });
    }

    // === Search Query Syntax parser tests (ADR 0019, A7) ===

    fn frame_with_app(
        bundle_id: Option<&str>,
        name: Option<&str>,
    ) -> capture_metadata::FrameMetadataSnapshot {
        capture_metadata::FrameMetadataSnapshot {
            app_bundle_id: bundle_id.map(str::to_string),
            app_name: name.map(str::to_string),
            window_title: None,
            window_id: None,
            browser_url: None,
            display_id: Some(1),
            metadata_redaction_reason: None,
            metadata_redaction_source_id: None,
        }
    }

    async fn seed_frame_with_text(
        infra: &AppInfra,
        path: &str,
        captured_at: &str,
        metadata: Option<capture_metadata::FrameMetadataSnapshot>,
        text: &str,
    ) -> crate::Frame {
        let mut new_frame = NewFrame::new("screen-session", path, captured_at);
        if let Some(metadata) = metadata {
            new_frame = new_frame.with_metadata_snapshot(metadata);
        }
        let frame = infra
            .insert_frame(&new_frame)
            .await
            .expect("frame should insert");
        let job = infra
            .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
            .await
            .expect("ocr job should enqueue");
        complete_job(
            infra,
            job,
            ProcessingResultDraft::new().with_result_text(text),
        )
        .await;
        frame
    }

    #[test]
    fn plain_text_search_still_works_end_to_end() {
        run_async_test(async {
            let dir = test_dir("plain-text-still-works");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let frame = seed_frame_with_text(
                &infra,
                "/tmp/plain-text-target.jpg",
                "2026-05-17T10:00:00Z",
                None,
                "the quarterly planning notes",
            )
            .await;

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "quarterly planning".to_string(),
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

            assert!(response.parse_errors.is_empty());
            assert_eq!(response.residual_query, "quarterly planning");
            assert_eq!(response.frames.len(), 1);
            assert_eq!(response.frames[0].representative_frame.id, frame.id);
        });
    }

    #[test]
    fn app_operator_desugars_into_refinement_end_to_end() {
        run_async_test(async {
            let dir = test_dir("app-operator-e2e");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let safari = seed_frame_with_text(
                &infra,
                "/tmp/app-op-safari.jpg",
                "2026-05-17T10:00:00Z",
                Some(frame_with_app(Some("com.apple.Safari"), Some("Safari"))),
                "shared target text",
            )
            .await;
            let _chrome = seed_frame_with_text(
                &infra,
                "/tmp/app-op-chrome.jpg",
                "2026-05-17T10:01:00Z",
                Some(frame_with_app(Some("com.google.Chrome"), Some("Chrome"))),
                "shared target text",
            )
            .await;

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "app:Safari target".to_string(),
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

            assert!(response.parse_errors.is_empty());
            assert_eq!(response.residual_query, "target");
            assert_eq!(response.applied_refinements.apps.len(), 1);
            assert_eq!(response.frames.len(), 1);
            assert_eq!(response.frames[0].representative_frame.id, safari.id);
        });
    }

    #[test]
    fn multi_app_operator_accumulates_as_or_end_to_end() {
        run_async_test(async {
            let dir = test_dir("multi-app-operator-e2e");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let safari = seed_frame_with_text(
                &infra,
                "/tmp/multi-app-safari.jpg",
                "2026-05-17T10:00:00Z",
                Some(frame_with_app(Some("com.apple.Safari"), Some("Safari"))),
                "shared target text",
            )
            .await;
            let chrome = seed_frame_with_text(
                &infra,
                "/tmp/multi-app-chrome.jpg",
                "2026-05-17T10:01:00Z",
                Some(frame_with_app(Some("com.google.Chrome"), Some("Chrome"))),
                "shared target text",
            )
            .await;
            let _notes = seed_frame_with_text(
                &infra,
                "/tmp/multi-app-notes.jpg",
                "2026-05-17T10:02:00Z",
                Some(frame_with_app(Some("com.apple.Notes"), Some("Notes"))),
                "shared target text",
            )
            .await;

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "app:Safari app:Chrome target".to_string(),
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

            assert!(response.parse_errors.is_empty());
            assert_eq!(response.applied_refinements.apps.len(), 2);
            let mut ids = response
                .frames
                .iter()
                .map(|frame| frame.representative_frame.id)
                .collect::<Vec<_>>();
            ids.sort_unstable();
            let mut expected = vec![safari.id, chrome.id];
            expected.sort_unstable();
            assert_eq!(ids, expected);
        });
    }

    #[test]
    fn source_operator_desugars_into_audio_refinement_end_to_end() {
        run_async_test(async {
            let dir = test_dir("source-operator-e2e");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let mic = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "mic-session",
                    1,
                    "/tmp/source-op-mic.m4a",
                    "2026-05-17T10:00:00Z",
                    "2026-05-17T10:00:10Z",
                ))
                .await
                .expect("mic segment should insert");
            let system = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::SystemAudio,
                    "system-session",
                    1,
                    "/tmp/source-op-system.m4a",
                    "2026-05-17T10:00:00Z",
                    "2026-05-17T10:00:10Z",
                ))
                .await
                .expect("system segment should insert");
            for segment in [&mic, &system] {
                let job = infra
                    .enqueue_processing_job(&ProcessingJobDraft::for_audio_segment_transcription(
                        segment.id,
                    ))
                    .await
                    .expect("transcription job should enqueue");
                complete_job(
                    &infra,
                    job,
                    ProcessingResultDraft::new().with_result_text("spoken target audio"),
                )
                .await;
            }

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "source:mic source:system target".to_string(),
                    frame_limit: Some(0),
                    frame_offset: None,
                    audio_limit: Some(10),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                    query_embedding: None,
                })
                .await
                .expect("search should succeed");

            assert!(response.parse_errors.is_empty());
            assert_eq!(response.residual_query, "target");
            assert_eq!(response.applied_refinements.audio_sources.len(), 2);
            assert_eq!(response.audio.len(), 2);
        });
    }

    #[test]
    fn body_phrase_operator_filters_end_to_end() {
        run_async_test(async {
            let dir = test_dir("body-phrase-e2e");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            let exact = seed_frame_with_text(
                &infra,
                "/tmp/body-phrase-exact.jpg",
                "2026-05-17T10:00:00Z",
                None,
                "the quarterly planning meeting",
            )
            .await;
            let _scrambled = seed_frame_with_text(
                &infra,
                "/tmp/body-phrase-scrambled.jpg",
                "2026-05-17T10:01:00Z",
                None,
                "planning the quarterly meeting",
            )
            .await;

            let response = infra
                .search_capture(SearchCaptureRequest {
                    query: "\"quarterly planning\"".to_string(),
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

            assert!(response.parse_errors.is_empty());
            assert_eq!(response.frames.len(), 1);
            assert_eq!(response.frames[0].representative_frame.id, exact.id);
        });
    }

    #[test]
    fn in_band_errors_suppress_results_without_throwing() {
        run_async_test(async {
            let dir = test_dir("in-band-errors");
            let infra = AppInfra::initialize(&dir)
                .await
                .expect("infra should initialize");
            seed_frame_with_text(
                &infra,
                "/tmp/in-band-frame.jpg",
                "2026-05-17T10:00:00Z",
                Some(frame_with_app(Some("com.apple.Safari"), Some("Safari"))),
                "target text everywhere",
            )
            .await;

            // Each of these is a strict mistake that must come back in
            // parse_errors with results suppressed, never as a thrown Err.
            let cases: &[(&str, &str)] = &[
                ("after:notadate target", "bad_date"),
                ("\"unterminated target", "unbalanced_quote"),
                ("app:Safari source:mic target", "app_source_conflict"),
                ("-target", "pure_negation"),
            ];

            for (query, expected_kind) in cases {
                let response = infra
                    .search_capture(SearchCaptureRequest {
                        query: query.to_string(),
                        frame_limit: Some(10),
                        frame_offset: None,
                        audio_limit: Some(10),
                        audio_offset: None,
                        snapshot_document_id: None,
                        refinements: None,
                        query_embedding: None,
                    })
                    .await
                    .unwrap_or_else(|error| panic!("`{query}` should not throw, got {error:?}"));

                assert!(
                    response
                        .parse_errors
                        .iter()
                        .any(|error| &error.kind == expected_kind),
                    "`{query}` should surface a {expected_kind} parse error, got {:?}",
                    response.parse_errors
                );
                assert!(
                    response.frames.is_empty() && response.audio.is_empty(),
                    "`{query}` should suppress results, got {} frames / {} audio",
                    response.frames.len(),
                    response.audio.len()
                );
            }
        });
    }

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

    // ----------------------------------------------------------------------
    // Hybrid Search: filter-then-rank + RRF fusion + "found by meaning" snippet
    // ----------------------------------------------------------------------

    /// The default English tier (`nomic-embed-text-v1.5`) embedding width — the
    /// dimension of the slice-1 `search_document_vectors vec0(embedding float[768])`
    /// table. Test vectors must match it or `store_vector` errors.
    const TEST_EMBED_DIM: usize = 768;

    /// Build a deterministic unit f32 vector keyed to `seed`, so two distinct
    /// seeds are far apart in L2 distance and a query close to one seed's vector
    /// is nearest to that anchor's stored vector under the brute-force KNN.
    fn seeded_vector(seed: usize) -> Vec<f32> {
        let mut v = vec![0.0_f32; TEST_EMBED_DIM];
        // One-hot in a slot chosen by the seed: orthogonal vectors are maximally
        // separated, so KNN nearest-neighbor order is unambiguous.
        v[seed % TEST_EMBED_DIM] = 1.0;
        v
    }

    /// Seed a `direct` frame anchor with OCR `text` and return its
    /// `search_documents.id` (the `vec0` rowid). The completed OCR projects the
    /// anchor on write, exactly as production does.
    async fn seed_frame_anchor(infra: &AppInfra, captured_at: &str, text: &str) -> i64 {
        let frame = infra
            .insert_frame(&NewFrame::new(
                "screen-session",
                &format!("/tmp/hybrid-{captured_at}.jpg"),
                captured_at,
            ))
            .await
            .expect("frame should insert");
        let job = infra
            .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
            .await
            .expect("ocr job should enqueue");
        complete_job(
            infra,
            job,
            ProcessingResultDraft::new().with_result_text(text),
        )
        .await;
        sqlx::query_scalar::<_, i64>(
            "SELECT id FROM search_documents WHERE frame_id = ?1 AND text_source_kind = 'direct' LIMIT 1",
        )
        .bind(frame.id)
        .fetch_one(infra.pool())
        .await
        .expect("direct anchor id should load")
    }

    /// Seed a `direct` audio (transcription) anchor with transcript `text` and
    /// return its `search_documents.id` (the `vec0` rowid). A single-segment
    /// transcription projects one `direct` `anchor_type = 'audio'` document on
    /// write, exactly as production does — the audio counterpart of
    /// [`seed_frame_anchor`].
    async fn seed_audio_anchor(infra: &AppInfra, started_at: &str, ended_at: &str, text: &str) -> i64 {
        let segment = infra
            .upsert_audio_segment(&NewAudioSegment::new(
                AudioSegmentSourceKind::Microphone,
                "mic-session",
                1,
                &format!("/tmp/hybrid-audio-{started_at}.m4a"),
                started_at,
                ended_at,
            ))
            .await
            .expect("segment should insert");
        let metadata = TranscriptionMetadata {
            provider: "test".to_string(),
            model_id: None,
            language: "en".to_string(),
            segments: vec![TranscriptionSegment {
                start_ms: 0,
                end_ms: 2_000,
                text: text.to_string(),
                confidence: None,
            }],
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
            infra,
            job,
            ProcessingResultDraft::new()
                .with_result_text(text)
                .with_structured_payload_json(
                    serde_json::to_string(&metadata).expect("metadata should serialize"),
                ),
        )
        .await;
        sqlx::query_scalar::<_, i64>(
            "SELECT id FROM search_documents \
             WHERE audio_segment_id = ?1 AND text_source_kind = 'direct' AND anchor_type = 'audio' LIMIT 1",
        )
        .bind(segment.id)
        .fetch_one(infra.pool())
        .await
        .expect("direct audio anchor id should load")
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
        assert!(bounded.ends_with('…'), "a truncated excerpt ends with an ellipsis");
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

    fn frame_hit_for_fusion(anchor_id: i64, snippet: &str, found_by_meaning: bool) -> FrameHit {
        FrameHit {
            anchor_id,
            group_key: format!("frame:{anchor_id}"),
            frame: Frame {
                id: anchor_id,
                session_id: "screen-session".to_string(),
                file_path: format!("/tmp/fuse-{anchor_id}.jpg"),
                captured_at: "2026-05-17T10:00:00Z".to_string(),
                width: None,
                height: None,
                equivalence: crate::FrameEquivalence {
                    hint: None,
                    proof: None,
                    version: None,
                    status: None,
                    error: None,
                },
                metadata_snapshot: None,
                created_at: "2026-05-17T10:00:00Z".to_string(),
                updated_at: "2026-05-17T10:00:00Z".to_string(),
            },
            snippet: snippet.to_string(),
            rank: 0.0,
            app_bundle_id: None,
            app_name: None,
            window_title: None,
            text_source_kind: "direct".to_string(),
            secret_redaction_count: 0,
            found_by_meaning,
        }
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
            assert!(frame_ids.iter().any(|&id| id == keyword_result.thumbnail_frame_id));
        });
    }

    /// The audio counterpart of the frame fusion test, and the regression guard
    /// for C1: a semantic **audio** hit must surface `found_by_meaning` without
    /// panicking. Every other hybrid test seeds frame anchors only and sets
    /// `audio_limit: Some(0)`, so the semantic audio path — which the always-on
    /// sweep exercises in the default steady state once any transcription anchor
    /// is embedded — was never covered.
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

    /// Degrade-to-keyword-only (H1): when the semantic fetch cannot run — here a
    /// query embedding whose width disagrees with the live `vec0` column, the
    /// dominant real-world failure during/after a model switch — the whole search
    /// must NOT fail. It must return the keyword (FTS) hits, never an `Err`. This
    /// is the ADR-0036 "no usable runtime → keyword-only, no regression" promise:
    /// a dimension disagreement degrades, it does not break search.
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

    async fn seed_frame_anchor_with_app(
        infra: &AppInfra,
        captured_at: &str,
        text: &str,
        bundle_id: &str,
        app_name: &str,
    ) -> i64 {
        let frame = infra
            .insert_frame(
                &NewFrame::new(
                    "screen-session",
                    &format!("/tmp/hybrid-app-{captured_at}.jpg"),
                    captured_at,
                )
                .with_metadata_snapshot(capture_metadata::FrameMetadataSnapshot {
                    app_bundle_id: Some(bundle_id.to_string()),
                    app_name: Some(app_name.to_string()),
                    window_title: None,
                    window_id: None,
                    browser_url: None,
                    display_id: None,
                    metadata_redaction_reason: None,
                    metadata_redaction_source_id: None,
                }),
            )
            .await
            .expect("frame should insert");
        let job = infra
            .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
            .await
            .expect("ocr job should enqueue");
        complete_job(
            infra,
            job,
            ProcessingResultDraft::new().with_result_text(text),
        )
        .await;
        sqlx::query_scalar::<_, i64>(
            "SELECT id FROM search_documents WHERE frame_id = ?1 AND text_source_kind = 'direct' LIMIT 1",
        )
        .bind(frame.id)
        .fetch_one(infra.pool())
        .await
        .expect("direct anchor id should load")
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
