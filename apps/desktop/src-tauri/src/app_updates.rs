use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};
use time::format_description::well_known::Rfc3339;
use url::Url;

use crate::{native_capture, windows};

pub const APP_UPDATE_STATUS_CHANGED_EVENT: &str = "app_update_status_changed";
const APP_UPDATE_AVAILABLE_NOTIFICATION_ID: &str = "app-update-available";
const APP_UPDATE_SETTINGS_FILE_NAME: &str = "app-update-settings.json";
pub const STABLE_UPDATE_ENDPOINT: &str =
    "https://github.com/shaik-zeeshan/mnema/releases/latest/download/latest.json";
pub const PREVIEW_UPDATE_ENDPOINT: &str =
    "https://shaik-zeeshan.github.io/mnema/updates/preview/latest.json";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AppUpdateChannel {
    Stable,
    Preview,
}

impl Default for AppUpdateChannel {
    fn default() -> Self {
        Self::Stable
    }
}

impl AppUpdateChannel {
    pub(crate) fn endpoint(self) -> &'static str {
        match self {
            Self::Stable => STABLE_UPDATE_ENDPOINT,
            Self::Preview => PREVIEW_UPDATE_ENDPOINT,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AppUpdateState {
    Idle,
    Checking,
    UpToDate,
    Available,
    /// A newer build exists (or the running build itself is) past a Licensed
    /// owner's Update Window. NOT installable — the UI directs the owner to renew
    /// or fetch the newest covered build (Perpetual Fallback). Never a hard lock;
    /// capture and recorded history are untouched.
    AvailableOutOfWindow,
    Downloading,
    Installing,
    RestartRequired,
    RecordingBlocked,
    Incompatible,
    Failed,
}

impl Default for AppUpdateState {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AppUpdateErrorKind {
    Network,
    Feed,
    Incompatible,
    Verification,
    Install,
    RecordingActive,
    Unknown,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateError {
    pub kind: AppUpdateErrorKind,
    pub message: String,
}

impl AppUpdateError {
    fn new(kind: AppUpdateErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for AppUpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for AppUpdateError {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateSettings {
    pub channel: AppUpdateChannel,
}

impl Default for AppUpdateSettings {
    fn default() -> Self {
        Self {
            channel: AppUpdateChannel::Stable,
        }
    }
}

#[derive(Debug, Default)]
pub struct AppUpdateSettingsRuntime {
    settings: Option<AppUpdateSettings>,
}

pub type AppUpdateSettingsState = Mutex<AppUpdateSettingsRuntime>;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateAppInfo {
    pub product_name: String,
    pub version: String,
    pub identifier: String,
    pub platform: String,
    pub arch: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateInfo {
    pub version: String,
    pub date: Option<String>,
    pub notes: Option<String>,
    pub channel: AppUpdateChannel,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateProgress {
    pub downloaded_bytes: u64,
    pub content_length_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateStatus {
    pub app: AppUpdateAppInfo,
    pub channel: AppUpdateChannel,
    pub state: AppUpdateState,
    pub update: Option<AppUpdateInfo>,
    pub progress: Option<AppUpdateProgress>,
    pub error: Option<AppUpdateError>,
    pub last_checked_at_unix_ms: Option<u64>,
    pub recording_active: bool,
}

#[derive(Default)]
pub struct AppUpdateRuntime {
    state: AppUpdateState,
    pending_update: Option<Update>,
    update: Option<AppUpdateInfo>,
    progress: Option<AppUpdateProgress>,
    error: Option<AppUpdateError>,
    last_checked_at_unix_ms: Option<u64>,
    restart_required: bool,
}

pub type AppUpdateRuntimeState = Mutex<AppUpdateRuntime>;

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn settings_file_path(app_handle: &tauri::AppHandle) -> PathBuf {
    if let Ok(config_dir) = app_handle.path().app_config_dir() {
        return config_dir.join(APP_UPDATE_SETTINGS_FILE_NAME);
    }

    PathBuf::from(".mnema").join(APP_UPDATE_SETTINGS_FILE_NAME)
}

pub(crate) fn load_app_update_settings_from_path(path: &Path) -> AppUpdateSettings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<AppUpdateSettings>(&raw).ok())
        .unwrap_or_default()
}

fn current_settings(
    app_handle: &tauri::AppHandle,
    state: &AppUpdateSettingsState,
) -> AppUpdateSettings {
    let mut runtime = state.lock().expect("app update settings state poisoned");
    if let Some(settings) = runtime.settings.clone() {
        return settings;
    }

    let settings = load_app_update_settings_from_path(&settings_file_path(app_handle));
    runtime.settings = Some(settings.clone());
    settings
}

fn persist_settings(
    app_handle: &tauri::AppHandle,
    state: &AppUpdateSettingsState,
    settings: AppUpdateSettings,
) -> Result<AppUpdateSettings, AppUpdateError> {
    let path = settings_file_path(app_handle);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            AppUpdateError::new(
                AppUpdateErrorKind::Unknown,
                format!("Failed to create update settings directory: {error}"),
            )
        })?;
    }

    let serialized = serde_json::to_string_pretty(&settings).map_err(|error| {
        AppUpdateError::new(
            AppUpdateErrorKind::Unknown,
            format!("Failed to serialize update settings: {error}"),
        )
    })?;
    std::fs::write(path, serialized).map_err(|error| {
        AppUpdateError::new(
            AppUpdateErrorKind::Unknown,
            format!("Failed to persist update settings: {error}"),
        )
    })?;

    let mut runtime = state.lock().expect("app update settings state poisoned");
    runtime.settings = Some(settings.clone());
    Ok(settings)
}

#[cfg(test)]
pub fn channel_endpoint(channel: AppUpdateChannel) -> &'static str {
    channel.endpoint()
}

#[cfg(test)]
pub fn channel_requires_additional_per_install_state(_channel: AppUpdateChannel) -> bool {
    false
}

fn app_info(app_handle: &tauri::AppHandle) -> AppUpdateAppInfo {
    let config = app_handle.config();
    AppUpdateAppInfo {
        product_name: config
            .product_name
            .clone()
            .unwrap_or_else(|| app_handle.package_info().name.clone()),
        version: config
            .version
            .clone()
            .unwrap_or_else(|| app_handle.package_info().version.to_string()),
        identifier: config.identifier.clone(),
        platform: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
    }
}

fn active_capture_session_blocks_install(session: &capture_types::NativeCaptureSession) -> bool {
    // Only the live-capture flags gate the install. `source_sessions` is NOT a
    // liveness signal: a stopped session deliberately preserves it as finalized
    // metadata (see `stopped_session_from_runtime`), so checking it left install
    // blocked with "RECORDING ACTIVE" long after recording stopped.
    session.is_running || session.is_user_paused
}

fn current_recording_active(app_handle: &tauri::AppHandle) -> bool {
    active_capture_session_blocks_install(&native_capture::current_native_capture_session(
        app_handle,
    ))
}

/// Derive the user-facing state and error from raw runtime fields.
///
/// When recording stops after an install was blocked, the runtime still carries
/// the `RecordingBlocked` state and its `RecordingActive` error. We remap the
/// state back to what the pending update warrants and drop the now-stale
/// "stop recording" error so the surfaced status stays self-consistent (no
/// `Available`/`recordingActive: false` paired with a "stop recording" error).
fn derive_state_and_error(
    runtime_state: AppUpdateState,
    restart_required: bool,
    has_update: bool,
    recording_active: bool,
    error: Option<AppUpdateError>,
) -> (AppUpdateState, Option<AppUpdateError>) {
    let recording_unblocked =
        runtime_state == AppUpdateState::RecordingBlocked && !recording_active;
    let state = if recording_unblocked {
        if restart_required {
            AppUpdateState::RestartRequired
        } else if has_update {
            AppUpdateState::Available
        } else {
            AppUpdateState::Idle
        }
    } else {
        runtime_state
    };
    let error = error.filter(|error| {
        !(recording_unblocked && error.kind == AppUpdateErrorKind::RecordingActive)
    });
    (state, error)
}

fn status_from_runtime(
    app_handle: &tauri::AppHandle,
    settings: AppUpdateSettings,
    runtime: &AppUpdateRuntime,
) -> AppUpdateStatus {
    let recording_active = current_recording_active(app_handle);
    let (state, error) = derive_state_and_error(
        runtime.state,
        runtime.restart_required,
        runtime.update.is_some(),
        recording_active,
        runtime.error.clone(),
    );
    let state = apply_running_build_window_gate(app_handle, state);

    AppUpdateStatus {
        app: app_info(app_handle),
        channel: settings.channel,
        state,
        update: runtime.update.clone(),
        progress: runtime.progress.clone(),
        error,
        last_checked_at_unix_ms: runtime.last_checked_at_unix_ms,
        recording_active,
    }
}

fn current_status(app_handle: &tauri::AppHandle) -> AppUpdateStatus {
    let settings = current_settings(
        app_handle,
        app_handle.state::<AppUpdateSettingsState>().inner(),
    );
    let runtime_state = app_handle.state::<AppUpdateRuntimeState>();
    let runtime = runtime_state
        .lock()
        .expect("app update runtime state poisoned");
    status_from_runtime(app_handle, settings, &runtime)
}

fn emit_current_status(app_handle: &tauri::AppHandle) {
    let status = current_status(app_handle);
    let _ = app_handle.emit(APP_UPDATE_STATUS_CHANGED_EVENT, status);
}

fn update_info_from_update(update: &Update, channel: AppUpdateChannel) -> AppUpdateInfo {
    AppUpdateInfo {
        version: update.version.clone(),
        date: update.date.and_then(|date| date.format(&Rfc3339).ok()),
        notes: update.body.clone(),
        channel,
    }
}

/// The remote build's release date as unix ms (the manifest `pub_date`, carried
/// on `update.date`). `None` when the manifest omitted a date.
fn update_release_date_ms(update: &Update) -> Option<i64> {
    update
        .date
        .map(|date| (date.unix_timestamp_nanos() / 1_000_000) as i64)
}

/// The running build's own release date as unix ms, stamped by `build.rs`.
/// `None` if the env var is somehow absent (never block on missing data).
fn running_build_date_ms() -> Option<i64> {
    option_env!("MNEMA_BUILD_DATE_MS").and_then(|raw| raw.parse::<i64>().ok())
}

/// The Update Window gate. A build dated strictly after the owner's
/// `update_through_ms` is outside the window. Only `Licensed` users are gated —
/// Trial / ReadOnly / TrialNotStarted / `None` (gate not yet computed) have no
/// Update Window, so updates flow normally. A missing `build_date_ms` is treated
/// as in-window: never decline an update on absent data.
fn build_out_of_window(
    status: Option<&capture_types::LicenseStatus>,
    build_date_ms: Option<i64>,
) -> bool {
    matches!(
        status,
        Some(capture_types::LicenseStatus::Licensed { update_through_ms, .. })
            if build_date_ms.is_some_and(|date| date > *update_through_ms)
    )
}

/// Fresh-install-after-lapse edge: if the running build itself is past the
/// owner's Update Window, surface `AvailableOutOfWindow` in place of a resting
/// `Idle`/`UpToDate` state so the UI can direct the owner. Remote-update gating
/// is decided in `run_update_check`, so those states pass through untouched.
fn apply_running_build_window_gate(
    app_handle: &tauri::AppHandle,
    state: AppUpdateState,
) -> AppUpdateState {
    if matches!(state, AppUpdateState::Idle | AppUpdateState::UpToDate)
        && build_out_of_window(
            crate::licensing::cached_status(app_handle).as_ref(),
            running_build_date_ms(),
        )
    {
        AppUpdateState::AvailableOutOfWindow
    } else {
        state
    }
}

pub(crate) fn map_update_error_kind(error: &tauri_plugin_updater::Error) -> AppUpdateErrorKind {
    use tauri_plugin_updater::Error;

    match error {
        Error::Reqwest(_) | Error::Network(_) | Error::InsecureTransportProtocol => {
            AppUpdateErrorKind::Network
        }
        Error::Serialization(_)
        | Error::ReleaseNotFound
        | Error::UrlParse(_)
        | Error::EmptyEndpoints
        | Error::Http(_)
        | Error::InvalidHeaderName(_)
        | Error::InvalidHeaderValue(_) => AppUpdateErrorKind::Feed,
        Error::UnsupportedArch
        | Error::UnsupportedOs
        | Error::TargetNotFound(_)
        | Error::TargetsNotFound(_) => AppUpdateErrorKind::Incompatible,
        Error::Minisign(_) | Error::Base64(_) | Error::SignatureUtf8(_) => {
            AppUpdateErrorKind::Verification
        }
        Error::FailedToDetermineExtractPath
        | Error::TempDirNotOnSameMountPoint
        | Error::BinaryNotFoundInArchive
        | Error::TempDirNotFound
        | Error::AuthenticationFailed
        | Error::DebInstallFailed
        | Error::PackageInstallFailed
        | Error::InvalidUpdaterFormat => AppUpdateErrorKind::Install,
        Error::Io(_) | Error::Semver(_) | Error::FormatDate | Error::Tauri(_) => {
            AppUpdateErrorKind::Unknown
        }
        #[allow(unreachable_patterns)]
        _ => AppUpdateErrorKind::Unknown,
    }
}

fn user_facing_error_message(kind: AppUpdateErrorKind) -> &'static str {
    match kind {
        AppUpdateErrorKind::Network => "Could not reach the update feed.",
        AppUpdateErrorKind::Feed => "Update feed could not be read.",
        AppUpdateErrorKind::Incompatible => "No compatible update is available for this Mac.",
        AppUpdateErrorKind::Verification => "Update could not be verified.",
        AppUpdateErrorKind::Install => "Update could not be installed.",
        AppUpdateErrorKind::RecordingActive => "Stop recording to install this update.",
        AppUpdateErrorKind::Unknown => "Update failed.",
    }
}

fn app_update_error_from_updater_error(
    context: &str,
    error: tauri_plugin_updater::Error,
) -> AppUpdateError {
    let kind = map_update_error_kind(&error);
    let message = user_facing_error_message(kind).to_string();
    native_capture::debug_log::log_warn(format!(
        "app update {context} failed: kind={kind:?}; error={error}"
    ));
    AppUpdateError::new(kind, message)
}

fn set_runtime_error(
    app_handle: &tauri::AppHandle,
    state: AppUpdateState,
    error: AppUpdateError,
) -> AppUpdateStatus {
    {
        let runtime_state = app_handle.state::<AppUpdateRuntimeState>();
        let mut runtime = runtime_state
            .lock()
            .expect("app update runtime state poisoned");
        runtime.state = state;
        runtime.error = Some(error);
        runtime.progress = None;
    }
    emit_current_status(app_handle);
    current_status(app_handle)
}

fn push_update_available_notification(app_handle: &tauri::AppHandle, update: &AppUpdateInfo) {
    native_capture::push_info_app_notification(
        app_handle,
        APP_UPDATE_AVAILABLE_NOTIFICATION_ID,
        "Mnema update available",
        &format!(
            "Version {} is ready to install from Settings.",
            update.version
        ),
        Some("about"),
        now_unix_ms(),
    );
}

#[cfg(test)]
fn startup_update_notification_for_update(
    update: &AppUpdateInfo,
    created_at_unix_ms: u64,
) -> native_capture::AppNotification {
    native_capture::AppNotification {
        id: APP_UPDATE_AVAILABLE_NOTIFICATION_ID.to_string(),
        severity: "info".to_string(),
        title: "Mnema update available".to_string(),
        message: format!(
            "Version {} is ready to install from Settings.",
            update.version
        ),
        created_at_unix_ms,
        action: Some(native_capture::AppNotificationAction::OpenSettingsTab {
            tab: "about".to_string(),
        }),
    }
}

async fn run_update_check(
    app_handle: &tauri::AppHandle,
    notify_available: bool,
) -> AppUpdateStatus {
    let settings = current_settings(
        app_handle,
        app_handle.state::<AppUpdateSettingsState>().inner(),
    );

    {
        let runtime_state = app_handle.state::<AppUpdateRuntimeState>();
        let mut runtime = runtime_state
            .lock()
            .expect("app update runtime state poisoned");
        if matches!(
            runtime.state,
            AppUpdateState::Checking | AppUpdateState::Downloading | AppUpdateState::Installing
        ) || runtime.restart_required
        {
            return status_from_runtime(app_handle, settings, &runtime);
        }
        runtime.state = AppUpdateState::Checking;
        runtime.pending_update = None;
        runtime.update = None;
        runtime.progress = None;
        runtime.error = None;
        runtime.last_checked_at_unix_ms = Some(now_unix_ms());
    }
    emit_current_status(app_handle);

    let endpoint = match Url::parse(settings.channel.endpoint()) {
        Ok(endpoint) => endpoint,
        Err(_error) => {
            return set_runtime_error(
                app_handle,
                AppUpdateState::Failed,
                AppUpdateError::new(
                    AppUpdateErrorKind::Feed,
                    user_facing_error_message(AppUpdateErrorKind::Feed),
                ),
            );
        }
    };

    let updater = match app_handle
        .updater_builder()
        .endpoints(vec![endpoint])
        .and_then(|builder| builder.build())
    {
        Ok(updater) => updater,
        Err(error) => {
            return set_runtime_error(
                app_handle,
                AppUpdateState::Failed,
                app_update_error_from_updater_error("setup", error),
            );
        }
    };

    match updater.check().await {
        Ok(Some(update)) => {
            let info = update_info_from_update(&update, settings.channel);
            // Update Window gate: a Licensed owner is never offered a build dated
            // after their `update_through`. We surface it (version shown) but keep
            // `pending_update = None` so it can't be installed — the UI directs the
            // owner to renew. Perpetual Fallback: their current build keeps working.
            let out_of_window = build_out_of_window(
                crate::licensing::cached_status(app_handle).as_ref(),
                update_release_date_ms(&update),
            );
            {
                let runtime_state = app_handle.state::<AppUpdateRuntimeState>();
                let mut runtime = runtime_state
                    .lock()
                    .expect("app update runtime state poisoned");
                runtime.state = if out_of_window {
                    AppUpdateState::AvailableOutOfWindow
                } else {
                    AppUpdateState::Available
                };
                runtime.pending_update = if out_of_window { None } else { Some(update) };
                runtime.update = Some(info.clone());
                runtime.progress = None;
                runtime.error = None;
                runtime.restart_required = false;
                runtime.last_checked_at_unix_ms = Some(now_unix_ms());
            }
            // If the channel changed while the check was in flight, the stored
            // result is from the wrong endpoint. Kick off a new check against
            // the current channel and return early without emitting the stale result.
            let current_channel = current_settings(
                app_handle,
                app_handle.state::<AppUpdateSettingsState>().inner(),
            )
            .channel;
            if current_channel != settings.channel {
                {
                    let runtime_state = app_handle.state::<AppUpdateRuntimeState>();
                    let mut runtime = runtime_state
                        .lock()
                        .expect("app update runtime state poisoned");
                    runtime.state = AppUpdateState::Idle;
                    runtime.pending_update = None;
                    runtime.update = None;
                    runtime.error = None;
                }
                spawn_update_check(app_handle);
                return current_status(app_handle);
            }
            // Out-of-window builds aren't installable, so don't push the
            // "ready to install from Settings" nudge — the Settings surface directs.
            if notify_available && !out_of_window {
                push_update_available_notification(app_handle, &info);
            }
            emit_current_status(app_handle);
            current_status(app_handle)
        }
        Ok(None) => {
            {
                let runtime_state = app_handle.state::<AppUpdateRuntimeState>();
                let mut runtime = runtime_state
                    .lock()
                    .expect("app update runtime state poisoned");
                runtime.state = AppUpdateState::UpToDate;
                runtime.pending_update = None;
                runtime.update = None;
                runtime.progress = None;
                runtime.error = None;
                runtime.restart_required = false;
                runtime.last_checked_at_unix_ms = Some(now_unix_ms());
            }
            let current_channel = current_settings(
                app_handle,
                app_handle.state::<AppUpdateSettingsState>().inner(),
            )
            .channel;
            if current_channel != settings.channel {
                {
                    let runtime_state = app_handle.state::<AppUpdateRuntimeState>();
                    let mut runtime = runtime_state
                        .lock()
                        .expect("app update runtime state poisoned");
                    runtime.state = AppUpdateState::Idle;
                }
                spawn_update_check(app_handle);
                return current_status(app_handle);
            }
            emit_current_status(app_handle);
            current_status(app_handle)
        }
        Err(error) => {
            let mapped = app_update_error_from_updater_error("check", error);
            let state = if mapped.kind == AppUpdateErrorKind::Incompatible {
                AppUpdateState::Incompatible
            } else {
                AppUpdateState::Failed
            };
            let current_channel = current_settings(
                app_handle,
                app_handle.state::<AppUpdateSettingsState>().inner(),
            )
            .channel;
            if current_channel != settings.channel {
                {
                    let runtime_state = app_handle.state::<AppUpdateRuntimeState>();
                    let mut runtime = runtime_state
                        .lock()
                        .expect("app update runtime state poisoned");
                    runtime.state = AppUpdateState::Idle;
                    runtime.error = None;
                }
                spawn_update_check(app_handle);
                return current_status(app_handle);
            }
            set_runtime_error(app_handle, state, mapped)
        }
    }
}

pub fn initialize(app_handle: &tauri::AppHandle) {
    let settings = current_settings(
        app_handle,
        app_handle.state::<AppUpdateSettingsState>().inner(),
    );
    native_capture::debug_log::log_info(format!(
        "loaded app update settings (channel={:?}, endpoint={})",
        settings.channel,
        settings.channel.endpoint()
    ));
}

pub fn start_startup_update_check(app_handle: &tauri::AppHandle) {
    let app_handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        let _ = run_update_check(&app_handle, true).await;
    });
}

fn spawn_update_check(app_handle: &tauri::AppHandle) {
    let app_handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        let _ = run_update_check(&app_handle, false).await;
    });
}

