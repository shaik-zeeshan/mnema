//! Licensing gate over the licensegate client crate: computes the offline
//! `LicenseStatus`, caches it for synchronous reads by other seams
//! (capture-stop, status bar), emits changes to the frontend, and exposes the
//! Tauri commands + the deferred-startup entry point.
//!
//! Division of labor: the `licensegate` crate owns verification (`Verifier`)
//! and the state machine (`evaluate`); `adapter` owns config + the pure
//! `Evaluation` → wire mapping; app-infra owns the keychain store and the
//! anti-rollback high-water mark; this module is the IO orchestration.

mod activation;
// pub(crate): `crl_refresh` fetches through the same one-config-point client.
pub(crate) mod adapter;
mod claim;
// pub(crate): its timer is started from deferred startup in `lib.rs`.
pub(crate) mod receipt_refresh;
mod renewed;
// pub(crate): its Tauri commands are registered by path in `lib.rs`.
pub(crate) mod reset;
mod trial;

pub(crate) use activation::maybe_spawn_activation;
use activation::read_over_cap_hint;
pub use claim::claim_from_deep_link;
pub use renewed::renewed_from_deep_link;
pub use trial::ensure_trial_started;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use capture_types::{ActivateLicenseResult, Activation, LicenseStatus};
use sqlx::SqlitePool;
use tauri::{Emitter, Manager};

use crate::app_infra::AppInfraState;

/// Event emitted after every status recompute. Payload is the `LicenseStatus`
/// (the camelCase tagged shape mirrored in `apps/desktop/src/lib/licensing.ts`).
pub const LICENSE_STATUS_EVENT: &str = "license_status";

/// Terminal deep-link endings that will never produce a `license_status` emit —
/// without this the deep-link receipt modal would spin forever on them.
/// `failed` carries a human message and shows the modal's failed face; `closed`
/// closes the receipt silently (the user declined the replacement confirm, or
/// the claim path handed off to its native email dialog). Hand-mirrored in
/// `$lib/license-deeplink-receipt.ts` (`LicenseDeepLinkDone`).
pub(crate) const LICENSE_DEEP_LINK_DONE_EVENT: &str = "license_deep_link_done";

#[derive(Debug, Clone, serde::Serialize)]
struct LicenseDeepLinkDonePayload {
    /// "failed" | "closed"
    outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<&'static str>,
}

pub(crate) fn emit_deep_link_done(
    app_handle: &tauri::AppHandle,
    outcome: &'static str,
    message: Option<&'static str>,
) {
    let _ = app_handle.emit(
        LICENSE_DEEP_LINK_DONE_EVENT,
        LicenseDeepLinkDonePayload { outcome, message },
    );
}

/// In-memory cache of the latest computed status. Other seams — the capture
/// gate and the status bar — read it synchronously via [`cached_status`]
/// instead of touching the DB/keychain on the hot path. `.manage(...)`-
/// registered in `lib.rs`.
pub struct LicenseGate(pub Mutex<Option<LicenseStatus>>);

/// In-memory over-cap hint: `(reset_url, buy_url)` set by the background
/// activation task on a `device_cap_reached` refusal and cleared on success.
/// The compute path reads it so a `Licensed` key in its Provisional Window
/// surfaces as `Activation::RefusedOverCap` instead of plain `Pending`.
/// `.manage(...)`-registered in `lib.rs` next to `LicenseGate`.
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
/// a lapsed status on the synchronous path: the refusal/stop doesn't wait, but
/// the cache/tray/Settings flip to the fresh state.
pub(crate) fn recompute_status_async(app_handle: &tauri::AppHandle, now_ms: i64) {
    // A refusal is the user's "why can't I record?" moment — kick the
    // background fixer too (trial issuance / activation), so connecting to the
    // internet and pressing Record again is enough to self-heal.
    maybe_spawn_activation(app_handle);
    let Some(infra) = app_handle.try_state::<AppInfraState>() else {
        return;
    };
    let infra = infra.inner().clone();
    let app_handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        compute_license_status(infra.pool(), &app_handle, now_ms).await;
    });
}

