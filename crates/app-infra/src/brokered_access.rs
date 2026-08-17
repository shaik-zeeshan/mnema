use std::{
    collections::HashMap,
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
use time::{format_description::well_known::Rfc3339, OffsetDateTime, UtcOffset};

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
    broker_speaker_refinement, broker_speakers, broker_speakers_for_audio, speaker_coverage,
    speaker_matched_recordings_in_range, speaker_matched_turns_for_segments,
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
/// How many brokered-command lines the activity log keeps.
const MAX_AUDIT_EVENTS: usize = 500;
/// A CLI Access permission dies this long after its last use. Idle expiry, not a
/// TTL: a tool in daily use is never re-prompted, and one tried once in March is
/// not still reading history in December (ADR 0059).
pub const BROKER_GRANT_IDLE_TTL_MS: u64 = 30 * 24 * 60 * 60 * 1000;
/// How stale `last_used_at_unix_ms` must be before a brokered call pays for a
/// flocked rewrite. One-hour resolution against a 30-day threshold, so the common
/// read path takes the shared read and returns without writing (ADR 0041).
const STAMP_INTERVAL_MS: u64 = 60 * 60 * 1000;
/// How far past a permission's own start a request may reach before the clamp is
/// REPORTED. The data clamp itself is always exact — `scope_start` is a security
/// boundary and never moves — this only suppresses the `scope_clamped` marker.
///
/// The marker exists to stop an agent reporting "nothing happened" for a window
/// it was never allowed to see. A sub-minute shortfall at the far edge of a
/// 24-hour window is not that: `--from` = "24 hours ago" is computed by the
/// caller and evaluated by the broker milliseconds later, so it is over the line
/// by construction, and `minimum_scope_for_start` is `age > 24h` — one
/// millisecond escalates a WHOLE BAND and the CLI opens an approval window
/// asking for a week on the most natural query the `lastDay` band has. A
/// spurious approval window on every ordinary query is the real harm.
const CLAMP_SLACK_MS: u64 = 60_000;
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrokerGrantScope {
    RecentDays { days: u32 },
    AllRetainedHistory,
}

impl BrokerGrantScope {
    pub const LAST_DAY: Self = Self::RecentDays { days: 1 };
    pub const LAST_7_DAYS: Self = Self::RecentDays { days: 7 };

    /// The one wire spelling of a scope, shared by the authorization channel, the
    /// CLI, and the clamp marker so the three cannot drift apart.
    ///
    /// `RecentDays` carries an arbitrary `u32`, and only three of its values have
    /// a wire spelling: 2..=7 days round UP to `last7Days` (narrower than the row
    /// grants, safe), but anything over 7 falls through to `allRetained`, which is
    /// WIDER than the row grants. Unreachable today — `from_wire_name` is the only
    /// writer and yields 1, 7, or `AllRetainedHistory` — but `access_status_line`
    /// and `verify_granted_scope`'s error message both print through here, so a
    /// future non-banded `RecentDays` would have them overstate the permission.
    pub fn wire_name(&self) -> &'static str {
        match self {
            Self::RecentDays { days } if *days <= 1 => "lastDay",
            Self::RecentDays { days } if *days <= 7 => "last7Days",
            Self::RecentDays { .. } | Self::AllRetainedHistory => "allRetained",
        }
    }

    pub fn from_wire_name(value: &str) -> Option<Self> {
        match value {
            "lastDay" => Some(Self::LAST_DAY),
            "last7Days" => Some(Self::LAST_7_DAYS),
            "allRetained" => Some(Self::AllRetainedHistory),
            _ => None,
        }
    }

    /// Does a permission at this scope cover everything `other` asks for?
    pub fn covers(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::AllRetainedHistory, _) => true,
            (Self::RecentDays { .. }, Self::AllRetainedHistory) => false,
            (Self::RecentDays { days }, Self::RecentDays { days: needed }) => days >= needed,
        }
    }
}

/// A standing per-tool CLI Access permission: one row per normalized Broker
/// Client Identity, with no calendar expiry. The row dies
/// [`BROKER_GRANT_IDLE_TTL_MS`] after its last use, or is `blocked` — a standing
/// user rejection that is denied without prompting and never idle-expires.
///
/// The `id` is what opaque result ids are MAC-signed against, so widening a
/// permission MUST keep it (see [`upsert_grant_for_identity`]); minting a new one
/// would kill every id already handed to a running agent.
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
    // ponytail: `default` (= 0) rather than a required field only so a
    // pre-ADR-0059 file parses instead of hard-failing the whole broker. A row
    // that lands here with 0 is instantly idle-expired and pruned, which is
    // exactly the "old files are ignored" the ADR asks for. No conversion shim.
    #[serde(default)]
    pub last_used_at_unix_ms: u64,
    pub scope: BrokerGrantScope,
    #[serde(default)]
    pub blocked: bool,
    #[serde(default)]
    pub blocked_at_unix_ms: Option<u64>,
}

/// Result of [`upsert_grant_for_identity`]: the single row for this identity, and
/// whether it was newly created or an existing row widened in place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerGrantUpsert {
    pub grant: BrokerGrant,
    pub created: bool,
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
    /// What the FILTERED speaker said in this recording, in order — present only
    /// when the request carried a `speaker` handle, and holding only that
    /// speaker's turns, never everyone's. Reading them needs no `show-text`.
    ///
    /// Absent on an unfiltered result (nothing was asked about a person) and on a
    /// filtered result whose matched turns carry no transcribed words. As on
    /// `show-text`, absence NEVER means silence — `snippet`, and `show-text`
    /// behind `opaqueId`, still hold the recording's words.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub turns: Vec<BrokerSpeakerTurn>,
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
    /// Only on a request that carried a `speaker` handle: how much audio the
    /// filter could not check. See [`BrokerSpeakerCoverage`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_coverage: Option<BrokerSpeakerCoverage>,
    /// The permission's scope cut the requested window SHORT: these results cover
    /// less than what was asked for. Never read an empty or thin page as "nothing
    /// happened in that window" while this is set — widen the tool's access first.
    #[serde(default)]
    pub scope_clamped: bool,
    /// Narrowest scope that would have covered the request — `lastDay`,
    /// `last7Days`, or `allRetained`. Present only when `scope_clamped`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_scope: Option<String>,
}

/// What a speaker filter could NOT see, counted in recordings over the same time
/// range the request covered. A speaker filter can only match through detected
/// speaker turns, so these two counts are the honest edge of every filtered
/// answer: they are the audio the filter had no way to check, and a non-zero
/// count means "this answer may be incomplete", never "they said nothing more".
///
/// TWO counts, because the remedies differ — collapsing them into one total
/// destroys the only thing that makes them actionable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerSpeakerCoverage {
    /// Recordings holding at least one voice nobody has named — no assignment and
    /// no recognition guess. Any of them could be the person that was asked
    /// about, and nothing here can tell. FIXABLE BY THE USER: labeling that voice
    /// in Mnema brings the recording into reach of this filter.
    pub recordings_with_unnamed_voices: u32,
    /// Recordings where speaker detection produced NOTHING at all — no turns, no
    /// voices, nobody to match. Common, and NOT the same as silence: the
    /// transcript is usually still there to read with `show-text`. NOT fixable by
    /// the user; no speaker filter can ever reach this audio.
    pub recordings_without_speaker_data: u32,
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
    /// OMITTED rather than sent as `null` when there is no name — the same rule
    /// [`BrokerSpeakerSummary::name`] follows, because both published contracts
    /// (`SKILL.md`, `crates/cli/CONTEXT.md`) describe one `name` rule for a
    /// nameless voice and an agent testing presence must not read the same voice
    /// as named on `show-text` and unnamed on `speakers`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    /// `voice` — ONE voice inside ONE capture SESSION, **not a person**. A session
    /// is a continuous sitting, and recordings are capped at 5 minutes, so one
    /// `voice` handle covers every consecutive recording in that sitting — filter
    /// on it and several come back. Across two sittings the same human gets two
    /// unrelated handles (and often several within one), and the handle dies when
    /// that session is re-analyzed. Never persist it, never merge two of them, and
    /// never present it as an identity.
    pub kind: String,
    /// `voice` only: the span this voice was heard over, in ms from the start of
    /// the ONE recording this handle was published for — turn offsets are relative
    /// to each recording's own start. It is NOT the handle's reach, which extends
    /// to every recording in the session ([`Self::kind`]); `speakers` omits it
    /// outright for exactly that reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_ms: Option<u64>,
}

