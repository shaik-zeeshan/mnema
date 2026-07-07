//! Licensing gate (slice 4): computes the offline `LicenseStatus`, caches it for
//! synchronous reads by other seams (capture-stop, status bar), emits changes to
//! the frontend, and exposes the Tauri commands + the deferred-startup entry
//! point. The verification core, keychain store, and SQLite projection all live
//! in `app_infra` (slices 1–3); this module is only the desktop-side wiring.

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use capture_types::{ActivateLicenseResult, LicenseStatus};
use sqlx::SqlitePool;
use tauri::{Emitter, Manager};

use crate::app_infra::AppInfraState;

/// Event emitted after every status recompute. Payload is the `LicenseStatus`
/// (the camelCase tagged shape mirrored in `apps/desktop/src/lib/licensing.ts`).
pub const LICENSE_STATUS_EVENT: &str = "license_status";

/// In-memory cache of the latest computed status. Other seams — slice 5's
/// capture-stop and the status bar — read it synchronously via
/// [`cached_status`] instead of touching the DB/keychain on the hot path.
/// `.manage(...)`-registered in `lib.rs`.
pub struct LicenseGate(pub Mutex<Option<LicenseStatus>>);

/// "Now" in unix milliseconds (UTC).
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Set the in-memory cache and emit the change event. Called at the end of every
/// recompute so cache and listeners never drift.
fn publish(app_handle: &tauri::AppHandle, status: &LicenseStatus) {
    if let Some(gate) = app_handle.try_state::<LicenseGate>() {
        if let Ok(mut slot) = gate.0.lock() {
            *slot = Some(status.clone());
        }
    }
    let _ = app_handle.emit(LICENSE_STATUS_EVENT, status);
    // Keep the native tray in sync: a flip to/from Read-Only Mode changes the
    // tray's status header + Start-Recording enablement, and nothing else
    // rebuilds the menu on a licensing change.
    crate::status_bar::refresh(app_handle);
}

/// The last computed status, for synchronous reads by the capture gate / status
/// bar. `None` before the deferred-startup gate has run once.
pub fn cached_status(app_handle: &tauri::AppHandle) -> Option<LicenseStatus> {
    app_handle
        .try_state::<LicenseGate>()
        .and_then(|gate| gate.0.lock().ok().and_then(|slot| slot.clone()))
}

/// Compute the current status from keychain + DB projection, cache it, and emit.
///
/// Order: anti-rollback bump first, then the Licensed branch (a valid stored key
/// wins outright — an invalid/garbage stored key falls through to trial rather
/// than hard-erroring), else the Trial branch keyed off the DB start with a
/// keychain fallback that survives uninstall.
pub async fn compute_license_status(
    pool: &SqlitePool,
    app_handle: &tauri::AppHandle,
    now_ms: i64,
) -> LicenseStatus {
    // Dev builds are never gated. Overridable with MNEMA_LICENSE_ENFORCE=1 to
    // exercise the real trial/read-only flow locally.
    // ponytail: compile-time bypass; release builds fall through to the real gate.
    if cfg!(debug_assertions) && std::env::var_os("MNEMA_LICENSE_ENFORCE").is_none() {
        let status = LicenseStatus::Licensed {
            update_through_ms: i64::MAX,
            in_window: true,
            email: "dev@localhost".into(),
        };
        publish(app_handle, &status);
        return status;
    }

    // Anti-rollback: record the high-water mark before reading it back below.
    let _ = app_infra::bump_max_timestamp_seen(pool, now_ms).await;

    let status = if let Some(status) = compute_licensed(pool, now_ms).await {
        status
    } else {
        compute_trial(pool, now_ms).await
    };
    publish(app_handle, &status);
    status
}

/// Licensed branch: `Some(Licensed{..})` when a valid signed key is stored,
/// `None` when there is no key or it fails verification.
async fn compute_licensed(pool: &SqlitePool, now_ms: i64) -> Option<LicenseStatus> {
    let key = app_infra::load_license_key().ok().flatten()?;
    let payload = app_infra::parse_and_verify_license(&key).ok()?;
    // Refresh the fast-read projection for the Settings UI. Best-effort.
    let _ = app_infra::cache_license_fields(
        pool,
        &payload.license_id,
        &payload.tier,
        payload.issued_at,
        payload.update_through,
        &payload.email,
    )
    .await;
    Some(LicenseStatus::Licensed {
        update_through_ms: payload.update_through,
        in_window: now_ms <= payload.update_through,
        email: payload.email,
    })
}

/// Trial branch: resolve the effective trial start (DB, else keychain fallback
/// restored into the DB on reinstall), then read days left / read-only.
async fn compute_trial(pool: &SqlitePool, now_ms: i64) -> LicenseStatus {
    // Lenient default on a read error: never wrongly lock the user out.
    let state = app_infra::read_licensing_state(pool).await.ok();

    let start = match state.as_ref().and_then(|s| s.trial_started_at_ms) {
        Some(start) => Some(start),
        None => match app_infra::load_trial_record()
            .ok()
            .flatten()
            .and_then(|r| r.trim().parse::<i64>().ok())
        {
            // Reinstall: keychain kept the start but the fresh DB lost it — restore.
            Some(kc_start) => {
                let _ = app_infra::set_trial_started_once(pool, kc_start).await;
                Some(kc_start)
            }
            None => None,
        },
    };

    let Some(start) = start else {
        return LicenseStatus::TrialNotStarted {
            trial_days: app_infra::TRIAL_LEN_DAYS,
        };
    };

    // `bump_max_timestamp_seen` already ran, so a missing read defaults to now.
    let max_seen = state.map(|s| s.max_timestamp_ever_seen_ms).unwrap_or(now_ms);
    let days = app_infra::trial_days_left(start, now_ms, max_seen, app_infra::TRIAL_LEN_DAYS);
    if days == 0 {
        LicenseStatus::ReadOnly
    } else {
        LicenseStatus::Trial {
            days_left: days,
            trial_end_ms: start + app_infra::TRIAL_LEN_DAYS as i64 * 86_400_000,
        }
    }
}

