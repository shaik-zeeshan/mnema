//! The **Semantic Index Backfill** worker (issue #123): a single self-healing
//! sweep-loop on the `mnema-deferred-startup` seam that derives a **Semantic
//! Search Vector** for every `direct` **Search Result Anchor** that has
//! searchable text but no vector yet.
//!
//! One query covers everything (ADR 0036): live capture and historical backfill
//! drain through the same `anchors_missing_vector` select, ordered newest-first
//! so fresh capture preempts the backlog. Progress lives entirely in the DB (the
//! presence/absence of a `vec0` row), so the sweep is resumable across restarts
//! with no in-memory cursor — a restart mid-backfill continues from DB state, and
//! a reprocessed anchor (delete + reinsert with a new id, old vector dropped by
//! the slice-1 `AFTER DELETE` trigger) reappears automatically.
//!
//! Like local transcription/OCR, the feature is **default-on but model-gated**:
//! with no **Semantic Search Model** installed the worker is a silent no-op
//! (logged once at INFO, never an error, never blocking capture, never
//! auto-downloading). It mirrors `spawn_user_context_worker` /
//! `spawn_retention_cleanup_worker`: one `tauri::async_runtime::spawn` loop,
//! tracked for graceful shutdown, that selects between an idle sleep and the
//! shutdown watch.
//!
//! CPU placement: the fastembed/ort embed is blocking model work, so it runs on a
//! blocking thread (never the tokio reactor, never the capture hot path). DB
//! reads/writes stay on the async loop.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use capture_types::{default_semantic_search_settings, SemanticSearchSettings};
use futures_util::{
    future::{select, Either},
    pin_mut,
};
use semantic_search::{
    detect_model_status, fastembed_output_key, fastembed_pooling, model_install_dir,
    resolve_descriptor, semantic_search_models_dir, SemanticSearchEmbedder,
    SemanticSearchModelDescriptor,
};
use tauri::{Emitter, Manager};
use tokio::sync::watch;

use crate::app_infra::{shutdown_aware_sleep, AppInfraState, BackgroundWorkersState};
use crate::semantic_search_models::{
    SemanticSearchModelDownloadProgressDto, SemanticSearchModelDownloadStatusDto,
    SEMANTIC_SEARCH_MODEL_DOWNLOAD_PROGRESS_EVENT,
};

/// How many anchors to drain per batch. A bounded batch keeps the worker
/// responsive to shutdown between batches and caps the blocking-thread hop cost,
/// while still amortizing the per-batch DB round-trips.
const SWEEP_BATCH_SIZE: i64 = 16;

/// Idle poll interval when there is nothing to embed (caught up, or the model is
/// not installed). Kept modest so the worker notices freshly captured anchors and
/// a just-installed model promptly, but it does no work on these ticks.
const IDLE_POLL_INTERVAL: Duration = Duration::from_secs(20);

/// Backoff after a batch error (a DB hiccup or an embed failure). Embedding never
/// blocks capture, so a failure just retries later rather than surfacing.
const ERROR_RETRY_INTERVAL: Duration = Duration::from_secs(30);

/// Maximum **consecutive** deterministic embed failures for a single
/// **Search Result Anchor** before it is quarantined (L3). Mirrors the
/// processing-job queue / User Context `RetryPolicy.max_attempts = 3`: three
/// genuine embed attempts on the same anchor id, then the anchor is left out of
/// the sweep so a deterministically-failing "poison pill" stops driving a 30s
/// error loop forever.
///
/// Quarantine is **per-anchor-id and in-memory** by deliberate design (no
/// migration): progress for this worker is already structurally DB-resident
/// (a stored vector removes the anchor from `anchors_missing_vector`), and the
/// repo's "retry only on reprocess" convention falls out for free — a reprocess
/// deletes + reinserts the search projection with a **new** `search_documents.id`
/// (the slice-1 `AFTER DELETE` trigger drops the old vector), so the new id is
/// simply absent from the quarantine map and is retried. A restart also clears
/// it (a fresh attempt after the embedder/runtime may have changed), which is the
/// desired liveness. A persistent column would have to live on `search_documents`
/// (the anchor id is ephemeral across reprocess), which neither fits the vec0
/// `{rowid, embedding}` store nor the reprocess-resets-identity semantics — so an
/// in-memory counter is the closer match to the existing convention here.
const MAX_CONSECUTIVE_ANCHOR_FAILURES: u32 = 3;

/// Maximum **consecutive** embedder LOAD failures before the model is treated as
/// corrupt/unavailable and the worker stops the tight 30s load-retry loop (CT3).
/// Reuses L3's bounded-retry convention. A model marker says "installed" on
/// presence alone — it never validates that the ONNX graph actually loads — so a
/// truncated/bit-rotted graph would otherwise fail `load_embedder` every 30s
/// forever. After this many consecutive load failures the worker surfaces a
/// "model appears corrupt — reinstall" signal on the model-status telemetry
/// channel and idles instead of hammering the doomed load.
const MAX_CONSECUTIVE_LOAD_FAILURES: u32 = 3;

