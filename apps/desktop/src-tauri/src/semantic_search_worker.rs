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
//! Compute placement: the candle embed is blocking model work (the forward pass
//! runs on the Apple GPU via Metal on macOS, or candle-CPU elsewhere — either way
//! a synchronous call that must not occupy the async reactor), so it runs on a
//! blocking thread (never the tokio reactor, never the capture hot path). DB
//! reads/writes stay on the async loop. Unlike the retired fastembed/ort path,
//! candle on Metal frees the P-cores by construction, so the embed no longer needs
//! a per-thread background-QoS downclock (ADR 0037); the tokio-level inter-batch
//! cooldown is kept (still useful to pace candle-CPU on non-macOS).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use capture_types::{default_semantic_search_settings, SemanticSearchSettings};
use futures_util::{
    future::{select, Either},
    pin_mut,
};
use semantic_search::{
    detect_model_status, model_install_dir, resolve_descriptor, semantic_search_models_dir,
    EmbedKind, SemanticSearchEmbedder, SemanticSearchModelDescriptor,
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

/// Consecutive idle passes (backlog drained) before the loaded embedder is
/// dropped to return its resident memory to the OS — the "decay to idle" lever.
///
/// The **Semantic Search Model** weights are large and resident for the whole
/// time the embedder is held, so a caught-up worker that keeps the embedder
/// loaded pins that floor forever. candle has no process-global memory arena to
/// retain (the retired `ort` path did), so dropping the embedder on a sustained
/// idle actually returns its weights (and GPU buffers) to the OS.
///
/// A grace period (not an immediate drop on the first empty peek) avoids
/// thrashing: live capture trickles fresh anchors in, and reloading the model
/// costs a model read + device init. At [`IDLE_POLL_INTERVAL`] = 20 s, 3 passes
/// is ~60 s of being caught up before the weights are released; the next anchor
/// pays one reload. This is the cheap, in-process approximation of a sidecar (a
/// process that exits when drained returns *everything*, including the base
/// runtime — this only frees the model).
const IDLE_PASSES_BEFORE_EMBEDDER_DROP: u32 = 3;

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
/// presence alone — it never validates that the safetensors weights actually load
/// into candle — so a truncated/bit-rotted model would otherwise fail
/// `load_embedder` every 30s forever. After this many consecutive load failures
/// the worker surfaces a
/// "model appears corrupt — reinstall" signal on the model-status telemetry
/// channel and idles instead of hammering the doomed load.
const MAX_CONSECUTIVE_LOAD_FAILURES: u32 = 3;

/// CPU pacing (the "Backfill CPU pacing" cross-cutting concern): a minimum
/// inter-batch cooldown so a large historical backfill does not sustain a
/// multi-core burn back-to-back concurrent with active OCR/transcription. It is
/// kept under candle (ADR 0037): on macOS the Metal forward leaves the CPU mostly
/// idle, but on candle-CPU (non-macOS) the forward is the CPU burn the cooldown
/// paces, so the inter-batch yield still earns its place. This mirrors the *shape*
/// of OCR's Execution Budget governor
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
    /// The `(provider, model_id)` a corrupt-model signal was surfaced for, if any,
    /// so the worker idles quietly for THAT selection rather than re-emitting every
    /// tick. Keyed by the signalled model identity — `None` until a model is
    /// flagged corrupt. Cleared (back to `None`) when the model goes unavailable on
    /// disk, when a load later succeeds, or — crucially — when the user switches to
    /// a DIFFERENT selection, so a valid model B is loaded normally even after model
    /// A was flagged corrupt. The latch only short-circuits to Idle when the
    /// currently-selected identity EQUALS the stored one.
    corrupt_model_signalled: Option<(String, String)>,
    /// Consecutive idle passes since the last pass that embedded something. Drives
    /// the idle-drop ([`IDLE_PASSES_BEFORE_EMBEDDER_DROP`]): once a caught-up
    /// worker has idled this many passes in a row, the embedder is dropped to
    /// return the model weights to the OS. Reset to 0 by any pass that does work.
    consecutive_idles: u32,
}

