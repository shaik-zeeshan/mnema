//! Licensing gate (slice 4): computes the offline `LicenseStatus`, caches it for
//! synchronous reads by other seams (capture-stop, status bar), emits changes to
//! the frontend, and exposes the Tauri commands + the deferred-startup entry
//! point. The verification core, keychain store, and SQLite projection all live
//! in `app_infra` (slices 1–3); this module is only the desktop-side wiring.

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use capture_types::{ActivateLicenseResult, Activation, LicenseStatus};
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

/// In-memory over-cap hint: `(reset_url, buy_url)` set by the background
/// activation task on a `409 over_cap` and cleared on a `200`. `compute_licensed`
/// reads it so a `Licensed` key in its Provisional Window surfaces as
/// `Activation::RefusedOverCap` (with the reset/buy links) instead of plain
/// `Pending`. `.manage(...)`-registered in `lib.rs` next to `LicenseGate`.
// ponytail: unkeyed by license id — only one license is active at a time and the
// task clears it on success; key it if multi-license ever lands.
pub struct ActivationHint(pub Mutex<Option<(String, String)>>);

/// "Now" in unix milliseconds (UTC). Shared with the capture-gate seams
/// (lifecycle start refusal, rotation boundary, status bar) so they all ask
/// the same clock question as the gate itself.
pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Fire-and-forget status recompute. Used by the capture seams when they catch
/// a lapsed trial on the synchronous path: the refusal/stop doesn't wait, but
/// the cache/tray/Settings flip from the stale `Trial` to `ReadOnly`.
pub(crate) fn recompute_status_async(app_handle: &tauri::AppHandle, now_ms: i64) {
    let Some(infra) = app_handle.try_state::<AppInfraState>() else {
        return;
    };
    let infra = infra.inner().clone();
    let app_handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        compute_license_status(infra.pool(), &app_handle, now_ms).await;
    });
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
            name: "Dev".into(),
            activation: Activation::Activated,
        };
        publish(app_handle, &status);
        return status;
    }

    // Anti-rollback: record the high-water mark before reading it back below.
    let _ = app_infra::bump_max_timestamp_seen(pool, now_ms).await;

    let status = if let Some(status) = compute_licensed(pool, app_handle, now_ms).await {
        status
    } else {
        compute_trial(pool, now_ms).await
    };
    publish(app_handle, &status);
    status
}

/// Licensed branch: `Some(Licensed{..})` when a valid signed key is stored,
/// `Some(Revoked)` when that key appears on the effective CRL, `None` when there
/// is no key or it fails verification. A revoked key does NOT fall through to
/// trial — it stays `Revoked` (blocking) so a refunded key can't reclaim a trial.
async fn compute_licensed(
    pool: &SqlitePool,
    app_handle: &tauri::AppHandle,
    now_ms: i64,
) -> Option<LicenseStatus> {
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
    // Revoked wins outright — never fall through to activation gating.
    let crl = load_effective_crl(pool).await;
    if is_key_revoked(&payload.license_id, crl.as_ref()) {
        return Some(LicenseStatus::Revoked);
    }

    let max_seen = app_infra::read_licensing_state(pool)
        .await
        .ok()
        .map(|s| s.max_timestamp_ever_seen_ms)
        .unwrap_or(now_ms);
    let activation = compute_activation(app_handle, &payload.license_id, now_ms, max_seen);

    Some(LicenseStatus::Licensed {
        update_through_ms: payload.update_through,
        in_window: now_ms <= payload.update_through,
        email: payload.email,
        name: payload.name.clone().unwrap_or_default(),
        activation,
    })
}

/// Read-only activation decision for the gate: verify a stored receipt for this
/// machine, else read the Provisional Window. Never writes — the background task
/// (`maybe_spawn_activation`) starts the clock and fetches the receipt.
fn compute_activation(
    app_handle: &tauri::AppHandle,
    license_id: &str,
    now_ms: i64,
    max_seen_ms: i64,
) -> Activation {
    // ponytail: non-macOS can't fingerprint the machine, so we can never verify a
    // receipt or fairly time-box a window — skip activation entirely, never lock out.
    let Ok(uuid) = app_infra::hardware_uuid() else {
        return Activation::Activated;
    };
    let machine_hash = app_infra::machine_hash(license_id, &uuid);

    let has_valid_receipt = app_infra::load_activation_receipt()
        .ok()
        .flatten()
        .and_then(|wire| app_infra::parse_and_verify_receipt(&wire, license_id, &machine_hash).ok())
        .is_some();

    // No stored provisional start yet → treat as freshly pending with a full
    // window (compute never starts the clock; the background task does).
    let start = provisional_start_for(license_id).unwrap_or(now_ms);
    activation_from(
        has_valid_receipt,
        start,
        now_ms,
        max_seen_ms,
        read_over_cap_hint(app_handle),
    )
}