/// CPU pacing (the "Backfill CPU pacing" cross-cutting concern): a minimum
/// inter-batch cooldown so a large historical backfill does not sustain a
/// multi-core burn back-to-back concurrent with active OCR/transcription. This
/// mirrors the *shape* of OCR's Execution Budget governor
/// (`ocr_budget::cooldown_duration`) — a cooldown scaled off the just-finished
/// batch's wall time and clamped to a [min, max] band — rather than the old
/// 0ms `SweepPass::DidWork` yield. The OCR governor lives in the desktop layer
/// and is OCR-specific (keyed on `recording_active`, persisted budget state); it
/// is not importable wholesale, so the same clamp-scaled-by-work-time pattern is
/// replicated here at backfill granularity. See the report for the exact
/// vs-OCR delta.
const BACKFILL_BATCH_COOLDOWN_MULTIPLIER: f64 = 1.0;
/// Lower bound of the inter-batch cooldown — a real yield even for a trivially
/// fast batch (the old 0ms gave none), so the sweep never busy-loops the cores.
const BACKFILL_BATCH_COOLDOWN_MIN: Duration = Duration::from_millis(150);
/// Upper bound of the inter-batch cooldown, so a slow batch still drains the
/// backlog in reasonable time rather than stalling.
const BACKFILL_BATCH_COOLDOWN_MAX: Duration = Duration::from_millis(2000);

/// The outcome of one sweep pass, deciding the loop's next sleep.
enum SweepPass {
    /// At least one anchor was embedded + stored this pass. The carried `Duration`
    /// is the CPU-pacing cooldown before the next batch — work-time-scaled and
    /// clamped (the "Backfill CPU pacing" gate), replacing the old 0ms yield so a
    /// large backfill does not sustain a back-to-back multi-core burn.
    DidWork(Duration),
    /// No anchors needed a vector (caught up) OR the model is not installed
    /// (silent no-op): sleep the idle interval.
    Idle,
    /// A recoverable error this pass: sleep the error-retry interval.
    Error,
    /// Shutdown was observed mid-pass (e.g. while a blocking embed batch was in
    /// flight, CT2): stop the loop now rather than waiting on in-flight work.
    Shutdown,
}

/// Mutable, in-memory worker state that outlives a single pass: the loaded
/// embedder plus the bounded-retry quarantine counters (L3 / CT3). All of it is
/// deliberately non-persistent — see [`MAX_CONSECUTIVE_ANCHOR_FAILURES`].
struct SweepState {
    /// The loaded **Semantic Search Model**, reused across passes. `None` until
    /// the first pass that needs it with an installed model.
    embedder: Option<LoadedEmbedder>,
    /// Log the "no model installed" no-op only once per inert stretch.
    logged_no_model: bool,
    /// Per-anchor **consecutive** deterministic-embed-failure counts (L3). Keyed
    /// by `search_documents.id`. An anchor at or above
    /// [`MAX_CONSECUTIVE_ANCHOR_FAILURES`] is quarantined: excluded from the batch
    /// until its id changes (reprocess) or the worker restarts. A successful store
    /// or a non-deterministic skip clears the entry.
    anchor_failures: HashMap<i64, u32>,
    /// Consecutive embedder LOAD failures (CT3). Reset to 0 on a successful load.
    /// At [`MAX_CONSECUTIVE_LOAD_FAILURES`] the model is treated as corrupt: the
    /// worker surfaces a reinstall signal once and idles instead of load-looping.
    consecutive_load_failures: u32,
    /// Set once a corrupt-model signal has been surfaced for the current selection
    /// so the worker idles quietly rather than re-emitting every tick. Cleared
    /// when the selection changes or a load later succeeds.
    corrupt_model_signalled: bool,
}

impl SweepState {
    fn new() -> Self {
        Self {
            embedder: None,
            logged_no_model: false,
            anchor_failures: HashMap::new(),
            consecutive_load_failures: 0,
            corrupt_model_signalled: false,
        }
    }

    /// Whether `anchor_id` is quarantined: it has already failed
    /// [`MAX_CONSECUTIVE_ANCHOR_FAILURES`] times in a row and must be excluded
    /// from the batch until its id changes (reprocess) or the worker restarts.
    fn is_anchor_quarantined(&self, anchor_id: i64) -> bool {
        self.anchor_failures
            .get(&anchor_id)
            .is_some_and(|&failures| failures >= MAX_CONSECUTIVE_ANCHOR_FAILURES)
    }

    /// Record one deterministic embed failure for `anchor_id`, returning the new
    /// consecutive-failure count and whether it has now reached the quarantine cap.
    fn record_anchor_embed_failure(&mut self, anchor_id: i64) -> (u32, bool) {
        let failures = self
            .anchor_failures
            .entry(anchor_id)
            .and_modify(|count| *count = count.saturating_add(1))
            .or_insert(1);
        (*failures, *failures >= MAX_CONSECUTIVE_ANCHOR_FAILURES)
    }

    /// Clear any failure streak for `anchor_id` (a clean store, or the anchor was
    /// deleted/reprocessed so its id is retired).
    fn clear_anchor_failures(&mut self, anchor_id: i64) {
        self.anchor_failures.remove(&anchor_id);
    }
}