/// One attributed stretch of speech: who said it, when, and the words.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerSpeakerTurn {
    /// Index into the response's `speakers[]`. ABSENT on a `search`/`timeline`
    /// result filtered by speaker: every turn there belongs to the one speaker the
    /// request named, so there is no `speakers[]` list and nothing to index into.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<usize>,
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
    /// What the FILTERED speaker said in this recording — same rules as
    /// [`BrokerSearchResult::turns`], including that absence is never silence.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub turns: Vec<BrokerSpeakerTurn>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerTimelineResponse {
    pub intervals: Vec<BrokerTimelineInterval>,
    pub limit: u32,
    /// The page is the NEWEST `limit` intervals (every branch orders
    /// `started_at DESC`), so a full page means the window almost certainly holds
    /// more and what came back is only its TAIL. At the default limit of 20 an
    /// 18-hour "what did I do today" window answers with the last few minutes;
    /// without this flag a caller cannot tell that apart from a quiet day, and
    /// reads a truncated tail as the whole span.
    // Same rule the CLI has always applied to its own `truncated` (`limit 0` can
    // never be complete), hoisted here so Ask AI and the CLI cannot disagree.
    // ponytail: `len >= limit` over-reports the one window holding exactly
    // `limit` intervals. An exact answer costs a COUNT over the same window, and
    // narrow-the-window is the right response either way.
    #[serde(default)]
    pub truncated: bool,
    /// Oldest / newest `startedAt` actually returned — the slice of the requested
    /// window this page really covers, which is what makes `truncated`
    /// actionable. `None` when nothing matched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub covered_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub covered_to: Option<String>,
    /// Only on a request that carried a `speaker` handle: how much audio the
    /// filter could not check. See [`BrokerSpeakerCoverage`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_coverage: Option<BrokerSpeakerCoverage>,
    /// The permission's scope cut the requested window SHORT: these results cover
    /// less than what was asked for. Never read an empty or thin page as "nothing
    /// happened in that window" while this is set — widen the tool's access first.
    #[serde(default)]
    pub scope_clamped: bool,
    /// Narrowest scope that would have covered the request — `lastDay`,
    /// `last7Days`, or `allRetained`. Present only when `scope_clamped`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_scope: Option<String>,
}

impl BrokerTimelineResponse {
    /// The one constructor for a timeline page: derives `truncated` + the covered
    /// span from the intervals themselves, so no branch can return a page that
    /// silently misreports its own coverage.
    pub fn page(
        intervals: Vec<BrokerTimelineInterval>,
        limit: u32,
        speaker_coverage: Option<BrokerSpeakerCoverage>,
    ) -> Self {
        // Bounds come from the intervals rather than from their order: both
        // branches happen to sort `started_at DESC` today, but a page that lies
        // about its coverage is exactly the failure this field exists to prevent.
        // The values are `Z`-normalized RFC3339, so lexical min/max is
        // chronological.
        let covered_from = intervals
            .iter()
            .map(|interval| interval.started_at.clone())
            .min();
        let covered_to = intervals
            .iter()
            .map(|interval| interval.started_at.clone())
            .max();
        Self {
            truncated: intervals.len() as u32 >= limit,
            covered_from,
            covered_to,
            intervals,
            limit,
            speaker_coverage,
            // Set by the caller that knows the grant; `page` only sees intervals.
            scope_clamped: false,
            required_scope: None,
        }
    }
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
    /// `None` for a `voice` handle — an unnamed voice has no name to report, so
    /// the key is OMITTED rather than sent as `null`, which is what both published
    /// agent contracts promise (`SKILL.md`, `crates/cli/CONTEXT.md`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
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

/// An `activities` request: the window to report, as RFC3339 UTC bounds. No
/// query and no `limit` — this is the chronological day-scale door, and a
/// relevance filter or a caller-chosen cap is what makes it stop being one. The
/// server-side [`MAX_ACTIVITIES`] cap is a runaway guard, not a paging knob.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerActivitiesRequest {
    pub from: String,
    pub to: String,
}

