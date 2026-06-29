use serde::{Deserialize, Serialize};

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
pub(super) struct NormalizedSearchRefinements {
    pub(super) date_range: Option<NormalizedDateRange>,
    pub(super) apps: Vec<NormalizedAppRefinement>,
    pub(super) window_title: Option<String>,
    pub(super) audio_sources: Vec<AudioSegmentSourceKind>,
    pub(super) screen_source: bool,
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