/// Spawn the **Semantic Index Backfill** worker. Mirrors
/// `spawn_user_context_worker`: tracks the handle for graceful shutdown and
/// selects between an idle sleep and the shutdown watch. The embedder is loaded
/// lazily on the first pass that has work AND an installed model, and is reused
/// across passes; it is dropped (and reloaded next time) if the selection becomes
/// unavailable, so a model switch from Settings is picked up without a restart.
pub fn spawn_semantic_index_backfill_worker(
    infra: AppInfraState,
    app_handle: tauri::AppHandle,
    background_workers: BackgroundWorkersState,
) {
    let mut shutdown_rx = background_workers.subscribe();
    crate::native_capture::debug_log::log_info(format!(
        "starting semantic index backfill worker (batch={}, idle_poll_ms={}, error_retry_ms={})",
        SWEEP_BATCH_SIZE,
        IDLE_POLL_INTERVAL.as_millis(),
        ERROR_RETRY_INTERVAL.as_millis(),
    ));

    let handle = tauri::async_runtime::spawn(async move {
        let infra = Arc::clone(&infra);
        // In-memory worker state reused across passes: the loaded embedder plus the
        // bounded-retry quarantine counters (L3/CT3). A `LoadedEmbedder` remembers
        // which model it is, so a Settings model switch reloads it.
        let mut state = SweepState::new();

        loop {
            if *shutdown_rx.borrow() {
                break;
            }

            let pass = run_sweep_pass(&infra, &app_handle, &mut state, &mut shutdown_rx).await;
            let sleep = match pass {
                SweepPass::DidWork(cooldown) => {
                    // Drain the rest of the backlog, but pace it: a work-time-scaled
                    // inter-batch cooldown (the CPU-pacing gate) keeps the sweep from
                    // a back-to-back multi-core burn. The shutdown watch is still
                    // polled across the cooldown so a quit mid-backfill is honored.
                    cooldown
                }
                SweepPass::Idle => IDLE_POLL_INTERVAL,
                SweepPass::Error => ERROR_RETRY_INTERVAL,
                SweepPass::Shutdown => break,
            };

            if shutdown_aware_sleep(&mut shutdown_rx, sleep).await {
                break;
            }
        }

        crate::native_capture::debug_log::log_info("stopped semantic index backfill worker");
    });
    background_workers.track(handle);
}

/// A loaded **Semantic Search Model**, tagged with the provider/model id it was
/// loaded for so a Settings model switch triggers a reload. Shared with the query
/// path (`semantic_search_query.rs`), which loads the same model to embed the
/// query string for **Hybrid Search**.
pub(crate) struct LoadedEmbedder {
    pub(crate) provider: String,
    pub(crate) model_id: String,
    pub(crate) embedder: SemanticSearchEmbedder,
}

