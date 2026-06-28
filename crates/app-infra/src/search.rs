use audio_transcription::TranscriptionMetadata;
use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqliteRow, Executor, QueryBuilder, Row, Sqlite, SqlitePool, Transaction};
use time::{format_description::well_known::Rfc3339, Duration, OffsetDateTime, UtcOffset};

use crate::db::CaptureDb;
use crate::{
    captured_frame_equivalence::CapturedFrameEquivalenceScope,
    processing::{map_frame_for_search, Frame},
    AppInfraError, AudioSegment, AudioSegmentSourceKind, ProcessingResult, Result,
    AUDIO_SEGMENT_SUBJECT_TYPE, AUDIO_TRANSCRIPTION_PROCESSOR, FRAME_SUBJECT_TYPE, OCR_PROCESSOR,
};

const DEFAULT_GROUP_LIMIT: u32 = 5;
const MAX_GROUP_LIMIT: u32 = 50;
const MIN_HIT_FETCH_LIMIT: i64 = 250;
const MAX_HIT_FETCH_LIMIT: i64 = 5_000;
const HIT_FETCH_OVERFETCH_PER_GROUP: i64 = 50;
const AUDIO_GROUP_GAP_MS: u64 = 2_000;
const AUDIO_FRAME_ALIGNMENT_WINDOW_SECONDS: i64 = 10;

/// Reciprocal rank fusion constant. The textbook `k = 60` from the TREC RRF
/// paper (Cormack et al. 2009): it damps the contribution of low-ranked tail
/// hits so the top of each list dominates the fused order without one list's
/// long tail swamping the other's head. `1 / (k + rank)` per list, summed.
const RRF_K: f64 = 60.0;

/// How many nearest **Semantic Search Vector**s the `vec0` KNN returns per query.
/// This is the meaning-tier candidate budget that RRF fuses with the **Text
/// Search** list; it is intentionally generous (there is no ANN in v1, so the
/// scan is already a filtered brute force) but bounded so an unrefined query
/// over a large index stays a fixed-cost top-k rather than a full table scan.
const SEMANTIC_KNN_LIMIT: i64 = 200;

