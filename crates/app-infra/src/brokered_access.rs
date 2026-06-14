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
    AUDIO_SEGMENT_SUBJECT_TYPE, FRAME_SUBJECT_TYPE,
};

const BROKER_GRANTS_FILE_NAME: &str = "broker-grants.json";
const BROKER_GRANTS_LOCK_FILE_NAME: &str = "broker-grants.lock";
const BROKER_AUDIT_LOCK_FILE_NAME: &str = "broker-audit.lock";
const BROKER_AUDIT_FILE_NAME: &str = "broker-audit.json";
const BROKER_OPAQUE_SECRET_FILE_NAME: &str = "broker-opaque-secret.bin";
const RECORDING_SETTINGS_FILE_NAME: &str = "recording-settings.json";
const DEFAULT_SEARCH_LIMIT: u32 = 20;
const MAX_SEARCH_LIMIT: u32 = 100;
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
pub struct BrokerOpenInMnemaResponse {
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerTimelineInterval {
    pub kind: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<BrokerSearchResultContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrokerTimelineResponse {
    pub intervals: Vec<BrokerTimelineInterval>,
    pub limit: u32,
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
    RecallContext(BrokerRecallContextRequest),
    OpenInMnema { opaque_id: String },
}

impl BrokeredCaptureRequest {
    fn command_type(&self) -> Option<&'static str> {
        match self {
            Self::AuthStatus => None,
            Self::Search(_) => Some("search"),
            Self::ShowText { .. } => Some("show_text"),
            Self::Timeline(_) => Some("timeline"),
            Self::RecallContext(_) => Some("recall_context"),
            Self::OpenInMnema { .. } => Some("open_in_mnema"),
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
    RecallContext(BrokerRecallContextResponse),
    OpenInMnema(BrokerOpenInMnemaResponse),
    Error(BrokerErrorResponse),
}

impl BrokeredCaptureResponse {
    fn result_count(&self) -> u32 {
        match self {
            Self::Search(response) => response.results.len() as u32,
            Self::ShowText(_) | Self::OpenInMnema(_) => 1,
            Self::Timeline(response) => response.intervals.len() as u32,
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
    pub async fn execute_for_ask_ai(
        &self,
        identity: BrokerClientIdentity,
        request: BrokeredCaptureRequest,
    ) -> Result<BrokeredCaptureResponse> {
        if matches!(request, BrokeredCaptureRequest::OpenInMnema { .. }) {
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
                match broker_timeline(&infra, grants, request).await? {
                    Ok(response) => Ok(BrokeredCaptureResponse::Timeline(response)),
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
) -> Result<SearchCaptureRefinements> {
    Ok(SearchCaptureRefinements {
        date_range: scoped_date_range(grants, from, to)?,
        apps: broker_app_refinement(app)?.into_iter().collect(),
        window_title: broker_optional_filter(window_title, "windowTitle")?,
        audio_sources: Vec::new(),
        screen_source: false,
    })
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
    let limit = request
        .limit
        .unwrap_or(DEFAULT_SEARCH_LIMIT)
        .min(MAX_SEARCH_LIMIT);
    let refinements = broker_search_refinements(
        grants,
        request.from,
        request.to,
        request.app,
        request.window_title,
    )?;
    let response = infra
        .search_capture(SearchCaptureRequest {
            query: request.query,
            frame_limit: Some(limit),
            frame_offset: Some(0),
            audio_limit: Some(limit),
            audio_offset: Some(0),
            snapshot_document_id: None,
            refinements: Some(refinements),
        })
        .await?;
    let opaque_secret = load_or_create_opaque_secret(config_dir)?;
    Ok(Ok(map_search_response(
        response,
        limit,
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
    Ok(Ok(BrokerShowTextResponse {
        opaque_id: opaque_id.to_string(),
        kind: reference.kind,
        text,
    }))
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
    if app.is_some() || window_title.is_some() {
        let intervals =
            broker_frame_timeline(infra, &range, app.as_ref(), window_title.as_deref(), limit)
                .await?;
        return Ok(Ok(BrokerTimelineResponse { intervals, limit }));
    }
    let mut intervals = broker_frame_timeline(infra, &range, None, None, limit).await?;
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
    limit: u32,
) -> Result<Vec<BrokerTimelineInterval>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT group_key, app_bundle_id, app_name, window_title, \
                MIN(absolute_start_at) AS started_at, MAX(absolute_end_at) AS ended_at, MAX(id) AS sort_id \
         FROM search_documents \
         WHERE anchor_type = 'frame' \
           AND julianday(absolute_end_at) >= julianday(",
    );
    query.push_bind(range.start_at.clone());
    query.push(") AND julianday(absolute_start_at) <= julianday(");
    query.push_bind(range.end_at.clone());
    query.push(")");
    push_broker_timeline_context_filters(&mut query, app, window_title);
    query.push(
        " GROUP BY group_key, app_bundle_id, app_name, window_title \
          ORDER BY started_at DESC, sort_id DESC LIMIT ",
    );
    query.push_bind(limit as i64);

    let rows = query.build().fetch_all(infra.pool()).await?;
    rows.into_iter()
        .map(|row| {
            let app_bundle_id: Option<String> = row.get("app_bundle_id");
            let app_name: Option<String> = row.get("app_name");
            let window_title: Option<String> = row.get("window_title");
            Ok(BrokerTimelineInterval {
                kind: "screen".to_string(),
                started_at: row.get("started_at"),
                ended_at: Some(row.get("ended_at")),
                reason: None,
                context: broker_search_result_context(app_bundle_id, app_name, window_title),
            })
        })
        .collect()
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

/// Trivial stopwords dropped from the question before token-overlap scoring.
const RECALL_STOPWORDS: &[&str] = &[
    "the", "and", "for", "are", "was", "were", "that", "this", "with", "what", "when", "where",
    "who", "why", "how", "did", "does", "have", "has", "had", "you", "your", "they", "them",
    "from", "about", "into", "over", "been", "being", "she", "her", "his", "him", "their", "our",
    "can", "could", "would", "should", "will", "shall", "may", "might", "any", "all", "some",
];

/// Lowercase, tokenize the query into words (length >= 3, punctuation stripped),
/// dropping trivial stopwords, then **stem** each survivor ([`recall_stem`]) so
/// the matcher is morphology-insensitive ("running" ~ "run"). Empty when the
/// query has no usable tokens. Tokens are de-duplicated so a repeated query word
/// cannot inflate the overlap score.
fn recall_query_tokens(query: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    for word in query.split(|ch: char| !ch.is_alphanumeric()) {
        let word = word.to_lowercase();
        if word.len() < 3 || RECALL_STOPWORDS.contains(&word.as_str()) {
            continue;
        }
        let stemmed = recall_stem(&word);
        if !tokens.contains(&stemmed) {
            tokens.push(stemmed);
        }
    }
    tokens
}

/// Cheap, hand-rolled English suffix stripper (#3) — NOT a real stemmer, just a
/// lexical-gap reducer applied identically to query tokens and corpus words so
/// "running"~"run", "coding"~"code", "tests"~"test", "quickly"~"quick" collapse
/// to a shared key. It does NOT try to produce a real dictionary stem; it only
/// has to be *consistent*, so a query word and the corpus word it should match
/// land on the same key.
///
/// Three passes: (1) strip one common suffix (`-ing`, `-edly`, `-ied`, `-ed`,
/// `-ly`, `-ies`, `-es`, `-s`); (2) collapse a doubled final consonant
/// ("runn" -> "run", "stopp" -> "stop") so the `-ing`/`-ed` doubling rule is
/// undone; (3) drop a single silent terminal `e` from whatever remains so the
/// un-suffixed form lines up with the suffixed one ("code" -> "cod" matches
/// "coding" -> "cod"). Guards against over-stemming: a suffix is only stripped
/// when a reasonable stem (>= 3 chars) remains, so short words like
/// "is"/"red"/"bus"/"ring" are left intact. No allocation when nothing changes.
fn recall_stem(word: &str) -> String {
    // Each rule: (suffix, min length of the FULL word to apply). Longer suffixes
    // first so `-ing` wins over `-s`. The min-length guards keep very short words
    // from being gutted.
    const RULES: &[(&str, usize)] = &[
        ("ing", 6),
        ("edly", 7),
        ("ied", 5),
        ("ed", 5),
        ("ly", 5),
        ("ies", 5),
        ("es", 5),
        ("s", 4),
    ];

    // Pass 1: strip the first matching suffix (if a >= 3-char stem survives).
    let mut stem = word;
    for (suffix, min_len) in RULES {
        if word.len() >= *min_len && word.ends_with(suffix) {
            let candidate = &word[..word.len() - suffix.len()];
            if candidate.len() >= 3 {
                stem = candidate;
                break;
            }
        }
    }

    let bytes = stem.as_bytes();
    let mut end = bytes.len();

    // Pass 2: collapse a doubled final consonant ("runn" -> "run").
    if end >= 2 {
        let last = bytes[end - 1];
        let prev = bytes[end - 2];
        let is_consonant = last.is_ascii_alphabetic() && !b"aeiou".contains(&last);
        if last == prev && is_consonant && end - 1 >= 3 {
            end -= 1;
        }
    }

    // Pass 3: drop a single silent terminal `e` ("code" -> "cod") so the
    // un-suffixed form matches the suffixed one. Keep >= 3 chars.
    if end >= 4 && bytes[end - 1] == b'e' {
        end -= 1;
    }

    stem[..end].to_string()
}

/// Split `text` into lowercased, stemmed whole-word keys (length >= 3), the same
/// normalization [`recall_query_tokens`] applies to the query so the two sides
/// compare like-for-like. Used to build per-document word sets for whole-word
/// (#1) matching and IDF (#2) document-frequency.
fn recall_doc_words(text: &str) -> std::collections::HashSet<String> {
    text.split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| word.len() >= 3)
        .map(|word| recall_stem(&word.to_lowercase()))
        .collect()
}

/// IDF-style weight for a token matching `df` of `n` candidate documents: rarer
/// tokens (low `df`) outweigh common ones. `ln((N+1)/(df+1)) + 1`, always
/// positive so any match still counts. (#2)
fn recall_idf_weight(n: usize, df: usize) -> f64 {
    (((n as f64 + 1.0) / (df as f64 + 1.0)).ln()) + 1.0
}

/// Build a token -> document-frequency map over the candidate `docs` (each a
/// pre-split whole-word set), counting only tokens that are actual query tokens.
/// (#2)
fn recall_document_frequencies(
    tokens: &[String],
    docs: &[std::collections::HashSet<String>],
) -> std::collections::HashMap<String, usize> {
    let mut df: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for token in tokens {
        let count = docs.iter().filter(|words| words.contains(token)).count();
        df.insert(token.clone(), count);
    }
    df
}

/// Whole-word (#1), rare-token-weighted (#2) relevance score of `tokens` against
/// a document's pre-split whole-word set `doc_words`: sums the IDF weight of each
/// query token present as a full (stemmed) word. Substring hits no longer count —
/// "cat" matches "cat" but not "category". Returns `0.0` when nothing matches.
fn recall_overlap_score(
    tokens: &[String],
    doc_words: &std::collections::HashSet<String>,
    df: &std::collections::HashMap<String, usize>,
    n: usize,
) -> f64 {
    if tokens.is_empty() {
        return 0.0;
    }
    tokens
        .iter()
        .filter(|token| doc_words.contains(*token))
        .map(|token| recall_idf_weight(n, df.get(token).copied().unwrap_or(0)))
        .sum()
}

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

fn outside_scope_error() -> BrokerErrorResponse {
    BrokerErrorResponse {
        error: BrokerAuthStatusKind::AuthorizationRequired,
        message: "result is unavailable or outside the grant scope".to_string(),
    }
}

fn open_mnema_deep_link(opaque_id: &str) -> Result<()> {
    let url = format!("mnema://open/{opaque_id}");
    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("open").arg(&url).status()?;
        if status.success() {
            Ok(())
        } else {
            Err(AppInfraError::BrokeredAccess(format!(
                "failed to open Mnema deep link with status {status}"
            )))
        }
    }
    #[cfg(target_os = "windows")]
    {
        let status = std::process::Command::new("cmd")
            .args(["/C", "start", "", &url])
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(AppInfraError::BrokeredAccess(format!(
                "failed to open Mnema deep link with status {status}"
            )))
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let status = std::process::Command::new("xdg-open").arg(&url).status()?;
        if status.success() {
            Ok(())
        } else {
            Err(AppInfraError::BrokeredAccess(format!(
                "failed to open Mnema deep link with status {status}"
            )))
        }
    }
}

fn map_search_response(
    response: SearchCaptureResponse,
    limit: u32,
    grant_id: Option<&str>,
    opaque_secret: &[u8],
) -> BrokerSearchResponse {
    let mut results = Vec::new();
    let mut frames = response.frames.into_iter();
    let mut audio = response.audio.into_iter();
    while results.len() < limit as usize {
        let before = results.len();
        if let Some(frame) = frames.next() {
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
                ),
                // Frame results have no sub-segment audio anchor.
                span_start_ms: None,
                span_end_ms: None,
                aligned_frame_id: None,
            });
            if results.len() >= limit as usize {
                break;
            }
        }
        if let Some(audio_result) = audio.next() {
            results.push(BrokerSearchResult {
                opaque_id: encode_signed_opaque_id(
                    "audio",
                    audio_result.audio_segment.id,
                    grant_id,
                    opaque_secret,
                ),
                kind: "audio".to_string(),
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
        }
        if results.len() == before {
            break;
        }
    }
    BrokerSearchResponse { results, limit }
}

fn broker_search_result_context(
    app_bundle_id: Option<String>,
    app_name: Option<String>,
    window_title: Option<String>,
) -> Option<BrokerSearchResultContext> {
    if app_bundle_id.is_none() && app_name.is_none() && window_title.is_none() {
        return None;
    }
    Some(BrokerSearchResultContext {
        app_bundle_id,
        app_name,
        window_title,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AppInfra, NewAudioSegment, NewFrame, ProcessingJobDraft, ProcessingResultDraft,
        SearchCaptureRefinements, SearchCaptureResponse, SearchDateRangeOrigin,
        SearchDateRangeRefinement,
    };

    fn run_async_test(test: impl std::future::Future<Output = ()>) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(test);
    }

    fn temp_config_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mnema-brokered-access-{name}-{}-{}",
            std::process::id(),
            now_unix_ms()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn temp_save_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mnema-brokered-access-save-{name}-{}-{}",
            std::process::id(),
            now_unix_ms()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn write_recording_settings(config_dir: &Path, save_dir: &Path) {
        let settings = RecordingSettingsFile {
            save_directory: save_dir.display().to_string(),
        };
        fs::write(
            config_dir.join(RECORDING_SETTINGS_FILE_NAME),
            serde_json::to_string(&settings).expect("settings should serialize"),
        )
        .expect("settings should write");
    }

    fn execute_request(
        config_dir: &Path,
        request: BrokeredCaptureRequest,
    ) -> BrokeredCaptureResponse {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let access = BrokeredCaptureAccess::from_config_dir(config_dir.to_path_buf());
        runtime
            .block_on(access.execute("mnema-cli", request))
            .unwrap()
    }

    fn test_conclusion(
        subject: &str,
        statement: &str,
        confidence: f64,
        status: capture_types::ConclusionStatus,
    ) -> capture_types::Conclusion {
        capture_types::Conclusion {
            id: 0,
            subject: subject.to_string(),
            statement: statement.to_string(),
            confidence,
            status,
            pinned: false,
            formed_at_ms: 0,
            last_supported_at_ms: 0,
            updated_at_ms: 0,
            evidence: Vec::new(),
        }
    }

    fn test_activity(title: &str, summary: &str, started_at_ms: i64) -> capture_types::Activity {
        capture_types::Activity {
            id: 0,
            title: title.to_string(),
            summary: summary.to_string(),
            category: None,
            focus: None,
            started_at_ms,
            ended_at_ms: started_at_ms + 1000,
            created_at_ms: started_at_ms,
            evidence: Vec::new(),
        }
    }

    #[test]
    fn recall_context_drops_sensitive_conclusions() {
        use capture_types::ConclusionStatus::Visible;
        let conclusions = vec![
            test_conclusion("Rust", "Is in a Rust learning phase", 0.9, Visible),
            // Sensitive: must NEVER be returned, even though it matches the query.
            test_conclusion("health", "user has depression", 0.95, Visible),
        ];
        let tokens = recall_query_tokens("tell me about rust and health and depression");
        let recalled = select_relevant_conclusions(&conclusions, &tokens, 10, true);
        assert!(
            recalled.iter().all(|c| !c.statement.contains("depression")),
            "sensitive conclusion leaked: {recalled:?}"
        );
        assert!(recalled.iter().any(|c| c.subject == "Rust"));
    }

    #[test]
    fn recall_context_drops_non_visible_conclusions() {
        use capture_types::ConclusionStatus::{Dismissed, Faded, Visible};
        let conclusions = vec![
            test_conclusion("Rust", "Likes Rust", 0.9, Visible),
            test_conclusion("Rust", "Dismissed Rust opinion", 0.9, Dismissed),
            test_conclusion("Rust", "Faded Rust opinion", 0.9, Faded),
        ];
        let tokens = recall_query_tokens("rust");
        let recalled = select_relevant_conclusions(&conclusions, &tokens, 10, true);
        assert_eq!(recalled.len(), 1);
        assert_eq!(recalled[0].statement, "Likes Rust");
    }

    #[test]
    fn recall_context_caps_relevant_conclusions() {
        use capture_types::ConclusionStatus::Visible;
        // 30 relevant, non-sensitive conclusions; the cap must bound the result.
        let conclusions: Vec<_> = (0..30)
            .map(|i| {
                test_conclusion(
                    "project alpha",
                    &format!("works on project alpha item {i}"),
                    0.5,
                    Visible,
                )
            })
            .collect();
        let tokens = recall_query_tokens("project alpha");
        let recalled =
            select_relevant_conclusions(&conclusions, &tokens, MAX_RECALL_CONTEXT_LIMIT as usize, true);
        assert_eq!(recalled.len(), MAX_RECALL_CONTEXT_LIMIT as usize);
        assert!(recalled.len() < conclusions.len());
    }

    #[test]
    fn recall_context_empty_query_falls_back_capped_not_whole_dossier() {
        use capture_types::ConclusionStatus::Visible;
        let conclusions: Vec<_> = (0..30)
            .map(|i| test_conclusion("subj", &format!("statement {i}"), i as f64 / 30.0, Visible))
            .collect();
        // Stopwords-only query yields no usable tokens.
        let tokens = recall_query_tokens("what is the and for");
        assert!(tokens.is_empty());
        let recalled = select_relevant_conclusions(&conclusions, &tokens, 5, true);
        assert_eq!(recalled.len(), 5, "fallback must still be capped");
        // Highest confidence first.
        assert!(recalled[0].confidence >= recalled[1].confidence);
    }

    #[test]
    fn recall_context_no_usable_tokens_suppresses_fallback_when_disabled() {
        use capture_types::ConclusionStatus::Visible;
        // The default path falls back to top-by-confidence on a no-token query
        // (above); with the fallback DISABLED (an episodic, time-ranged turn) the
        // SAME no-token query must yield ZERO conclusions instead of a confidence
        // dump of unrelated standing beliefs.
        let conclusions: Vec<_> = (0..10)
            .map(|i| test_conclusion("subj", &format!("statement {i}"), i as f64 / 10.0, Visible))
            .collect();
        let tokens = recall_query_tokens("what is the and for");
        assert!(tokens.is_empty());
        let recalled = select_relevant_conclusions(&conclusions, &tokens, 5, false);
        assert!(
            recalled.is_empty(),
            "fallback disabled must suppress the confidence dump: {recalled:?}"
        );
    }

    #[test]
    fn recall_context_disabling_fallback_does_not_affect_token_query() {
        use capture_types::ConclusionStatus::Visible;
        // With usable tokens the `allow_confidence_fallback` flag has no effect:
        // the normal score>0 path runs regardless of the flag.
        let conclusions = vec![
            test_conclusion("Rust", "Likes Rust", 0.9, Visible),
            test_conclusion("Python", "Likes Python", 0.9, Visible),
        ];
        let tokens = recall_query_tokens("rust");
        let with = select_relevant_conclusions(&conclusions, &tokens, 10, true);
        let without = select_relevant_conclusions(&conclusions, &tokens, 10, false);
        assert_eq!(with.len(), 1);
        assert_eq!(without.len(), 1);
        assert_eq!(with[0].statement, without[0].statement);
        assert_eq!(without[0].subject, "Rust");
    }

    #[test]
    fn recall_context_range_present_caps_conclusions_low() {
        use capture_types::ConclusionStatus::Visible;
        // Many conclusions all match the query, but a present time range caps the
        // recalled set at RANGE_PRESENT_CONCLUSION_LIMIT even though `limit` is
        // higher — proving the episodic de-emphasis.
        let conclusions: Vec<_> = (0..30)
            .map(|i| {
                test_conclusion(
                    "project alpha",
                    &format!("works on project alpha item {i}"),
                    0.5,
                    Visible,
                )
            })
            .collect();
        let tokens = recall_query_tokens("project alpha");
        // The handler passes `limit.min(RANGE_PRESENT_CONCLUSION_LIMIT)` and
        // `allow_confidence_fallback = false` when a range is present.
        let limit = (MAX_RECALL_CONTEXT_LIMIT as usize).min(RANGE_PRESENT_CONCLUSION_LIMIT);
        let recalled = select_relevant_conclusions(&conclusions, &tokens, limit, false);
        assert_eq!(recalled.len(), RANGE_PRESENT_CONCLUSION_LIMIT);
    }

    #[test]
    fn recall_context_relevance_filters_and_caps_activities() {
        let activities = vec![
            test_activity("Code review", "Reviewed the parser pull request", 3000),
            test_activity("Lunch break", "Ate a sandwich", 2000),
            test_activity("Parser work", "Wrote a new parser module", 1000),
        ];
        let tokens = recall_query_tokens("parser");
        let recalled = select_relevant_activities(&activities, &tokens, 10);
        assert_eq!(recalled.len(), 2);
        // Both relevant; recency tie-break puts the later one first.
        assert_eq!(recalled[0].title, "Code review");
        assert!(recalled.iter().all(|a| !a.title.contains("Lunch")));
    }

    // --- #1 whole-word matching: no substring false positives --------------

    #[test]
    fn recall_word_boundary_matching_rejects_substrings() {
        use capture_types::ConclusionStatus::Visible;
        // Query token "cat" must NOT match "category"/"education" (substring), only
        // the whole word "cat".
        let conclusions = vec![
            test_conclusion("work", "spends time on category triage", 0.9, Visible),
            test_conclusion("pets", "adopted a cat last month", 0.5, Visible),
        ];
        let tokens = recall_query_tokens("cat");
        let recalled = select_relevant_conclusions(&conclusions, &tokens, 10, true);
        assert_eq!(recalled.len(), 1, "only the whole-word match should survive");
        assert_eq!(recalled[0].subject, "pets");
    }

    #[test]
    fn recall_word_boundary_matching_on_activities_rejects_substrings() {
        // "run" must not match "running errands" via substring inside another word,
        // but stemming collapses "running" -> "run", so it SHOULD match as a word.
        let activities = vec![
            test_activity("Prepped a meal", "chopped vegetables for dinner", 2000),
            test_activity("Morning jog", "went running in the park", 1000),
        ];
        let tokens = recall_query_tokens("run");
        let recalled = select_relevant_activities(&activities, &tokens, 10);
        assert_eq!(recalled.len(), 1);
        assert_eq!(recalled[0].title, "Morning jog");
    }

    // --- #2 IDF weighting: rare token outranks common token ----------------

    #[test]
    fn recall_idf_weight_favors_rare_tokens() {
        // Rarer token (lower df) must weigh more than a common one.
        let rare = recall_idf_weight(100, 1);
        let common = recall_idf_weight(100, 90);
        assert!(rare > common, "rare {rare} should outweigh common {common}");
        // Always positive so any match still counts.
        assert!(recall_idf_weight(100, 100) > 0.0);
    }

    #[test]
    fn recall_idf_ranks_distinctive_match_above_common_match() {
        use capture_types::ConclusionStatus::Visible;
        // "rust" appears in many candidates (common); "kazoo" in one (rare). A
        // single-token query matching the rare word should outrank a single-token
        // query matching the common word, all confidence equal.
        let mut conclusions: Vec<_> = (0..10)
            .map(|i| test_conclusion("rust", &format!("uses rust at work {i}"), 0.5, Visible))
            .collect();
        conclusions.push(test_conclusion(
            "music",
            "plays the kazoo on weekends",
            0.5,
            Visible,
        ));
        // Query both a common and a rare token; the rare-token doc must rank first.
        let tokens = recall_query_tokens("rust kazoo");
        let recalled = select_relevant_conclusions(&conclusions, &tokens, 11, true);
        assert_eq!(
            recalled[0].statement, "plays the kazoo on weekends",
            "rare-token match must rank above common-token matches: {recalled:?}"
        );
    }

    // --- #3 stemmer: collapses common suffixes, guards short words ---------

    #[test]
    fn recall_stem_collapses_common_suffixes() {
        // The stem need not be a real word — only consistent. What matters is
        // that morphological variants collapse to the SAME key.
        assert_eq!(recall_stem("running"), "run");
        assert_eq!(recall_stem("tests"), "test");
        assert_eq!(recall_stem("quickly"), "quick");
        assert_eq!(recall_stem("reviewed"), "review");
        // Cross-form keys agree so the matcher bridges the lexical gap.
        assert_eq!(recall_stem("coding"), recall_stem("code"));
        assert_eq!(recall_stem("parsing"), recall_stem("parse"));
        assert_eq!(recall_stem("runs"), recall_stem("running"));
        assert_eq!(recall_stem("tested"), recall_stem("tests"));
    }

    #[test]
    fn recall_stem_guards_against_over_stemming_short_words() {
        // Short words must survive intact rather than being gutted.
        assert_eq!(recall_stem("is"), "is");
        assert_eq!(recall_stem("red"), "red");
        assert_eq!(recall_stem("bus"), "bus");
        assert_eq!(recall_stem("cat"), "cat");
        assert_eq!(recall_stem("ring"), "ring"); // not stemmed to "r"
    }

    #[test]
    fn recall_stemming_bridges_lexical_gap() {
        use capture_types::ConclusionStatus::Visible;
        // Query "running" should reach a conclusion that says "run".
        let conclusions = vec![test_conclusion(
            "fitness",
            "likes to run every morning",
            0.9,
            Visible,
        )];
        let tokens = recall_query_tokens("running");
        let recalled = select_relevant_conclusions(&conclusions, &tokens, 10, true);
        assert_eq!(recalled.len(), 1, "stemming should bridge running~run");
    }

    // --- #4 sensitive-activity filtering -----------------------------------

    #[test]
    fn recall_context_drops_sensitive_activities() {
        // An Activity whose title/summary lands in a sensitive category must be
        // dropped before scoring, exactly like sensitive Conclusions.
        let activities = vec![
            test_activity("Therapy session", "attended a therapy appointment", 2000),
            test_activity("Code review", "reviewed the therapy scheduler code", 1000),
        ];
        // Query matches both, but the sensitive one must never be returned.
        let tokens = recall_query_tokens("therapy");
        let recalled = select_relevant_activities(&activities, &tokens, 10);
        assert!(
            recalled.iter().all(|a| a.title != "Therapy session"),
            "sensitive activity leaked: {recalled:?}"
        );
        // The benign code-review activity (its TEXT trips the guardrail via
        // "therapy" too) — confirm guardrail symmetry: anything matching the
        // sensitive term list is dropped, biasing to over-suppression like
        // conclusions do. So NOTHING relevant survives here.
        assert!(recalled.is_empty(), "over-suppression by design: {recalled:?}");
    }

    #[test]
    fn recall_context_keeps_benign_activities_when_sensitive_present() {
        let activities = vec![
            test_activity("Doctor visit", "discussed medication options", 3000),
            test_activity("Parser work", "wrote a new parser module", 2000),
        ];
        let tokens = recall_query_tokens("parser medication");
        let recalled = select_relevant_activities(&activities, &tokens, 10);
        // The medication (sensitive) activity is dropped; the parser one stays.
        assert_eq!(recalled.len(), 1);
        assert_eq!(recalled[0].title, "Parser work");
    }

    // --- fallbacks remain intact (no usable tokens) ------------------------

    #[test]
    fn recall_context_empty_query_falls_back_most_recent_activities_capped() {
        let activities: Vec<_> = (0..30)
            .map(|i| test_activity(&format!("act {i}"), "summary", 1000 + i as i64))
            .collect();
        // Stopwords-only query yields no usable tokens.
        let tokens = recall_query_tokens("what is the and for");
        assert!(tokens.is_empty());
        let recalled = select_relevant_activities(&activities, &tokens, 5);
        assert_eq!(recalled.len(), 5, "fallback must still be capped");
        // Most-recent first.
        assert!(recalled[0].started_at >= recalled[1].started_at);
    }

    #[test]
    fn recall_context_command_type_and_result_count() {
        let request = BrokeredCaptureRequest::RecallContext(BrokerRecallContextRequest {
            query: "anything".to_string(),
            limit: None,
            from: None,
            to: None,
        });
        assert_eq!(request.command_type(), Some("recall_context"));

        let response = BrokeredCaptureResponse::RecallContext(BrokerRecallContextResponse {
            conclusions: vec![BrokerRecalledConclusion {
                subject: "s".to_string(),
                statement: "t".to_string(),
                confidence: 0.5,
                status: "visible".to_string(),
            }],
            activities: vec![BrokerRecalledActivity {
                title: "a".to_string(),
                summary: "b".to_string(),
                category: None,
                focus: None,
                started_at: "1970-01-01T00:00:00Z".to_string(),
                ended_at: "1970-01-01T00:00:01Z".to_string(),
            }],
        });
        assert_eq!(response.result_count(), 2);
    }

    /// A seeded in-range Activity that matches the query is recalled; a recent
    /// out-of-range Activity that ALSO matches is excluded by the `from`/`to`
    /// window. Conclusions are unaffected by the window (they have no wire
    /// timestamp). An unparseable bound is IGNORED gracefully — the turn still
    /// succeeds with the other (valid) bound applied.
    #[test]
    fn recall_context_filters_activities_by_time_window_and_ignores_bad_bound() {
        run_async_test(async {
            use crate::user_context::store::{NewActivity, NewActivityEvidence, NewConclusion};

            let config_dir = temp_config_dir("recall-window");
            let save_dir = temp_save_dir("recall-window");
            let infra = AppInfra::initialize(&save_dir)
                .await
                .expect("infra should initialize");
            let store = infra.user_context();

            // An OLD activity that matches "parser", inside the window.
            store
                .insert_activity_with_evidence(NewActivity {
                    title: "parser internals".to_string(),
                    summary: "worked on the parser module".to_string(),
                    category: None,
                    focus: None,
                    started_at_ms: 1_000,
                    ended_at_ms: 1_001,
                    derivation_run_id: None,
                    evidence: vec![NewActivityEvidence {
                        subject_type: "frame".to_string(),
                        subject_id: 1,
                        captured_at_ms: Some(1_000),
                    }],
                })
                .await
                .expect("seed old activity");
            // A RECENT activity that ALSO matches "parser", but outside the window.
            store
                .insert_activity_with_evidence(NewActivity {
                    title: "parser rewrite".to_string(),
                    summary: "rewrote the parser".to_string(),
                    category: None,
                    focus: None,
                    started_at_ms: 50_000,
                    ended_at_ms: 50_001,
                    derivation_run_id: None,
                    evidence: vec![NewActivityEvidence {
                        subject_type: "frame".to_string(),
                        subject_id: 2,
                        captured_at_ms: Some(50_000),
                    }],
                })
                .await
                .expect("seed recent activity");
            // A visible Conclusion that mentions the query subject — must survive
            // regardless of the time window (Conclusions are never time-scoped).
            store
                .upsert_conclusion(NewConclusion {
                    subject: "parser".to_string(),
                    statement: "The user maintains a parser".to_string(),
                    confidence: 0.9,
                    formed_at_ms: 1_000,
                    last_supported_at_ms: 1_000,
                })
                .await
                .expect("seed conclusion");

            let grant = create_grant(
                &config_dir,
                "Local agent",
                1,
                BrokerGrantScope::AllRetainedHistory,
            )
            .expect("grant should create");

            // Window [500ms, 10_000ms): one valid `from`, and a deliberately
            // BAD `to` that must be ignored gracefully (the turn still runs with
            // only `from` applied, so the recent out-of-range match survives —
            // see the assertion below for the both-valid-bounds case).
            let bad_to = broker_recall_context(
                &infra,
                &[grant.clone()],
                BrokerRecallContextRequest {
                    query: "parser".to_string(),
                    limit: None,
                    from: Some("1970-01-01T00:00:00.500Z".to_string()),
                    to: Some("not-a-timestamp".to_string()),
                },
            )
            .await
            .expect("recall should run")
            .expect("recall should be authorized");
            // Bad `to` ignored → only `from` applied → BOTH parser activities
            // survive (turn did not error).
            assert_eq!(bad_to.activities.len(), 2, "{:?}", bad_to.activities);

            // Both bounds valid: window [500ms, 10_000ms) catches only the old
            // parser activity; the recent one is excluded.
            let windowed = broker_recall_context(
                &infra,
                &[grant],
                BrokerRecallContextRequest {
                    query: "parser".to_string(),
                    limit: None,
                    from: Some("1970-01-01T00:00:00.500Z".to_string()),
                    to: Some("1970-01-01T00:00:10Z".to_string()),
                },
            )
            .await
            .expect("recall should run")
            .expect("recall should be authorized");

            assert_eq!(windowed.activities.len(), 1, "{:?}", windowed.activities);
            assert_eq!(windowed.activities[0].title, "parser internals");
            // Conclusion is unaffected by the activity time window.
            assert_eq!(windowed.conclusions.len(), 1);
            assert_eq!(windowed.conclusions[0].subject, "parser");
        });
    }

    /// End-to-end regression (#4): an Activity is persisted *unfiltered*, so the
    /// ONLY guardrail on the Activity egress path is the broker re-filter in
    /// `select_relevant_activities`. Drive the full `broker_recall_context` over a
    /// real store and assert a sensitive Activity never appears in
    /// `BrokerRecallContextResponse.activities`. This is the test the load-bearing
    /// comment points at — if someone deletes the "redundant"-looking filter line,
    /// THIS goes red even though derivation-time tests stay green.
    #[test]
    fn sensitive_activity_never_egresses_via_recall_context() {
        run_async_test(async {
            use crate::user_context::store::{NewActivity, NewActivityEvidence};

            let config_dir = temp_config_dir("recall-sensitive-activity");
            let save_dir = temp_save_dir("recall-sensitive-activity");
            let infra = AppInfra::initialize(&save_dir)
                .await
                .expect("infra should initialize");
            let store = infra.user_context();

            // A SENSITIVE activity persisted unfiltered (derivation does NOT drop
            // Activities), matching the query on a benign token ("appointment").
            store
                .insert_activity_with_evidence(NewActivity {
                    title: "Therapy appointment".to_string(),
                    summary: "attended a therapy appointment".to_string(),
                    category: None,
                    focus: None,
                    started_at_ms: 2_000,
                    ended_at_ms: 2_001,
                    derivation_run_id: None,
                    evidence: vec![NewActivityEvidence {
                        subject_type: "frame".to_string(),
                        subject_id: 1,
                        captured_at_ms: Some(2_000),
                    }],
                })
                .await
                .expect("seed sensitive activity");
            // A benign activity matching the same query token, to prove recall is
            // working (not just empty) while the sensitive one is excluded.
            store
                .insert_activity_with_evidence(NewActivity {
                    title: "Dentist appointment".to_string(),
                    summary: "booked a dentist appointment".to_string(),
                    category: None,
                    focus: None,
                    started_at_ms: 1_000,
                    ended_at_ms: 1_001,
                    derivation_run_id: None,
                    evidence: vec![NewActivityEvidence {
                        subject_type: "frame".to_string(),
                        subject_id: 2,
                        captured_at_ms: Some(1_000),
                    }],
                })
                .await
                .expect("seed benign activity");

            let grant = create_grant(
                &config_dir,
                "Local agent",
                1,
                BrokerGrantScope::AllRetainedHistory,
            )
            .expect("grant should create");

            let response = broker_recall_context(
                &infra,
                &[grant],
                BrokerRecallContextRequest {
                    query: "appointment".to_string(),
                    limit: None,
                    from: None,
                    to: None,
                },
            )
            .await
            .expect("recall should run")
            .expect("recall should be authorized");

            // The sensitive activity must NOT appear in the response, in neither
            // title nor summary (no sensitive text crosses the boundary).
            assert!(
                response.activities.iter().all(|a| {
                    !crate::user_context::guardrail::is_sensitive(&a.title, &a.summary)
                }),
                "sensitive activity egressed via recall_context: {:?}",
                response.activities
            );
            assert!(
                response
                    .activities
                    .iter()
                    .all(|a| a.title != "Therapy appointment"),
                "therapy activity leaked: {:?}",
                response.activities
            );
            // The benign appointment still comes back — recall is genuinely working.
            assert!(
                response
                    .activities
                    .iter()
                    .any(|a| a.title == "Dentist appointment"),
                "benign activity should still be recalled: {:?}",
                response.activities
            );
        });
    }

    /// A range-present query with NO usable tokens (all stopwords) returns ZERO
    /// conclusions — the no-token confidence fallback is suppressed for episodic
    /// turns — while still returning the date-filtered activities. This proves
    /// Conclusions are de-emphasized without harming the activity timeline that
    /// actually answers an episodic question.
    #[test]
    fn recall_context_range_present_no_tokens_drops_conclusions_keeps_activities() {
        run_async_test(async {
            use crate::user_context::store::{NewActivity, NewActivityEvidence, NewConclusion};

            let config_dir = temp_config_dir("recall-range-no-tokens");
            let save_dir = temp_save_dir("recall-range-no-tokens");
            let infra = AppInfra::initialize(&save_dir)
                .await
                .expect("infra should initialize");
            let store = infra.user_context();

            // An in-window activity (any text; the query has no usable tokens, so
            // the activity path degrades to the most-recent in-window set).
            store
                .insert_activity_with_evidence(NewActivity {
                    title: "morning standup".to_string(),
                    summary: "discussed the sprint plan".to_string(),
                    category: None,
                    focus: None,
                    started_at_ms: 1_000,
                    ended_at_ms: 1_001,
                    derivation_run_id: None,
                    evidence: vec![NewActivityEvidence {
                        subject_type: "frame".to_string(),
                        subject_id: 1,
                        captured_at_ms: Some(1_000),
                    }],
                })
                .await
                .expect("seed in-window activity");
            // A high-confidence standing belief that the OLD (rangeless) path would
            // have dumped via the no-token confidence fallback.
            store
                .upsert_conclusion(NewConclusion {
                    subject: "habits".to_string(),
                    statement: "The user prefers dark mode".to_string(),
                    confidence: 0.99,
                    formed_at_ms: 1_000,
                    last_supported_at_ms: 1_000,
                })
                .await
                .expect("seed conclusion");

            let grant = create_grant(
                &config_dir,
                "Local agent",
                1,
                BrokerGrantScope::AllRetainedHistory,
            )
            .expect("grant should create");

            // Stopwords-only query → no usable tokens. A present range (valid
            // bounds) suppresses the confidence fallback.
            let ranged = broker_recall_context(
                &infra,
                &[grant],
                BrokerRecallContextRequest {
                    query: "what did i do".to_string(),
                    limit: None,
                    from: Some("1970-01-01T00:00:00.500Z".to_string()),
                    to: Some("1970-01-01T00:00:10Z".to_string()),
                },
            )
            .await
            .expect("recall should run")
            .expect("recall should be authorized");

            assert!(
                ranged.conclusions.is_empty(),
                "range-present no-token query must drop conclusions: {:?}",
                ranged.conclusions
            );
            // Activities intact: the in-window activity still comes back.
            assert_eq!(ranged.activities.len(), 1, "{:?}", ranged.activities);
            assert_eq!(ranged.activities[0].title, "morning standup");
        });
    }

    #[test]
    fn capture_request_without_active_grants_returns_authorization_error_without_audit() {
        let config_dir = temp_config_dir("no-grants");

        let response = execute_request(
            &config_dir,
            BrokeredCaptureRequest::Search(BrokerSearchRequest {
                query: "meeting".to_string(),
                from: None,
                to: None,
                limit: Some(5),
                app: None,
                window_title: None,
            }),
        );

        assert_eq!(
            response,
            BrokeredCaptureResponse::Error(BrokerErrorResponse::authorization_required())
        );
        assert!(load_audit_events(&config_dir).unwrap().events.is_empty());
    }

    #[test]
    fn invalid_open_request_is_shaped_and_audited_by_brokered_capture_access() {
        let config_dir = temp_config_dir("invalid-open");
        create_grant_from_request(
            &config_dir,
            BrokerGrantCreateRequest {
                label: Some("Local agent".to_string()),
                duration_hours: Some(1),
                all_retained_history: Some(false),
            },
        )
        .unwrap();

        let response = execute_request(
            &config_dir,
            BrokeredCaptureRequest::OpenInMnema {
                opaque_id: "not-valid".to_string(),
            },
        );

        assert_eq!(
            response,
            BrokeredCaptureResponse::Error(BrokerErrorResponse {
                error: BrokerAuthStatusKind::AuthorizationRequired,
                message: "invalid opaque result id".to_string(),
            })
        );
        let audit = load_audit_events(&config_dir).unwrap();
        assert_eq!(audit.events.len(), 1);
        assert_eq!(audit.events[0].tool_identity, "mnema-cli");
        assert_eq!(audit.events[0].command_type, "open_in_mnema");
        assert_eq!(audit.events[0].result_count, 0);
        assert_eq!(audit.events[0].scope_class, "time_scoped");
    }

    #[test]
    fn grant_create_request_applies_default_label_and_duration_cap() {
        let config_dir = temp_config_dir("create-grant");

        let grant = create_grant_from_request(
            &config_dir,
            BrokerGrantCreateRequest {
                label: None,
                duration_hours: Some(24 * 31),
                all_retained_history: Some(true),
            },
        )
        .unwrap();

        assert_eq!(grant.label, "Local agent");
        assert_eq!(grant.scope, BrokerGrantScope::AllRetainedHistory);
        assert_eq!(
            grant.expires_at_unix_ms - grant.created_at_unix_ms,
            24 * 30 * 60 * 60 * 1000
        );
    }

    #[test]
    fn app_identifier_config_dir_uses_supplied_identifier() {
        if std::env::var_os("MNEMA_APP_CONFIG_DIR").is_some() {
            return;
        }
        let path = default_app_config_dir_for_identifier("com.example.mnema-test")
            .expect("config dir should resolve");

        assert!(path.ends_with("com.example.mnema-test"));
    }

    #[test]
    fn empty_opaque_id_is_invalid_instead_of_panicking() {
        assert_eq!(decode_opaque_id(""), None);
        assert_eq!(opaque_capture_reference(""), None);
    }

    #[test]
    fn signed_opaque_capture_reference_requires_broker_signature() {
        let config_dir = temp_config_dir("signed-opaque-reference");
        let secret = load_or_create_opaque_secret(&config_dir).expect("secret should load");
        let opaque_id = encode_signed_opaque_id("frame", 17, Some("grant-1"), &secret);

        assert_eq!(
            signed_opaque_capture_reference(&config_dir, "f11").expect("unsigned should parse"),
            None
        );
        assert_eq!(
            signed_opaque_capture_reference(&config_dir, &opaque_id).expect("signed should parse"),
            Some(BrokerOpaqueCaptureReference {
                opaque_id,
                frame_id: Some(17),
                audio_segment_id: None,
                grant_id: Some("grant-1".to_string()),
                kind: "frame".to_string(),
            })
        );
    }

    #[test]
    fn broker_search_interleaves_audio_before_applying_limit() {
        let secret = b"test broker opaque secret with enough bytes";
        let frame = |id: i64| crate::Frame {
            id,
            session_id: "screen-session".to_string(),
            file_path: format!("/tmp/frame-{id}.jpg"),
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
        };
        let audio_segment = crate::AudioSegment {
            id: 22,
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
        let response = SearchCaptureResponse {
            normalized_query: "target".to_string(),
            snapshot_document_id: 1,
            frames: vec![
                crate::FrameSearchResult {
                    group_key: "frame:11".to_string(),
                    representative_frame: frame(11),
                    group_start_at: "2026-05-17T10:00:00Z".to_string(),
                    group_end_at: "2026-05-17T10:00:00Z".to_string(),
                    match_count: 1,
                    snippet: "frame target".to_string(),
                    app_bundle_id: Some("com.example.Linear".to_string()),
                    app_name: Some("Linear".to_string()),
                    window_title: Some("Roadmap".to_string()),
                    thumbnail_frame_id: 11,
                    text_source_kind: "direct".to_string(),
                    secret_redaction_count: 0,
                    has_secret_redactions: false,
                },
                crate::FrameSearchResult {
                    group_key: "frame:12".to_string(),
                    representative_frame: frame(12),
                    group_start_at: "2026-05-17T10:01:00Z".to_string(),
                    group_end_at: "2026-05-17T10:01:00Z".to_string(),
                    match_count: 1,
                    snippet: "second frame target".to_string(),
                    app_bundle_id: None,
                    app_name: None,
                    window_title: None,
                    thumbnail_frame_id: 12,
                    text_source_kind: "direct".to_string(),
                    secret_redaction_count: 0,
                    has_secret_redactions: false,
                },
            ],
            audio: vec![crate::AudioSearchResult {
                group_key: "audio:22:0-1000".to_string(),
                audio_segment,
                source_kind: AudioSegmentSourceKind::Microphone,
                span_start_ms: 0,
                span_end_ms: 1_000,
                absolute_start_at: "2026-05-17T10:00:00Z".to_string(),
                absolute_end_at: "2026-05-17T10:00:01Z".to_string(),
                match_count: 1,
                snippet: "audio target".to_string(),
                aligned_frame: None,
                secret_redaction_count: 0,
                has_secret_redactions: false,
            }],
            has_more_frames: false,
            has_more_audio: false,
            applied_refinements: SearchCaptureRefinements {
                date_range: Some(SearchDateRangeRefinement {
                    start_at: "2026-05-17T00:00:00Z".to_string(),
                    end_at: "2026-05-18T00:00:00Z".to_string(),
                    origin: Some(SearchDateRangeOrigin::VisibleTimeline),
                }),
                apps: Vec::new(),
                window_title: None,
                audio_sources: Vec::new(),
                screen_source: false,
            },
            residual_query: "target".to_string(),
            parse_errors: Vec::new(),
        };

        let mapped = map_search_response(response, 2, Some("grant-1"), secret);

        assert_eq!(
            mapped
                .results
                .iter()
                .map(|result| result.kind.as_str())
                .collect::<Vec<_>>(),
            vec!["frame", "audio"]
        );
        assert!(mapped.results[0].opaque_id.contains('.'));
        assert_ne!(mapped.results[0].opaque_id, "fb");
        assert_eq!(
            mapped.results[0].context,
            Some(BrokerSearchResultContext {
                app_bundle_id: Some("com.example.Linear".to_string()),
                app_name: Some("Linear".to_string()),
                window_title: Some("Roadmap".to_string()),
            })
        );
        assert_eq!(mapped.results[1].context, None);
    }

    #[test]
    fn broker_timeline_filters_screen_intervals_by_app_and_window_title() {
        run_async_test(async {
            let config_dir = temp_config_dir("timeline-context");
            let save_dir = temp_save_dir("timeline-context");
            let infra = AppInfra::initialize(&save_dir)
                .await
                .expect("infra should initialize");

            for (file_name, captured_at, window_title) in [
                (
                    "timeline-roadmap.jpg",
                    "2026-05-17T10:00:00Z",
                    "Roadmap Grooming",
                ),
                ("timeline-planning.jpg", "2026-05-17T10:01:00Z", "Planning"),
            ] {
                let frame = infra
                    .insert_frame(
                        &NewFrame::new(
                            "screen-session",
                            save_dir.join(file_name).display().to_string(),
                            captured_at,
                        )
                        .with_metadata_snapshot(
                            capture_metadata::FrameMetadataSnapshot {
                                app_bundle_id: Some("com.example.Linear".to_string()),
                                app_name: Some("Linear".to_string()),
                                window_title: Some(window_title.to_string()),
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
                    .expect("OCR job should enqueue");
                let running = infra
                    .claim_queued_processing_job(job.id)
                    .await
                    .expect("OCR job should claim")
                    .expect("OCR job should exist");
                infra
                    .complete_processing_job(
                        running.id,
                        &ProcessingResultDraft::new().with_result_text("timeline body"),
                    )
                    .await
                    .expect("OCR job should complete");
            }

            let grant = create_grant(
                &config_dir,
                "Local agent",
                1,
                BrokerGrantScope::AllRetainedHistory,
            )
            .expect("grant should create");

            let response = broker_timeline(
                &infra,
                &[grant],
                BrokerTimelineRequest {
                    from: "2026-05-17T00:00:00Z".to_string(),
                    to: "2026-05-18T00:00:00Z".to_string(),
                    limit: Some(5),
                    app: Some("Linear".to_string()),
                    window_title: Some("roadmap".to_string()),
                },
            )
            .await
            .expect("timeline should run")
            .expect("timeline should be authorized");

            assert_eq!(response.intervals.len(), 1);
            assert_eq!(response.intervals[0].kind, "screen");
            assert_eq!(
                response.intervals[0]
                    .context
                    .as_ref()
                    .and_then(|context| context.app_name.as_deref()),
                Some("Linear")
            );
            assert_eq!(
                response.intervals[0]
                    .context
                    .as_ref()
                    .and_then(|context| context.window_title.as_deref()),
                Some("Roadmap Grooming")
            );
        });
    }

    #[test]
    fn broker_timeline_without_context_filters_includes_screen_and_audio_intervals() {
        run_async_test(async {
            let config_dir = temp_config_dir("timeline-all-sources");
            let save_dir = temp_save_dir("timeline-all-sources");
            let infra = AppInfra::initialize(&save_dir)
                .await
                .expect("infra should initialize");

            let frame = infra
                .insert_frame(&NewFrame::new(
                    "screen-session",
                    save_dir.join("timeline-screen.jpg").display().to_string(),
                    "2026-05-17T10:01:00Z",
                ))
                .await
                .expect("frame should insert");
            let job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
                .await
                .expect("OCR job should enqueue");
            let running = infra
                .claim_queued_processing_job(job.id)
                .await
                .expect("OCR job should claim")
                .expect("OCR job should exist");
            infra
                .complete_processing_job(
                    running.id,
                    &ProcessingResultDraft::new().with_result_text("timeline body"),
                )
                .await
                .expect("OCR job should complete");

            infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "mic-session",
                    1,
                    save_dir.join("audio.m4a").display().to_string(),
                    "2026-05-17T10:00:00Z",
                    "2026-05-17T10:00:30Z",
                ))
                .await
                .expect("audio segment should insert");

            let grant = create_grant(
                &config_dir,
                "Local agent",
                1,
                BrokerGrantScope::AllRetainedHistory,
            )
            .expect("grant should create");

            let response = broker_timeline(
                &infra,
                &[grant],
                BrokerTimelineRequest {
                    from: "2026-05-17T00:00:00Z".to_string(),
                    to: "2026-05-18T00:00:00Z".to_string(),
                    limit: Some(5),
                    app: None,
                    window_title: None,
                },
            )
            .await
            .expect("timeline should run")
            .expect("timeline should be authorized");

            assert_eq!(
                response
                    .intervals
                    .iter()
                    .map(|interval| interval.kind.as_str())
                    .collect::<Vec<_>>(),
                vec!["screen", "audio_microphone"]
            );
        });
    }

    #[test]
    fn broker_show_text_authorizes_audio_by_segment_overlap() {
        run_async_test(async {
            let config_dir = temp_config_dir("audio-overlap");
            let save_dir = temp_save_dir("audio-overlap");
            let infra = AppInfra::initialize(&save_dir)
                .await
                .expect("infra should initialize");
            let now = now_unix_ms();
            let started_at = format_unix_ms(now.saturating_sub(2 * 24 * 60 * 60 * 1000));
            let ended_at = format_unix_ms(now);
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "mic-session",
                    1,
                    save_dir.join("audio.m4a").display().to_string(),
                    started_at,
                    ended_at,
                ))
                .await
                .expect("segment should insert");
            let job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_audio_segment_transcription(
                    segment.id,
                ))
                .await
                .expect("job should enqueue");
            let running = infra
                .claim_queued_processing_job(job.id)
                .await
                .expect("job should claim")
                .expect("job should exist");
            infra
                .complete_processing_job(
                    running.id,
                    &ProcessingResultDraft::new().with_result_text("overlapping transcript"),
                )
                .await
                .expect("job should complete");
            let grant = create_grant(
                &config_dir,
                "Local agent",
                1,
                BrokerGrantScope::RecentDays { days: 1 },
            )
            .expect("grant should create");
            let secret = load_or_create_opaque_secret(&config_dir).expect("secret should load");
            let opaque_id = encode_signed_opaque_id("audio", segment.id, Some(&grant.id), &secret);

            let response = broker_show_text(&config_dir, &infra, &[grant], &opaque_id)
                .await
                .expect("show text should run")
                .expect("overlapping audio should be authorized");

            assert_eq!(response.text, "overlapping transcript");
        });
    }

    #[test]
    fn ask_ai_show_text_authorizes_all_retained_without_persisted_grant() {
        run_async_test(async {
            let config_dir = temp_config_dir("ask-ai-show-text");
            let save_dir = temp_save_dir("ask-ai-show-text");
            let infra = AppInfra::initialize(&save_dir)
                .await
                .expect("infra should initialize");
            write_recording_settings(&config_dir, &save_dir);
            let now = now_unix_ms();
            let started_at = format_unix_ms(now.saturating_sub(2 * 24 * 60 * 60 * 1000));
            let ended_at = format_unix_ms(now.saturating_sub(2 * 24 * 60 * 60 * 1000));
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "mic-session",
                    1,
                    save_dir.join("audio.m4a").display().to_string(),
                    started_at,
                    ended_at,
                ))
                .await
                .expect("segment should insert");
            let job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_audio_segment_transcription(
                    segment.id,
                ))
                .await
                .expect("job should enqueue");
            let running = infra
                .claim_queued_processing_job(job.id)
                .await
                .expect("job should claim")
                .expect("job should exist");
            infra
                .complete_processing_job(
                    running.id,
                    &ProcessingResultDraft::new().with_result_text("all retained transcript"),
                )
                .await
                .expect("job should complete");

            let secret = load_or_create_opaque_secret(&config_dir).expect("secret should load");
            let opaque_id =
                encode_signed_opaque_id("audio", segment.id, Some(ASK_AI_BROKER_GRANT_ID), &secret);

            let access = BrokeredCaptureAccess::from_config_dir(config_dir.clone());
            let identity =
                BrokerClientIdentity::new("PI", BrokerClientIdentitySource::Inferred).unwrap();

            let response = access
                .execute_for_ask_ai(identity, BrokeredCaptureRequest::ShowText { opaque_id })
                .await
                .unwrap();

            match response {
                BrokeredCaptureResponse::ShowText(show_text) => {
                    assert_eq!(show_text.text, "all retained transcript");
                }
                other => panic!("expected ShowText response, got {other:?}"),
            }

            assert!(load_grants(&config_dir).unwrap().grants.is_empty());

            let audit = load_audit_events(&config_dir).unwrap();
            assert_eq!(audit.events.len(), 1);
            let event = &audit.events[0];
            assert_eq!(event.scope_class, "all_retained_history");
            assert_eq!(event.grant_id, Some(ASK_AI_BROKER_GRANT_ID.to_string()));
            assert_eq!(event.command_type, "show_text");
            assert_eq!(event.tool_identity, "PI");
        });
    }

    #[test]
    fn ask_ai_timeline_reaches_all_retained_history() {
        run_async_test(async {
            let config_dir = temp_config_dir("ask-ai-timeline");
            let save_dir = temp_save_dir("ask-ai-timeline");
            let infra = AppInfra::initialize(&save_dir)
                .await
                .expect("infra should initialize");
            write_recording_settings(&config_dir, &save_dir);
            let now = now_unix_ms();
            let started_at = format_unix_ms(now.saturating_sub(2 * 24 * 60 * 60 * 1000));
            let ended_at = format_unix_ms(now.saturating_sub(2 * 24 * 60 * 60 * 1000));
            infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "mic-session",
                    1,
                    save_dir.join("audio.m4a").display().to_string(),
                    started_at,
                    ended_at,
                ))
                .await
                .expect("segment should insert");

            let access = BrokeredCaptureAccess::from_config_dir(config_dir.clone());
            let identity =
                BrokerClientIdentity::new("PI", BrokerClientIdentitySource::Inferred).unwrap();

            let from = format_unix_ms(now.saturating_sub(3 * 24 * 60 * 60 * 1000));
            let to = format_unix_ms(now);
            let response = access
                .execute_for_ask_ai(
                    identity,
                    BrokeredCaptureRequest::Timeline(BrokerTimelineRequest {
                        from,
                        to,
                        limit: Some(50),
                        app: None,
                        window_title: None,
                    }),
                )
                .await
                .unwrap();

            match response {
                BrokeredCaptureResponse::Timeline(timeline) => {
                    assert!(!timeline.intervals.is_empty());
                    assert!(timeline
                        .intervals
                        .iter()
                        .any(|interval| interval.kind == "audio_microphone"));
                }
                other => panic!("expected Timeline response, got {other:?}"),
            }

            assert!(load_grants(&config_dir).unwrap().grants.is_empty());
        });
    }

    #[test]
    fn ask_ai_rejects_open_in_mnema_as_non_data_tool() {
        run_async_test(async {
            let access =
                BrokeredCaptureAccess::from_config_dir(temp_config_dir("ask-ai-open").clone());
            let identity =
                BrokerClientIdentity::new("PI", BrokerClientIdentitySource::Inferred).unwrap();

            let response = access
                .execute_for_ask_ai(
                    identity,
                    BrokeredCaptureRequest::OpenInMnema {
                        opaque_id: "anything".into(),
                    },
                )
                .await
                .unwrap();

            assert_eq!(
                response,
                BrokeredCaptureResponse::Error(BrokerErrorResponse::authorization_required())
            );
        });
    }

    #[test]
    fn broker_show_text_resolves_equivalent_reuse_frame_text() {
        run_async_test(async {
            let config_dir = temp_config_dir("equivalent-reuse-show-text");
            let save_dir = temp_save_dir("equivalent-reuse-show-text");
            let infra = AppInfra::initialize(&save_dir)
                .await
                .expect("infra should initialize");
            let equivalence = crate::FrameEquivalence {
                hint: Some("same-screen".to_string()),
                proof: Some(vec![9; 1024]),
                version: Some(1),
                status: Some(crate::FrameEquivalenceStatus::Ready),
                error: None,
            };
            let first = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/broker-show-text-source.jpg",
                        "2026-05-17T10:00:00Z",
                    )
                    .with_equivalence(equivalence.clone()),
                    None,
                )
                .await
                .expect("first frame should capture");
            let job = first.job.expect("first frame should enqueue OCR");
            let running = infra
                .claim_queued_processing_job(job.id)
                .await
                .expect("job should claim")
                .expect("job should exist");
            infra
                .complete_processing_job(
                    running.id,
                    &ProcessingResultDraft::new().with_result_text("reused frame text"),
                )
                .await
                .expect("job should complete");

            let second = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/broker-show-text-duplicate.jpg",
                        "2026-05-17T10:00:02Z",
                    )
                    .with_equivalence(equivalence),
                    None,
                )
                .await
                .expect("second frame should capture");
            assert!(second.job.is_none());
            assert!(infra
                .list_processing_results_for_subject(&ProcessingSubject::frame(second.frame.id))
                .await
                .expect("results should list")
                .is_empty());

            let grant = create_grant(
                &config_dir,
                "Local agent",
                1,
                BrokerGrantScope::AllRetainedHistory,
            )
            .expect("grant should create");
            let secret = load_or_create_opaque_secret(&config_dir).expect("secret should load");
            let opaque_id =
                encode_signed_opaque_id("frame", second.frame.id, Some(&grant.id), &secret);

            let response = broker_show_text(&config_dir, &infra, &[grant], &opaque_id)
                .await
                .expect("show text should run")
                .expect("equivalent-reuse frame should resolve source text");

            assert_eq!(response.text, "reused frame text");
        });
    }

    #[test]
    fn broker_show_text_rejects_equivalent_reuse_source_outside_scope() {
        run_async_test(async {
            let config_dir = temp_config_dir("equivalent-reuse-show-text-outside-scope");
            let save_dir = temp_save_dir("equivalent-reuse-show-text-outside-scope");
            let infra = AppInfra::initialize(&save_dir)
                .await
                .expect("infra should initialize");
            let equivalence = crate::FrameEquivalence {
                hint: Some("same-screen".to_string()),
                proof: Some(vec![10; 1024]),
                version: Some(1),
                status: Some(crate::FrameEquivalenceStatus::Ready),
                error: None,
            };
            let now = now_unix_ms();
            let source_captured_at = format_unix_ms(now.saturating_sub(2 * 24 * 60 * 60 * 1000));
            let duplicate_captured_at = format_unix_ms(now.saturating_sub(60 * 1000));
            let first = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/broker-show-text-source-outside-scope.jpg",
                        &source_captured_at,
                    )
                    .with_equivalence(equivalence.clone()),
                    None,
                )
                .await
                .expect("first frame should capture");
            let job = first.job.expect("first frame should enqueue OCR");
            let running = infra
                .claim_queued_processing_job(job.id)
                .await
                .expect("job should claim")
                .expect("job should exist");
            infra
                .complete_processing_job(
                    running.id,
                    &ProcessingResultDraft::new().with_result_text("out-of-scope reused text"),
                )
                .await
                .expect("job should complete");

            let second = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/broker-show-text-duplicate-in-scope.jpg",
                        &duplicate_captured_at,
                    )
                    .with_equivalence(equivalence),
                    None,
                )
                .await
                .expect("second frame should capture");
            assert!(second.job.is_none());

            let grant = create_grant(
                &config_dir,
                "Local agent",
                1,
                BrokerGrantScope::RecentDays { days: 1 },
            )
            .expect("grant should create");
            let secret = load_or_create_opaque_secret(&config_dir).expect("secret should load");
            let opaque_id =
                encode_signed_opaque_id("frame", second.frame.id, Some(&grant.id), &secret);

            let response = broker_show_text(&config_dir, &infra, &[grant], &opaque_id)
                .await
                .expect("show text should run");

            assert_eq!(response, Err(outside_scope_error()));
        });
    }

    #[test]
    fn broker_rejects_unsigned_opaque_ids_for_authorized_commands() {
        run_async_test(async {
            let config_dir = temp_config_dir("unsigned-opaque");
            let save_dir = temp_save_dir("unsigned-opaque");
            let infra = AppInfra::initialize(&save_dir)
                .await
                .expect("infra should initialize");
            let grant = create_grant(
                &config_dir,
                "Local agent",
                1,
                BrokerGrantScope::AllRetainedHistory,
            )
            .expect("grant should create");

            let response = broker_authorize_opaque_reference(&config_dir, &infra, &[grant], "f1")
                .await
                .expect("authorization should run");

            assert_eq!(response, Err(invalid_opaque_id_error()));
        });
    }

    #[test]
    fn active_opaque_authorization_rejects_revoked_grant_replay() {
        run_async_test(async {
            let config_dir = temp_config_dir("revoked-opaque-replay");
            let save_dir = temp_save_dir("revoked-opaque-replay");
            write_recording_settings(&config_dir, &save_dir);
            let infra = AppInfra::initialize(&save_dir)
                .await
                .expect("infra should initialize");
            let frame = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/broker-revoked-replay.jpg",
                        &format_unix_ms(now_unix_ms()),
                    ),
                    None,
                )
                .await
                .expect("frame should capture")
                .frame;
            let grant = create_grant(
                &config_dir,
                "Local agent",
                1,
                BrokerGrantScope::AllRetainedHistory,
            )
            .expect("grant should create");
            let secret = load_or_create_opaque_secret(&config_dir).expect("secret should load");
            let opaque_id = encode_signed_opaque_id("frame", frame.id, Some(&grant.id), &secret);

            assert!(revoke_grant(&config_dir, &grant.id).expect("grant should revoke"));

            let response = authorize_active_opaque_capture_reference(&config_dir, &opaque_id)
                .await
                .expect("authorization should run");

            assert_eq!(response, None);
        });
    }

    #[test]
    fn active_opaque_authorization_rejects_ids_for_different_active_grant() {
        run_async_test(async {
            let config_dir = temp_config_dir("cross-grant-opaque-replay");
            let save_dir = temp_save_dir("cross-grant-opaque-replay");
            write_recording_settings(&config_dir, &save_dir);
            let infra = AppInfra::initialize(&save_dir)
                .await
                .expect("infra should initialize");
            let frame = infra
                .capture_frame(
                    &NewFrame::new(
                        "screen-session",
                        "/tmp/broker-cross-grant-replay.jpg",
                        &format_unix_ms(now_unix_ms()),
                    ),
                    None,
                )
                .await
                .expect("frame should capture")
                .frame;
            let original_grant = create_grant(
                &config_dir,
                "Original agent",
                1,
                BrokerGrantScope::AllRetainedHistory,
            )
            .expect("grant should create");
            let _other_grant = create_grant(
                &config_dir,
                "Other agent",
                1,
                BrokerGrantScope::AllRetainedHistory,
            )
            .expect("other grant should create");
            let secret = load_or_create_opaque_secret(&config_dir).expect("secret should load");
            let opaque_id =
                encode_signed_opaque_id("frame", frame.id, Some(&original_grant.id), &secret);

            assert!(revoke_grant(&config_dir, &original_grant.id).expect("grant should revoke"));

            let response = authorize_active_opaque_capture_reference(&config_dir, &opaque_id)
                .await
                .expect("authorization should run");

            assert_eq!(response, None);
        });
    }
}