/// Called whenever the native capture session changes. If install was previously
/// blocked by an active recording, re-emit the update status so the frontend
/// panel reflects the recording-stopped state immediately.
pub fn on_capture_session_changed(app_handle: &tauri::AppHandle) {
    let state = {
        let runtime_state = app_handle.state::<AppUpdateRuntimeState>();
        let runtime = runtime_state
            .lock()
            .expect("app update runtime state poisoned");
        runtime.state
    };
    if state == AppUpdateState::RecordingBlocked {
        emit_current_status(app_handle);
    }
}

#[tauri::command]
pub fn get_app_update_status(app_handle: tauri::AppHandle) -> AppUpdateStatus {
    current_status(&app_handle)
}

#[tauri::command]
pub async fn check_for_app_update(app_handle: tauri::AppHandle) -> AppUpdateStatus {
    // Piggyback the CRL refresh on a manual "check for updates" (ADR 0052).
    crate::crl_refresh::spawn_crl_refresh(app_handle.clone());
    run_update_check(&app_handle, false).await
}

#[tauri::command]
pub async fn set_app_update_channel(
    app_handle: tauri::AppHandle,
    channel: AppUpdateChannel,
) -> AppUpdateStatus {
    let settings_state = app_handle.state::<AppUpdateSettingsState>();
    let settings = AppUpdateSettings { channel };
    if let Err(error) = persist_settings(&app_handle, settings_state.inner(), settings) {
        return set_runtime_error(&app_handle, AppUpdateState::Failed, error);
    }

    {
        let runtime_state = app_handle.state::<AppUpdateRuntimeState>();
        let runtime = runtime_state
            .lock()
            .expect("app update runtime state poisoned");
        if runtime.restart_required
            || matches!(
                runtime.state,
                AppUpdateState::Checking | AppUpdateState::Downloading | AppUpdateState::Installing
            )
        {
            drop(runtime);
            emit_current_status(&app_handle);
            return current_status(&app_handle);
        }
    }

    run_update_check(&app_handle, false).await
}

