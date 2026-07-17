//! Anonymous CRL fetch + verbatim monotonic cache (ADR 0052, licensegate).
//!
//! One anonymous GET of the per-product CRL (`GET /v1/crl/{slug}` via the
//! licensegate client) — NO identifier, no auth header, nothing about the user
//! is ever sent. A candidate document (fetched, or the CI-baked fresh-install
//! floor) is accepted only if it verifies AND its `issued_at` is strictly newer
//! than the cache (rollback-proof); any failure keeps the cache and the license
//! stands (staleness never locks). On accept the gate recomputes so a flip to
//! `Revoked`/Read-Only is live (capture stops at the next seam and the
//! in-flight segment commits normally).

use tauri::Manager;

use crate::app_infra::AppInfraState;
use crate::licensing::adapter;
use crate::native_capture::debug_log;

/// CI-baked fresh-install floor: release CI fetches the live prod CRL
/// (`https://license.mnema.day/v1/crl/mnema`) and exports the signed wire as
/// `MNEMA_CRL_FLOOR` before the build (macos-release.yml), so a fresh install
/// starts from a real revocation list before its first fetch. Local/dev builds
/// leave it unset — no floor.
fn baked_floor_wire() -> Option<&'static str> {
    match option_env!("MNEMA_CRL_FLOOR") {
        Some(wire) if !wire.trim().is_empty() => Some(wire),
        _ => None,
    }
}

/// Fire-and-forget: seed the baked floor into the cache (same monotonic accept
/// as a fetch, so it never downgrades a newer cache), then one fetch + apply.
/// Never blocks the caller.
pub fn spawn_crl_refresh(app_handle: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        if let Some(wire) = baked_floor_wire() {
            apply_candidate(&app_handle, wire, "baked floor").await;
        }
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
    // Anonymous GET — fetch_crl sends no auth header and no identifier.
    let wire = match adapter::client().fetch_crl(adapter::product_slug()).await {
        Ok(wire) => wire,
        Err(error) => {
            debug_log::log_warn(format!("CRL fetch failed, keeping cache: {error:?}"));
            return;
        }
    };
    apply_candidate(&app_handle, &wire, "fetched").await;
}

/// Verify one candidate wire, gate it on monotonic freshness against the
/// cache, store it verbatim, and recompute the license gate so a newly-revoked
/// active key flips to Read-Only live. Any failure keeps the cache.
async fn apply_candidate(app_handle: &tauri::AppHandle, wire: &str, source: &str) {
    let Some(infra) = app_handle.try_state::<AppInfraState>() else {
        return;
    };
    let infra = infra.inner().clone();
    let pool = infra.pool();

    let Some(candidate) = crate::licensing::verify_crl_wire(wire) else {
        debug_log::log_warn(format!("{source} CRL failed verification, keeping cache"));
        return;
    };

    let cached_wire = app_infra::load_cached_crl(pool).await.ok().flatten();
    if !supersedes(
        cached_wire.as_deref(),
        &candidate,
        &crate::licensing::verify_crl_wire,
    ) {
        return;
    }

    if let Err(error) = app_infra::store_cached_crl(pool, wire).await {
        debug_log::log_warn(format!("CRL cache write failed, keeping old cache: {error}"));
        return;
    }
    debug_log::log_info(format!(
        "accepted {source} CRL: issued_at={} revoked={}",
        candidate.issued_at,
        candidate.revoked_license_ids.len()
    ));
    crate::licensing::compute_license_status(pool, app_handle, crate::licensing::now_ms()).await;
}

/// Whether a verified candidate replaces the cached wire: yes when the cache
/// is absent or unverifiable (contributes nothing — re-verified on every
/// read), otherwise only when strictly newer (`licensegate::accept`, the
/// rollback-proof monotonic gate; `issued_at` is RFC 3339 UTC, so
/// lexicographic order is chronological order).
fn supersedes(
    cached_wire: Option<&str>,
    candidate: &licensegate::Crl,
    verify: &dyn Fn(&str) -> Option<licensegate::Crl>,
) -> bool {
    let cached = cached_wire.and_then(verify);
    licensegate::accept(cached.as_ref(), candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crl(issued_at: &str) -> licensegate::Crl {
        licensegate::Crl {
            kid: "56475aa7".to_string(),
            issued_at: issued_at.to_string(),
            revoked_license_ids: vec![],
        }
    }

    #[test]
    fn accept_only_strictly_newer() {
        // Pins the monotonic gate this module relies on (rollback-proof).
        let cached = crl("2026-07-10T00:00:00Z");
        // No cache yet → accept anything.
        assert!(licensegate::accept(None, &cached));
        // Strictly newer → accept.
        assert!(licensegate::accept(
            Some(&cached),
            &crl("2026-07-11T00:00:00Z")
        ));
        // Equal or older → reject.
        assert!(!licensegate::accept(
            Some(&cached),
            &crl("2026-07-10T00:00:00Z")
        ));
        assert!(!licensegate::accept(
            Some(&cached),
            &crl("2026-07-09T00:00:00Z")
        ));
    }

    #[test]
    fn floor_seeds_fresh_installs_and_never_downgrades_a_newer_cache() {
        // The wire IS the issued_at in this fake verifier; "garbage" fails
        // verification — exactly the shapes apply_candidate feeds supersedes.
        let verify = |wire: &str| (wire != "garbage").then(|| crl(wire));

        // Fresh install: empty cache → the baked floor seeds it.
        assert!(supersedes(None, &crl("2026-07-10T00:00:00Z"), &verify));
        // A newer fetched CRL replaces the floor-seeded cache.
        assert!(supersedes(
            Some("2026-07-10T00:00:00Z"),
            &crl("2026-07-11T00:00:00Z"),
            &verify
        ));
        // The floor never overrides a newer or equal cached CRL.
        assert!(!supersedes(
            Some("2026-07-12T00:00:00Z"),
            &crl("2026-07-10T00:00:00Z"),
            &verify
        ));
        assert!(!supersedes(
            Some("2026-07-10T00:00:00Z"),
            &crl("2026-07-10T00:00:00Z"),
            &verify
        ));
        // An unverifiable cache contributes nothing — the candidate wins.
        assert!(supersedes(Some("garbage"), &crl("2026-07-10T00:00:00Z"), &verify));
    }

    #[test]
    fn local_builds_carry_no_floor() {
        // MNEMA_CRL_FLOOR is only exported by release CI; a normal build (this
        // test build included) must have no floor.
        assert_eq!(baked_floor_wire(), None);
    }
}
