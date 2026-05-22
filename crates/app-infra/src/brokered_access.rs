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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrokeredCaptureRequest {
    AuthStatus,
    Search(BrokerSearchRequest),
    ShowText { opaque_id: String },
    Timeline(BrokerTimelineRequest),
    OpenInMnema { opaque_id: String },
}

impl BrokeredCaptureRequest {
    fn command_type(&self) -> Option<&'static str> {
        match self {
            Self::AuthStatus => None,
            Self::Search(_) => Some("search"),
            Self::ShowText { .. } => Some("show_text"),
            Self::Timeline(_) => Some("timeline"),
            Self::OpenInMnema { .. } => Some("open_in_mnema"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum BrokeredCaptureResponse {
    AuthStatus(BrokerAuthStatus),
    Search(BrokerSearchResponse),
    ShowText(BrokerShowTextResponse),
    Timeline(BrokerTimelineResponse),
    OpenInMnema(BrokerOpenInMnemaResponse),
    Error(BrokerErrorResponse),
}

impl BrokeredCaptureResponse {
    fn result_count(&self) -> u32 {
        match self {
            Self::Search(response) => response.results.len() as u32,
            Self::ShowText(_) | Self::OpenInMnema(_) => 1,
            Self::Timeline(response) => response.intervals.len() as u32,
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
        AppInfra::initialize(save_directory).await
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
        dirs::config_dir().map(|dir| dir.join("com.shaikzeeshan.mnema"))
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
        app: broker_app_refinement(app)?,
        window_title: broker_optional_filter(window_title, "windowTitle")?,
        audio_source: None,
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
    Ok(Ok(map_search_response(response, limit, &opaque_secret)))
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
    let infra = AppInfra::initialize(save_directory).await?;
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
    let in_scope = match reference.kind.as_str() {
        "frame" => {
            let Some(frame) = infra
                .get_frame(reference.frame_id.expect("frame reference has id"))
                .await?
            else {
                return Ok(Err(outside_scope_error()));
            };
            timestamp_within_scope(grants, &frame.captured_at)?
        }
        "audio" => {
            let Some(audio) = infra
                .get_audio_segment(reference.audio_segment_id.expect("audio reference has id"))
                .await?
            else {
                return Ok(Err(outside_scope_error()));
            };
            range_overlaps_scope(grants, &audio.started_at, &audio.ended_at)?
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
            context: None,
        });
    }
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

fn encode_opaque_id(kind: &str, id: i64) -> String {
    let tag = match kind {
        "frame" => "f",
        "audio" => "a",
        _ => "x",
    };
    format!("{tag}{:x}", id.max(0))
}

fn encode_signed_opaque_id(kind: &str, id: i64, secret: &[u8]) -> String {
    let payload = encode_opaque_id(kind, id);
    let signature = opaque_signature(&payload, secret);
    format!("{payload}.{signature}")
}

fn decode_opaque_id(value: &str) -> Option<(String, i64)> {
    let value = value
        .split_once('.')
        .map_or(value, |(payload, _signature)| payload);
    let mut chars = value.chars();
    let kind = chars.next()?;
    let rest = chars.as_str();
    let id = i64::from_str_radix(rest, 16).ok()?;
    let kind = match kind {
        'f' => "frame",
        'a' => "audio",
        _ => return None,
    };
    Some((kind.to_string(), id))
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
    let (kind, id) = decode_opaque_id(payload)?;
    Some(BrokerOpaqueCaptureReference {
        opaque_id: value.to_string(),
        frame_id: (kind == "frame").then_some(id),
        audio_segment_id: (kind == "audio").then_some(id),
        kind,
    })
}

pub fn opaque_capture_reference(value: &str) -> Option<BrokerOpaqueCaptureReference> {
    let (kind, id) = decode_opaque_id(value)?;
    Some(BrokerOpaqueCaptureReference {
        opaque_id: value.to_string(),
        frame_id: (kind == "frame").then_some(id),
        audio_segment_id: (kind == "audio").then_some(id),
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
                    opaque_secret,
                ),
                kind: "audio".to_string(),
                snippet: audio_result.snippet,
                started_at: audio_result.absolute_start_at,
                ended_at: audio_result.absolute_end_at,
                context: None,
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
    fn empty_opaque_id_is_invalid_instead_of_panicking() {
        assert_eq!(decode_opaque_id(""), None);
        assert_eq!(opaque_capture_reference(""), None);
    }

    #[test]
    fn signed_opaque_capture_reference_requires_broker_signature() {
        let config_dir = temp_config_dir("signed-opaque-reference");
        let secret = load_or_create_opaque_secret(&config_dir).expect("secret should load");
        let opaque_id = encode_signed_opaque_id("frame", 17, &secret);

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
                app: None,
                window_title: None,
                audio_source: None,
            },
        };

        let mapped = map_search_response(response, 2, secret);

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
            let opaque_id = encode_signed_opaque_id("audio", segment.id, &secret);

            let response = broker_show_text(&config_dir, &infra, &[grant], &opaque_id)
                .await
                .expect("show text should run")
                .expect("overlapping audio should be authorized");

            assert_eq!(response.text, "overlapping transcript");
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
            let opaque_id = encode_signed_opaque_id("frame", second.frame.id, &secret);

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
            let opaque_id = encode_signed_opaque_id("frame", second.frame.id, &secret);

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
            let opaque_id = encode_signed_opaque_id("frame", frame.id, &secret);

            assert!(revoke_grant(&config_dir, &grant.id).expect("grant should revoke"));

            let response = authorize_active_opaque_capture_reference(&config_dir, &opaque_id)
                .await
                .expect("authorization should run");

            assert_eq!(response, None);
        });
    }
}
