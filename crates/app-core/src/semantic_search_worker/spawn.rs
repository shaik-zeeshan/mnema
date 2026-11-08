//! Spawns the **Semantic Index Backfill** worker; the sweep lives in the parent
//! module.

use std::sync::Arc;

use super::{
    advance_idle_drop, maybe_release_embedder_on_idle_decay, run_sweep_pass, SweepPass, SweepState,
    ERROR_RETRY_INTERVAL, IDLE_POLL_INTERVAL, SWEEP_BATCH_SIZE,
};
use crate::{AppCore, AppInfraState};

use crate::app_infra::{shutdown_aware_sleep, BackgroundWorkersState};

/// Spawn the **Semantic Index Backfill** worker. Mirrors
/// `spawn_user_context_worker`: tracks the handle for graceful shutdown and
/// selects between an idle sleep and the shutdown watch. The embedder is loaded
/// lazily on the first pass that has work AND an installed model, and is reused
/// across passes; it is dropped (and reloaded next time) if the selection becomes
/// unavailable, so a model switch from Settings is picked up without a restart.
pub fn spawn_semantic_index_backfill_worker(
    infra: AppInfraState,
    core: &Arc<AppCore>,
    background_workers: BackgroundWorkersState,
) {
    let mut shutdown_rx = background_workers.subscribe();
    crate::debug_log::log_info(format!(
        "starting semantic index backfill worker (batch={}, idle_poll_ms={}, error_retry_ms={})",
        SWEEP_BATCH_SIZE,
        IDLE_POLL_INTERVAL.as_millis(),
        ERROR_RETRY_INTERVAL.as_millis(),
    ));

    // The debug surface's window onto this worker (`get_semantic_index_status`):
    // the sweep publishes a snapshot of its otherwise task-local health here after
    // every pass. Write-only from the worker's side — nothing here feeds back into
    // the sweep.
    let health: crate::debug_status::SemanticWorkerHealthState = core
        .state::<crate::debug_status::SemanticWorkerHealthState>()
        .as_ref()
        .clone();

    let handle = core.spawn({
        let core = Arc::clone(core);
        async move {
            let infra = Arc::clone(&infra);
            // In-memory worker state reused across passes: the loaded embedder plus the
            // bounded-retry quarantine counters (L3/CT3). A `LoadedEmbedder` remembers
            // which model it is, so a Settings model switch reloads it.
            let mut state = SweepState::new();

            loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                let pass = run_sweep_pass(&infra, &core, &mut state, &mut shutdown_rx).await;
                let sleep = match pass {
                    SweepPass::DidWork(cooldown) => {
                        // Work happened: reset the idle streak so the embedder stays warm
                        // for the rest of the drain (work never releases — returns false).
                        advance_idle_drop(
                            &mut state.consecutive_idles,
                            true,
                            state.embedder.is_some(),
                        );
                        // Drain the rest of the backlog, but pace it: a work-time-scaled
                        // inter-batch cooldown (the CPU-pacing gate) keeps the sweep from
                        // a back-to-back multi-core burn. The shutdown watch is still
                        // polled across the cooldown so a quit mid-backfill is honored.
                        cooldown
                    }
                    SweepPass::Idle => {
                        // Idle-drop ("decay to idle"): once the backlog has stayed drained
                        // for a grace period, release the embedder so the model weights are
                        // returned to the OS instead of pinning the floor while caught up.
                        // Fires (and logs) exactly once per idle stretch; with the `ort` CPU
                        // arena now disabled the session drop actually reclaims the memory,
                        // and the next anchor pays one reload (`embedder_matches(&None, ..)`
                        // is false). See [`advance_idle_drop`].
                        maybe_release_embedder_on_idle_decay(&mut state, "idle");
                        IDLE_POLL_INTERVAL
                    }
                    SweepPass::Error => {
                        // F3: a persistent error loop (e.g. a poison window the worker keeps
                        // retrying, or a DB hiccup) is NOT work — advance the same idle-drop
                        // decay so the embedder (large model weights) is released after the
                        // grace period rather than staying pinned resident for the whole spin.
                        // Before this fix the Error arm never touched `consecutive_idles`, so
                        // a 30s error loop held the weights forever. The per-anchor isolation
                        // (F3) is what eventually quarantines a poison cluster so the loop
                        // exits; until then the weights no longer have to stay resident.
                        maybe_release_embedder_on_idle_decay(&mut state, "error");
                        ERROR_RETRY_INTERVAL
                    }
                    SweepPass::Shutdown => break,
                };

                // Publish this pass's health for the debug surface. After the arms above,
                // so `model_loaded` reflects an idle-decay drop rather than lagging a pass.
                // The guard is dropped before the sleep below (never held across an await).
                *health.lock().unwrap_or_else(|poison| poison.into_inner()) =
                    state.health_snapshot();

                if shutdown_aware_sleep(&mut shutdown_rx, sleep).await {
                    break;
                }
            }

            crate::debug_log::log_info("stopped semantic index backfill worker");
        }
    });
    background_workers.track(handle);
}
