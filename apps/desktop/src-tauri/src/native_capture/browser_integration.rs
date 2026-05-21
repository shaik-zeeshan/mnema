use capture_types::{
    BrowserFamily, BrowserFamilyIntegrationStatus, BrowserIntegrationCoverageState,
    BrowserIntegrationPairingAction, BrowserIntegrationPairingState, BrowserIntegrationStatus,
    BrowserMetadataReason, BrowserMetadataSignalV1, BrowserMetadataSource, BrowserMetadataState,
    BrowserSecureEntryReason, BrowserSecureEntrySignalV1, BrowserSecureEntryState,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tauri::Manager;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpListener;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

const SECURE_ENTRY_KIND: &str = "browser_secure_entry_signal";
const METADATA_KIND: &str = "browser_metadata_signal";
const HEARTBEAT_TIMEOUT: Duration = Duration::from_millis(3_000);
const PAIRING_TTL_MS: u64 = 10 * 60 * 1_000;
const CHROMIUM_EXTENSION_ID: &str = "bnnoebfihdapbhcalgddficdehgggnng";
const NATIVE_HOST_NAME: &str = "com.shaikzeeshan.mnema.browser_integration";

pub type BrowserIntegrationState = Mutex<BrowserIntegrationRuntime>;
const IPC_CONFIG_FILE_NAME: &str = "browser-integration-runtime.json";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserIntegrationFamilyRequest {
    pub browser_family: BrowserFamily,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallBrowserIntegrationNativeHostRequest {
    #[serde(default)]
    pub extension_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallBrowserIntegrationNativeHostResponse {
    pub manifest_paths: Vec<String>,
    pub host_path: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserIntegrationIpcConfig {
    host: String,
    port: u16,
    ipc_token: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserIntegrationHostEnvelope {
    ipc_token: String,
    pairing_token: String,
    channel: String,
    secure_entry: Option<BrowserSecureEntrySignalV1>,
    metadata: Option<BrowserMetadataSignalV1>,
}

#[derive(Debug, Clone)]
struct FamilyRuntime {
    pairing_state: BrowserIntegrationPairingState,
    pairing_token_hash: Option<String>,
    pairing_expires_at_unix_ms: Option<u64>,
    last_secure_sequence: Option<u64>,
    last_metadata_sequence: Option<u64>,
    secure_entry_state: BrowserSecureEntryState,
    secure_entry_reason: BrowserSecureEntryReason,
    metadata_state: BrowserMetadataState,
    metadata_reason: BrowserMetadataReason,
    metadata_url: Option<String>,
    last_observed_at_unix_ms: Option<u64>,
    active_missing_clear: bool,
}

impl Default for FamilyRuntime {
    fn default() -> Self {
        Self {
            pairing_state: BrowserIntegrationPairingState::Unpaired,
            pairing_token_hash: None,
            pairing_expires_at_unix_ms: None,
            last_secure_sequence: None,
            last_metadata_sequence: None,
            secure_entry_state: BrowserSecureEntryState::Unavailable,
            secure_entry_reason: BrowserSecureEntryReason::ExtensionNotPaired,
            metadata_state: BrowserMetadataState::Unavailable,
            metadata_reason: BrowserMetadataReason::ExtensionNotPaired,
            metadata_url: None,
            last_observed_at_unix_ms: None,
            active_missing_clear: false,
        }
    }
}

#[derive(Debug, Default)]
pub struct BrowserIntegrationRuntime {
    families: BTreeMap<BrowserFamily, FamilyRuntime>,
    ipc_started: bool,
    ipc_config: Option<BrowserIntegrationIpcConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BrowserSafetyAggregate {
    Clear,
    Active,
    SourceLostFailClosed,
}

impl BrowserIntegrationRuntime {
    fn family_mut(&mut self, family: BrowserFamily) -> &mut FamilyRuntime {
        self.families.entry(family).or_default()
    }

    fn family(&self, family: BrowserFamily) -> FamilyRuntime {
        self.families.get(&family).cloned().unwrap_or_default()
    }

    pub(crate) fn browser_safety_state(&mut self, now_unix_ms: u64) -> BrowserSafetyAggregate {
        let mut active = false;
        let mut lost = false;
        for family in [BrowserFamily::Safari, BrowserFamily::Chromium] {
            let runtime = self.family_mut(family);
            if runtime.secure_entry_state == BrowserSecureEntryState::Active {
                active = true;
                if runtime
                    .last_observed_at_unix_ms
                    .is_some_and(|observed| now_unix_ms.saturating_sub(observed) > HEARTBEAT_TIMEOUT.as_millis() as u64)
                {
                    runtime.secure_entry_state = BrowserSecureEntryState::Unavailable;
                    runtime.secure_entry_reason = BrowserSecureEntryReason::NativeMessagingUnavailable;
                    runtime.active_missing_clear = true;
                    lost = true;
                }
            }
            if runtime.active_missing_clear {
                lost = true;
            }
        }
        if lost {
            BrowserSafetyAggregate::SourceLostFailClosed
        } else if active {
            BrowserSafetyAggregate::Active
        } else {
            BrowserSafetyAggregate::Clear
        }
    }

    pub(crate) fn latest_metadata_url(&self) -> Option<String> {
        [BrowserFamily::Safari, BrowserFamily::Chromium]
            .into_iter()
            .filter_map(|family| self.families.get(&family))
            .filter(|runtime| runtime.metadata_state == BrowserMetadataState::Available)
            .max_by_key(|runtime| runtime.last_observed_at_unix_ms.unwrap_or(0))
            .and_then(|runtime| runtime.metadata_url.clone())
    }

    fn status(&self) -> BrowserIntegrationStatus {
        let safari = self.family_status(BrowserFamily::Safari);
        let chromium = self.family_status(BrowserFamily::Chromium);
        let metadata_source = if [safari.clone(), chromium.clone()].iter().any(|status| {
            status.metadata_state == BrowserMetadataState::Available
                && status.coverage_state != BrowserIntegrationCoverageState::Unavailable
        }) {
            BrowserMetadataSource::BrowserExtension
        } else {
            BrowserMetadataSource::NativeBrowserUrlProbe
        };
        BrowserIntegrationStatus {
            native_apps: BrowserIntegrationCoverageState::Reliable,
            safari,
            chromium,
            metadata_source,
        }
    }

    fn set_ipc_config(&mut self, config: BrowserIntegrationIpcConfig) {
        self.ipc_started = true;
        self.ipc_config = Some(config);
    }

    fn family_status(&self, browser_family: BrowserFamily) -> BrowserFamilyIntegrationStatus {
        let runtime = self.family(browser_family);
        let coverage_state = match runtime.pairing_state {
            BrowserIntegrationPairingState::Paired
                if runtime.secure_entry_state != BrowserSecureEntryState::Unavailable =>
            {
                BrowserIntegrationCoverageState::Reliable
            }
            BrowserIntegrationPairingState::Paired => BrowserIntegrationCoverageState::Partial,
            BrowserIntegrationPairingState::Pairing | BrowserIntegrationPairingState::Unpaired => {
                BrowserIntegrationCoverageState::Unavailable
            }
        };
        BrowserFamilyIntegrationStatus {
            browser_family,
            pairing_state: runtime.pairing_state,
            coverage_state,
            secure_entry_state: runtime.secure_entry_state,
            secure_entry_reason: runtime.secure_entry_reason,
            metadata_state: runtime.metadata_state,
            metadata_reason: runtime.metadata_reason,
            last_observed_at_unix_ms: runtime.last_observed_at_unix_ms,
        }
    }

    fn start_pairing(&mut self, family: BrowserFamily, now_unix_ms: u64) -> BrowserIntegrationPairingAction {
        let token = pairing_token(family, now_unix_ms);
        let expires_at = now_unix_ms.saturating_add(PAIRING_TTL_MS);
        let runtime = self.family_mut(family);
        runtime.pairing_state = BrowserIntegrationPairingState::Pairing;
        runtime.pairing_token_hash = Some(token_hash(&token));
        runtime.pairing_expires_at_unix_ms = Some(expires_at);
        runtime.secure_entry_state = BrowserSecureEntryState::Unavailable;
        runtime.secure_entry_reason = BrowserSecureEntryReason::ExtensionNotPaired;
        runtime.metadata_state = BrowserMetadataState::Unavailable;
        runtime.metadata_reason = BrowserMetadataReason::ExtensionNotPaired;
        BrowserIntegrationPairingAction {
            browser_family: family,
            pairing_state: BrowserIntegrationPairingState::Pairing,
            setup_url: Some(format!("mnema-browser-extension://pair?family={}&token={token}", family_slug(family))),
            expires_at_unix_ms: Some(expires_at),
        }
    }

    fn revoke_pairing(&mut self, family: BrowserFamily) -> BrowserIntegrationPairingAction {
        let runtime = self.family_mut(family);
        *runtime = FamilyRuntime::default();
        BrowserIntegrationPairingAction {
            browser_family: family,
            pairing_state: BrowserIntegrationPairingState::Unpaired,
            setup_url: None,
            expires_at_unix_ms: None,
        }
    }

    fn accept_pairing_token(
        &mut self,
        family: BrowserFamily,
        token: &str,
        now_unix_ms: u64,
    ) -> Result<(), BrowserSecureEntryReason> {
        let runtime = self.family_mut(family);
        validate_pairing(runtime, token, now_unix_ms)
            .map_err(|_| BrowserSecureEntryReason::ExtensionNotPaired)?;
        runtime.pairing_state = BrowserIntegrationPairingState::Paired;
        runtime.secure_entry_state = BrowserSecureEntryState::Clear;
        runtime.secure_entry_reason = BrowserSecureEntryReason::NoFocusedCredentialControl;
        runtime.metadata_state = BrowserMetadataState::Unavailable;
        runtime.metadata_reason = BrowserMetadataReason::MetadataDisabled;
        runtime.last_observed_at_unix_ms = Some(now_unix_ms);
        runtime.active_missing_clear = false;
        Ok(())
    }

    pub(crate) fn accept_secure_entry_signal(
        &mut self,
        token: &str,
        signal: BrowserSecureEntrySignalV1,
        now_unix_ms: u64,
    ) -> Result<(), BrowserSecureEntryReason> {
        if signal.version != 1 || signal.kind != SECURE_ENTRY_KIND {
            return Err(BrowserSecureEntryReason::PageUnsupported);
        }
        let runtime = self.family_mut(signal.browser_family);
        validate_pairing(runtime, token, now_unix_ms)
            .map_err(|_| BrowserSecureEntryReason::ExtensionNotPaired)?;
        if runtime
            .last_secure_sequence
            .is_some_and(|previous| signal.sequence <= previous)
        {
            return Err(BrowserSecureEntryReason::PageUnsupported);
        }
        runtime.pairing_state = BrowserIntegrationPairingState::Paired;
        runtime.last_secure_sequence = Some(signal.sequence);
        runtime.secure_entry_state = signal.state;
        runtime.secure_entry_reason = signal.reason;
        runtime.last_observed_at_unix_ms = Some(signal.observed_at_unix_ms);
        if signal.state != BrowserSecureEntryState::Active {
            runtime.active_missing_clear = false;
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub(crate) fn accept_metadata_signal(
        &mut self,
        token: &str,
        signal: BrowserMetadataSignalV1,
        now_unix_ms: u64,
    ) -> Result<(), BrowserMetadataReason> {
        if signal.version != 1 || signal.kind != METADATA_KIND {
            return Err(BrowserMetadataReason::PageUnsupported);
        }
        let runtime = self.family_mut(signal.browser_family);
        validate_pairing(runtime, token, now_unix_ms)
            .map_err(|_| BrowserMetadataReason::ExtensionNotPaired)?;
        if runtime
            .last_metadata_sequence
            .is_some_and(|previous| signal.sequence <= previous)
        {
            return Err(BrowserMetadataReason::PageUnsupported);
        }
        runtime.pairing_state = BrowserIntegrationPairingState::Paired;
        runtime.last_metadata_sequence = Some(signal.sequence);
        runtime.metadata_state = signal.state;
        runtime.metadata_reason = signal.reason;
        runtime.metadata_url = signal.url;
        runtime.last_observed_at_unix_ms = Some(signal.observed_at_unix_ms);
        Ok(())
    }
}

pub(crate) fn initialize(app_handle: tauri::AppHandle) {
    if let Err(error) = install_known_chromium_native_host(&app_handle) {
        super::debug_log::log_warn(format!(
            "browser integration native host auto-install skipped: {error}"
        ));
    }
    tauri::async_runtime::spawn(async move {
        if let Err(error) = start_ipc_listener(app_handle.clone()).await {
            super::debug_log::log_warn(format!("browser integration IPC failed to start: {error}"));
        }
    });
}

async fn start_ipc_listener(app_handle: tauri::AppHandle) -> Result<(), String> {
    {
        let state = app_handle.state::<BrowserIntegrationState>();
        if state
            .lock()
            .map_err(|_| "browser integration state is unavailable".to_string())?
            .ipc_started
        {
            return Ok(());
        }
    }

    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|error| format!("bind localhost listener: {error}"))?;
    let addr = listener
        .local_addr()
        .map_err(|error| format!("read listener address: {error}"))?;
    let ipc_token = pairing_token(BrowserFamily::Chromium, super::runtime::now_unix_ms());
    let config = BrowserIntegrationIpcConfig {
        host: "127.0.0.1".to_string(),
        port: addr.port(),
        ipc_token,
    };
    persist_ipc_config(&app_handle, &config)?;
    app_handle
        .state::<BrowserIntegrationState>()
        .lock()
        .map_err(|_| "browser integration state is unavailable".to_string())?
        .set_ipc_config(config.clone());

    loop {
        let Ok((stream, peer)) = listener.accept().await else {
            continue;
        };
        if !peer_is_loopback(peer) {
            continue;
        }
        let app_handle = app_handle.clone();
        let config = config.clone();
        tauri::async_runtime::spawn(async move {
            let mut lines = BufReader::new(stream).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(envelope) = serde_json::from_str::<BrowserIntegrationHostEnvelope>(&line) {
                    handle_host_envelope(&app_handle, &config, envelope).await;
                }
            }
        });
    }
}

fn peer_is_loopback(peer: SocketAddr) -> bool {
    peer.ip().is_loopback()
}

fn ipc_config_path(app_handle: &tauri::AppHandle) -> PathBuf {
    app_handle
        .path()
        .app_config_dir()
        .unwrap_or_else(|_| PathBuf::from(".mnema"))
        .join(IPC_CONFIG_FILE_NAME)
}

fn persist_ipc_config(
    app_handle: &tauri::AppHandle,
    config: &BrowserIntegrationIpcConfig,
) -> Result<(), String> {
    let path = ipc_config_path(app_handle);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create browser integration config dir: {error}"))?;
    }
    let mut file = std::fs::File::create(path)
        .map_err(|error| format!("create browser integration config: {error}"))?;
    let raw = serde_json::to_string_pretty(config)
        .map_err(|error| format!("serialize browser integration config: {error}"))?;
    file.write_all(raw.as_bytes())
        .map_err(|error| format!("write browser integration config: {error}"))
}

async fn handle_host_envelope(
    app_handle: &tauri::AppHandle,
    config: &BrowserIntegrationIpcConfig,
    envelope: BrowserIntegrationHostEnvelope,
) {
    if envelope.ipc_token != config.ipc_token {
        return;
    }
    let now = super::runtime::now_unix_ms();
    let secure_entry_for_audit = envelope.secure_entry.clone();
    let accepted = {
        let state = app_handle.state::<BrowserIntegrationState>();
        let Ok(mut runtime) = state.lock() else {
            return;
        };
        match envelope.channel.as_str() {
            "pair" => envelope
                .secure_entry
                .as_ref()
                .map(|signal| {
                    runtime.accept_pairing_token(
                        signal.browser_family,
                        &envelope.pairing_token,
                        now,
                    )
                })
                .transpose()
                .is_ok(),
            "secureEntry" => envelope
                .secure_entry
                .map(|signal| runtime.accept_secure_entry_signal(&envelope.pairing_token, signal, now))
                .transpose()
                .is_ok(),
            "metadata" => envelope
                .metadata
                .map(|signal| runtime.accept_metadata_signal(&envelope.pairing_token, signal, now))
                .transpose()
                .is_ok(),
            _ => false,
        }
    };
    if accepted {
        if envelope.channel == "secureEntry" || envelope.channel == "pair" {
            record_coverage_event(app_handle, secure_entry_for_audit).await;
        }
        super::request_capture_safety_check(app_handle);
    }
}

async fn record_coverage_event(
    app_handle: &tauri::AppHandle,
    secure_entry: Option<BrowserSecureEntrySignalV1>,
) {
    let Some(signal) = secure_entry else {
        return;
    };
    let Some(infra) = app_handle.try_state::<crate::app_infra::AppInfraState>() else {
        return;
    };
    let occurred_at = OffsetDateTime::from_unix_timestamp(
        (signal.observed_at_unix_ms / 1000).min(i64::MAX as u64) as i64,
    )
    .unwrap_or_else(|_| OffsetDateTime::now_utc())
    .format(&Rfc3339)
    .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string());
    let browser_family = family_slug(signal.browser_family);
    let state = secure_state_slug(signal.state);
    let reason = secure_reason_slug(signal.reason);
    let infra = infra.inner().clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = infra
            .capture_safety()
            .record_browser_integration_coverage_event(
                &occurred_at,
                browser_family,
                state,
                reason,
            )
            .await
        {
            super::debug_log::log_warn(format!(
                "failed to record browser integration coverage event: {error}"
            ));
        }
    });
}

fn secure_state_slug(state: BrowserSecureEntryState) -> &'static str {
    match state {
        BrowserSecureEntryState::Active => "active",
        BrowserSecureEntryState::Clear => "clear",
        BrowserSecureEntryState::Unavailable => "unavailable",
    }
}

fn secure_reason_slug(reason: BrowserSecureEntryReason) -> &'static str {
    match reason {
        BrowserSecureEntryReason::FocusedPasswordControl => "focused_password_control",
        BrowserSecureEntryReason::FocusedRelatedCredentialControl => {
            "focused_related_credential_control"
        }
        BrowserSecureEntryReason::FocusedAutocompleteCredentialControl => {
            "focused_autocomplete_credential_control"
        }
        BrowserSecureEntryReason::NoFocusedCredentialControl => "no_focused_credential_control",
        BrowserSecureEntryReason::ExtensionNotInstalled => "extension_not_installed",
        BrowserSecureEntryReason::ExtensionNotPaired => "extension_not_paired",
        BrowserSecureEntryReason::NativeMessagingUnavailable => "native_messaging_unavailable",
        BrowserSecureEntryReason::WebsitePermissionUnavailable => "website_permission_unavailable",
        BrowserSecureEntryReason::BrowserUnsupported => "browser_unsupported",
        BrowserSecureEntryReason::PageUnsupported => "page_unsupported",
    }
}

