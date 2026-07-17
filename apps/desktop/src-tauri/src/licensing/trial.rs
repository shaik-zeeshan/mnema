//! Server-issued trial flow: at first Capture the app asks licensegate to mint
//! a trial key (`POST /v1/trials`, product-salted machine hash) and immediately
//! chains activation — online users end fully Activated in one breath. Capture
//! never blocks on issuance: a failed request leaves capture running and the
//! app retries at launch, on the daily CRL tick, and when a refused start
//! recomputes. After [`TRIAL_ISSUANCE_CEILING_MS`] of never reaching the server
//! since the FIRST attempted issuance, capture pauses ("connect once to start
//! your trial") until one issuance succeeds.
//!
//! The first-attempt stamp lives in the keychain (`licensegate_trial_issuance`),
//! is write-once, and is rollback-guarded: it is minted at the *guarded* now and
//! compared against the guarded now, so winding the clock back never stretches
//! the pre-issuance window.

use capture_types::{Activation, LicenseStatus};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use super::{activation, adapter};

/// 7 days: the one grace policy shared with the Provisional Window — pre-issuance
/// offline rides the same leash as never-finished activation.
const TRIAL_ISSUANCE_CEILING_MS: i64 = 7 * 86_400_000;

/// The offline-issuance ceiling, overridable in debug builds via
/// `MNEMA_TRIAL_LEN_MS` — the only client-side trial duration left (the 30 days
/// themselves are server-issued now), repurposed to make the day-7 gate testable.
fn ceiling_ms() -> i64 {
    if cfg!(debug_assertions) {
        if let Some(ms) = std::env::var("MNEMA_TRIAL_LEN_MS")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
        {
            return ms;
        }
    }
    TRIAL_ISSUANCE_CEILING_MS
}

/// Keychain record under `licensegate_trial_issuance`: when issuance was first
/// ATTEMPTED (not when it succeeded — success stores a key, which supersedes
/// this record entirely), plus whether the server declared this machine's
/// trial already spent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct TrialIssuance {
    pub(super) first_attempt_at_ms: i64,
    #[serde(default)]
    pub(super) used: bool,
}

fn load_stamp() -> Option<TrialIssuance> {
    app_infra::load_trial_issuance()
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
}

fn store_stamp(stamp: &TrialIssuance) {
    if let Ok(json) = serde_json::to_string(stamp) {
        let _ = app_infra::store_trial_issuance(&json);
    }
}

/// Write-once: an existing stamp is never replaced (re-attempts must not slide
/// the ceiling); only a first attempt mints one, at the guarded now.
fn stamped_first_attempt(existing: Option<TrialIssuance>, guarded_now_ms: i64) -> TrialIssuance {
    existing.unwrap_or(TrialIssuance {
        first_attempt_at_ms: guarded_now_ms,
        used: false,
    })
}

// ---------------------------------------------------------------------------
// Keyless status: what a machine without a stored key reads as.
// ---------------------------------------------------------------------------

/// The keyless branch of the gate (IO shell over [`keyless_status_from`]).
pub(super) async fn keyless_status(pool: &SqlitePool, now_ms: i64) -> LicenseStatus {
    let max_seen = app_infra::read_max_timestamp_seen(pool)
        .await
        .unwrap_or(now_ms);
    keyless_status_from(load_stamp().as_ref(), now_ms.max(max_seen), ceiling_ms())
}

/// Pure keyless decision, over the guarded now.
pub(super) fn keyless_status_from(
    stamp: Option<&TrialIssuance>,
    guarded_now_ms: i64,
    ceiling_ms: i64,
) -> LicenseStatus {
    match stamp {
        // The server declared this machine's trial spent (keychain-wiped
        // reinstall after a trial): honest expired-trial UX with the buy door.
        Some(TrialIssuance { used: true, .. }) => LicenseStatus::ReadOnly,
        // Issuance attempted but the server never reached for the whole
        // ceiling: pause capture until one issuance succeeds.
        Some(stamp) if guarded_now_ms - stamp.first_attempt_at_ms >= ceiling_ms => {
            trial_ceiling_status()
        }
        // Never attempted, or attempted recently: capture allowed.
        _ => LicenseStatus::TrialNotStarted {
            trial_days: adapter::trial_len_days(),
        },
    }
}

