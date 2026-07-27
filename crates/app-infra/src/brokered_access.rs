use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use fs2::FileExt;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{QueryBuilder, Row, Sqlite};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::{
    AppInfra, AppInfraError, AudioSegmentSourceKind, ProcessingSubject, Result,
    SearchAppRefinement, SearchAppRefinementKind, SearchCaptureRefinements, SearchCaptureRequest,
    SearchCaptureResponse, SearchDateRangeOrigin, SearchDateRangeRefinement,
    SearchSpeakerRefinement, AUDIO_SEGMENT_SUBJECT_TYPE, FRAME_SUBJECT_TYPE,
};

// Read-time URL guard. The `guard_url` entry point turns a raw captured browser
// URL into a sanitized, secret-redacted `host[:port]/path`. It is consumed both
// internally (broker search/timeline context) and across crates (the desktop DTO
// + Ask AI source mappers) via the crate-root `guard_browser_url` re-export.
mod url_guard;

pub use url_guard::{
    guard_url, ip_is_disallowed_fetch_target, is_disallowed_fetch_url, secret_scrubbed_fetch_target,
    URL_GUARD_VERSION,
};

mod speakers;

use speakers::{
    broker_speaker_refinement, broker_speakers, broker_speakers_for_audio,
    speaker_matched_audio_segment_ids,
};

const BROKER_GRANTS_FILE_NAME: &str = "broker-grants.json";
const BROKER_GRANTS_LOCK_FILE_NAME: &str = "broker-grants.lock";
const BROKER_AUDIT_LOCK_FILE_NAME: &str = "broker-audit.lock";
const BROKER_AUDIT_FILE_NAME: &str = "broker-audit.json";
const BROKER_OPAQUE_SECRET_FILE_NAME: &str = "broker-opaque-secret.bin";
const RECORDING_SETTINGS_FILE_NAME: &str = "recording-settings.json";
const DEFAULT_SEARCH_LIMIT: u32 = 20;
const MAX_SEARCH_LIMIT: u32 = 100;
/// How deep a cursor walk may go per anchor kind (20 full pages). The cursor is
/// client round-tripped and unsigned, so its offsets are attacker-chosen; capping
/// them keeps the work one request can buy bounded. See `BrokerSearchCursor::decode`.
const MAX_CURSOR_OFFSET: u32 = 10 * MAX_SEARCH_LIMIT;
const OPAQUE_SIGNATURE_HEX_LEN: usize = 32;
const DEFAULT_APP_IDENTIFIER: &str = env!("MNEMA_APP_IDENTIFIER");