fn validate_pairing(runtime: &FamilyRuntime, token: &str, now_unix_ms: u64) -> Result<(), ()> {
    if runtime
        .pairing_expires_at_unix_ms
        .is_some_and(|expires_at| now_unix_ms > expires_at)
    {
        return Err(());
    }
    match runtime.pairing_token_hash.as_deref() {
        Some(expected) if expected == token_hash(token) => Ok(()),
        _ => Err(()),
    }
}

fn pairing_token(family: BrowserFamily, now_unix_ms: u64) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{family:?}:{now_unix_ms}:{}", std::process::id()));
    format!("{:x}", hasher.finalize())
}

fn token_hash(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn family_slug(family: BrowserFamily) -> &'static str {
    match family {
        BrowserFamily::Safari => "safari",
        BrowserFamily::Chromium => "chromium",
    }
}

#[tauri::command]
pub fn get_browser_integration_status(
    state: tauri::State<'_, BrowserIntegrationState>,
) -> BrowserIntegrationStatus {
    state.lock().map(|runtime| runtime.status()).unwrap_or_else(|_| {
        BrowserIntegrationRuntime::default().status()
    })
}

#[tauri::command]
pub fn start_browser_integration_pairing(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, BrowserIntegrationState>,
    request: BrowserIntegrationFamilyRequest,
) -> Result<BrowserIntegrationPairingAction, String> {
    let now = super::runtime::now_unix_ms();
    let action = state
        .lock()
        .map_err(|_| "browser integration state is unavailable".to_string())?
        .start_pairing(request.browser_family, now);
    super::request_capture_safety_check(&app_handle);
    Ok(action)
}

