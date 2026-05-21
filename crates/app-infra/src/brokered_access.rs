use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::{
    AppInfra, AppInfraError, AudioSegmentSourceKind, ProcessingSubject, Result,
    SearchCaptureRefinements, SearchCaptureRequest, SearchCaptureResponse, SearchDateRangeOrigin,
    SearchDateRangeRefinement,
};

const BROKER_GRANTS_FILE_NAME: &str = "broker-grants.json";
const BROKER_AUDIT_FILE_NAME: &str = "broker-audit.json";
const RECORDING_SETTINGS_FILE_NAME: &str = "recording-settings.json";
const DEFAULT_SEARCH_LIMIT: u32 = 20;
const MAX_SEARCH_LIMIT: u32 = 100;

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
    pub created_at_unix_ms: u64,
    pub expires_at_unix_ms: u64,
    pub scope: BrokerGrantScope,
    #[serde(default)]
    pub revoked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct BrokerGrantFile {
    pub grants: Vec<BrokerGrant>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct BrokerAuditFile {
    pub events: Vec<BrokerAuditEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerAuditEvent {
    pub tool_identity: String,
    pub command_type: String,
    pub timestamp_unix_ms: u64,
    pub result_count: u32,
    pub scope_class: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerSearchRequest {
    pub query: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerSearchResult {
    pub opaque_id: String,
    pub kind: String,
    pub snippet: String,
    pub started_at: String,
    pub ended_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerSearchResponse {
    pub results: Vec<BrokerSearchResult>,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerShowTextResponse {
    pub opaque_id: String,
    pub kind: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerTimelineRequest {
    pub from: String,
    pub to: String,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerTimelineInterval {
    pub kind: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerTimelineResponse {
    pub intervals: Vec<BrokerTimelineInterval>,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct RecordingSettingsFile {
    save_directory: String,
}

pub fn default_app_config_dir() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("MNEMA_APP_CONFIG_DIR") {
        return Some(PathBuf::from(path));
    }
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir().map(|home| {
            home.join("Library")
                .join("Application Support")
                .join("com.shaikzeeshan.mnema")
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        dirs::config_dir().map(|dir| dir.join("mnema"))
    }
}

pub fn default_save_directory_from_config(config_dir: &Path) -> Result<Option<PathBuf>> {
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

pub fn load_grants(config_dir: &Path) -> Result<BrokerGrantFile> {
    let path = config_dir.join(BROKER_GRANTS_FILE_NAME);
    if !path.exists() {
        return Ok(BrokerGrantFile::default());
    }
    let raw = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

pub fn save_grants(config_dir: &Path, grants: &BrokerGrantFile) -> Result<()> {
    fs::create_dir_all(config_dir)?;
    let path = config_dir.join(BROKER_GRANTS_FILE_NAME);
    let raw = serde_json::to_string_pretty(grants)?;
    fs::write(path, raw)?;
    Ok(())
}

pub fn load_audit_events(config_dir: &Path) -> Result<BrokerAuditFile> {
    let path = config_dir.join(BROKER_AUDIT_FILE_NAME);
    if !path.exists() {
        return Ok(BrokerAuditFile::default());
    }
    let raw = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

pub fn record_audit_event(
    config_dir: &Path,
    tool_identity: impl Into<String>,
    command_type: impl Into<String>,
    result_count: u32,
    scope_class: impl Into<String>,
) -> Result<()> {
    fs::create_dir_all(config_dir)?;
    let mut audit = load_audit_events(config_dir)?;
    audit.events.push(BrokerAuditEvent {
        tool_identity: tool_identity.into(),
        command_type: command_type.into(),
        timestamp_unix_ms: now_unix_ms(),
        result_count,
        scope_class: scope_class.into(),
    });
    if audit.events.len() > 500 {
        let drop_count = audit.events.len().saturating_sub(500);
        audit.events.drain(0..drop_count);
    }
    let path = config_dir.join(BROKER_AUDIT_FILE_NAME);
    fs::write(path, serde_json::to_string_pretty(&audit)?)?;
    Ok(())
}

pub fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

pub fn grant_is_active(grant: &BrokerGrant, now_unix_ms: u64) -> bool {
    !grant.revoked && grant.expires_at_unix_ms > now_unix_ms
}

pub fn active_grants(grants: &BrokerGrantFile, now_unix_ms: u64) -> Vec<BrokerGrant> {
    grants
        .grants
        .iter()
        .filter(|grant| grant_is_active(grant, now_unix_ms))
        .cloned()
        .collect()
}

fn format_unix_ms(unix_ms: u64) -> String {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(unix_ms) * 1_000_000)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn parse_rfc3339(value: &str) -> Result<OffsetDateTime> {
    OffsetDateTime::parse(value, &Rfc3339)
        .map_err(|error| AppInfraError::InvalidSearchRequest(error.to_string()))
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

pub fn scope_class(grants: &[BrokerGrant]) -> String {
    if grants
        .iter()
        .any(|grant| matches!(grant.scope, BrokerGrantScope::AllRetainedHistory))
    {
        "all_retained_history".to_string()
    } else {
        "time_scoped".to_string()
    }
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

fn timestamp_within_scope(grants: &[BrokerGrant], timestamp: &str) -> Result<bool> {
    let Some(scope_start) = effective_scope_start(grants, now_unix_ms()) else {
        return Ok(true);
    };
    let timestamp = parse_rfc3339(timestamp)?;
    let start = OffsetDateTime::from_unix_timestamp_nanos(i128::from(scope_start) * 1_000_000)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH);
    Ok(timestamp >= start)
}

pub fn auth_status_for_config(config_dir: &Path) -> Result<BrokerAuthStatus> {
    let grants = load_grants(config_dir)?;
    let active_count = active_grants(&grants, now_unix_ms()).len();
    if active_count == 0 {
        Ok(BrokerAuthStatus::authorization_required())
    } else {
        Ok(BrokerAuthStatus::authorized(active_count))
    }
}

pub fn create_grant(
    config_dir: &Path,
    label: impl Into<String>,
    duration_hours: u64,
    scope: BrokerGrantScope,
) -> Result<BrokerGrant> {
    let mut grants = load_grants(config_dir)?;
    let now = now_unix_ms();
    let grant = BrokerGrant {
        id: format!("{now:x}-{:x}", grants.grants.len()),
        label: label.into(),
        created_at_unix_ms: now,
        expires_at_unix_ms: now.saturating_add(duration_hours.saturating_mul(60 * 60 * 1000)),
        scope,
        revoked: false,
    };
    grants.grants.push(grant.clone());
    save_grants(config_dir, &grants)?;
    Ok(grant)
}

pub fn revoke_grant(config_dir: &Path, grant_id: &str) -> Result<bool> {
    let mut grants = load_grants(config_dir)?;
    let mut changed = false;
    for grant in &mut grants.grants {
        if grant.id == grant_id && !grant.revoked {
            grant.revoked = true;
            changed = true;
        }
    }
    if changed {
        save_grants(config_dir, &grants)?;
    }
    Ok(changed)
}

pub async fn broker_search(
    infra: &AppInfra,
    grants: &[BrokerGrant],
    request: BrokerSearchRequest,
) -> Result<std::result::Result<BrokerSearchResponse, BrokerErrorResponse>> {
    if grants.is_empty() {
        return Ok(Err(BrokerErrorResponse::authorization_required()));
    }
    let limit = request
        .limit
        .unwrap_or(DEFAULT_SEARCH_LIMIT)
        .min(MAX_SEARCH_LIMIT);
    let date_range = scoped_date_range(grants, request.from, request.to)?;
    let response = infra
        .search_capture(SearchCaptureRequest {
            query: request.query,
            frame_limit: Some(limit),
            frame_offset: Some(0),
            audio_limit: Some(limit),
            audio_offset: Some(0),
            snapshot_document_id: None,
            refinements: Some(SearchCaptureRefinements {
                date_range,
                app: None,
                audio_source: None,
            }),
        })
        .await?;
    Ok(Ok(map_search_response(response, limit)))
}

pub async fn broker_show_text(
    infra: &AppInfra,
    grants: &[BrokerGrant],
    opaque_id: &str,
) -> Result<std::result::Result<BrokerShowTextResponse, BrokerErrorResponse>> {
    if grants.is_empty() {
        return Ok(Err(BrokerErrorResponse::authorization_required()));
    }
    let Some((kind, id)) = decode_opaque_id(opaque_id) else {
        return Ok(Err(BrokerErrorResponse {
            error: BrokerAuthStatusKind::AuthorizationRequired,
            message: "invalid opaque result id".to_string(),
        }));
    };
    let subject = match kind.as_str() {
        "frame" => {
            let Some(frame) = infra.get_frame(id).await? else {
                return Ok(Err(BrokerErrorResponse {
                    error: BrokerAuthStatusKind::AuthorizationRequired,
                    message: "result is unavailable or outside the grant scope".to_string(),
                }));
            };
            if !timestamp_within_scope(grants, &frame.captured_at)? {
                return Ok(Err(BrokerErrorResponse {
                    error: BrokerAuthStatusKind::AuthorizationRequired,
                    message: "result is unavailable or outside the grant scope".to_string(),
                }));
            }
            ProcessingSubject::frame(id)
        }
        "audio" => {
            let Some(audio) = infra.get_audio_segment(id).await? else {
                return Ok(Err(BrokerErrorResponse {
                    error: BrokerAuthStatusKind::AuthorizationRequired,
                    message: "result is unavailable or outside the grant scope".to_string(),
                }));
            };
            if !timestamp_within_scope(grants, &audio.started_at)? {
                return Ok(Err(BrokerErrorResponse {
                    error: BrokerAuthStatusKind::AuthorizationRequired,
                    message: "result is unavailable or outside the grant scope".to_string(),
                }));
            }
            ProcessingSubject::audio_segment(id)
        }
        _ => {
            return Ok(Err(BrokerErrorResponse {
                error: BrokerAuthStatusKind::AuthorizationRequired,
                message: "invalid opaque result id".to_string(),
            }));
        }
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
    let Some(result) = result else {
        return Ok(Err(BrokerErrorResponse {
            error: BrokerAuthStatusKind::AuthorizationRequired,
            message: "result is unavailable or outside the grant scope".to_string(),
        }));
    };
    Ok(Ok(BrokerShowTextResponse {
        opaque_id: opaque_id.to_string(),
        kind,
        text: result.result_text.unwrap_or_default(),
    }))
}

pub async fn broker_timeline(
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
    let mut intervals = Vec::new();
    for audio in infra
        .list_audio_segments_overlapping_range(&range.start_at, &range.end_at, None, None)
        .await?
        .into_iter()
        .take(limit as usize)
    {
        intervals.push(BrokerTimelineInterval {
            kind: match audio.source_kind {
                AudioSegmentSourceKind::Microphone => "audio_microphone".to_string(),
                AudioSegmentSourceKind::SystemAudio => "audio_system".to_string(),
            },
            started_at: audio.started_at,
            ended_at: Some(audio.ended_at),
            reason: None,
        });
    }
    intervals.truncate(limit as usize);
    Ok(Ok(BrokerTimelineResponse { intervals, limit }))
}

pub fn encode_opaque_id(kind: &str, id: i64) -> String {
    let tag = match kind {
        "frame" => "f",
        "audio" => "a",
        _ => "x",
    };
    format!("{tag}{:x}", id.max(0))
}

pub fn decode_opaque_id(value: &str) -> Option<(String, i64)> {
    let (kind, rest) = value.split_at(1);
    let id = i64::from_str_radix(rest, 16).ok()?;
    let kind = match kind {
        "f" => "frame",
        "a" => "audio",
        _ => return None,
    };
    Some((kind.to_string(), id))
}

fn map_search_response(response: SearchCaptureResponse, limit: u32) -> BrokerSearchResponse {
    let mut results = Vec::new();
    for frame in response.frames {
        results.push(BrokerSearchResult {
            opaque_id: encode_opaque_id("frame", frame.representative_frame.id),
            kind: "frame".to_string(),
            snippet: frame.snippet,
            started_at: frame.group_start_at,
            ended_at: frame.group_end_at,
        });
    }
    for audio in response.audio {
        results.push(BrokerSearchResult {
            opaque_id: encode_opaque_id("audio", audio.audio_segment.id),
            kind: "audio".to_string(),
            snippet: audio.snippet,
            started_at: audio.absolute_start_at,
            ended_at: audio.absolute_end_at,
        });
    }
    results.truncate(limit as usize);
    BrokerSearchResponse { results, limit }
}
