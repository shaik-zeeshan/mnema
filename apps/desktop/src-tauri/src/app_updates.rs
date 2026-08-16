use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};
use time::format_description::well_known::Rfc3339;
use url::Url;

use crate::{native_capture, windows};

pub const APP_UPDATE_STATUS_CHANGED_EVENT: &str = "app_update_status_changed";
const APP_UPDATE_AVAILABLE_NOTIFICATION_ID: &str = "app-update-available";
const APP_UPDATE_SETTINGS_FILE_NAME: &str = "app-update-settings.json";
// Update feeds live on Cloudflare R2 behind release.mnema.day (one latest.json
// per channel, written by the promote workflow), so they stay reachable even
// if the source repository goes private. See docs/release-process.md.
pub const STABLE_UPDATE_ENDPOINT: &str = "https://release.mnema.day/stable/latest.json";
pub const PREVIEW_UPDATE_ENDPOINT: &str = "https://release.mnema.day/preview/latest.json";

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
    /// Version the "update available" notification was last pushed for. The
    /// periodic check re-finds the same update every tick; without this it would
    /// re-push (and un-dismiss) the same notice every couple of hours.
    notified_version: Option<String>,
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

/// Whether capture is live enough that the install must stop it first.
///
/// Only the live-capture flags count. `source_sessions` is NOT a liveness
/// signal: a stopped session deliberately preserves it as finalized metadata
/// (see `stopped_session_from_runtime`).
fn capture_session_needs_stop_before_install(
    session: &capture_types::NativeCaptureSession,
) -> bool {
    session.is_running || session.is_user_paused
}