#[tauri::command]
pub fn revoke_browser_integration_pairing(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, BrowserIntegrationState>,
    request: BrowserIntegrationFamilyRequest,
) -> Result<BrowserIntegrationPairingAction, String> {
    let action = state
        .lock()
        .map_err(|_| "browser integration state is unavailable".to_string())?
        .revoke_pairing(request.browser_family);
    super::request_capture_safety_check(&app_handle);
    Ok(action)
}

#[tauri::command]
pub fn rotate_browser_integration_pairing(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, BrowserIntegrationState>,
    request: BrowserIntegrationFamilyRequest,
) -> Result<BrowserIntegrationPairingAction, String> {
    let now = super::runtime::now_unix_ms();
    let action = state
        .lock()
        .map_err(|_| "browser integration state is unavailable".to_string())?
        .start_pairing(request.browser_family, now);
    super::request_capture_safety_check(&app_handle);
    Ok(action)
}

#[tauri::command]
pub fn install_browser_integration_native_host(
    app_handle: tauri::AppHandle,
    request: InstallBrowserIntegrationNativeHostRequest,
) -> Result<InstallBrowserIntegrationNativeHostResponse, String> {
    let extension_id = request
        .extension_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(CHROMIUM_EXTENSION_ID);
    install_chromium_native_host_for_extension(&app_handle, extension_id)
}