/// The day-7 unreachable-server gate rides `Licensed` + `Lapsed` — the existing
/// blocking "connect once" state (wire types are frozen; no new variant). The
/// empty `email` is the discriminator `capture_refusal_copy` keys the
/// "start your trial" copy on: a keyless machine has no customer identity by
/// construction, while a real key always carries one.
pub(super) fn trial_ceiling_status() -> LicenseStatus {
    LicenseStatus::Licensed {
        update_through_ms: i64::MAX,
        in_window: true,
        email: String::new(),
        name: String::new(),
        activation: Activation::Lapsed,
    }
}

// ---------------------------------------------------------------------------
// Issuance: the mint → activate chain and its retry hooks.
// ---------------------------------------------------------------------------

/// `true` when a mint retry is due: keyless, an attempt was made, and the
/// server hasn't declared the trial spent. Never `true` before first capture
/// (no stamp) — the trial starts at first Capture, not at launch.
pub(super) fn pending_issuance() -> bool {
    app_infra::load_license_key().ok().flatten().is_none()
        && load_stamp().is_some_and(|s| !s.used)
}

/// First-successful-Capture seam (also the `start_trial` command and every
/// retry hook): attempt issuance if one is due, then recompute. Callers spawn
/// this — capture never waits on it.
pub async fn ensure_trial_started(
    pool: &SqlitePool,
    app_handle: &tauri::AppHandle,
    now_ms: i64,
) -> LicenseStatus {
    attempt_issuance(pool, app_handle, now_ms).await;
    super::compute_license_status(pool, app_handle, now_ms).await
}

async fn attempt_issuance(pool: &SqlitePool, app_handle: &tauri::AppHandle, now_ms: i64) {
    // Dev builds are never gated and must not hit the server on every capture.
    if cfg!(debug_assertions) && std::env::var_os("MNEMA_LICENSE_ENFORCE").is_none() {
        return;
    }
    let Some(verifier) = adapter::verifier() else {
        return;
    };
    // A stored, verifiable key (trial or paid) means issuance already happened;
    // the activation task owns everything from here.
    if app_infra::load_license_key()
        .ok()
        .flatten()
        .is_some_and(|wire| verifier.verify_license(&wire).is_ok())
    {
        return;
    }
    let stamp = load_stamp();
    if stamp.as_ref().is_some_and(|s| s.used) {
        return; // trial spent — ReadOnly; nothing to ask the server
    }
    // No hardware fingerprint (non-macOS): no trial hash, no issuance — and no
    // stamp either, so the offline ceiling can never gate those platforms.
    let Ok(uuid) = app_infra::hardware_uuid() else {
        return;
    };

    let max_seen = app_infra::read_max_timestamp_seen(pool)
        .await
        .unwrap_or(now_ms);
    let stamp = stamped_first_attempt(stamp, now_ms.max(max_seen));
    store_stamp(&stamp);

    // Product-salted trial hash (licensegate SPEC §2) — unlinkable to the
    // activation hash by construction, never the raw uuid.
    let machine_hash = licensegate::trial_machine_hash(adapter::product_slug(), &uuid);
    let result = adapter::client().trial(&machine_hash).await;
    match classify_trial(result, |wire| verifier.verify_license(wire).is_ok()) {
        TrialDisposition::Install(key_wire) => {
            if let Err(error) = app_infra::store_license_key(&key_wire) {
                tauri_plugin_log::log::warn!(target: "mnema_lib::licensing", "trial key issued but could not be stored: {error}");
                return;
            }
            tauri_plugin_log::log::info!(target: "mnema_lib::licensing", "trial issued; chaining activation");
            // One breath: mint → activate. `run_activation` stamps first_seen
            // and stores the receipt; a failure here just leaves the ordinary
            // 7-day Provisional Window running (daily tick retries).
            activation::run_activation(app_handle.clone()).await;
        }
        TrialDisposition::MarkUsed => {
            store_stamp(&TrialIssuance {
                used: true,
                ..stamp
            });
            tauri_plugin_log::log::warn!(target: "mnema_lib::licensing", "trial refused: already used on this machine");
        }
        TrialDisposition::Retry { reason } => {
            tauri_plugin_log::log::warn!(target: "mnema_lib::licensing", "trial issuance failed: {reason} (from {}); capture stays available until the offline ceiling", adapter::base_url());
        }
    }
}