fn status_from_runtime(
    app_handle: &tauri::AppHandle,
    settings: AppUpdateSettings,
    runtime: &AppUpdateRuntime,
) -> AppUpdateStatus {
    let state = apply_running_build_window_gate(app_handle, runtime.state);
    let error = runtime.error.clone();

    AppUpdateStatus {
        app: app_info(app_handle),
        channel: settings.channel,
        state,
        update: runtime.update.clone(),
        progress: runtime.progress.clone(),
        error,
        last_checked_at_unix_ms: runtime.last_checked_at_unix_ms,
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
/// is decided in `run_update_check` (see [`decide_remote_update`]), so those
/// states pass through untouched. Pure over (status, build date) — the
/// app-handle wrapper below feeds it the cached gate status.
fn running_build_window_gate(
    state: AppUpdateState,
    status: Option<&capture_types::LicenseStatus>,
    build_date_ms: Option<i64>,
) -> AppUpdateState {
    if matches!(state, AppUpdateState::Idle | AppUpdateState::UpToDate)
        && build_out_of_window(status, build_date_ms)
    {
        AppUpdateState::AvailableOutOfWindow
    } else {
        state
    }
}

fn apply_running_build_window_gate(
    app_handle: &tauri::AppHandle,
    state: AppUpdateState,
) -> AppUpdateState {
    running_build_window_gate(
        state,
        crate::licensing::cached_status(app_handle).as_ref(),
        running_build_date_ms(),
    )
}

/// What `run_update_check` does with a found remote update: the stored state,
/// whether the update stays installable (`pending_update` kept), and whether
/// the "ready to install" notification fires. Out-of-window builds are surfaced
/// (version shown) but never installable and never nudge — the Settings surface
/// directs the owner to renew instead.
#[derive(Debug, PartialEq, Eq)]
struct RemoteUpdateDecision {
    state: AppUpdateState,
    installable: bool,
    notify: bool,
}

fn decide_remote_update(out_of_window: bool, notify_available: bool) -> RemoteUpdateDecision {
    RemoteUpdateDecision {
        state: if out_of_window {
            AppUpdateState::AvailableOutOfWindow
        } else {
            AppUpdateState::Available
        },
        installable: !out_of_window,
        notify: notify_available && !out_of_window,
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
            let decision = decide_remote_update(out_of_window, notify_available);
            {
                let runtime_state = app_handle.state::<AppUpdateRuntimeState>();
                let mut runtime = runtime_state
                    .lock()
                    .expect("app update runtime state poisoned");
                runtime.state = decision.state;
                runtime.pending_update = if decision.installable {
                    Some(update)
                } else {
                    None
                };
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
            // Push once per version: the periodic check re-finds the same update
            // every tick, and re-pushing would resurrect a dismissed notice.
            let already_notified = {
                let runtime_state = app_handle.state::<AppUpdateRuntimeState>();
                let mut runtime = runtime_state
                    .lock()
                    .expect("app update runtime state poisoned");
                let already = runtime.notified_version.as_deref() == Some(info.version.as_str());
                if decision.notify && !already {
                    runtime.notified_version = Some(info.version.clone());
                }
                already
            };
            if decision.notify && !already_notified {
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

/// How often a running app re-checks the feed. Mnema is left running for days,
/// so a startup-only check means a user can sit on a stale build indefinitely.
/// For reference: Sparkle defaults to 24 h (1 h hard minimum), Chrome's Omaha to
/// ~5 h. A tick is one GET of a small static R2 file, so the interval is a
/// notification-noise choice, not a bandwidth one.
const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(2 * 60 * 60);

/// Checks immediately (the first `interval` tick fires at once) and every
/// `UPDATE_CHECK_INTERVAL` after that, for the life of the process.
pub fn start_update_check_timer(app_handle: &tauri::AppHandle) {
    let app_handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(UPDATE_CHECK_INTERVAL);
        loop {
            interval.tick().await;
            let _ = run_update_check(&app_handle, true).await;
        }
    });
}

fn spawn_update_check(app_handle: &tauri::AppHandle) {
    let app_handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        let _ = run_update_check(&app_handle, false).await;
    });
}

#[tauri::command]
pub fn get_app_update_status(app_handle: tauri::AppHandle) -> AppUpdateStatus {
    current_status(&app_handle)
}

#[tauri::command]
pub async fn check_for_app_update(app_handle: tauri::AppHandle) -> AppUpdateStatus {
    // Piggyback the CRL refresh on a manual "check for updates" (ADR 0056).
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

async fn stop_capture_before_install(app_handle: &tauri::AppHandle) {
    let session = native_capture::current_native_capture_session(app_handle);
    if !capture_session_needs_stop_before_install(&session) {
        return;
    }
    native_capture::debug_log::log_info("stopping capture to install an app update");
    let stop_app_handle = app_handle.clone();
    match tauri::async_runtime::spawn_blocking(move || {
        native_capture::stop_native_capture_from_app_handle(&stop_app_handle)
    })
    .await
    {
        Ok(Ok(_)) => {}
        // Best-effort: a failed stop must not strand an already-downloaded
        // update. The install proceeds; the restart's graceful exit gets
        // another chance to finalize capture.
        Ok(Err(error)) => native_capture::debug_log::log_warn(format!(
            "stopping capture before update install failed: {error:?}"
        )),
        Err(error) => native_capture::debug_log::log_warn(format!(
            "stopping capture before update install panicked: {error}"
        )),
    }
}

#[tauri::command]
pub async fn install_app_update(app_handle: tauri::AppHandle) -> AppUpdateStatus {
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

    // The download ran alongside capture; the bundle swap must not. Stop it
    // ourselves rather than refusing the install and telling the user to —
    // the graceful stop finalizes the in-flight segment, and the restart that
    // follows re-arms capture through the normal auto-start path.
    stop_capture_before_install(&app_handle).await;

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

/// The only reason to refuse: nothing was installed. A live capture is NOT a
/// reason — `request_graceful_restart_after_update` finalizes it before
/// relaunching, same as any other graceful exit.
fn restart_after_update_error(restart_required: bool) -> Option<AppUpdateError> {
    (!restart_required).then(|| {
        AppUpdateError::new(
            AppUpdateErrorKind::Install,
            "No installed update is waiting for restart.",
        )
    })
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
    if let Some(error) = restart_after_update_error(restart_required) {
        set_runtime_error(&app_handle, AppUpdateState::Failed, error.clone());
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
    fn install_stops_a_running_capture_session_first() {
        let mut session = stopped_session();
        session.is_running = true;

        assert!(capture_session_needs_stop_before_install(&session));
    }

    #[test]
    fn install_stops_a_user_paused_capture_session_first() {
        let mut session = stopped_session();
        session.is_user_paused = true;

        assert!(capture_session_needs_stop_before_install(&session));
    }

    #[test]
    fn stopped_session_with_finalized_source_metadata_needs_no_stop() {
        // A stopped session preserves `source_sessions` as finalized metadata
        // (see `stopped_session_from_runtime`). That is not a liveness signal, so
        // it must not trigger a pointless stop before the install.
        let mut session = stopped_session();
        session.source_sessions = Some(SourceSessions {
            screen: Some(SourceSessionMeta {
                session_id: "s1".into(),
                started_at_unix_ms: 1,
            }),
            microphone: None,
            system_audio: None,
        });

        assert!(!capture_session_needs_stop_before_install(&session));
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
        let error = restart_after_update_error(false)
            .expect("missing pending update should reject restart");

        assert_eq!(error.kind, AppUpdateErrorKind::Install);
    }

    #[test]
    fn restart_command_allows_restart_while_capture_is_running() {
        // The graceful restart stops and finalizes capture itself.
        assert_eq!(restart_after_update_error(true), None);
    }

    #[test]
    fn update_window_gate_only_declines_out_of_window_licensed_builds() {
        let licensed = |update_through_ms| capture_types::LicenseStatus::Licensed {
            update_through_ms,
            in_window: true,
            email: "a@b.c".into(),
            name: String::new(),
            activation: capture_types::Activation::Activated,
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

    fn out_of_window_licensed() -> capture_types::LicenseStatus {
        capture_types::LicenseStatus::Licensed {
            update_through_ms: 1_000,
            in_window: false,
            email: "a@b.c".into(),
            name: String::new(),
            activation: capture_types::Activation::Activated,
        }
    }

    #[test]
    fn running_build_window_gate_flips_only_resting_states() {
        let status = out_of_window_licensed();
        let build_after_window = Some(2_000);

        // Idle / UpToDate → AvailableOutOfWindow when the running build is past
        // the owner's window (the fresh-install-after-lapse edge).
        for state in [AppUpdateState::Idle, AppUpdateState::UpToDate] {
            assert_eq!(
                running_build_window_gate(state, Some(&status), build_after_window),
                AppUpdateState::AvailableOutOfWindow,
            );
        }
        // Every other state passes through untouched (remote gating is decided
        // in run_update_check, in-flight states must not be rewritten).
        for state in [
            AppUpdateState::Checking,
            AppUpdateState::Available,
            AppUpdateState::AvailableOutOfWindow,
            AppUpdateState::Downloading,
            AppUpdateState::Installing,
            AppUpdateState::RestartRequired,
            AppUpdateState::Incompatible,
            AppUpdateState::Failed,
        ] {
            assert_eq!(
                running_build_window_gate(state, Some(&status), build_after_window),
                state,
            );
        }
    }

    #[test]
    fn running_build_window_gate_never_flips_in_window_or_unknown() {
        // In-window build, missing build date, or no computed status → resting
        // states stay resting.
        for (status, build_date) in [
            (Some(out_of_window_licensed()), Some(500)), // build within window
            (Some(out_of_window_licensed()), None),      // no build date
            (None, Some(2_000)),                         // gate not yet computed
        ] {
            assert_eq!(
                running_build_window_gate(AppUpdateState::Idle, status.as_ref(), build_date),
                AppUpdateState::Idle,
            );
        }
    }

    #[test]
    fn out_of_window_remote_update_is_never_installable_and_never_notifies() {
        // Out of window: surfaced, but pending_update is dropped and the
        // "ready to install" nudge is suppressed even on a notify check.
        assert_eq!(
            decide_remote_update(true, true),
            RemoteUpdateDecision {
                state: AppUpdateState::AvailableOutOfWindow,
                installable: false,
                notify: false,
            },
        );
        assert_eq!(
            decide_remote_update(true, false).state,
            AppUpdateState::AvailableOutOfWindow,
        );
        // In window: installable; notification tracks the caller's flag.
        assert_eq!(
            decide_remote_update(false, true),
            RemoteUpdateDecision {
                state: AppUpdateState::Available,
                installable: true,
                notify: true,
            },
        );
        assert_eq!(
            decide_remote_update(false, false),
            RemoteUpdateDecision {
                state: AppUpdateState::Available,
                installable: true,
                notify: false,
            },
        );
    }
}
