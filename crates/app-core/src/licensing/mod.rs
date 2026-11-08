//! Licensing gate over the licensegate client crate: computes the offline
//! `LicenseStatus`, caches it for synchronous reads by other seams
//! (capture-stop, status bar), emits changes to the frontend, and exposes the
//! command bodies + the deferred-startup entry point.
//!
//! Division of labor: the `licensegate` crate owns verification (`Verifier`)
//! and the state machine (`evaluate`); `adapter` owns config + the pure
//! `Evaluation` → wire mapping; app-infra owns the keychain store and the
//! anti-rollback high-water mark; this module is the IO orchestration.

mod activation;
pub mod adapter;
mod claim;
// pub: its timer is started from deferred startup in the host.
pub mod receipt_refresh;
mod renewed;
pub mod reset;
mod trial;

pub use activation::maybe_spawn_activation;
use activation::read_over_cap_hint;
pub use claim::claim_from_deep_link;
pub use renewed::renewed_from_deep_link;
pub use trial::ensure_trial_started;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use capture_types::{ActivateLicenseResult, Activation, LicenseStatus};
use sqlx::SqlitePool;

use crate::{AppCore, AppInfraState, ConfirmDialog, CoreEvent, DialogKind};

/// Event emitted after every status recompute. Payload is the `LicenseStatus`
/// (the camelCase tagged shape mirrored in `apps/desktop/src/lib/licensing.ts`).
pub const LICENSE_STATUS_EVENT: &str = "license_status";

/// Terminal deep-link endings that will never produce a `license_status` emit —
/// without this the deep-link receipt modal would spin forever on them.
/// `failed` carries a human message and shows the modal's failed face; `closed`
/// closes the receipt silently (the user declined the replacement confirm, or
/// the claim path handed off to its native email dialog). Hand-mirrored in
/// `$lib/license-deeplink-receipt.ts` (`LicenseDeepLinkDone`).
pub const LICENSE_DEEP_LINK_DONE_EVENT: &str = "license_deep_link_done";

#[derive(Debug, Clone, serde::Serialize)]
struct LicenseDeepLinkDonePayload {
    /// "failed" | "closed"
    outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<&'static str>,
}

pub(crate) fn emit_deep_link_done(
    core: &AppCore,
    outcome: &'static str,
    message: Option<&'static str>,
) {
    if let Ok(payload) = serde_json::to_value(LicenseDeepLinkDonePayload { outcome, message }) {
        core.host().emit(CoreEvent::LicenseDeepLinkDone(payload));
    }
}

/// In-memory cache of the latest computed status. Other seams — the capture
/// gate and the status bar — read it synchronously via [`cached_status`]
/// instead of touching the DB/keychain on the hot path. `.manage(...)`-
/// registered on the `AppCore` by the host.
pub struct LicenseGate(pub Mutex<Option<LicenseStatus>>);

/// In-memory over-cap hint: `(reset_url, buy_url)` set by the background
/// activation task on a `device_cap_reached` refusal and cleared on success.
/// The compute path reads it so a `Licensed` key in its Provisional Window
/// surfaces as `Activation::RefusedOverCap` instead of plain `Pending`.
/// Registered on the `AppCore` by the host next to `LicenseGate`.
// ponytail: unkeyed by license id — only one license is active at a time and the
// task clears it on success; key it if multi-license ever lands.
pub struct ActivationHint(pub Mutex<Option<(String, String)>>);

