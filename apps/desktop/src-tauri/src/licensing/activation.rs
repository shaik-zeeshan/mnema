//! Once-per-machine activation (ADR 0053): the background attempt + retry over
//! `licensegate::Client::activate`, and the in-memory over-cap hint it feeds
//! back into the compute path. Retries piggyback the daily CRL tick.

use tauri::Manager;

use super::{adapter, ensure_first_seen, is_key_revoked, load_effective_crl, now_ms};
use crate::app_infra::AppInfraState;
use crate::licensing::ActivationHint;

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

pub(super) fn read_over_cap_hint(app_handle: &tauri::AppHandle) -> Option<(String, String)> {
    app_handle
        .try_state::<ActivationHint>()
        .and_then(|hint| hint.0.lock().ok().and_then(|slot| slot.clone()))
}

/// Spawn the background license fixer if there's work to do. Never blocks; a
/// no-op (already activated, no key and no pending trial, or revoked) just
/// returns early inside the task. Safe to call repeatedly — the daily CRL tick
/// and the capture-refusal recompute do.
pub(crate) fn maybe_spawn_activation(app_handle: &tauri::AppHandle) {
    let app_handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        // Keyless with a pending trial issuance: retry the mint first (it
        // chains activation itself). This is how issuance retries piggyback
        // the launch gate, the daily CRL tick, and refused capture starts.
        if super::trial::pending_issuance() {
            let Some(state) = app_handle.try_state::<AppInfraState>() else {
                return;
            };
            let infra = std::sync::Arc::clone(&*state);
            super::trial::ensure_trial_started(infra.pool(), &app_handle, now_ms()).await;
            return;
        }
        run_activation(app_handle).await;
    });
}

/// What to do with the activation call's outcome — pure and IO-free
/// (`run_activation` is the shell that executes it).
#[derive(Debug, PartialEq, Eq)]
enum ActivationDisposition {
    /// Success with a receipt that verifies for this license+machine: store it,
    /// clear any over-cap hint, recompute (badge flips to Activated).
    StoreReceipt(String),
    /// The server says the license itself is dead (revoked/suspended/expired).
    /// Don't grant/deny here — the offline evaluation is the gate; just
    /// recompute so status reflects whatever the CRL/key already know.
    Recompute,
    /// `device_cap_reached` with both links: surface them, recompute
    /// (→ `RefusedOverCap` while still inside the Provisional Window).
    SetOverCap { reset_url: String, buy_url: String },
    /// Anything else (unverifiable receipt, link-less cap refusal, transport,
    /// unknown refusals): change nothing — the Provisional Window keeps
    /// running and the daily tick retries. `reason` feeds the log line.
    Retry { reason: String },
}

/// Classify an activation result. `verify` is the receipt check for this
/// license+machine (injected so the accept path is testable without crypto/IO).
fn classify_activation(
    result: Result<licensegate::ActivateOutcome, licensegate::ApiError>,
    verify: impl Fn(&str) -> bool,
) -> ActivationDisposition {
    use licensegate::{ApiError, RefusalCode};
    match result {
        Ok(outcome) => {
            if verify(&outcome.receipt) {
                ActivationDisposition::StoreReceipt(outcome.receipt)
            } else {
                ActivationDisposition::Retry {
                    reason: "server receipt did not verify".to_string(),
                }
            }
        }
        Err(ApiError::Refused { code, message }) => match code {
            RefusalCode::DeviceCapReached {
                reset_url: Some(reset_url),
                buy_url: Some(buy_url),
                ..
            } => ActivationDisposition::SetOverCap { reset_url, buy_url },
            RefusalCode::DeviceCapReached { .. } => ActivationDisposition::Retry {
                reason: "device_cap_reached without reset/buy links".to_string(),
            },
            RefusalCode::LicenseRevoked
            | RefusalCode::LicenseSuspended
            | RefusalCode::LicenseExpired => ActivationDisposition::Recompute,
            other => ActivationDisposition::Retry {
                reason: format!("refused: {other:?} ({message})"),
            },
        },
        Err(ApiError::Transport(error)) => ActivationDisposition::Retry {
            reason: format!("transport: {error}"),
        },
    }
}

