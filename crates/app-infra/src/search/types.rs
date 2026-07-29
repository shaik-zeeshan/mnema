use serde::{Deserialize, Serialize};
use sqlx::{QueryBuilder, Sqlite};

use crate::processing::Frame;
use crate::{AudioSegment, AudioSegmentSourceKind};

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
    /// Case-insensitive substring over the frame's GUARDED url (`host[:port]/path`
    /// — query strings and fragments are never indexed, so they can never be
    /// filtered on either).
    #[serde(default)]
    pub url: Option<String>,
    /// Case-sensitive regular expression over the same guarded url (opt into
    /// case-insensitivity with `(?i)`). Validated at normalization time so an
    /// invalid pattern surfaces a parse error instead of a SQL failure.
    #[serde(default)]
    pub url_regex: Option<String>,
    #[serde(default)]
    pub audio_sources: Vec<AudioSegmentSourceKind>,
    /// `source:screen` restricts results to captured frames (screen), skipping
    /// audio. It is the frame-side counterpart of `audio_sources` and cannot be
    /// combined with them.
    #[serde(default)]
    pub screen_source: bool,
    /// One speaker's audio. Like `audio_sources` it narrows to AUDIO — a captured
    /// frame carries no voice — so it cannot be combined with the app / window
    /// title / url filters, which exist only on frames.
    #[serde(default)]
    pub speaker: Option<SearchSpeakerRefinement>,
}

/// Which speaker to narrow to, as the DECODED row it addresses. The opaque
/// handle an agent holds is decoded at the broker boundary; search only ever
/// sees the row id, so there is one place that can misread a handle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchSpeakerRefinement {
    /// `person_profiles.id` — the same human across sessions and channels.
    Person(i64),
    /// `recording_speaker_clusters.id` — ONE voice inside ONE capture SESSION.
    /// The row is `UNIQUE(session_id, provider, provider_cluster_id)`, so matching
    /// turns on it can never reach another session's audio — but a session is not
    /// a recording. Segments are capped at 5 minutes, so one sitting is several
    /// consecutive `audio_segments`, and `resolve_stable_speaker_cluster`
    /// deliberately reuses this row across all of them. Matching on it therefore
    /// spans every recording in that sitting, by design.
    Cluster(i64),
}

impl SearchSpeakerRefinement {
    /// `EXISTS` over one audio segment's speaker turns.
    ///
    /// The person arm reuses `broker_collapse_speakers`'s precedence VERBATIM: a
    /// user assignment wins, and a recognition guess counts only where the
    /// cluster carries no assignment. Excluding recognition would make the filter
    /// contradict `show-text` about the same audio (and, since nothing in this app
    /// ever auto-links, return close to nothing); including it means results carry
    /// voice matches the user never confirmed. That is the accepted trade.
    ///
    /// `audio_segment_id_sql` is the caller's own column expression — a literal at
    /// every call site, never caller input — so the one predicate serves both the
    /// search index and the timeline's `audio_segments` scan.
    pub(crate) fn push_exists_predicate(
        &self,
        query: &mut QueryBuilder<'_, Sqlite>,
        audio_segment_id_sql: &str,
    ) {
        query.push("EXISTS (SELECT 1");
        self.push_matching_turns_source(query);
        query.push(" AND speaker_turns.audio_segment_id = ");
        query.push(audio_segment_id_sql);
        query.push(")");
    }

    /// ` FROM speaker_turns [JOIN …] WHERE <this speaker>` — the half of
    /// [`Self::push_exists_predicate`] that decides WHOSE turns these are, split out
    /// so a caller can select the matched turn ROWS instead of merely testing for
    /// them. Shared on purpose: two copies of this precedence would eventually
    /// disagree, and a filter that returns one person's recordings with another
    /// person's words in them is worse than no words at all. Callers append their
    /// own ` AND …` restriction.
    pub(crate) fn push_matching_turns_source(&self, query: &mut QueryBuilder<'_, Sqlite>) {
        query.push(" FROM speaker_turns ");
        match self {
            Self::Person(person_id) => {
                query.push(
                    "JOIN recording_speaker_clusters \
                            ON recording_speaker_clusters.id = speaker_turns.cluster_id \
                     WHERE (recording_speaker_clusters.person_id = ",
                );
                query.push_bind(*person_id);
                query.push(
                    " OR (recording_speaker_clusters.person_id IS NULL \
                          AND recording_speaker_clusters.recognition_person_id = ",
                );
                query.push_bind(*person_id);
                query.push("))");
            }
            Self::Cluster(cluster_id) => {
                query.push("WHERE speaker_turns.cluster_id = ");
                query.push_bind(*cluster_id);
            }
        }
    }
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FrameSearchResult {
    /// Relevance score of the group's best anchor — LOWER is better, matching the
    /// BM25 ordering the grouping uses. Comparable between frame and audio results
    /// because both score against the same `search_documents_fts` index with the
    /// same weights, so a consumer can merge the two lists into one ranked page.
    /// CAVEAT: under **Hybrid Search** this is an RRF score derived from a hit's
    /// POSITION within its own kind's list, which is not comparable across kinds.
    #[serde(default)]
    pub rank: f64,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AudioSearchResult {
    /// Relevance score of the group's best anchor — LOWER is better, matching the
    /// BM25 ordering the grouping uses. Comparable between frame and audio results
    /// because both score against the same `search_documents_fts` index with the
    /// same weights, so a consumer can merge the two lists into one ranked page.
    /// CAVEAT: under **Hybrid Search** this is an RRF score derived from a hit's
    /// POSITION within its own kind's list, which is not comparable across kinds.
    #[serde(default)]
    pub rank: f64,
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
pub(super) struct NormalizedSearchRefinements {
    pub(super) date_range: Option<NormalizedDateRange>,
    pub(super) apps: Vec<NormalizedAppRefinement>,
    pub(super) window_title: Option<String>,
    pub(super) url: Option<String>,
    pub(super) url_regex: Option<String>,
    pub(super) audio_sources: Vec<AudioSegmentSourceKind>,
    pub(super) screen_source: bool,
    pub(super) speaker: Option<SearchSpeakerRefinement>,
    pub(super) applied: SearchCaptureRefinements,
}

#[derive(Debug, Clone)]
pub(super) struct NormalizedDateRange {
    pub(super) start_at: String,
    pub(super) end_at: String,
}

#[derive(Debug, Clone)]
pub(super) enum NormalizedAppRefinement {
    Any { value: String, search_key: String },
    BundleId { value: String },
    AppName { search_key: String },
}

pub(crate) struct EquivalentReuseText {
    pub(crate) result_text: String,
    pub(crate) source_subject_type: String,
    pub(crate) source_subject_id: i64,
}

pub(super) fn normalize_app_bundle_id_for_search(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

pub(super) fn normalize_app_name_for_search(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_lowercase())
}