/// Concurrent recomputes are last-writer-wins on [`publish`], but "last to
/// finish" is not "freshest": a recompute holding a pre-CRL-refresh snapshot can
/// finish after the refresh's recompute and republish `Licensed` over `Revoked`.
/// Each compute claims a generation at START (before it reads any state); a
/// publish is dropped when a later-started compute has already published —
/// later-started means it read state at least as fresh.
static COMPUTE_GENERATION: AtomicU64 = AtomicU64::new(1);
static LAST_PUBLISHED_GENERATION: AtomicU64 = AtomicU64::new(0);

fn begin_compute() -> u64 {
    COMPUTE_GENERATION.fetch_add(1, Ordering::SeqCst)
}

/// Atomically claim the right to publish for a compute that started at
/// `generation`. False when a later-started compute already published.
fn try_claim_publish(last_published: &AtomicU64, generation: u64) -> bool {
    last_published
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |last| {
            (generation >= last).then_some(generation)
        })
        .is_ok()
}

/// Set the in-memory cache and emit the change event. Called at the end of every
/// recompute so cache and listeners never drift. Stale computes (an older
/// `generation` racing a newer publish) are dropped.
fn publish(app_handle: &tauri::AppHandle, status: &LicenseStatus, generation: u64) {
    if !try_claim_publish(&LAST_PUBLISHED_GENERATION, generation) {
        return;
    }
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

/// Compute the current status from keychain + crate evaluation, cache it, emit.
///
/// Order: anti-rollback bump first, then the Licensed branch (a valid stored key
/// wins outright — an invalid/garbage stored key falls through rather than
/// hard-erroring), else the keyless state (trial not started / issuance-ceiling
/// gate / trial-already-used, from the trial-issuance stamp).
pub async fn compute_license_status(
    pool: &SqlitePool,
    app_handle: &tauri::AppHandle,
    now_ms: i64,
) -> LicenseStatus {
    // Claimed before any state is read — the publish-ordering anchor.
    let generation = begin_compute();

    // Dev builds are never gated. Overridable with MNEMA_LICENSE_ENFORCE=1 to
    // exercise the real flow locally.
    // ponytail: compile-time bypass; release builds fall through to the real gate.
    if cfg!(debug_assertions) && std::env::var_os("MNEMA_LICENSE_ENFORCE").is_none() {
        let status = LicenseStatus::Licensed {
            update_through_ms: i64::MAX,
            in_window: true,
            email: "dev@localhost".into(),
            name: "Dev".into(),
            activation: Activation::Activated,
        };
        publish(app_handle, &status, generation);
        return status;
    }

    // Anti-rollback: record the high-water mark before reading it back below.
    let _ = app_infra::bump_max_timestamp_seen(pool, now_ms).await;

    let status = match compute_licensed(pool, app_handle, now_ms).await {
        Some(status) => status,
        // Keyless install: the trial-issuance stamp decides (not started /
        // day-7 unreachable gate / trial already used on this machine).
        None => trial::keyless_status(pool, now_ms).await,
    };
    publish(app_handle, &status, generation);
    status
}

/// Licensed branch: `Some(status)` when a valid signed key is stored (Revoked /
/// ReadOnly / Licensed per the adapter mapping), `None` when there is no key or
/// it fails verification. A revoked key does NOT fall through — `map_status`
/// returns `Revoked` (blocking) so a refunded key can't reclaim a trial.
async fn compute_licensed(
    pool: &SqlitePool,
    app_handle: &tauri::AppHandle,
    now_ms: i64,
) -> Option<LicenseStatus> {
    let verifier = adapter::verifier()?;
    let key_wire = app_infra::load_license_key().ok().flatten()?;
    let key = verifier.verify_license(&key_wire).ok()?;

    let crl = load_effective_crl(&verifier, pool).await;
    let max_seen = app_infra::read_max_timestamp_seen(pool)
        .await
        .unwrap_or(now_ms);
    // Mnema owns the clock: the guarded now is what the crate evaluates with.
    let guarded_now_ms = now_ms.max(max_seen);

    // ponytail: non-macOS can't fingerprint the machine, so it can never verify
    // a receipt or fairly time-box a window — map_status forces Activated there.
    let machine = app_infra::hardware_uuid().ok();
    let receipt = machine.as_ref().and_then(|uuid| {
        app_infra::load_activation_receipt()
            .ok()
            .flatten()
            .and_then(|wire| verifier.verify_receipt_bound(&wire, uuid).ok())
    });

    // No stamp yet → treat as freshly seen with a full window (compute never
    // writes; the store path and the background task stamp it).
    let first_seen_ms = adapter::first_seen_in(
        app_infra::load_first_seen().ok().flatten().as_deref(),
        &key.license_id,
    )
    .unwrap_or(guarded_now_ms);

    let eval = licensegate::evaluate(
        &key,
        receipt.as_ref(),
        crl.as_ref(),
        first_seen_ms / 1000,
        adapter::PROVISIONAL_WINDOW_DAYS,
        guarded_now_ms / 1000,
    );
    if eval.clock_tampered {
        // Log-only, never a lock: a broken clock never punishes a paying customer.
        tauri_plugin_log::log::warn!(target: "mnema_lib::licensing", "clock appears rolled back behind a held signed artifact (diagnostic only; gating uses the guarded clock)");
    }

    Some(adapter::map_status(
        &eval,
        &key,
        guarded_now_ms,
        machine.is_some(),
        read_over_cap_hint(app_handle),
    ))
}

/// The effective CRL for enforcement: the cached wire, re-verified on every
/// read — a tampered/garbage cache row contributes nothing. `None` when absent.
/// The CI-baked fresh-install floor is seeded INTO this cache at startup
/// through the same monotonic accept (`crl_refresh`), so the cache is the one
/// source here.
async fn load_effective_crl(
    verifier: &licensegate::Verifier,
    pool: &SqlitePool,
) -> Option<licensegate::Crl> {
    app_infra::load_cached_crl(pool)
        .await
        .ok()
        .flatten()
        .and_then(|wire| verifier.verify_crl(&wire).ok())
}

/// Verify a fetched CRL wire against this build's pinned key. Used by
/// `crl_refresh` so the verifier config lives in exactly one place.
pub(crate) fn verify_crl_wire(wire: &str) -> Option<licensegate::Crl> {
    adapter::verifier()?.verify_crl(wire).ok()
}

/// Pure membership check used by both the gate and the activation paths.
fn is_key_revoked(license_id: &str, crl: Option<&licensegate::Crl>) -> bool {
    crl.is_some_and(|crl| crl.revoked_license_ids.iter().any(|id| id == license_id))
}

/// Stamp `first_seen_at` beside the key: write-once per license id (a re-paste
/// never resets the clock; a genuinely different license gets its own stamp),
/// rollback-guarded because callers pass the guarded now.
fn ensure_first_seen(license_id: &str, guarded_now_ms: i64) {
    let stored = app_infra::load_first_seen().ok().flatten();
    if adapter::first_seen_in(stored.as_deref(), license_id).is_some() {
        return;
    }
    let record = adapter::FirstSeen {
        license_id: license_id.to_string(),
        first_seen_at_ms: guarded_now_ms,
    };
    if let Ok(json) = serde_json::to_string(&record) {
        let _ = app_infra::store_first_seen(&json);
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

    // Dev-only test knob: forget that issuance was ever attempted (or refused
    // as already-used). Stored keys stay put, and the server still remembers
    // this machine's trial — reset that on the licensegate side.
    #[cfg(debug_assertions)]
    if std::env::var_os("MNEMA_TRIAL_RESET").is_some() {
        match app_infra::clear_trial_issuance() {
            Ok(()) => crate::native_capture::debug_log::log_info(
                "MNEMA_TRIAL_RESET: cleared the trial-issuance stamp (stored keys untouched)",
            ),
            Err(error) => crate::native_capture::debug_log::log_error(format!(
                "MNEMA_TRIAL_RESET: could not clear the trial-issuance stamp: {error}"
            )),
        }
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

/// A blocking licensing decision for the capture seams: code + copy for the
/// start refusal, and the revoked flag the rotation-stop notification keys on.
/// Deliberately NOT a `CaptureSuspension`: a license block never self-heals and
/// is never transient liveness (ADR 0021/0040) — it clears only when the user
/// buys/activates, so it must not touch `capture_suspension`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LicenseBlock {
    pub(crate) code: &'static str,
    pub(crate) message: &'static str,
    pub(crate) revoked: bool,
}

/// The single gate decision both capture seams (start refusal, rotation stop)
/// share: `Some(block)` when `status` refuses capture at `now_ms`, `None` when
/// capture may proceed — including `status == None` (the deferred gate hasn't
/// run yet): treat unknown as allow, NEVER lock on unknown.
pub(crate) fn license_block(status: Option<&LicenseStatus>, now_ms: i64) -> Option<LicenseBlock> {
    let status = status.filter(|s| !s.capture_allowed_at(now_ms))?;
    let (code, message) = capture_refusal_copy(status);
    Some(LicenseBlock {
        code,
        message,
        revoked: matches!(status, LicenseStatus::Revoked),
    })
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
        // The empty email is the trial-issuance-ceiling discriminator: a real
        // key always carries the customer's email; only the keyless day-7 gate
        // (`trial::trial_ceiling_status`) manufactures a Lapsed with none.
        LicenseStatus::Licensed {
            activation: Activation::Lapsed,
            email,
            ..
        } if email.is_empty() => (
            "capture_refused_trial_unissued",
            "We couldn't reach the server to start your free trial — connect to the internet once to start it. Everything you already recorded stays browsable and searchable.",
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

/// Frontend door to the server-issued trial: attempts the mint → activate
/// chain and returns the recomputed status. Thin wrapper over
/// [`ensure_trial_started`].
#[tauri::command]
pub async fn start_trial(
    state: tauri::State<'_, AppInfraState>,
    app_handle: tauri::AppHandle,
) -> Result<LicenseStatus, String> {
    let infra = std::sync::Arc::clone(&*state);
    Ok(ensure_trial_started(infra.pool(), &app_handle, now_ms()).await)
}

/// The store-or-reject decision for a pasted/deep-linked key, pure over an
/// injected verifier (the real one is pinned to this build's public key and
/// can't mint in tests). A returned `Err` means the key must NEVER be stored:
/// garbage never sticks and an authentic-but-revoked key gets an honest,
/// distinct message.
fn vet_license_key(
    key: &str,
    crl: Option<&licensegate::Crl>,
    verify: impl Fn(&str) -> Result<licensegate::LicenseKey, licensegate::Error>,
) -> Result<licensegate::LicenseKey, String> {
    let payload = verify(key).map_err(|_| "This license key is invalid or corrupted.".to_string())?;
    if is_key_revoked(&payload.license_id, crl) {
        return Err("This license has been revoked.".to_string());
    }
    Ok(payload)
}

/// Shared verify → store → stamp → recompute path for a pasted or deep-linked
/// key. An invalid key is never stored and returns a human-readable error.
async fn install_license_key(
    pool: &SqlitePool,
    app_handle: &tauri::AppHandle,
    key: &str,
) -> Result<LicenseStatus, String> {
    let Some(verifier) = adapter::verifier() else {
        return Err("Licensing is not configured in this build.".to_string());
    };
    // Verify (and revocation-vet) BEFORE storing so a bad paste never sticks.
    let crl = load_effective_crl(&verifier, pool).await;
    let payload = vet_license_key(key, crl.as_ref(), |wire| verifier.verify_license(wire))?;

    app_infra::store_license_key(key)
        .map_err(|error| format!("Could not save the license key: {error}"))?;

    // first_seen_at: stamped at first key store, write-once per license id.
    let now = now_ms();
    let max_seen = app_infra::read_max_timestamp_seen(pool).await.unwrap_or(now);
    ensure_first_seen(&payload.license_id, now.max(max_seen));

    let status = compute_license_status(pool, app_handle, now).await;
    // Kick off once-per-machine activation in the background; the returned status
    // is the pending one, and the receipt flips it to Activated when it lands.
    maybe_spawn_activation(app_handle);
    Ok(status)
}

/// Verify + store a pasted license key, then recompute.
#[tauri::command]
pub async fn activate_license(
    key: String,
    state: tauri::State<'_, AppInfraState>,
    app_handle: tauri::AppHandle,
) -> Result<ActivateLicenseResult, String> {
    let infra = std::sync::Arc::clone(&*state);
    let status = install_license_key(infra.pool(), &app_handle, &key).await?;
    Ok(ActivateLicenseResult { status })
}

/// Pure decision for the deep-link overwrite guard: confirmation is needed
/// only when a healthy Licensed machine would swap to a *different* license.
fn replacing_healthy_license(
    status: Option<&LicenseStatus>,
    stored_id: Option<&str>,
    incoming_id: &str,
) -> bool {
    matches!(status, Some(LicenseStatus::Licensed { .. }))
        && stored_id.is_some_and(|stored| stored != incoming_id)
}

/// Deep-link overwrite guard (2026-07-18, `docs/licensing/CONTEXT.md`): any
/// webpage can fire the activate/claim deep links, so a link that would
/// replace a healthy Licensed key with a *different* license asks first —
/// closing the silent-swap shape (attacker installs their own valid key, then
/// refunds it; the victim drops to Revoked weeks later with no visible cause).
/// Paste, fresh installs, same-license replays, and unhealthy states stay
/// frictionless. An incoming key that doesn't verify never needs the dialog —
/// `install_license_key` rejects it on its own.
pub(crate) fn deep_link_replacement_needs_confirm(
    app_handle: &tauri::AppHandle,
    incoming_key: &str,
) -> bool {
    let Some(verifier) = adapter::verifier() else {
        return false;
    };
    let Ok(incoming) = verifier.verify_license(incoming_key) else {
        return false;
    };
    let stored_id = app_infra::load_license_key()
        .ok()
        .flatten()
        .and_then(|stored| verifier.verify_license(&stored).ok())
        .map(|payload| payload.license_id);
    replacing_healthy_license(
        cached_status(app_handle).as_ref(),
        stored_id.as_deref(),
        &incoming.license_id,
    )
}

/// Blocking replace-license confirm for the deep-link paths. Resolves `false`
/// on Cancel, dismissal, or a dropped dialog — declining is always the safe
/// default.
pub(crate) async fn confirm_license_replacement(app_handle: &tauri::AppHandle) -> bool {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
    let (tx, rx) = tokio::sync::oneshot::channel();
    app_handle
        .dialog()
        .message(
            "This link would replace the license currently active on this Mac \
             with a different one. Only continue if you expected this.",
        )
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Replace License".to_string(),
            "Keep Current License".to_string(),
        ))
        .kind(MessageDialogKind::Warning)
        .title("Replace your Mnema license?")
        .show(move |confirmed| {
            let _ = tx.send(confirmed);
        });
    rx.await.unwrap_or(false)
}

/// Activate from a `mnema://license/activate?key=…` deep link. Same verify →
/// store → recompute path as [`activate_license`], but callable from the
/// deep-link handler in `lib.rs` where we only hold an `AppHandle`. Success
/// surfaces to the UI through the emitted `license_status` event (the store
/// re-renders); failures are logged, not shown — a bad link just leaves the
/// current status untouched.
pub async fn activate_from_deep_link(app_handle: tauri::AppHandle, key: String) {
    let Some(state) = app_handle.try_state::<AppInfraState>() else {
        return;
    };
    if deep_link_replacement_needs_confirm(&app_handle, &key)
        && !confirm_license_replacement(&app_handle).await
    {
        tauri_plugin_log::log::info!(target: "mnema_lib::licensing", "deep-link license replacement declined by the user");
        emit_deep_link_done(&app_handle, "closed", None);
        return;
    }
    let infra = std::sync::Arc::clone(&*state);
    if let Err(error) = install_license_key(infra.pool(), &app_handle, &key).await {
        tauri_plugin_log::log::warn!(target: "mnema_lib::licensing", "deep-link license key rejected: {error}");
        emit_deep_link_done(
            &app_handle,
            "failed",
            Some(
                "This license key couldn't be verified. If it arrived by email, \
                 paste it in Settings → License.",
            ),
        );
    }
}

/// Manual Receipt Refresh (ADR 0055): the Settings "Refresh license status"
/// button and the Read-Only screen's "Re-check license". Forces a
/// re-activation; the recomputed status reaches the frontend through the
/// existing `license_status` event. `Err` only means the server couldn't be
/// reached (it feeds the button's transient "failed" note) — the stored
/// receipt stays either way; staleness never locks.
#[tauri::command]
pub async fn refresh_license_now(app_handle: tauri::AppHandle) -> Result<(), String> {
    if activation::refresh_receipt(app_handle).await {
        Ok(())
    } else {
        Err("could not reach the license server".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crl_naming(id: &str) -> licensegate::Crl {
        licensegate::Crl {
            kid: "56475aa7".to_string(),
            issued_at: "2026-07-10T00:00:00Z".to_string(),
            revoked_license_ids: vec![id.to_string()],
        }
    }

    fn key_with_id(license_id: &str) -> licensegate::LicenseKey {
        licensegate::LicenseKey {
            kid: "24f6ed6a".into(),
            license_id: license_id.into(),
            plan: "pro".into(),
            customer: licensegate::Customer {
                name: "Owner".into(),
                email: "owner@example.com".into(),
            },
            entitlements: vec![licensegate::Entitlement {
                key: "app".into(),
                expires_at: None,
            }],
            issued_at: "2026-07-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn stale_compute_never_publishes_over_a_newer_one() {
        let last = AtomicU64::new(0);
        // In-order publishes all land.
        assert!(try_claim_publish(&last, 1));
        assert!(try_claim_publish(&last, 2));
        // A compute that started BEFORE the last published one is dropped —
        // this is the CRL-refresh race: stale `Licensed` can't overwrite `Revoked`.
        assert!(!try_claim_publish(&last, 1));
        // The newest generation republishing (same compute re-checked) still lands.
        assert!(try_claim_publish(&last, 2));
        assert!(try_claim_publish(&last, 3));
    }

    #[test]
    fn is_key_revoked_hit_miss_and_no_crl() {
        let crl = crl_naming("01PRESS");
        assert!(is_key_revoked("01PRESS", Some(&crl)));
        assert!(!is_key_revoked("01FRIEND", Some(&crl)));
        assert!(!is_key_revoked("01PRESS", None));
    }

    #[test]
    fn deep_link_confirm_only_guards_healthy_license_swaps() {
        let licensed = licensed_with(Activation::Activated);
        // The one guarded shape: healthy Licensed + a different incoming id.
        assert!(replacing_healthy_license(Some(&licensed), Some("01OLD"), "01NEW"));
        // Same license id (replayed link) → frictionless.
        assert!(!replacing_healthy_license(Some(&licensed), Some("01OLD"), "01OLD"));
        // No stored key (fresh claim) → frictionless.
        assert!(!replacing_healthy_license(Some(&licensed), None, "01NEW"));
        // Unhealthy states: replacement can only help → frictionless.
        for status in [LicenseStatus::ReadOnly, LicenseStatus::Revoked] {
            assert!(!replacing_healthy_license(Some(&status), Some("01OLD"), "01NEW"));
        }
        assert!(!replacing_healthy_license(None, Some("01OLD"), "01NEW"));
    }

    // ── license_block: the shared capture-gate decision ────────────────────

    // A real key's status always carries the customer email — an empty email
    // on `Lapsed` means the trial-issuance ceiling (see `capture_refusal_copy`).
    fn licensed_with(activation: Activation) -> LicenseStatus {
        LicenseStatus::Licensed {
            update_through_ms: 0,
            in_window: true,
            email: "owner@example.com".into(),
            name: String::new(),
            activation,
        }
    }

    #[test]
    fn license_block_blocks_with_the_right_code_and_revoked_flag() {
        let block = license_block(Some(&LicenseStatus::ReadOnly), 0).expect("blocked");
        assert_eq!(block.code, "capture_refused_read_only");
        assert!(!block.revoked);

        let block = license_block(Some(&LicenseStatus::Revoked), 0).expect("blocked");
        assert_eq!(block.code, "capture_refused_revoked");
        assert!(block.revoked);

        let block =
            license_block(Some(&licensed_with(Activation::Lapsed)), 0).expect("blocked");
        assert_eq!(block.code, "capture_refused_unactivated");
        assert!(!block.revoked);

        // A cached Trial whose window has since lapsed blocks at `now` — the
        // first start after expiry must not slip through on the stale cache.
        let stale_trial = LicenseStatus::Trial {
            days_left: 1,
            trial_end_ms: 1_000,
        };
        let block = license_block(Some(&stale_trial), 1_000).expect("blocked");
        assert_eq!(block.code, "capture_refused_read_only");
    }

    #[test]
    fn license_block_allows_running_states_and_never_locks_on_unknown() {
        // Unknown (deferred gate hasn't run) → allow.
        assert_eq!(license_block(None, i64::MAX), None);
        for status in [
            LicenseStatus::TrialNotStarted { trial_days: 30 },
            LicenseStatus::Trial {
                days_left: 5,
                trial_end_ms: i64::MAX,
            },
            licensed_with(Activation::Activated),
            licensed_with(Activation::Pending {
                provisional_days_left: 3,
            }),
            licensed_with(Activation::RefusedOverCap {
                reset_url: "https://reset".into(),
                buy_url: "https://buy".into(),
            }),
        ] {
            assert_eq!(license_block(Some(&status), 0), None, "{status:?}");
        }
    }

    // ── vet_license_key: the store-or-reject decision ──────────────────────

    #[test]
    fn vet_rejects_garbage_before_any_store() {
        let err = vet_license_key("garbage", None, |_| Err(licensegate::Error::Envelope))
            .expect_err("garbage must be rejected");
        assert!(err.contains("invalid or corrupted"));
    }

    #[test]
    fn vet_rejects_authentic_but_revoked_with_a_distinct_message() {
        let crl = crl_naming("01REFUNDED");
        let err = vet_license_key("payload.sig", Some(&crl), |_| Ok(key_with_id("01REFUNDED")))
            .expect_err("revoked must be rejected");
        assert!(err.contains("revoked"));
        assert!(!err.contains("invalid"));
    }

    #[test]
    fn vet_accepts_a_valid_unrevoked_key() {
        let crl = crl_naming("01SOMEONE_ELSE");
        let payload = vet_license_key("payload.sig", Some(&crl), |_| Ok(key_with_id("01MINE")))
            .expect("valid key must pass");
        assert_eq!(payload.license_id, "01MINE");
    }

    #[test]
    fn capture_refusal_copy_is_honest_per_state() {
        // Revoked reads as "revoked" with its own code — never trial-ended copy.
        let (code, message) = capture_refusal_copy(&LicenseStatus::Revoked);
        assert_eq!(code, "capture_refused_revoked");
        assert!(message.contains("revoked"));
        assert!(!message.contains("trial"));

        // A lapsed activation reads as the unactivated copy — not trial, not revoked.
        let (code, message) = capture_refusal_copy(&licensed_with(Activation::Lapsed));
        assert_eq!(code, "capture_refused_unactivated");
        assert!(message.contains("activation"));
        assert!(!message.contains("trial"));
        assert!(!message.contains("revoked"));

        // The keyless issuance ceiling (Lapsed with no customer email) reads as
        // "connect once to start your trial" — never the finish-activation copy.
        let (code, message) = capture_refusal_copy(&trial::trial_ceiling_status());
        assert_eq!(code, "capture_refused_trial_unissued");
        assert!(message.contains("trial"));
        assert!(message.contains("connect"));
        assert!(!message.contains("activation"));

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