fn install_known_chromium_native_host(app_handle: &tauri::AppHandle) -> Result<(), String> {
    install_chromium_native_host_for_extension(app_handle, CHROMIUM_EXTENSION_ID).map(|_| ())
}

fn install_chromium_native_host_for_extension(
    app_handle: &tauri::AppHandle,
    extension_id: &str,
) -> Result<InstallBrowserIntegrationNativeHostResponse, String> {
    if !extension_id
        .chars()
        .all(|ch| matches!(ch, 'a'..='p'))
        || extension_id.len() < 16
    {
        return Err("Enter the Chromium extension ID from the extension details page.".to_string());
    }
    let host_path = browser_integration_host_path(app_handle)?;
    let manifest = serde_json::json!({
        "name": NATIVE_HOST_NAME,
        "description": "Mnema browser integration native messaging host",
        "path": host_path,
        "type": "stdio",
        "allowed_origins": [format!("chrome-extension://{extension_id}/")]
    });
    let raw = serde_json::to_string_pretty(&manifest)
        .map_err(|error| format!("serialize native host manifest: {error}"))?;
    let mut written = Vec::new();
    for dir in native_host_manifest_dirs() {
        std::fs::create_dir_all(&dir)
            .map_err(|error| format!("create native messaging host directory {}: {error}", dir.display()))?;
        let path = dir.join(format!("{NATIVE_HOST_NAME}.json"));
        std::fs::write(&path, &raw)
            .map_err(|error| format!("write native messaging host manifest {}: {error}", path.display()))?;
        written.push(path.to_string_lossy().to_string());
    }
    Ok(InstallBrowserIntegrationNativeHostResponse {
        manifest_paths: written,
        host_path,
    })
}

