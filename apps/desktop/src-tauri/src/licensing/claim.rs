//! Purchase claim flow: `mnema://license/claim?checkout_id=…`, Polar's
//! success-redirect target. The order webhook may still be in flight when the
//! buyer lands back in the app, so this polls `POST /v1/keys/claim` briefly
//! and installs the delivered key with zero paste — the same verify → store →
//! stamp → chain-activation path as a pasted key. Giving up is never a dead
//! end: every purchase also emails the key (the durable record, 30-day claim
//! window notwithstanding), so the fallback is a "check your email" note.

use std::time::Duration;

use tauri::Manager;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

use super::adapter;
use crate::app_infra::AppInfraState;

/// Poll budget: the Polar webhook usually lands within seconds; 15 × 2s ≈ 30s
/// covers the lag without holding a task open toward the 30-day claim window —
/// past the budget, email is the path.
const CLAIM_POLL_INTERVAL: Duration = Duration::from_secs(2);
const CLAIM_POLL_ATTEMPTS: u32 = 15;

/// What to do with one claim-poll outcome — pure and IO-free
/// (`claim_from_deep_link` is the shell that executes it).
#[derive(Debug, PartialEq, Eq)]
enum ClaimDisposition {
    /// Key delivered: install it (verify → store → stamp → chain activation).
    Install(String),
    /// `pending` (webhook lag — indistinguishable from an unknown/expired ref
    /// by design), an unknown future status, or transport trouble: poll again
    /// until the budget runs out. `reason` feeds the log line.
    KeepPolling { reason: String },
    /// A server refusal (unauthorized, invalid request, rate limited): another
    /// poll inside the budget won't change it — fall back to email now.
    GiveUp { reason: String },
}

fn classify_claim(
    result: Result<licensegate::api::ClaimOutcome, licensegate::ApiError>,
) -> ClaimDisposition {
    use licensegate::api::ClaimOutcome;
    use licensegate::ApiError;
    match result {
        Ok(ClaimOutcome::Ready { key, .. }) => ClaimDisposition::Install(key),
        Ok(ClaimOutcome::Pending) => ClaimDisposition::KeepPolling {
            reason: "pending (webhook may be in flight)".to_string(),
        },
        Ok(ClaimOutcome::Other) => ClaimDisposition::KeepPolling {
            reason: "unknown claim status from a newer server".to_string(),
        },
        Err(ApiError::Transport(error)) => ClaimDisposition::KeepPolling {
            reason: format!("transport: {error}"),
        },
        Err(ApiError::Refused { code, message }) => ClaimDisposition::GiveUp {
            reason: format!("refused: {code:?} ({message})"),
        },
    }
}

/// Claim deep-link entry point (spawned by `dispatch_deep_link` in `lib.rs`).
/// Success surfaces through the recomputed status / `license_status` event —
/// the UI flips to Licensed with no paste; the fallback is the email dialog.
pub async fn claim_from_deep_link(app_handle: tauri::AppHandle, checkout_id: String) {
    let Some(state) = app_handle.try_state::<AppInfraState>() else {
        return;
    };
    let infra = std::sync::Arc::clone(&*state);
    let pool = infra.pool();

    for attempt in 1..=CLAIM_POLL_ATTEMPTS {
        match classify_claim(adapter::client().claim(&checkout_id).await) {
            ClaimDisposition::Install(key) => {
                // Deep-link overwrite guard: a claimed key that would replace
                // a different, healthy license asks first (CONTEXT.md
                // 2026-07-18). Declining is a deliberate stop, not the email
                // fallback.
                if super::deep_link_replacement_needs_confirm(&app_handle, &key)
                    && !super::confirm_license_replacement(&app_handle).await
                {
                    tauri_plugin_log::log::info!(target: "mnema_lib::licensing", "claim license replacement declined by the user");
                    return;
                }
                // Same chain as a paste: verify+vet → store → first_seen stamp
                // → recompute → background activation (non-macOS installs the
                // key too; activation there is a no-op that maps to Activated).
                match super::install_license_key(pool, &app_handle, &key).await {
                    Ok(_) => {
                        tauri_plugin_log::log::info!(target: "mnema_lib::licensing", "claim delivered the license key; installed and chaining activation");
                        return;
                    }
                    Err(error) => {
                        // The server minted a key this build rejects (revoked,
                        // or product/config mismatch): retrying won't help.
                        tauri_plugin_log::log::warn!(target: "mnema_lib::licensing", "claimed key could not be installed: {error}");
                        break;
                    }
                }
            }
            ClaimDisposition::KeepPolling { reason } => {
                tauri_plugin_log::log::info!(target: "mnema_lib::licensing", "claim attempt {attempt}/{CLAIM_POLL_ATTEMPTS}: {reason}");
            }
            ClaimDisposition::GiveUp { reason } => {
                tauri_plugin_log::log::warn!(target: "mnema_lib::licensing", "claim gave up: {reason} (from {})", adapter::base_url());
                break;
            }
        }
        if attempt < CLAIM_POLL_ATTEMPTS {
            tokio::time::sleep(CLAIM_POLL_INTERVAL).await;
        }
    }
    email_fallback_dialog(&app_handle);
}

/// The durable fallback: every purchase emails the key, so a claim that never
/// resolves inside the poll budget is a "check your email" note, not an error.
fn email_fallback_dialog(app_handle: &tauri::AppHandle) {
    app_handle
        .dialog()
        .message(
            "Thanks for your purchase! We couldn't fetch your license key automatically yet — \
             it's on its way to your email. Paste it in Settings → License when it arrives.",
        )
        .kind(MessageDialogKind::Info)
        .title("Check your email for the key")
        .show(|_| {});
}

#[cfg(test)]
mod tests {
    use super::*;
    use licensegate::api::ClaimOutcome;
    use licensegate::{ApiError, RefusalCode};

    #[test]
    fn ready_installs_the_delivered_key() {
        assert_eq!(
            classify_claim(Ok(ClaimOutcome::Ready {
                key: "MNEMA-payload.sig".into(),
                product: "Mnema".into(),
            })),
            ClaimDisposition::Install("MNEMA-payload.sig".to_string())
        );
    }

    #[test]
    fn pending_unknown_status_and_transport_keep_polling() {
        // Pending covers webhook lag AND unknown/expired refs by design —
        // never terminal inside the budget; a future status or a network blip
        // isn't either.
        for result in [
            Ok(ClaimOutcome::Pending),
            Ok(ClaimOutcome::Other),
            Err(ApiError::Transport("timeout".into())),
        ] {
            assert!(
                matches!(
                    classify_claim(result),
                    ClaimDisposition::KeepPolling { .. }
                ),
                "must keep polling"
            );
        }
    }

    #[test]
    fn refusals_give_up_to_the_email_fallback() {
        // A refusal won't change within the 30s budget — the email (which
        // always arrives) is the fallback, not more polling.
        for code in [
            RefusalCode::Unauthorized,
            RefusalCode::InvalidRequest,
            RefusalCode::RateLimited {
                retry_after_seconds: Some(60),
            },
            RefusalCode::Other("brand_new".into()),
        ] {
            assert!(
                matches!(
                    classify_claim(Err(ApiError::Refused {
                        code,
                        message: "msg".into(),
                    })),
                    ClaimDisposition::GiveUp { .. }
                ),
                "must give up"
            );
        }
    }
}
