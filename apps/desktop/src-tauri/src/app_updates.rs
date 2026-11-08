use std::{
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::Manager;
use tauri_plugin_updater::{Update, UpdaterExt};
use time::format_description::well_known::Rfc3339;
use url::Url;

use app_core::updates::{
    apply_running_build_window_gate, build_out_of_window, current_settings, decide_remote_update,
    emit_status, out_of_window_notice, persist_settings, restart_after_update_error,
    update_size_bytes, user_facing_error_message, AppUpdateAppInfo, AppUpdateChannel,
    AppUpdateError, AppUpdateErrorKind, AppUpdateInfo, AppUpdateProgress, AppUpdateSettings,
    AppUpdateState, AppUpdateStatus, UPDATE_CHECK_INTERVAL,
};
use app_core::AppCore;

use crate::{native_capture, windows};

const APP_UPDATE_AVAILABLE_NOTIFICATION_ID: &str = "app-update-available";

pub(crate) mod tray;

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

fn core(app_handle: &tauri::AppHandle) -> Arc<AppCore> {
    app_handle.state::<Arc<AppCore>>().inner().clone()
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
    let state = apply_running_build_window_gate(&core(app_handle), runtime.state);
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
    let settings = current_settings(&core(app_handle));
    let runtime_state = app_handle.state::<AppUpdateRuntimeState>();
    let runtime = runtime_state
        .lock()
        .expect("app update runtime state poisoned");
    status_from_runtime(app_handle, settings, &runtime)
}

/// The Tauri host refreshes the tray's update row on this event, after the emit.
fn emit_current_status(app_handle: &tauri::AppHandle) {
    emit_status(&core(app_handle), current_status(app_handle));
}

fn update_info_from_update(update: &Update, channel: AppUpdateChannel) -> AppUpdateInfo {
    AppUpdateInfo {
        version: update.version.clone(),
        date: update.date.and_then(|date| date.format(&Rfc3339).ok()),
        notes: update.body.clone(),
        channel,
        size_bytes: update_size_bytes(&update.raw_json, update.download_url.as_str()),
    }
}

/// The remote build's release date as unix ms (the manifest `pub_date`, carried
/// on `update.date`). `None` when the manifest omitted a date.
fn update_release_date_ms(update: &Update) -> Option<i64> {
    update
        .date
        .map(|date| (date.unix_timestamp_nanos() / 1_000_000) as i64)
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

fn app_update_error_from_updater_error(
    context: &str,
    error: tauri_plugin_updater::Error,
) -> AppUpdateError {
    let kind = map_update_error_kind(&error);
    let message = user_facing_error_message(kind).to_string();
    app_core::debug_log::log_warn(format!(
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

/// The proactive "there's an update" nudge: Mnema's own small update window
/// (`AppWindow::Update`), live-bound to `APP_UPDATE_STATUS_CHANGED_EVENT`.
///
/// It used to be a `tauri-plugin-dialog` message dialog, and that was a bug: a
/// PARENTLESS dialog is not an in-process `NSAlert`. rfd routes it to
/// `CFUserNotificationDisplayAlert`, drawn by the system's UserNotificationCenter
/// agent, with no handle to dismiss it and no lifetime tie to us. Installing from
/// the tray then restarts the app out from under it (`complete_graceful_exit`
/// ends in `_exit(0)`, running no cleanup) and the alert was left on screen
/// advertising a version the user already had. Any surface raised here has to
/// die when the process does — only our own window does.
fn prompt_update_available(app_handle: &tauri::AppHandle) {
    if let Err(error) = windows::open_update_window(app_handle) {
        app_core::debug_log::log_warn(format!(
            "failed to open the update window for an available update: {error}"
        ));
    }
}

/// Install the pending update and restart into it. Drives the tray item, which
/// is reachable with no window open — hence the failure dialog, since the
/// Settings error surface may not be on screen. A restart already staged (state
/// `RestartRequired`) short-circuits inside `install_app_update`, so this is
/// also the "finish the update" path.
///
/// The failure dialog is the same parentless system alert described on
/// [`prompt_update_available`], but it's safe here: a failed install never
/// restarts, so nothing exits out from under it.
pub(crate) fn install_and_restart(app_handle: tauri::AppHandle) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

    tauri::async_runtime::spawn(async move {
        let status = install_app_update(app_handle.clone()).await;
        if status.state == AppUpdateState::RestartRequired {
            windows::request_graceful_restart_after_update(&app_handle);
            return;
        }
        let message = status
            .error
            .map(|error| error.message)
            .unwrap_or_else(|| user_facing_error_message(AppUpdateErrorKind::Install).to_string());
        app_handle
            .dialog()
            .message(message)
            .title("Mnema couldn't install the update")
            .kind(MessageDialogKind::Error)
            .show(|_| {});
    });
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
    let settings = current_settings(&core(app_handle));

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
                app_core::licensing::cached_status(&core(app_handle)).as_ref(),
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
            let current_channel = current_settings(&core(app_handle)).channel;
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
            // Out of window: open the window (its renew state) once per version,
            // gated on disk, and skip the "ready to install" bell — it isn't.
            if decision.notify && out_of_window {
                let stored = current_settings(&core(app_handle));
                if let Some(next) = out_of_window_notice(&stored, &info.version) {
                    // Open only once recorded: an unwritable gate would reopen
                    // the window every tick.
                    match persist_settings(&core(app_handle), next) {
                        Ok(_) => prompt_update_available(app_handle),
                        Err(error) => app_core::debug_log::log_warn(format!(
                            "skipped the out-of-window update notice: {error}"
                        )),
                    }
                }
                emit_current_status(app_handle);
                return current_status(app_handle);
            }
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
                prompt_update_available(app_handle);
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
            let current_channel = current_settings(&core(app_handle)).channel;
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
            let current_channel = current_settings(&core(app_handle)).channel;
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
    let settings = current_settings(&core(app_handle));
    app_core::debug_log::log_info(format!(
        "loaded app update settings (channel={:?}, endpoint={})",
        settings.channel,
        settings.channel.endpoint()
    ));
}

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

pub(crate) fn spawn_update_check(app_handle: &tauri::AppHandle) {
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
    app_core::crl_refresh::spawn_crl_refresh(
        app_handle
            .state::<std::sync::Arc<app_core::AppCore>>()
            .inner()
            .clone(),
    );
    run_update_check(&app_handle, false).await
}

#[tauri::command]
pub async fn set_app_update_channel(
    app_handle: tauri::AppHandle,
    channel: AppUpdateChannel,
) -> AppUpdateStatus {
    let settings = AppUpdateSettings {
        channel,
        ..current_settings(&core(&app_handle))
    };
    if let Err(error) = persist_settings(&core(&app_handle), settings) {
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
    app_core::debug_log::log_info("stopping capture to install an app update");
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
        Ok(Err(error)) => app_core::debug_log::log_warn(format!(
            "stopping capture before update install failed: {error:?}"
        )),
        Err(error) => app_core::debug_log::log_warn(format!(
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
                current_settings(&core(&app_handle)),
                &runtime,
            );
        }
        if matches!(
            runtime.state,
            AppUpdateState::Downloading | AppUpdateState::Installing
        ) {
            return status_from_runtime(
                &app_handle,
                current_settings(&core(&app_handle)),
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
                current_settings(&core(&app_handle)),
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
mod tests;