/// Character budget for a meaning-only **Search Snippet**: the leading
/// `body_text` excerpt rendered when a hit matched the query vector but carries
/// no **Text Search** term to highlight. Sized to roughly match the FTS
/// `snippet(...)` 12-token window so a "found by meaning" card reads at the same
/// length as a keyword card.
const MEANING_SNIPPET_CHAR_BUDGET: usize = 120;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SearchCaptureRequest {
    pub query: String,
    pub frame_limit: Option<u32>,
    pub frame_offset: Option<u32>,
    pub audio_limit: Option<u32>,
    pub audio_offset: Option<u32>,
    pub snapshot_document_id: Option<i64>,
    pub refinements: Option<SearchCaptureRefinements>,
    /// The **Semantic Search** query vector, pre-computed by the caller (the
    /// desktop layer embeds the query string with the loaded **Semantic Search
    /// Model**; app-infra takes no `ort`/`fastembed` dependency). When `Some`,
    /// **Hybrid Search** fuses a `vec0` KNN over this vector with the FTS5
    /// **Text Search** ranking by reciprocal rank fusion. When `None` — no model
    /// installed, no vectors, or a query that produced no embedding — search
    /// degrades to today's keyword-only behavior with no regression.
    #[serde(default)]
    pub query_embedding: Option<Vec<f32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchCaptureRefinements {
    pub date_range: Option<SearchDateRangeRefinement>,
    #[serde(default)]
    pub apps: Vec<SearchAppRefinement>,
    pub window_title: Option<String>,
    #[serde(default)]
    pub audio_sources: Vec<AudioSegmentSourceKind>,
    /// `source:screen` restricts results to captured frames (screen), skipping
    /// audio. It is the frame-side counterpart of `audio_sources` and cannot be
    /// combined with them.
    #[serde(default)]
    pub screen_source: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchDateRangeRefinement {
    pub start_at: String,
    pub end_at: String,
    pub origin: Option<SearchDateRangeOrigin>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchDateRangeOrigin {
    VisibleTimeline,
    Today,
    LastHour,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchAppRefinement {
    pub kind: SearchAppRefinementKind,
    pub value: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchAppRefinementKind {
    Any,
    BundleId,
    AppName,
}

/// A strict validation problem detected while interpreting [`SearchQuerySyntax`].
///
/// Parse errors are returned in-band on [`SearchCaptureResponse::parse_errors`]
/// rather than thrown, so a clear operator mistake surfaces an inline,
/// span-highlighted message instead of a misleading or empty search. Spans are
/// character (Unicode scalar) offsets into the original raw query.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchParseError {
    /// Machine-readable category (for example `bad_date`, `unbalanced_quote`).
    pub kind: String,
    /// Human-readable explanation for the search input.
    pub message: String,
    /// Character (Unicode scalar) offset of the start of the offending token.
    pub start: u32,
    /// Character (Unicode scalar) offset of the end of the offending token.
    pub end: u32,
    /// The original raw token text that triggered the error.
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchableApp {
    pub bundle_id: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchCaptureResponse {
    pub normalized_query: String,
    pub snapshot_document_id: i64,
    pub frames: Vec<FrameSearchResult>,
    pub audio: Vec<AudioSearchResult>,
    pub has_more_frames: bool,
    pub has_more_audio: bool,
    pub applied_refinements: SearchCaptureRefinements,
    /// The body query that remains after extracting field operators, i.e. the
    /// text that drives FTS matching once typed scope is desugared into chips.
    pub residual_query: String,
    /// Strict validation problems found while interpreting query syntax.
    pub parse_errors: Vec<SearchParseError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FrameSearchResult {
    pub group_key: String,
    pub representative_frame: Frame,
    pub group_start_at: String,
    pub group_end_at: String,
    pub match_count: u32,
    pub snippet: String,
    pub app_bundle_id: Option<String>,
    pub app_name: Option<String>,
    pub window_title: Option<String>,
    /// The representative frame's captured `browser_url` (raw, as recorded in the
    /// metadata snapshot). Read-time from the SAME representative frame whose
    /// `id` mints the opaque result id, so a consumer's guarded URL matches the
    /// result's landing frame. `None` when the frame had no browser URL. The
    /// broker boundary (not search) applies the read-time URL guard before
    /// exposing this to a consumer.
    #[serde(default)]
    pub browser_url: Option<String>,
    pub thumbnail_frame_id: i64,
    pub text_source_kind: String,
    pub secret_redaction_count: u32,
    pub has_secret_redactions: bool,
    /// A meaning-only **Semantic Search** hit: the group matched the query
    /// vector but no **Text Search** term, so `snippet` is a leading `body_text`
    /// excerpt tagged "found by meaning" rather than a highlighted FTS snippet.
    /// `false` whenever any grouped anchor also matched **Text Search**.
    #[serde(default)]
    pub found_by_meaning: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioSearchResult {
    pub group_key: String,
    pub audio_segment: AudioSegment,
    pub source_kind: AudioSegmentSourceKind,
    pub span_start_ms: u64,
    pub span_end_ms: u64,
    pub absolute_start_at: String,
    pub absolute_end_at: String,
    pub match_count: u32,
    pub snippet: String,
    pub aligned_frame: Option<Frame>,
    pub secret_redaction_count: u32,
    pub has_secret_redactions: bool,
    /// A meaning-only **Semantic Search** hit (see [`FrameSearchResult::found_by_meaning`]).
    #[serde(default)]
    pub found_by_meaning: bool,
}

#[derive(Debug, Clone)]
struct NormalizedSearchRefinements {
    date_range: Option<NormalizedDateRange>,
    apps: Vec<NormalizedAppRefinement>,
    window_title: Option<String>,
    audio_sources: Vec<AudioSegmentSourceKind>,
    screen_source: bool,
    applied: SearchCaptureRefinements,
}

#[derive(Debug, Clone)]
struct NormalizedDateRange {
    start_at: String,
    end_at: String,
}

#[derive(Debug, Clone)]
enum NormalizedAppRefinement {
    Any { value: String, search_key: String },
    BundleId { value: String },
    AppName { search_key: String },
}

#[derive(Clone)]
pub struct SearchStore {
    db: CaptureDb,
}

pub(crate) struct EquivalentReuseText {
    pub(crate) result_text: String,
    pub(crate) source_subject_type: String,
    pub(crate) source_subject_id: i64,
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
        let mut transaction = self.db.write().begin().await?;
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
        .fetch_all(&mut *transaction)
        .await?;

        for row in rows {
            project_processing_result_in_transaction(
                &mut transaction,
                &map_processing_result_for_search(row)?,
            )
            .await?;
        }
        backfill_missing_equivalent_reuse_projections(&mut transaction).await?;
        backfill_missing_app_bundle_id_projection(&mut transaction).await?;
        backfill_missing_app_name_search_key_projection(&mut transaction).await?;

        transaction.commit().await?;
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

/// Outcome of normalizing refinements: either the normalized form, or a set of
/// in-band parse errors that should suppress results without throwing.
type NormalizationOutcome = std::result::Result<NormalizedSearchRefinements, Vec<SearchParseError>>;

/// Builds a [`SearchParseError`] with a whole-query span. Used for refinement
/// problems that have no narrower token origin (for example the app/source
/// conflict, which spans the combination rather than one token).
fn whole_query_parse_error(kind: &str, message: impl Into<String>) -> SearchParseError {
    SearchParseError {
        kind: kind.to_string(),
        message: message.into(),
        start: 0,
        end: 0,
        token: String::new(),
    }
}

fn normalize_search_refinements(
    refinements: Option<SearchCaptureRefinements>,
) -> Result<NormalizationOutcome> {
    let refinements = refinements.unwrap_or_default();
    let screen_source = refinements.screen_source;
    let mut errors: Vec<SearchParseError> = Vec::new();

    if !refinements.apps.is_empty() && !refinements.audio_sources.is_empty() {
        errors.push(whole_query_parse_error(
            "app_source_conflict",
            "app and source operators cannot be combined: app narrows screen results while source narrows audio results",
        ));
    }

    if screen_source && !refinements.audio_sources.is_empty() {
        errors.push(whole_query_parse_error(
            "screen_audio_source_conflict",
            "source:screen cannot be combined with source:mic or source:system: screen narrows captured frames while those narrow audio",
        ));
    }

    let date_range = match refinements.date_range {
        Some(range) => match normalize_date_range_refinement(range) {
            Ok(resolved) => Some(resolved),
            Err(error) => {
                errors.push(error);
                None
            }
        },
        None => None,
    };

    let mut normalized_apps = Vec::new();
    let mut applied_apps = Vec::new();
    for app in refinements.apps {
        match normalize_app_refinement(app) {
            Ok((normalized, applied)) => {
                if !applied_apps.contains(&applied) {
                    normalized_apps.push(normalized);
                    applied_apps.push(applied);
                }
            }
            Err(error) => errors.push(error),
        }
    }

    let window_title = match refinements.window_title {
        Some(value) => {
            let value = value.trim().to_string();
            if value.is_empty() {
                errors.push(whole_query_parse_error(
                    "empty_value",
                    "windowTitle must be non-empty",
                ));
                None
            } else {
                Some(value)
            }
        }
        None => None,
    };

    let mut audio_sources = Vec::new();
    for source in refinements.audio_sources {
        if !audio_sources.contains(&source) {
            audio_sources.push(source);
        }
    }

    if !errors.is_empty() {
        return Ok(Err(errors));
    }

    Ok(Ok(NormalizedSearchRefinements {
        date_range: date_range
            .as_ref()
            .map(|(normalized, _)| normalized.clone()),
        apps: normalized_apps,
        window_title: window_title.clone(),
        audio_sources: audio_sources.clone(),
        screen_source,
        applied: SearchCaptureRefinements {
            date_range: date_range.map(|(_, applied)| applied),
            apps: applied_apps,
            window_title,
            audio_sources,
            screen_source,
        },
    }))
}

fn normalize_date_range_refinement(
    range: SearchDateRangeRefinement,
) -> std::result::Result<(NormalizedDateRange, SearchDateRangeRefinement), SearchParseError> {
    let start = OffsetDateTime::parse(range.start_at.trim(), &Rfc3339).map_err(|_| {
        whole_query_parse_error(
            "bad_date",
            "date range start must be a valid RFC3339 timestamp",
        )
    })?;
    let end = OffsetDateTime::parse(range.end_at.trim(), &Rfc3339).map_err(|_| {
        whole_query_parse_error(
            "bad_date",
            "date range end must be a valid RFC3339 timestamp",
        )
    })?;
    if start > end {
        return Err(whole_query_parse_error(
            "bad_date",
            "date range start must be before or equal to date range end",
        ));
    }
    let start_at = format_rfc3339_for_search(start)
        .map_err(|error| whole_query_parse_error("bad_date", error.to_string()))?;
    let end_at = format_rfc3339_for_search(end)
        .map_err(|error| whole_query_parse_error("bad_date", error.to_string()))?;
    Ok((
        NormalizedDateRange {
            start_at: start_at.clone(),
            end_at: end_at.clone(),
        },
        SearchDateRangeRefinement {
            start_at,
            end_at,
            origin: range.origin,
        },
    ))
}

fn normalize_app_refinement(
    app: SearchAppRefinement,
) -> std::result::Result<(NormalizedAppRefinement, SearchAppRefinement), SearchParseError> {
    let value = app.value.trim().to_string();
    let display_name = app.display_name.trim().to_string();
    if value.is_empty() {
        return Err(whole_query_parse_error(
            "empty_value",
            "app value must be non-empty",
        ));
    }
    let normalized = match app.kind {
        SearchAppRefinementKind::Any => NormalizedAppRefinement::Any {
            value: value.clone(),
            search_key: normalize_app_name_for_search(&value).ok_or_else(|| {
                whole_query_parse_error("empty_value", "app value must be non-empty")
            })?,
        },
        SearchAppRefinementKind::BundleId => NormalizedAppRefinement::BundleId {
            value: value.clone(),
        },
        SearchAppRefinementKind::AppName => NormalizedAppRefinement::AppName {
            search_key: normalize_app_name_for_search(&value).ok_or_else(|| {
                whole_query_parse_error("empty_value", "app value must be non-empty")
            })?,
        },
    };
    Ok((
        normalized,
        SearchAppRefinement {
            kind: app.kind,
            value,
            display_name: if display_name.is_empty() {
                app.value.trim().to_string()
            } else {
                display_name
            },
        },
    ))
}

fn format_rfc3339_for_search(value: OffsetDateTime) -> Result<String> {
    value
        .to_offset(UtcOffset::UTC)
        .format(&Rfc3339)
        .map_err(|error| AppInfraError::InvalidSearchRequest(error.to_string()))
}

pub(crate) async fn project_processing_result_in_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
    result: &ProcessingResult,
) -> Result<()> {
    delete_projection_for_subject_processor(
        transaction,
        &result.subject_type,
        result.subject_id,
        &result.processor,
    )
    .await?;

    if result.processor == OCR_PROCESSOR && result.subject_type == FRAME_SUBJECT_TYPE {
        project_frame_ocr_result(transaction, result).await?;
    } else if result.processor == AUDIO_TRANSCRIPTION_PROCESSOR
        && result.subject_type == AUDIO_SEGMENT_SUBJECT_TYPE
    {
        project_audio_transcription_result(transaction, result).await?;
    }

    Ok(())
}

async fn delete_projection_for_subject_processor(
    transaction: &mut Transaction<'_, Sqlite>,
    subject_type: &str,
    subject_id: i64,
    processor: &str,
) -> Result<()> {
    sqlx::query(
        "DELETE FROM search_documents \
         WHERE anchor_type = CASE WHEN ?1 = 'frame' THEN 'frame' ELSE 'audio' END \
           AND ((?1 = 'frame' AND frame_id = ?2) OR (?1 = 'audio_segment' AND audio_segment_id = ?2))\
           AND processing_result_id IN (SELECT id FROM processing_results WHERE processor = ?3)",
    )
    .bind(subject_type)
    .bind(subject_id)
    .bind(processor)
    .execute(&mut **transaction)
    .await?;

    Ok(())
}

async fn backfill_missing_equivalent_reuse_projections(
    transaction: &mut Transaction<'_, Sqlite>,
) -> Result<()> {
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
            WHERE subject_type = ?1 AND processor = ?2 \
            GROUP BY subject_type, subject_id, processor\
         ) latest_results ON latest_results.id = processing_results.id \
         JOIN frames AS source_frames ON source_frames.id = processing_results.subject_id \
         WHERE LENGTH(TRIM(COALESCE(processing_results.result_text, ''))) > 0 \
           AND source_frames.equivalence_status = 'ready' \
           AND source_frames.equivalence_hint IS NOT NULL \
           AND source_frames.equivalence_proof IS NOT NULL \
           AND source_frames.equivalence_version IS NOT NULL \
           AND EXISTS (\
                SELECT 1 FROM frames AS target_frames \
                WHERE target_frames.session_id = source_frames.session_id \
                  AND target_frames.id != source_frames.id \
                  AND target_frames.equivalence_status = 'ready' \
                  AND target_frames.equivalence_hint = source_frames.equivalence_hint \
                  AND NOT EXISTS (\
                        SELECT 1 FROM search_documents AS direct_docs \
                        WHERE direct_docs.anchor_type = 'frame' \
                          AND direct_docs.frame_id = target_frames.id \
                          AND direct_docs.text_source_kind = 'direct'\
                  ) \
                  AND NOT EXISTS (\
                        SELECT 1 FROM search_documents AS reuse_docs \
                        WHERE reuse_docs.anchor_type = 'frame' \
                          AND reuse_docs.frame_id = target_frames.id \
                          AND reuse_docs.text_source_kind = 'equivalent_reuse'\
                  )\
           ) \
         ORDER BY processing_results.id ASC",
    )
    .bind(FRAME_SUBJECT_TYPE)
    .bind(OCR_PROCESSOR)
    .fetch_all(&mut **transaction)
    .await?;

    for row in rows {
        project_missing_equivalent_reuse_documents_for_processing_result(
            transaction,
            &map_processing_result_for_search(row)?,
        )
        .await?;
    }

    Ok(())
}

async fn backfill_missing_app_bundle_id_projection(
    transaction: &mut Transaction<'_, Sqlite>,
) -> Result<()> {
    let rows = sqlx::query(
        "SELECT search_documents.id, frame_metadata_snapshots.snapshot_json \
         FROM search_documents \
         JOIN frames ON frames.id = search_documents.frame_id \
         LEFT JOIN frame_metadata_snapshots ON frame_metadata_snapshots.id = frames.metadata_snapshot_id \
         WHERE search_documents.anchor_type = 'frame' \
           AND search_documents.app_bundle_id IS NULL",
    )
    .fetch_all(&mut **transaction)
    .await?;

    for row in rows {
        let bundle_id = row
            .get::<Option<String>, _>("snapshot_json")
            .map(|snapshot_json| {
                serde_json::from_str::<capture_metadata::FrameMetadataSnapshot>(&snapshot_json)
            })
            .transpose()?
            .and_then(|snapshot| snapshot.app_bundle_id)
            .and_then(|bundle_id| {
                normalize_app_bundle_id_for_search(&bundle_id).map(str::to_string)
            });
        if let Some(bundle_id) = bundle_id {
            sqlx::query("UPDATE search_documents SET app_bundle_id = ?1 WHERE id = ?2")
                .bind(bundle_id)
                .bind(row.get::<i64, _>("id"))
                .execute(&mut **transaction)
                .await?;
        } else {
            sqlx::query("UPDATE search_documents SET app_bundle_id = '' WHERE id = ?1")
                .bind(row.get::<i64, _>("id"))
                .execute(&mut **transaction)
                .await?;
        }
    }

    Ok(())
}

async fn backfill_missing_app_name_search_key_projection(
    transaction: &mut Transaction<'_, Sqlite>,
) -> Result<()> {
    let rows = sqlx::query(
        "SELECT id, app_name \
         FROM search_documents \
         WHERE app_name_search_key IS NULL",
    )
    .fetch_all(&mut **transaction)
    .await?;

    for row in rows {
        let id: i64 = row.get("id");
        let search_key = row
            .get::<Option<String>, _>("app_name")
            .as_deref()
            .and_then(normalize_app_name_for_search)
            .unwrap_or_default();
        sqlx::query("UPDATE search_documents SET app_name_search_key = ?1 WHERE id = ?2")
            .bind(search_key)
            .bind(id)
            .execute(&mut **transaction)
            .await?;
    }

    Ok(())
}

fn normalize_app_bundle_id_for_search(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn normalize_app_name_for_search(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_lowercase())
}

async fn project_frame_ocr_result(
    transaction: &mut Transaction<'_, Sqlite>,
    result: &ProcessingResult,
) -> Result<()> {
    let Some(frame) = get_frame_for_search_in_transaction(transaction, result.subject_id).await?
    else {
        return Ok(());
    };

    delete_equivalent_reuse_projections_for_source_result(transaction, result, &frame).await?;

    let Some(text) = result
        .result_text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    else {
        return Ok(());
    };

    let (app_bundle_id, app_name, window_title) = frame
        .metadata_snapshot
        .as_ref()
        .map(|metadata| {
            (
                metadata.app_bundle_id.clone(),
                metadata.app_name.clone(),
                metadata.window_title.clone(),
            )
        })
        .unwrap_or((None, None, None));

    let group_key = frame_search_group_key(&frame);
    let context_text = search_context_text(app_name.as_deref(), window_title.as_deref(), None);
    let app_name_search_key = app_name.as_deref().and_then(normalize_app_name_for_search);

    insert_search_document(
        transaction,
        NewSearchDocument {
            anchor_type: "frame",
            frame_id: Some(frame.id),
            audio_segment_id: None,
            processing_result_id: Some(result.id),
            span_start_ms: None,
            span_end_ms: None,
            absolute_start_at: &frame.captured_at,
            absolute_end_at: &frame.captured_at,
            source_kind: None,
            session_id: &frame.session_id,
            app_bundle_id: app_bundle_id.as_deref(),
            app_name: app_name.as_deref(),
            app_name_search_key: app_name_search_key.as_deref(),
            window_title: window_title.as_deref(),
            group_key: &group_key,
            text_source_kind: "direct",
            body_text: text,
            context_text: &context_text,
        },
    )
    .await?;

    project_equivalent_reuse_documents_for_source_frame(transaction, &frame, result.id, text).await
}

async fn project_missing_equivalent_reuse_documents_for_processing_result(
    transaction: &mut Transaction<'_, Sqlite>,
    result: &ProcessingResult,
) -> Result<()> {
    let Some(frame) = get_frame_for_search_in_transaction(transaction, result.subject_id).await?
    else {
        return Ok(());
    };

    let Some(text) = result
        .result_text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    else {
        return Ok(());
    };

    project_missing_equivalent_reuse_documents_for_source_frame(
        transaction,
        &frame,
        result.id,
        text,
    )
    .await
}

async fn delete_equivalent_reuse_projections_for_source_result(
    transaction: &mut Transaction<'_, Sqlite>,
    result: &ProcessingResult,
    source_frame: &Frame,
) -> Result<()> {
    sqlx::query(
        "DELETE FROM search_documents \
         WHERE text_source_kind = 'equivalent_reuse' \
           AND processing_result_id IN (\
                SELECT id FROM processing_results \
                WHERE subject_type = ?1 AND subject_id = ?2 AND processor = ?3\
           )",
    )
    .bind(&result.subject_type)
    .bind(result.subject_id)
    .bind(&result.processor)
    .execute(&mut **transaction)
    .await?;

    for frame in equivalent_reuse_candidate_frames(transaction, source_frame).await? {
        sqlx::query(
            "DELETE FROM search_documents \
             WHERE text_source_kind = 'equivalent_reuse' \
               AND anchor_type = 'frame' \
               AND frame_id = ?1",
        )
        .bind(frame.id)
        .execute(&mut **transaction)
        .await?;
    }

    delete_equivalent_reuse_projection_for_frame(transaction, source_frame.id).await?;

    Ok(())
}

pub(crate) async fn project_equivalent_frame_reuse_in_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
    frame: &Frame,
    related_frame_id: i64,
) -> Result<()> {
    let Some(source_doc) = sqlx::query(
        "SELECT search_documents.processing_result_id, \
                COALESCE(processing_results.result_text, search_documents.body_text) AS source_text \
         FROM search_documents \
         LEFT JOIN processing_results ON processing_results.id = search_documents.processing_result_id \
         WHERE search_documents.anchor_type = 'frame' \
           AND search_documents.frame_id = ?1 \
           AND (\
                search_documents.processing_result_id IS NULL \
                OR search_documents.processing_result_id IN (\
                    SELECT id FROM processing_results \
                    WHERE subject_type = 'frame' AND processor = ?2\
                )\
           ) \
         ORDER BY search_documents.id DESC LIMIT 1",
    )
    .bind(related_frame_id)
    .bind(OCR_PROCESSOR)
    .fetch_optional(&mut **transaction)
    .await?
    else {
        return Ok(());
    };

    project_equivalent_reuse_document_for_frame(
        transaction,
        frame,
        source_doc.get("processing_result_id"),
        source_doc.get::<String, _>("source_text").trim(),
    )
    .await
}

async fn project_equivalent_reuse_documents_for_source_frame(
    transaction: &mut Transaction<'_, Sqlite>,
    source_frame: &Frame,
    processing_result_id: i64,
    text: &str,
) -> Result<()> {
    let frames = equivalent_reuse_candidate_frames(transaction, source_frame).await?;

    for frame in frames {
        if frame_has_projection(transaction, frame.id, "direct").await? {
            continue;
        }
        project_equivalent_reuse_document_for_frame(
            transaction,
            &frame,
            Some(processing_result_id),
            text,
        )
        .await?;
    }

    Ok(())
}

async fn project_missing_equivalent_reuse_documents_for_source_frame(
    transaction: &mut Transaction<'_, Sqlite>,
    source_frame: &Frame,
    processing_result_id: i64,
    text: &str,
) -> Result<()> {
    let frames = equivalent_reuse_candidate_frames(transaction, source_frame).await?;

    for frame in frames {
        if frame_has_projection(transaction, frame.id, "direct").await?
            || frame_has_projection(transaction, frame.id, "equivalent_reuse").await?
        {
            continue;
        }
        project_equivalent_reuse_document_for_frame(
            transaction,
            &frame,
            Some(processing_result_id),
            text,
        )
        .await?;
    }

    Ok(())
}

async fn equivalent_reuse_candidate_frames(
    transaction: &mut Transaction<'_, Sqlite>,
    source_frame: &Frame,
) -> Result<Vec<Frame>> {
    let Some((hint, proof, version)) = source_frame.equivalence.ready_parts() else {
        return Ok(Vec::new());
    };

    let rows = sqlx::query(
        "SELECT frames.id, session_id, file_path, captured_at, width, height, \
                equivalence_hint, equivalence_proof, equivalence_version, equivalence_status, equivalence_error, \
                frame_metadata_snapshots.snapshot_json AS metadata_snapshot_json, \
                frames.created_at, frames.updated_at, \
                COALESCE((\
                    SELECT COUNT(*) FROM secret_redactions \
                    WHERE secret_redactions.anchor_type = 'frame' \
                      AND secret_redactions.frame_id = frames.id\
                ), 0) AS secret_redaction_count \
         FROM frames \
         LEFT JOIN frame_metadata_snapshots ON frame_metadata_snapshots.id = frames.metadata_snapshot_id \
         WHERE frames.session_id = ?1 \
           AND frames.id != ?2 \
           AND frames.equivalence_hint = ?3",
    )
    .bind(&source_frame.session_id)
    .bind(source_frame.id)
    .bind(hint)
    .fetch_all(&mut **transaction)
    .await?;

    let mut frames = Vec::new();
    for row in rows {
        let frame = map_frame_for_search(row)?;
        if !equivalent_reuse_scope_allows_source(&frame, source_frame) {
            continue;
        }
        let Some((_target_hint, target_proof, target_version)) = frame.equivalence.ready_parts()
        else {
            continue;
        };
        if target_version != version
            || !capture_screen::captured_frame_equivalence_proofs_match(
                version,
                proof,
                target_proof,
            )
        {
            continue;
        }
        frames.push(frame);
    }

    Ok(frames)
}

async fn get_frame_for_search_in_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
    frame_id: i64,
) -> Result<Option<Frame>> {
    let row = sqlx::query(
        "SELECT frames.id, session_id, file_path, captured_at, width, height, \
                equivalence_hint, equivalence_proof, equivalence_version, equivalence_status, equivalence_error, \
                frame_metadata_snapshots.snapshot_json AS metadata_snapshot_json, \
                frames.created_at, frames.updated_at \
         FROM frames \
         LEFT JOIN frame_metadata_snapshots ON frame_metadata_snapshots.id = frames.metadata_snapshot_id \
         WHERE frames.id = ?1",
    )
    .bind(frame_id)
    .fetch_optional(&mut **transaction)
    .await?;

    row.map(map_frame_for_search).transpose()
}

async fn frame_has_projection(
    transaction: &mut Transaction<'_, Sqlite>,
    frame_id: i64,
    text_source_kind: &str,
) -> Result<bool> {
    Ok(sqlx::query(
        "SELECT 1 FROM search_documents \
         WHERE search_documents.anchor_type = 'frame' \
           AND search_documents.frame_id = ?1 \
           AND search_documents.text_source_kind = ?2 \
         LIMIT 1",
    )
    .bind(frame_id)
    .bind(text_source_kind)
    .fetch_optional(&mut **transaction)
    .await?
    .is_some())
}

fn equivalent_reuse_scope_allows_source(target_frame: &Frame, source_frame: &Frame) -> bool {
    match CapturedFrameEquivalenceScope::from_frame(target_frame) {
        CapturedFrameEquivalenceScope::Session => true,
        CapturedFrameEquivalenceScope::HiddenSegmentWorkspace { frames_dir_prefix } => {
            source_frame.file_path.starts_with(&frames_dir_prefix)
        }
    }
}

async fn project_equivalent_reuse_document_for_frame(
    transaction: &mut Transaction<'_, Sqlite>,
    frame: &Frame,
    processing_result_id: Option<i64>,
    text: &str,
) -> Result<()> {
    let (app_bundle_id, app_name, window_title) = frame
        .metadata_snapshot
        .as_ref()
        .map(|metadata| {
            (
                metadata.app_bundle_id.clone(),
                metadata.app_name.clone(),
                metadata.window_title.clone(),
            )
        })
        .unwrap_or((None, None, None));
    let group_key = frame_search_group_key(frame);
    let context_text = search_context_text(app_name.as_deref(), window_title.as_deref(), None);
    let app_name_search_key = app_name.as_deref().and_then(normalize_app_name_for_search);

    delete_equivalent_reuse_projection_for_frame(transaction, frame.id).await?;

    insert_search_document(
        transaction,
        NewSearchDocument {
            anchor_type: "frame",
            frame_id: Some(frame.id),
            audio_segment_id: None,
            processing_result_id,
            span_start_ms: None,
            span_end_ms: None,
            absolute_start_at: &frame.captured_at,
            absolute_end_at: &frame.captured_at,
            source_kind: None,
            session_id: &frame.session_id,
            app_bundle_id: app_bundle_id.as_deref(),
            app_name: app_name.as_deref(),
            app_name_search_key: app_name_search_key.as_deref(),
            window_title: window_title.as_deref(),
            group_key: &group_key,
            text_source_kind: "equivalent_reuse",
            body_text: text,
            context_text: &context_text,
        },
    )
    .await
}

async fn delete_equivalent_reuse_projection_for_frame(
    transaction: &mut Transaction<'_, Sqlite>,
    frame_id: i64,
) -> Result<()> {
    sqlx::query(
        "DELETE FROM search_documents \
         WHERE text_source_kind = 'equivalent_reuse' \
           AND anchor_type = 'frame' \
           AND frame_id = ?1",
    )
    .bind(frame_id)
    .execute(&mut **transaction)
    .await?;

    Ok(())
}

async fn project_audio_transcription_result(
    transaction: &mut Transaction<'_, Sqlite>,
    result: &ProcessingResult,
) -> Result<()> {
    let Some(text) = result
        .result_text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    else {
        return Ok(());
    };

    let segment = get_audio_segment_for_search(&mut **transaction, result.subject_id).await?;
    let Some(segment) = segment else {
        return Ok(());
    };
    let fallback_span_end_ms = audio_segment_duration_ms(&segment)?;
    let spans = transcription_spans(result, text, fallback_span_end_ms);
    for (index, span) in spans.into_iter().enumerate() {
        let span_text = span.text.trim();
        if span_text.is_empty() {
            continue;
        }
        let absolute_start_at = timestamp_plus_ms(&segment.started_at, span.start_ms)?;
        let absolute_end_at = timestamp_plus_ms(&segment.started_at, span.end_ms)?;
        let group_key = format!("audio:{}:{index}", segment.id);
        let context_text = search_context_text(None, None, Some(segment.source_kind.as_str()));
        insert_search_document(
            transaction,
            NewSearchDocument {
                anchor_type: "audio",
                frame_id: None,
                audio_segment_id: Some(segment.id),
                processing_result_id: Some(result.id),
                span_start_ms: Some(span.start_ms as i64),
                span_end_ms: Some(span.end_ms as i64),
                absolute_start_at: &absolute_start_at,
                absolute_end_at: &absolute_end_at,
                source_kind: Some(segment.source_kind.as_str()),
                session_id: &segment.source_session_id,
                app_bundle_id: None,
                app_name: None,
                app_name_search_key: None,
                window_title: None,
                group_key: &group_key,
                text_source_kind: "direct",
                body_text: span_text,
                context_text: &context_text,
            },
        )
        .await?;
    }

    Ok(())
}

struct NewSearchDocument<'a> {
    anchor_type: &'a str,
    frame_id: Option<i64>,
    audio_segment_id: Option<i64>,
    processing_result_id: Option<i64>,
    span_start_ms: Option<i64>,
    span_end_ms: Option<i64>,
    absolute_start_at: &'a str,
    absolute_end_at: &'a str,
    source_kind: Option<&'a str>,
    session_id: &'a str,
    app_bundle_id: Option<&'a str>,
    app_name: Option<&'a str>,
    app_name_search_key: Option<&'a str>,
    window_title: Option<&'a str>,
    group_key: &'a str,
    text_source_kind: &'a str,
    body_text: &'a str,
    context_text: &'a str,
}

async fn insert_search_document(
    transaction: &mut Transaction<'_, Sqlite>,
    doc: NewSearchDocument<'_>,
) -> Result<()> {
    let insert = sqlx::query(
        "INSERT INTO search_documents (\
            anchor_type, frame_id, audio_segment_id, processing_result_id, span_start_ms, span_end_ms, \
            absolute_start_at, absolute_end_at, source_kind, session_id, app_bundle_id, app_name, app_name_search_key, window_title, \
            group_key, text_source_kind, body_text, context_text\
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
    )
    .bind(doc.anchor_type)
    .bind(doc.frame_id)
    .bind(doc.audio_segment_id)
    .bind(doc.processing_result_id)
    .bind(doc.span_start_ms)
    .bind(doc.span_end_ms)
    .bind(doc.absolute_start_at)
    .bind(doc.absolute_end_at)
    .bind(doc.source_kind)
    .bind(doc.session_id)
    .bind(
        doc.app_bundle_id
            .and_then(normalize_app_bundle_id_for_search)
            .unwrap_or_default(),
    )
    .bind(doc.app_name)
    .bind(doc.app_name_search_key.unwrap_or_default())
    .bind(doc.window_title)
    .bind(doc.group_key)
    .bind(doc.text_source_kind)
    .bind(doc.body_text)
    .bind(doc.context_text)
    .execute(&mut **transaction)
    .await?;
    let rowid = insert.last_insert_rowid();

    sqlx::query(
        "INSERT INTO search_documents_fts(rowid, body_text, context_text) VALUES (?1, ?2, ?3)",
    )
    .bind(rowid)
    .bind(doc.body_text)
    .bind(doc.context_text)
    .execute(&mut **transaction)
    .await?;

    Ok(())
}

#[derive(Debug, Clone)]
struct TranscriptSpan {
    start_ms: u64,
    end_ms: u64,
    text: String,
}

fn transcription_spans(
    result: &ProcessingResult,
    fallback_text: &str,
    fallback_end_ms: u64,
) -> Vec<TranscriptSpan> {
    if let Some(payload) = result.structured_payload_json.as_deref() {
        if let Ok(metadata) = serde_json::from_str::<TranscriptionMetadata>(payload) {
            let segments = metadata
                .segments
                .into_iter()
                .filter(|segment| !segment.text.trim().is_empty())
                .map(|segment| TranscriptSpan {
                    start_ms: segment.start_ms,
                    end_ms: segment.end_ms.max(segment.start_ms),
                    text: segment.text,
                })
                .collect::<Vec<_>>();
            if !segments.is_empty() {
                return segments;
            }

            if !metadata.words.is_empty() {
                return metadata
                    .words
                    .chunks(24)
                    .filter_map(|words| {
                        let first = words.first()?;
                        let last = words.last()?;
                        Some(TranscriptSpan {
                            start_ms: first.start_ms,
                            end_ms: last.end_ms.max(first.start_ms),
                            text: words
                                .iter()
                                .map(|word| word.text.as_str())
                                .collect::<Vec<_>>()
                                .join(" "),
                        })
                    })
                    .collect();
            }
        }
    }

    vec![TranscriptSpan {
        start_ms: 0,
        end_ms: fallback_end_ms,
        text: fallback_text.to_string(),
    }]
}

fn audio_segment_duration_ms(segment: &AudioSegment) -> Result<u64> {
    let started_at = OffsetDateTime::parse(&segment.started_at, &Rfc3339).map_err(|error| {
        AppInfraError::FrameBatchFinalize(format!(
            "invalid audio segment start timestamp '{}': {error}",
            segment.started_at
        ))
    })?;
    let ended_at = OffsetDateTime::parse(&segment.ended_at, &Rfc3339).map_err(|error| {
        AppInfraError::FrameBatchFinalize(format!(
            "invalid audio segment end timestamp '{}': {error}",
            segment.ended_at
        ))
    })?;
    let duration_ms = (ended_at - started_at).whole_milliseconds().max(0);
    Ok(duration_ms.try_into().unwrap_or(u64::MAX))
}

fn timestamp_plus_ms(started_at: &str, offset_ms: u64) -> Result<String> {
    let start = OffsetDateTime::parse(started_at, &Rfc3339).map_err(|error| {
        AppInfraError::FrameBatchFinalize(format!(
            "invalid search timestamp '{started_at}': {error}"
        ))
    })?;
    let timestamp = start
        .checked_add(Duration::milliseconds(
            offset_ms.try_into().unwrap_or(i64::MAX),
        ))
        .ok_or_else(|| {
            AppInfraError::FrameBatchFinalize("search timestamp overflow".to_string())
        })?;
    Ok(timestamp.format(&Rfc3339).map_err(|error| {
        AppInfraError::FrameBatchFinalize(format!("failed to format search timestamp: {error}"))
    })?)
}

fn normalize_query(query: &str) -> String {
    query.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn fts_query_for_plain_text(query: &str) -> String {
    let terms = query
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|term| !term.is_empty())
        .collect::<Vec<_>>();
    let mut searchable_terms = terms
        .iter()
        .copied()
        .filter(|term| term.chars().count() >= 2)
        .collect::<Vec<_>>();
    if searchable_terms.is_empty() && query.chars().count() >= 2 {
        searchable_terms = terms;
    }
    searchable_terms
        .into_iter()
        .map(|term| fts_quote_phrase_term(term))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Wraps a single token as a safe FTS5 quoted phrase term, doubling embedded
/// quotes. This is the canonical escaping used everywhere body text reaches
/// MATCH so raw user input never reaches FTS5 unquoted.
fn fts_quote_phrase_term(term: &str) -> String {
    format!("\"{}\"", term.replace('"', "\"\""))
}

// === Search Query Syntax (ADR 0019) ===
//
// `parse_search_query` is the backend-canonical parser. It is quote-aware,
// recognizes only the known field operators, extracts them into refinements,
// translates the residual body operators into a safe FTS5 expression, and
// returns any strict validation problems as in-band parse errors with
// character (Unicode scalar) spans into the original raw query.

/// The known field operator keys. Any other `key:value` token stays literal
/// body text so URL, code, and `error:404`-style searches keep working.
const FIELD_OPERATOR_KEYS: &[&str] = &["app", "source", "after", "before", "date"];

/// Result of parsing a raw search query into refinements + residual body FTS.
#[derive(Debug, Clone, Default)]
pub(crate) struct ParsedQuery {
    /// The safe FTS5 match expression derived from the residual body.
    pub(crate) fts_body: String,
    /// `app:` operators, extracted as `Any`-kind app refinements.
    pub(crate) apps: Vec<SearchAppRefinement>,
    /// `source:` operators, extracted as audio source kinds.
    pub(crate) audio_sources: Vec<AudioSegmentSourceKind>,
    /// `source:screen` operator, restricting results to captured frames.
    pub(crate) screen_source: bool,
    /// `after:`/`before:`/`date:` operators resolved to a single date range.
    pub(crate) date_range: Option<SearchDateRangeRefinement>,
    /// The plain residual body text (operators stripped) for display and FTS.
    pub(crate) residual_query: String,
    /// Strict validation problems found during parsing.
    pub(crate) errors: Vec<SearchParseError>,
}

/// One tokenizer token, carrying the original character span for error
/// reporting and whether the token (or its value) was quoted.
#[derive(Debug, Clone)]
struct QueryToken {
    /// The token text with surrounding quotes removed.
    text: String,
    /// True when the token text was wrapped in double quotes.
    quoted: bool,
    /// Character (Unicode scalar) start offset into the original raw query.
    start: u32,
    /// Character (Unicode scalar) end offset (exclusive) into the raw query.
    end: u32,
    /// The raw token slice exactly as typed (including quotes), for echoing.
    raw: String,
}

/// Outcome of quote-aware tokenization.
struct Tokenized {
    tokens: Vec<QueryToken>,
    /// Present when a quote was opened but never closed.
    unbalanced_quote: Option<SearchParseError>,
}

/// Quote-aware tokenizer. Splits on unquoted whitespace, keeps quoted runs
/// (including embedded whitespace) as a single token, and tracks character
/// spans into the original query.
fn tokenize_query(raw: &str) -> Tokenized {
    let chars: Vec<char> = raw.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0_usize;
    let len = chars.len();

    while index < len {
        // Skip unquoted whitespace between tokens.
        while index < len && chars[index].is_whitespace() {
            index += 1;
        }
        if index >= len {
            break;
        }

        let token_start = index;
        let mut text = String::new();
        let mut had_quote = false;
        let mut had_unquoted = false;

        while index < len {
            let ch = chars[index];
            if ch == '"' {
                had_quote = true;
                // Toggle quote mode. Any whitespace inside quotes is literal.
                let mut in_quote = true;
                index += 1;
                while index < len {
                    let inner = chars[index];
                    if inner == '"' {
                        // A doubled `""` inside the run is an escaped literal
                        // quote: consume both and keep one `"` in the phrase
                        // rather than closing and reopening the quoted run.
                        if index + 1 < len && chars[index + 1] == '"' {
                            text.push('"');
                            index += 2;
                            continue;
                        }
                        in_quote = false;
                        index += 1;
                        break;
                    }
                    text.push(inner);
                    index += 1;
                }
                if in_quote {
                    // Unterminated quote: report against the rest of the query.
                    let token_end = len as u32;
                    return Tokenized {
                        tokens,
                        unbalanced_quote: Some(SearchParseError {
                            kind: "unbalanced_quote".to_string(),
                            message: "a quoted phrase is missing its closing quote".to_string(),
                            start: token_start as u32,
                            end: token_end,
                            token: chars[token_start..len].iter().collect(),
                        }),
                    };
                }
            } else if ch.is_whitespace() {
                break;
            } else {
                text.push(ch);
                had_unquoted = true;
                index += 1;
            }
        }

        let token_end = index as u32;
        // A token is "quoted" (a literal body phrase) only when it was a pure
        // quoted run with no characters outside the quotes. A mixed token such
        // as `app:"Google Chrome"` keeps its key visible so it can still be
        // recognized as a field operator rather than a literal body phrase.
        let quoted = had_quote && !had_unquoted;
        let raw: String = chars[token_start..index as usize].iter().collect();
        tokens.push(QueryToken {
            text,
            quoted,
            start: token_start as u32,
            end: token_end,
            raw,
        });
    }

    Tokenized {
        tokens,
        unbalanced_quote: None,
    }
}

/// Splits a token into a `(key, value)` field-operator pair when it looks like
/// `key:value` with a non-empty alphanumeric key. The split happens on the
/// first unquoted colon in the raw token, so `app:"Google Chrome"` and
/// `error:404` are both detected as `key:value` shapes (recognition of known
/// keys happens separately).
fn split_field_operator(token: &QueryToken) -> Option<(String, String, bool)> {
    // Fully-quoted tokens are always literal body phrases, never operators.
    if token.quoted {
        return None;
    }
    let raw = &token.raw;
    let colon = raw.find(':')?;
    let key = &raw[..colon];
    if key.is_empty() || !key.chars().all(|ch| ch.is_alphanumeric()) {
        return None;
    }
    let value_raw = &raw[colon + 1..];
    // Strip surrounding quotes on the value (e.g. app:"Google Chrome").
    let (value, value_quoted) =
        if value_raw.starts_with('"') && value_raw.ends_with('"') && value_raw.chars().count() >= 2
        {
            (
                value_raw[1..value_raw.len() - 1].replace("\"\"", "\""),
                true,
            )
        } else {
            (value_raw.to_string(), false)
        };
    Some((key.to_string(), value, value_quoted))
}

/// The operator-stripped residual body of a raw search query — the text the
/// meaning-vector embed should use (so `app:`/`before:`/quoted operators don't
/// pollute the **Semantic Search Vector**). Mirrors what FTS ranks on.
///
/// app-infra takes no embedding-runtime dependency, so it cannot embed the query
/// itself; the desktop layer embeds, and this exposes the residual it should feed
/// the embedder. An all-operators query yields an empty residual, which the caller
/// treats as "no meaning vector" (keyword-only).
pub fn semantic_search_residual_query(raw: &str) -> String {
    parse_search_query(raw).residual_query
}

/// Parses a raw query into refinements and a safe FTS body. See module comment.
pub(crate) fn parse_search_query(raw: &str) -> ParsedQuery {
    let tokenized = tokenize_query(raw);
    let mut parsed = ParsedQuery::default();
    if let Some(error) = tokenized.unbalanced_quote {
        parsed.errors.push(error);
    }

    let local_today = local_today_date();
    let mut residual_tokens: Vec<QueryToken> = Vec::new();

    for token in tokenized.tokens {
        if let Some((key, value, value_quoted)) = split_field_operator(&token) {
            let lower_key = key.to_lowercase();
            if FIELD_OPERATOR_KEYS.contains(&lower_key.as_str()) {
                apply_field_operator(
                    &mut parsed,
                    &lower_key,
                    &value,
                    value_quoted,
                    &token,
                    local_today,
                );
                continue;
            }
        }
        // Not a known field operator: stays literal body text.
        residual_tokens.push(token);
    }

    parsed.residual_query = residual_tokens
        .iter()
        .map(|token| token.raw.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    parsed.fts_body = fts_body_for_tokens(&residual_tokens, &mut parsed.errors);

    parsed
}

/// Applies one recognized field operator to the parsed refinements.
fn apply_field_operator(
    parsed: &mut ParsedQuery,
    key: &str,
    value: &str,
    _value_quoted: bool,
    token: &QueryToken,
    local_today: time::Date,
) {
    let trimmed = value.trim();
    match key {
        "app" => {
            if trimmed.is_empty() {
                parsed.errors.push(token_parse_error(
                    token,
                    "empty_value",
                    "app: needs an application name or bundle id",
                ));
                return;
            }
            let app = SearchAppRefinement {
                kind: SearchAppRefinementKind::Any,
                value: trimmed.to_string(),
                display_name: trimmed.to_string(),
            };
            if !parsed.apps.contains(&app) {
                parsed.apps.push(app);
            }
        }
        "source" => match trimmed.to_lowercase().as_str() {
            "mic" | "microphone" => {
                if !parsed
                    .audio_sources
                    .contains(&AudioSegmentSourceKind::Microphone)
                {
                    parsed.audio_sources.push(AudioSegmentSourceKind::Microphone);
                }
            }
            "system" | "system_audio" => {
                if !parsed
                    .audio_sources
                    .contains(&AudioSegmentSourceKind::SystemAudio)
                {
                    parsed
                        .audio_sources
                        .push(AudioSegmentSourceKind::SystemAudio);
                }
            }
            "screen" => parsed.screen_source = true,
            _ => parsed.errors.push(token_parse_error(
                token,
                "unknown_source",
                "source: must be mic, system, or screen",
            )),
        },
        "after" => match resolve_point_date(trimmed, local_today) {
            Some(date) => set_date_bound(parsed, Some(start_of_day_rfc3339(date)), None),
            None => parsed.errors.push(token_parse_error(
                token,
                "bad_date",
                "after: needs a date (YYYY-MM-DD) or relative point (today, yesterday, Nd, Nh)",
            )),
        },
        "before" => match resolve_point_date(trimmed, local_today) {
            Some(date) => set_date_bound(parsed, None, Some(end_of_day_rfc3339(date))),
            None => parsed.errors.push(token_parse_error(
                token,
                "bad_date",
                "before: needs a date (YYYY-MM-DD) or relative point (today, yesterday, Nd, Nh)",
            )),
        },
        "date" => match resolve_day_or_period(trimmed, local_today) {
            Some((start_date, end_date)) => set_date_bound(
                parsed,
                Some(start_of_day_rfc3339(start_date)),
                Some(end_of_day_rfc3339(end_date)),
            ),
            None => parsed.errors.push(token_parse_error(
                token,
                "bad_date",
                "date: needs a day or period (today, yesterday, last-week, this-week, last-month, this-month, or YYYY-MM-DD)",
            )),
        },
        _ => {}
    }
}

/// Writes one or both bounds into the single date range slot, last-write-wins
/// per bound. A one-sided write leaves the other bound at the wide-open
/// sentinel so the range stays half-open at day granularity.
fn set_date_bound(parsed: &mut ParsedQuery, start: Option<String>, end: Option<String>) {
    let existing = parsed.date_range.take();
    let mut start_at = existing
        .as_ref()
        .map(|range| range.start_at.clone())
        .unwrap_or_else(open_lower_bound_rfc3339);
    let mut end_at = existing
        .as_ref()
        .map(|range| range.end_at.clone())
        .unwrap_or_else(open_upper_bound_rfc3339);
    if let Some(start) = start {
        start_at = start;
    }
    if let Some(end) = end {
        end_at = end;
    }
    parsed.date_range = Some(SearchDateRangeRefinement {
        start_at,
        end_at,
        origin: None,
    });
}

fn token_parse_error(token: &QueryToken, kind: &str, message: &str) -> SearchParseError {
    SearchParseError {
        kind: kind.to_string(),
        message: message.to_string(),
        start: token.start,
        end: token.end,
        token: token.raw.clone(),
    }
}

// --- Body Match Operator → FTS5 translation ---

/// One residual body element after operator interpretation.
#[derive(Debug, Clone)]
enum BodyTerm {
    /// A positive matchable element (already a safe FTS5 fragment).
    Positive(String),
    /// A negated element (`-term`); requires a positive sibling in its AND group.
    Negative(String),
}

/// Translates residual tokens into a safe FTS5 expression. Body operators:
/// quoted phrase, `-term` exclusion (FTS5 NOT), `OR` (uppercase only),
/// `term*` prefix (>=2 leading chars), and implicit AND between terms.
///
/// When the residual contains no body operators at all, this delegates to the
/// exact plain-text path so operator-free queries behave identically to before.
fn fts_body_for_tokens(tokens: &[QueryToken], errors: &mut Vec<SearchParseError>) -> String {
    if tokens.is_empty() {
        return String::new();
    }

    let has_body_operator = tokens.iter().any(|token| {
        token.quoted
            || token.text == "OR"
            || token.text.starts_with('-')
            || token.text.ends_with('*')
    });

    if !has_body_operator {
        let joined = tokens
            .iter()
            .map(|token| token.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        return fts_query_for_plain_text(&normalize_query(&joined));
    }

    // Split into OR-groups on bare uppercase `OR` tokens; within each group,
    // terms are implicitly ANDed (AND binds tighter than OR in FTS5).
    let mut groups: Vec<Vec<&QueryToken>> = vec![Vec::new()];
    let mut or_tokens: Vec<&QueryToken> = Vec::new();
    for token in tokens {
        if !token.quoted && token.text == "OR" {
            or_tokens.push(token);
            groups.push(Vec::new());
            continue;
        }
        groups
            .last_mut()
            .expect("there is always at least one group")
            .push(token);
    }

    // A dangling `OR` (leading, trailing, or doubled) leaves an empty AND-group.
    // ADR 0019 mandates strict validation of malformed Body Match Operators, so
    // reject it as an in-band parse error instead of silently rewriting
    // `foo OR` into `foo`. Attribute the error to the OR adjacent to the gap.
    if let Some(empty_index) = groups.iter().position(|group| group.is_empty()) {
        let or_index = empty_index
            .saturating_sub(1)
            .min(or_tokens.len().saturating_sub(1));
        if let Some(token) = or_tokens.get(or_index) {
            errors.push(token_parse_error(
                token,
                "dangling_or",
                "OR needs a search term on both sides",
            ));
        }
        return String::new();
    }

    let mut rendered_groups: Vec<String> = Vec::new();
    for group in groups {
        if let Some(rendered) = fts_and_group(&group, errors) {
            if !rendered.is_empty() {
                rendered_groups.push(rendered);
            }
        }
    }

    rendered_groups
        .into_iter()
        .map(|group| format!("({group})"))
        .collect::<Vec<_>>()
        .join(" OR ")
}

/// Renders one implicit-AND group of tokens into an FTS5 fragment. Returns
/// `None` (and records an error) for pure-negation groups.
fn fts_and_group(tokens: &[&QueryToken], errors: &mut Vec<SearchParseError>) -> Option<String> {
    let mut body_terms: Vec<BodyTerm> = Vec::new();
    let mut positive_count = 0_usize;
    let mut negative_origin: Option<&QueryToken> = None;

    for token in tokens {
        if token.quoted {
            // Quoted phrase forces literal matching of the whole phrase.
            if token.text.trim().is_empty() {
                continue;
            }
            body_terms.push(BodyTerm::Positive(fts_quote_phrase_term(&token.text)));
            positive_count += 1;
            continue;
        }

        let text = token.text.as_str();
        if let Some(stripped) = text.strip_prefix('-') {
            if let Some(fragment) = fts_fragment_for_word(stripped) {
                body_terms.push(BodyTerm::Negative(fragment));
                if negative_origin.is_none() {
                    negative_origin = Some(token);
                }
            }
            continue;
        }

        if let Some(fragment) = fts_fragment_for_word(text) {
            body_terms.push(BodyTerm::Positive(fragment));
            positive_count += 1;
        }
    }

    if positive_count == 0 {
        if let Some(token) = negative_origin {
            errors.push(token_parse_error(
                token,
                "pure_negation",
                "an exclusion (-term) needs at least one positive term to match",
            ));
        }
        return None;
    }

    let positives = body_terms
        .iter()
        .filter_map(|term| match term {
            BodyTerm::Positive(fragment) => Some(fragment.clone()),
            BodyTerm::Negative(_) => None,
        })
        .collect::<Vec<_>>()
        .join(" ");
    let negatives = body_terms
        .iter()
        .filter_map(|term| match term {
            BodyTerm::Negative(fragment) => Some(fragment.clone()),
            BodyTerm::Positive(_) => None,
        })
        .collect::<Vec<_>>();

    if negatives.is_empty() {
        Some(positives)
    } else {
        Some(format!("{positives} NOT {}", negatives.join(" NOT ")))
    }
}

/// Converts a single bare word into a safe FTS5 fragment, honoring the `term*`
/// prefix operator (needs >=2 leading alphanumeric chars, else literal). The
/// word is split on non-alphanumerics like the plain-text path so symbols stay
/// safe; an all-symbol word yields no fragment.
fn fts_fragment_for_word(word: &str) -> Option<String> {
    let wants_prefix = word.ends_with('*');
    let core = if wants_prefix {
        word.trim_end_matches('*')
    } else {
        word
    };

    // Keep only alphanumeric runs, mirroring the plain-text tokenizer so the
    // quoted FTS term never contains FTS5-significant punctuation.
    let cleaned = core
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if cleaned.is_empty() {
        return None;
    }

    if wants_prefix && cleaned.len() == 1 && cleaned[0].chars().count() >= 2 {
        // term* → prefix query: "term"*
        return Some(format!("{}*", fts_quote_phrase_term(cleaned[0])));
    }

    // Otherwise treat the cleaned parts as a phrase (handles symbol-joined
    // words and prefix tokens that did not qualify, which become literal).
    Some(
        cleaned
            .into_iter()
            .map(fts_quote_phrase_term)
            .collect::<Vec<_>>()
            .join(" "),
    )
}

// --- Merge parsed field operators into caller refinements ---

/// Merges parsed field operators into any caller-supplied refinements per each
/// field's multiplicity rule: apps/sources accumulate (set, dedup) and the date
/// slot is overwritten by parsed date operators (last-write-wins).
fn merge_parsed_field_operators(
    base: Option<SearchCaptureRefinements>,
    parsed: &ParsedQuery,
) -> SearchCaptureRefinements {
    let mut refinements = base.unwrap_or_default();

    for app in &parsed.apps {
        if !refinements.apps.contains(app) {
            refinements.apps.push(app.clone());
        }
    }
    for source in &parsed.audio_sources {
        if !refinements.audio_sources.contains(source) {
            refinements.audio_sources.push(source.clone());
        }
    }
    if parsed.screen_source {
        refinements.screen_source = true;
    }
    if let Some(date_range) = &parsed.date_range {
        refinements.date_range = Some(date_range.clone());
    }

    refinements
}

// --- Date resolution (ADR 0019, A3) ---
//
// All date operators resolve to frozen concrete instants at parse time in the
// LOCAL timezone, both bounds inclusive at day granularity. The sound local
// offset is obtained per-calendar-date via chrono::Local (mirroring
// capture_retention), avoiding the `time` crate's unsound `local-offset`
// feature. Week start defaults to Monday (locale first weekday).

/// The current local calendar date, used as the anchor for relative tokens.
fn local_today_date() -> time::Date {
    use chrono::Datelike;
    let now = chrono::Local::now().date_naive();
    time::Date::from_calendar_date(
        now.year(),
        time::Month::try_from(now.month() as u8).unwrap_or(time::Month::January),
        now.day() as u8,
    )
    .unwrap_or_else(|_| OffsetDateTime::now_utc().date())
}

/// Resolves an `after:`/`before:` point to a single calendar date. Accepts an
/// absolute `YYYY-MM-DD`, or a relative point: `today`, `yesterday`, `Nd`
/// (N days ago), `Nh` (N hours ago, resolved to that day).
fn resolve_point_date(value: &str, today: time::Date) -> Option<time::Date> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    match value.to_lowercase().as_str() {
        "today" => return Some(today),
        "yesterday" => return today.previous_day(),
        _ => {}
    }

    if let Ok(date) = time::Date::parse(value, &time::format_description::well_known::Iso8601::DATE)
    {
        return Some(date);
    }

    if let Some(days) = parse_relative_count(value, 'd') {
        return today.checked_sub(time::Duration::days(days));
    }
    if let Some(hours) = parse_relative_count(value, 'h') {
        // `Nh` resolves to the day N hours before local "now".
        let now = local_now_offset_datetime();
        let resolved = now.checked_sub(time::Duration::hours(hours))?;
        return Some(resolved.date());
    }

    None
}

/// Resolves a `date:` value to an inclusive `(start_date, end_date)` span:
/// a single day, or a named period (today, yesterday, last/this week/month).
fn resolve_day_or_period(value: &str, today: time::Date) -> Option<(time::Date, time::Date)> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    match value.to_lowercase().as_str() {
        "today" => return Some((today, today)),
        "yesterday" => {
            let yesterday = today.previous_day()?;
            return Some((yesterday, yesterday));
        }
        "this-week" => return Some(week_span(today, 0)),
        "last-week" => return Some(week_span(today, -1)),
        "this-month" => return Some(month_span(today, 0)),
        "last-month" => return Some(month_span(today, -1)),
        _ => {}
    }

    if let Ok(date) = time::Date::parse(value, &time::format_description::well_known::Iso8601::DATE)
    {
        return Some((date, date));
    }

    None
}

/// Returns the inclusive Monday..Sunday span for the week containing `today`,
/// shifted by `week_offset` weeks. Week start = Monday (locale default).
fn week_span(today: time::Date, week_offset: i64) -> (time::Date, time::Date) {
    let weekday_from_monday = today.weekday().number_days_from_monday() as i64;
    let monday = today
        .checked_sub(time::Duration::days(weekday_from_monday))
        .unwrap_or(today);
    let start = monday
        .checked_add(time::Duration::weeks(week_offset))
        .unwrap_or(monday);
    let end = start.checked_add(time::Duration::days(6)).unwrap_or(start);
    (start, end)
}

/// Returns the inclusive first..last day span for the month containing `today`,
/// shifted by `month_offset` months.
fn month_span(today: time::Date, month_offset: i64) -> (time::Date, time::Date) {
    let (mut year, mut month_index) =
        (today.year() as i64, today.month() as i64 - 1 + month_offset);
    year += month_index.div_euclid(12);
    month_index = month_index.rem_euclid(12);
    let month = time::Month::try_from((month_index + 1) as u8).unwrap_or(time::Month::January);
    let start = time::Date::from_calendar_date(year as i32, month, 1).unwrap_or(today);
    let last_day = days_in_month(year as i32, month);
    let end = time::Date::from_calendar_date(year as i32, month, last_day).unwrap_or(start);
    (start, end)
}

fn days_in_month(year: i32, month: time::Month) -> u8 {
    match month {
        time::Month::January
        | time::Month::March
        | time::Month::May
        | time::Month::July
        | time::Month::August
        | time::Month::October
        | time::Month::December => 31,
        time::Month::April | time::Month::June | time::Month::September | time::Month::November => {
            30
        }
        time::Month::February => {
            if time::util::is_leap_year(year) {
                29
            } else {
                28
            }
        }
    }
}

/// Parses a relative count like `7d`/`1h`. Returns the numeric magnitude when
/// the value is digits followed by exactly the expected unit char.
fn parse_relative_count(value: &str, unit: char) -> Option<i64> {
    let lower = value.to_lowercase();
    let stripped = lower.strip_suffix(unit)?;
    if stripped.is_empty() || !stripped.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    stripped.parse::<i64>().ok()
}

/// The local offset for a given calendar date, resolved soundly through
/// chrono::Local (see capture_retention::local_midnight_offset). Falls back to
/// the current local offset, then UTC.
fn local_offset_for_date(date: time::Date) -> UtcOffset {
    use chrono::{Local, LocalResult, NaiveDate, Offset, TimeZone};
    let resolved = NaiveDate::from_ymd_opt(
        date.year(),
        u32::from(u8::from(date.month())),
        u32::from(date.day()),
    )
    .and_then(|local_date| local_date.and_hms_opt(0, 0, 0))
    .and_then(
        |local_midnight| match Local.from_local_datetime(&local_midnight) {
            LocalResult::Single(datetime) => Some(datetime.offset().fix().local_minus_utc()),
            LocalResult::Ambiguous(earliest, _) => Some(earliest.offset().fix().local_minus_utc()),
            LocalResult::None => None,
        },
    )
    .and_then(|offset_seconds| UtcOffset::from_whole_seconds(offset_seconds).ok());
    resolved.unwrap_or_else(|| local_now_offset_datetime().offset())
}

/// Current instant as an `OffsetDateTime` carrying the local offset, resolved
/// through chrono (the `time` crate's `now_local` is feature-gated/unsound).
fn local_now_offset_datetime() -> OffsetDateTime {
    use chrono::Offset;
    let now = chrono::Local::now();
    let offset_seconds = now.offset().fix().local_minus_utc();
    let offset = UtcOffset::from_whole_seconds(offset_seconds).unwrap_or(UtcOffset::UTC);
    OffsetDateTime::now_utc().to_offset(offset)
}

/// `after:D` → D 00:00:00 local, formatted as RFC3339 for the existing
/// `normalize_search_refinements` date path (which converts to UTC).
fn start_of_day_rfc3339(date: time::Date) -> String {
    let offset = local_offset_for_date(date);
    date.with_hms_milli(0, 0, 0, 0)
        .expect("midnight is always valid")
        .assume_offset(offset)
        .format(&Rfc3339)
        .expect("RFC3339 formatting of a valid datetime should succeed")
}

/// `before:D` → D 23:59:59.999 local, formatted as RFC3339.
fn end_of_day_rfc3339(date: time::Date) -> String {
    let offset = local_offset_for_date(date);
    date.with_hms_milli(23, 59, 59, 999)
        .expect("end-of-day is always valid")
        .assume_offset(offset)
        .format(&Rfc3339)
        .expect("RFC3339 formatting of a valid datetime should succeed")
}

/// A wide-open lower bound used when only an upper date bound is supplied.
fn open_lower_bound_rfc3339() -> String {
    "0001-01-01T00:00:00Z".to_string()
}

/// A wide-open upper bound used when only a lower date bound is supplied.
fn open_upper_bound_rfc3339() -> String {
    "9999-12-31T23:59:59.999Z".to_string()
}

fn clamp_limit(limit: Option<u32>) -> u32 {
    if limit == Some(0) {
        return 0;
    }
    limit
        .unwrap_or(DEFAULT_GROUP_LIMIT)
        .clamp(1, MAX_GROUP_LIMIT)
}

fn hit_fetch_limit(offset: usize, limit: u32) -> i64 {
    let requested_groups = offset
        .saturating_add(limit as usize)
        .saturating_add(1)
        .min((MAX_HIT_FETCH_LIMIT / HIT_FETCH_OVERFETCH_PER_GROUP) as usize);
    ((requested_groups as i64) * HIT_FETCH_OVERFETCH_PER_GROUP)
        .max(MIN_HIT_FETCH_LIMIT)
        .min(MAX_HIT_FETCH_LIMIT)
}

fn frame_search_group_key(frame: &Frame) -> String {
    frame
        .equivalence
        .ready_parts()
        .map(|(hint, proof, version)| {
            let scope = frame_search_group_scope_identity(frame);
            format!(
                "frame:eq:{}:{version}:{hint}:{}:{scope}",
                frame.session_id,
                proof_identity(proof)
            )
        })
        .unwrap_or_else(|| format!("frame:{}", frame.id))
}

fn frame_search_group_scope_identity(frame: &Frame) -> String {
    match CapturedFrameEquivalenceScope::from_frame(frame) {
        CapturedFrameEquivalenceScope::Session => "scope:session".to_string(),
        CapturedFrameEquivalenceScope::HiddenSegmentWorkspace { frames_dir_prefix } => {
            format!(
                "scope:hidden:{}",
                proof_identity(frames_dir_prefix.as_bytes())
            )
        }
    }
}

fn proof_identity(proof: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in proof {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn search_context_text(
    app_name: Option<&str>,
    window_title: Option<&str>,
    source_kind: Option<&str>,
) -> String {
    [app_name, window_title, source_kind]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug, Clone)]
struct FrameHit {
    /// `search_documents.id` of this **Search Result Anchor** — the fusion key
    /// for RRF and the `vec0` rowid. One anchor can surface from both the
    /// **Text Search** and the **Semantic Search** candidate lists; fusion
    /// dedups on this id before grouping.
    anchor_id: i64,
    group_key: String,
    frame: Frame,
    snippet: String,
    /// The hit's ranking score, **lower is better**. For an FTS-only path this
    /// is BM25; once **Hybrid Search** fuses, it is the negated RRF score so the
    /// existing ASC sort keeps the best hit first.
    rank: f64,
    app_bundle_id: Option<String>,
    app_name: Option<String>,
    window_title: Option<String>,
    text_source_kind: String,
    secret_redaction_count: u32,
    /// True when this anchor entered via the `vec0` KNN with no **Text Search**
    /// term to highlight, so `snippet` is a leading `body_text` excerpt.
    found_by_meaning: bool,
}

#[derive(Debug, Clone)]
struct AudioHit {
    /// `search_documents.id` — the RRF fusion key and `vec0` rowid (see [`FrameHit::anchor_id`]).
    anchor_id: i64,
    audio_segment: AudioSegment,
    source_kind: AudioSegmentSourceKind,
    span_start_ms: u64,
    span_end_ms: u64,
    snippet: String,
    rank: f64,
    secret_redaction_count: u32,
    /// True for a meaning-only hit (see [`FrameHit::found_by_meaning`]).
    found_by_meaning: bool,
}

async fn fetch_search_document_high_water_mark(pool: &SqlitePool) -> Result<i64> {
    let row =
        sqlx::query("SELECT COALESCE(MAX(id), 0) AS snapshot_document_id FROM search_documents")
            .fetch_one(pool)
            .await?;
    Ok(row.get("snapshot_document_id"))
}

fn push_search_refinement_predicates(
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
fn degrade_to_keyword_only<T>(kind: &str, result: Result<Vec<T>>) -> Vec<T> {
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

async fn fetch_grouped_frame_hits(
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
        Some(embedding) => {
            degrade_to_keyword_only(
                "frame",
                fetch_semantic_frame_hits(pool, embedding, snapshot_document_id, refinements).await,
            )
        }
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
        let text_group_count = groups.iter().filter(|group| !group.found_by_meaning).count();
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

async fn fetch_grouped_audio_hits(
    pool: &SqlitePool,
    fts_query: &str,
    fts_is_searchable: bool,
    snapshot_document_id: i64,
    refinements: &NormalizedSearchRefinements,
    query_embedding: Option<&[f32]>,
) -> Result<Vec<AudioSearchResult>> {
    let semantic_hits = match query_embedding {
        Some(embedding) => {
            degrade_to_keyword_only(
                "audio",
                fetch_semantic_audio_hits(pool, embedding, snapshot_document_id, refinements).await,
            )
        }
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

/// A leading `body_text` excerpt for a meaning-only **Search Snippet**: the hit
/// matched the query vector but has no **Text Search** term to mark, so we show
/// the start of the captured text rather than an FTS `snippet(...)`. Whitespace
/// is collapsed and the excerpt is char-bounded (never byte-sliced through a
/// multibyte scalar) with an ellipsis when truncated. Redaction is *not* applied
/// here: the `secret_redactions` rollup carried on the result drives the same
/// masking a **Text Search** snippet uses, at the same egress boundary.
fn meaning_snippet(body_text: &str) -> String {
    let collapsed = body_text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= MEANING_SNIPPET_CHAR_BUDGET {
        return collapsed;
    }
    let truncated: String = collapsed.chars().take(MEANING_SNIPPET_CHAR_BUDGET).collect();
    format!("{}…", truncated.trim_end())
}

/// Re-impose the **Semantic Candidate Set** order (nearest-first) on hydrated
/// hits. The seam returns ordered `anchor_id`s, but the hydration projection
/// (`WHERE id IN (…)`) returns rows in an arbitrary order, so the candidate-set
/// position — the entire rank-only payload **Hybrid Search** RRF fuses on — is
/// restored here from the candidate list, keyed by `anchor_id`. Hits whose anchor
/// somehow fell out between the KNN and the hydration (a delete racing the read)
/// sort to the tail and are harmless.
fn order_by_candidate_set<T>(mut hits: Vec<T>, candidate_set: &[i64], key: impl Fn(&T) -> i64) -> Vec<T> {
    let position: std::collections::HashMap<i64, usize> = candidate_set
        .iter()
        .enumerate()
        .map(|(index, id)| (*id, index))
        .collect();
    hits.sort_by_key(|hit| position.get(&key(hit)).copied().unwrap_or(usize::MAX));
    hits
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
    Ok(order_by_candidate_set(hits, &candidate_set, |hit| hit.anchor_id))
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
    Ok(order_by_candidate_set(hits, &candidate_set, |hit| hit.anchor_id))
}

/// Push the in-scope **Search Result Anchor** rowid sub-select that constrains a
/// `vec0` KNN to the **Search Refinement** scope. This reuses the exact same
/// `push_search_refinement_predicates` `WHERE` that the **Text Search** path
/// builds (date range, app, window-title, source) plus the `anchor_type` and
/// snapshot bound, so the meaning tier scopes identically to the keyword tier.
/// An unrefined query yields the all-anchors-of-this-type set, so the KNN
/// becomes a plain top-k.
fn push_in_scope_anchor_rowids(
    query: &mut QueryBuilder<'_, Sqlite>,
    anchor_type: &str,
    snapshot_document_id: i64,
    refinements: &NormalizedSearchRefinements,
) {
    query.push("SELECT search_documents.id FROM search_documents WHERE search_documents.anchor_type = ");
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

fn rrf_fuse_frame_hits(text_hits: &[FrameHit], semantic_hits: &[FrameHit]) -> Vec<FrameHit> {
    rrf_fuse_hits(text_hits, semantic_hits, |hit| hit.anchor_id, |hit, rank| hit.rank = rank)
}

/// Audio counterpart of [`rrf_fuse_frame_hits`].
fn rrf_fuse_audio_hits(text_hits: &[AudioHit], semantic_hits: &[AudioHit]) -> Vec<AudioHit> {
    rrf_fuse_hits(text_hits, semantic_hits, |hit| hit.anchor_id, |hit, rank| hit.rank = rank)
}

fn group_frame_hits(hits: &[FrameHit]) -> Vec<FrameSearchResult> {
    let mut groups: Vec<(String, Vec<FrameHit>)> = Vec::new();
    for hit in hits {
        let group_index = groups.iter().position(|(_group_key, group_hits)| {
            group_hits
                .first()
                .is_some_and(|representative| frame_hits_are_equivalent(representative, &hit))
        });
        if let Some(index) = group_index {
            groups[index].1.push(hit.clone());
        } else {
            groups.push((hit.group_key.clone(), vec![hit.clone()]));
        }
    }

    let mut results = groups
        .into_iter()
        .filter_map(|(group_key, mut hits)| {
            hits.sort_by(|a, b| {
                a.rank
                    .total_cmp(&b.rank)
                    .then_with(|| b.frame.captured_at.cmp(&a.frame.captured_at))
            });
            let representative = hits
                .iter()
                .max_by(|a, b| a.frame.captured_at.cmp(&b.frame.captured_at))?;
            let group_start_at = hits
                .iter()
                .map(|hit| hit.frame.captured_at.as_str())
                .min()
                .unwrap_or(representative.frame.captured_at.as_str())
                .to_string();
            let group_end_at = hits
                .iter()
                .map(|hit| hit.frame.captured_at.as_str())
                .max()
                .unwrap_or(representative.frame.captured_at.as_str())
                .to_string();
            let best_rank = hits
                .iter()
                .map(|hit| hit.rank)
                .min_by(|a, b| a.total_cmp(b))
                .unwrap_or(f64::INFINITY);
            let secret_redaction_count = hits
                .iter()
                .map(|hit| hit.secret_redaction_count)
                .max()
                .unwrap_or(0);
            // The group is meaning-only when no grouped anchor matched **Text
            // Search**: then there is no FTS term to highlight, so the snippet is
            // the leading `body_text` excerpt the semantic fetch carried. As soon
            // as any anchor matched a query term we prefer that highlighted
            // snippet and the group is a normal **Text Search** result.
            let text_hit = hits.iter().find(|hit| !hit.found_by_meaning);
            let found_by_meaning = text_hit.is_none();
            let snippet = text_hit.unwrap_or(&hits[0]).snippet.clone();
            Some((
                best_rank,
                FrameSearchResult {
                    group_key,
                    representative_frame: representative.frame.clone(),
                    group_start_at,
                    group_end_at,
                    match_count: hits.len() as u32,
                    snippet,
                    app_bundle_id: representative.app_bundle_id.clone(),
                    app_name: representative.app_name.clone(),
                    window_title: representative.window_title.clone(),
                    // Read-time: the representative frame's snapshot already
                    // carries `browser_url` (parsed by `map_frame_for_search`
                    // from the existing `frame_metadata_snapshots` join), so any
                    // historical frame is covered without an index column or
                    // backfill. The broker guards this URL before exposure.
                    browser_url: representative
                        .frame
                        .metadata_snapshot
                        .as_ref()
                        .and_then(|snapshot| snapshot.browser_url.clone()),
                    thumbnail_frame_id: representative.frame.id,
                    text_source_kind: representative.text_source_kind.clone(),
                    secret_redaction_count,
                    has_secret_redactions: secret_redaction_count > 0,
                    found_by_meaning,
                },
            ))
        })
        .collect::<Vec<_>>();

    results.sort_by(|(a_rank, a), (b_rank, b)| {
        a_rank
            .total_cmp(b_rank)
            .then_with(|| b.group_end_at.cmp(&a.group_end_at))
    });
    results.into_iter().map(|(_rank, result)| result).collect()
}

fn frame_hits_are_equivalent(left: &FrameHit, right: &FrameHit) -> bool {
    if left.frame.session_id != right.frame.session_id {
        return false;
    }

    let Some((_left_hint, left_proof, left_version)) = left.frame.equivalence.ready_parts() else {
        return left.frame.id == right.frame.id;
    };
    let Some((_right_hint, right_proof, right_version)) = right.frame.equivalence.ready_parts()
    else {
        return false;
    };
    CapturedFrameEquivalenceScope::from_frame(&left.frame)
        == CapturedFrameEquivalenceScope::from_frame(&right.frame)
        && left_version == right_version
        && capture_screen::captured_frame_equivalence_proofs_match(
            left_version,
            left_proof,
            right_proof,
        )
}

fn group_audio_hits(hits: &[AudioHit]) -> Result<Vec<AudioSearchResult>> {
    let mut hits = hits.to_vec();
    hits.sort_by(|a, b| {
        a.audio_segment
            .id
            .cmp(&b.audio_segment.id)
            .then_with(|| a.span_start_ms.cmp(&b.span_start_ms))
            .then_with(|| a.span_end_ms.cmp(&b.span_end_ms))
            .then_with(|| a.rank.total_cmp(&b.rank))
    });

    let mut groups: Vec<Vec<AudioHit>> = Vec::new();
    for hit in hits {
        if let Some(last_group) = groups.last_mut() {
            if let Some(last) = last_group.last() {
                if last.audio_segment.id == hit.audio_segment.id
                    && hit.span_start_ms <= last.span_end_ms.saturating_add(AUDIO_GROUP_GAP_MS)
                {
                    last_group.push(hit);
                    continue;
                }
            }
        }
        groups.push(vec![hit]);
    }

    let mut results = Vec::new();
    for mut group in groups {
        group.sort_by(|a, b| {
            a.rank
                .total_cmp(&b.rank)
                .then_with(|| a.span_start_ms.cmp(&b.span_start_ms))
        });
        let first = group.first().expect("group should not be empty");
        let span_start_ms = group.iter().map(|hit| hit.span_start_ms).min().unwrap_or(0);
        let span_end_ms = group
            .iter()
            .map(|hit| hit.span_end_ms)
            .max()
            .unwrap_or(span_start_ms);
        let absolute_start_at = timestamp_plus_ms(&first.audio_segment.started_at, span_start_ms)?;
        let absolute_end_at = timestamp_plus_ms(&first.audio_segment.started_at, span_end_ms)?;
        let secret_redaction_count = group
            .iter()
            .map(|hit| hit.secret_redaction_count)
            .max()
            .unwrap_or(0);
        // Meaning-only when no grouped span matched **Text Search** (see
        // `group_frame_hits`): then the snippet is the leading `body_text`
        // excerpt the semantic fetch carried, not a highlighted FTS snippet.
        let text_hit = group.iter().find(|hit| !hit.found_by_meaning);
        let found_by_meaning = text_hit.is_none();
        let snippet = text_hit.unwrap_or(first).snippet.clone();
        results.push((
            first.rank,
            AudioSearchResult {
                group_key: format!(
                    "audio:{}:{}-{}",
                    first.audio_segment.id, span_start_ms, span_end_ms
                ),
                audio_segment: first.audio_segment.clone(),
                source_kind: first.source_kind.clone(),
                span_start_ms,
                span_end_ms,
                absolute_start_at,
                absolute_end_at,
                match_count: group.len() as u32,
                snippet,
                aligned_frame: None,
                secret_redaction_count,
                has_secret_redactions: secret_redaction_count > 0,
                found_by_meaning,
            },
        ));
    }

    results.sort_by(|(a_rank, a), (b_rank, b)| {
        a_rank
            .total_cmp(b_rank)
            .then_with(|| b.absolute_start_at.cmp(&a.absolute_start_at))
            .then_with(|| a.group_key.cmp(&b.group_key))
    });
    Ok(results.into_iter().map(|(_rank, result)| result).collect())
}

fn map_processing_result_for_search(row: SqliteRow) -> Result<ProcessingResult> {
    Ok(ProcessingResult {
        id: row.get("id"),
        job_id: row.get("job_id"),
        subject_type: row.get("subject_type"),
        subject_id: row.get("subject_id"),
        processor: row.get("processor"),
        result_text: row.get("result_text"),
        structured_payload_json: row.get("structured_payload_json"),
        processor_version: row.get("processor_version"),
        redaction_detector_version: row.get("redaction_detector_version"),
        redaction_checked_at: row.get("redaction_checked_at"),
        created_at: row.get("created_at"),
    })
}

async fn align_audio_results(pool: &SqlitePool, results: &mut [AudioSearchResult]) -> Result<()> {
    for result in results {
        let mut candidate_session_ids = Vec::new();
        if let Some(screen_source_session_id) =
            screen_source_session_id_for_audio_alignment(pool, &result.audio_segment).await?
        {
            candidate_session_ids.push(screen_source_session_id);
        }
        if !candidate_session_ids
            .iter()
            .any(|session_id| session_id == &result.audio_segment.source_session_id)
        {
            candidate_session_ids.push(result.audio_segment.source_session_id.clone());
        }

        result.aligned_frame = None;
        for session_id in candidate_session_ids {
            if let Some(frame) =
                find_aligned_frame(pool, &session_id, &result.absolute_start_at).await?
            {
                result.aligned_frame = Some(frame);
                break;
            }
        }
    }
    Ok(())
}

async fn screen_source_session_id_for_audio_alignment(
    pool: &SqlitePool,
    segment: &AudioSegment,
) -> Result<Option<String>> {
    if let Some(capture_segment_id) = segment.capture_segment_id {
        let row = sqlx::query(
            "SELECT capture_sessions.screen_source_session_id \
             FROM capture_segments \
             JOIN capture_sessions ON capture_sessions.capture_session_id = capture_segments.capture_session_id \
             WHERE capture_segments.id = ?1 \
             ORDER BY capture_sessions.id DESC LIMIT 1",
        )
        .bind(capture_segment_id)
        .fetch_optional(pool)
        .await?;
        if let Some(session_id) =
            row.and_then(|row| normalized_source_session_id(row.get("screen_source_session_id")))
        {
            return Ok(Some(session_id));
        }
    }

    let source_column = match segment.source_kind {
        AudioSegmentSourceKind::Microphone => "microphone_source_session_id",
        AudioSegmentSourceKind::SystemAudio => "system_audio_source_session_id",
    };
    let query = format!(
        "SELECT screen_source_session_id \
         FROM capture_sessions \
         WHERE {source_column} = ?1 \
         ORDER BY id DESC LIMIT 1",
    );
    let row = sqlx::query(&query)
        .bind(&segment.source_session_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.and_then(|row| normalized_source_session_id(row.get("screen_source_session_id"))))
}

fn normalized_source_session_id(session_id: Option<String>) -> Option<String> {
    session_id.and_then(|session_id| {
        let trimmed = session_id.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

async fn find_aligned_frame(
    pool: &SqlitePool,
    session_id: &str,
    absolute_start_at: &str,
) -> Result<Option<Frame>> {
    let target = OffsetDateTime::parse(absolute_start_at, &Rfc3339).map_err(|error| {
        AppInfraError::FrameBatchFinalize(format!(
            "invalid search timestamp '{absolute_start_at}': {error}"
        ))
    })?;
    let before_start = target
        .checked_sub(Duration::seconds(AUDIO_FRAME_ALIGNMENT_WINDOW_SECONDS))
        .ok_or_else(|| {
            AppInfraError::FrameBatchFinalize("search alignment timestamp overflow".to_string())
        })?
        .format(&Rfc3339)
        .map_err(|error| {
            AppInfraError::FrameBatchFinalize(format!("failed to format search timestamp: {error}"))
        })?;
    let after_end = target
        .checked_add(Duration::seconds(AUDIO_FRAME_ALIGNMENT_WINDOW_SECONDS))
        .ok_or_else(|| {
            AppInfraError::FrameBatchFinalize("search alignment timestamp overflow".to_string())
        })?
        .format(&Rfc3339)
        .map_err(|error| {
            AppInfraError::FrameBatchFinalize(format!("failed to format search timestamp: {error}"))
        })?;

    let before = sqlx::query(
        "SELECT frames.id, session_id, file_path, captured_at, width, height, \
                equivalence_hint, equivalence_proof, equivalence_version, equivalence_status, equivalence_error, \
                frame_metadata_snapshots.snapshot_json AS metadata_snapshot_json, \
                frames.created_at, frames.updated_at \
         FROM frames \
         LEFT JOIN frame_metadata_snapshots ON frame_metadata_snapshots.id = frames.metadata_snapshot_id \
         WHERE session_id = ?1 AND captured_at >= ?2 AND captured_at <= ?3 \
         ORDER BY captured_at DESC, frames.id DESC LIMIT 1",
    )
    .bind(session_id)
    .bind(before_start)
    .bind(absolute_start_at)
    .fetch_optional(pool)
    .await?;
    if let Some(row) = before {
        return map_frame_for_search(row).map(Some);
    }

    let after = sqlx::query(
        "SELECT frames.id, session_id, file_path, captured_at, width, height, \
                equivalence_hint, equivalence_proof, equivalence_version, equivalence_status, equivalence_error, \
                frame_metadata_snapshots.snapshot_json AS metadata_snapshot_json, \
                frames.created_at, frames.updated_at \
         FROM frames \
         LEFT JOIN frame_metadata_snapshots ON frame_metadata_snapshots.id = frames.metadata_snapshot_id \
         WHERE session_id = ?1 AND captured_at > ?2 AND captured_at <= ?3 \
         ORDER BY captured_at ASC, frames.id ASC LIMIT 1",
    )
    .bind(session_id)
    .bind(absolute_start_at)
    .bind(after_end)
    .fetch_optional(pool)
    .await?;

    after.map(map_frame_for_search).transpose()
}

async fn get_audio_segment_for_search<'e, E>(
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

fn map_audio_hit(row: SqliteRow) -> Result<AudioHit> {
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
    Ok(AudioHit {
        anchor_id: row.get("document_id"),
        audio_segment,
        source_kind,
        span_start_ms: row
            .get::<Option<i64>, _>("span_start_ms")
            .unwrap_or(0)
            .max(0) as u64,
        span_end_ms: row.get::<Option<i64>, _>("span_end_ms").unwrap_or(0).max(0) as u64,
        snippet: row.get("snippet"),
        rank: row.get("rank"),
        secret_redaction_count: u32::try_from(row.get::<i64, _>("secret_redaction_count"))
            .unwrap_or(u32::MAX),
        // An FTS `MATCH` hit is a **Text Search** match; the semantic fetch path
        // builds its own `AudioHit`s with `found_by_meaning: true`.
        found_by_meaning: false,
    })
}

fn map_audio_segment_for_search(row: SqliteRow) -> Result<AudioSegment> {
    Ok(AudioSegment {
        id: row.get("id"),
        source_kind: AudioSegmentSourceKind::from_str(row.get::<String, _>("source_kind").as_str()),
        source_session_id: row.get("source_session_id"),
        segment_index: row.get("segment_index"),
        file_path: row.get("file_path"),
        started_at: row.get("started_at"),
        ended_at: row.get("ended_at"),
        capture_segment_id: row.get("capture_segment_id"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
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
    fn search_refinement_dates_normalize_to_utc() {
        let normalized = normalize_search_refinements(Some(SearchCaptureRefinements {
            date_range: Some(SearchDateRangeRefinement {
                start_at: "2026-05-17T04:59:00-05:00".to_string(),
                end_at: "2026-05-17T05:01:00-05:00".to_string(),
                origin: Some(SearchDateRangeOrigin::LastHour),
            }),
            apps: Vec::new(),
            window_title: None,
            audio_sources: Vec::new(),
            screen_source: false,
        }))
        .expect("refinements should not error")
        .expect("refinements should normalize");

        let range = normalized
            .date_range
            .as_ref()
            .expect("date range should be present");
        assert_eq!(range.start_at, "2026-05-17T09:59:00Z");
        assert_eq!(range.end_at, "2026-05-17T10:01:00Z");
        assert_eq!(
            normalized
                .applied
                .date_range
                .as_ref()
                .map(|range| (range.start_at.as_str(), range.end_at.as_str())),
            Some(("2026-05-17T09:59:00Z", "2026-05-17T10:01:00Z"))
        );
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
    fn plain_text_query_has_no_operators_and_matches_plain_fts() {
        // A query with no operators must behave exactly as the plain-text path:
        // residual equals the input, no refinements, and the FTS body is exactly
        // what the legacy plain-text translator produces.
        let parsed = parse_search_query("hello world");
        assert!(parsed.errors.is_empty());
        assert!(parsed.apps.is_empty());
        assert!(parsed.audio_sources.is_empty());
        assert!(parsed.date_range.is_none());
        assert_eq!(parsed.residual_query, "hello world");
        assert_eq!(
            parsed.fts_body,
            fts_query_for_plain_text(&normalize_query("hello world"))
        );
        assert_eq!(parsed.fts_body, "\"hello\" \"world\"");
    }

    #[test]
    fn quoted_phrase_body_operator_forces_literal_phrase() {
        let parsed = parse_search_query("\"hello world\"");
        assert!(parsed.errors.is_empty());
        assert_eq!(parsed.fts_body, "(\"hello world\")");
    }

    #[test]
    fn quoted_phrase_preserves_doubled_quotes_as_literal_quote() {
        // `""` inside a quoted run is an escaped literal `"`, not a close+reopen.
        // `"he said ""hi"""` must parse the phrase `he said "hi"` rather than
        // collapsing the doubled quotes away into `he said hi`.
        let parsed = parse_search_query("\"he said \"\"hi\"\"\"");
        assert!(
            parsed.errors.is_empty(),
            "unexpected parse errors: {:?}",
            parsed.errors
        );
        assert_eq!(parsed.fts_body, "(\"he said \"\"hi\"\"\")");
    }

    #[test]
    fn exclusion_body_operator_with_positive_term_is_fts_not() {
        let parsed = parse_search_query("error -warning");
        assert!(parsed.errors.is_empty());
        assert_eq!(parsed.fts_body, "(\"error\" NOT \"warning\")");
    }

    #[test]
    fn uppercase_or_body_operator_splits_groups_lowercase_or_is_literal() {
        let parsed = parse_search_query("foo OR bar");
        assert!(parsed.errors.is_empty());
        assert_eq!(parsed.fts_body, "(\"foo\") OR (\"bar\")");

        // lowercase `or` is a literal AND term, never a group split.
        let lower = parse_search_query("foo or bar");
        assert!(lower.errors.is_empty());
        assert_eq!(lower.fts_body, "\"foo\" \"or\" \"bar\"");
    }

    #[test]
    fn dangling_or_body_operator_is_a_parse_error() {
        // Leading, trailing, and doubled `OR` all leave an empty AND-group.
        // Strict validation (ADR 0019) rejects them instead of silently
        // rewriting into a broader valid search; no FTS body is produced.
        for query in ["foo OR", "OR foo", "foo OR OR bar"] {
            let parsed = parse_search_query(query);
            assert_eq!(
                parsed.errors.len(),
                1,
                "expected one dangling_or error for {query:?}, got {:?}",
                parsed.errors
            );
            assert_eq!(parsed.errors[0].kind, "dangling_or");
            assert!(
                parsed.fts_body.is_empty(),
                "dangling OR should produce no FTS body for {query:?}, got {:?}",
                parsed.fts_body
            );
        }

        // A well-formed OR with terms on both sides still parses cleanly.
        let ok = parse_search_query("foo OR bar");
        assert!(ok.errors.is_empty());
        assert_eq!(ok.fts_body, "(\"foo\") OR (\"bar\")");
    }

    #[test]
    fn prefix_body_operator_requires_two_leading_chars() {
        let parsed = parse_search_query("term*");
        assert!(parsed.errors.is_empty());
        assert_eq!(parsed.fts_body, "(\"term\"*)");

        // A single leading char does not qualify; it stays a literal term.
        let short = parse_search_query("a*");
        assert!(short.errors.is_empty());
        assert_eq!(short.fts_body, "(\"a\")");
    }

    #[test]
    fn app_field_operator_desugars_into_any_app_refinement() {
        let parsed = parse_search_query("app:Safari report");
        assert!(parsed.errors.is_empty());
        assert_eq!(parsed.apps.len(), 1);
        assert!(matches!(parsed.apps[0].kind, SearchAppRefinementKind::Any));
        assert_eq!(parsed.apps[0].value, "Safari");
        assert_eq!(parsed.residual_query, "report");
        assert_eq!(parsed.fts_body, "\"report\"");
    }

    #[test]
    fn app_field_operator_supports_quoted_multiword_and_reverse_dns() {
        let quoted = parse_search_query("app:\"Google Chrome\"");
        assert!(quoted.errors.is_empty());
        assert_eq!(quoted.apps.len(), 1);
        assert_eq!(quoted.apps[0].value, "Google Chrome");
        assert!(matches!(quoted.apps[0].kind, SearchAppRefinementKind::Any));

        // A reverse-DNS-looking value still works via the Any match kind because
        // a recognized `app:` value is never re-split as a field operator.
        let bundle = parse_search_query("app:com.google.Chrome");
        assert!(bundle.errors.is_empty());
        assert_eq!(bundle.apps.len(), 1);
        assert_eq!(bundle.apps[0].value, "com.google.Chrome");
        assert!(matches!(bundle.apps[0].kind, SearchAppRefinementKind::Any));
    }

    #[test]
    fn multiple_app_operators_accumulate_and_dedupe() {
        let parsed = parse_search_query("app:Safari app:Chrome app:Safari");
        assert!(parsed.errors.is_empty());
        let values = parsed
            .apps
            .iter()
            .map(|app| app.value.as_str())
            .collect::<Vec<_>>();
        assert_eq!(values, vec!["Safari", "Chrome"]);
    }

    #[test]
    fn source_field_operator_maps_to_audio_source_kinds() {
        let mic = parse_search_query("source:mic");
        assert!(mic.errors.is_empty());
        assert_eq!(mic.audio_sources, vec![AudioSegmentSourceKind::Microphone]);

        let both = parse_search_query("source:mic source:system");
        assert!(both.errors.is_empty());
        assert_eq!(
            both.audio_sources,
            vec![
                AudioSegmentSourceKind::Microphone,
                AudioSegmentSourceKind::SystemAudio
            ]
        );
    }

    #[test]
    fn unknown_source_value_is_in_band_error() {
        let parsed = parse_search_query("source:bluetooth");
        assert!(parsed.audio_sources.is_empty());
        assert_eq!(parsed.errors.len(), 1);
        assert_eq!(parsed.errors[0].kind, "unknown_source");
    }

    #[test]
    fn source_screen_operator_sets_screen_source_without_audio_sources() {
        let parsed = parse_search_query("source:screen meeting");
        assert!(parsed.errors.is_empty());
        assert!(parsed.screen_source);
        assert!(parsed.audio_sources.is_empty());
    }

    #[test]
    fn screen_and_audio_source_conflict_is_in_band_error() {
        let parsed = parse_search_query("source:screen source:mic");
        assert!(parsed.errors.is_empty());

        let errors =
            normalize_search_refinements(Some(merge_parsed_field_operators(None, &parsed)))
                .expect("conflict should not throw")
                .expect_err("screen + audio source should surface in-band parse errors");
        assert!(
            errors
                .iter()
                .any(|error| error.kind == "screen_audio_source_conflict"),
            "expected a screen_audio_source_conflict parse error, got {errors:?}"
        );
    }

    #[test]
    fn date_field_operators_resolve_to_a_single_overwriting_slot() {
        let parsed = parse_search_query("after:2026-01-01 before:2026-01-31");
        assert!(parsed.errors.is_empty());
        let range = parsed.date_range.expect("date range should be set");
        assert!(range.start_at.starts_with("2026-01-01T00:00:00"));
        assert!(range.end_at.starts_with("2026-01-31T23:59:59"));

        // `date:` writes both bounds; last write wins per slot.
        let day = parse_search_query("after:2020-01-01 date:2026-05-17");
        assert!(day.errors.is_empty());
        let day_range = day.date_range.expect("date range should be set");
        assert!(day_range.start_at.starts_with("2026-05-17T00:00:00"));
        assert!(day_range.end_at.starts_with("2026-05-17T23:59:59"));
    }

    #[test]
    fn bad_date_value_is_in_band_error() {
        let parsed = parse_search_query("after:notadate");
        assert!(parsed.date_range.is_none());
        assert_eq!(parsed.errors.len(), 1);
        assert_eq!(parsed.errors[0].kind, "bad_date");
        assert_eq!(parsed.errors[0].token, "after:notadate");
    }

    #[test]
    fn unknown_key_value_tokens_stay_literal_body_text() {
        // URL/code/error searches must keep working: only the known keys are
        // field operators, every other `key:value` is literal body text.
        for raw in ["http://github.com", "error:404", "fix: bug"] {
            let parsed = parse_search_query(raw);
            assert!(
                parsed.apps.is_empty()
                    && parsed.audio_sources.is_empty()
                    && parsed.date_range.is_none(),
                "`{raw}` must not desugar into any field operator"
            );
            assert!(parsed.errors.is_empty(), "`{raw}` must not error");
            assert!(
                parsed
                    .residual_query
                    .contains(raw.split(' ').next().unwrap()),
                "`{raw}` should remain in the residual body, got {:?}",
                parsed.residual_query
            );
        }

        // The literal http URL still produces a non-empty (safe) FTS body.
        let url = parse_search_query("http://github.com");
        assert!(!url.fts_body.is_empty());
    }

    #[test]
    fn unbalanced_quote_is_in_band_error() {
        let parsed = parse_search_query("\"unterminated phrase");
        assert!(
            parsed
                .errors
                .iter()
                .any(|error| error.kind == "unbalanced_quote"),
            "expected an unbalanced_quote error, got {:?}",
            parsed.errors
        );
    }

    #[test]
    fn pure_negation_without_positive_term_is_in_band_error() {
        let parsed = parse_search_query("-foo");
        assert!(
            parsed
                .errors
                .iter()
                .any(|error| error.kind == "pure_negation"),
            "expected a pure_negation error, got {:?}",
            parsed.errors
        );
    }

    #[test]
    fn error_spans_are_character_offsets_into_the_raw_query() {
        // Use a multi-byte prefix to prove spans are character (not byte) offsets.
        let parsed = parse_search_query("café source:bluetooth");
        let error = parsed
            .errors
            .iter()
            .find(|error| error.kind == "unknown_source")
            .expect("unknown_source error");
        // "café " is 5 characters; the 16-char token spans chars [5, 21).
        assert_eq!(error.start, 5);
        assert_eq!(error.end, 21);
        assert_eq!(error.token, "source:bluetooth");
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