pub(super) async fn run_activation(app_handle: tauri::AppHandle) {
    let Some(state) = app_handle.try_state::<AppInfraState>() else {
        return;
    };
    let infra = std::sync::Arc::clone(&*state);
    let pool = infra.pool();

    // Need a stored, authentic, non-revoked key — else nothing to activate.
    let Some(verifier) = adapter::verifier() else {
        return;
    };
    let Some(key_wire) = app_infra::load_license_key().ok().flatten() else {
        return;
    };
    let Ok(key) = verifier.verify_license(&key_wire) else {
        return;
    };
    let license_id = key.license_id.clone();
    let crl = load_effective_crl(&verifier, pool).await;
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
    // The license-salted derivation (licensegate SPEC §2) — never the raw uuid.
    let machine_hash = licensegate::machine_hash(&license_id, &uuid);

    // Already activated on this machine → done.
    let already_activated = app_infra::load_activation_receipt()
        .ok()
        .flatten()
        .and_then(|wire| verifier.verify_receipt_bound(&wire, &uuid).ok())
        .is_some_and(|receipt| receipt.license_id == license_id);
    if already_activated {
        return;
    }

    // Stamp first_seen (write-once) and record the high-water mark before the
    // first network attempt.
    let now = now_ms();
    let max_seen = app_infra::read_max_timestamp_seen(pool).await.unwrap_or(now);
    ensure_first_seen(&license_id, now.max(max_seen));
    let _ = app_infra::bump_max_timestamp_seen(pool, now).await;

    let result = adapter::client()
        .activate(&license_id, &machine_hash, None)
        .await;
    let verify = |wire: &str| {
        verifier
            .verify_receipt_bound(wire, &uuid)
            .map(|receipt| receipt.license_id == license_id)
            .unwrap_or(false)
    };
    match classify_activation(result, verify) {
        ActivationDisposition::StoreReceipt(receipt_wire) => {
            if let Err(error) = app_infra::store_activation_receipt(&receipt_wire) {
                tauri_plugin_log::log::warn!(target: "mnema_lib::licensing", "activation failed: could not store receipt: {error}");
                return;
            }
            clear_over_cap_hint(&app_handle);
            // Recompute → badge flips to Activated.
            super::compute_license_status(pool, &app_handle, now_ms()).await;
        }
        ActivationDisposition::Recompute => {
            tauri_plugin_log::log::warn!(target: "mnema_lib::licensing", "activation refused: license {license_id} reported dead by the server; the offline evaluation enforces it");
            super::compute_license_status(pool, &app_handle, now_ms()).await;
        }
        ActivationDisposition::SetOverCap { reset_url, buy_url } => {
            set_over_cap_hint(&app_handle, reset_url, buy_url);
            super::compute_license_status(pool, &app_handle, now_ms()).await;
        }
        ActivationDisposition::Retry { reason } => {
            // Leave the Provisional Window running; the daily tick retries.
            tauri_plugin_log::log::warn!(target: "mnema_lib::licensing", "activation failed: {reason} (from {})", adapter::base_url());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use licensegate::{ApiError, RefusalCode};

    fn verify_ok(_: &str) -> bool {
        true
    }
    fn verify_fail(_: &str) -> bool {
        false
    }

    fn ok_outcome(receipt: &str) -> Result<licensegate::ActivateOutcome, ApiError> {
        Ok(licensegate::ActivateOutcome {
            created: true,
            receipt: receipt.to_string(),
        })
    }

    fn refused(code: RefusalCode) -> Result<licensegate::ActivateOutcome, ApiError> {
        Err(ApiError::Refused {
            code,
            message: "msg".to_string(),
        })
    }

    #[test]
    fn success_with_verifiable_receipt_stores_it() {
        assert_eq!(
            classify_activation(ok_outcome("payload.sig"), verify_ok),
            ActivationDisposition::StoreReceipt("payload.sig".to_string()),
        );
    }

    #[test]
    fn success_with_unverifiable_receipt_retries_without_storing() {
        assert!(matches!(
            classify_activation(ok_outcome("forged.sig"), verify_fail),
            ActivationDisposition::Retry { .. }
        ));
    }

    #[test]
    fn dead_license_refusals_recompute_only() {
        // The offline evaluation is the gate — a server refusal never stores
        // or denies by itself.
        for code in [
            RefusalCode::LicenseRevoked,
            RefusalCode::LicenseSuspended,
            RefusalCode::LicenseExpired,
        ] {
            assert_eq!(
                classify_activation(refused(code), verify_ok),
                ActivationDisposition::Recompute,
            );
        }
    }

    #[test]
    fn device_cap_with_links_surfaces_them_without_links_retries() {
        assert_eq!(
            classify_activation(
                refused(RefusalCode::DeviceCapReached {
                    cap: Some(3),
                    reset_url: Some("https://reset".into()),
                    buy_url: Some("https://buy".into()),
                }),
                verify_ok,
            ),
            ActivationDisposition::SetOverCap {
                reset_url: "https://reset".to_string(),
                buy_url: "https://buy".to_string(),
            },
        );
        assert!(matches!(
            classify_activation(
                refused(RefusalCode::DeviceCapReached {
                    cap: Some(3),
                    reset_url: None,
                    buy_url: None,
                }),
                verify_ok,
            ),
            ActivationDisposition::Retry { .. }
        ));
    }

    #[test]
    fn transport_and_unknown_refusals_retry_leaving_the_window_untouched() {
        assert!(matches!(
            classify_activation(Err(ApiError::Transport("timeout".into())), verify_ok),
            ActivationDisposition::Retry { .. }
        ));
        for code in [
            RefusalCode::RateLimited {
                retry_after_seconds: Some(30),
            },
            RefusalCode::Unauthorized,
            RefusalCode::LicenseNotFound,
            RefusalCode::Other("brand_new".into()),
        ] {
            assert!(
                matches!(
                    classify_activation(refused(code.clone()), verify_ok),
                    ActivationDisposition::Retry { .. }
                ),
                "{code:?} must retry"
            );
        }
    }
}