impl SweepState {
    fn new() -> Self {
        Self {
            embedder: None,
            logged_no_model: false,
            anchor_failures: HashMap::new(),
            consecutive_load_failures: 0,
            corrupt_model_signalled: None,
            consecutive_idles: 0,
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

    /// Whether the currently-selected `(provider, model_id)` has already been
    /// signalled corrupt this stretch (CT3). True only when a corrupt signal was
    /// raised for *exactly this* identity, so a switch to a different (valid) model
    /// is never short-circuited by a latch raised for the old one.
    fn corrupt_latch_matches(&self, provider: &str, model_id: &str) -> bool {
        self.corrupt_model_signalled
            .as_ref()
            .is_some_and(|(p, m)| p == provider && m == model_id)
    }

    /// Reconcile the corrupt-model latch and the load counter against the
    /// currently-selected `(provider, model_id)`. If a corrupt signal is latched for
    /// a DIFFERENT selection (the user switched models), clear it and reset the load
    /// counter (and drop any cached embedder, which is for the old model) so the new
    /// selection gets a clean set of load attempts. A no-op when the latch already
    /// names this selection or is unset.
    fn reconcile_selection(&mut self, provider: &str, model_id: &str) {
        if matches!(&self.corrupt_model_signalled, Some((p, m)) if p == provider && m == model_id) {
            return;
        }
        if self.corrupt_model_signalled.is_some() {
            // A latch raised for a different model: the selection changed, so the
            // new model deserves a fresh start.
            self.corrupt_model_signalled = None;
            self.consecutive_load_failures = 0;
            self.embedder = None;
        }
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
                    // Work happened: reset the idle streak so the embedder stays warm
                    // for the rest of the drain (work never releases — returns false).
                    advance_idle_drop(&mut state.consecutive_idles, true, state.embedder.is_some());
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
                    if advance_idle_drop(
                        &mut state.consecutive_idles,
                        false,
                        state.embedder.is_some(),
                    ) {
                        state.embedder = None;
                        crate::native_capture::debug_log::log_info(format!(
                            "semantic index backfill released the embedder after {} idle passes (model weights returned to the OS; reloads on next anchor)",
                            state.consecutive_idles
                        ));
                    }
                    IDLE_POLL_INTERVAL
                }
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

/// Distinguishes a `load_embedder` failure from a successful-load-but-embed
/// failure when both now run inside the one `spawn_blocking` (M1). The closure
/// returns `Result<(LoadedEmbedder, Vec<per-anchor results>), LoadError>`: an
/// `Err(LoadError)` means the model never loaded (→ CT3 load-failure accounting:
/// `consecutive_load_failures`, the corrupt-model signal), while an `Ok` with
/// per-anchor `Err`s inside the `Vec` means the model loaded fine and individual
/// anchors failed to embed (→ the existing per-anchor L3 quarantine handling). The
/// two failure modes stay branchable exactly as they were when the load ran inline
/// on the reactor.
struct LoadError {
    error: String,
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
        state.corrupt_model_signalled = None;
        if !state.logged_no_model {
            crate::native_capture::debug_log::log_info(
                "semantic index backfill skipped: no Semantic Search Model installed (silent no-op)",
            );
            state.logged_no_model = true;
        }
        return SweepPass::Idle;
    }
    state.logged_no_model = false;

    // Resolve the catalog descriptor (dimension/window/pooling + install path) for
    // the selected model BEFORE the corrupt-model latch check, so the latch can be
    // keyed by the model it was raised for. (Availability said yes but the
    // descriptor vanished — defensive; treat as unavailable for this pass.)
    let Some(descriptor) = resolve_selected_descriptor(&settings) else {
        state.embedder = None;
        return SweepPass::Idle;
    };

    // CT3: reconcile the corrupt-model latch against the currently-selected model.
    // The latch is keyed by the `(provider, model_id)` it was raised for: switching
    // to a DIFFERENT (valid) model clears the latch, the load counter, and the
    // stale cached embedder so model B loads cleanly even after model A was flagged
    // corrupt. The `!available` branch above clears it on uninstall/reinstall.
    state.reconcile_selection(&descriptor.provider, &descriptor.model_id);

    // CT3: if the *currently-selected* model has already been signalled corrupt this
    // stretch (N consecutive load failures), idle quietly rather than re-attempting
    // the doomed load every tick. Only the matching identity short-circuits — a
    // different selection was cleared by `reconcile_selection` above.
    if state.corrupt_latch_matches(&descriptor.provider, &descriptor.model_id) {
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

    // The catalog descriptor (dimension/window/pooling + install path) for the
    // selected model was resolved above (before the corrupt-model latch). The actual
    // embedder load is deferred into the blocking task below (M1): `load_embedder`
    // does heavy synchronous I/O + model init (`fs::read`, `from_mmaped_safetensors`
    // over hundreds of MB, Metal/device init, tokenizer load), so it must never run
    // on the tokio reactor — it is folded into the same `spawn_blocking` as the
    // forward, mirroring the query path.

    // Embed the batch on a blocking thread: BOTH the (conditional) embedder load
    // AND the candle forward are synchronous model work (Metal GPU on macOS /
    // candle-CPU elsewhere) that must stay off the tokio reactor, then store each
    // vector. The cached embedder is moved into the blocking task and back out so it
    // survives across passes (it is shared `&self`-immutable for the embed itself).
    // If the cached embedder is already for this descriptor it is reused; otherwise
    // it is (re)loaded inside the task. The batch is wall-timed to scale the
    // CPU-pacing cooldown that follows.
    //
    // The cached embedder is taken out here so it can be moved into the task; on a
    // load failure (which clears the slot inside the task) it stays `None` and is
    // retried/loaded next pass, matching the pre-M1 behavior where a failed load
    // also left `state.embedder` unset.
    let cached = if embedder_matches(&state.embedder, &descriptor) {
        state.embedder.take()
    } else {
        // A different model (Settings switch) or nothing cached: drop it and (re)load
        // inside the task below.
        state.embedder = None;
        None
    };
    let texts: Vec<(i64, String)> = batch
        .iter()
        .map(|anchor| (anchor.anchor_id, anchor.body_text.clone()))
        .collect();

    let embed_started_at = Instant::now();
    let app_data_dir_for_task = app_data_dir.clone();
    let descriptor_for_task = descriptor.clone();
    // CT2: race the blocking load+embed against the shutdown watch so a quit
    // mid-batch does not wait on a full batch of model load+forward work. If
    // shutdown wins, the blocking task is abandoned (it only loads/computes in
    // memory — dropping it leaves no partial DB state) and the worker stops.
    // `select` polls the shutdown future first, so an already-requested shutdown
    // also wins immediately. The embedder moved into the abandoned task is lost; the
    // worker is exiting anyway.
    let embed_task = tauri::async_runtime::spawn_blocking(move || {
        // M1: do the heavy `load_embedder` here (off the reactor) when the cached
        // embedder is absent or for a different model. A load failure short-circuits
        // with `LoadError` so the caller runs the CT3 load-failure accounting
        // (consecutive-load-failure counter, corrupt-model signal) — kept DISTINCT
        // from a successful-load-but-per-anchor-embed-failure, which is carried in
        // the returned per-anchor `Vec` for the existing L3 handling.
        let loaded = match cached {
            Some(loaded) => loaded,
            None => match load_embedder(&app_data_dir_for_task, &descriptor_for_task) {
                Ok(loaded) => loaded,
                Err(error) => return Err(LoadError { error }),
            },
        };
        // candle on Metal frees the P-cores by construction, so the retired
        // per-thread background-QoS downclock is gone (ADR 0037); the embed runs at
        // the blocking thread's default QoS. The embedder is `&self`-immutable for
        // embedding, so no `mut` is needed — it is still owned by (and returned out
        // of) this task so it survives across passes.
        // One batched candle call for the whole batch (vs one `embed_text` per
        // anchor): fewer total forward passes, so the backlog drains sooner.
        // `embed_texts` returns exactly one result per text, in order, with the
        // same overflow-split/single-passthrough/multi-mean-pool semantics as
        // `embed_text`. `bodies` borrows `texts`, so build `out` after it returns.
        let bodies: Vec<&str> = texts.iter().map(|(_, body)| body.as_str()).collect();
        let results = loaded.embedder.embed_texts(&bodies, EmbedKind::Document);
        let out: Vec<(i64, std::result::Result<Vec<f32>, String>)> = texts
            .iter()
            .map(|(anchor_id, _)| *anchor_id)
            .zip(results)
            .map(|(anchor_id, result)| (anchor_id, result.map_err(|error| error.to_string())))
            .collect();
        Ok((loaded, out))
    });
    let shutdown_changed = shutdown_rx.changed();
    pin_mut!(embed_task, shutdown_changed);
    let load_embed = match select(embed_task, shutdown_changed).await {
        Either::Left((join_result, _)) => match join_result {
            Ok(load_embed) => load_embed,
            Err(error) => {
                crate::native_capture::debug_log::log_error(format!(
                    "semantic index backfill embed task panicked/cancelled: {error}"
                ));
                // The embedder (if any) was moved into the failed task; it will be
                // reloaded next pass (the slot is already `None` from the take above).
                return SweepPass::Error;
            }
        },
        Either::Right((_, _)) => {
            // Shutdown requested mid-load/embed: abandon the in-flight batch and stop.
            return SweepPass::Shutdown;
        }
    };
    // Branch load-vs-embed exactly as before M1, just sourced from the task result:
    //   - `Err(LoadError)` => CT3 load-failure accounting (distinct failure mode).
    //   - `Ok((loaded, embedded))` => the model loaded OK; per-anchor results carry
    //     any embed failures for the existing L3 handling below.
    let (loaded, embedded) = match load_embed {
        Ok(pair) => {
            // A successful load (or a reuse of the cached embedder, which also proves
            // the weights are fine) resets CT3.
            state.consecutive_load_failures = 0;
            state.corrupt_model_signalled = None;
            pair
        }
        Err(LoadError { error }) => {
            // CT3: availability is presence+marker only — it never validates that the
            // safetensors weights actually load into candle. A truncated / bit-rotted
            // model fails here every 30s forever. Count consecutive load failures;
            // once they hit the cap, surface a "reinstall" signal on the model-status
            // telemetry channel and idle instead of hammering the doomed load. The
            // load now runs on the blocking thread (M1), so this accounting happens
            // after the task returns rather than inline on the reactor — the branching
            // is otherwise identical.
            state.consecutive_load_failures = state.consecutive_load_failures.saturating_add(1);
            crate::native_capture::debug_log::log_error(format!(
                "semantic index backfill failed to load model '{}/{}' (consecutive load failures: {}): {error}",
                descriptor.provider, descriptor.model_id, state.consecutive_load_failures
            ));
            if state.consecutive_load_failures >= MAX_CONSECUTIVE_LOAD_FAILURES {
                signal_model_appears_corrupt(app_handle, &descriptor, &error);
                // Latch the corrupt signal to THIS selection's identity so a later
                // switch to a different (valid) model is not short-circuited by it.
                state.corrupt_model_signalled =
                    Some((descriptor.provider.clone(), descriptor.model_id.clone()));
                // The slot is already `None` (taken above; the task did not return an
                // embedder on a load failure).
                return SweepPass::Idle;
            }
            return SweepPass::Error;
        }
    };
    // Restore the embedder for the next pass.
    state.embedder = Some(loaded);

    // Whole-batch vs per-anchor failure (data-integrity gate): `embed_texts` now
    // isolates a true poison input to its own text (a failing chunk fails only its
    // own text, after a per-chunk retry of any failed sub-batch), so a single
    // failing anchor among healthy siblings is a genuine per-anchor fault. Several
    // anchors failing TOGETHER, by contrast, is the signature of a transient
    // whole-batch fault (e.g. a recurring GPU OOM that fails every chunk even at the
    // single-chunk shape). Crediting each of those a deterministic L3 failure would
    // quarantine up to a whole 16-anchor newest-first window of healthy anchors on
    // one transient fault. So a per-anchor deterministic failure is only credited
    // when an anchor fails in ISOLATION (exactly one embed error in the batch); when
    // more than one anchor fails together the batch is treated as a transient error
    // (back off, quarantine nobody). A genuine single poison anchor among healthy
    // siblings still surfaces alone (the others store), so it still accrues toward
    // quarantine and is eventually excluded — that path is preserved.
    let embed_failure_count = embedded
        .iter()
        .filter(|(_, result)| result.is_err())
        .count();
    let isolated_embed_failure = is_isolated_embed_failure(embed_failure_count);

    let mut stored = 0u64;
    // Transient errors (DB re-check / store failures, OR a whole-batch embed fault):
    // worth a 30s retry.
    let mut transient_errors = 0u64;
    // Deterministic embed failures (a candle error on this exact input):
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
                if !isolated_embed_failure {
                    // More than one anchor in this batch failed to embed.
                    // `embed_texts` already isolates a true poison input to its own
                    // text (a failing chunk fails only its own text, after a
                    // per-chunk retry), so multiple anchors failing TOGETHER is the
                    // signature of a transient whole-batch fault (e.g. a GPU OOM that
                    // recurs even at the single-chunk shape), NOT several independent
                    // poison pills. Treat it as transient — back off and retry the
                    // whole batch later — WITHOUT crediting any anchor a deterministic
                    // L3 failure, so a transient OOM never quarantines a window of
                    // healthy anchors. Do not bump the anchor's streak.
                    transient_errors += 1;
                    crate::native_capture::debug_log::log_error(format!(
                        "semantic index backfill batch embed failure ({embed_failure_count} anchors in this batch); treating as transient (no quarantine); anchor {anchor_id} last error: {error}"
                    ));
                    continue;
                }
                // L3: a deterministic embed failure for this anchor IN ISOLATION (it
                // was the only anchor in the batch to fail, while siblings stored
                // fine — so the fault is the input, not the batch). Bump its
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

/// Whether an embed failure in a batch should be credited as a deterministic
/// per-anchor (L3) failure, given how many anchors in the SAME batch failed.
///
/// Data-integrity gate: `SemanticSearchEmbedder::embed_texts` isolates a true
/// poison input to its own text (a failing chunk fails only its own text), so a
/// single failing anchor among healthy siblings is a genuine per-anchor fault and
/// is credited toward quarantine. When MORE THAN ONE anchor in the batch fails
/// together, that is the signature of a transient whole-batch fault (e.g. a
/// recurring GPU OOM that fails every chunk even on the per-chunk retry) rather
/// than per-anchor poison — so it is treated as transient (back off) and NO anchor
/// is credited a deterministic failure, which would otherwise quarantine a whole
/// newest-first window of healthy anchors. A genuine single poison anchor still
/// surfaces alone (its healthy batch-mates store), so it still accrues toward
/// quarantine and is eventually excluded.
fn is_isolated_embed_failure(embed_failure_count: usize) -> bool {
    embed_failure_count == 1
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
/// Goes through the shared resolver, which is now a pure manifest lookup over the
/// hand-coded candle catalog (ADR 0037): a known id resolves to a descriptor with
/// the right architecture/dimension/window/pooling/layout, and an unknown id is
/// simply `None` (there is no fastembed synthesis to fall back on). This is what
/// lets the worker load + embed under the selected model and the query path embed
/// the search text with the same model.
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

/// Advance the idle-drop state machine for one completed sweep pass and report
/// whether the embedder should be released now (the "decay to idle" lever).
///
/// `did_work` (a `DidWork` pass) resets the streak to 0 — the embedder stays warm
/// for the rest of the drain, and work never triggers a release (returns `false`).
/// Otherwise the pass was a caught-up `Idle`: the streak grows, and once it
/// reaches [`IDLE_PASSES_BEFORE_EMBEDDER_DROP`] *with a model still loaded* the
/// embedder is released. It returns `true` exactly **once per idle stretch** —
/// the caller drops the embedder, so the next idle pass has `has_embedder = false`
/// and returns `false` (no re-drop, no re-log) until work reloads it. `Error`
/// passes don't call this, so a transient error neither grows nor resets the
/// streak.
fn advance_idle_drop(consecutive_idles: &mut u32, did_work: bool, has_embedder: bool) -> bool {
    if did_work {
        *consecutive_idles = 0;
        return false;
    }
    *consecutive_idles = consecutive_idles.saturating_add(1);
    *consecutive_idles >= IDLE_PASSES_BEFORE_EMBEDDER_DROP && has_embedder
}

/// Load the embedder for `descriptor` from its install directory under
/// `semantic_search_models/{provider}/{model_id}/`.
///
/// The candle backend (built inside `load_from_dir`) reads everything it needs —
/// architecture, dimension, window, pooling, on-disk layout — from the one
/// `descriptor`. There is no ONNX intra-op thread cap to resolve (ADR 0037):
/// candle runs the forward on the Metal GPU on macOS / candle-CPU elsewhere, with
/// no thread-pool spin-wait to clamp.
pub(crate) fn load_embedder(
    app_data_dir: &std::path::Path,
    descriptor: &SemanticSearchModelDescriptor,
) -> Result<LoadedEmbedder, String> {
    let models_dir = semantic_search_models_dir(app_data_dir);
    let install_dir = model_install_dir(&models_dir, &descriptor.provider, &descriptor.model_id)
        .map_err(|error| error.to_string())?;
    let embedder = SemanticSearchEmbedder::load_from_dir(&install_dir, descriptor)
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
    use semantic_search::{builtin_model_manifest, SemanticSearchPooling};

    #[test]
    fn descriptor_pooling_is_hand_coded_per_model() {
        // Pooling rides the hand-coded descriptor (ADR 0037), never a guess from the
        // model id: nomic/e5 are Mean, bge-m3 is Cls. The worker loads each model
        // through `load_from_dir(&install_dir, descriptor)`, so the candle backend
        // pools with exactly `descriptor.pooling` — this pins the declared value the
        // loader is handed.
        let pooling_for = |slug: &str| -> SemanticSearchPooling {
            resolve_descriptor(semantic_search::SEMANTIC_SEARCH_PROVIDER_ID, slug)
                .unwrap_or_else(|| panic!("{slug} must resolve"))
                .pooling
        };

        assert_eq!(pooling_for("nomic-embed-text-v1.5"), SemanticSearchPooling::Mean);
        assert_eq!(pooling_for("multilingual-e5-small"), SemanticSearchPooling::Mean);
        assert_eq!(pooling_for("bge-m3"), SemanticSearchPooling::Cls);
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
    fn idle_drop_releases_the_embedder_exactly_once_after_the_grace_period() {
        // The "decay to idle" lever: a caught-up worker holds the model weights
        // resident, so after a grace period of consecutive idle passes the embedder
        // must be released — but only once per idle stretch, not every tick.
        let mut idles = 0u32;

        // Idle passes short of the grace threshold keep the embedder warm.
        for _ in 1..IDLE_PASSES_BEFORE_EMBEDDER_DROP {
            assert!(
                !advance_idle_drop(&mut idles, false, true),
                "must not release before the grace threshold"
            );
        }
        // The Nth consecutive idle pass releases it (caller drops the embedder).
        assert_eq!(idles, IDLE_PASSES_BEFORE_EMBEDDER_DROP - 1);
        assert!(
            advance_idle_drop(&mut idles, false, true),
            "releases exactly at the grace threshold"
        );

        // After the drop the embedder is gone: further idle passes must NOT signal a
        // release again (no re-drop, no re-log) — once per idle stretch.
        assert!(
            !advance_idle_drop(&mut idles, false, false),
            "no second release while already unloaded"
        );
        assert!(!advance_idle_drop(&mut idles, false, false));
    }

    #[test]
    fn idle_drop_streak_resets_on_any_work_pass() {
        // A single pass that embeds something resets the streak, so a worker that is
        // still trickling through a backlog never sheds the warm embedder mid-drain.
        let mut idles = 0u32;

        // Idle right up to the brink...
        for _ in 1..IDLE_PASSES_BEFORE_EMBEDDER_DROP {
            assert!(!advance_idle_drop(&mut idles, false, true));
        }
        // ...then a work pass resets the streak (and never releases on work).
        assert!(!advance_idle_drop(&mut idles, true, true));
        assert_eq!(idles, 0, "work resets the idle streak");

        // The next idle stretch must start counting from zero, not fire immediately.
        for _ in 1..IDLE_PASSES_BEFORE_EMBEDDER_DROP {
            assert!(
                !advance_idle_drop(&mut idles, false, true),
                "post-work idle streak restarts from zero"
            );
        }
        assert!(advance_idle_drop(&mut idles, false, true));
    }

    #[test]
    fn idle_drop_never_fires_without_a_loaded_embedder() {
        // The no-model / already-unloaded idle path: a worker with nothing loaded
        // (e.g. no Semantic Search Model installed) idles forever without ever
        // signalling a release, no matter how long the streak grows.
        let mut idles = 0u32;
        for _ in 0..(IDLE_PASSES_BEFORE_EMBEDDER_DROP + 5) {
            assert!(
                !advance_idle_drop(&mut idles, false, false),
                "nothing to release when no embedder is loaded"
            );
        }
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
    fn corrupt_latch_is_keyed_to_the_signalled_model_identity() {
        // CT3: the corrupt-model latch must only short-circuit the selection it was
        // raised for. A latch for model A does not idle a switch to model B.
        let mut state = SweepState::new();
        let provider = "mnema";
        let model_a = "corrupt-model";
        let model_b = "good-model";

        // No latch initially: nothing matches.
        assert!(!state.corrupt_latch_matches(provider, model_a));

        // Latch model A corrupt (as the load-failure cap site does).
        state.corrupt_model_signalled = Some((provider.to_string(), model_a.to_string()));
        state.consecutive_load_failures = MAX_CONSECUTIVE_LOAD_FAILURES;
        // Pretend a stale embedder for A is cached.
        assert!(state.corrupt_latch_matches(provider, model_a));
        assert!(
            !state.corrupt_latch_matches(provider, model_b),
            "a different model is never matched by A's latch"
        );

        // Reconciling against the SAME model is a no-op: the latch (and counter) hold.
        state.reconcile_selection(provider, model_a);
        assert_eq!(
            state.corrupt_model_signalled,
            Some((provider.to_string(), model_a.to_string()))
        );
        assert_eq!(state.consecutive_load_failures, MAX_CONSECUTIVE_LOAD_FAILURES);
        assert!(state.corrupt_latch_matches(provider, model_a));
    }

    #[test]
    fn switching_models_clears_a_corrupt_latch_raised_for_the_old_model() {
        // The regression FIX 1 fixes: after model A is flagged corrupt, switching to
        // a valid model B must clear the latch (and the load counter, and the stale
        // cached embedder) so B is loaded instead of the worker idling forever.
        let mut state = SweepState::new();
        let provider = "mnema";
        let model_a = "corrupt-model";
        let model_b = "good-model";

        state.corrupt_model_signalled = Some((provider.to_string(), model_a.to_string()));
        state.consecutive_load_failures = MAX_CONSECUTIVE_LOAD_FAILURES;

        // The user switches to model B: reconcile clears the latch keyed to A.
        state.reconcile_selection(provider, model_b);
        assert_eq!(state.corrupt_model_signalled, None, "A's latch is cleared on switch");
        assert_eq!(state.consecutive_load_failures, 0, "load counter resets for the new model");
        assert!(
            !state.corrupt_latch_matches(provider, model_b),
            "model B is not short-circuited and proceeds to load"
        );
    }

    #[test]
    fn a_whole_batch_embed_failure_does_not_quarantine_each_anchor() {
        // FIX 2: a transient whole-batch fault surfaces as MORE THAN ONE failing
        // anchor in the batch, and must be treated as transient (no per-anchor
        // quarantine), while a genuine single poison anchor (one failure, healthy
        // siblings) is still credited toward quarantine.
        assert!(
            !is_isolated_embed_failure(0),
            "no failures => nothing to credit"
        );
        assert!(
            is_isolated_embed_failure(1),
            "exactly one anchor failing in isolation is a genuine per-anchor fault"
        );
        for batch_failures in 2..=SWEEP_BATCH_SIZE as usize {
            assert!(
                !is_isolated_embed_failure(batch_failures),
                "{batch_failures} anchors failing together is a transient batch fault, not poison"
            );
        }
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
