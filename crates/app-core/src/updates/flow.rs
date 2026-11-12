//! The GPUI app's updater I/O (BUILD-PROMPT §9.2): check the feed every 2 h,
//! then download → minisign → unpack → code-signature gate → stop capture →
//! atomic bundle swap → `RestartRequired`. Same feed, key, licence gate and
//! states as the Tauri app; only the install step is new. The Tauri host keeps
//! its own `tauri_plugin_updater` flow and never manages an [`Updater`].

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use futures_util::StreamExt as _;

use super::bundle::{extract_app, replace_bundle};
use super::feed::{client, fetch_feed, newer_release, RemoteRelease};
use super::runtime::{gate_error, update_info, Updater, NOT_SIGNED_BY_MNEMA};
use super::settings::{current_settings, persist_settings, AppUpdateChannel, AppUpdateSettings};
use super::status::{
    apply_running_build_window_gate, build_out_of_window, decide_remote_update, emit_status,
    out_of_window_notice, restart_after_update_error, user_facing_error_message, AppUpdateError,
    AppUpdateErrorKind, AppUpdateState, AppUpdateStatus, UPDATE_CHECK_INTERVAL,
};
use super::verify::signature_is_valid;
use crate::{AppCore, DialogKind, ExitKind, MessageDialog, WindowRequest};

const APP_UPDATE_AVAILABLE_NOTIFICATION_ID: &str = "app-update-available";

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn updater(core: &AppCore) -> Option<Arc<Updater>> {
    core.try_state::<Updater>()
}

/// The status the update window, Settings and the tray show; `None` when this
/// host runs no updater.
pub fn current_status(core: &AppCore) -> Option<AppUpdateStatus> {
    let updater = updater(core)?;
    let channel = current_settings(core).channel;
    let rt = updater.lock();
    Some(AppUpdateStatus {
        app: updater.config.app.clone(),
        channel,
        state: apply_running_build_window_gate(core, rt.state),
        update: rt.update.clone(),
        progress: rt.progress.clone(),
        error: rt.error.clone(),
        last_checked_at_unix_ms: rt.last_checked_at_unix_ms,
    })
}

/// Whether Install has a release to fetch (the tray's update row).
pub fn installable(core: &AppCore) -> bool {
    updater(core).is_some_and(|u| u.lock().pending.is_some())
}

fn emit_current(core: &AppCore) -> AppUpdateStatus {
    let status = current_status(core).expect("updater managed");
    emit_status(core, status.clone());
    status
}

fn fail(core: &AppCore, updater: &Updater, error: AppUpdateError) -> AppUpdateStatus {
    updater.lock().fail(error);
    emit_current(core)
}

/// One feed check. `notify` (the timer's checks) pushes the in-app notice and
/// opens the update window once per version.
pub async fn run_update_check(core: &Arc<AppCore>, notify: bool) -> Option<AppUpdateStatus> {
    let updater = updater(core)?;
    let settings = current_settings(core);
    let begun = updater.lock().begin_check(now_unix_ms());
    if !begun {
        return current_status(core);
    }
    emit_current(core);

    let url = updater.config.feed_url(settings.channel);
    let result = match fetch_feed(&url).await {
        Ok(raw) => newer_release(
            &raw,
            updater.config.platform_key,
            &updater.config.app.version,
        ),
        Err(error) => Err(error),
    };
    // The channel changed mid-check: this answer came from the wrong feed.
    if current_settings(core).channel != settings.channel {
        updater.lock().discard_stale_check();
        spawn_update_check(core);
        return current_status(core);
    }
    match result {
        Ok(Some(release)) => Some(found(core, &updater, release, settings, notify)),
        Ok(None) => {
            updater.lock().up_to_date();
            Some(emit_current(core))
        }
        Err(error) => Some(fail(core, &updater, error)),
    }
}

fn found(
    core: &Arc<AppCore>,
    updater: &Updater,
    release: RemoteRelease,
    settings: AppUpdateSettings,
    notify: bool,
) -> AppUpdateStatus {
    let info = update_info(&release, settings.channel);
    // Licence Update Window: a Licensed owner is shown a newer build dated past
    // `update_through` but can't install it (Perpetual Fallback).
    let out_of_window = build_out_of_window(
        crate::licensing::cached_status(core).as_ref(),
        release.date_ms,
    );
    let decision = decide_remote_update(out_of_window, notify);
    let version = info.version.clone();
    updater.lock().found(release, info, &decision);
    if decision.notify && out_of_window {
        if let Some(next) = out_of_window_notice(&current_settings(core), &version) {
            // Open only once recorded, or an unwritable gate reopens it every tick.
            match persist_settings(core, next) {
                Ok(_) => open_update_window(core),
                Err(e) => log::warn!("skipped the out-of-window update notice: {e}"),
            }
        }
    } else if decision.notify && updater.lock().take_notice(&version) {
        crate::native_capture::app_notifications::push_info_app_notification(
            core,
            APP_UPDATE_AVAILABLE_NOTIFICATION_ID,
            "Mnema update available",
            &format!("Version {version} is ready to install from Settings."),
            Some("about"),
            now_unix_ms(),
        );
        open_update_window(core);
    }
    emit_current(core)
}