/// Stable grant id for the in-app Ask AI agent's All Retained Broker Scope access.
///
/// Ask AI is authorized by the Ask AI Setting at the Tauri layer rather than by a
/// persisted, user-approved broker grant, so its scope is represented by a synthetic
/// in-memory grant. The id is a constant (not generated per call) so opaque ids issued
/// by a `search` call re-authorize on a later `show-text` call.
pub const ASK_AI_BROKER_GRANT_ID: &str = "ask-ai-all-retained";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrokerAuthStatusKind {
    Authorized,
    AuthorizationRequired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerAuthStatus {
    pub status: BrokerAuthStatusKind,
    pub reason: Option<String>,
    pub active_grant_count: usize,
}

impl BrokerAuthStatus {
    pub fn authorization_required() -> Self {
        Self {
            status: BrokerAuthStatusKind::AuthorizationRequired,
            reason: Some(
                "Mnema UI authorization is required before brokered capture access is available"
                    .to_string(),
            ),
            active_grant_count: 0,
        }
    }

    pub fn authorized(active_grant_count: usize) -> Self {
        Self {
            status: BrokerAuthStatusKind::Authorized,
            reason: None,
            active_grant_count,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerErrorResponse {
    pub error: BrokerAuthStatusKind,
    pub message: String,
}

impl BrokerErrorResponse {
    pub fn authorization_required() -> Self {
        let status = BrokerAuthStatus::authorization_required();
        Self {
            error: status.status,
            message: status.reason.unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerClientIdentity {
    pub label: String,
    pub normalized_label: String,
    pub source: BrokerClientIdentitySource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BrokerClientIdentitySource {
    Explicit,
    Env,
    Inferred,
    Defaulted,
}

impl BrokerClientIdentity {
    pub fn new(label: impl Into<String>, source: BrokerClientIdentitySource) -> Result<Self> {
        let label = label.into();
        let normalized_label = normalize_client_label(&label).ok_or_else(|| {
            AppInfraError::BrokeredAccess("CLI Access client name is invalid".to_string())
        })?;
        Ok(Self {
            label: display_client_label(&label),
            normalized_label,
            source,
        })
    }

    pub fn default_cli() -> Self {
        Self {
            label: "mnema CLI".to_string(),
            normalized_label: "mnema cli".to_string(),
            source: BrokerClientIdentitySource::Defaulted,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrokerGrantScope {
    RecentDays { days: u32 },
    AllRetainedHistory,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerGrant {
    pub id: String,
    pub label: String,
    #[serde(default = "default_grant_normalized_label")]
    pub normalized_label: String,
    #[serde(default = "default_grant_identity_source")]
    pub identity_source: BrokerClientIdentitySource,
    pub created_at_unix_ms: u64,
    pub expires_at_unix_ms: u64,
    pub scope: BrokerGrantScope,
    #[serde(default)]
    pub revoked: bool,
    #[serde(default)]
    pub revoked_at_unix_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct BrokerGrantFile {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub grants: Vec<BrokerGrant>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct BrokerAuditFile {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub events: Vec<BrokerAuditEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerAuditEvent {
    pub tool_identity: String,
    #[serde(default)]
    pub normalized_tool_identity: String,
    #[serde(default = "default_grant_identity_source")]
    pub identity_source: BrokerClientIdentitySource,
    pub command_type: String,
    pub timestamp_unix_ms: u64,
    pub result_count: u32,
    pub scope_class: String,
    #[serde(default)]
    pub grant_id: Option<String>,
    #[serde(default)]
    pub outcome: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerGrantCreateRequest {
    pub label: Option<String>,
    pub duration_hours: Option<u64>,
    pub all_retained_history: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerSearchRequest {
    pub query: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: Option<u32>,
    pub app: Option<String>,
    pub window_title: Option<String>,
    /// Case-insensitive substring over the indexed (guarded) url.
    pub url: Option<String>,
    /// Case-sensitive regex over the same url.
    pub url_regex: Option<String>,
    /// An opaque speaker handle from `speakers` (or from any `show-text` speaker),
    /// narrowing to AUDIO this person or voice was heard in. Matches turns the user
    /// ASSIGNED and turns voice recognition GUESSED where no assignment exists, so
    /// results can include people the user never confirmed. Cannot be combined with
    /// `app`, `windowTitle`, `url`, or `urlRegex` — those live on captured frames,
    /// which carry no voice.
    #[serde(default)]
    pub speaker: Option<String>,
    /// Opaque `nextCursor` from a previous page of the SAME query. Carries the
    /// search-document high-water mark plus per-anchor offsets, so a walk stays
    /// pinned to the snapshot it started on and never re-reads or skips rows as
    /// new captures land. Absent = first page.
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerSearchResult {
    pub opaque_id: String,
    pub kind: String,
    pub snippet: String,
    pub started_at: String,
    pub ended_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<BrokerSearchResultContext>,
    // Audio Search Result Anchor: sub-segment match timing + aligned frame for
    // audio results so consumers can land on the cited moment rather than the
    // segment start. Always `None` for frame results (no sub-segment anchor).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span_start_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span_end_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aligned_frame_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerSearchResultContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_bundle_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_title: Option<String>,
    /// Guarded host+path of the page behind this result (read-time, sanitized
    /// + secret-redacted). Cloud-facing; the raw URL never appears here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerSearchResponse {
    pub results: Vec<BrokerSearchResult>,
    /// Page-size CEILING applied after server-side clamping to
    /// [`MAX_SEARCH_LIMIT`] — not a promise of how many rows came back. The
    /// search layer additionally caps each anchor kind at its own group limit,
    /// so a short page can still have a `next_cursor`; that field, never the
    /// row count, is what says whether the walk is done.
    pub limit: u32,
    /// Cursor for the next page, or `None` when this page exhausted the matches.
    /// Feed it back verbatim as [`BrokerSearchRequest::cursor`] with the same
    /// query and filters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Decoded [`BrokerSearchRequest::cursor`]: the snapshot the walk is pinned to
/// plus how many frame / audio anchors it has already consumed. Encoded as
/// `v1:<snapshot>:<frameOffset>:<audioOffset>` — unsigned on purpose, since it
/// carries no reference to data (scope is re-derived from live grants on every
/// request, so a forged cursor can only skip rows, never widen access) — but a
/// forged OFFSET still buys work, so both offsets are bounded by
/// [`MAX_CURSOR_OFFSET`] at decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BrokerSearchCursor {
    snapshot_document_id: i64,
    frame_offset: u32,
    audio_offset: u32,
}

impl BrokerSearchCursor {
    fn encode(&self) -> String {
        format!(
            "v1:{}:{}:{}",
            self.snapshot_document_id, self.frame_offset, self.audio_offset
        )
    }

    fn decode(raw: &str) -> Result<Self> {
        let invalid =
            || AppInfraError::InvalidSearchRequest("cursor is not a valid search cursor".into());
        let mut parts = raw.trim().split(':');
        if parts.next() != Some("v1") {
            return Err(invalid());
        }
        let mut next = || parts.next().ok_or_else(invalid);
        let snapshot_document_id: i64 = next()?.parse().map_err(|_| invalid())?;
        let frame_offset: u32 = next()?.parse().map_err(|_| invalid())?;
        let audio_offset: u32 = next()?.parse().map_err(|_| invalid())?;
        if parts.next().is_some() || snapshot_document_id < 0 {
            return Err(invalid());
        }
        // The broker only ever mints an offset by ADVANCING it by what one page
        // consumed, so anything past the paging window is a forgery. It is not
        // harmless: `frame_offset` becomes `needed_groups` in the search layer's
        // drain loop, which then fetches and re-groups (quadratically) the ENTIRE
        // match set for the query before handing back an empty page.
        if frame_offset > MAX_CURSOR_OFFSET || audio_offset > MAX_CURSOR_OFFSET {
            return Err(AppInfraError::InvalidSearchRequest(
                "cursor is past the end of the paging window — narrow the query or its time range"
                    .into(),
            ));
        }
        Ok(Self {
            snapshot_document_id,
            frame_offset,
            audio_offset,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerShowTextResponse {
    pub opaque_id: String,
    pub kind: String,
    pub text: String,
    /// One entry per speaker cluster heard in an audio result, ordered by first
    /// turn. Empty for frames and for audio without speaker analysis.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub speakers: Vec<BrokerSpeaker>,
    /// Which speaker said which words, each turn pointing at a `speakers[]` index.
    ///
    /// An ATTRIBUTION OVERLAY on `text`, never a decomposition of it: it may cover
    /// only part of the recording, and `text` always carries the full transcript.
    ///
    /// ABSENT `turns` MEANS "COULD NOT ATTRIBUTE", **NOT** "NOBODY SPOKE". Speaker
    /// detection produces nothing at all for plenty of audio the transcriber
    /// handled fine, so a recording with words in `text` and no `turns` is normal
    /// and is NOT silence. Never report an unattributed recording as empty, as
    /// nobody speaking, or as no one being present — read `text` instead.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub turns: Vec<BrokerSpeakerTurn>,
}

/// A voice in a brokered audio result. `name` is only ever a person the user
/// created a profile for — an unrecognized cluster carries `None`, never the
/// internal "Speaker 2" label, which would be noise to the caller.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerSpeaker {
    pub name: Option<String>,
    /// `assigned` (the user said so), `recognized` (voice match), `unknown`.
    pub attribution: String,
    /// Recognition confidence (`high`/`medium`/`low`) when `recognized`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<String>,
    /// How to address this speaker. Always present: address people by handle,
    /// never by matching the `name` string, which two profiles can share.
    pub handle: BrokerSpeakerHandle,
}

/// An opaque id addressing a speaker. NOT a capture reference — `show-text` and
/// `open` reject it, because a person is not a captured frame or recording.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerSpeakerHandle {
    pub id: String,
    /// `person` — a person the user has a profile for. Stable: it spans sessions
    /// and channels and survives a rename.
    ///
    /// `voice` — ONE voice inside ONE recording, **not a person**. The same human
    /// gets a different `voice` handle in every recording (and often several
    /// within one), and the handle dies when that recording is re-analyzed. Never
    /// persist it, never merge two of them, and never present it as an identity.
    pub kind: String,
    /// `voice` only: the span this voice was heard over, in ms from the start of
    /// the recording — the whole extent this handle means anything across.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_ms: Option<u64>,
}

/// One attributed stretch of speech: who said it, when, and the words.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerSpeakerTurn {
    /// Index into the response's `speakers[]`.
    pub speaker: usize,
    /// Milliseconds from the start of the recording.
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerOpenInMnemaResponse {
    pub opened: bool,
    pub opaque_id: String,
}

/// Response shape for `OpenCapturedUrl`. RETAINED for protocol/match-arm
/// stability only: the broker NEVER produces a success here — `OpenCapturedUrl`
/// is rejected for every caller (see `execute_authorized_request`). Opening the
/// raw captured `browser_url` is exclusively the LOCAL desktop Tauri command
/// keyed off `frame_id` behind a user click (ADR 0038); the raw URL is local-only
/// and was never carried on this struct (only `opaque_id`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerOpenCapturedUrlResponse {
    pub opened: bool,
    pub opaque_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerOpaqueCaptureReference {
    pub opaque_id: String,
    pub kind: String,
    pub frame_id: Option<i64>,
    pub audio_segment_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerTimelineRequest {
    pub from: String,
    pub to: String,
    pub limit: Option<u32>,
    pub app: Option<String>,
    pub window_title: Option<String>,
    /// Case-insensitive substring over the indexed (guarded) url.
    pub url: Option<String>,
    /// Case-sensitive regex over the same url.
    pub url_regex: Option<String>,
    /// An opaque speaker handle, narrowing the timeline to audio this person or
    /// voice was heard in — "when was Priya talking yesterday" has no query string
    /// in it, so it is answerable here and nowhere else. Same matching rules and
    /// same conflict as [`BrokerSearchRequest::speaker`].
    #[serde(default)]
    pub speaker: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerTimelineInterval {
    pub kind: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    /// Followable capture id for this interval (`show-text` resolves it). Absent
    /// when the interval has no representative capture to point at.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opaque_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<BrokerSearchResultContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerTimelineResponse {
    pub intervals: Vec<BrokerTimelineInterval>,
    pub limit: u32,
}

/// Who was heard **inside the grant's own time scope** — never the global people
/// list, which spans audio this grant does not cover.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct BrokerSpeakersRequest {
    /// Case-insensitive substring over a person's name. Present, it searches
    /// NAMED people only — a quiet person ranks below the cap and is otherwise
    /// unfindable, which is what this filter exists for.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerSpeakersResponse {
    /// Ranked by total speaking time in scope, longest first.
    pub speakers: Vec<BrokerSpeakerSummary>,
    /// Page-size ceiling after server-side clamping.
    pub limit: u32,
    /// More speakers were heard than `limit` returned. **This list is NOT
    /// everyone** when set — narrow the search with `name` rather than reading
    /// the ranked page as the full roster.
    pub truncated: bool,
}

/// One voice heard in scope: how to address it, how long it spoke, and how much
/// of that identity the user actually confirmed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerSpeakerSummary {
    /// `None` for a `voice` handle — an unnamed voice has no name to report.
    pub name: Option<String>,
    pub handle: BrokerSpeakerHandle,
    /// Total speaking time in scope, milliseconds. The ranking key.
    pub speaking_ms: u64,
    /// Turns the USER assigned to this person. Confirmed identity.
    pub assigned_turns: u32,
    /// Turns matched to this person by VOICE RECOGNITION with no assignment —
    /// guesses, not confirmations. Weigh them before filtering on this handle.
    pub recognized_turns: u32,
}

/// A `recall_context` request: the user's question, an optional cap on how many
/// recalled items to return, and optional `from`/`to` RFC3339 UTC bounds that
/// scope the recalled ACTIVITIES by date (mirroring `search`/`timeline`). The
/// cap is clamped server-side so it can never return the whole dossier; the time
/// bounds filter activities only — Conclusions are standing beliefs and carry no
/// wire timestamp, so they are never scoped. Omitting both bounds is the legacy
/// recency-bounded keyword behavior.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerRecallContextRequest {
    pub query: String,
    pub limit: Option<u32>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
}

/// A single redacted Conclusion returned by `recall_context`. Carries no ids,
/// evidence refs, or anything pointing at raw frames/audio — only the distilled,
/// already-redacted English belief.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerRecalledConclusion {
    pub subject: String,
    pub statement: String,
    pub confidence: f64,
    pub status: String,
}

/// A single redacted Activity returned by `recall_context`. Carries no ids or
/// evidence refs; times are RFC3339 strings like the other broker responses.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerRecalledActivity {
    pub title: String,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
    pub started_at: String,
    pub ended_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerRecallContextResponse {
    pub conclusions: Vec<BrokerRecalledConclusion>,
    pub activities: Vec<BrokerRecalledActivity>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrokeredCaptureRequest {
    AuthStatus,
    Search(BrokerSearchRequest),
    ShowText { opaque_id: String },
    Timeline(BrokerTimelineRequest),
    Speakers(BrokerSpeakersRequest),
    RecallContext(BrokerRecallContextRequest),
    OpenInMnema { opaque_id: String },
    OpenCapturedUrl { opaque_id: String },
}

impl BrokeredCaptureRequest {
    fn command_type(&self) -> Option<&'static str> {
        match self {
            Self::AuthStatus => None,
            Self::Search(_) => Some("search"),
            Self::ShowText { .. } => Some("show_text"),
            Self::Timeline(_) => Some("timeline"),
            // The audit records THAT a speaker lookup ran, never the name it was
            // given or the handles it returned — `record_audit_event` stores no
            // request parameters, deliberately.
            Self::Speakers(_) => Some("speakers"),
            Self::RecallContext(_) => Some("recall_context"),
            Self::OpenInMnema { .. } => Some("open_in_mnema"),
            Self::OpenCapturedUrl { .. } => Some("open_captured_url"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum BrokeredCaptureResponse {
    AuthStatus(BrokerAuthStatus),
    Search(BrokerSearchResponse),
    ShowText(BrokerShowTextResponse),
    Timeline(BrokerTimelineResponse),
    Speakers(BrokerSpeakersResponse),
    RecallContext(BrokerRecallContextResponse),
    OpenInMnema(BrokerOpenInMnemaResponse),
    OpenCapturedUrl(BrokerOpenCapturedUrlResponse),
    Error(BrokerErrorResponse),
}

impl BrokeredCaptureResponse {
    fn result_count(&self) -> u32 {
        match self {
            Self::Search(response) => response.results.len() as u32,
            Self::ShowText(_) | Self::OpenInMnema(_) | Self::OpenCapturedUrl(_) => 1,
            Self::Timeline(response) => response.intervals.len() as u32,
            Self::Speakers(response) => response.speakers.len() as u32,
            Self::RecallContext(response) => {
                (response.conclusions.len() + response.activities.len()) as u32
            }
            Self::AuthStatus(_) | Self::Error(_) => 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BrokeredCaptureAccess {
    config_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct RecordingSettingsFile {
    save_directory: String,
}

impl BrokeredCaptureAccess {
    pub fn from_config_dir(config_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
        }
    }

    pub fn from_app_identifier(identifier: &str) -> Result<Self> {
        let config_dir = default_app_config_dir_for_identifier(identifier).ok_or_else(|| {
            AppInfraError::BrokeredAccess("failed to resolve Mnema app config dir".to_string())
        })?;
        Ok(Self::from_config_dir(config_dir))
    }

    pub fn from_default_app_config_dir() -> Result<Self> {
        let config_dir = default_app_config_dir().ok_or_else(|| {
            AppInfraError::BrokeredAccess("failed to resolve Mnema app config dir".to_string())
        })?;
        Ok(Self::from_config_dir(config_dir))
    }

    pub async fn execute(
        &self,
        tool_identity: impl Into<String>,
        request: BrokeredCaptureRequest,
    ) -> Result<BrokeredCaptureResponse> {
        let identity =
            BrokerClientIdentity::new(tool_identity.into(), BrokerClientIdentitySource::Explicit)
                .unwrap_or_else(|_| BrokerClientIdentity::default_cli());
        self.execute_for_identity(identity, request).await
    }

    pub async fn execute_for_identity(
        &self,
        identity: BrokerClientIdentity,
        request: BrokeredCaptureRequest,
    ) -> Result<BrokeredCaptureResponse> {
        if matches!(&request, BrokeredCaptureRequest::AuthStatus) {
            return Ok(BrokeredCaptureResponse::AuthStatus(auth_status_for_config(
                &self.config_dir,
                Some(&identity),
            )?));
        }

        let command_type = request.command_type();
        let grants = self.active_grants_for_identity(&identity)?;
        let response = if grants.is_empty() {
            BrokeredCaptureResponse::Error(BrokerErrorResponse::authorization_required())
        } else {
            self.execute_authorized_request(&grants, request).await?
        };

        if let Some(command_type) = command_type {
            self.audit_result(&grants, identity, command_type, response.result_count())?;
        }

        Ok(response)
    }

    /// Execute a brokered query at All Retained Broker Scope for the in-app Ask AI agent.
    ///
    /// Unlike [`execute_for_identity`], Ask AI access is gated by the Ask AI Setting at the
    /// Tauri layer (fail-closed) rather than by a persisted broker grant, so this path injects
    /// a synthetic All Retained grant instead of loading disk grants. Only the agent's data
    /// tools are permitted; `OpenInMnema` is an app-mediated handoff (ADR 0024) and is rejected.
    /// `OpenCapturedUrl` is rejected here too (defense-in-depth: the shared
    /// `execute_authorized_request` also rejects it universally — the broker never opens a raw
    /// captured URL for any caller; that is local-desktop-only, see ADR 0038).
    pub async fn execute_for_ask_ai(
        &self,
        identity: BrokerClientIdentity,
        request: BrokeredCaptureRequest,
    ) -> Result<BrokeredCaptureResponse> {
        if matches!(
            request,
            BrokeredCaptureRequest::OpenInMnema { .. }
                | BrokeredCaptureRequest::OpenCapturedUrl { .. }
        ) {
            return Ok(BrokeredCaptureResponse::Error(
                BrokerErrorResponse::authorization_required(),
            ));
        }
        if matches!(&request, BrokeredCaptureRequest::AuthStatus) {
            return Ok(BrokeredCaptureResponse::AuthStatus(
                BrokerAuthStatus::authorized(1),
            ));
        }
        let command_type = request.command_type();
        let grants = vec![ask_ai_all_retained_grant(&identity)];
        let response = self.execute_authorized_request(&grants, request).await?;
        if let Some(command_type) = command_type {
            self.audit_result(&grants, identity, command_type, response.result_count())?;
        }
        Ok(response)
    }

    pub fn list_grants(&self) -> Result<BrokerGrantFile> {
        load_grants(&self.config_dir)
    }

    pub fn create_grant(&self, request: BrokerGrantCreateRequest) -> Result<BrokerGrant> {
        create_grant_from_request(&self.config_dir, request)
    }

    pub fn create_grant_for_identity(
        &self,
        identity: BrokerClientIdentity,
        duration_hours: u64,
        scope: BrokerGrantScope,
    ) -> Result<BrokerGrant> {
        create_grant_for_identity(&self.config_dir, identity, duration_hours, scope)
    }

    pub fn revoke_grant(&self, grant_id: &str) -> Result<bool> {
        revoke_grant(&self.config_dir, grant_id)
    }

    pub fn revoke_grants_for_client(&self, client_label: &str) -> Result<u32> {
        revoke_grants_for_client(&self.config_dir, client_label)
    }

    pub fn list_history(&self) -> Result<BrokerAuditFile> {
        load_audit_events(&self.config_dir)
    }

    fn active_grants_for_identity(
        &self,
        identity: &BrokerClientIdentity,
    ) -> Result<Vec<BrokerGrant>> {
        let grants = load_grants(&self.config_dir)?;
        Ok(active_grants_for_identity(&grants, identity, now_unix_ms()))
    }

    async fn execute_authorized_request(
        &self,
        grants: &[BrokerGrant],
        request: BrokeredCaptureRequest,
    ) -> Result<BrokeredCaptureResponse> {
        match request {
            BrokeredCaptureRequest::AuthStatus => Ok(BrokeredCaptureResponse::AuthStatus(
                BrokerAuthStatus::authorized(grants.len()),
            )),
            BrokeredCaptureRequest::Search(request) => {
                let infra = self.initialize_infra().await?;
                match broker_search(&self.config_dir, &infra, grants, request).await? {
                    Ok(response) => Ok(BrokeredCaptureResponse::Search(response)),
                    Err(error) => Ok(BrokeredCaptureResponse::Error(error)),
                }
            }
            BrokeredCaptureRequest::ShowText { opaque_id } => {
                let infra = self.initialize_infra().await?;
                match broker_show_text(&self.config_dir, &infra, grants, &opaque_id).await? {
                    Ok(response) => Ok(BrokeredCaptureResponse::ShowText(response)),
                    Err(error) => Ok(BrokeredCaptureResponse::Error(error)),
                }
            }
            BrokeredCaptureRequest::Timeline(request) => {
                let infra = self.initialize_infra().await?;
                match broker_timeline(&self.config_dir, &infra, grants, request).await? {
                    Ok(response) => Ok(BrokeredCaptureResponse::Timeline(response)),
                    Err(error) => Ok(BrokeredCaptureResponse::Error(error)),
                }
            }
            BrokeredCaptureRequest::Speakers(request) => {
                let infra = self.initialize_infra().await?;
                match broker_speakers(&self.config_dir, &infra, grants, request).await? {
                    Ok(response) => Ok(BrokeredCaptureResponse::Speakers(response)),
                    Err(error) => Ok(BrokeredCaptureResponse::Error(error)),
                }
            }
            BrokeredCaptureRequest::RecallContext(request) => {
                let infra = self.initialize_infra().await?;
                match broker_recall_context(&infra, grants, request).await? {
                    Ok(response) => Ok(BrokeredCaptureResponse::RecallContext(response)),
                    Err(error) => Ok(BrokeredCaptureResponse::Error(error)),
                }
            }
            BrokeredCaptureRequest::OpenInMnema { opaque_id } => {
                if opaque_capture_reference(&opaque_id).is_none() {
                    return Ok(BrokeredCaptureResponse::Error(invalid_opaque_id_error()));
                }
                let infra = self.initialize_infra().await?;
                match broker_authorize_opaque_reference(
                    &self.config_dir,
                    &infra,
                    grants,
                    &opaque_id,
                )
                .await?
                {
                    Ok(_) => {
                        open_mnema_deep_link(&opaque_id)?;
                        Ok(BrokeredCaptureResponse::OpenInMnema(
                            BrokerOpenInMnemaResponse {
                                opened: true,
                                opaque_id,
                            },
                        ))
                    }
                    Err(error) => Ok(BrokeredCaptureResponse::Error(error)),
                }
            }
            BrokeredCaptureRequest::OpenCapturedUrl { .. } => {
                // The broker NEVER opens a raw captured URL for ANY caller. Doing
                // so would let any grant-holding external/CLI agent navigate the
                // user's authenticated browser to an in-scope captured URL the
                // moment a grant passes — a CSRF/replay primitive (ADR 0038: the
                // raw URL "materializes only on the user's click", it is not an
                // agent tool). Opening the raw `browser_url` is therefore
                // EXCLUSIVELY the LOCAL desktop path (the Tauri `open_captured_url`
                // command keyed off `frame_id`, behind a trusted-frontend user
                // click) which does NOT route through the broker. The request
                // variant is kept so the public protocol + all match arms in other
                // crates keep compiling; here it is always rejected.
                //
                // Closing this caller also seals the latent Windows
                // `cmd /C start` arg-injection sink in `open_external_url`: only
                // internal `mnema://` deep-link ids reach that opener now, never an
                // attacker-influenced captured URL.
                Ok(BrokeredCaptureResponse::Error(
                    BrokerErrorResponse::authorization_required(),
                ))
            }
        }
    }

    async fn initialize_infra(&self) -> Result<AppInfra> {
        let save_directory =
            default_save_directory_from_config(&self.config_dir)?.ok_or_else(|| {
                AppInfraError::BrokeredAccess(
                    "failed to resolve Mnema saveDirectory from recording settings".to_string(),
                )
            })?;
        // Brokered access is a read-only consumer that never spawns workers, so it
        // must not run startup maintenance (orphaned-job reconciliation) against a
        // database the live desktop app may be actively processing. See ADR 0020.
        AppInfra::initialize_read_only(save_directory).await
    }

    fn audit_result(
        &self,
        grants: &[BrokerGrant],
        identity: BrokerClientIdentity,
        command_type: &str,
        result_count: u32,
    ) -> Result<()> {
        if grants.is_empty() {
            return Ok(());
        }
        record_audit_event(
            &self.config_dir,
            identity,
            command_type,
            result_count,
            scope_class(grants),
            grants.first().map(|grant| grant.id.clone()),
        )
    }
}

pub fn execute_default_broker_request(
    tool_identity: impl Into<String>,
    request: BrokeredCaptureRequest,
) -> Result<BrokeredCaptureResponse> {
    let access = BrokeredCaptureAccess::from_default_app_config_dir()?;
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|error| AppInfraError::BrokeredAccess(error.to_string()))?;
    runtime.block_on(access.execute(tool_identity, request))
}

fn default_app_config_dir() -> Option<PathBuf> {
    default_app_config_dir_for_identifier(DEFAULT_APP_IDENTIFIER)
}

fn default_app_config_dir_for_identifier(identifier: &str) -> Option<PathBuf> {
    if let Ok(path) = std::env::var("MNEMA_APP_CONFIG_DIR") {
        return Some(PathBuf::from(path));
    }
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir().map(|home| {
            home.join("Library")
                .join("Application Support")
                .join(identifier)
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        dirs::config_dir().map(|dir| dir.join(identifier))
    }
}

fn default_save_directory_from_config(config_dir: &Path) -> Result<Option<PathBuf>> {
    if let Ok(path) = std::env::var("MNEMA_SAVE_DIRECTORY") {
        return Ok(Some(PathBuf::from(path)));
    }
    let settings_path = config_dir.join(RECORDING_SETTINGS_FILE_NAME);
    if !settings_path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(settings_path)?;
    let settings: RecordingSettingsFile = serde_json::from_str(&raw)?;
    Ok(Some(PathBuf::from(settings.save_directory)))
}

fn default_schema_version() -> u32 {
    1
}

fn default_grant_normalized_label() -> String {
    BrokerClientIdentity::default_cli().normalized_label
}

fn default_grant_identity_source() -> BrokerClientIdentitySource {
    BrokerClientIdentitySource::Defaulted
}

pub fn normalize_client_label(value: &str) -> Option<String> {
    let cleaned = value
        .chars()
        .map(|ch| if ch == '-' || ch == '_' { ' ' } else { ch })
        .filter(|ch| !ch.is_control())
        .collect::<String>();
    let normalized = cleaned
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn display_client_label(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_control())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_loaded_grant(grant: &mut BrokerGrant) {
    if grant.label.trim().is_empty() || grant.label == "Local agent" {
        grant.label = "mnema CLI".to_string();
        grant.normalized_label = BrokerClientIdentity::default_cli().normalized_label;
    } else {
        grant.label = display_client_label(&grant.label);
        if grant.normalized_label.trim().is_empty() {
            grant.normalized_label = normalize_client_label(&grant.label)
                .unwrap_or_else(|| BrokerClientIdentity::default_cli().normalized_label);
        }
    }
}

fn normalize_loaded_grant_file(mut grants: BrokerGrantFile) -> BrokerGrantFile {
    if grants.schema_version == 0 {
        grants.schema_version = 1;
    }
    for grant in &mut grants.grants {
        normalize_loaded_grant(grant);
    }
    grants
}

fn load_grants(config_dir: &Path) -> Result<BrokerGrantFile> {
    let path = config_dir.join(BROKER_GRANTS_FILE_NAME);
    if !path.exists() {
        return Ok(BrokerGrantFile {
            schema_version: 1,
            grants: Vec::new(),
        });
    }
    let raw = fs::read_to_string(path)?;
    Ok(normalize_loaded_grant_file(serde_json::from_str(&raw)?))
}

fn save_grants_locked(config_dir: &Path, grants: &BrokerGrantFile) -> Result<()> {
    let path = config_dir.join(BROKER_GRANTS_FILE_NAME);
    let temp_path = config_dir.join(format!("{BROKER_GRANTS_FILE_NAME}.tmp"));
    let raw = serde_json::to_string_pretty(grants)?;
    fs::write(&temp_path, raw)?;
    fs::rename(temp_path, path)?;
    Ok(())
}

fn with_grants_lock<T>(
    config_dir: &Path,
    f: impl FnOnce(&mut BrokerGrantFile) -> Result<T>,
) -> Result<T> {
    fs::create_dir_all(config_dir)?;
    let lock_path = config_dir.join(BROKER_GRANTS_LOCK_FILE_NAME);
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(lock_path)?;
    lock.lock_exclusive()?;
    let mut grants = load_grants(config_dir)?;
    let result = f(&mut grants);
    let unlock_result = lock.unlock();
    match (result, unlock_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error.into()),
    }
}

fn load_audit_events(config_dir: &Path) -> Result<BrokerAuditFile> {
    let path = config_dir.join(BROKER_AUDIT_FILE_NAME);
    if !path.exists() {
        return Ok(BrokerAuditFile::default());
    }
    let raw = fs::read_to_string(path)?;
    let mut audit: BrokerAuditFile = serde_json::from_str(&raw)?;
    if audit.schema_version == 0 {
        audit.schema_version = 1;
    }
    for event in &mut audit.events {
        if event.normalized_tool_identity.is_empty() {
            event.normalized_tool_identity = normalize_client_label(&event.tool_identity)
                .unwrap_or_else(|| BrokerClientIdentity::default_cli().normalized_label);
        }
    }
    Ok(audit)
}

fn record_audit_event(
    config_dir: &Path,
    identity: BrokerClientIdentity,
    command_type: impl Into<String>,
    result_count: u32,
    scope_class: impl Into<String>,
    grant_id: Option<String>,
) -> Result<()> {
    fs::create_dir_all(config_dir)?;
    let lock_path = config_dir.join(BROKER_AUDIT_LOCK_FILE_NAME);
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(lock_path)?;
    lock.lock_exclusive()?;

    let mut audit = load_audit_events(config_dir)?;
    audit.events.push(BrokerAuditEvent {
        tool_identity: identity.label,
        normalized_tool_identity: identity.normalized_label,
        identity_source: identity.source,
        command_type: command_type.into(),
        timestamp_unix_ms: now_unix_ms(),
        result_count,
        scope_class: scope_class.into(),
        grant_id,
        outcome: Some("success".to_string()),
    });
    if audit.events.len() > 500 {
        let drop_count = audit.events.len().saturating_sub(500);
        audit.events.drain(0..drop_count);
    }
    let path = config_dir.join(BROKER_AUDIT_FILE_NAME);
    let result = fs::write(path, serde_json::to_string_pretty(&audit)?);
    let unlock_result = lock.unlock();
    match (result, unlock_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), _) => Err(error.into()),
        (Ok(()), Err(error)) => Err(error.into()),
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn grant_is_active(grant: &BrokerGrant, now_unix_ms: u64) -> bool {
    !grant.revoked && grant.expires_at_unix_ms > now_unix_ms
}

fn active_grants(grants: &BrokerGrantFile, now_unix_ms: u64) -> Vec<BrokerGrant> {
    grants
        .grants
        .iter()
        .filter(|grant| grant_is_active(grant, now_unix_ms))
        .cloned()
        .collect()
}

fn active_grants_for_identity(
    grants: &BrokerGrantFile,
    identity: &BrokerClientIdentity,
    now_unix_ms: u64,
) -> Vec<BrokerGrant> {
    grants
        .grants
        .iter()
        .filter(|grant| {
            grant_is_active(grant, now_unix_ms)
                && grant
                    .normalized_label
                    .eq_ignore_ascii_case(&identity.normalized_label)
        })
        .cloned()
        .collect()
}

fn format_unix_ms(unix_ms: u64) -> String {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(unix_ms) * 1_000_000)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

pub fn format_broker_unix_ms(unix_ms: u64) -> String {
    format_unix_ms(unix_ms)
}

fn parse_rfc3339(value: &str) -> Result<OffsetDateTime> {
    OffsetDateTime::parse(value, &Rfc3339)
        .map_err(|error| AppInfraError::InvalidSearchRequest(error.to_string()))
}

/// Parse an optional `recall_context` RFC3339 UTC bound into unix-ms, IGNORING a
/// missing or unparseable value (returns `None`). Unlike `search`/`timeline`
/// (whose `scoped_date_range` hard-errors a bad bound), `recall_context`
/// degrades gracefully to its recency-bounded behavior rather than failing the
/// turn — so we reuse the same `parse_rfc3339` parser but discard the error.
fn recall_bound_to_unix_ms(value: Option<&str>) -> Option<i64> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    let parsed = parse_rfc3339(value).ok()?;
    // Floor to whole milliseconds; the overlap predicate compares against the
    // `*_ms` columns.
    Some((parsed.unix_timestamp_nanos() / 1_000_000) as i64)
}

fn effective_scope_start(grants: &[BrokerGrant], now_unix_ms: u64) -> Option<u64> {
    if grants
        .iter()
        .any(|grant| matches!(grant.scope, BrokerGrantScope::AllRetainedHistory))
    {
        return None;
    }
    grants
        .iter()
        .filter_map(|grant| match grant.scope {
            BrokerGrantScope::RecentDays { days } => Some(
                now_unix_ms.saturating_sub(u64::from(days).saturating_mul(24 * 60 * 60 * 1000)),
            ),
            BrokerGrantScope::AllRetainedHistory => None,
        })
        .min()
}

fn scope_class(grants: &[BrokerGrant]) -> String {
    if grants
        .iter()
        .any(|grant| matches!(grant.scope, BrokerGrantScope::AllRetainedHistory))
    {
        "all_retained_history".to_string()
    } else {
        "time_scoped".to_string()
    }
}

fn opaque_issuing_grant(grants: &[BrokerGrant]) -> Option<&BrokerGrant> {
    grants.iter().max_by_key(|grant| match grant.scope {
        BrokerGrantScope::AllRetainedHistory => u32::MAX,
        BrokerGrantScope::RecentDays { days } => days,
    })
}

fn scoped_date_range(
    grants: &[BrokerGrant],
    from: Option<String>,
    to: Option<String>,
) -> Result<Option<SearchDateRangeRefinement>> {
    let now = now_unix_ms();
    let scope_start = effective_scope_start(grants, now);
    if scope_start.is_none() && from.is_none() && to.is_none() {
        return Ok(None);
    }

    let default_start = scope_start.unwrap_or(0);
    let requested_start = match from {
        Some(value) => Some(parse_rfc3339(&value)?),
        None => None,
    };
    let requested_end = match to {
        Some(value) => Some(parse_rfc3339(&value)?),
        None => None,
    };
    let scope_start_dt =
        OffsetDateTime::from_unix_timestamp_nanos(i128::from(default_start) * 1_000_000)
            .unwrap_or(OffsetDateTime::UNIX_EPOCH);
    let now_dt = OffsetDateTime::from_unix_timestamp_nanos(i128::from(now) * 1_000_000)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH);
    let start_dt = requested_start
        .unwrap_or(scope_start_dt)
        .max(scope_start_dt);
    let end_dt = requested_end.unwrap_or(now_dt).min(now_dt);
    if end_dt < start_dt {
        return Err(AppInfraError::InvalidSearchRequest(
            "requested broker time range is outside the grant scope".to_string(),
        ));
    }
    Ok(Some(SearchDateRangeRefinement {
        start_at: start_dt
            .format(&Rfc3339)
            .unwrap_or_else(|_| format_unix_ms(default_start)),
        end_at: end_dt
            .format(&Rfc3339)
            .unwrap_or_else(|_| format_unix_ms(now)),
        origin: Some(SearchDateRangeOrigin::VisibleTimeline),
    }))
}

fn broker_search_refinements(
    grants: &[BrokerGrant],
    from: Option<String>,
    to: Option<String>,
    app: Option<String>,
    window_title: Option<String>,
    url: Option<String>,
    url_regex: Option<String>,
    speaker: Option<SearchSpeakerRefinement>,
) -> Result<SearchCaptureRefinements> {
    Ok(SearchCaptureRefinements {
        date_range: scoped_date_range(grants, from, to)?,
        apps: broker_app_refinement(app)?.into_iter().collect(),
        window_title: broker_optional_filter(window_title, "windowTitle")?,
        url: broker_optional_filter(url, "url")?,
        url_regex: broker_url_regex_filter(url_regex)?,
        audio_sources: Vec::new(),
        screen_source: false,
        speaker,
    })
}

/// "What did Priya say in Zoom" reads answerable and is not: the app, window
/// title, and url live on captured FRAMES, the voice lives on AUDIO, and no row
/// joins the two. Answering it with an empty page would be reported to the user as
/// "Priya said nothing in Zoom" — a confident lie — so the combination is refused
/// outright, as a request error rather than a result.
fn speaker_screen_filter_conflict() -> AppInfraError {
    AppInfraError::InvalidSearchRequest(
        "speaker cannot be combined with app, windowTitle, url, or urlRegex: a speaker filter \
         matches recorded audio, and audio carries no app, window title, or url to match against \
         — ask for the speaker first, then search the screen filters over the times it returns"
            .to_string(),
    )
}

fn broker_app_refinement(app: Option<String>) -> Result<Option<SearchAppRefinement>> {
    let Some(value) = broker_optional_filter(app, "app")? else {
        return Ok(None);
    };
    Ok(Some(SearchAppRefinement {
        kind: SearchAppRefinementKind::Any,
        display_name: value.clone(),
        value,
    }))
}

/// `urlRegex`, rejected as a request error when it is not a valid pattern. Search
/// also validates during refinement normalization, but the timeline builds its SQL
/// directly — so a client typo surfaces here as a clear error on BOTH paths rather
/// than as an opaque `REGEXP` failure from SQLite.
fn broker_url_regex_filter(value: Option<String>) -> Result<Option<String>> {
    let Some(value) = broker_optional_filter(value, "urlRegex")? else {
        return Ok(None);
    };
    regex::Regex::new(&value).map_err(|error| {
        AppInfraError::InvalidSearchRequest(format!(
            "urlRegex is not a valid regular expression: {error}"
        ))
    })?;
    Ok(Some(value))
}

fn broker_optional_filter(value: Option<String>, field_name: &str) -> Result<Option<String>> {
    value
        .map(|value| {
            let value = value.trim().to_string();
            if value.is_empty() {
                Err(AppInfraError::InvalidSearchRequest(format!(
                    "{field_name} must be non-empty"
                )))
            } else {
                Ok(value)
            }
        })
        .transpose()
}

fn push_broker_timeline_context_filters(
    query: &mut QueryBuilder<'_, Sqlite>,
    app: Option<&SearchAppRefinement>,
    window_title: Option<&str>,
    url: Option<&str>,
    url_regex: Option<&str>,
) {
    if let Some(app) = app {
        query.push(" AND (LOWER(TRIM(COALESCE(app_bundle_id, ''))) = LOWER(");
        query.push_bind(app.value.clone());
        query.push(") OR app_name_search_key = ");
        query.push_bind(app.value.to_lowercase());
        query.push(")");
    }
    if let Some(window_title) = window_title {
        query.push(" AND LOWER(COALESCE(window_title, '')) LIKE LOWER(");
        query.push_bind(sqlite_contains_like_pattern(window_title));
        query.push(") ESCAPE '\\'");
    }
    if let Some(url) = url {
        query.push(" AND LOWER(COALESCE(url, '')) LIKE LOWER(");
        query.push_bind(sqlite_contains_like_pattern(url));
        query.push(") ESCAPE '\\'");
    }
    if let Some(url_regex) = url_regex {
        // `X REGEXP Y` invokes `regexp(Y, X)` — pattern first, matching sqlx's
        // registered implementation, so the infix form is correct as written.
        query.push(" AND COALESCE(url, '') REGEXP ");
        query.push_bind(url_regex.to_string());
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

fn timestamp_within_scope(grants: &[BrokerGrant], timestamp: &str) -> Result<bool> {
    let Some(scope_start) = effective_scope_start(grants, now_unix_ms()) else {
        return Ok(true);
    };
    let timestamp = parse_rfc3339(timestamp)?;
    let start = OffsetDateTime::from_unix_timestamp_nanos(i128::from(scope_start) * 1_000_000)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH);
    Ok(timestamp >= start)
}

fn range_overlaps_scope(grants: &[BrokerGrant], started_at: &str, ended_at: &str) -> Result<bool> {
    let Some(scope_start) = effective_scope_start(grants, now_unix_ms()) else {
        return Ok(true);
    };
    let ended_at = parse_rfc3339(ended_at)?;
    let scope_start =
        OffsetDateTime::from_unix_timestamp_nanos(i128::from(scope_start) * 1_000_000)
            .unwrap_or(OffsetDateTime::UNIX_EPOCH);
    if ended_at < scope_start {
        return Ok(false);
    }
    parse_rfc3339(started_at)?;
    Ok(true)
}

fn auth_status_for_config(
    config_dir: &Path,
    identity: Option<&BrokerClientIdentity>,
) -> Result<BrokerAuthStatus> {
    let grants = load_grants(config_dir)?;
    let active_count = identity.map_or_else(
        || active_grants(&grants, now_unix_ms()).len(),
        |identity| active_grants_for_identity(&grants, identity, now_unix_ms()).len(),
    );
    if active_count == 0 {
        Ok(BrokerAuthStatus::authorization_required())
    } else {
        Ok(BrokerAuthStatus::authorized(active_count))
    }
}

fn create_grant(
    config_dir: &Path,
    label: impl Into<String>,
    duration_hours: u64,
    scope: BrokerGrantScope,
) -> Result<BrokerGrant> {
    let identity = BrokerClientIdentity::new(label.into(), BrokerClientIdentitySource::Explicit)?;
    create_grant_for_identity(config_dir, identity, duration_hours, scope)
}

fn create_grant_for_identity(
    config_dir: &Path,
    identity: BrokerClientIdentity,
    duration_hours: u64,
    scope: BrokerGrantScope,
) -> Result<BrokerGrant> {
    with_grants_lock(config_dir, |grants| {
        let now = now_unix_ms();
        let grant = BrokerGrant {
            id: format!("{now:x}-{:x}", grants.grants.len()),
            label: identity.label,
            normalized_label: identity.normalized_label,
            identity_source: identity.source,
            created_at_unix_ms: now,
            expires_at_unix_ms: now.saturating_add(duration_hours.saturating_mul(60 * 60 * 1000)),
            scope,
            revoked: false,
            revoked_at_unix_ms: None,
        };
        grants.grants.push(grant.clone());
        save_grants_locked(config_dir, grants)?;
        Ok(grant)
    })
}

fn create_grant_from_request(
    config_dir: &Path,
    request: BrokerGrantCreateRequest,
) -> Result<BrokerGrant> {
    let scope = if request.all_retained_history.unwrap_or(false) {
        BrokerGrantScope::AllRetainedHistory
    } else {
        BrokerGrantScope::RecentDays { days: 1 }
    };
    create_grant(
        config_dir,
        request.label.unwrap_or_else(|| "Local agent".to_string()),
        request.duration_hours.unwrap_or(24).clamp(1, 24 * 30),
        scope,
    )
}

fn ask_ai_all_retained_grant(identity: &BrokerClientIdentity) -> BrokerGrant {
    BrokerGrant {
        id: ASK_AI_BROKER_GRANT_ID.to_string(),
        label: identity.label.clone(),
        normalized_label: identity.normalized_label.clone(),
        identity_source: identity.source.clone(),
        created_at_unix_ms: 0,
        expires_at_unix_ms: u64::MAX,
        scope: BrokerGrantScope::AllRetainedHistory,
        revoked: false,
        revoked_at_unix_ms: None,
    }
}

fn revoke_grant(config_dir: &Path, grant_id: &str) -> Result<bool> {
    with_grants_lock(config_dir, |grants| {
        let mut changed = false;
        let now = now_unix_ms();
        for grant in &mut grants.grants {
            if grant.id == grant_id && !grant.revoked {
                grant.revoked = true;
                grant.revoked_at_unix_ms = Some(now);
                changed = true;
            }
        }
        if changed {
            save_grants_locked(config_dir, grants)?;
        }
        Ok(changed)
    })
}

fn revoke_grants_for_client(config_dir: &Path, client_label: &str) -> Result<u32> {
    let Some(normalized_label) = normalize_client_label(client_label) else {
        return Ok(0);
    };
    with_grants_lock(config_dir, |grants| {
        let mut changed = 0u32;
        let now = now_unix_ms();
        for grant in &mut grants.grants {
            if !grant.revoked
                && grant
                    .normalized_label
                    .eq_ignore_ascii_case(&normalized_label)
            {
                grant.revoked = true;
                grant.revoked_at_unix_ms = Some(now);
                changed = changed.saturating_add(1);
            }
        }
        if changed > 0 {
            save_grants_locked(config_dir, grants)?;
        }
        Ok(changed)
    })
}

async fn broker_search(
    config_dir: &Path,
    infra: &AppInfra,
    grants: &[BrokerGrant],
    request: BrokerSearchRequest,
) -> Result<std::result::Result<BrokerSearchResponse, BrokerErrorResponse>> {
    if grants.is_empty() {
        return Ok(Err(BrokerErrorResponse::authorization_required()));
    }
    // Clamped from BELOW too: a zero-sized page can never consume an anchor, so
    // the cursor would advance by nothing and `more` would be false — the walk
    // would report itself exhausted (`next_cursor: None`) on its first page while
    // every match is still unseen. `limit` reaches here straight off the wire
    // (the MCP `mnema_search` tool passes it through unvalidated), so the floor
    // belongs here rather than in every caller.
    let limit = request
        .limit
        .unwrap_or(DEFAULT_SEARCH_LIMIT)
        .clamp(1, MAX_SEARCH_LIMIT);
    let cursor = request
        .cursor
        .as_deref()
        .map(BrokerSearchCursor::decode)
        .transpose()?;
    let speaker = match broker_speaker_refinement(config_dir, grants, request.speaker)? {
        Ok(speaker) => speaker,
        Err(error) => return Ok(Err(error)),
    };
    if speaker.is_some()
        && (request.app.is_some()
            || request.window_title.is_some()
            || request.url.is_some()
            || request.url_regex.is_some())
    {
        return Err(speaker_screen_filter_conflict());
    }
    let refinements = broker_search_refinements(
        grants,
        request.from,
        request.to,
        request.app,
        request.window_title,
        request.url,
        request.url_regex,
        speaker,
    )?;
    let response = infra
        .search_capture(SearchCaptureRequest {
            query: request.query,
            frame_limit: Some(limit),
            frame_offset: Some(cursor.map(|cursor| cursor.frame_offset).unwrap_or(0)),
            audio_limit: Some(limit),
            audio_offset: Some(cursor.map(|cursor| cursor.audio_offset).unwrap_or(0)),
            snapshot_document_id: cursor.map(|cursor| cursor.snapshot_document_id),
            refinements: Some(refinements),
            // Brokered access is keyword-only: the broker never runs the local
            // **Semantic Search Model**, so it passes no query vector.
            query_embedding: None,
        })
        .await?;
    let opaque_secret = load_or_create_opaque_secret(config_dir)?;
    Ok(Ok(map_search_response(
        response,
        limit,
        cursor,
        opaque_issuing_grant(grants).map(|grant| grant.id.as_str()),
        &opaque_secret,
    )))
}

async fn broker_show_text(
    config_dir: &Path,
    infra: &AppInfra,
    grants: &[BrokerGrant],
    opaque_id: &str,
) -> Result<std::result::Result<BrokerShowTextResponse, BrokerErrorResponse>> {
    if grants.is_empty() {
        return Ok(Err(BrokerErrorResponse::authorization_required()));
    }
    let reference =
        match broker_authorize_opaque_reference(config_dir, infra, grants, opaque_id).await? {
            Ok(reference) => reference,
            Err(error) => return Ok(Err(error)),
        };
    let subject = match reference.kind.as_str() {
        "frame" => ProcessingSubject::frame(reference.frame_id.expect("frame reference has id")),
        "audio" => ProcessingSubject::audio_segment(
            reference.audio_segment_id.expect("audio reference has id"),
        ),
        _ => return Ok(Err(invalid_opaque_id_error())),
    };
    let result = infra
        .list_processing_results_for_subject(&subject)
        .await?
        .into_iter()
        .filter(|result| {
            result
                .result_text
                .as_deref()
                .is_some_and(|text| !text.trim().is_empty())
        })
        .max_by_key(|result| result.id);
    let text = if let Some(result) = result {
        result.result_text.unwrap_or_default()
    } else if reference.kind == "frame" {
        broker_equivalent_reuse_text_for_frame(
            infra,
            grants,
            reference.frame_id.expect("frame reference has id"),
        )
        .await?
        .unwrap_or_default()
    } else {
        String::new()
    };
    if text.trim().is_empty() {
        return Ok(Err(BrokerErrorResponse {
            error: BrokerAuthStatusKind::AuthorizationRequired,
            message: "result is unavailable or outside the grant scope".to_string(),
        }));
    };
    let (kind, speakers, turns) = if subject.subject_type == AUDIO_SEGMENT_SUBJECT_TYPE {
        // Kind comes from the segment, not from the opaque id's `"audio"` prefix:
        // reporting the prefix here would leave a third audio vocabulary behind
        // the one `search` and `timeline` publish.
        let Some(segment) = infra.get_audio_segment(subject.subject_id).await? else {
            return Ok(Err(outside_scope_error()));
        };
        // Handles are minted under the grant that authorized THIS read, so they
        // never outlive the scope the caller already holds.
        let secret = load_or_create_opaque_secret(config_dir)?;
        let (speakers, turns) = broker_speakers_for_audio(
            infra,
            subject.subject_id,
            reference.grant_id.as_deref(),
            &secret,
        )
        .await?;
        (
            broker_audio_kind(&segment.source_kind).to_string(),
            speakers,
            turns,
        )
    } else {
        (reference.kind, Vec::new(), Vec::new())
    };
    Ok(Ok(BrokerShowTextResponse {
        opaque_id: opaque_id.to_string(),
        kind,
        text,
        speakers,
        turns,
    }))
}

/// One audio vocabulary across `search`, `timeline`, and `show-text`: an agent that
/// cannot tell the user's own voice from playback will attribute a podcast to them.
fn broker_audio_kind(source_kind: &AudioSegmentSourceKind) -> &'static str {
    match source_kind {
        AudioSegmentSourceKind::Microphone => "audio_microphone",
        AudioSegmentSourceKind::SystemAudio => "audio_system",
    }
}

async fn broker_equivalent_reuse_text_for_frame(
    infra: &AppInfra,
    grants: &[BrokerGrant],
    frame_id: i64,
) -> Result<Option<String>> {
    let Some(reuse) = infra
        .search
        .equivalent_reuse_text_for_frame(frame_id)
        .await?
    else {
        return Ok(None);
    };
    let source_in_scope = match reuse.source_subject_type.as_str() {
        FRAME_SUBJECT_TYPE => {
            let Some(frame) = infra.get_frame(reuse.source_subject_id).await? else {
                return Ok(None);
            };
            timestamp_within_scope(grants, &frame.captured_at)?
        }
        AUDIO_SEGMENT_SUBJECT_TYPE => {
            let Some(audio) = infra.get_audio_segment(reuse.source_subject_id).await? else {
                return Ok(None);
            };
            range_overlaps_scope(grants, &audio.started_at, &audio.ended_at)?
        }
        _ => false,
    };
    if source_in_scope {
        Ok(Some(reuse.result_text))
    } else {
        Ok(None)
    }
}

pub async fn authorize_active_opaque_capture_reference(
    config_dir: &Path,
    opaque_id: &str,
) -> Result<Option<BrokerOpaqueCaptureReference>> {
    let grants = load_grants(config_dir)?;
    let grants = active_grants(&grants, now_unix_ms());
    if grants.is_empty() {
        return Ok(None);
    }
    let Some(save_directory) = default_save_directory_from_config(config_dir)? else {
        return Ok(None);
    };
    // Read-only authorization: skip startup maintenance so this never reconciles
    // the live desktop app's running jobs (see ADR 0020 / `initialize_read_only`).
    let infra = AppInfra::initialize_read_only(save_directory).await?;
    match broker_authorize_opaque_reference(config_dir, &infra, &grants, opaque_id).await? {
        Ok(reference) => Ok(Some(reference)),
        Err(_) => Ok(None),
    }
}

async fn broker_authorize_opaque_reference(
    config_dir: &Path,
    infra: &AppInfra,
    grants: &[BrokerGrant],
    opaque_id: &str,
) -> Result<std::result::Result<BrokerOpaqueCaptureReference, BrokerErrorResponse>> {
    if grants.is_empty() {
        return Ok(Err(BrokerErrorResponse::authorization_required()));
    }
    let secret = load_or_create_opaque_secret(config_dir)?;
    let Some(reference) = decode_signed_opaque_id(opaque_id, &secret) else {
        return Ok(Err(invalid_opaque_id_error()));
    };
    let Some(grant_id) = reference.grant_id.as_deref() else {
        return Ok(Err(invalid_opaque_id_error()));
    };
    let scoped_grants = grants
        .iter()
        .filter(|grant| grant.id == grant_id)
        .cloned()
        .collect::<Vec<_>>();
    if scoped_grants.is_empty() {
        return Ok(Err(outside_scope_error()));
    }
    let in_scope = match reference.kind.as_str() {
        "frame" => {
            let Some(frame) = infra
                .get_frame(reference.frame_id.expect("frame reference has id"))
                .await?
            else {
                return Ok(Err(outside_scope_error()));
            };
            timestamp_within_scope(&scoped_grants, &frame.captured_at)?
        }
        "audio" => {
            let Some(audio) = infra
                .get_audio_segment(reference.audio_segment_id.expect("audio reference has id"))
                .await?
            else {
                return Ok(Err(outside_scope_error()));
            };
            range_overlaps_scope(&scoped_grants, &audio.started_at, &audio.ended_at)?
        }
        _ => return Ok(Err(invalid_opaque_id_error())),
    };
    if !in_scope {
        return Ok(Err(outside_scope_error()));
    }
    Ok(Ok(reference))
}

async fn broker_timeline(
    config_dir: &Path,
    infra: &AppInfra,
    grants: &[BrokerGrant],
    request: BrokerTimelineRequest,
) -> Result<std::result::Result<BrokerTimelineResponse, BrokerErrorResponse>> {
    if grants.is_empty() {
        return Ok(Err(BrokerErrorResponse::authorization_required()));
    }
    let limit = request
        .limit
        .unwrap_or(DEFAULT_SEARCH_LIMIT)
        .min(MAX_SEARCH_LIMIT);
    let range = scoped_date_range(grants, Some(request.from), Some(request.to))?
        .expect("timeline always supplies a scoped date range");
    let app = broker_app_refinement(request.app)?;
    let window_title = broker_optional_filter(request.window_title, "windowTitle")?;
    let url = broker_optional_filter(request.url, "url")?;
    let url_regex = broker_url_regex_filter(request.url_regex)?;
    let speaker = match broker_speaker_refinement(config_dir, grants, request.speaker)? {
        Ok(speaker) => speaker,
        Err(error) => return Ok(Err(error)),
    };
    if speaker.is_some()
        && (app.is_some() || window_title.is_some() || url.is_some() || url_regex.is_some())
    {
        return Err(speaker_screen_filter_conflict());
    }
    let opaque_secret = load_or_create_opaque_secret(config_dir)?;
    let opaque_grant_id = opaque_issuing_grant(grants).map(|grant| grant.id.as_str());
    if app.is_some() || window_title.is_some() || url.is_some() || url_regex.is_some() {
        // Any context filter narrows to captured frames: audio segments carry no
        // app, window title, or url to match against.
        let intervals = broker_frame_timeline(
            infra,
            &range,
            app.as_ref(),
            window_title.as_deref(),
            url.as_deref(),
            url_regex.as_deref(),
            limit,
            opaque_grant_id,
            &opaque_secret,
        )
        .await?;
        return Ok(Ok(BrokerTimelineResponse { intervals, limit }));
    }
    // The mirror of the context-filter branch above: a speaker filter narrows to
    // audio, because a captured frame carries no voice to match against.
    let speaker_matched = match speaker.as_ref() {
        Some(speaker) => Some(speaker_matched_audio_segment_ids(infra, speaker, &range).await?),
        None => None,
    };
    let mut intervals = if speaker.is_some() {
        Vec::new()
    } else {
        broker_frame_timeline(
            infra,
            &range,
            None,
            None,
            None,
            None,
            limit,
            opaque_grant_id,
            &opaque_secret,
        )
        .await?
    };
    for audio in infra
        .list_audio_segments_overlapping_range(&range.start_at, &range.end_at, None, None)
        .await?
        .into_iter()
        // Filtered BEFORE the cap, or a page of unmatched segments would hide
        // every one the speaker was actually heard in.
        .filter(|audio| {
            speaker_matched
                .as_ref()
                .is_none_or(|matched| matched.contains(&audio.id))
        })
        .take(limit as usize)
    {
        intervals.push(BrokerTimelineInterval {
            kind: broker_audio_kind(&audio.source_kind).to_string(),
            started_at: audio.started_at,
            ended_at: Some(audio.ended_at),
            opaque_id: Some(encode_signed_opaque_id(
                "audio",
                audio.id,
                opaque_grant_id,
                &opaque_secret,
            )),
            context: None,
        });
    }
    intervals.sort_by(|left, right| {
        right
            .started_at
            .cmp(&left.started_at)
            .then_with(|| right.kind.cmp(&left.kind))
    });
    intervals.truncate(limit as usize);
    Ok(Ok(BrokerTimelineResponse { intervals, limit }))
}

async fn broker_frame_timeline(
    infra: &AppInfra,
    range: &SearchDateRangeRefinement,
    app: Option<&SearchAppRefinement>,
    window_title: Option<&str>,
    url: Option<&str>,
    url_regex: Option<&str>,
    limit: u32,
    opaque_grant_id: Option<&str>,
    opaque_secret: &[u8],
) -> Result<Vec<BrokerTimelineInterval>> {
    // `representative_frame_id` must be the frame_id of the SAME `MAX(id)` row that
    // drives the interval's ordering (its landing frame), DETERMINISTICALLY. A bare
    // `frame_id` selected alongside the aggregates is NOT safe: SQLite only
    // guarantees a bare column tracks the min/max row when there is exactly one
    // min/max in the group; with two-plus aggregates (here MIN + two MAX) the row a
    // bare column is taken from is documented-arbitrary (sqlite.org "Bare columns in
    // an aggregate query"). So we compute the grouping + `MAX(id) AS sort_id` in a
    // CTE, then JOIN back to `search_documents` on the primary key (`s.id = g.sort_id`)
    // to read THAT exact row's `frame_id`. Kept INTERNAL (never put on the wire
    // struct); only its guarded url crosses the broker boundary read-time.
    let mut query = QueryBuilder::<Sqlite>::new(
        "WITH grouped AS ( \
           SELECT group_key, app_bundle_id, app_name, window_title, \
                  MIN(absolute_start_at) AS started_at, MAX(absolute_end_at) AS ended_at, \
                  MAX(id) AS sort_id \
           FROM search_documents \
           WHERE anchor_type = 'frame' \
             AND julianday(absolute_end_at) >= julianday(",
    );
    query.push_bind(range.start_at.clone());
    query.push(") AND julianday(absolute_start_at) <= julianday(");
    query.push_bind(range.end_at.clone());
    query.push(")");
    push_broker_timeline_context_filters(&mut query, app, window_title, url, url_regex);
    query.push(
        "   GROUP BY group_key, app_bundle_id, app_name, window_title \
         ) \
         SELECT g.group_key, g.app_bundle_id, g.app_name, g.window_title, \
                g.started_at, g.ended_at, g.sort_id, \
                s.frame_id AS representative_frame_id \
         FROM grouped g \
         JOIN search_documents s ON s.id = g.sort_id \
         ORDER BY g.started_at DESC, g.sort_id DESC LIMIT ",
    );
    query.push_bind(limit as i64);

    let rows = query.build().fetch_all(infra.read_pool()).await?;

    // Read-time URL guard: load the representative (landing) frames' metadata
    // snapshots in a SINGLE batched query (keyed by frame id), NOT one sequential
    // `get_frame` round-trip per interval. With `limit` clamped to
    // MAX_SEARCH_LIMIT=100 the old per-interval loop was an N+1 of up to 100
    // sequential DB round-trips on this interactive broker tool path; the IN-query
    // collapses them to one. The raw frame id never crosses to the wire, and only
    // guarded http(s) host+path survives — everything else guards to `None`.
    let representative_frame_ids: Vec<i64> = rows
        .iter()
        .filter_map(|row| row.get::<Option<i64>, _>("representative_frame_id"))
        .collect();
    let snapshots = infra
        .get_frame_metadata_snapshots(&representative_frame_ids)
        .await?;

    let mut intervals = Vec::with_capacity(rows.len());
    for row in rows {
        let app_bundle_id: Option<String> = row.get("app_bundle_id");
        let app_name: Option<String> = row.get("app_name");
        let window_title: Option<String> = row.get("window_title");
        let representative_frame_id: Option<i64> = row.get("representative_frame_id");
        let url = representative_frame_id
            .and_then(|frame_id| snapshots.get(&frame_id))
            .and_then(|snapshot| snapshot.browser_url.as_deref())
            .and_then(url_guard::guard_url);
        intervals.push(BrokerTimelineInterval {
            kind: "frame".to_string(),
            started_at: row.get("started_at"),
            ended_at: Some(row.get("ended_at")),
            opaque_id: representative_frame_id.map(|frame_id| {
                encode_signed_opaque_id("frame", frame_id, opaque_grant_id, opaque_secret)
            }),
            context: broker_search_result_context(app_bundle_id, app_name, window_title, url),
        });
    }
    Ok(intervals)
}

const DEFAULT_RECALL_CONTEXT_LIMIT: u32 = 8;
const MAX_RECALL_CONTEXT_LIMIT: u32 = 20;

/// When an explicit `from`/`to` time range is supplied, the question is episodic
/// ("what did I do in window X"), so standing-belief Conclusions are de-emphasized
/// to keep them from competing with the activity timeline that answers the
/// question. Cap recalled Conclusions this low in that case (Activities still get
/// the full `limit`).
const RANGE_PRESENT_CONCLUSION_LIMIT: usize = 3;

/// `recall_context`: return ONLY the User-Context Conclusions/Activities relevant
/// to the question, redacted, capped, and never sensitive. This deliberately never
/// returns the whole dossier — both lists are relevance-filtered against the
/// question and hard-capped at [`MAX_RECALL_CONTEXT_LIMIT`].
///
/// Relevance is scored in-memory by whole-word (#1), lightly-stemmed (#3),
/// rare-token-weighted (#2 IDF) overlap of the query tokens against each item's
/// text — a token only counts when it appears as a full (stemmed) word, and rare
/// tokens outweigh common ones. Activity candidates are pulled with a DB-side
/// keyword pre-filter (#5) so an older-but-relevant Activity is reachable, not
/// just the most-recent window.
///
/// Sensitive Conclusions AND sensitive Activities are dropped via the same hard
/// guardrail (`crate::user_context::guardrail::is_sensitive`, #4) used at
/// derivation time, and only Visible (not Faded, not Dismissed) Conclusions are
/// eligible. No ids or evidence refs cross the boundary.
///
/// For Conclusions the guardrail re-filter is belt-and-suspenders (derivation
/// never persists a sensitive Conclusion). For **Activities it is LOAD-BEARING**:
/// an Activity's `title`/`summary` is persisted *unfiltered*, so the broker-side
/// `is_sensitive` filter in `select_relevant_activities` is the only thing
/// stopping a sensitive Activity from reaching a cloud engine. Do not remove it as
/// "redundant" — see `guardrail.rs` and the `sensitive_activity_never_*`
/// regression test below.
///
/// When an explicit time range is present (either `from` OR `to` parsed to a real
/// bound), the question is episodic, so Conclusions are de-emphasized so they don't
/// crowd out the activity timeline: they're capped at
/// [`RANGE_PRESENT_CONCLUSION_LIMIT`] instead of the full `limit`, AND the
/// no-token confidence fallback is suppressed (a confidence dump of unrelated
/// standing beliefs is pure noise in an episodic answer — see
/// `select_relevant_conclusions`). Activities are unaffected: always the full
/// `limit`, date-scoped by the same bounds.
async fn broker_recall_context(
    infra: &AppInfra,
    grants: &[BrokerGrant],
    request: BrokerRecallContextRequest,
) -> Result<std::result::Result<BrokerRecallContextResponse, BrokerErrorResponse>> {
    if grants.is_empty() {
        return Ok(Err(BrokerErrorResponse::authorization_required()));
    }
    let limit = request
        .limit
        .unwrap_or(DEFAULT_RECALL_CONTEXT_LIMIT)
        .min(MAX_RECALL_CONTEXT_LIMIT)
        .max(1) as usize;

    let store = infra.user_context();
    // Non-faded conclusions only; `list_conclusions(false)` already excludes faded.
    let conclusions = store.list_conclusions(false).await?;

    let tokens = recall_query_tokens(&request.query);

    // Optional `from`/`to` UTC bounds scope the ACTIVITIES by date (Conclusions
    // are standing beliefs and are never scoped). A bad/unparseable bound is
    // IGNORED gracefully (that bound becomes `None`) rather than erroring the
    // turn — `recall_context` favors degrading to its recency-bounded behavior
    // over failing, unlike `search`/`timeline` whose `scoped_date_range` parse
    // hard-errors. We mirror those handlers' `parse_rfc3339` parser but discard
    // the error via `.ok()`.
    let from_ms = recall_bound_to_unix_ms(request.from.as_deref());
    let to_ms = recall_bound_to_unix_ms(request.to.as_deref());

    // A time range is "present" when EITHER bound parsed to a real value — a bad
    // bound that lenient-parsed to `None` does not count. A present range means the
    // turn is episodic, so we de-emphasize the standing-belief Conclusions: cap them
    // low and disable the no-token confidence fallback (see below). Activities are
    // untouched — they ALWAYS get the full `limit`.
    let range_present = from_ms.is_some() || to_ms.is_some();
    let conclusion_limit = if range_present {
        limit.min(RANGE_PRESENT_CONCLUSION_LIMIT)
    } else {
        limit
    };
    let allow_confidence_fallback = !range_present;

    // #5: relevance-bounded (not recency-bounded) Activity candidates. We push the
    // query tokens into a DB-side `LIKE` pre-filter (`search_recent_activities`) so
    // an older-but-relevant Activity is a candidate even when the recent window is
    // saturated by recent-but-irrelevant Activities — the old
    // `list_recent_activities(MAX*4)` window could never reach it. The DB pass is a
    // cheap recall-favoring superset (raw substring `LIKE` on the un-stemmed
    // tokens); the in-memory scorer below does the precise whole-word + stemmed +
    // IDF ranking and the hard `limit` cap. We still pull a generous candidate cap
    // so the in-memory scorer ranks across a wide set rather than a thin slice.
    //
    // When there are no usable query tokens, `search_recent_activities` degrades to
    // the most-recent window — the same fallback set the old path used.
    const ACTIVITY_CANDIDATE_CAP: i64 = 200;
    let activities = store
        .search_recent_activities(&tokens, from_ms, to_ms, ACTIVITY_CANDIDATE_CAP)
        .await?;

    let conclusions = select_relevant_conclusions(
        &conclusions,
        &tokens,
        conclusion_limit,
        allow_confidence_fallback,
    );
    let activities = select_relevant_activities(&activities, &tokens, limit);

    Ok(Ok(BrokerRecallContextResponse {
        conclusions,
        activities,
    }))
}

// Lexical relevance primitives now live in `crate::lexical` (shared with the
// User Context distillation candidate selector). Imported under their historical
// `recall_*` names so the call sites + tests below read unchanged; the test-only
// `stem` / `idf_weight` aliases are imported in the `tests` module.
use crate::lexical::{
    doc_words as recall_doc_words, document_frequencies as recall_document_frequencies,
    overlap_score as recall_overlap_score, query_tokens as recall_query_tokens,
};

/// Convert a snake_case-serde enum value to its wire string (e.g. `Creating` ->
/// `"creating"`), so recalled activities carry the same category/focus labels the
/// rest of the stack uses.
fn snake_case_enum_string<T: Serialize>(value: &T) -> Option<String> {
    match serde_json::to_value(value).ok()? {
        serde_json::Value::String(s) => Some(s),
        _ => None,
    }
}

/// Pure relevance + sensitive-filter + cap for Conclusions. Drops sensitive and
/// non-Visible Conclusions, then scores the rest by whole-word (#1), stemmed (#3),
/// rare-token-weighted (#2 IDF) overlap of the query against subject+statement.
/// Keeps score>0 (sorted by score desc, confidence desc) and truncates to `limit`
/// so the whole dossier can never be returned. IDF document-frequency is computed
/// over the non-sensitive, Visible candidate set only.
///
/// `allow_confidence_fallback` gates the no-token path: when the query has no
/// usable tokens and the flag is `true`, fall back to top-by-confidence (the
/// default `recall_context` behavior). When it is `false` (an episodic, time-ranged
/// turn), suppress that fallback and return an empty list instead — dumping
/// unrelated standing beliefs into an episodic answer is pure noise. With usable
/// tokens the flag has no effect: the normal score>0 path runs either way.
fn select_relevant_conclusions(
    conclusions: &[capture_types::Conclusion],
    tokens: &[String],
    limit: usize,
    allow_confidence_fallback: bool,
) -> Vec<BrokerRecalledConclusion> {
    // Eligible candidates first (Visible + non-sensitive), so the IDF corpus and
    // the scoring set are the same population.
    let candidates: Vec<&capture_types::Conclusion> = conclusions
        .iter()
        .filter(|c| matches!(c.status, capture_types::ConclusionStatus::Visible))
        .filter(|c| !crate::user_context::guardrail::is_sensitive(&c.subject, &c.statement))
        .collect();

    let docs: Vec<std::collections::HashSet<String>> = candidates
        .iter()
        .map(|c| recall_doc_words(&format!("{} {}", c.subject, c.statement)))
        .collect();
    let df = recall_document_frequencies(tokens, &docs);
    let n = candidates.len();

    let mut scored: Vec<(f64, &capture_types::Conclusion)> = candidates
        .iter()
        .zip(docs.iter())
        .map(|(c, words)| (recall_overlap_score(tokens, words, &df, n), *c))
        .collect();

    if tokens.is_empty() {
        // No usable query tokens. The confidence fallback is only safe for the
        // default (non-episodic) path: when it's disabled (a time-ranged turn),
        // return NOTHING rather than dumping unrelated standing beliefs into an
        // episodic answer.
        if !allow_confidence_fallback {
            return Vec::new();
        }
        // Fall back to top-by-confidence, STILL capped.
        scored.sort_by(|a, b| {
            b.1.confidence
                .partial_cmp(&a.1.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    } else {
        scored.retain(|(score, _)| *score > 0.0);
        scored.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    b.1.confidence
                        .partial_cmp(&a.1.confidence)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });
    }

    scored
        .into_iter()
        .take(limit)
        .map(|(_, c)| BrokerRecalledConclusion {
            subject: c.subject.clone(),
            statement: c.statement.clone(),
            confidence: c.confidence,
            status: snake_case_enum_string(&c.status).unwrap_or_else(|| "visible".to_string()),
        })
        .collect()
}

/// Pure relevance + sensitive-filter + cap for Activities. Drops sensitive
/// Activities via the SAME hard guardrail used for Conclusions (#4) — an
/// Activity's `title` reads as the "subject", its `summary` as the "statement",
/// closing the asymmetry where Activity text crossed the broker boundary
/// unfiltered. Then scores survivors by whole-word (#1), stemmed (#3),
/// rare-token-weighted (#2 IDF) overlap of the query against title+summary+
/// category. Keeps score>0 (sorted by score desc, recency desc), falls back to
/// most-recent when the query has no usable tokens, and truncates to `limit`. No
/// ids or evidence refs cross the boundary.
fn select_relevant_activities(
    activities: &[capture_types::Activity],
    tokens: &[String],
    limit: usize,
) -> Vec<BrokerRecalledActivity> {
    // #4: guardrail Activities the same way Conclusions are guardrailed. The
    // guardrail is pure text-pattern matching (subject + statement combined), so
    // running it over title (as subject) + summary (as statement) catches a
    // sensitive Activity before it can be scored or returned.
    //
    // LOAD-BEARING — DO NOT REMOVE. Unlike Conclusions (filtered at derivation
    // time, so never persisted), an Activity's title/summary is persisted
    // UNFILTERED. This line is the ONLY thing stopping a sensitive Activity from
    // egressing to a cloud engine via recall_context. Removing it as "redundant"
    // silently opens a sensitive-text leak — see the `sensitive_activity_never_*`
    // regression test and `guardrail.rs`.
    let candidates: Vec<&capture_types::Activity> = activities
        .iter()
        .filter(|a| !crate::user_context::guardrail::is_sensitive(&a.title, &a.summary))
        .collect();

    let docs: Vec<std::collections::HashSet<String>> = candidates
        .iter()
        .map(|a| {
            let category = a
                .category
                .as_ref()
                .and_then(snake_case_enum_string)
                .unwrap_or_default();
            recall_doc_words(&format!("{} {} {}", a.title, a.summary, category))
        })
        .collect();
    let df = recall_document_frequencies(tokens, &docs);
    let n = candidates.len();

    let mut scored: Vec<(f64, &capture_types::Activity)> = candidates
        .iter()
        .zip(docs.iter())
        .map(|(a, words)| (recall_overlap_score(tokens, words, &df, n), *a))
        .collect();

    if tokens.is_empty() {
        // No usable query tokens: fall back to most-recent, STILL capped.
        scored.sort_by(|a, b| b.1.started_at_ms.cmp(&a.1.started_at_ms));
    } else {
        scored.retain(|(score, _)| *score > 0.0);
        scored.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.1.started_at_ms.cmp(&a.1.started_at_ms))
        });
    }

    scored
        .into_iter()
        .take(limit)
        .map(|(_, a)| BrokerRecalledActivity {
            title: a.title.clone(),
            summary: a.summary.clone(),
            category: a.category.as_ref().and_then(snake_case_enum_string),
            focus: a.focus.as_ref().and_then(snake_case_enum_string),
            started_at: format_unix_ms(a.started_at_ms.max(0) as u64),
            ended_at: format_unix_ms(a.ended_at_ms.max(0) as u64),
        })
        .collect()
}

fn encode_opaque_id(kind: &str, id: i64) -> String {
    let tag = match kind {
        "frame" => "f",
        "audio" => "a",
        _ => "x",
    };
    format!("{tag}{:x}", id.max(0))
}

fn encode_signed_opaque_id(kind: &str, id: i64, grant_id: Option<&str>, secret: &[u8]) -> String {
    let mut payload = encode_opaque_id(kind, id);
    if let Some(grant_id) = grant_id {
        payload.push_str(":g");
        payload.push_str(grant_id);
    }
    let signature = opaque_signature(&payload, secret);
    format!("{payload}.{signature}")
}

fn decode_opaque_id(value: &str) -> Option<(String, i64)> {
    decode_opaque_payload(value).map(|(kind, id, _grant_id)| (kind, id))
}

fn decode_opaque_payload(value: &str) -> Option<(String, i64, Option<String>)> {
    let value = value
        .split_once('.')
        .map_or(value, |(payload, _signature)| payload);
    let (value, grant_id) = value
        .split_once(":g")
        .map_or((value, None), |(payload, grant_id)| {
            (payload, Some(grant_id.to_string()))
        });
    let mut chars = value.chars();
    let kind = chars.next()?;
    let rest = chars.as_str();
    let id = i64::from_str_radix(rest, 16).ok()?;
    let kind = match kind {
        'f' => "frame",
        'a' => "audio",
        _ => return None,
    };
    Some((kind.to_string(), id, grant_id))
}

fn decode_signed_opaque_id(value: &str, secret: &[u8]) -> Option<BrokerOpaqueCaptureReference> {
    let (payload, signature) = value.split_once('.')?;
    if signature.len() != OPAQUE_SIGNATURE_HEX_LEN
        || !signature.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return None;
    }
    if !opaque_signature_matches(payload, signature, secret) {
        return None;
    }
    let (kind, id, grant_id) = decode_opaque_payload(payload)?;
    Some(BrokerOpaqueCaptureReference {
        opaque_id: value.to_string(),
        frame_id: (kind == "frame").then_some(id),
        audio_segment_id: (kind == "audio").then_some(id),
        grant_id,
        kind,
    })
}

pub fn opaque_capture_reference(value: &str) -> Option<BrokerOpaqueCaptureReference> {
    let (kind, id) = decode_opaque_id(value)?;
    Some(BrokerOpaqueCaptureReference {
        opaque_id: value.to_string(),
        frame_id: (kind == "frame").then_some(id),
        audio_segment_id: (kind == "audio").then_some(id),
        grant_id: None,
        kind,
    })
}

pub fn signed_opaque_capture_reference(
    config_dir: &Path,
    value: &str,
) -> Result<Option<BrokerOpaqueCaptureReference>> {
    let path = config_dir.join(BROKER_OPAQUE_SECRET_FILE_NAME);
    if !path.exists() {
        return Ok(None);
    }
    let mut secret = Vec::new();
    File::open(&path)?.read_to_end(&mut secret)?;
    if secret.len() < 32 {
        return Ok(None);
    }
    Ok(decode_signed_opaque_id(value, &secret))
}

fn opaque_signature(payload: &str, secret: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret);
    hasher.update(b":");
    hasher.update(payload.as_bytes());
    let digest = hasher.finalize();
    digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn opaque_signature_matches(payload: &str, signature: &str, secret: &[u8]) -> bool {
    let expected = opaque_signature(payload, secret);
    let expected = expected.as_bytes();
    let signature = signature.as_bytes();
    expected.len() == signature.len()
        && expected
            .iter()
            .zip(signature.iter())
            .fold(0_u8, |acc, (left, right)| acc | (left ^ right))
            == 0
}

fn load_or_create_opaque_secret(config_dir: &Path) -> Result<Vec<u8>> {
    fs::create_dir_all(config_dir)?;
    let path = config_dir.join(BROKER_OPAQUE_SECRET_FILE_NAME);
    if path.exists() {
        let mut secret = Vec::new();
        File::open(&path)?.read_to_end(&mut secret)?;
        if secret.len() >= 32 {
            return Ok(secret);
        }
    }

    let lock_path = config_dir.join(BROKER_GRANTS_LOCK_FILE_NAME);
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(lock_path)?;
    lock.lock_exclusive()?;
    if path.exists() {
        let mut secret = Vec::new();
        File::open(&path)?.read_to_end(&mut secret)?;
        if secret.len() >= 32 {
            lock.unlock()?;
            return Ok(secret);
        }
    }

    let mut secret = vec![0_u8; 32];
    rand::thread_rng().fill_bytes(&mut secret);
    let mut file = File::create(path)?;
    file.write_all(&secret)?;
    let unlock_result = lock.unlock();
    unlock_result?;
    Ok(secret)
}

fn invalid_opaque_id_error() -> BrokerErrorResponse {
    BrokerErrorResponse {
        error: BrokerAuthStatusKind::AuthorizationRequired,
        message: "invalid opaque result id".to_string(),
    }
}

/// A handle this broker never signed, or one mangled in transit. Rejected the way
/// an invalid opaque capture id is: never as "everyone" and never as "nobody".
fn invalid_speaker_handle_error() -> BrokerErrorResponse {
    BrokerErrorResponse {
        error: BrokerAuthStatusKind::AuthorizationRequired,
        message: "invalid speaker handle".to_string(),
    }
}

fn outside_scope_error() -> BrokerErrorResponse {
    BrokerErrorResponse {
        error: BrokerAuthStatusKind::AuthorizationRequired,
        message: "result is unavailable or outside the grant scope".to_string(),
    }
}

fn open_mnema_deep_link(opaque_id: &str) -> Result<()> {
    let url = format!("mnema://open/{opaque_id}");
    open_external_url(&url)
}

/// Open a URL in the platform default handler via the OS opener command.
///
/// This is now reached ONLY by `open_mnema_deep_link` with an internal
/// `mnema://open/<opaque_id>` deep link — the broker never opens a raw captured
/// `browser_url` for any caller (that is local-desktop-only; see the
/// `OpenCapturedUrl` arm of `execute_authorized_request` and ADR 0038). Because
/// only internally-constructed `mnema://` ids reach the Windows `cmd /C start`
/// branch, the latent argument-injection sink it carried is no longer reachable
/// from any attacker-influenced (captured) input.
fn open_external_url(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("open").arg(url).status()?;
        if status.success() {
            Ok(())
        } else {
            Err(AppInfraError::BrokeredAccess(format!(
                "failed to open URL with status {status}"
            )))
        }
    }
    #[cfg(target_os = "windows")]
    {
        let status = std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(AppInfraError::BrokeredAccess(format!(
                "failed to open URL with status {status}"
            )))
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let status = std::process::Command::new("xdg-open").arg(url).status()?;
        if status.success() {
            Ok(())
        } else {
            Err(AppInfraError::BrokeredAccess(format!(
                "failed to open URL with status {status}"
            )))
        }
    }
}

fn map_search_response(
    response: SearchCaptureResponse,
    limit: u32,
    cursor: Option<BrokerSearchCursor>,
    grant_id: Option<&str>,
    opaque_secret: &[u8],
) -> BrokerSearchResponse {
    // ONE ranked page across both anchor kinds. Search returns frames and audio
    // as two separately-ranked lists; merging them by score (rather than
    // alternating one of each) is sound because both are scored by the same BM25
    // weights over the same `search_documents_fts` index. Each list is already
    // rank-ordered, so a front-of-list merge yields the global top `limit`.
    let (frame_page, audio_page) = (response.frames.len(), response.audio.len());
    let snapshot_document_id = response.snapshot_document_id;
    let (has_more_frames, has_more_audio) = (response.has_more_frames, response.has_more_audio);
    let mut frames = response.frames.into_iter().peekable();
    let mut audio = response.audio.into_iter().peekable();
    let (mut frames_taken, mut audio_taken) = (0u32, 0u32);
    let mut results = Vec::new();
    while results.len() < limit as usize {
        let take_frame = match (frames.peek(), audio.peek()) {
            (Some(frame), Some(audio_result)) => frame_outranks_audio(
                (frame.rank, &frame.group_start_at),
                (audio_result.rank, &audio_result.absolute_start_at),
            ),
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };
        if take_frame {
            let Some(frame) = frames.next() else { break };
            results.push(BrokerSearchResult {
                opaque_id: encode_signed_opaque_id(
                    "frame",
                    frame.representative_frame.id,
                    grant_id,
                    opaque_secret,
                ),
                kind: "frame".to_string(),
                snippet: frame.snippet,
                started_at: frame.group_start_at,
                ended_at: frame.group_end_at,
                context: broker_search_result_context(
                    frame.app_bundle_id,
                    frame.app_name,
                    frame.window_title,
                    // Read-time URL guard: the representative frame's captured
                    // `browser_url` (from the metadata snapshot) is sanitized +
                    // secret-redacted before it leaves the broker boundary. Only
                    // http(s) URLs survive; everything else guards to `None`.
                    frame
                        .browser_url
                        .as_deref()
                        .and_then(url_guard::guard_url),
                ),
                // Frame results have no sub-segment audio anchor.
                span_start_ms: None,
                span_end_ms: None,
                aligned_frame_id: None,
            });
            frames_taken += 1;
        } else {
            let Some(audio_result) = audio.next() else { break };
            results.push(BrokerSearchResult {
                opaque_id: encode_signed_opaque_id(
                    "audio",
                    audio_result.audio_segment.id,
                    grant_id,
                    opaque_secret,
                ),
                kind: broker_audio_kind(&audio_result.audio_segment.source_kind).to_string(),
                snippet: audio_result.snippet,
                started_at: audio_result.absolute_start_at,
                ended_at: audio_result.absolute_end_at,
                context: None,
                // Audio Search Result Anchor: carry the match span + aligned
                // frame so a consumer can land on the cited transcript moment.
                span_start_ms: Some(audio_result.span_start_ms as i64),
                span_end_ms: Some(audio_result.span_end_ms as i64),
                aligned_frame_id: audio_result.aligned_frame.as_ref().map(|frame| frame.id),
            });
            audio_taken += 1;
        }
    }
    // More to walk when either anchor kind has rows this page never emitted:
    // left behind by the `limit` cap, or still behind the per-kind `has_more`.
    // `limit == 0` emits nothing and so can never advance the cursor — handing one
    // back would make the documented "page until nextCursor is absent" walk loop
    // on an identical empty page forever.
    let more = limit > 0
        && ((frames_taken as usize) < frame_page
            || (audio_taken as usize) < audio_page
            || has_more_frames
            || has_more_audio);
    let base = cursor.unwrap_or(BrokerSearchCursor {
        snapshot_document_id,
        frame_offset: 0,
        audio_offset: 0,
    });
    let next_cursor = more.then(|| {
        BrokerSearchCursor {
            // Pin every later page to the snapshot this walk started on, so
            // captures landing mid-walk cannot shift rows across pages.
            snapshot_document_id: base.snapshot_document_id,
            // Resume from what this page CONSUMED per kind, not `offset + limit`:
            // a rank-merged page can be all frames, all audio, or any split.
            frame_offset: base.frame_offset + frames_taken,
            audio_offset: base.audio_offset + audio_taken,
        }
        .encode()
    });
    BrokerSearchResponse {
        results,
        limit,
        next_cursor,
    }
}

/// Order one frame result against one audio result: better (LOWER) rank first,
/// newest first on an exact tie, frame before audio to keep the merge total.
fn frame_outranks_audio(frame: (f64, &str), audio: (f64, &str)) -> bool {
    match frame.0.total_cmp(&audio.0) {
        std::cmp::Ordering::Less => true,
        std::cmp::Ordering::Greater => false,
        std::cmp::Ordering::Equal => frame.1 >= audio.1,
    }
}

fn broker_search_result_context(
    app_bundle_id: Option<String>,
    app_name: Option<String>,
    window_title: Option<String>,
    url: Option<String>,
) -> Option<BrokerSearchResultContext> {
    if app_bundle_id.is_none() && app_name.is_none() && window_title.is_none() && url.is_none() {
        return None;
    }
    Some(BrokerSearchResultContext {
        app_bundle_id,
        app_name,
        window_title,
        url,
    })
}

#[cfg(test)]
mod tests;