/// Pure activation classifier — the whole state machine, no IO.
fn activation_from(
    has_valid_receipt: bool,
    provisional_started_at_ms: i64,
    now_ms: i64,
    max_seen_ms: i64,
    over_cap: Option<(String, String)>,
) -> Activation {
    if has_valid_receipt {
        return Activation::Activated;
    }
    let days_left = app_infra::provisional_days_left(
        provisional_started_at_ms,
        now_ms,
        max_seen_ms,
        app_infra::PROVISIONAL_WINDOW_DAYS,
    );
    if days_left == 0 {
        return Activation::Lapsed;
    }
    match over_cap {
        Some((reset_url, buy_url)) => Activation::RefusedOverCap { reset_url, buy_url },
        None => Activation::Pending {
            provisional_days_left: days_left,
        },
    }
}

/// The stored provisional-window start for `license_id`, if any. A state for a
/// different license id reads as absent (a new license gets a fresh window).
fn provisional_start_for(license_id: &str) -> Option<i64> {
    app_infra::load_activation_state()
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str::<app_infra::ActivationState>(&json).ok())
        .filter(|state| state.license_id == license_id)
        .map(|state| state.provisional_started_at_ms)
}

/// The effective CRL for enforcement: the freshest verified document of
/// {baked-in floor, verbatim cache}. The cache is re-verified on every read —
/// a tampered/garbage cache row contributes nothing. `None` when neither exists.
async fn load_effective_crl(pool: &SqlitePool) -> Option<app_infra::Crl> {
    let cached = app_infra::load_cached_crl(pool)
        .await
        .ok()
        .flatten()
        .and_then(|wire| app_infra::parse_and_verify_crl(&wire).ok());
    app_infra::effective_crl(app_infra::baked_crl(), cached)
}