/// Run one sweep pass: gate on the installed model, drain up to one batch of
/// anchors newest-first (skipping quarantined poison-pills), embed each on a
/// blocking thread, and store the vectors. Never panics; any error is logged and
/// turned into [`SweepPass::Error`].
async fn run_sweep_pass(
    infra: &AppInfraState,
    app_handle: &tauri::AppHandle,
    state: &mut SweepState,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> SweepPass {
    // The user's selected **Semantic Search Model Tier** (Settings slice #125):
    // the worker reloads the embedder when the provider/model id changes
    // (`embedder_matches`), so a model switch from Settings is picked up live with
    // no restart. A model-tier change to a non-768 dim is preceded by the Settings
    // re-index, which clears the vec0 table; the worker then re-derives every
    // anchor under the new model.
    let settings = effective_semantic_search_settings(app_handle);

    let app_data_dir = match app_handle.path().app_data_dir() {
        Ok(dir) => dir,
        Err(error) => {
            crate::native_capture::debug_log::log_error(format!(
                "semantic index backfill could not resolve app data dir: {error}"
            ));
            return SweepPass::Error;
        }
    };

    // Model-gating: silent no-op when no model is installed (matches the
    // transcription backfill skip). `Err` only on a corrupt marker — treat it as
    // unavailable, never a capture-blocking failure.
    let available = selected_model_available(&app_data_dir, &settings);
    if !available {
        // Drop any previously-loaded embedder so a model that was uninstalled (or a
        // selection turned off) stops holding the session, and the next install
        // reloads cleanly. Reset the CT3 corrupt-model latch and load counter so a
        // fresh (re)install gets a clean set of load attempts.
        state.embedder = None;
        state.consecutive_load_failures = 0;
        state.corrupt_model_signalled = false;
        if !state.logged_no_model {
            crate::native_capture::debug_log::log_info(
                "semantic index backfill skipped: no Semantic Search Model installed (silent no-op)",
            );
            state.logged_no_model = true;
        }
        return SweepPass::Idle;
    }
    state.logged_no_model = false;

    // CT3: if the selected model has already been signalled corrupt this stretch
    // (N consecutive load failures), idle quietly rather than re-attempting the
    // doomed load every tick. The latch is cleared when the selection changes or
    // the model is reinstalled (the `!available` branch above) so a reinstall is
    // retried cleanly.
    if state.corrupt_model_signalled {
        return SweepPass::Idle;
    }

    // Peek the backlog before paying for an embedder load: if nothing needs a
    // vector we idle without touching the model.
    let raw_batch = match infra.semantic_search().anchors_missing_vector(SWEEP_BATCH_SIZE).await {
        Ok(batch) => batch,
        Err(error) => {
            crate::native_capture::debug_log::log_error(format!(
                "semantic index backfill failed to read anchors missing a vector: {error}"
            ));
            return SweepPass::Error;
        }
    };
    if raw_batch.is_empty() {
        // Caught up: also forget any quarantine entries that no longer name a live
        // missing anchor, so the map cannot grow unbounded over a long uptime.
        state.anchor_failures.clear();
        return SweepPass::Idle;
    }

    // L3 quarantine: drop anchors that have already failed deterministically
    // `MAX_CONSECUTIVE_ANCHOR_FAILURES` times in a row. They are excluded from the
    // batch (so a poison-pill never re-drives the 30s error loop), and the backlog
    // drains *around* them. A reprocess gives the anchor a new id (absent from the
    // map) so it is retried; a restart clears the map entirely.
    let batch: Vec<_> = raw_batch
        .into_iter()
        .filter(|anchor| !state.is_anchor_quarantined(anchor.anchor_id))
        .collect();
    if batch.is_empty() {
        // Every anchor in the peeked window is quarantined. Idle (not Error): there
        // is nothing to retry until a reprocess or restart frees one.
        return SweepPass::Idle;
    }

    // Resolve the catalog descriptor (dimension/window/pooling + install path) for
    // the selected model, then load the embedder if not already loaded for it.
    let Some(descriptor) = resolve_selected_descriptor(&settings) else {
        // Availability said yes but the descriptor vanished — defensive; treat as
        // unavailable for this pass.
        state.embedder = None;
        return SweepPass::Idle;
    };
    if !embedder_matches(&state.embedder, &descriptor) {
        match load_embedder(&app_data_dir, &descriptor) {
            Ok(loaded) => {
                state.embedder = Some(loaded);
                // A successful load proves the graph is not corrupt: reset CT3.
                state.consecutive_load_failures = 0;
                state.corrupt_model_signalled = false;
            }
            Err(error) => {
                // CT3: availability is presence+marker only — it never validates that
                // the ONNX graph actually loads. A truncated / bit-rotted graph fails
                // here every 30s forever. Count consecutive load failures; once they
                // hit the cap, surface a "reinstall" signal on the model-status
                // telemetry channel and idle instead of hammering the doomed load.
                state.consecutive_load_failures =
                    state.consecutive_load_failures.saturating_add(1);
                crate::native_capture::debug_log::log_error(format!(
                    "semantic index backfill failed to load model '{}/{}' (consecutive load failures: {}): {error}",
                    descriptor.provider, descriptor.model_id, state.consecutive_load_failures
                ));
                if state.consecutive_load_failures >= MAX_CONSECUTIVE_LOAD_FAILURES {
                    signal_model_appears_corrupt(app_handle, &descriptor, &error);
                    state.corrupt_model_signalled = true;
                    state.embedder = None;
                    return SweepPass::Idle;
                }
                return SweepPass::Error;
            }
        }
    }

    // Embed the batch on a blocking thread (fastembed/ort is CPU model work), then
    // store each vector. The embedder is moved into the blocking task and back out
    // so it survives across passes. The batch is wall-timed to scale the CPU-pacing
    // cooldown that follows.
    let loaded = state.embedder.take().expect("embedder loaded above");
    let texts: Vec<(i64, String)> = batch
        .iter()
        .map(|anchor| (anchor.anchor_id, anchor.body_text.clone()))
        .collect();

    let embed_started_at = Instant::now();
    // CT2: race the blocking embed against the shutdown watch so a quit mid-batch
    // does not wait on a full batch of model work. If shutdown wins, the blocking
    // task is abandoned (it only computes vectors in memory — dropping it leaves no
    // partial DB state) and the worker stops. `select` polls the shutdown future
    // first, so an already-requested shutdown also wins immediately. The embedder
    // moved into the abandoned task is lost; the worker is exiting anyway.
    let embed_task = tauri::async_runtime::spawn_blocking(move || {
        let mut loaded = loaded;
        let mut out: Vec<(i64, std::result::Result<Vec<f32>, String>)> =
            Vec::with_capacity(texts.len());
        for (anchor_id, body_text) in texts {
            let result = loaded
                .embedder
                .embed_text(&body_text)
                .map_err(|error| error.to_string());
            out.push((anchor_id, result));
        }
        (loaded, out)
    });
    let shutdown_changed = shutdown_rx.changed();
    pin_mut!(embed_task, shutdown_changed);
    let (loaded, embedded) = match select(embed_task, shutdown_changed).await {
        Either::Left((join_result, _)) => match join_result {
            Ok(pair) => pair,
            Err(error) => {
                crate::native_capture::debug_log::log_error(format!(
                    "semantic index backfill embed task panicked/cancelled: {error}"
                ));
                // The embedder was moved into the failed task; it will be reloaded.
                return SweepPass::Error;
            }
        },
        Either::Right((_, _)) => {
            // Shutdown requested mid-embed: abandon the in-flight batch and stop.
            return SweepPass::Shutdown;
        }
    };
    // Restore the embedder for the next pass.
    state.embedder = Some(loaded);

    let mut stored = 0u64;
    // Transient errors (DB re-check / store failures): worth a 30s retry.
    let mut transient_errors = 0u64;
    // Deterministic embed failures (a fastembed/ort error on this exact input):
    // counted toward per-anchor quarantine, NOT toward the transient retry loop.
    let mut embed_failures = 0u64;
    let mut quarantined = 0u64;
    let mut dimension_skips = 0u64;
    for (anchor_id, result) in embedded {
        match result {
            Ok(vector) => {
                // Re-check just before storing so a vector derived from text that
                // was deleted (retention / Delete Recent) or already replaced by a
                // reprocess mid-embed is never inserted as an orphan. The atomic
                // row-conditioned store is the real correctness boundary (M1); this
                // re-check is an early-out optimization.
                match infra.semantic_search().anchor_still_missing_vector(anchor_id).await {
                    // `store_vector_if_dimension_matches` is the worker half of the
                    // single dimension authority (the live vec0 column width). If the
                    // embedder reloaded at a new dimension but the table has not yet
                    // been rebuilt — the non-atomic model-switch window, or
                    // permanently after a failed rebuild — the store is **skipped, not
                    // errored**, so the sweep idles instead of error-looping a doomed
                    // batch every 30s forever. Startup reconciliation rebuilds the
                    // stuck table so the dimensions agree again and the skipped
                    // anchors re-embed.
                    Ok(true) => match infra
                        .semantic_search()
                        .store_vector_if_dimension_matches(anchor_id, &vector)
                        .await
                    {
                        Ok(true) => {
                            stored += 1;
                            // A clean store clears any prior failure streak.
                            state.clear_anchor_failures(anchor_id);
                        }
                        Ok(false) => {
                            // Either a dimension mismatch (awaiting re-index) or the
                            // anchor vanished between the re-check and the atomic
                            // store. Neither is a poison-pill, so do not count it
                            // toward quarantine.
                            dimension_skips += 1;
                        }
                        Err(error) => {
                            transient_errors += 1;
                            crate::native_capture::debug_log::log_error(format!(
                                "semantic index backfill failed to store vector for anchor {anchor_id}: {error}"
                            ));
                        }
                    },
                    Ok(false) => {
                        // The anchor was deleted or reprocessed mid-embed; skip it
                        // (the new anchor, if any, is picked up next pass). Clear any
                        // streak so a replaced id starts fresh.
                        state.clear_anchor_failures(anchor_id);
                    }
                    Err(error) => {
                        transient_errors += 1;
                        crate::native_capture::debug_log::log_error(format!(
                            "semantic index backfill failed to re-check anchor {anchor_id}: {error}"
                        ));
                    }
                }
            }
            Err(error) => {
                // L3: a deterministic embed failure for this anchor. Bump its
                // consecutive-failure count; quarantine it once it hits the cap so
                // it stops driving the error loop. This is NOT a transient error.
                embed_failures += 1;
                let (failures, now_quarantined) = state.record_anchor_embed_failure(anchor_id);
                if now_quarantined {
                    quarantined += 1;
                    crate::native_capture::debug_log::log_warn(format!(
                        "semantic index backfill quarantined anchor {anchor_id} after {failures} consecutive embed failures (excluded from the sweep until reprocess/restart); last error: {error}"
                    ));
                } else {
                    crate::native_capture::debug_log::log_error(format!(
                        "semantic index backfill failed to embed anchor {anchor_id} (failure {failures}/{MAX_CONSECUTIVE_ANCHOR_FAILURES}): {error}"
                    ));
                }
            }
        }
    }

    if stored > 0 {
        // Backlog count after this batch, so the log shows historical-backfill
        // progress draining toward zero. Best-effort: a count failure never
        // affects the sweep.
        let backlog = infra
            .semantic_search()
            .count_anchors_missing_vector()
            .await
            .unwrap_or(-1);
        crate::native_capture::debug_log::log_info(format!(
            "semantic index backfill embedded {stored} anchor(s) (batch={}, embed_failures={embed_failures}, transient_errors={transient_errors}, backlog={backlog})",
            batch.len()
        ));
        // CPU pacing: scale the inter-batch cooldown off this batch's wall time and
        // clamp it, then loop. This replaces the old 0ms yield so a large backfill
        // paces itself instead of sustaining a back-to-back multi-core burn.
        return SweepPass::DidWork(backfill_batch_cooldown(embed_started_at.elapsed()));
    }
    if transient_errors > 0 {
        // A real DB hiccup: back off and retry the same batch.
        return SweepPass::Error;
    }
    if dimension_skips > 0 {
        // Every stored anchor was skipped on a dimension mismatch (a switch in
        // flight, or a table stuck at the old dimension after a failed rebuild).
        // Idle — not Error — so the worker does not burn a 30s retry loop; startup
        // reconciliation (or a successful rebuild) restores agreement.
        crate::native_capture::debug_log::log_info(format!(
            "semantic index backfill idled: {dimension_skips} anchor(s) skipped on a vector-dimension mismatch with the live index (awaiting re-index)"
        ));
        return SweepPass::Idle;
    }
    if quarantined > 0 && embed_failures == quarantined {
        // The only non-stored outcome this pass was newly-quarantined poison-pills
        // (nothing left to retry sooner). Idle so the worker does not spin the 30s
        // error loop on anchors it has just decided to skip.
        return SweepPass::Idle;
    }
    if embed_failures > 0 {
        // Embed failures still under the quarantine cap: retry them after a backoff.
        return SweepPass::Error;
    }
    // The batch was non-empty but every anchor was skipped by the re-check (all
    // deleted/reprocessed mid-embed): treat as work so we loop and pick up the
    // replacements (with a minimal pace), but it is effectively idle if nothing
    // remains.
    SweepPass::DidWork(BACKFILL_BATCH_COOLDOWN_MIN)
}

/// CPU-pacing cooldown between backfill batches: the just-finished batch's wall
/// time scaled by [`BACKFILL_BATCH_COOLDOWN_MULTIPLIER`] and clamped to
/// `[BACKFILL_BATCH_COOLDOWN_MIN, BACKFILL_BATCH_COOLDOWN_MAX]`. This mirrors the
/// shape of OCR's Execution Budget governor (`ocr_budget::cooldown_duration`),
/// which clamps `work_ms * multiplier`, so the heavier a batch was the longer the
/// worker yields the cores before the next one — never busy-looping (the floor)
/// and never stalling the backlog (the ceiling).
fn backfill_batch_cooldown(batch_duration: Duration) -> Duration {
    let scaled = batch_duration.mul_f64(BACKFILL_BATCH_COOLDOWN_MULTIPLIER);
    scaled.clamp(BACKFILL_BATCH_COOLDOWN_MIN, BACKFILL_BATCH_COOLDOWN_MAX)
}

/// Surface a "model appears corrupt — reinstall" signal on the model-status
/// telemetry channel (CT3). Reuses the existing download-progress event the
/// Settings UI already listens on: a `Failed` status with a reinstall message
/// triggers a model-status reload in Settings (it reacts to `failed` by reloading
/// status), so the user sees the model flip to "not installed / needs reinstall"
/// rather than the worker silently load-looping forever. Also logged at WARN.
fn signal_model_appears_corrupt(
    app_handle: &tauri::AppHandle,
    descriptor: &SemanticSearchModelDescriptor,
    error: &str,
) {
    crate::native_capture::debug_log::log_warn(format!(
        "semantic index backfill: model '{}/{}' appears corrupt after {MAX_CONSECUTIVE_LOAD_FAILURES} consecutive load failures — surfacing reinstall signal; last error: {error}",
        descriptor.provider, descriptor.model_id
    ));
    let payload = SemanticSearchModelDownloadProgressDto {
        provider: descriptor.provider.clone(),
        model_id: descriptor.model_id.clone(),
        status: SemanticSearchModelDownloadStatusDto::Failed,
        downloaded_bytes: 0,
        total_bytes: None,
        message: Some(
            "The installed semantic search model appears corrupt and could not be loaded. Reinstall it from Settings.".to_string(),
        ),
    };
    if let Err(emit_error) =
        app_handle.emit(SEMANTIC_SEARCH_MODEL_DOWNLOAD_PROGRESS_EVENT, payload)
    {
        crate::native_capture::debug_log::log_error(format!(
            "semantic index backfill: failed to emit corrupt-model signal: {emit_error}"
        ));
    }
}

/// The effective **Semantic Search** selection for the worker and the query
/// embedder: the user's `RecordingSettings.semantic_search`. Falls back to the
/// default-on English-tier selection when the settings state is not yet managed
/// (early startup) so historical capture still gets vectored once a model is
/// installed.
pub(crate) fn effective_semantic_search_settings(
    app_handle: &tauri::AppHandle,
) -> SemanticSearchSettings {
    match app_handle.try_state::<crate::native_capture::RecordingSettingsState>() {
        Some(state) => {
            crate::native_capture::read_recording_settings(state.inner()).semantic_search
        }
        None => default_semantic_search_settings(),
    }
}

/// Resolve the catalog descriptor for the selected model, or `None` when the
/// selection is disabled / unset / unknown.
///
/// Goes through the shared resolver, so a **Custom**-picked fastembed model
/// (synthesized from fastembed's `ModelInfo`) resolves to a descriptor with the
/// correct dimension/window/layout — not just the 3 guided manifest tiers. This
/// is what lets the worker load + embed under a Custom model and the query path
/// embed the search text with the same model.
pub(crate) fn resolve_selected_descriptor(
    settings: &SemanticSearchSettings,
) -> Option<SemanticSearchModelDescriptor> {
    if !settings.enabled {
        return None;
    }
    let model_id = settings.model_id.as_deref()?;
    resolve_descriptor(&settings.provider, model_id)
}

/// Whether the selected **Semantic Search Model** (manifest tier OR Custom pick)
/// is installed on disk — the worker/query model-gate.
///
/// Mirrors the crate's `selected_semantic_search_model_available` but routes
/// through [`resolve_selected_descriptor`] so a Custom model outside the manifest
/// is recognized once downloaded. Returns `false` (a silent no-op, never an error)
/// when disabled / unselected / unresolvable / not yet installed; a corrupt marker
/// is treated as unavailable too.
pub(crate) fn selected_model_available(
    app_data_dir: &std::path::Path,
    settings: &SemanticSearchSettings,
) -> bool {
    let Some(descriptor) = resolve_selected_descriptor(settings) else {
        return false;
    };
    detect_model_status(semantic_search_models_dir(app_data_dir), &descriptor)
        .map(|status| status.is_available())
        .unwrap_or(false)
}

/// Whether the currently-loaded embedder is for `descriptor`'s exact
/// provider/model id (so a Settings model switch reloads it).
fn embedder_matches(
    slot: &Option<LoadedEmbedder>,
    descriptor: &SemanticSearchModelDescriptor,
) -> bool {
    slot.as_ref().is_some_and(|loaded| {
        loaded.provider == descriptor.provider && loaded.model_id == descriptor.model_id
    })
}

/// Load the embedder for `descriptor` from its install directory under
/// `semantic_search_models/{provider}/{model_id}/`.
pub(crate) fn load_embedder(
    app_data_dir: &std::path::Path,
    descriptor: &SemanticSearchModelDescriptor,
) -> Result<LoadedEmbedder, String> {
    let models_dir = semantic_search_models_dir(app_data_dir);
    let install_dir = model_install_dir(&models_dir, &descriptor.provider, &descriptor.model_id)
        .map_err(|error| error.to_string())?;
    let embedder = SemanticSearchEmbedder::load_from_dir(
        &install_dir,
        descriptor.max_tokens,
        // Pooling comes from the descriptor (captured from fastembed's own
        // `get_default_pooling_method`), NOT a guess from the model id. CLS-trained
        // Custom picks (mxbai / gte / snowflake-arctic) don't start with "bge", so
        // the old prefix guess silently mean-pooled them; carrying pooling through
        // the descriptor reads them at the `[CLS]` token correctly.
        fastembed_pooling(descriptor.pooling),
        &descriptor.expected_layout,
        // The output key (fastembed's `ModelInfo.output_key`) also rides the
        // descriptor: `None` for the default single output, or a named tensor for a
        // model that declares one.
        fastembed_output_key(&descriptor.output_key),
    )
    .map_err(|error| error.to_string())?;
    Ok(LoadedEmbedder {
        provider: descriptor.provider.clone(),
        model_id: descriptor.model_id.clone(),
        embedder,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use semantic_search::{builtin_model_manifest, Pooling, SemanticSearchPooling};

    #[test]
    fn descriptor_pooling_drives_the_loader_for_guided_and_custom_models() {
        // The worker loads each model with `fastembed_pooling(descriptor.pooling)`
        // — the pooling fastembed itself assigns — not a guess from the id. This
        // pins both the guided tiers AND the Custom-pickable CLS models the old
        // `starts_with("bge")` guess silently mean-pooled.
        let pooling_for = |slug: &str| -> Pooling {
            let descriptor = resolve_descriptor(semantic_search::FASTEMBED_PROVIDER_ID, slug)
                .unwrap_or_else(|| panic!("{slug} must resolve"));
            fastembed_pooling(descriptor.pooling)
        };

        // Guided tiers: nomic/e5 are Mean, bge-m3 is Cls.
        assert!(matches!(pooling_for("nomic-embed-text-v1.5"), Pooling::Mean));
        assert!(matches!(pooling_for("multilingual-e5-small"), Pooling::Mean));
        assert!(matches!(pooling_for("bge-m3"), Pooling::Cls));

        // Custom-pickable CLS models that do NOT start with "bge" — the exact set
        // the prefix guess got wrong. All must be Cls, never Mean.
        for slug in [
            "mxbai-embed-large-v1",
            "gte-base-en-v1.5",
            "gte-large-en-v1.5",
            "snowflake-arctic-embed-m",
        ] {
            assert!(
                matches!(pooling_for(slug), Pooling::Cls),
                "{slug} is CLS-pooled in fastembed; the loader must not mean-pool it"
            );
        }

        // The descriptor's serde-mirror pooling matches the resolved fastembed one.
        let bge = resolve_descriptor(semantic_search::FASTEMBED_PROVIDER_ID, "bge-m3")
            .expect("bge-m3");
        assert_eq!(bge.pooling, SemanticSearchPooling::Cls);
    }

    #[test]
    fn resolve_descriptor_honors_enabled_and_known_model() {
        let mut settings = default_semantic_search_settings();
        // Default English tier resolves to the nomic 768-dim descriptor.
        let descriptor = resolve_selected_descriptor(&settings).expect("default descriptor");
        assert_eq!(descriptor.model_id, "nomic-embed-text-v1.5");
        assert_eq!(descriptor.dimension, 768);

        // Disabled => no descriptor (the worker idles).
        settings.enabled = false;
        assert!(resolve_selected_descriptor(&settings).is_none());

        // Unknown model => no descriptor.
        settings.enabled = true;
        settings.model_id = Some("not-a-real-model".to_string());
        assert!(resolve_selected_descriptor(&settings).is_none());

        // No model selected => no descriptor.
        settings.model_id = None;
        assert!(resolve_selected_descriptor(&settings).is_none());
    }

    #[test]
    fn embedder_matches_only_the_same_provider_and_model() {
        let manifest = builtin_model_manifest();
        let nomic = manifest
            .models
            .iter()
            .find(|m| m.model_id == "nomic-embed-text-v1.5")
            .expect("nomic descriptor");

        // No loaded embedder => never matches.
        assert!(!embedder_matches(&None, nomic));
    }

    #[test]
    fn anchor_is_quarantined_only_after_n_consecutive_failures() {
        // L3: a deterministically-failing anchor must be retried up to the cap and
        // quarantined exactly at it — never retried forever.
        let mut state = SweepState::new();
        let anchor_id = 42;

        assert!(!state.is_anchor_quarantined(anchor_id), "clean anchor is eligible");

        // Each failure short of the cap leaves the anchor still eligible to retry.
        for expected in 1..MAX_CONSECUTIVE_ANCHOR_FAILURES {
            let (failures, now_quarantined) = state.record_anchor_embed_failure(anchor_id);
            assert_eq!(failures, expected);
            assert!(!now_quarantined, "not quarantined before the cap");
            assert!(!state.is_anchor_quarantined(anchor_id));
        }

        // The Nth consecutive failure quarantines it.
        let (failures, now_quarantined) = state.record_anchor_embed_failure(anchor_id);
        assert_eq!(failures, MAX_CONSECUTIVE_ANCHOR_FAILURES);
        assert!(now_quarantined, "quarantined exactly at the cap");
        assert!(state.is_anchor_quarantined(anchor_id));
    }

    #[test]
    fn a_clean_store_resets_an_anchor_failure_streak() {
        // A transient blip that later succeeds must not accumulate toward
        // quarantine: clearing the streak resets the counter.
        let mut state = SweepState::new();
        let anchor_id = 7;

        state.record_anchor_embed_failure(anchor_id);
        state.record_anchor_embed_failure(anchor_id);
        // A successful store (or delete/reprocess) clears the streak.
        state.clear_anchor_failures(anchor_id);
        assert!(!state.is_anchor_quarantined(anchor_id));

        // It now takes the full cap of consecutive failures again to quarantine.
        for _ in 1..MAX_CONSECUTIVE_ANCHOR_FAILURES {
            assert!(!state.record_anchor_embed_failure(anchor_id).1);
        }
        assert!(state.record_anchor_embed_failure(anchor_id).1);
    }

    #[test]
    fn reprocessing_an_anchor_with_a_new_id_escapes_quarantine() {
        // A reprocess deletes + reinserts the search projection with a NEW
        // search_documents.id; the in-memory quarantine is keyed by id, so the
        // replacement id is simply absent from the map and is retried. This is the
        // "retry only on reprocess" convention the persistent workers express via a
        // new row.
        let mut state = SweepState::new();
        let old_id = 100;
        for _ in 0..MAX_CONSECUTIVE_ANCHOR_FAILURES {
            state.record_anchor_embed_failure(old_id);
        }
        assert!(state.is_anchor_quarantined(old_id), "old id is quarantined");

        let new_id = 101; // the reprocessed anchor's fresh id
        assert!(
            !state.is_anchor_quarantined(new_id),
            "the reprocessed id is not quarantined and is retried"
        );
    }

    #[test]
    fn backfill_cooldown_clamps_work_time_to_the_pacing_band() {
        // CPU pacing: the inter-batch cooldown is the batch wall time scaled and
        // clamped to [MIN, MAX], so a fast batch still yields the cores (floor) and
        // a slow batch does not stall the backlog (ceiling).
        assert_eq!(
            backfill_batch_cooldown(Duration::ZERO),
            BACKFILL_BATCH_COOLDOWN_MIN,
            "a trivially fast batch still pays the floor (the old 0ms gave none)"
        );
        assert_eq!(
            backfill_batch_cooldown(Duration::from_secs(60)),
            BACKFILL_BATCH_COOLDOWN_MAX,
            "a very slow batch is capped at the ceiling"
        );
        // A mid-band batch scales through unclamped (multiplier is 1.0).
        let mid = Duration::from_millis(500);
        assert_eq!(backfill_batch_cooldown(mid), mid);
        assert!(backfill_batch_cooldown(mid) >= BACKFILL_BATCH_COOLDOWN_MIN);
        assert!(backfill_batch_cooldown(mid) <= BACKFILL_BATCH_COOLDOWN_MAX);
    }
}
