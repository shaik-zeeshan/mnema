//! Receipt Refresh background cadence (ADR 0055): scoped to exactly one
//! state — a paid license whose Update Window has lapsed. Every 4 hours for
//! the first 14 days after lapse (renewals cluster near lapse), then daily,
//! stopping the moment a refresh reports the window open. Expired trials and
//! Revoked licenses never schedule — a healthy, in-window activated machine
//! sends nothing, ever.
//!
//! Cadence needs no new storage: "time since lapse" derives from the
//! receipt's own `update_through` (lapse instant = window end), which the
//! computed status already carries as `update_through_ms`.

use std::time::Duration;

use capture_types::LicenseStatus;
use tauri::Manager;

use crate::app_infra::AppInfraState;

/// First 14 days after lapse: renewals cluster near lapse.
const DENSE_LAPSE_MS: i64 = 14 * 24 * 60 * 60 * 1000;
const DENSE_INTERVAL: Duration = Duration::from_secs(4 * 60 * 60);
const SPARSE_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// Local-only recheck while nothing is scheduled, so a lapse that happens
/// while the app runs (Mnema runs 24/7) starts the cadence within one dense
/// interval. Never a network call.
const IDLE_RECHECK: Duration = DENSE_INTERVAL;

/// Started once from deferred startup, beside the daily CRL timer. Consults
/// the current computed status on EVERY tick decision; a heal during the
/// sleep stops the refresh.
pub fn start_receipt_refresh_timer(app_handle: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            let Some(interval) = tick_decision(&app_handle).await else {
                tokio::time::sleep(IDLE_RECHECK).await;
                continue;
            };
            tauri_plugin_log::log::info!(target: "mnema_lib::licensing", "receipt refresh scheduled in {}h (update window lapsed)", interval.as_secs() / 3600);
            tokio::time::sleep(interval).await;
            if tick_decision(&app_handle).await.is_some() {
                tauri_plugin_log::log::info!(target: "mnema_lib::licensing", "receipt refresh tick: update window still lapsed, re-activating");
                super::activation::refresh_receipt(app_handle.clone()).await;
            } else {
                tauri_plugin_log::log::info!(target: "mnema_lib::licensing", "receipt refresh tick skipped: no longer lapsed");
            }
        }
    });
}

/// One tick decision: recompute the status (fresh, not the cache — nothing
/// else recomputes when the window lapses on a quietly running machine) and
/// pick the cadence. `None` = do not refresh.
async fn tick_decision(app_handle: &tauri::AppHandle) -> Option<Duration> {
    let infra = app_handle.try_state::<AppInfraState>()?.inner().clone();
    let now = super::now_ms();
    let status = super::compute_license_status(infra.pool(), app_handle, now).await;
    let has_receipt = app_infra::load_activation_receipt().ok().flatten().is_some();
    cadence_interval(&status, has_receipt, now)
}

/// The pure cadence decision: `None` = never schedule (healthy/in-window,
/// expired trial, revoked, or no receipt to refresh), `Some(4h)` while lapsed
/// under 14 days, `Some(24h)` after.
fn cadence_interval(status: &LicenseStatus, has_receipt: bool, now_ms: i64) -> Option<Duration> {
    let LicenseStatus::Licensed {
        update_through_ms,
        in_window,
        ..
    } = status
    else {
        return None;
    };
    if *in_window || !has_receipt {
        return None;
    }
    if now_ms.saturating_sub(*update_through_ms) < DENSE_LAPSE_MS {
        Some(DENSE_INTERVAL)
    } else {
        Some(SPARSE_INTERVAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capture_types::Activation;

    const DAY_MS: i64 = 24 * 60 * 60 * 1000;

    fn licensed(update_through_ms: i64, in_window: bool) -> LicenseStatus {
        LicenseStatus::Licensed {
            update_through_ms,
            in_window,
            email: "owner@example.com".into(),
            name: String::new(),
            activation: Activation::Activated,
        }
    }

    #[test]
    fn healthy_in_window_never_schedules() {
        assert_eq!(cadence_interval(&licensed(100 * DAY_MS, true), true, DAY_MS), None);
    }

    #[test]
    fn lapsed_under_14_days_refreshes_every_4_hours() {
        let lapse = 100 * DAY_MS;
        assert_eq!(
            cadence_interval(&licensed(lapse, false), true, lapse + 13 * DAY_MS),
            Some(DENSE_INTERVAL)
        );
    }

    #[test]
    fn lapsed_14_days_or_more_refreshes_daily() {
        let lapse = 100 * DAY_MS;
        assert_eq!(
            cadence_interval(&licensed(lapse, false), true, lapse + 14 * DAY_MS),
            Some(SPARSE_INTERVAL)
        );
        assert_eq!(
            cadence_interval(&licensed(lapse, false), true, lapse + 400 * DAY_MS),
            Some(SPARSE_INTERVAL)
        );
    }

    #[test]
    fn expired_trial_and_revoked_never_schedule() {
        for status in [
            LicenseStatus::Trial {
                days_left: 0,
                trial_end_ms: 0,
            },
            LicenseStatus::ReadOnly,
            LicenseStatus::Revoked,
            LicenseStatus::TrialNotStarted { trial_days: 30 },
        ] {
            assert_eq!(cadence_interval(&status, true, i64::MAX), None, "{status:?}");
        }
    }

    #[test]
    fn lapsed_without_a_receipt_never_schedules() {
        // Nothing to refresh — the never-activated path stays with the
        // provisional retry loop, not this cadence.
        assert_eq!(cadence_interval(&licensed(0, false), false, DAY_MS), None);
    }

    #[test]
    fn heal_stops_the_timer() {
        // Same license, window reopened after a refresh landed → no more ticks.
        let lapse = 100 * DAY_MS;
        let now = lapse + DAY_MS;
        assert!(cadence_interval(&licensed(lapse, false), true, now).is_some());
        assert_eq!(cadence_interval(&licensed(lapse + 365 * DAY_MS, true), true, now), None);
    }
}