/// Deferred-startup gate entry point. Runs on the `mnema-deferred-startup`
/// thread AFTER the window opens — never blocks launch, never gates
/// synchronously. Computes, caches, and emits the status.
pub fn run_license_gate(app_handle: &tauri::AppHandle) {
    let Some(infra) = app_handle.try_state::<AppInfraState>() else {
        crate::native_capture::debug_log::log_error(
            "app infrastructure state was not initialized; skipping license gate",
        );
        return;
    };
    let infra = infra.inner().clone();
    let status = tauri::async_runtime::block_on(compute_license_status(
        infra.pool(),
        app_handle,
        now_ms(),
    ));
    crate::native_capture::debug_log::log_info(format!(
        "license gate computed status: {}",
        status_label(&status)
    ));
}

/// Idempotently start the trial clock (DB + keychain fallback) and recompute.
/// Exposed for slice 6 to call from the first-successful-Capture seam; this
/// module does NOT hook the lifecycle itself.
pub async fn ensure_trial_started(
    pool: &SqlitePool,
    app_handle: &tauri::AppHandle,
    now_ms: i64,
) -> LicenseStatus {
    let _ = app_infra::set_trial_started_once(pool, now_ms).await;
    // Keychain fallback survives uninstall; only write it when confirmed absent
    // so a reinstall (or a read error) never overwrites the original start.
    if matches!(app_infra::load_trial_record(), Ok(None)) {
        let _ = app_infra::store_trial_record(&now_ms.to_string());
    }
    compute_license_status(pool, app_handle, now_ms).await
}

/// Short label for logs — avoids leaking the licensee email at INFO level.
fn status_label(status: &LicenseStatus) -> &'static str {
    match status {
        LicenseStatus::TrialNotStarted { .. } => "trialNotStarted",
        LicenseStatus::Trial { .. } => "trial",
        LicenseStatus::ReadOnly => "readOnly",
        LicenseStatus::Licensed { .. } => "licensed",
    }
}

// ---------------------------------------------------------------------------
// Tauri commands (names must stay byte-identical to the frontend `invoke(...)`).
// ---------------------------------------------------------------------------

/// Frontend snapshot/reattach: return the cached status if present, else compute
/// (which caches + emits) once.
#[tauri::command]
pub async fn get_license_status(
    state: tauri::State<'_, AppInfraState>,
    app_handle: tauri::AppHandle,
) -> Result<LicenseStatus, String> {
    if let Some(status) = cached_status(&app_handle) {
        return Ok(status);
    }
    let infra = std::sync::Arc::clone(&*state);
    Ok(compute_license_status(infra.pool(), &app_handle, now_ms()).await)
}

/// Idempotently start the trial and return the fresh status. Thin wrapper over
/// [`ensure_trial_started`].
#[tauri::command]
pub async fn start_trial(
    state: tauri::State<'_, AppInfraState>,
    app_handle: tauri::AppHandle,
) -> Result<LicenseStatus, String> {
    let infra = std::sync::Arc::clone(&*state);
    Ok(ensure_trial_started(infra.pool(), &app_handle, now_ms()).await)
}

/// Verify + store a pasted license key, then recompute. An invalid key is never
/// stored and returns a human-readable error.
#[tauri::command]
pub async fn activate_license(
    key: String,
    state: tauri::State<'_, AppInfraState>,
    app_handle: tauri::AppHandle,
) -> Result<ActivateLicenseResult, String> {
    // Verify BEFORE storing so a garbage paste never sticks.
    app_infra::parse_and_verify_license(&key)
        .map_err(|_| "This license key is invalid or corrupted.".to_string())?;
    app_infra::store_license_key(&key)
        .map_err(|error| format!("Could not save the license key: {error}"))?;

    let infra = std::sync::Arc::clone(&*state);
    let status = compute_license_status(infra.pool(), &app_handle, now_ms()).await;
    Ok(ActivateLicenseResult { status })
}

/// Activate from a `mnema://license/activate?key=…` deep link. Same verify → store
/// → recompute path as [`activate_license`], but callable from the deep-link
/// handler in `lib.rs` where we only hold an `AppHandle`. Success surfaces to the
/// UI through the emitted `license_status` event (the store re-renders); failures
/// are logged, not shown — a bad link just leaves the current status untouched.
pub async fn activate_from_deep_link(app_handle: tauri::AppHandle, key: String) {
    if app_infra::parse_and_verify_license(&key).is_err() {
        tauri_plugin_log::log::warn!(target: "mnema_lib::licensing", "deep-link license key failed verification");
        return;
    }
    if let Err(error) = app_infra::store_license_key(&key) {
        tauri_plugin_log::log::warn!(target: "mnema_lib::licensing", "deep-link license store failed: {error}");
        return;
    }
    if let Some(state) = app_handle.try_state::<AppInfraState>() {
        let infra = std::sync::Arc::clone(&*state);
        compute_license_status(infra.pool(), &app_handle, now_ms()).await;
    }
}