#[tauri::command]
pub async fn install_app_update(app_handle: tauri::AppHandle) -> AppUpdateStatus {
    let session = native_capture::current_native_capture_session(&app_handle);
    if active_capture_session_blocks_install(&session) {
        return set_runtime_error(
            &app_handle,
            AppUpdateState::RecordingBlocked,
            AppUpdateError::new(
                AppUpdateErrorKind::RecordingActive,
                user_facing_error_message(AppUpdateErrorKind::RecordingActive),
            ),
        );
    }

    let update = {
        let runtime_state = app_handle.state::<AppUpdateRuntimeState>();
        let mut runtime = runtime_state
            .lock()
            .expect("app update runtime state poisoned");
        if runtime.restart_required {
            return status_from_runtime(
                &app_handle,
                current_settings(
                    &app_handle,
                    app_handle.state::<AppUpdateSettingsState>().inner(),
                ),
                &runtime,
            );
        }
        if matches!(
            runtime.state,
            AppUpdateState::Downloading | AppUpdateState::Installing
        ) {
            return status_from_runtime(
                &app_handle,
                current_settings(
                    &app_handle,
                    app_handle.state::<AppUpdateSettingsState>().inner(),
                ),
                &runtime,
            );
        }
        let Some(update) = runtime.pending_update.clone() else {
            runtime.state = AppUpdateState::Failed;
            runtime.error = Some(AppUpdateError::new(
                AppUpdateErrorKind::Install,
                "Check for updates before installing.",
            ));
            return status_from_runtime(
                &app_handle,
                current_settings(
                    &app_handle,
                    app_handle.state::<AppUpdateSettingsState>().inner(),
                ),
                &runtime,
            );
        };
        runtime.state = AppUpdateState::Downloading;
        runtime.progress = Some(AppUpdateProgress {
            downloaded_bytes: 0,
            content_length_bytes: None,
        });
        runtime.error = None;
        update
    };
    emit_current_status(&app_handle);

    let progress_app_handle = app_handle.clone();
    let progress_result = update
        .download(
            move |chunk_length, content_length| {
                {
                    let runtime_state = progress_app_handle.state::<AppUpdateRuntimeState>();
                    let mut runtime = runtime_state
                        .lock()
                        .expect("app update runtime state poisoned");
                    let downloaded = runtime
                        .progress
                        .as_ref()
                        .map(|progress| progress.downloaded_bytes)
                        .unwrap_or(0)
                        .saturating_add(chunk_length as u64);
                    runtime.state = AppUpdateState::Downloading;
                    runtime.progress = Some(AppUpdateProgress {
                        downloaded_bytes: downloaded,
                        content_length_bytes: content_length,
                    });
                }
                emit_current_status(&progress_app_handle);
            },
            {
                let app_handle = app_handle.clone();
                move || {
                    {
                        let runtime_state = app_handle.state::<AppUpdateRuntimeState>();
                        let mut runtime = runtime_state
                            .lock()
                            .expect("app update runtime state poisoned");
                        runtime.state = AppUpdateState::Installing;
                    }
                    emit_current_status(&app_handle);
                }
            },
        )
        .await;

    let bytes = match progress_result {
        Ok(bytes) => bytes,
        Err(error) => {
            return set_runtime_error(
                &app_handle,
                AppUpdateState::Failed,
                app_update_error_from_updater_error("download", error),
            );
        }
    };

    let session = native_capture::current_native_capture_session(&app_handle);
    if active_capture_session_blocks_install(&session) {
        return set_runtime_error(
            &app_handle,
            AppUpdateState::RecordingBlocked,
            AppUpdateError::new(
                AppUpdateErrorKind::RecordingActive,
                user_facing_error_message(AppUpdateErrorKind::RecordingActive),
            ),
        );
    }

    {
        let runtime_state = app_handle.state::<AppUpdateRuntimeState>();
        let mut runtime = runtime_state
            .lock()
            .expect("app update runtime state poisoned");
        runtime.state = AppUpdateState::Installing;
    }
    emit_current_status(&app_handle);

    if let Err(error) = update.install(bytes) {
        return set_runtime_error(
            &app_handle,
            AppUpdateState::Failed,
            app_update_error_from_updater_error("install", error),
        );
    }

    {
        let runtime_state = app_handle.state::<AppUpdateRuntimeState>();
        let mut runtime = runtime_state
            .lock()
            .expect("app update runtime state poisoned");
        runtime.state = AppUpdateState::RestartRequired;
        runtime.progress = None;
        runtime.error = None;
        runtime.restart_required = true;
    }
    emit_current_status(&app_handle);
    current_status(&app_handle)
}