/// One derived episode returned by `activities`.
///
/// Deliberately NOT [`BrokerRecalledActivity`], despite sharing six fields: the
/// two doors have different privacy contracts. `recall_context` emits no ids at
/// all, by design. This one emits a followable `opaque_id` so the model can cite
/// what it summarized — which grants nothing `search`/`timeline` do not already
/// grant on the same grant scope, but IS a different boundary. Keeping them
/// separate is what stops "just populate the optional field" from quietly
/// erasing the distinction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerActivity {
    pub title: String,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
    pub started_at: String,
    pub ended_at: String,
    /// The activity's best surviving evidence frame, for citation. Absent when
    /// every frame that grounded it has aged out of Retention — the activity
    /// itself outlives them (ADR 0029), so this is expected on old windows, not
    /// an error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opaque_id: Option<String>,
    /// App / window / guarded url of THAT evidence frame — the one capture
    /// `opaque_id` points at, not a claim about the whole episode. An episode can
    /// span several apps, but the frame behind it was captured in exactly one,
    /// and that is what a source card for this id renders.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<BrokerSearchResultContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerActivitiesResponse {
    /// Oldest-first: this answers "walk me through the window", so chronological
    /// order IS the answer's shape.
    pub activities: Vec<BrokerActivity>,
    /// The sub-range of the REQUESTED window that derivation has actually
    /// covered, as RFC3339 UTC. Activities are derived asynchronously in 2–30
    /// minute windows, so the newest stretch of any live window has not been
    /// summarized yet and the oldest may predate backfill.
    ///
    /// This is the field that keeps an empty list honest. Without it "no
    /// activities" reads as "you did nothing", when it usually means "not
    /// summarized yet" — the same failure mode as a truncated `timeline` page
    /// read as a whole day. Both `None` means derivation has covered NO part of
    /// this window, so the list says nothing about it either way.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived_until: Option<String>,
    /// The runaway guard tripped: more than [`MAX_ACTIVITIES`] episodes overlap
    /// this window and the OLDEST were kept. Narrow the window.
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrokeredCaptureRequest {
    AuthStatus,
    Search(BrokerSearchRequest),
    ShowText { opaque_id: String },
    Timeline(BrokerTimelineRequest),
    Speakers(BrokerSpeakersRequest),
    RecallContext(BrokerRecallContextRequest),
    Activities(BrokerActivitiesRequest),
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
            Self::Activities(_) => Some("activities"),
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
    // AFTER `RecallContext`: this enum is `#[serde(untagged)]`, so variants are
    // tried in declaration order and both carry an `activities` array. A
    // recall payload also carries the required `conclusions`, which this variant
    // lacks, so ordering it second means each shape lands on its own arm.
    Activities(BrokerActivitiesResponse),
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
            Self::Activities(response) => response.activities.len() as u32,
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
        let grant = self.active_grant_for_identity(&identity)?;
        let (response, outcome) = match grant.as_ref() {
            None => (
                BrokeredCaptureResponse::Error(BrokerErrorResponse::authorization_required()),
                "denied",
            ),
            Some(grant) => {
                // Coarse, and deliberately not `?`: a stamping failure must not
                // fail a read. The worst case is the permission idling out early
                // and the user being asked once more.
                let _ = touch_last_used(&self.config_dir, &identity.normalized_label);
                let response = self
                    .execute_authorized_request(std::slice::from_ref(grant), request)
                    .await?;
                let outcome = if matches!(response, BrokeredCaptureResponse::Error(_)) {
                    "scope_rejected"
                } else {
                    "success"
                };
                (response, outcome)
            }
        };

        if let Some(command_type) = command_type {
            let audited = self.audit_result(
                grant.as_ref(),
                identity,
                command_type,
                response.result_count(),
                outcome,
            );
            // A REFUSAL must survive its own audit line. Denials are recorded now,
            // and this write is on the path of a caller holding no permission at
            // all — so an unusable sink (full disk, unwritable config dir) would
            // replace `authorizationRequired` with a broker error. That is the one
            // response the CLI's approval flow keys off: an `Io` error is not
            // `outside_grant_scope`, so the CLI returns before
            // `response_requires_authorization` ever runs and a first-time tool
            // gets no approval window at all — on the one call that exists to open
            // it. There is no permission here to protect by failing closed.
            //
            // A call that was actually SERVED still fails on an unrecorded access,
            // exactly as it did before denials were logged.
            if grant.is_some() {
                audited?;
            }
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
        // NO audit event. Ask AI runs an agent loop, so one event per tool call
        // evicted every real CLI event from the 500-slot FIFO within a couple of
        // dozen conversations. Ask AI already cites its sources per answer in
        // `AnswerSourceCard`, which is better evidence than an audit line, and it
        // can never appear as a permission row anyway (ADR 0059).
        let grants = vec![ask_ai_all_retained_grant(&identity)];
        self.execute_authorized_request(&grants, request).await
    }

    pub fn list_grants(&self) -> Result<BrokerGrantFile> {
        load_grants(&self.config_dir)
    }

    /// Approve (or widen) this identity's standing permission. Upserts: the row
    /// keeps its id so opaque ids already issued to the tool keep resolving.
    pub fn upsert_grant_for_identity(
        &self,
        identity: BrokerClientIdentity,
        scope: BrokerGrantScope,
    ) -> Result<BrokerGrantUpsert> {
        upsert_grant_for_identity(&self.config_dir, identity, scope)
    }

    pub fn block_client(&self, client_label: &str) -> Result<bool> {
        block_client(&self.config_dir, client_label)
    }

    pub fn unblock_client(&self, client_label: &str) -> Result<bool> {
        unblock_client(&self.config_dir, client_label)
    }

    pub fn list_history(&self) -> Result<BrokerAuditFile> {
        load_audit_events(&self.config_dir)
    }

    fn active_grant_for_identity(
        &self,
        identity: &BrokerClientIdentity,
    ) -> Result<Option<BrokerGrant>> {
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
            BrokeredCaptureRequest::Activities(request) => {
                let infra = self.initialize_infra().await?;
                match broker_activities(&self.config_dir, &infra, grants, request).await? {
                    Ok(response) => Ok(BrokeredCaptureResponse::Activities(response)),
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

    /// One audit line per brokered command, INCLUDING the ones that were refused —
    /// a permission log that only records successes cannot answer "did anything
    /// try and get turned away".
    fn audit_result(
        &self,
        grant: Option<&BrokerGrant>,
        identity: BrokerClientIdentity,
        command_type: &str,
        result_count: u32,
        outcome: &str,
    ) -> Result<()> {
        record_audit_event(
            &self.config_dir,
            identity,
            command_type,
            result_count,
            grant.map_or_else(|| "none".to_string(), scope_class),
            grant.map(|grant| grant.id.clone()),
            outcome,
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

/// Longest client name kept. A tool NAME, not a payload: `--client`,
/// `MNEMA_CLI_CLIENT` and `AI_AGENT` all reach here straight off the wire with
/// no length of their own, and each one is stored TWICE per audit line (raw +
/// normalized) in a file capped at 500 lines but not at 500 line-lengths.
///
/// A refused command now writes an audit line too, so a caller with NO
/// permission sets that file's size. Measured, uncapped: a 64 KB `--client`
/// grows `broker-audit.json` to 62.6 MB, at which point one refused command
/// costs 210 ms instead of 1.8 ms (every command rewrites the whole file) and
/// the Settings Access panel re-reads all 62.6 MB over IPC every 30 s to render
/// 20 rows. Capping the name is the one bound that closes both.
///
/// BYTES, not chars: the file is measured in bytes and a char is up to four of
/// them, so a 120-CHAR cap bounds nothing. Measured, 120 emoji push the same file
/// to 621 KB and one refused command to 915 µs, against 261 KB / 540 µs for 120
/// ASCII characters — and ASCII is the one shape that makes a char cap look like
/// a byte cap, which is why the regression test spells all three.
const MAX_CLIENT_LABEL_BYTES: usize = 120;

/// Cut a collapsed label to [`MAX_CLIENT_LABEL_BYTES`] on a char boundary.
///
/// Applied inside `display_client_label`, the ONE collapse both the displayed
/// name and the identity key run through, so `normalize_client_label` inherits
/// the same cut. Not in `BrokerClientIdentity::new`: the direct callers of
/// `normalize_client_label` (the standing-block check, `set_client_blocked`, the
/// audit backfill) would otherwise key on an uncapped name that no stored row
/// could ever match.
///
/// The cap is a collision surface by construction — two names sharing their
/// first 120 bytes become one permission row. 120 is chosen to sit far above any
/// real tool name while still bounding the audit line.
fn cap_client_label(collapsed: String) -> String {
    if collapsed.len() <= MAX_CLIENT_LABEL_BYTES {
        return collapsed;
    }
    // Back off to the nearest char boundary — at most three bytes, so still O(1).
    let mut cut = MAX_CLIENT_LABEL_BYTES;
    while cut > 0 && !collapsed.is_char_boundary(cut) {
        cut -= 1;
    }
    // Re-trim: the cut can land just after a space and leave a trailing one,
    // which would no longer round-trip through this same collapse.
    collapsed[..cut].trim_end().to_string()
}

pub fn normalize_client_label(value: &str) -> Option<String> {
    // Separators first, then the ONE display collapse: the identity key is the
    // displayed name lowercased, so the two cannot drift into disagreeing about
    // whether two spellings are the same tool.
    let separated = value
        .chars()
        .map(|ch| if ch == '-' || ch == '_' { ' ' } else { ch })
        .collect::<String>();
    // Re-cap AFTER lowercasing. `to_lowercase` can GROW a string ('İ' becomes
    // "i\u{307}"), so a name whose collapsed form sits exactly at the cap would
    // store a key one char over it — and Settings blocks by the STORED key, which
    // comes back through this same function. A key this function cannot reproduce
    // from its own output is a block the user cannot enforce.
    let normalized = cap_client_label(display_client_label(&separated).to_lowercase());
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

/// Unicode's `Default_Ignorable_Code_Point` set plus the blank-glyph outliers it
/// leaves out (U+2800 BRAILLE PATTERN BLANK, the U+FFF9..=U+FFFB annotation
/// controls): the closed list of "this is meant to render as nothing".
///
/// `char::is_control` covers only category `Cc`, so without this a client named
/// `"Claude\u{200B} Code"` keys a DIFFERENT permission row than `"Claude Code"`
/// while rendering identically in the access list and the approval window. The
/// standing block is keyed on `normalize_client_label`, so an identity key that
/// disagrees with what the user reads is a block the user cannot enforce.
/// Mirrors `is_invisible_or_reordering` in the authorization channel.
fn is_invisible_client_label_char(ch: char) -> bool {
    matches!(ch,
        '\u{00AD}' | '\u{034F}' | '\u{061C}' | '\u{2800}' | '\u{3164}' | '\u{FEFF}' | '\u{FFA0}'
        | '\u{115F}'..='\u{1160}'
        | '\u{17B4}'..='\u{17B5}'
        | '\u{180B}'..='\u{180F}'
        | '\u{200B}'..='\u{200F}'
        | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{206F}'
        | '\u{FE00}'..='\u{FE0F}'
        | '\u{FFF0}'..='\u{FFFB}'
        | '\u{1BCA0}'..='\u{1BCA3}'
        | '\u{1D173}'..='\u{1D17A}'
        | '\u{E0000}'..='\u{E0FFF}'
    )
}

/// The one collapse a client name goes through. Invisibles are dropped, control
/// characters fold to the space they already render as (matching the approval
/// window's own copy), and whitespace runs collapse.
fn display_client_label(value: &str) -> String {
    cap_client_label(
        value
            .chars()
            .filter(|ch| !is_invisible_client_label_char(*ch))
            .map(|ch| if ch.is_control() { ' ' } else { ch })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// Display-normalize a stored row. It must NOT rewrite `normalized_label` into
/// something other than what `normalize_client_label` produces for the row's own
/// label: that key is the one-row-per-identity invariant, and a load-time rewrite
/// that disagrees with the upsert's lookup silently re-opens the duplicate-row
/// bug this design exists to close. (The old "Local agent" → "mnema CLI" rewrite
/// did exactly that; it was a pre-rename shim and there are no installed users.)
fn normalize_loaded_grant(grant: &mut BrokerGrant) {
    if grant.label.trim().is_empty() {
        grant.label = BrokerClientIdentity::default_cli().label;
    } else {
        grant.label = display_client_label(&grant.label);
    }
    if grant.normalized_label.trim().is_empty() {
        grant.normalized_label = normalize_client_label(&grant.label)
            .unwrap_or_else(|| BrokerClientIdentity::default_cli().normalized_label);
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

fn read_grants_file(config_dir: &Path) -> Result<BrokerGrantFile> {
    let path = config_dir.join(BROKER_GRANTS_FILE_NAME);
    if !path.exists() {
        return Ok(BrokerGrantFile {
            schema_version: 1,
            grants: Vec::new(),
        });
    }
    let raw = fs::read_to_string(path)?;
    // A zero-length file is the artifact of a crash between `save_grants_locked`'s
    // rename and the data blocks reaching disk. It holds no permission and no
    // block, so there is nothing to protect by failing closed on it — and failing
    // would wedge every brokered command AND the approval upsert that is the only
    // way out, until someone deletes the file by hand. Non-empty garbage still
    // fails closed: that may be a mangled block list.
    if raw.trim().is_empty() {
        return Ok(BrokerGrantFile {
            schema_version: 1,
            grants: Vec::new(),
        });
    }
    Ok(normalize_loaded_grant_file(serde_json::from_str(&raw)?))
}

/// Read + prune. The prune is IN MEMORY here: this is the read path, and a
/// brokered read must not take the exclusive lock to rewrite a config file
/// (ADR 0041). The rewrite happens the next time anything opens the lock — see
/// [`with_grants_lock`].
fn load_grants(config_dir: &Path) -> Result<BrokerGrantFile> {
    let mut grants = read_grants_file(config_dir)?;
    prune_dead_grants(&mut grants, now_unix_ms());
    Ok(grants)
}

/// Write a file no other account on the machine can read.
///
/// The permission file is an access-control store, the audit file names every tool
/// that read the user's history, and the opaque-id secret is a key. On macOS they
/// live inside a `0700` app-support directory, so this is the second fence rather
/// than the only one — but the directory's mode is not this module's to guarantee,
/// and the mode has to ride the write it protects: these are all temp+rename
/// writes and a `rename` keeps the TEMP file's mode, so a chmod after the rename
/// would publish every byte for the window in between.
///
/// The mode is set twice on purpose. `mode()` applies only when `open` creates the
/// file, and a temp file left behind by a crashed write already exists.
fn write_owner_only(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(bytes)?;
    // Durable BEFORE the rename that publishes it. `rename` is ordered against
    // the directory entry, not against the data blocks, so without this a crash
    // in between leaves an entry carrying the new SIZE over blocks that were
    // never written — a file of the right length full of NULs. That artifact is
    // not `trim`-empty, so `read_grants_file`'s crash-artifact branch does not
    // catch it and `serde_json` hard-fails: every door errors at once, including
    // the approval upsert that is the only way back, and CLI Access stays bricked
    // until someone deletes the file by hand.
    //
    // Syncing here rather than widening that branch keeps the permission file
    // fail-CLOSED on garbage: the tolerant branch's rationale ("holds no
    // permission and no block") is false for a file that did hold a block, so
    // making the artifact impossible is the fix, not learning to read it.
    //
    // The directory entry is deliberately NOT synced: losing the rename leaves
    // the OLD file, which is safe. Costs one fsync per write — the audit log's is
    // the only per-command one, at well under the process spawn it rides on.
    file.sync_all()
}

fn save_grants_locked(config_dir: &Path, grants: &BrokerGrantFile) -> Result<()> {
    let path = config_dir.join(BROKER_GRANTS_FILE_NAME);
    let temp_path = config_dir.join(format!("{BROKER_GRANTS_FILE_NAME}.tmp"));
    let raw = serde_json::to_string_pretty(grants)?;
    write_owner_only(&temp_path, raw.as_bytes())?;
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
    let result = (|| {
        let mut grants = read_grants_file(config_dir)?;
        // The only place a prune reaches disk. Written before `f` runs rather
        // than merged into its own save, because `f` owns whether it saves at
        // all — and a prune write is rare by construction (rows live 30 days).
        if prune_dead_grants(&mut grants, now_unix_ms()) {
            save_grants_locked(config_dir, &grants)?;
        }
        f(&mut grants)
    })();
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
    outcome: &str,
) -> Result<()> {
    fs::create_dir_all(config_dir)?;
    let lock_path = config_dir.join(BROKER_AUDIT_LOCK_FILE_NAME);
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(lock_path)?;
    lock.lock_exclusive()?;

    let path = config_dir.join(BROKER_AUDIT_FILE_NAME);
    // A log that will not parse is ALREADY lost, and failing on it as well is
    // what turns one half-finished write into a permanent brick: every brokered
    // command propagates this error instead of answering, refusals included now
    // that those are recorded too — so the caller never even gets the
    // `authorizationRequired` its approval flow keys off. Quarantine the bytes
    // that are left and start a fresh log.
    let mut audit = load_audit_events(config_dir).unwrap_or_else(|_| {
        // Never over an existing quarantine. `rename` overwrites, and the file
        // already sitting there is the one holding real evidence: a log that lost
        // its tail to a half-finished write still carries every earlier line of
        // which tool read the user's history, and it is the only copy there is. A
        // second corruption erasing it is the loss this quarantine exists to
        // prevent, happening to the quarantine itself.
        // ponytail: one slot, first corruption wins. Uniquify the name if a
        // second one ever needs keeping — but not by littering the config dir.
        let quarantine = path.with_extension("json.corrupt");
        if !quarantine.exists() {
            let _ = fs::rename(&path, &quarantine);
        }
        BrokerAuditFile {
            schema_version: default_schema_version(),
            events: Vec::new(),
        }
    });
    audit.events.push(BrokerAuditEvent {
        tool_identity: identity.label,
        normalized_tool_identity: identity.normalized_label,
        identity_source: identity.source,
        command_type: command_type.into(),
        timestamp_unix_ms: now_unix_ms(),
        result_count,
        scope_class: scope_class.into(),
        grant_id,
        outcome: Some(outcome.to_string()),
    });
    // A `denied` line is written with NO permission at all: anything that can run
    // `mnema` appends one just by asking for access it does not have, and a
    // blocked tool still reaches this sink. Plain FIFO would let a few hundred
    // refusals erase every record of what an APPROVED tool actually read — the
    // same eviction that took Ask AI out of this log in this change. So OLD
    // refusals are dropped first, and only when none are left does plain FIFO run.
    //
    // The event just pushed is exempt from that scan. Without the exemption, a log
    // whose 500 lines are all successes — the steady state of any approved tool —
    // evicts every NEW refusal on arrival and can never again answer "did anything
    // try and get turned away", which is the question this log exists for. A
    // denial flood still costs at most ONE non-denied line in total: every later
    // denial recycles the previous denial's slot.
    // ponytail: only `denied` is unauthenticated; a flood of `scope_rejected`
    // still needs an approved permission, and is itself evidence. Split their
    // budgets if that ever stops being true.
    let mut over = audit.events.len().saturating_sub(MAX_AUDIT_EVENTS);
    if over > 0 {
        let newest = audit.events.len() - 1;
        let mut index = 0;
        audit.events.retain(|event| {
            let drop = over > 0 && index < newest && event.outcome.as_deref() == Some("denied");
            index += 1;
            if drop {
                over -= 1;
            }
            !drop
        });
        audit.events.drain(0..over);
    }
    // Temp + rename, exactly as `save_grants_locked` does: a plain `fs::write`
    // truncates the log before it writes a byte, so a crash or a full disk
    // partway through is what leaves the unparseable file handled above.
    let temp_path = config_dir.join(format!("{BROKER_AUDIT_FILE_NAME}.tmp"));
    let result = write_owner_only(&temp_path, serde_json::to_string_pretty(&audit)?.as_bytes())
        .and_then(|()| fs::rename(&temp_path, &path));
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
    !grant.blocked
        && now_unix_ms.saturating_sub(grant.last_used_at_unix_ms) < BROKER_GRANT_IDLE_TTL_MS
}

/// Drop rows that are neither usable nor a standing rejection. A BLOCKED row is
/// kept forever: idle expiry is benign disuse and re-prompts, blocking is a
/// decision the user made and must not be re-issued every time the tool runs.
///
/// Reports whether anything went, so the file is rewritten only on a real prune.
fn prune_dead_grants(grants: &mut BrokerGrantFile, now_unix_ms: u64) -> bool {
    let before = grants.grants.len();
    grants
        .grants
        .retain(|grant| grant.blocked || grant_is_active(grant, now_unix_ms));
    grants.grants.len() != before
}

fn active_grants(grants: &BrokerGrantFile, now_unix_ms: u64) -> Vec<BrokerGrant> {
    grants
        .grants
        .iter()
        .filter(|grant| grant_is_active(grant, now_unix_ms))
        .cloned()
        .collect()
}

/// One identity resolves to AT MOST one row (ADR 0059): creation upserts under
/// the grants lock, so there is nothing left to union here.
fn active_grants_for_identity(
    grants: &BrokerGrantFile,
    identity: &BrokerClientIdentity,
    now_unix_ms: u64,
) -> Option<BrokerGrant> {
    grants
        .grants
        .iter()
        .find(|grant| {
            grant_is_active(grant, now_unix_ms)
                && grant
                    .normalized_label
                    .eq_ignore_ascii_case(&identity.normalized_label)
        })
        .cloned()
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

/// `None` = All Retained History, i.e. no lower bound at all.
fn effective_scope_start(grant: &BrokerGrant, now_unix_ms: u64) -> Option<u64> {
    match grant.scope {
        BrokerGrantScope::AllRetainedHistory => None,
        BrokerGrantScope::RecentDays { days } => {
            Some(now_unix_ms.saturating_sub(u64::from(days).saturating_mul(24 * 60 * 60 * 1000)))
        }
    }
}

fn scope_class(grant: &BrokerGrant) -> String {
    match grant.scope {
        BrokerGrantScope::AllRetainedHistory => "all_retained_history".to_string(),
        BrokerGrantScope::RecentDays { .. } => "time_scoped".to_string(),
    }
}

/// A scoped date range plus whether the permission cut it short.
///
/// Clamping and returning plain success is the failure this type exists to stop:
/// an agent that asked for two weeks, got one day, and reported "nothing there"
/// is a confidently incomplete answer, which for a recall product is the most
/// damaging failure mode there is.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ScopedRange {
    refinement: Option<SearchDateRangeRefinement>,
    /// `Some(scope)` = the requested window reached further back than the
    /// permission allows, and this is the narrowest scope that would have covered
    /// it. `None` = nothing was narrowed.
    clamped_to_scope: Option<BrokerGrantScope>,
}

fn scoped_date_range(
    grant: &BrokerGrant,
    from: Option<String>,
    to: Option<String>,
) -> Result<ScopedRange> {
    let now = now_unix_ms();
    let scope_start = effective_scope_start(grant, now);
    if scope_start.is_none() && from.is_none() && to.is_none() {
        return Ok(ScopedRange {
            refinement: None,
            clamped_to_scope: None,
        });
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
    // Clamped only when the caller ASKED for something older than the permission
    // reaches. A bare request with no `from` is not a narrowed request — it never
    // named a window to narrow, and an overshoot within [`CLAMP_SLACK_MS`] is the
    // caller's clock, not a window they were denied.
    let clamped_to_scope = requested_start
        .filter(|_| scope_start.is_some())
        .map(|requested| (requested.unix_timestamp_nanos() / 1_000_000).max(0) as u64)
        .filter(|requested_ms| default_start.saturating_sub(*requested_ms) > CLAMP_SLACK_MS)
        .map(|requested_ms| minimum_scope_for_start(requested_ms, now));
    let end_dt = requested_end.unwrap_or(now_dt).min(now_dt);
    if end_dt < start_dt {
        return Err(AppInfraError::InvalidSearchRequest(
            "requested broker time range is outside the grant scope".to_string(),
        ));
    }
    // Normalize both bounds to UTC BEFORE they become strings. `parse_rfc3339`
    // keeps whatever offset the caller sent, and `Rfc3339` formatting preserves
    // it — but capture rows are stored as RFC3339-with-`Z`, and the audio-segment
    // overlap predicate (`audio_segments::list_overlapping_range`, mirrored in
    // `speakers::speaker_matched_segments_in_range`) compares those strings
    // LEXICOGRAPHICALLY. An offset-carrying bound like `2026-08-15T00:00:00+05:30`
    // then sorts as if that wall clock were UTC and silently drops every row on
    // the other side of the UTC date boundary — for a `+05:30` caller, the first
    // 5.5 hours of their local day. The frame timeline's `julianday()` compare was
    // always offset-correct, so the two halves of one `timeline` call disagreed.
    // Normalizing here fixes both call sites at once and keeps the predicates
    // index-friendly (a `julianday(column)` compare could not use an index).
    let start_dt = start_dt.to_offset(UtcOffset::UTC);
    let end_dt = end_dt.to_offset(UtcOffset::UTC);
    Ok(ScopedRange {
        refinement: Some(SearchDateRangeRefinement {
            start_at: start_dt
                .format(&Rfc3339)
                .unwrap_or_else(|_| format_unix_ms(default_start)),
            end_at: end_dt
                .format(&Rfc3339)
                .unwrap_or_else(|_| format_unix_ms(now)),
            origin: Some(SearchDateRangeOrigin::VisibleTimeline),
        }),
        clamped_to_scope,
    })
}

fn broker_search_refinements(
    grant: &BrokerGrant,
    from: Option<String>,
    to: Option<String>,
    app: Option<String>,
    window_title: Option<String>,
    url: Option<String>,
    url_regex: Option<String>,
    speaker: Option<SearchSpeakerRefinement>,
) -> Result<(SearchCaptureRefinements, Option<BrokerGrantScope>)> {
    let range = scoped_date_range(grant, from, to)?;
    Ok((
        SearchCaptureRefinements {
            date_range: range.refinement,
            apps: broker_app_refinement(app)?.into_iter().collect(),
            window_title: broker_optional_filter(window_title, "windowTitle")?,
            url: broker_optional_filter(url, "url")?,
            url_regex: broker_url_regex_filter(url_regex)?,
            audio_sources: Vec::new(),
            screen_source: false,
            speaker,
        },
        range.clamped_to_scope,
    ))
}

/// A grant is a TIME BOX, and `from`/`to` are clamped to it — but the QUERY
/// STRING carries its own `date:`/`after:`/`before:` operators, and those
/// OVERWRITE the range the broker derived from the grant (last-write-wins inside
/// `search_capture`). `before:2021-01-01` on a one-day grant therefore reads
/// audio from years the caller was never granted, snippet and — since the speaker
/// filter — that person's verbatim turns with it.
///
/// Refused rather than clamped: the broker publishes `from`/`to` for exactly this
/// question and clamps THOSE to the grant, so nothing is lost but the unclamped
/// spelling of it.
fn query_date_operator_conflict() -> AppInfraError {
    AppInfraError::InvalidSearchRequest(
        "date:, after:, and before: are not accepted inside a brokered query: the grant sets \
         the time window — use the `from` and `to` parameters, which are clamped to it"
            .to_string(),
    )
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

fn timestamp_within_scope(grant: &BrokerGrant, timestamp: &str) -> Result<bool> {
    let Some(scope_start) = effective_scope_start(grant, now_unix_ms()) else {
        return Ok(true);
    };
    let timestamp = parse_rfc3339(timestamp)?;
    let start = OffsetDateTime::from_unix_timestamp_nanos(i128::from(scope_start) * 1_000_000)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH);
    Ok(timestamp >= start)
}

fn range_overlaps_scope(grant: &BrokerGrant, started_at: &str, ended_at: &str) -> Result<bool> {
    let Some(scope_start) = effective_scope_start(grant, now_unix_ms()) else {
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
        |identity| {
            usize::from(active_grants_for_identity(&grants, identity, now_unix_ms()).is_some())
        },
    );
    if active_count == 0 {
        Ok(BrokerAuthStatus::authorization_required())
    } else {
        Ok(BrokerAuthStatus::authorized(active_count))
    }
}

fn random_grant_id() -> String {
    let mut bytes = [0_u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The one way a CLI Access permission is written.
///
/// UPSERT, never append: at most one row per normalized identity. An existing row
/// is mutated IN PLACE and **keeps its id** — opaque result ids are MAC-signed
/// against the issuing grant id, so re-minting it on a widen would fail
/// re-authorization for every id already handed to a running agent, mid-task.
///
/// An approval NEVER clears `blocked`. The channel short-circuits a blocked
/// client before the window opens, but that check runs at connect time and the
/// approval arrives whenever the user acts — so a block applied while the window
/// was open would otherwise be reverted by the Allow that window then sends. A
/// blocked row comes back unchanged (`created: false`, `grant.blocked` still
/// true) and the caller refuses on it; only Settings lifts a block.
fn upsert_grant_for_identity(
    config_dir: &Path,
    identity: BrokerClientIdentity,
    scope: BrokerGrantScope,
) -> Result<BrokerGrantUpsert> {
    with_grants_lock(config_dir, |grants| {
        let now = now_unix_ms();
        let existing = grants.grants.iter_mut().find(|grant| {
            grant
                .normalized_label
                .eq_ignore_ascii_case(&identity.normalized_label)
        });
        let upsert = match existing {
            // A standing block set AFTER the approval window opened. The channel
            // checks for one when the socket CONNECTS, so the whole life of the
            // consent window sits between that check and this write: the row can
            // have been blocked in Settings in the meantime. The block is the
            // newer decision, so it wins and the row is handed back untouched
            // for the caller to refuse on.
            Some(grant) if grant.blocked => BrokerGrantUpsert {
                grant: grant.clone(),
                created: false,
            },
            Some(grant) => {
                grant.label = identity.label;
                grant.normalized_label = identity.normalized_label;
                grant.identity_source = identity.source;
                grant.scope = scope;
                grant.last_used_at_unix_ms = now;
                grant.blocked = false;
                grant.blocked_at_unix_ms = None;
                BrokerGrantUpsert {
                    grant: grant.clone(),
                    created: false,
                }
            }
            None => {
                let grant = BrokerGrant {
                    id: random_grant_id(),
                    label: identity.label,
                    normalized_label: identity.normalized_label,
                    identity_source: identity.source,
                    created_at_unix_ms: now,
                    last_used_at_unix_ms: now,
                    scope,
                    blocked: false,
                    blocked_at_unix_ms: None,
                };
                grants.grants.push(grant.clone());
                BrokerGrantUpsert {
                    grant,
                    created: true,
                }
            }
        };
        if !upsert.grant.blocked {
            save_grants_locked(config_dir, grants)?;
        }
        Ok(upsert)
    })
}

/// Stamp a permission's last use, but only when the stored value is more than
/// [`STAMP_INTERVAL_MS`] stale.
///
/// The shared read comes FIRST and the common call returns from it without ever
/// opening the exclusive lock: a brokered read stays a read (ADR 0041), and
/// one-hour resolution is plenty against a 30-day idle threshold.
pub fn touch_last_used(config_dir: &Path, normalized_label: &str) -> Result<()> {
    let now = now_unix_ms();
    let stale = |grant: &BrokerGrant| {
        grant
            .normalized_label
            .eq_ignore_ascii_case(normalized_label)
            && now.saturating_sub(grant.last_used_at_unix_ms) > STAMP_INTERVAL_MS
    };
    if !load_grants(config_dir)?.grants.iter().any(stale) {
        return Ok(());
    }
    with_grants_lock(config_dir, |grants| {
        let mut changed = false;
        for grant in grants.grants.iter_mut().filter(|grant| stale(grant)) {
            grant.last_used_at_unix_ms = now;
            changed = true;
        }
        if changed {
            save_grants_locked(config_dir, grants)?;
        }
        Ok(())
    })
}

/// Block a client: a standing rejection, denied without prompting and never
/// idle-expired. Deleting the row instead would re-prompt the next time the tool
/// ran, which is not a rejection.
fn block_client(config_dir: &Path, client_label: &str) -> Result<bool> {
    set_client_blocked(config_dir, client_label, true)
}

fn unblock_client(config_dir: &Path, client_label: &str) -> Result<bool> {
    set_client_blocked(config_dir, client_label, false)
}

fn set_client_blocked(config_dir: &Path, client_label: &str, blocked: bool) -> Result<bool> {
    let Some(normalized_label) = normalize_client_label(client_label) else {
        return Ok(false);
    };
    with_grants_lock(config_dir, |grants| {
        let now = now_unix_ms();
        let mut changed = false;
        for grant in grants.grants.iter_mut().filter(|grant| {
            grant.blocked != blocked
                && grant
                    .normalized_label
                    .eq_ignore_ascii_case(&normalized_label)
        }) {
            grant.blocked = blocked;
            grant.blocked_at_unix_ms = blocked.then_some(now);
            // Un-blocking restarts the idle clock; the row may have sat blocked
            // for longer than the idle threshold and would otherwise be pruned
            // out from under the click that re-enabled it.
            if !blocked {
                grant.last_used_at_unix_ms = now;
            }
            changed = true;
        }
        if changed {
            save_grants_locked(config_dir, grants)?;
        }
        Ok(changed)
    })
}

/// The narrowest scope that would cover a request reaching back to
/// `start_unix_ms`. Shared so the broker's clamp marker and the CLI's
/// `needed_scope_for` cannot disagree about what a `--from` requires.
pub fn minimum_scope_for_start(start_unix_ms: u64, now_unix_ms: u64) -> BrokerGrantScope {
    let age_ms = now_unix_ms.saturating_sub(start_unix_ms);
    if age_ms > 7 * 24 * 60 * 60 * 1000 {
        BrokerGrantScope::AllRetainedHistory
    } else if age_ms > 24 * 60 * 60 * 1000 {
        BrokerGrantScope::LAST_7_DAYS
    } else {
        BrokerGrantScope::LAST_DAY
    }
}

fn ask_ai_all_retained_grant(identity: &BrokerClientIdentity) -> BrokerGrant {
    BrokerGrant {
        id: ASK_AI_BROKER_GRANT_ID.to_string(),
        label: identity.label.clone(),
        normalized_label: identity.normalized_label.clone(),
        identity_source: identity.source.clone(),
        created_at_unix_ms: 0,
        // Synthetic and in-memory only: Ask AI is authorized by the Ask AI
        // Setting at the Tauri layer, so this row NEVER reaches the permission
        // file and must never be idle-expired out from under a live turn.
        last_used_at_unix_ms: now_unix_ms(),
        scope: BrokerGrantScope::AllRetainedHistory,
        blocked: false,
        blocked_at_unix_ms: None,
    }
}

async fn broker_search(
    config_dir: &Path,
    infra: &AppInfra,
    grants: &[BrokerGrant],
    request: BrokerSearchRequest,
) -> Result<std::result::Result<BrokerSearchResponse, BrokerErrorResponse>> {
    let Some(grant) = grants.first() else {
        return Ok(Err(BrokerErrorResponse::authorization_required()));
    };
    if crate::search::query_carries_date_operator(&request.query) {
        return Err(query_date_operator_conflict());
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
            || request.url_regex.is_some()
            // Spelled as a query operator it is the SAME filter: `app:`/
            // `source:screen` merge into the refinements below, and the pair
            // would come back as an empty page instead of this refusal.
            || crate::search::query_carries_screen_filter(&request.query))
    {
        return Err(speaker_screen_filter_conflict());
    }
    let (refinements, clamped_to_scope) = broker_search_refinements(
        grant,
        request.from,
        request.to,
        request.app,
        request.window_title,
        request.url,
        request.url_regex,
        speaker.clone(),
    )?;
    let range = refinements.date_range.clone();
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
    // Only a FILTERED request pays for turns: the filter already decided whose
    // audio this is, so returning the matched rows saves an agent a `show-text`
    // per result. Unfiltered, there is no speaker to return words for and no
    // second query runs — which is why this is not an N+1 by another name.
    let (matched_turns, speaker_coverage) = match speaker.as_ref() {
        Some(speaker) => {
            let audio_segment_ids: Vec<i64> = response
                .audio
                .iter()
                .map(|result| result.audio_segment.id)
                .collect();
            (
                speaker_matched_turns_for_segments(infra, speaker, &audio_segment_ids).await?,
                Some(speaker_coverage(infra, speaker, range.as_ref()).await?),
            )
        }
        None => (HashMap::new(), None),
    };
    let mut mapped = map_search_response(
        response,
        limit,
        cursor,
        Some(grant.id.as_str()),
        &opaque_secret,
        matched_turns,
    );
    mapped.speaker_coverage = speaker_coverage;
    mapped.scope_clamped = clamped_to_scope.is_some();
    mapped.required_scope = clamped_to_scope.map(|scope| scope.wire_name().to_string());
    Ok(Ok(mapped))
}

async fn broker_show_text(
    config_dir: &Path,
    infra: &AppInfra,
    grants: &[BrokerGrant],
    opaque_id: &str,
) -> Result<std::result::Result<BrokerShowTextResponse, BrokerErrorResponse>> {
    let Some(grant) = grants.first() else {
        return Ok(Err(BrokerErrorResponse::authorization_required()));
    };
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
            grant,
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
    grant: &BrokerGrant,
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
            timestamp_within_scope(grant, &frame.captured_at)?
        }
        AUDIO_SEGMENT_SUBJECT_TYPE => {
            let Some(audio) = infra.get_audio_segment(reuse.source_subject_id).await? else {
                return Ok(None);
            };
            range_overlaps_scope(grant, &audio.started_at, &audio.ended_at)?
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
    // The ISSUING row, not the caller's set: an id is signed against exactly one
    // grant id, and that row's scope is the only one allowed to authorize it.
    let Some(issuing_grant) = grants.iter().find(|grant| grant.id == grant_id) else {
        return Ok(Err(outside_scope_error()));
    };
    let in_scope = match reference.kind.as_str() {
        "frame" => {
            let Some(frame) = infra
                .get_frame(reference.frame_id.expect("frame reference has id"))
                .await?
            else {
                return Ok(Err(outside_scope_error()));
            };
            timestamp_within_scope(issuing_grant, &frame.captured_at)?
        }
        "audio" => {
            let Some(audio) = infra
                .get_audio_segment(reference.audio_segment_id.expect("audio reference has id"))
                .await?
            else {
                return Ok(Err(outside_scope_error()));
            };
            range_overlaps_scope(issuing_grant, &audio.started_at, &audio.ended_at)?
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
    let Some(grant) = grants.first() else {
        return Ok(Err(BrokerErrorResponse::authorization_required()));
    };
    let limit = request
        .limit
        .unwrap_or(DEFAULT_SEARCH_LIMIT)
        .min(MAX_SEARCH_LIMIT);
    let scoped = scoped_date_range(grant, Some(request.from), Some(request.to))?;
    let clamped_to_scope = scoped.clamped_to_scope;
    let range = scoped
        .refinement
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
    let opaque_grant_id = Some(grant.id.as_str());
    let mark_clamp = |mut page: BrokerTimelineResponse| {
        page.scope_clamped = clamped_to_scope.is_some();
        page.required_scope = clamped_to_scope.map(|scope| scope.wire_name().to_string());
        page
    };
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
        // Unreachable with a speaker filter: that combination errors above.
        return Ok(Ok(mark_clamp(BrokerTimelineResponse::page(
            intervals, limit, None,
        ))));
    }
    // The mirror of the context-filter branch above: a speaker filter narrows to
    // audio, because a captured frame carries no voice to match against. The one
    // query yields both the matched recordings and what was said in them.
    let (speaker_page, speaker_matched, speaker_coverage) = match speaker.as_ref() {
        Some(speaker) => {
            let (page, matched) =
                speaker_matched_recordings_in_range(infra, speaker, &range, limit).await?;
            (
                Some(page),
                Some(matched),
                Some(speaker_coverage(infra, speaker, Some(&range)).await?),
            )
        }
        None => (None, None, None),
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
    // The speaker page IS the answer: it already applied `list_overlapping_range`'s
    // overlap predicate, its ordering, AND the cap, so re-reading the window here
    // would materialize every recording in the grant only to throw all but `limit`
    // of them away. Retention defaults to NEVER and segments are capped at five
    // minutes, so "the window" on a months-wide grant is tens of thousands of rows.
    let recordings = match speaker_page {
        Some(page) => page,
        None => {
            infra
                .list_audio_segments_overlapping_range(&range.start_at, &range.end_at, None, None)
                .await?
        }
    };
    for audio in recordings.into_iter().take(limit as usize) {
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
            turns: speaker_matched
                .as_ref()
                .and_then(|matched| matched.get(&audio.id))
                .cloned()
                .unwrap_or_default(),
        });
    }
    intervals.sort_by(|left, right| {
        right
            .started_at
            .cmp(&left.started_at)
            .then_with(|| right.kind.cmp(&left.kind))
    });
    intervals.truncate(limit as usize);
    Ok(Ok(mark_clamp(BrokerTimelineResponse::page(
        intervals,
        limit,
        speaker_coverage,
    ))))
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
            // A frame interval carries no voice — a speaker filter never reaches
            // this branch (it errors on the context filters that do).
            turns: Vec::new(),
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

/// Runaway guard on the chronological activities door. A day of 2–30 minute
/// derivation windows is tens of episodes, so this is far above any real day and
/// exists only so a year-wide window cannot materialize the whole dossier.
const MAX_ACTIVITIES: usize = 500;

/// `activities`: the derived episodes overlapping a window, oldest-first.
///
/// This is the day-scale door the other tools could not be. `timeline` has the
/// window but not the altitude (its intervals are per-frame rows, newest-first,
/// capped — a full page is the tail of the window, not the window); and
/// `recall_context` has the altitude but is keyword-filtered and hard-capped,
/// because it answers "what do you know about me", not "what happened between
/// these two times". Neither can walk a day.
///
/// Three properties are load-bearing:
///
/// - **Uncapped within the window** (short of [`MAX_ACTIVITIES`]) and in
///   chronological order, because a relevance-ranked top-N is what made the
///   existing door unable to answer this.
/// - **Guardrailed.** An Activity's `title`/`summary` is persisted UNFILTERED, so
///   `guardrail::is_sensitive` here is the only thing standing between a
///   sensitive episode and a cloud engine — exactly as in
///   `select_relevant_activities`, and load-bearing for the same reason. Do not
///   drop it as redundant with derivation-time filtering; derivation does not
///   filter these.
/// - **Coverage-reporting.** See [`BrokerActivitiesResponse::derived_from`]: an
///   empty list from an underived window must never read as an empty day.
async fn broker_activities(
    config_dir: &Path,
    infra: &AppInfra,
    grants: &[BrokerGrant],
    request: BrokerActivitiesRequest,
) -> Result<std::result::Result<BrokerActivitiesResponse, BrokerErrorResponse>> {
    let Some(grant) = grants.first() else {
        return Ok(Err(BrokerErrorResponse::authorization_required()));
    };
    // Same clamp every dated tool uses: the caller can never widen past the
    // grant's own scope, and bounds come back UTC-normalized.
    // ponytail: no `scopeClamped` marker here — `activities` already carries
    // `derivedFrom`/`derivedUntil` for "this list does not cover your window",
    // and ADR 0059 scopes the marker to search + timeline. Add it if the CLI
    // grows an `activities --from` that reaches past a permission.
    let range = scoped_date_range(grant, Some(request.from), Some(request.to))?
        .refinement
        .expect("activities always supplies a scoped date range");
    let (Some(range_start_ms), Some(range_end_ms)) = (
        recall_bound_to_unix_ms(Some(&range.start_at)),
        recall_bound_to_unix_ms(Some(&range.end_at)),
    ) else {
        // `scoped_date_range` just formatted these from parsed instants, so a
        // failure here is not a caller error to report back.
        return Err(AppInfraError::InvalidSearchRequest(
            "activities range could not be converted to unix milliseconds".to_string(),
        ));
    };

    let store = infra.user_context();
    let activities = store
        .list_activities_in_range(range_start_ms, range_end_ms)
        .await?;

    // Guardrail BEFORE the cap, so a sensitive episode cannot consume a slot and
    // silently push a reportable one past the limit.
    let mut activities: Vec<capture_types::Activity> = activities
        .into_iter()
        .filter(|a| !crate::user_context::guardrail::is_sensitive(&a.title, &a.summary))
        .collect();
    let truncated = activities.len() > MAX_ACTIVITIES;
    activities.truncate(MAX_ACTIVITIES);

    // Two batched lookups for the whole page, never one per activity: the
    // headline frame ids, then those frames' metadata in a single IN-query (the
    // same N+1 the timeline's snapshot read exists to avoid).
    let activity_ids: Vec<i64> = activities.iter().map(|a| a.id).collect();
    let headline_frames = store.headline_frames_for_activities(&activity_ids).await?;
    let headline_frame_ids: Vec<i64> = headline_frames.values().copied().collect();
    let snapshots = infra
        .get_frame_metadata_snapshots(&headline_frame_ids)
        .await?;
    let opaque_secret = load_or_create_opaque_secret(config_dir)?;
    let opaque_grant_id = Some(grant.id.as_str());

    let activities = activities
        .into_iter()
        .map(|activity| BrokerActivity {
            // Read-time URL guard, exactly as on the timeline path: only a
            // guarded http(s) host+path survives, everything else guards to
            // `None`, and the raw frame id never crosses the boundary.
            context: headline_frames
                .get(&activity.id)
                .and_then(|frame_id| snapshots.get(frame_id))
                .and_then(|snapshot| {
                    broker_search_result_context(
                        snapshot.app_bundle_id.clone(),
                        snapshot.app_name.clone(),
                        snapshot.window_title.clone(),
                        snapshot
                            .browser_url
                            .as_deref()
                            .and_then(url_guard::guard_url),
                    )
                }),
            opaque_id: headline_frames.get(&activity.id).map(|frame_id| {
                encode_signed_opaque_id("frame", *frame_id, opaque_grant_id, &opaque_secret)
            }),
            title: activity.title,
            summary: activity.summary,
            category: activity.category.as_ref().and_then(snake_case_enum_string),
            focus: activity.focus.as_ref().and_then(snake_case_enum_string),
            started_at: format_unix_ms(activity.started_at_ms.max(0) as u64),
            ended_at: format_unix_ms(activity.ended_at_ms.max(0) as u64),
        })
        .collect();

    // Coverage = the requested window intersected with what derivation has
    // actually summarized. `covered_until_ms` excludes failed runs (a failed run
    // summarized nothing), and the oldest windowed run start is the trailing edge
    // backfill has reached. An empty intersection reports both as `None`.
    let coverage_start = store.oldest_derivation_run_window_start().await?;
    let coverage_end = store.covered_until_ms().await?;
    let (derived_from, derived_until) = match (coverage_start, coverage_end) {
        (Some(coverage_start), Some(coverage_end)) => {
            let from = range_start_ms.max(coverage_start);
            let until = range_end_ms.min(coverage_end);
            if from <= until {
                (
                    Some(format_unix_ms(from.max(0) as u64)),
                    Some(format_unix_ms(until.max(0) as u64)),
                )
            } else {
                (None, None)
            }
        }
        _ => (None, None),
    };

    Ok(Ok(BrokerActivitiesResponse {
        activities,
        derived_from,
        derived_until,
        truncated,
    }))
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

/// The opaque id's tag: a secret-prefix MAC, `SHA-256(secret ‖ ':' ‖ payload)`
/// truncated to 128 bits — deliberately NOT HMAC, and here is why that is sound.
///
/// A secret-prefix MAC's one real weakness is length extension: SHA-256's digest
/// IS its final internal state, so anyone holding a tag can continue hashing and
/// produce a valid tag for `payload ‖ padding ‖ suffix` without the secret. Three
/// independent things stop that here, and each is enough on its own:
///
/// 1. **Truncation.** Only 16 of the 32 digest bytes are published, so a forger
///    never learns the state to continue from and would have to guess the other
///    128 bits. This is why the truncation is load-bearing, not cosmetic.
/// 2. **Extension can only APPEND, and the parts that matter are at the front.**
///    A payload is `<kind><hex id>[:g<grant id>]` (see [`encode_signed_opaque_id`]):
///    the kind and the row id are the head, so no appended suffix can retarget the
///    id at a different frame or audio segment.
/// 3. **The grant id runs to the end of the payload, and the FIRST `:g` wins**
///    ([`decode_opaque_payload`]). Appending `:g<other grant>` therefore does not
///    replace the grant id — it lands *inside* it, yielding one long id that
///    matches no row, and [`broker_authorize_opaque_reference`] refuses on it. So
///    an extension cannot borrow a wider permission's scope either.
///
/// The comparison in [`opaque_signature_matches`] is constant-time over equal
/// lengths, so the tag cannot be walked out byte by byte either.
///
/// ponytail: hand-rolled secret-prefix MAC, sound only for this fixed-shape,
/// head-anchored payload. Anything with attacker-chosen or trailing-significant
/// fields — or a full-width tag — needs real HMAC-SHA256 first (`hmac` is not a
/// dependency; the crate already has `sha2`, so a 30-line HMAC or the crate, either
/// way). Do not copy this construction to a new payload without re-checking 2 and 3.
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
    write_owner_only(&path, &secret)?;
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

/// `matched_turns` is what the FILTERED speaker said, keyed by audio segment —
/// empty for every unfiltered request, which is what keeps `turns` off results
/// nobody asked about a person for.
fn map_search_response(
    response: SearchCaptureResponse,
    limit: u32,
    cursor: Option<BrokerSearchCursor>,
    grant_id: Option<&str>,
    opaque_secret: &[u8],
    matched_turns: HashMap<i64, Vec<BrokerSpeakerTurn>>,
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
                    frame.browser_url.as_deref().and_then(url_guard::guard_url),
                ),
                // Frame results have no sub-segment audio anchor.
                span_start_ms: None,
                span_end_ms: None,
                aligned_frame_id: None,
                // A frame carries no voice, so a speaker filter never returns one.
                turns: Vec::new(),
            });
            frames_taken += 1;
        } else {
            let Some(audio_result) = audio.next() else {
                break;
            };
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
                // Cloned, never removed: one recording can answer a query twice
                // (two matched moments past the audio grouping gap), and taking
                // the entry would leave the second result saying she was silent.
                turns: matched_turns
                    .get(&audio_result.audio_segment.id)
                    .cloned()
                    .unwrap_or_default(),
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
        // Filled in by `broker_search`, which knows whether a speaker was filtered
        // on; unfiltered responses carry no counts at all rather than zeroes.
        speaker_coverage: None,
        // Likewise: the grant is what decides these, and it does not reach here.
        scope_clamped: false,
        required_scope: None,
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