fn browser_integration_host_path(app_handle: &tauri::AppHandle) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|error| format!("resolve current executable: {error}"))?;
    let exe_dir = exe
        .parent()
        .ok_or_else(|| "current executable has no parent directory".to_string())?;
    let mut candidates = vec![
        exe_dir.join("mnema_browser_integration_host"),
        exe_dir
            .parent()
            .unwrap_or(exe_dir)
            .join("Resources")
            .join("browser-integration-native-host.sh"),
    ];
    if let Ok(resource_dir) = app_handle.path().resource_dir() {
        candidates.push(resource_dir.join("browser-integration-native-host.sh"));
        candidates.push(resource_dir.join("mnema_browser_integration_host"));
        candidates.push(
            resource_dir
                .join("target")
                .join("release")
                .join("mnema_browser_integration_host"),
        );
    }
    candidates.extend([
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("debug")
            .join("mnema_browser_integration_host"),
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("release")
            .join("mnema_browser_integration_host"),
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("..")
            .join("target")
            .join("debug")
            .join("mnema_browser_integration_host"),
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("..")
            .join("target")
            .join("release")
            .join("mnema_browser_integration_host"),
    ]);
    candidates
        .into_iter()
        .find(|path| path.exists())
        .map(|path| path.to_string_lossy().to_string())
        .ok_or_else(|| {
            "Native host binary was not found. Run `cargo build --manifest-path apps/desktop/src-tauri/Cargo.toml --bin mnema_browser_integration_host` first.".to_string()
        })
}

