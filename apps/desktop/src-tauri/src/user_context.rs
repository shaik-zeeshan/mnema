//! **User Context** on the Tauri side: the command wrappers and the two worker
//! loops. Derivation, distillation, the digest and the sweeps live in
//! `app_core::user_context`; the loops stay here until `BackgroundWorkersState`
//! and `shutdown_aware_sleep` move with `app_infra` (S2 B6).

use std::sync::Arc;

use app_core::user_context::subject_vector_worker::{
    maybe_release_embedder_on_idle_decay, run_sweep_pass, SweepPass, SweepState,
    ERROR_RETRY_INTERVAL, IDLE_POLL_INTERVAL, SWEEP_BATCH_SIZE,
};
use app_core::user_context::worker::{worker_tick, WorkerCadence};
use app_core::{AppCore, AppInfraState};
use tauri::Manager;

use crate::app_infra::{shutdown_aware_sleep, BackgroundWorkersState};

pub mod commands;

/// Spawn the background User Context derivation worker. Mirrors
/// `spawn_retention_cleanup_worker`: tracks the handle for graceful shutdown and
/// selects between a tier-paced sleep and the shutdown watch.
pub fn spawn_user_context_worker(
    infra: AppInfraState,
    app_handle: tauri::AppHandle,
    background_workers: BackgroundWorkersState,
) {
    let mut shutdown_rx = background_workers.subscribe();
    app_core::debug_log::log_info("starting user context derivation worker (idle_when_disabled)");
    let core = app_handle.state::<Arc<AppCore>>().inner().clone();
    let handle = tauri::async_runtime::spawn(async move {
        let infra = Arc::clone(&infra);
        // The slower Conclusion-distillation beat is paced relative to the
        // frequent Activity beat; this state survives across ticks.
        let mut cadence = WorkerCadence {
            last_distillation_at_ms: infra
                .user_context()
                .latest_distillation_summary()
                .await
                .ok()
                .flatten()
                .map(|(at_ms, _, _)| at_ms),
            ..WorkerCadence::default()
        };
        loop {
            if *shutdown_rx.borrow() {
                break;
            }

            // Every tick path is error-recording (no `?`, no `unwrap`), so a
            // tick returns an idle/paced sleep rather than panicking the worker.
            let next_sleep = worker_tick(&infra, &core, &mut cadence).await;

            if shutdown_aware_sleep(&mut shutdown_rx, next_sleep).await {
                break;
            }
        }
        app_core::debug_log::log_info("stopped user context derivation worker");
    });
    background_workers.track(handle);
}

/// Spawn the **Subject Vector Backfill** worker. Mirrors
/// [`crate::semantic_search_worker::spawn_semantic_index_backfill_worker`]: tracks
/// the handle for graceful shutdown and selects between an idle sleep and the
/// shutdown watch. The embedder is loaded lazily on the first pass that has work
/// AND an installed model, reused across passes, and dropped (reloaded next time)
/// on a model switch or a sustained idle so a Settings change is picked up without
/// a restart.
pub fn spawn_user_context_subject_vector_worker(
    infra: AppInfraState,
    app_handle: tauri::AppHandle,
    background_workers: BackgroundWorkersState,
) {
    let mut shutdown_rx = background_workers.subscribe();
    app_core::debug_log::log_info(format!(
        "starting user-context subject vector worker (batch={}, idle_poll_ms={}, error_retry_ms={})",
        SWEEP_BATCH_SIZE,
        IDLE_POLL_INTERVAL.as_millis(),
        ERROR_RETRY_INTERVAL.as_millis(),
    ));

    let core = app_handle.state::<Arc<AppCore>>().inner().clone();
    let handle = tauri::async_runtime::spawn(async move {
        let infra = Arc::clone(&infra);
        let mut state = SweepState::default();

        loop {
            if *shutdown_rx.borrow() {
                break;
            }

            let pass = run_sweep_pass(&infra, &core, &mut state, &mut shutdown_rx).await;
            let sleep = match pass {
                SweepPass::DidWork(cooldown) => {
                    state.consecutive_idles = 0;
                    cooldown
                }
                SweepPass::Idle => {
                    maybe_release_embedder_on_idle_decay(&mut state, "idle");
                    IDLE_POLL_INTERVAL
                }
                SweepPass::Error => {
                    // A persistent error loop is not work: decay the embedder the same
                    // way an idle stretch does, so the model weights are released after
                    // the grace period rather than pinned resident for the whole spin.
                    maybe_release_embedder_on_idle_decay(&mut state, "error");
                    ERROR_RETRY_INTERVAL
                }
                SweepPass::Shutdown => break,
            };

            if shutdown_aware_sleep(&mut shutdown_rx, sleep).await {
                break;
            }
        }

        app_core::debug_log::log_info("stopped user-context subject vector worker");
    });
    background_workers.track(handle);
}
