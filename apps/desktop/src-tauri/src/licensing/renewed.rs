//! Renewal return flow: `mnema://license/renewed`, the Polar renewal
//! product's success-redirect target (ADR 0055). A renewal extends the
//! buyer's *existing* license server-side — the machine already holds the
//! key, so no claim endpoint is involved; it only needs fresh dates. The
//! order webhook may still be in flight when the buyer lands back in the
//! app, so this polls Receipt Refresh briefly until the recomputed status
//! shows the Update Window open, then stops. Giving up is silent — the
//! lapsed background cadence (`receipt_refresh`) covers the rest.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use capture_types::LicenseStatus;

/// Dedup for the poll loop: the `renewed` deep link is web-fireable at zero
/// cost, and stacked loops could rate-limit us against the server (ADR 0055).
static POLL_RUNNING: AtomicBool = AtomicBool::new(false);

/// Poll budget: the webhook usually lands within seconds; 30 × 2s ≈ 60s,
/// past which the background cadence is the path.
const RENEWED_POLL_INTERVAL: Duration = Duration::from_secs(2);
const RENEWED_POLL_ATTEMPTS: u32 = 30;

/// Pure stop decision: the renewal has landed once the recomputed status
/// shows a Licensed key with the Update Window open.
fn window_open(status: Option<&LicenseStatus>) -> bool {
    matches!(
        status,
        Some(LicenseStatus::Licensed {
            in_window: true,
            ..
        })
    )
}

/// Renewed deep-link entry point (spawned by `dispatch_deep_link` in
/// `lib.rs`). Success surfaces through the recomputed status /
/// `license_status` event — the extended window shows up in Settings with
/// nothing to paste.
pub async fn renewed_from_deep_link(app_handle: tauri::AppHandle) {
    if POLL_RUNNING.swap(true, Ordering::AcqRel) {
        tauri_plugin_log::log::info!(target: "mnema_lib::licensing", "renewed deep link: poll already running, ignoring");
        return;
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            POLL_RUNNING.store(false, Ordering::Release);
        }
    }
    let _reset = Reset;
    for attempt in 1..=RENEWED_POLL_ATTEMPTS {
        let reached = super::activation::refresh_receipt(app_handle.clone()).await;
        if window_open(super::cached_status(&app_handle).as_ref()) {
            tauri_plugin_log::log::info!(target: "mnema_lib::licensing", "renewed deep link: update window open after {attempt} refresh attempt(s)");
            return;
        }
        tauri_plugin_log::log::info!(target: "mnema_lib::licensing", "renewed deep link: attempt {attempt}/{RENEWED_POLL_ATTEMPTS}, window still closed (server reached: {reached})");
        if attempt < RENEWED_POLL_ATTEMPTS {
            tokio::time::sleep(RENEWED_POLL_INTERVAL).await;
        }
    }
    tauri_plugin_log::log::info!(target: "mnema_lib::licensing", "renewed deep link: window still closed after {RENEWED_POLL_ATTEMPTS} attempts; the background cadence takes over");
}

#[cfg(test)]
mod tests {
    use super::*;
    use capture_types::Activation;

    #[test]
    fn poll_stops_only_on_an_open_update_window() {
        let licensed = |in_window| LicenseStatus::Licensed {
            update_through_ms: 0,
            in_window,
            email: "owner@example.com".into(),
            name: String::new(),
            activation: Activation::Activated,
        };
        assert!(window_open(Some(&licensed(true))));
        // Still lapsed, non-licensed, or no computed status yet → keep polling.
        assert!(!window_open(Some(&licensed(false))));
        assert!(!window_open(Some(&LicenseStatus::ReadOnly)));
        assert!(!window_open(None));
    }
}