fn restart_after_update_error(
    restart_required: bool,
    session: &capture_types::NativeCaptureSession,
) -> Option<AppUpdateError> {
    if !restart_required {
        return Some(AppUpdateError::new(
            AppUpdateErrorKind::Install,
            "No installed update is waiting for restart.",
        ));
    }
    if active_capture_session_blocks_install(session) {
        return Some(AppUpdateError::new(
            AppUpdateErrorKind::RecordingActive,
            user_facing_error_message(AppUpdateErrorKind::RecordingActive),
        ));
    }
    None
}

#[tauri::command]
pub fn restart_after_app_update(app_handle: tauri::AppHandle) -> Result<(), AppUpdateError> {
    let restart_required = {
        let runtime_state = app_handle.state::<AppUpdateRuntimeState>();
        let restart_required = runtime_state
            .lock()
            .expect("app update runtime state poisoned")
            .restart_required;
        restart_required
    };
    let session = native_capture::current_native_capture_session(&app_handle);
    if let Some(error) = restart_after_update_error(restart_required, &session) {
        let state = if error.kind == AppUpdateErrorKind::RecordingActive {
            AppUpdateState::RecordingBlocked
        } else {
            AppUpdateState::Failed
        };
        set_runtime_error(&app_handle, state, error.clone());
        return Err(error);
    }

    windows::request_graceful_restart_after_update(&app_handle);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use capture_types::{NativeCaptureSession, SourceSessionMeta, SourceSessions};

    fn stopped_session() -> NativeCaptureSession {
        NativeCaptureSession {
            is_running: false,
            is_inactivity_paused: false,
            is_user_paused: false,
            is_low_disk_suspended: false,
            requested_sources: None,
            output_files: None,
            source_sessions: None,
        }
    }

    #[test]
    fn default_settings_loads_stable_when_no_config_exists() {
        let dir = tempfile::tempdir().expect("temp dir should be created");
        let settings = load_app_update_settings_from_path(&dir.path().join("missing.json"));

        assert_eq!(settings.channel, AppUpdateChannel::Stable);
    }

    #[test]
    fn preview_channel_persists_and_reloads() {
        let dir = tempfile::tempdir().expect("temp dir should be created");
        let path = dir.path().join("app-update-settings.json");
        std::fs::write(
            &path,
            serde_json::to_string(&AppUpdateSettings {
                channel: AppUpdateChannel::Preview,
            })
            .expect("settings should serialize"),
        )
        .expect("settings should write");

        let settings = load_app_update_settings_from_path(&path);

        assert_eq!(settings.channel, AppUpdateChannel::Preview);
    }

    #[test]
    fn invalid_settings_file_falls_back_to_stable() {
        let dir = tempfile::tempdir().expect("temp dir should be created");
        let path = dir.path().join("app-update-settings.json");
        std::fs::write(&path, "{not-json").expect("invalid settings should write");

        let settings = load_app_update_settings_from_path(&path);

        assert_eq!(settings.channel, AppUpdateChannel::Stable);
    }

    #[test]
    fn channel_endpoint_selection_returns_stable_and_preview_urls() {
        assert_eq!(
            channel_endpoint(AppUpdateChannel::Stable),
            STABLE_UPDATE_ENDPOINT
        );
        assert_eq!(
            channel_endpoint(AppUpdateChannel::Preview),
            PREVIEW_UPDATE_ENDPOINT
        );
    }

    #[test]
    fn preview_channel_requires_no_extra_per_install_state_after_opt_in() {
        assert!(!channel_requires_additional_per_install_state(
            AppUpdateChannel::Preview
        ));
    }

    #[test]
    fn install_is_blocked_when_current_capture_session_is_running() {
        let mut session = stopped_session();
        session.is_running = true;

        assert!(active_capture_session_blocks_install(&session));
    }

    #[test]
    fn install_is_blocked_during_user_capture_pause() {
        let mut session = stopped_session();
        session.is_user_paused = true;

        assert!(active_capture_session_blocks_install(&session));
    }

    #[test]
    fn install_is_not_blocked_when_stopped_session_retains_finalized_source_metadata() {
        // A stopped session preserves `source_sessions` as finalized metadata
        // (see `stopped_session_from_runtime`). That is not a liveness signal, so
        // it must not keep the install stuck on "Stop recording to install".
        let mut session = stopped_session();
        session.source_sessions = Some(SourceSessions {
            screen: Some(SourceSessionMeta {
                session_id: "s1".into(),
                started_at_unix_ms: 1,
            }),
            microphone: None,
            system_audio: None,
        });

        assert!(!active_capture_session_blocks_install(&session));
    }

    #[test]
    fn startup_check_availability_notification_targets_about_settings() {
        let update = AppUpdateInfo {
            version: "0.3.0".into(),
            date: None,
            notes: None,
            channel: AppUpdateChannel::Stable,
        };

        let notification = startup_update_notification_for_update(&update, 42);

        assert_eq!(notification.id, APP_UPDATE_AVAILABLE_NOTIFICATION_ID);
        assert_eq!(notification.severity, "info");
        match notification.action {
            Some(native_capture::AppNotificationAction::OpenSettingsTab { tab }) => {
                assert_eq!(tab, "about");
            }
            _ => panic!("expected open settings action"),
        }
    }

    #[test]
    fn update_error_mapping_buckets_common_errors() {
        assert_eq!(
            map_update_error_kind(&tauri_plugin_updater::Error::ReleaseNotFound),
            AppUpdateErrorKind::Feed
        );
        assert_eq!(
            map_update_error_kind(&tauri_plugin_updater::Error::UnsupportedOs),
            AppUpdateErrorKind::Incompatible
        );
        assert_eq!(
            map_update_error_kind(&tauri_plugin_updater::Error::Network("offline".into())),
            AppUpdateErrorKind::Network
        );
        assert_eq!(
            map_update_error_kind(&tauri_plugin_updater::Error::PackageInstallFailed),
            AppUpdateErrorKind::Install
        );
    }

    #[test]
    fn restart_command_rejects_when_no_installed_update_is_pending() {
        let session = stopped_session();
        let error = restart_after_update_error(false, &session)
            .expect("missing pending update should reject restart");

        assert_eq!(error.kind, AppUpdateErrorKind::Install);
    }

    #[test]
    fn restart_command_rejects_if_capture_starts_before_restart() {
        let mut session = stopped_session();
        session.is_running = true;

        let error = restart_after_update_error(true, &session)
            .expect("running capture should reject restart");

        assert_eq!(error.kind, AppUpdateErrorKind::RecordingActive);
    }

    fn recording_active_error() -> AppUpdateError {
        AppUpdateError::new(
            AppUpdateErrorKind::RecordingActive,
            user_facing_error_message(AppUpdateErrorKind::RecordingActive),
        )
    }

    #[test]
    fn stopping_recording_clears_stale_recording_blocked_error() {
        let (state, error) = derive_state_and_error(
            AppUpdateState::RecordingBlocked,
            false,
            true,
            false,
            Some(recording_active_error()),
        );

        assert_eq!(state, AppUpdateState::Available);
        assert_eq!(error, None);
    }

    #[test]
    fn stopping_recording_restores_restart_required_without_stale_error() {
        let (state, error) = derive_state_and_error(
            AppUpdateState::RecordingBlocked,
            true,
            true,
            false,
            Some(recording_active_error()),
        );

        assert_eq!(state, AppUpdateState::RestartRequired);
        assert_eq!(error, None);
    }

    #[test]
    fn recording_block_keeps_its_error_while_recording_is_active() {
        let (state, error) = derive_state_and_error(
            AppUpdateState::RecordingBlocked,
            false,
            true,
            true,
            Some(recording_active_error()),
        );

        assert_eq!(state, AppUpdateState::RecordingBlocked);
        assert_eq!(
            error.map(|error| error.kind),
            Some(AppUpdateErrorKind::RecordingActive)
        );
    }

    #[test]
    fn update_window_gate_only_declines_out_of_window_licensed_builds() {
        let licensed = |update_through_ms| capture_types::LicenseStatus::Licensed {
            update_through_ms,
            in_window: true,
            email: "a@b.c".into(),
        };

        // Build released after the window → out of window.
        assert!(build_out_of_window(Some(&licensed(1_000)), Some(2_000)));
        // Build within the window → allowed.
        assert!(!build_out_of_window(Some(&licensed(2_000)), Some(1_000)));
        // Build exactly at the boundary → allowed (`<=` is in window).
        assert!(!build_out_of_window(Some(&licensed(1_000)), Some(1_000)));
        // Missing build date → never decline.
        assert!(!build_out_of_window(Some(&licensed(1_000)), None));
        // Non-Licensed states have no Update Window: never gated.
        assert!(!build_out_of_window(
            Some(&capture_types::LicenseStatus::ReadOnly),
            Some(9_999)
        ));
        assert!(!build_out_of_window(
            Some(&capture_types::LicenseStatus::Trial {
                days_left: 3,
                trial_end_ms: 0
            }),
            Some(9_999)
        ));
        // Gate not yet computed → updates flow.
        assert!(!build_out_of_window(None, Some(9_999)));
    }

    #[test]
    fn non_recording_failures_keep_their_error() {
        let failure = AppUpdateError::new(AppUpdateErrorKind::Network, "boom");
        let (state, error) = derive_state_and_error(
            AppUpdateState::Failed,
            false,
            false,
            false,
            Some(failure.clone()),
        );

        assert_eq!(state, AppUpdateState::Failed);
        assert_eq!(error, Some(failure));
    }
}
