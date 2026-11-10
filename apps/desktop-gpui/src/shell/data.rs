//! Fills the shell's models from `AppCore`. Views never call the backend; this is
//! the one place that does, off the main thread. `build` is the one rule set
//! from the commands' answers to the model; the parity fixture feeds it too.

use std::sync::Arc;

use app_core::native_capture::lifecycle::NativeCaptureState;
use app_core::recording_settings::{current_recording_settings, RecordingSettingsState};
use app_core::{AppCore, AppInfraState};
use app_infra::StorageUsage;
use capture_types::{
    CapturePermissionState, LicenseStatus, NativeCaptureSession, RecordingSettings,
    RuntimeSourcesStatus, UsageChartsResponse,
};

use super::model::{self, EngineFlag, NotificationRow, ShellModel, StatusBarModel};

/// What the shell's commands answered (`None` = not read / failed).
#[derive(Default)]
pub struct Inputs {
    /// `get_recording_settings`.
    pub settings: Option<RecordingSettings>,
    /// `get_capture_permissions`.
    pub screen_permission: Option<CapturePermissionState>,
    pub session: Option<NativeCaptureSession>,
    /// `get_idle_debug.runtimeSources`.
    pub runtime: Option<RuntimeSourcesStatus>,
    /// `get_license_status`.
    pub license: Option<LicenseStatus>,
    /// `get_usage_charts` over today.
    pub usage: Option<UsageChartsResponse>,
    /// `get_storage_usage`.
    pub storage: Option<StorageUsage>,
    pub free_bytes: Option<u64>,
    /// `get_ai_runtime_status`, reduced.
    pub engine: Option<EngineFlag>,
    /// `get_app_notifications`.
    pub notifications: Vec<NotificationRow>,
}

fn session_start_ms(session: Option<&NativeCaptureSession>) -> Option<i64> {
    let s = session?.source_sessions.as_ref()?;
    [&s.screen, &s.microphone, &s.system_audio]
        .into_iter()
        .flatten()
        .map(|m| m.started_at_unix_ms as i64)
        .min()
}

/// The session's part of the status bar (also the 2 s live refresh).
fn apply_session(
    status: &mut StatusBarModel,
    session: Option<&NativeCaptureSession>,
    runtime: Option<&RuntimeSourcesStatus>,
) {
    status.running = session.is_some_and(|s| s.is_running);
    status.user_paused = session.is_some_and(|s| s.is_user_paused);
    status.inactivity_paused = session.is_some_and(|s| s.is_inactivity_paused);
    status.low_disk = session.is_some_and(|s| s.is_low_disk_suspended);
    status.runtime = runtime.map(model::runtime_sources);
    status.session_start_ms = session_start_ms(session);
}

pub fn build(i: Inputs) -> ShellModel {
    let mut status = StatusBarModel {
        selected: i.settings.as_ref().map_or([false; 3], |s| {
            [
                s.capture_screen,
                s.capture_microphone,
                s.capture_system_audio,
            ]
        }),
        screen_permission: i.screen_permission,
        read_only: matches!(
            i.license,
            Some(LicenseStatus::ReadOnly | LicenseStatus::Revoked)
        ),
        engine: i.engine,
        tracked_ms: i
            .usage
            .map(|u| u.time_per_app.iter().map(|a| a.active_ms).sum()),
        storage_bytes: i.storage.map(|u| {
            u.screen_bytes
                + u.microphone_bytes
                + u.system_audio_bytes
                + u.index_bytes
                + u.models_bytes
        }),
        free_bytes: i.free_bytes,
        local_only: i.settings.as_ref().is_none_or(model::is_local_only),
        ..Default::default()
    };
    apply_session(&mut status, i.session.as_ref(), i.runtime.as_ref());
    status.stopped_reason = model::stopped_reason_from(&i.notifications, status.session_start_ms);
    ShellModel {
        status,
        notifications: i.notifications,
        developer_options: i.settings.is_some_and(|s| s.developer_options_enabled),
    }
}

fn runtime_sources(core: &AppCore, running: bool) -> Option<RuntimeSourcesStatus> {
    // Only while recording, like `subscribeRuntimeSources`.
    let state = core.try_state::<NativeCaptureState>().filter(|_| running)?;
    Some(app_core::native_capture::activity::get_idle_debug(&state).runtime_sources)
}

/// Local midnight today, in unix ms (the status bar's "today").
fn today_start_ms(now_ms: i64) -> i64 {
    const DAY_MS: i64 = 86_400_000;
    let offset = crate::native::sys::local_offset_seconds() as i64 * 1000;
    (now_ms + offset).div_euclid(DAY_MS) * DAY_MS - offset
}

/// Runs on the tokio runtime (`Tokio::spawn`).
pub async fn load(core: &Arc<AppCore>) -> ShellModel {
    let settings = core
        .try_state::<RecordingSettingsState>()
        .map(|s| current_recording_settings(&s));
    let capture = core.try_state::<NativeCaptureState>().is_some();
    let infra = core.try_state::<AppInfraState>();
    // Only the open capture root is measured (never a default folder nobody opened).
    let measure = infra.is_some();
    // The permission read and the storage walk block: keep them off the async workers.
    let blocking = core.clone();
    let (permissions, storage) = core
        .spawn_blocking(move || {
            let permissions = capture
                .then(|| app_core::native_capture::permissions::get_capture_permissions(&blocking));
            let storage = measure
                .then(|| app_core::managed_storage_layout::get_storage_usage(&blocking, None));
            (permissions, storage)
        })
        .await
        .unwrap_or((None, None));
    let session = permissions.as_ref().map(|p| p.session.clone());
    let runtime = runtime_sources(core, session.as_ref().is_some_and(|s| s.is_running));
    let (license, usage) = match &infra {
        Some(infra) => {
            let now = crate::native::now_unix_ms() as i64;
            (
                app_core::licensing::get_license_status(core, infra)
                    .await
                    .ok(),
                app_core::usage_charts::get_usage_charts(
                    infra,
                    Some(today_start_ms(now)),
                    Some(now),
                )
                .await
                .ok(),
            )
        }
        None => (None, None),
    };
    let engine = match settings {
        Some(_) => app_core::ai_runtime::get_ai_runtime_status(core)
            .await
            .ok()
            .and_then(|s| serde_json::to_value(s).ok())
            .and_then(|v| model::engine_flag(&v)),
        None => None,
    };
    let notifications = notifications(core);
    build(Inputs {
        settings,
        screen_permission: permissions.map(|p| p.permissions.screen),
        session,
        runtime,
        license,
        usage,
        free_bytes: storage.as_ref().and_then(|s| s.free_bytes),
        storage: storage.map(|s| s.usage),
        engine,
        notifications,
    })
}

pub fn notifications(core: &AppCore) -> Vec<NotificationRow> {
    core.try_state::<app_core::native_capture::app_notifications::AppNotificationsState>()
        .map(|_| app_core::native_capture::app_notifications::get_app_notifications(core))
        .and_then(|list| serde_json::to_value(list).ok())
        .map(|v| model::notification_rows(&v))
        .unwrap_or_default()
}

/// The 2 s poll while recording: the session and its per-source state only
/// (no disk walk, no engine ping).
pub fn refresh_live(core: &AppCore, status: &mut StatusBarModel) {
    if core.try_state::<NativeCaptureState>().is_none() {
        return;
    }
    let session = app_core::native_capture::control::current_native_capture_session(core);
    let runtime = runtime_sources(core, session.is_running);
    apply_session(status, Some(&session), runtime.as_ref());
}