fn native_host_manifest_dirs() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    [
        "Google/Chrome",
        "Chromium",
        "BraveSoftware/Brave-Browser",
        "Microsoft Edge",
    ]
    .into_iter()
    .map(|browser| {
        home.join("Library")
            .join("Application Support")
            .join(browser)
            .join("NativeMessagingHosts")
    })
    .collect()
}

pub(crate) fn aggregate_browser_safety(
    app_handle: &tauri::AppHandle,
) -> BrowserSafetyAggregate {
    let Some(state) = app_handle.try_state::<BrowserIntegrationState>() else {
        return BrowserSafetyAggregate::Clear;
    };
    state
        .lock()
        .map(|mut runtime| runtime.browser_safety_state(super::runtime::now_unix_ms()))
        .unwrap_or(BrowserSafetyAggregate::Clear)
}

pub(crate) fn latest_browser_extension_metadata_url(
    app_handle: &tauri::AppHandle,
) -> Option<String> {
    app_handle
        .try_state::<BrowserIntegrationState>()?
        .lock()
        .ok()
        .and_then(|runtime| runtime.latest_metadata_url())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_secure_entry_sequences_are_rejected() {
        let mut runtime = BrowserIntegrationRuntime::default();
        let action = runtime.start_pairing(BrowserFamily::Safari, 100);
        let token = action.setup_url.unwrap();
        let token = token.split("token=").nth(1).unwrap();
        let mut signal = BrowserSecureEntrySignalV1 {
            version: 1,
            kind: SECURE_ENTRY_KIND.to_string(),
            browser_family: BrowserFamily::Safari,
            state: BrowserSecureEntryState::Active,
            reason: BrowserSecureEntryReason::FocusedPasswordControl,
            observed_at_unix_ms: 200,
            sequence: 10,
        };
        runtime
            .accept_secure_entry_signal(token, signal.clone(), 200)
            .expect("first signal accepted");
        signal.sequence = 9;
        assert_eq!(
            runtime.accept_secure_entry_signal(token, signal, 201),
            Err(BrowserSecureEntryReason::PageUnsupported)
        );
    }

    #[test]
    fn invalid_pairing_is_rejected_as_extension_not_paired() {
        let mut runtime = BrowserIntegrationRuntime::default();
        runtime.start_pairing(BrowserFamily::Safari, 100);
        let signal = BrowserSecureEntrySignalV1 {
            version: 1,
            kind: SECURE_ENTRY_KIND.to_string(),
            browser_family: BrowserFamily::Safari,
            state: BrowserSecureEntryState::Active,
            reason: BrowserSecureEntryReason::FocusedPasswordControl,
            observed_at_unix_ms: 200,
            sequence: 1,
        };

        assert_eq!(
            runtime.accept_secure_entry_signal("wrong-token", signal, 200),
            Err(BrowserSecureEntryReason::ExtensionNotPaired)
        );
        assert_eq!(runtime.browser_safety_state(200), BrowserSafetyAggregate::Clear);
    }

    #[test]
    fn explicit_pairing_token_marks_family_paired_before_focus_signal() {
        let mut runtime = BrowserIntegrationRuntime::default();
        let action = runtime.start_pairing(BrowserFamily::Chromium, 100);
        let token = action.setup_url.unwrap();
        let token = token.split("token=").nth(1).unwrap();

        runtime
            .accept_pairing_token(BrowserFamily::Chromium, token, 200)
            .expect("pairing token should be accepted");

        let status = runtime.family_status(BrowserFamily::Chromium);
        assert_eq!(status.pairing_state, BrowserIntegrationPairingState::Paired);
        assert_eq!(
            status.coverage_state,
            BrowserIntegrationCoverageState::Reliable
        );
        assert_eq!(status.secure_entry_state, BrowserSecureEntryState::Clear);
    }

    #[test]
    fn clear_signal_after_active_allows_resume_path() {
        let mut runtime = BrowserIntegrationRuntime::default();
        let action = runtime.start_pairing(BrowserFamily::Safari, 100);
        let token = action.setup_url.unwrap();
        let token = token.split("token=").nth(1).unwrap();
        runtime
            .accept_secure_entry_signal(
                token,
                BrowserSecureEntrySignalV1 {
                    version: 1,
                    kind: SECURE_ENTRY_KIND.to_string(),
                    browser_family: BrowserFamily::Safari,
                    state: BrowserSecureEntryState::Active,
                    reason: BrowserSecureEntryReason::FocusedPasswordControl,
                    observed_at_unix_ms: 200,
                    sequence: 1,
                },
                200,
            )
            .expect("active signal accepted");
        runtime
            .accept_secure_entry_signal(
                token,
                BrowserSecureEntrySignalV1 {
                    version: 1,
                    kind: SECURE_ENTRY_KIND.to_string(),
                    browser_family: BrowserFamily::Safari,
                    state: BrowserSecureEntryState::Clear,
                    reason: BrowserSecureEntryReason::NoFocusedCredentialControl,
                    observed_at_unix_ms: 300,
                    sequence: 2,
                },
                300,
            )
            .expect("clear signal accepted");

        assert_eq!(runtime.browser_safety_state(300), BrowserSafetyAggregate::Clear);
    }

    #[test]
    fn heartbeat_timeout_while_active_fails_closed() {
        let mut runtime = BrowserIntegrationRuntime::default();
        let action = runtime.start_pairing(BrowserFamily::Chromium, 100);
        let token = action.setup_url.unwrap();
        let token = token.split("token=").nth(1).unwrap();
        runtime
            .accept_secure_entry_signal(
                token,
                BrowserSecureEntrySignalV1 {
                    version: 1,
                    kind: SECURE_ENTRY_KIND.to_string(),
                    browser_family: BrowserFamily::Chromium,
                    state: BrowserSecureEntryState::Active,
                    reason: BrowserSecureEntryReason::FocusedPasswordControl,
                    observed_at_unix_ms: 200,
                    sequence: 1,
                },
                200,
            )
            .expect("signal accepted");
        assert_eq!(runtime.browser_safety_state(3_500), BrowserSafetyAggregate::SourceLostFailClosed);
    }

    #[test]
    fn unavailable_before_active_does_not_suspend() {
        let mut runtime = BrowserIntegrationRuntime::default();
        assert_eq!(runtime.browser_safety_state(10_000), BrowserSafetyAggregate::Clear);
    }
}