/// "Now" in unix milliseconds (UTC). Shared with the capture-gate seams
/// (lifecycle start refusal, rotation boundary, status bar) so they all ask
/// the same clock question as the gate itself.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Fire-and-forget status recompute. Used by the capture seams when they catch
/// a lapsed status on the synchronous path: the refusal/stop doesn't wait, but
/// the cache/tray/Settings flip to the fresh state.
pub fn recompute_status_async(core: &Arc<AppCore>, now_ms: i64) {
    // A refusal is the user's "why can't I record?" moment — kick the
    // background fixer too (trial issuance / activation), so connecting to the
    // internet and pressing Record again is enough to self-heal.
    maybe_spawn_activation(core);
    let Some(infra) = core.try_state::<AppInfraState>() else {
        return;
    };
    let infra = (*infra).clone();
    let task_core = Arc::clone(core);
    core.spawn(async move {
        compute_license_status(infra.pool(), &task_core, now_ms).await;
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
///
/// The host reacts to `LicenseStatus` after delivering it: it re-checks for
/// updates when the status changed (the update gate reads this status, so a
/// renewal or a Refresh flips a stale `availableOutOfWindow` now, not on the next
/// 2 h tick), then refreshes the tray (a flip to/from Read-Only Mode changes the
/// tray's status header + Start-Recording enablement, and nothing else rebuilds
/// the menu on a licensing change).
fn publish(core: &AppCore, status: &LicenseStatus, generation: u64) {
    if !try_claim_publish(&LAST_PUBLISHED_GENERATION, generation) {
        return;
    }
    if let Some(gate) = core.try_state::<LicenseGate>() {
        if let Ok(mut slot) = gate.0.lock() {
            *slot = Some(status.clone());
        }
    }
    if let Ok(payload) = serde_json::to_value(status) {
        core.host().emit(CoreEvent::LicenseStatus(payload));
    }
}

/// The last computed status, for synchronous reads by the capture gate / status
/// bar. `None` before the deferred-startup gate has run once.
pub fn cached_status(core: &AppCore) -> Option<LicenseStatus> {
    core.try_state::<LicenseGate>()
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
    core: &AppCore,
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
        publish(core, &status, generation);
        return status;
    }

    // Anti-rollback: record the high-water mark before reading it back below.
    let _ = app_infra::bump_max_timestamp_seen(pool, now_ms).await;

    let status = match compute_licensed(pool, core, now_ms).await {
        Some(status) => status,
        // Keyless install: the trial-issuance stamp decides (not started /
        // day-7 unreachable gate / trial already used on this machine).
        None => trial::keyless_status(pool, now_ms).await,
    };
    publish(core, &status, generation);
    status
}

/// Licensed branch: `Some(status)` when a valid signed key is stored (Revoked /
/// ReadOnly / Licensed per the adapter mapping), `None` when there is no key or
/// it fails verification. A revoked key does NOT fall through — `map_status`
/// returns `Revoked` (blocking) so a refunded key can't reclaim a trial.
async fn compute_licensed(
    pool: &SqlitePool,
    core: &AppCore,
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
        log::warn!(target: "mnema_lib::licensing", "clock appears rolled back behind a held signed artifact (diagnostic only; gating uses the guarded clock)");
    }

    Some(adapter::map_status(
        &eval,
        &key,
        guarded_now_ms,
        machine.is_some(),
        read_over_cap_hint(core),
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
pub fn verify_crl_wire(wire: &str) -> Option<licensegate::Crl> {
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
pub fn run_license_gate(core: &Arc<AppCore>) {
    let Some(infra) = core.try_state::<AppInfraState>() else {
        crate::debug_log::log_error(
            "app infrastructure state was not initialized; skipping license gate",
        );
        return;
    };
    let infra = (*infra).clone();

    // Dev-only test knob: forget that issuance was ever attempted (or refused
    // as already-used). Stored keys stay put, and the server still remembers
    // this machine's trial — reset that on the licensegate side.
    #[cfg(debug_assertions)]
    if std::env::var_os("MNEMA_TRIAL_RESET").is_some() {
        match app_infra::clear_trial_issuance() {
            Ok(()) => crate::debug_log::log_info(
                "MNEMA_TRIAL_RESET: cleared the trial-issuance stamp (stored keys untouched)",
            ),
            Err(error) => crate::debug_log::log_error(format!(
                "MNEMA_TRIAL_RESET: could not clear the trial-issuance stamp: {error}"
            )),
        }
    }

    let status = core.block_on(compute_license_status(infra.pool(), core, now_ms()));
    crate::debug_log::log_info(format!(
        "license gate computed status: {}",
        status_label(&status)
    ));
    // Try to finish once-per-machine activation on startup (no-op if already
    // activated, no key, or revoked). Retries piggyback the daily CRL tick.
    maybe_spawn_activation(core);
}

/// A blocking licensing decision for the capture seams: code + copy for the
/// start refusal, and the revoked flag the rotation-stop notification keys on.
/// Deliberately NOT a `CaptureSuspension`: a license block never self-heals and
/// is never transient liveness (ADR 0021/0040) — it clears only when the user
/// buys/activates, so it must not touch `capture_suspension`.
#[derive(Debug, PartialEq, Eq)]
pub struct LicenseBlock {
    pub code: &'static str,
    pub message: &'static str,
    pub revoked: bool,
}

/// The single gate decision both capture seams (start refusal, rotation stop)
/// share: `Some(block)` when `status` refuses capture at `now_ms`, `None` when
/// capture may proceed — including `status == None` (the deferred gate hasn't
/// run yet): treat unknown as allow, NEVER lock on unknown.
pub fn license_block(status: Option<&LicenseStatus>, now_ms: i64) -> Option<LicenseBlock> {
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
pub fn capture_refusal_copy(status: &LicenseStatus) -> (&'static str, &'static str) {
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
// Command bodies (the host's command names must stay byte-identical to the
// frontend `invoke(...)`).
// ---------------------------------------------------------------------------

/// Frontend snapshot/reattach: return the cached status if present, else compute
/// (which caches + emits) once.
pub async fn get_license_status(
    core: &AppCore,
    state: &AppInfraState,
) -> Result<LicenseStatus, String> {
    if let Some(status) = cached_status(core) {
        return Ok(status);
    }
    let infra = Arc::clone(state);
    Ok(compute_license_status(infra.pool(), core, now_ms()).await)
}

/// Frontend door to the server-issued trial: attempts the mint → activate
/// chain and returns the recomputed status. Thin wrapper over
/// [`ensure_trial_started`].
pub async fn start_trial(
    core: &Arc<AppCore>,
    state: &AppInfraState,
) -> Result<LicenseStatus, String> {
    let infra = Arc::clone(state);
    Ok(ensure_trial_started(infra.pool(), core, now_ms()).await)
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
    core: &Arc<AppCore>,
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

    let status = compute_license_status(pool, core, now).await;
    // Kick off once-per-machine activation in the background; the returned status
    // is the pending one, and the receipt flips it to Activated when it lands.
    maybe_spawn_activation(core);
    Ok(status)
}

/// Verify + store a pasted license key, then recompute.
pub async fn activate_license(
    key: String,
    core: &Arc<AppCore>,
    state: &AppInfraState,
) -> Result<ActivateLicenseResult, String> {
    let infra = Arc::clone(state);
    let status = install_license_key(infra.pool(), core, &key).await?;
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
    core: &AppCore,
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
        cached_status(core).as_ref(),
        stored_id.as_deref(),
        &incoming.license_id,
    )
}

/// Blocking replace-license confirm for the deep-link paths. Resolves `false`
/// on Cancel, dismissal, or a dropped dialog — declining is always the safe
/// default.
pub(crate) async fn confirm_license_replacement(core: &AppCore) -> bool {
    let rx = core.host().confirm(ConfirmDialog {
        title: Some("Replace your Mnema license?".to_string()),
        message: "This link would replace the license currently active on this Mac \
             with a different one. Only continue if you expected this."
            .to_string(),
        kind: DialogKind::Warning,
        ok_label: "Replace License".to_string(),
        cancel_label: "Keep Current License".to_string(),
    });
    rx.await.unwrap_or(false)
}

/// Activate from a `mnema://license/activate?key=…` deep link. Same verify →
/// store → recompute path as [`activate_license`], but callable from the
/// host's deep-link handler. Success
/// surfaces to the UI through the emitted `license_status` event (the store
/// re-renders); failures are logged, not shown — a bad link just leaves the
/// current status untouched.
pub async fn activate_from_deep_link(core: Arc<AppCore>, key: String) {
    let Some(state) = core.try_state::<AppInfraState>() else {
        return;
    };
    if deep_link_replacement_needs_confirm(&core, &key)
        && !confirm_license_replacement(&core).await
    {
        log::info!(target: "mnema_lib::licensing", "deep-link license replacement declined by the user");
        emit_deep_link_done(&core, "closed", None);
        return;
    }
    let infra = Arc::clone(&*state);
    if let Err(error) = install_license_key(infra.pool(), &core, &key).await {
        log::warn!(target: "mnema_lib::licensing", "deep-link license key rejected: {error}");
        emit_deep_link_done(
            &core,
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
pub async fn refresh_license_now(core: Arc<AppCore>) -> Result<(), String> {
    if activation::refresh_receipt(core).await {
        Ok(())
    } else {
        Err("could not reach the license server".to_string())
    }
}

#[cfg(test)]
mod tests;