/// Pure membership check used by both the gate and the activation paths.
fn is_key_revoked(license_id: &str, crl: Option<&app_infra::Crl>) -> bool {
    crl.is_some_and(|crl| app_infra::is_revoked(license_id, crl))
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

    // Dev-only test knob: MNEMA_TRIAL_LEN_MS shrinks the whole trial window
    // (e.g. 300000 = 5 min) so the trial→read-only flip is testable in one
    // sitting. Days display pins to 1 while active. Compiled out of release.
    #[cfg(debug_assertions)]
    if let Some(len_ms) = std::env::var("MNEMA_TRIAL_LEN_MS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
    {
        let trial_end_ms = start + len_ms;
        return if now_ms.max(max_seen) >= trial_end_ms {
            LicenseStatus::ReadOnly
        } else {
            LicenseStatus::Trial {
                days_left: 1,
                trial_end_ms,
            }
        };
    }

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

    // Dev-only test knob: MNEMA_TRIAL_RESET=1 wipes the stored trial start
    // (DB row + keychain record) once at launch, so the fresh-trial flow can
    // be re-run. Compiled out of release; nothing production un-starts a trial.
    #[cfg(debug_assertions)]
    if std::env::var_os("MNEMA_TRIAL_RESET").is_some() {
        let _ = app_infra::delete_trial_record();
        let _ = tauri::async_runtime::block_on(app_infra::clear_trial_started(infra.pool()));
        crate::native_capture::debug_log::log_info(
            "MNEMA_TRIAL_RESET: cleared stored trial start (DB + keychain)",
        );
    }

    let status = tauri::async_runtime::block_on(compute_license_status(
        infra.pool(),
        app_handle,
        now_ms(),
    ));
    crate::native_capture::debug_log::log_info(format!(
        "license gate computed status: {}",
        status_label(&status)
    ));
    // Try to finish once-per-machine activation on startup (no-op if already
    // activated, no key, or revoked). Retries piggyback the daily CRL tick.
    maybe_spawn_activation(app_handle);
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

/// Honest copy for refusing capture, per blocking state. A `Revoked` (refunded/
/// leaked) key reads as "revoked" — never conflated with a lapsed trial, which
/// the offline model deliberately keeps distinct (see `LicenseStatus::Revoked`).
/// Returns `(error_code, message)`; the capture-start seam surfaces both, while
/// the rotation-stop and tray derive their own copy from the same distinction.
pub(crate) fn capture_refusal_copy(status: &LicenseStatus) -> (&'static str, &'static str) {
    match status {
        LicenseStatus::Revoked => (
            "capture_refused_revoked",
            "This license has been revoked. Contact support if you think this is a mistake — everything you already recorded stays browsable and searchable.",
        ),
        LicenseStatus::Licensed {
            activation: Activation::Lapsed,
            ..
        } => (
            "capture_refused_unactivated",
            "We couldn't confirm your license — connect to the internet once to finish activation. Everything you already recorded stays browsable and searchable.",
        ),
        _ => (
            "capture_refused_read_only",
            "Your trial has ended. Buy a license to resume recording — everything you already recorded stays browsable and searchable.",
        ),
    }
}

/// Short label for logs — avoids leaking the licensee email at INFO level.
fn status_label(status: &LicenseStatus) -> &'static str {
    match status {
        LicenseStatus::TrialNotStarted { .. } => "trialNotStarted",
        LicenseStatus::Trial { .. } => "trial",
        LicenseStatus::ReadOnly => "readOnly",
        LicenseStatus::Revoked => "revoked",
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
    let payload = app_infra::parse_and_verify_license(&key)
        .map_err(|_| "This license key is invalid or corrupted.".to_string())?;

    // Authentic-but-revoked: reject with an honest, distinct message and never
    // store — a revoked key must not activate.
    let infra = std::sync::Arc::clone(&*state);
    let crl = load_effective_crl(infra.pool()).await;
    if is_key_revoked(&payload.license_id, crl.as_ref()) {
        return Err("This license has been revoked.".to_string());
    }

    app_infra::store_license_key(&key)
        .map_err(|error| format!("Could not save the license key: {error}"))?;

    let status = compute_license_status(infra.pool(), &app_handle, now_ms()).await;
    // Kick off once-per-machine activation in the background; the returned status
    // is the pending one, and the receipt flips it to Activated when it lands.
    maybe_spawn_activation(&app_handle);
    Ok(ActivateLicenseResult { status })
}

/// Activate from a `mnema://license/activate?key=…` deep link. Same verify → store
/// → recompute path as [`activate_license`], but callable from the deep-link
/// handler in `lib.rs` where we only hold an `AppHandle`. Success surfaces to the
/// UI through the emitted `license_status` event (the store re-renders); failures
/// are logged, not shown — a bad link just leaves the current status untouched.
pub async fn activate_from_deep_link(app_handle: tauri::AppHandle, key: String) {
    let payload = match app_infra::parse_and_verify_license(&key) {
        Ok(payload) => payload,
        Err(_) => {
            tauri_plugin_log::log::warn!(target: "mnema_lib::licensing", "deep-link license key failed verification");
            return;
        }
    };
    let Some(state) = app_handle.try_state::<AppInfraState>() else {
        return;
    };
    let infra = std::sync::Arc::clone(&*state);
    // Revoked keys are never stored — leave the current status untouched.
    let crl = load_effective_crl(infra.pool()).await;
    if is_key_revoked(&payload.license_id, crl.as_ref()) {
        tauri_plugin_log::log::warn!(target: "mnema_lib::licensing", "deep-link license key has been revoked");
        return;
    }
    if let Err(error) = app_infra::store_license_key(&key) {
        tauri_plugin_log::log::warn!(target: "mnema_lib::licensing", "deep-link license store failed: {error}");
        return;
    }
    compute_license_status(infra.pool(), &app_handle, now_ms()).await;
    maybe_spawn_activation(&app_handle);
}

// ---------------------------------------------------------------------------
// Once-per-machine activation (ADR 0053): background attempt + retry.
// ---------------------------------------------------------------------------

/// Fallback activation endpoint, used only when neither env override is set.
/// Same host as [`crate::crl_refresh`]'s default worker; release CI overrides
/// via `MNEMA_ACTIVATION_URL` (a seller-owned domain) before shipping.
const DEFAULT_ACTIVATION_URL: &str =
    "https://mnema-fulfillment.shaikzeeshan999.workers.dev/activate";

/// The activation URL, most- to least-specific — mirrors `crl_refresh::crl_url`:
/// `MNEMA_DEV_ACTIVATION_URL` (debug runtime) → build-time `MNEMA_ACTIVATION_URL`
/// → [`DEFAULT_ACTIVATION_URL`].
fn activation_url() -> String {
    #[cfg(debug_assertions)]
    if let Ok(url) = std::env::var("MNEMA_DEV_ACTIVATION_URL") {
        if !url.trim().is_empty() {
            return url;
        }
    }
    match option_env!("MNEMA_ACTIVATION_URL") {
        Some(url) if !url.trim().is_empty() => url.to_string(),
        _ => DEFAULT_ACTIVATION_URL.to_string(),
    }
}

fn set_over_cap_hint(app_handle: &tauri::AppHandle, reset_url: String, buy_url: String) {
    if let Some(hint) = app_handle.try_state::<ActivationHint>() {
        if let Ok(mut slot) = hint.0.lock() {
            *slot = Some((reset_url, buy_url));
        }
    }
}

fn clear_over_cap_hint(app_handle: &tauri::AppHandle) {
    if let Some(hint) = app_handle.try_state::<ActivationHint>() {
        if let Ok(mut slot) = hint.0.lock() {
            *slot = None;
        }
    }
}

fn read_over_cap_hint(app_handle: &tauri::AppHandle) -> Option<(String, String)> {
    app_handle
        .try_state::<ActivationHint>()
        .and_then(|hint| hint.0.lock().ok().and_then(|slot| slot.clone()))
}

/// Ensure `activation_state` has a provisional start for `license_id`, write-once
/// (mirrors `set_trial_started_once`): only writes when currently absent for this
/// id, so a re-paste of the same license never resets the clock. A state for a
/// *different* id is replaced (a new license opens its own fresh window).
fn ensure_provisional_started(license_id: &str, now_ms: i64) {
    if provisional_start_for(license_id).is_some() {
        return;
    }
    let state = app_infra::ActivationState {
        license_id: license_id.to_string(),
        provisional_started_at_ms: now_ms,
    };
    if let Ok(json) = serde_json::to_string(&state) {
        let _ = app_infra::store_activation_state(&json);
    }
}

/// Spawn the once-per-machine activation attempt if there's work to do. Never
/// blocks; a no-op (already activated, no key, or revoked) just returns early
/// inside the task. Safe to call repeatedly — the daily CRL tick does.
pub(crate) fn maybe_spawn_activation(app_handle: &tauri::AppHandle) {
    let app_handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        run_activation(app_handle).await;
    });
}

