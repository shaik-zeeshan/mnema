//! The updater's status (the state machine's states and the wire payload), the
//! license Update Window gate, and the pure decisions the check/install flow makes.

use serde::Serialize;
use std::time::Duration;

use super::settings::{AppUpdateChannel, AppUpdateSettings};
use crate::{AppCore, CoreEvent};

pub const APP_UPDATE_STATUS_CHANGED_EVENT: &str = "app_update_status_changed";

/// How often a running app re-checks the feed. Mnema is left running for days,
/// so a startup-only check means a user can sit on a stale build indefinitely.
/// For reference: Sparkle defaults to 24 h (1 h hard minimum), Chrome's Omaha to
/// ~5 h. A tick is one GET of a small static R2 file, so the interval is a
/// notification-noise choice, not a bandwidth one.
pub const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(2 * 60 * 60);

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
    pub fn new(kind: AppUpdateErrorKind, message: impl Into<String>) -> Self {
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
    /// Archive size in bytes from the feed's `platforms[k].size` (written by
    /// the promote workflow). `None` for feeds published before it existed.
    pub size_bytes: Option<u64>,
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

/// Publish a status. The Tauri host also refreshes the tray's update row here.
pub fn emit_status(core: &AppCore, status: AppUpdateStatus) {
    core.host().emit(CoreEvent::AppUpdateStatusChanged(status));
}

pub fn user_facing_error_message(kind: AppUpdateErrorKind) -> &'static str {
    match kind {
        AppUpdateErrorKind::Network => "Could not reach the update feed.",
        AppUpdateErrorKind::Feed => "Update feed could not be read.",
        AppUpdateErrorKind::Incompatible => "No compatible update is available for this Mac.",
        AppUpdateErrorKind::Verification => "Update could not be verified.",
        AppUpdateErrorKind::Install => "Update could not be installed.",
        AppUpdateErrorKind::Unknown => "Update failed.",
    }
}

/// The `size` of the platform entry the updater selected. `Update.raw_json` is
/// the whole manifest and `Update.target` is only the OS ("darwin"), not the
/// key it picked (`darwin-aarch64-app` vs `darwin-aarch64`), so match the entry
/// by the download URL the updater resolved.
pub fn update_size_bytes(raw_json: &serde_json::Value, download_url: &str) -> Option<u64> {
    raw_json
        .get("platforms")?
        .as_object()?
        .values()
        .find(|platform| platform.get("url").and_then(|url| url.as_str()) == Some(download_url))?
        .get("size")?
        .as_u64()
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
pub fn build_out_of_window(
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
/// core wrapper below feeds it the cached gate status.
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

pub fn apply_running_build_window_gate(core: &AppCore, state: AppUpdateState) -> AppUpdateState {
    running_build_window_gate(
        state,
        crate::licensing::cached_status(core).as_ref(),
        running_build_date_ms(),
    )
}

/// What `run_update_check` does with a found remote update: the stored state,
/// whether the update stays installable (`pending_update` kept), and whether
/// the owner is notified. Out-of-window builds are surfaced (version shown) but
/// never installable; their notice opens the update window once per version
/// (see [`out_of_window_notice`]) and skips the "ready to install" bell.
#[derive(Debug, PartialEq, Eq)]
pub struct RemoteUpdateDecision {
    pub state: AppUpdateState,
    pub installable: bool,
    pub notify: bool,
}

pub fn decide_remote_update(out_of_window: bool, notify_available: bool) -> RemoteUpdateDecision {
    RemoteUpdateDecision {
        state: if out_of_window {
            AppUpdateState::AvailableOutOfWindow
        } else {
            AppUpdateState::Available
        },
        installable: !out_of_window,
        notify: notify_available,
    }
}

/// The settings to persist when an out-of-window `version` should open the
/// update window, or `None` when that version already did (once per version).
pub fn out_of_window_notice(
    settings: &AppUpdateSettings,
    version: &str,
) -> Option<AppUpdateSettings> {
    (settings.notified_out_of_window_version.as_deref() != Some(version)).then(|| {
        AppUpdateSettings {
            notified_out_of_window_version: Some(version.to_string()),
            ..settings.clone()
        }
    })
}

/// The only reason to refuse: nothing was installed. A live capture is NOT a
/// reason — `request_graceful_restart_after_update` finalizes it before
/// relaunching, same as any other graceful exit.
pub fn restart_after_update_error(restart_required: bool) -> Option<AppUpdateError> {
    (!restart_required).then(|| {
        AppUpdateError::new(
            AppUpdateErrorKind::Install,
            "No installed update is waiting for restart.",
        )
    })
}

#[cfg(test)]
mod tests;