/// The proactive nudge is Mnema's own update window, never a parentless system
/// alert (which outlives the process on restart; see the Tauri `app_updates.rs`).
fn open_update_window(core: &AppCore) {
    if let Err(e) = core.host().open_window(WindowRequest::Update) {
        log::warn!("failed to open the update window for an available update: {e}");
    }
}

/// `WindowSignal::StartUpdateCheckTimer`: check now, then every 2 h, for the
/// life of the process.
pub fn start_update_check_timer(core: &Arc<AppCore>) {
    let task_core = Arc::clone(core);
    core.spawn(async move {
        let mut interval = tokio::time::interval(UPDATE_CHECK_INTERVAL);
        loop {
            interval.tick().await;
            run_update_check(&task_core, true).await;
        }
    });
}

pub fn spawn_update_check(core: &Arc<AppCore>) {
    let task_core = Arc::clone(core);
    core.spawn(async move {
        run_update_check(&task_core, false).await;
    });
}

/// "Check for updates" (Settings): also refreshes the licence CRL (ADR 0056).
pub async fn check_for_app_update(core: &Arc<AppCore>) -> Option<AppUpdateStatus> {
    crate::crl_refresh::spawn_crl_refresh(Arc::clone(core));
    run_update_check(core, false).await
}

pub async fn set_app_update_channel(
    core: &Arc<AppCore>,
    channel: AppUpdateChannel,
) -> Option<AppUpdateStatus> {
    let updater = updater(core)?;
    let settings = AppUpdateSettings {
        channel,
        ..current_settings(core)
    };
    if let Err(error) = persist_settings(core, settings) {
        return Some(fail(core, &updater, error));
    }
    run_update_check(core, false).await
}

/// Downloads, verifies and installs the pending release. Resolves to the new
/// status either way (a failure is a `Failed` status, never an `Err`).
pub async fn install_app_update(core: &Arc<AppCore>) -> Option<AppUpdateStatus> {
    let updater = updater(core)?;
    // Bound first: a `match` scrutinee's guard would live through the arms,
    // which lock again.
    let begun = updater.lock().begin_install();
    let release = match begun {
        Ok(Some(release)) => release,
        Ok(None) => return current_status(core),
        Err(()) => return Some(emit_current(core)),
    };
    emit_current(core);

    let bytes = match download(core, &updater, &release).await {
        Ok(bytes) => bytes,
        Err(error) => return Some(fail(core, &updater, error)),
    };
    // Verifying hashes the whole archive: blocking work, off the runtime workers.
    let (signature, pubkey) = (release.signature.clone(), updater.config.pubkey);
    let (bytes, signed) = core
        .run_blocking(move || {
            let signed = signature_is_valid(&bytes, &signature, pubkey);
            (bytes, signed)
        })
        .await;
    if !signed {
        log::error!("update install refused: SignatureInvalid");
        let error = AppUpdateError::new(AppUpdateErrorKind::Verification, NOT_SIGNED_BY_MNEMA);
        return Some(fail(core, &updater, error));
    }
    updater.lock().installing();
    emit_current(core);

    let task_core = Arc::clone(core);
    let task_updater = Arc::clone(&updater);
    let installed = core
        .spawn_blocking(move || install_bundle(&task_core, &task_updater, &bytes))
        .await
        .unwrap_or_else(|e| Err(install_error(format!("install task panicked: {e}"))));
    match installed {
        Ok(()) => {
            log::info!("installed app update {}", release.version);
            updater.lock().installed();
            Some(emit_current(core))
        }
        Err(error) => Some(fail(core, &updater, error)),
    }
}

fn install_error(detail: String) -> AppUpdateError {
    log::error!("app update install failed: {detail}");
    let kind = AppUpdateErrorKind::Install;
    AppUpdateError::new(kind, user_facing_error_message(kind))
}