async fn run_activation(app_handle: tauri::AppHandle) {
    let Some(state) = app_handle.try_state::<AppInfraState>() else {
        return;
    };
    let infra = std::sync::Arc::clone(&*state);
    let pool = infra.pool();

    // Need a stored, authentic, non-revoked key — else nothing to activate.
    let Some(key) = app_infra::load_license_key().ok().flatten() else {
        return;
    };
    let Ok(payload) = app_infra::parse_and_verify_license(&key) else {
        return;
    };
    let license_id = payload.license_id;
    let crl = load_effective_crl(pool).await;
    if is_key_revoked(&license_id, crl.as_ref()) {
        return;
    }

    let uuid = match app_infra::hardware_uuid() {
        Ok(uuid) => uuid,
        Err(error) => {
            // Non-macOS / no fingerprint: compute already treats this as Activated.
            tauri_plugin_log::log::warn!(target: "mnema_lib::licensing", "activation skipped: cannot read hardware uuid: {error}");
            return;
        }
    };
    let machine_hash = app_infra::machine_hash(&license_id, &uuid);

    // Already activated on this machine → done.
    let already_activated = app_infra::load_activation_receipt()
        .ok()
        .flatten()
        .and_then(|wire| app_infra::parse_and_verify_receipt(&wire, &license_id, &machine_hash).ok())
        .is_some();
    if already_activated {
        return;
    }

    // Start the Provisional Window clock (write-once) and record the high-water
    // mark before the first network attempt.
    ensure_provisional_started(&license_id, now_ms());
    let _ = app_infra::bump_max_timestamp_seen(pool, now_ms()).await;

    let body = serde_json::json!({
        "schema": 1,
        "license_id": license_id,
        "machine_hash": machine_hash,
    });
    let response = match reqwest::Client::new()
        .post(activation_url())
        .json(&body)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            // Network unreachable/timeout: leave the window running, retry next tick.
            tauri_plugin_log::log::warn!(target: "mnema_lib::licensing", "activation failed: network error contacting {}: {error}", activation_url());
            return;
        }
    };

    match response.status().as_u16() {
        200 => {
            #[derive(serde::Deserialize)]
            struct ActivateBody {
                receipt: String,
            }
            let receipt_wire = match response.json::<ActivateBody>().await {
                Ok(parsed) => parsed.receipt,
                Err(error) => {
                    tauri_plugin_log::log::warn!(target: "mnema_lib::licensing", "activation failed: 200 body missing/!receipt: {error}");
                    return;
                }
            };
            if let Err(error) =
                app_infra::parse_and_verify_receipt(&receipt_wire, &license_id, &machine_hash)
            {
                tauri_plugin_log::log::warn!(target: "mnema_lib::licensing", "activation failed: server receipt did not verify: {error}");
                return;
            }
            if let Err(error) = app_infra::store_activation_receipt(&receipt_wire) {
                tauri_plugin_log::log::warn!(target: "mnema_lib::licensing", "activation failed: could not store receipt: {error}");
                return;
            }
            clear_over_cap_hint(&app_handle);
            // Recompute → badge flips to Activated.
            compute_license_status(pool, &app_handle, now_ms()).await;
        }
        403 => {
            // Server says revoked. Don't grant/deny here — the CRL is the gate;
            // just recompute so status reflects whatever the CRL already knows.
            tauri_plugin_log::log::warn!(target: "mnema_lib::licensing", "activation refused: license {license_id} reported revoked (server 403); CRL enforces revocation");
            compute_license_status(pool, &app_handle, now_ms()).await;
        }
        409 => {
            #[derive(serde::Deserialize)]
            struct OverCapBody {
                reset_url: String,
                buy_url: String,
            }
            match response.json::<OverCapBody>().await {
                Ok(over_cap) => {
                    set_over_cap_hint(&app_handle, over_cap.reset_url, over_cap.buy_url);
                    // Recompute → RefusedOverCap while still inside the window.
                    compute_license_status(pool, &app_handle, now_ms()).await;
                }
                Err(error) => {
                    tauri_plugin_log::log::warn!(target: "mnema_lib::licensing", "activation failed: 409 over_cap body malformed: {error}");
                }
            }
        }
        other => {
            // 5xx / unexpected: leave the window running, retry next tick.
            tauri_plugin_log::log::warn!(target: "mnema_lib::licensing", "activation failed: unexpected HTTP {other} from {}", activation_url());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crl_naming(id: &str) -> app_infra::Crl {
        app_infra::Crl {
            schema: 1,
            issued_at: 1_700_000_000_000,
            revoked_license_ids: vec![id.to_string()],
        }
    }

    #[test]
    fn is_key_revoked_hit_miss_and_no_crl() {
        let crl = crl_naming("comp:press");
        assert!(is_key_revoked("comp:press", Some(&crl)));
        assert!(!is_key_revoked("comp:friend", Some(&crl)));
        assert!(!is_key_revoked("comp:press", None));
    }

    #[test]
    fn activation_from_state_machine() {
        let day_ms = 86_400_000i64;
        // A valid receipt short-circuits to Activated regardless of the window.
        assert_eq!(
            activation_from(true, 0, i64::MAX, i64::MAX, None),
            Activation::Activated,
        );
        // Inside the window, no over-cap hint → Pending with days left.
        assert_eq!(
            activation_from(false, 0, day_ms, day_ms, None),
            Activation::Pending {
                provisional_days_left: app_infra::PROVISIONAL_WINDOW_DAYS - 1,
            },
        );
        // Inside the window, over-cap hint set → RefusedOverCap surfaces the links.
        assert_eq!(
            activation_from(
                false,
                0,
                day_ms,
                day_ms,
                Some(("https://reset".into(), "https://buy".into())),
            ),
            Activation::RefusedOverCap {
                reset_url: "https://reset".into(),
                buy_url: "https://buy".into(),
            },
        );
        // Window elapsed → Lapsed (even with an over-cap hint still around).
        let past_window = app_infra::PROVISIONAL_WINDOW_DAYS as i64 * day_ms + 1;
        assert_eq!(
            activation_from(false, 0, past_window, past_window, None),
            Activation::Lapsed,
        );
    }

    #[test]
    fn capture_refusal_copy_is_honest_per_state() {
        // Revoked reads as "revoked" with its own code — never trial-ended copy.
        let (code, message) = capture_refusal_copy(&LicenseStatus::Revoked);
        assert_eq!(code, "capture_refused_revoked");
        assert!(message.contains("revoked"));
        assert!(!message.contains("trial"));

        // A lapsed activation reads as the unactivated copy — not trial, not revoked.
        let (code, message) = capture_refusal_copy(&LicenseStatus::Licensed {
            update_through_ms: 0,
            in_window: true,
            email: String::new(),
            name: String::new(),
            activation: Activation::Lapsed,
        });
        assert_eq!(code, "capture_refused_unactivated");
        assert!(message.contains("activation"));
        assert!(!message.contains("trial"));
        assert!(!message.contains("revoked"));

        // A lapsed trial / read-only reads as the trial-ended copy.
        for status in [
            LicenseStatus::ReadOnly,
            LicenseStatus::Trial {
                days_left: 0,
                trial_end_ms: 0,
            },
        ] {
            let (code, message) = capture_refusal_copy(&status);
            assert_eq!(code, "capture_refused_read_only");
            assert!(message.contains("trial"));
        }
    }
}