/// What to do with the trial call's outcome — pure and IO-free
/// (`attempt_issuance` is the shell that executes it).
#[derive(Debug, PartialEq, Eq)]
enum TrialDisposition {
    /// Key that verifies for this build: store it, chain activation.
    Install(String),
    /// `trial_already_used`: this machine's trial is spent forever. Persist
    /// the fact so the ReadOnly mapping survives offline restarts.
    MarkUsed,
    /// Everything else (offline, server down, unverifiable key, unknown or
    /// config refusals): change nothing — capture keeps running and the
    /// retry hooks try again. `reason` feeds the log line.
    Retry { reason: String },
}

/// Classify a trial-issuance result. `verify` is the license check for this
/// build's pinned key (injected so the accept path is testable without crypto).
fn classify_trial(
    result: Result<licensegate::api::TrialOutcome, licensegate::ApiError>,
    verify: impl Fn(&str) -> bool,
) -> TrialDisposition {
    use licensegate::{ApiError, RefusalCode};
    match result {
        Ok(outcome) => {
            if verify(&outcome.license_key) {
                TrialDisposition::Install(outcome.license_key)
            } else {
                TrialDisposition::Retry {
                    reason: "server trial key did not verify".to_string(),
                }
            }
        }
        Err(ApiError::Refused {
            code: RefusalCode::TrialAlreadyUsed,
            ..
        }) => TrialDisposition::MarkUsed,
        Err(ApiError::Refused { code, message }) => TrialDisposition::Retry {
            reason: format!("refused: {code:?} ({message})"),
        },
        Err(ApiError::Transport(error)) => TrialDisposition::Retry {
            reason: format!("transport: {error}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use licensegate::api::TrialOutcome;
    use licensegate::{ApiError, RefusalCode};

    const DAY_MS: i64 = 86_400_000;
    const CEILING: i64 = TRIAL_ISSUANCE_CEILING_MS;

    fn stamp_at(first_attempt_at_ms: i64) -> TrialIssuance {
        TrialIssuance {
            first_attempt_at_ms,
            used: false,
        }
    }

    // ── keyless mapping table ───────────────────────────────────────────────

    #[test]
    fn keyless_table_never_attempted_recent_ceiling_and_used() {
        // Never attempted → trial not started, capture allowed.
        let status = keyless_status_from(None, i64::MAX, CEILING);
        assert_eq!(status, LicenseStatus::TrialNotStarted { trial_days: 30 });
        assert!(status.capture_allowed_at(i64::MAX));

        // Attempted, still inside the ceiling → capture stays allowed while
        // the app quietly retries (one ms short of the boundary).
        let status = keyless_status_from(Some(&stamp_at(0)), CEILING - 1, CEILING);
        assert_eq!(status, LicenseStatus::TrialNotStarted { trial_days: 30 });

        // At the ceiling → the blocking "connect once to start your trial" gate.
        let status = keyless_status_from(Some(&stamp_at(0)), CEILING, CEILING);
        assert_eq!(status, trial_ceiling_status());
        assert!(!status.capture_allowed_at(CEILING));

        // Server said trial_already_used → ReadOnly (trial-expired UX, buy door).
        let used = TrialIssuance {
            first_attempt_at_ms: 0,
            used: true,
        };
        assert_eq!(
            keyless_status_from(Some(&used), 1, CEILING),
            LicenseStatus::ReadOnly
        );
    }

    #[test]
    fn ceiling_gate_blocks_capture_with_the_trial_copy_not_the_activation_copy() {
        let block =
            super::super::license_block(Some(&trial_ceiling_status()), 0).expect("blocked");
        assert_eq!(block.code, "capture_refused_trial_unissued");
        assert!(!block.revoked);
        assert!(block.message.contains("trial"));
        assert!(block.message.contains("connect"));
        assert!(!block.message.contains("activation"));
    }

    // ── first-attempt stamp: write-once + rollback-guarded ─────────────────

    #[test]
    fn first_attempt_stamp_is_write_once() {
        // First attempt mints at the guarded now.
        assert_eq!(stamped_first_attempt(None, 500), stamp_at(500));
        // Every later attempt keeps the original stamp — retries never slide
        // the ceiling forward.
        assert_eq!(
            stamped_first_attempt(Some(stamp_at(500)), 500 + 6 * DAY_MS),
            stamp_at(500)
        );
    }

    #[test]
    fn winding_the_clock_back_never_stretches_the_ceiling() {
        // Stamp at T; the high-water mark has seen T+7d; the wall clock is
        // wound back to T+1d. The guarded now (max of the two) still gates.
        let stamp = stamp_at(0);
        let max_seen = CEILING;
        let rolled_back_wall = DAY_MS;
        let guarded_now = rolled_back_wall.max(max_seen);
        assert_eq!(
            keyless_status_from(Some(&stamp), guarded_now, CEILING),
            trial_ceiling_status()
        );

        // And a rollback can't mint a backdated stamp either: the mint uses
        // the guarded now, so the stamp is never below the high-water mark.
        let minted = stamped_first_attempt(None, rolled_back_wall.max(max_seen));
        assert_eq!(minted.first_attempt_at_ms, max_seen);
    }

    // ── classify_trial: the mint-outcome decision ───────────────────────────

    fn ok_outcome(key: &str) -> Result<TrialOutcome, ApiError> {
        Ok(TrialOutcome {
            created: true,
            license_key: key.to_string(),
        })
    }

    fn refused(code: RefusalCode) -> Result<TrialOutcome, ApiError> {
        Err(ApiError::Refused {
            code,
            message: "msg".to_string(),
        })
    }

    #[test]
    fn verified_key_installs_unverifiable_key_retries() {
        assert_eq!(
            classify_trial(ok_outcome("payload.sig"), |_| true),
            TrialDisposition::Install("payload.sig".to_string())
        );
        assert!(matches!(
            classify_trial(ok_outcome("forged.sig"), |_| false),
            TrialDisposition::Retry { .. }
        ));
    }

    #[test]
    fn trial_already_used_marks_used_everything_else_retries() {
        assert_eq!(
            classify_trial(refused(RefusalCode::TrialAlreadyUsed), |_| true),
            TrialDisposition::MarkUsed
        );
        // Server misconfig, rate limits, and transport never punish the user:
        // capture keeps running, the retry hooks try again.
        for result in [
            refused(RefusalCode::TrialsNotEnabled),
            refused(RefusalCode::RateLimited {
                retry_after_seconds: Some(30),
            }),
            refused(RefusalCode::Other("brand_new".into())),
            Err(ApiError::Transport("timeout".into())),
        ] {
            assert!(
                matches!(
                    classify_trial(result, |_| true),
                    TrialDisposition::Retry { .. }
                ),
                "must retry"
            );
        }
    }

    #[test]
    fn stamp_round_trips_and_tolerates_a_missing_used_field() {
        let json = serde_json::to_string(&stamp_at(42)).unwrap();
        assert_eq!(
            serde_json::from_str::<TrialIssuance>(&json).unwrap(),
            stamp_at(42)
        );
        // Forward tolerance: `used` defaults to false when absent.
        let parsed: TrialIssuance =
            serde_json::from_str(r#"{"first_attempt_at_ms":7}"#).unwrap();
        assert_eq!(parsed, stamp_at(7));
    }
}