/// Streams the archive, publishing progress about once per percent (or MiB).
async fn download(
    core: &AppCore,
    updater: &Updater,
    release: &RemoteRelease,
) -> Result<Vec<u8>, AppUpdateError> {
    let network = |e: reqwest::Error| {
        log::warn!("app update download failed: kind=Network; error={e}");
        let kind = AppUpdateErrorKind::Network;
        AppUpdateError::new(kind, user_facing_error_message(kind))
    };
    let response = client()
        .get(&release.url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(network)?;
    let total = response.content_length().or(release.size_bytes);
    let step = total.map_or(1 << 20, |t| (t / 100).max(1));
    // The header is a hint until the signature checks out: cap the reservation.
    let mut bytes = Vec::with_capacity(total.unwrap_or(0).min(1 << 30) as usize);
    let mut stream = response.bytes_stream();
    let mut reported = 0u64;
    while let Some(chunk) = stream.next().await {
        bytes.extend_from_slice(&chunk.map_err(network)?);
        let done = bytes.len() as u64;
        if done - reported >= step {
            reported = done;
            updater.lock().downloaded(done, total);
            emit_current(core);
        }
    }
    Ok(bytes)
}

/// Unpack next to the installed app (one volume, so the swap is two renames),
/// gate, stop capture, swap. Blocking. The staging dir and the old app moved
/// into it are deleted when it drops.
fn install_bundle(
    core: &Arc<AppCore>,
    updater: &Updater,
    archive: &[u8],
) -> Result<(), AppUpdateError> {
    let Some(installed) = updater.config.installed_app.clone() else {
        log::warn!("update install disabled: not running from an app bundle");
        return Err(gate_error(super::GateRefusal::NoOwnTeam));
    };
    let parent = installed.parent().unwrap_or(std::path::Path::new("/"));
    // shortcut: no admin-privilege fallback for an app in a folder this user
    // can't write (Tauri's AppleScript path); add one if users install that way.
    let staging = tempfile::Builder::new()
        .prefix(".mnema-update-")
        .tempdir_in(parent)
        .map_err(|e| install_error(format!("staging dir: {e}")))?;
    let new_app =
        extract_app(archive, staging.path()).map_err(|e| install_error(format!("unpack: {e}")))?;
    (updater.config.gate)(&new_app).map_err(gate_error)?;

    // The swap must not run under a live capture. A failed stop doesn't strand
    // a verified update: the restart's graceful exit finalizes capture again.
    stop_capture_before_install(core);
    replace_bundle(&installed, &new_app, &staging.path().join("previous.app"))
        .map_err(|e| install_error(format!("swap: {e}")))
}

fn stop_capture_before_install(core: &Arc<AppCore>) {
    use crate::native_capture::{control, lifecycle::NativeCaptureState};
    if core.try_state::<NativeCaptureState>().is_none() {
        return;
    }
    let session = control::current_native_capture_session(core);
    // Only the live flags: a stopped session keeps `source_sessions` as metadata.
    if !(session.is_running || session.is_user_paused) {
        return;
    }
    log::info!("stopping capture to install an app update");
    if let Err(e) = control::stop_native_capture_from_core(core) {
        log::warn!("stopping capture before update install failed: {e:?}");
    }
}

pub fn restart_after_app_update(core: &Arc<AppCore>) -> Result<(), AppUpdateError> {
    let restart_required = updater(core).is_some_and(|u| u.lock().restart_required);
    if let Some(error) = restart_after_update_error(restart_required) {
        if let Some(updater) = updater(core) {
            fail(core, &updater, error.clone());
        }
        return Err(error);
    }
    core.shutdown(ExitKind::Restart);
    Ok(())
}

/// Install, then relaunch into it (the window's "Install and restart" and the
/// tray row; a staged restart short-circuits to the relaunch). The tray has no
/// window to show a failure in, so `alert_on_failure` raises an in-app alert.
pub fn install_and_restart(core: &Arc<AppCore>, alert_on_failure: bool) {
    let task_core = Arc::clone(core);
    core.spawn(async move {
        let core = task_core;
        let Some(status) = install_app_update(&core).await else {
            return;
        };
        if status.state == AppUpdateState::RestartRequired {
            core.shutdown(ExitKind::Restart);
            return;
        }
        if alert_on_failure {
            let kind = AppUpdateErrorKind::Install;
            core.host().message(MessageDialog {
                title: "Mnema couldn't install the update".into(),
                message: status
                    .error
                    .map(|e| e.message)
                    .unwrap_or_else(|| user_facing_error_message(kind).into()),
                kind: DialogKind::Error,
            });
        }
    });
}

#[cfg(test)]
mod tests;
