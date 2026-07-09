//! Anonymous CRL fetch + verbatim monotonic cache (ADR 0052, slice 4).
//!
//! One GET of a public static file — NO identifier, and no header about the
//! user, is ever sent. A fetched document is accepted only if it verifies AND
//! its `issued_at` is strictly newer than the cache (rollback-proof); any
//! failure keeps the cache and the license stands (staleness never locks).
//! On accept the gate recomputes so a flip to `Revoked`/Read-Only is live
//! (`publish` updates the cache, emits `license_status`, and refreshes the tray;
//! capture stops at the next seam and the in-flight segment commits normally).

use tauri::Manager;

use crate::app_infra::AppInfraState;
use crate::native_capture::debug_log;

/// Fallback revocation-list URL, used only when the build-time `MNEMA_CRL_URL`
/// is unset. Currently a dev `workers.dev` deploy — fine for development, but
/// NOT for a public release (a `workers.dev` host baked into a binary can't be
/// repointed once abandoned). Release CI sets `MNEMA_CRL_URL` to a seller-owned
/// custom domain in front of the worker (e.g. `crl.mnema.app`); do that before
/// shipping to real users.
const DEFAULT_CRL_URL: &str =
    "https://mnema-fulfillment.shaikzeeshan999.workers.dev/revocations.json";

/// The URL to fetch, most- to least-specific:
/// 1. `MNEMA_DEV_CRL_URL` at runtime (debug builds only) — simulate a revoked
///    key locally without a rebuild.
/// 2. `MNEMA_CRL_URL` baked in at build time — set by release CI to the
///    production domain (`build.rs` re-runs when it changes).
/// 3. [`DEFAULT_CRL_URL`] — the dev fallback.
fn crl_url() -> String {
    #[cfg(debug_assertions)]
    if let Ok(url) = std::env::var("MNEMA_DEV_CRL_URL") {
        if !url.trim().is_empty() {
            return url;
        }
    }
    match option_env!("MNEMA_CRL_URL") {
        Some(url) if !url.trim().is_empty() => url.to_string(),
        _ => DEFAULT_CRL_URL.to_string(),
    }
}

/// Accept a fetched CRL iff its `issued_at` is strictly newer than the cached
/// one. The baked floor is a floor, not a rollback target — compare against the
/// cache specifically, so a fresh install's baked snapshot can't block a
/// legitimately newer fetched list (and vice versa).
fn should_accept(new_issued_at: i64, current_issued_at: Option<i64>) -> bool {
    match current_issued_at {
        Some(current) => new_issued_at > current,
        None => true,
    }
}

/// Fire-and-forget one anonymous CRL fetch + apply. Never blocks the caller.
pub fn spawn_crl_refresh(app_handle: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        refresh_once(app_handle).await;
    });
}

/// Refresh every 24h. Mnema runs 24/7, so launch-only checks could lag weeks.
pub fn start_daily_crl_timer(app_handle: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(24 * 60 * 60));
        // The first tick fires immediately; startup already did an initial
        // fetch, so consume it and only act on subsequent daily ticks.
        interval.tick().await;
        loop {
            interval.tick().await;
            refresh_once(app_handle.clone()).await;
            // Piggyback the daily tick to retry once-per-machine activation
            // (no-op once a receipt is stored, or when there's nothing to activate).
            crate::licensing::maybe_spawn_activation(&app_handle);
        }
    });
}

async fn refresh_once(app_handle: tauri::AppHandle) {
    let Some(infra) = app_handle.try_state::<AppInfraState>() else {
        return;
    };
    let infra = infra.inner().clone();
    let pool = infra.pool();

    // Anonymous GET — nothing about the user is sent.
    let body = match reqwest::get(crl_url()).await {
        Ok(response) => match response.text().await {
            Ok(body) => body,
            Err(error) => {
                debug_log::log_warn(format!("CRL fetch read failed, keeping cache: {error}"));
                return;
            }
        },
        Err(error) => {
            debug_log::log_warn(format!("CRL fetch failed, keeping cache: {error}"));
            return;
        }
    };

    let fetched = match app_infra::parse_and_verify_crl(&body) {
        Ok(crl) => crl,
        Err(error) => {
            debug_log::log_warn(format!("CRL verify failed, keeping cache: {error}"));
            return;
        }
    };

    // Freshness is decided against the cache only (re-verified on read).
    let current_issued_at = app_infra::load_cached_crl(pool)
        .await
        .ok()
        .flatten()
        .and_then(|wire| app_infra::parse_and_verify_crl(&wire).ok())
        .map(|crl| crl.issued_at);

    if !should_accept(fetched.issued_at, current_issued_at) {
        return;
    }

    if let Err(error) = app_infra::store_cached_crl(pool, &body).await {
        debug_log::log_warn(format!("CRL cache write failed, keeping old cache: {error}"));
        return;
    }
    debug_log::log_info(format!(
        "accepted CRL update: issued_at={} revoked={}",
        fetched.issued_at,
        fetched.revoked_license_ids.len()
    ));
    // Recompute so a newly-revoked active key flips to Read-Only live.
    crate::licensing::compute_license_status(pool, &app_handle, crate::licensing::now_ms()).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_accept_only_strictly_newer() {
        // No cache yet → accept anything.
        assert!(should_accept(100, None));
        // Strictly newer → accept.
        assert!(should_accept(200, Some(100)));
        // Equal or older → reject (rollback-proof).
        assert!(!should_accept(100, Some(100)));
        assert!(!should_accept(50, Some(100)));
    }
}
