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
use crate::debug_status::SemanticWorkerHealth;
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

/// Maximum **consecutive** transient-only passes over the SAME anchor-id set
/// before the worker re-runs that residual batch ONE ANCHOR AT A TIME to
/// re-isolate genuine poison (F3). The batched whole-window embed credits a
/// per-anchor L3 failure only when exactly one anchor fails in isolation, so a
/// CLUSTER of >=2 deterministically-failing anchors sharing the newest-first
/// window is misclassified transient forever: nobody is quarantined, the window
/// never slides, and the backlog behind it never drains. After this many
/// consecutive transient-only passes that do not shrink the set, the worker drops
/// to per-anchor embedding (each text in its own `embed_texts` call), so every
/// genuine poison anchor fails in isolation, accrues toward quarantine, and is
/// eventually excluded — sliding the window past the cluster. Kept small so a real
/// cluster is broken up promptly while a single transient blip (which clears on
/// its own next pass) never triggers the slower per-anchor path.
const MAX_TRANSIENT_PASSES_BEFORE_PER_ANCHOR_ISOLATION: u32 = 2;

/// Coarse cadence for the logging-only backlog `count_anchors_missing_vector`
/// (F6). The count is a full COUNT(*) over the missing set and is purely for the
/// progress log line, yet the sweep can run it every ~1-2s on the shared capture
/// pool during a fast drain. Only run it once every this-many work passes so it is
/// not a per-pass full operation contending with capture; the peek
/// (`anchors_missing_vector`) stays per-pass as the real work driver.
const BACKLOG_COUNT_EVERY_N_WORK_PASSES: u32 = 16;

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
    /// Why the last embedder LOAD failed, kept only so the debug surface can show
    /// WHY the model will not load (before this it was logged and dropped). Never
    /// read by the sweep itself; cleared alongside `consecutive_load_failures`.
    last_load_error: Option<String>,
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
    /// Clustered-poison detector (F3): the sorted anchor-id set of the most recent
    /// pass that ended transient-only (a whole-batch embed fault, no store, no
    /// quarantine) and how many consecutive transient-only passes have now landed on
    /// EXACTLY that set. When a cluster of >=2 deterministically-failing anchors
    /// shares the newest-first window, the batched embed misclassifies it transient
    /// every pass, so the set never shrinks; once the count reaches
    /// [`MAX_TRANSIENT_PASSES_BEFORE_PER_ANCHOR_ISOLATION`] the next pass embeds the
    /// residual ONE ANCHOR AT A TIME to re-isolate the genuine poison and credit
    /// per-anchor L3 failures, sliding the window past it. Cleared (back to `None`)
    /// by any pass that stores, quarantines, or otherwise makes progress, and by a
    /// model switch.
    transient_stuck: Option<(Vec<i64>, u32)>,
    /// Work-pass counter for the coarse backlog-count cadence (F6). The
    /// logging-only `count_anchors_missing_vector` runs only once every
    /// [`BACKLOG_COUNT_EVERY_N_WORK_PASSES`] work passes instead of every pass, so a
    /// fast drain does not hammer the shared capture pool with a full COUNT(*).
    work_passes_since_count: u32,
    /// The `(provider, model_id)` the previous pass reconciled against, so
    /// [`reconcile_selection`] can detect ANY model switch — not only one where a
    /// corrupt-model latch happened to be set (F4). On a switch the vec0 table is
    /// rebuilt and every anchor re-enters the missing set, so the model-specific L3
    /// quarantine map and clustered-poison detector must be cleared for the new model
    /// even on a clean (never-flagged-corrupt) switch. `None` until the first pass.
    last_selection: Option<(String, String)>,
}

impl SweepState {
    fn new() -> Self {
        Self {
            embedder: None,
            logged_no_model: false,
            anchor_failures: HashMap::new(),
            consecutive_load_failures: 0,
            last_load_error: None,
            corrupt_model_signalled: None,
            consecutive_idles: 0,
            transient_stuck: None,
            work_passes_since_count: 0,
            last_selection: None,
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

    /// Record one embedder LOAD failure (CT3): bump the consecutive-load-failure
    /// streak and keep `error` so the debug surface can show why the model will not
    /// load. Returns the new streak (the caller logs it and compares it against
    /// [`MAX_CONSECUTIVE_LOAD_FAILURES`]).
    fn record_load_failure(&mut self, error: &str) -> u32 {
        self.consecutive_load_failures = self.consecutive_load_failures.saturating_add(1);
        self.last_load_error = Some(error.to_string());
        self.consecutive_load_failures
    }

    /// Forget every load-failure record: the streak, the CT3 corrupt-model latch,
    /// and the last load error. Called when the model loaded (or the cached embedder
    /// proved the weights are fine), when the selection goes unavailable, and on a
    /// model switch — in each case the recorded failures belong to a past attempt on
    /// a model that is no longer the one being retried.
    fn clear_load_failures(&mut self) {
        self.consecutive_load_failures = 0;
        self.corrupt_model_signalled = None;
        self.last_load_error = None;
    }

    /// The debug-surface snapshot of this worker's health, published after every
    /// sweep pass. A pure read of the state above — `quarantined_count` is derived
    /// (the map also holds sub-cap streaks, which are not quarantines).
    fn health_snapshot(&self) -> SemanticWorkerHealth {
        SemanticWorkerHealth {
            model_loaded: self.embedder.is_some(),
            consecutive_load_failures: self.consecutive_load_failures,
            quarantined_count: self
                .anchor_failures
                .values()
                .filter(|&&failures| failures >= MAX_CONSECUTIVE_ANCHOR_FAILURES)
                .count(),
            last_load_error: self.last_load_error.clone(),
        }
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

    /// Reconcile all model-keyed in-memory state against the currently-selected
    /// `(provider, model_id)` at the top of a pass. On ANY model switch (detected via
    /// `last_selection`, so it fires on a clean switch too — not only when a
    /// corrupt-model latch happened to be set), clear the per-model state so the new
    /// model starts fresh:
    ///   - the CT3 corrupt-model latch + the consecutive-load-failure counter, so a
    ///     valid model B loads normally even after model A was flagged corrupt;
    ///   - the cached embedder, which is for the old model (`embedder_matches` would
    ///     reload it anyway; clearing here is explicit);
    ///   - **F4**: the L3 per-anchor quarantine map and the F3 clustered-poison
    ///     detector. Both are keyed by `search_documents.id` but are only meaningful
    ///     for the model that produced the failure (a tokenizer-incompatible input
    ///     under model A may embed fine under model B). On a switch the vec0 table is
    ///     rebuilt and every anchor re-enters the missing set, so a stale quarantine
    ///     from A would wrongly filter that anchor from every batch under B until
    ///     restart. Clearing here is the fix — and it must run on a clean switch, not
    ///     just a corrupt-latched one, which is why it keys off `last_selection`.
    ///   - the idle-drop streak, so the switch does not satisfy the idle-drop on a
    ///     stale count from the old model's drain.
    /// A no-op when the selection is unchanged from the previous pass.
    fn reconcile_selection(&mut self, provider: &str, model_id: &str) {
        let selection = (provider.to_string(), model_id.to_string());
        // A switch is detected EITHER by the tracked previous selection differing
        // (the clean-switch path that F4 needs) OR by a corrupt latch standing for a
        // different model (the original CT3 path — preserved so a latch raised for A
        // is cleared even on the first reconcile that names B, before `last_selection`
        // has been primed).
        let switched_from_last = self
            .last_selection
            .as_ref()
            .is_some_and(|previous| previous != &selection);
        let latch_names_other_model =
            matches!(&self.corrupt_model_signalled, Some((p, m)) if (p.as_str(), m.as_str()) != (provider, model_id));
        let switched = switched_from_last || latch_names_other_model;
        self.last_selection = Some(selection);
        if !switched {
            // Unchanged selection (or the very first pass with no stale latch): keep the
            // corrupt latch and all counters exactly as they were. CT3 short-circuiting
            // for THIS model is handled by the caller via `corrupt_latch_matches`.
            return;
        }
        // A genuine model switch: every piece of model-keyed state belongs to the old
        // model and must not bleed into the new one.
        self.clear_load_failures();
        self.embedder = None;
        self.anchor_failures.clear();
        self.transient_stuck = None;
        self.consecutive_idles = 0;
    }

    /// F3: whether the worker is already stuck on EXACTLY this anchor-id set — it has
    /// ended transient-only over the same set at least
    /// [`MAX_TRANSIENT_PASSES_BEFORE_PER_ANCHOR_ISOLATION`] consecutive passes — and
    /// so should embed the residual ONE ANCHOR AT A TIME this pass to re-isolate the
    /// genuine poison the batched embed keeps misclassifying transient. `batch_ids`
    /// need not be sorted; it is compared against the stored sorted signature.
    fn is_stuck_on(&self, batch_ids: &[i64]) -> bool {
        match &self.transient_stuck {
            Some((prev_ids, streak)) if *streak >= MAX_TRANSIENT_PASSES_BEFORE_PER_ANCHOR_ISOLATION => {
                let mut sorted = batch_ids.to_vec();
                sorted.sort_unstable();
                *prev_ids == sorted
            }
            _ => false,
        }
    }

    /// Record one transient-only pass (a whole-batch embed fault: no store, no
    /// quarantine, no progress) over `batch_ids` and update the clustered-poison
    /// detector (F3). A pass that ends transient-only over the SAME set as last time
    /// bumps the streak; a different set (the window slid, so progress was made
    /// elsewhere) restarts it. The streak is read by [`is_stuck_on`] on the NEXT pass
    /// to decide per-anchor isolation; the detector is reset by
    /// [`clear_transient_stuck`] on any progress pass.
    fn record_transient_pass(&mut self, mut batch_ids: Vec<i64>) {
        batch_ids.sort_unstable();
        let streak = match self.transient_stuck.take() {
            Some((prev_ids, streak)) if prev_ids == batch_ids => streak.saturating_add(1),
            _ => 1,
        };
        self.transient_stuck = Some((batch_ids, streak));
    }

    /// Clear the clustered-poison detector (F3): any pass that stored, quarantined,
    /// or otherwise made progress means the window is no longer stuck, so the next
    /// transient-only pass starts a fresh streak rather than inheriting a stale one.
    fn clear_transient_stuck(&mut self) {
        self.transient_stuck = None;
    }

    /// F18: prune the L3 quarantine map against the live missing set, retaining only
    /// ids still peeked this pass. The bulk `anchor_failures.clear()` only fires when
    /// the backlog reaches empty, which continuous capture can prevent indefinitely;
    /// without this prune the sub-cap entries (an anchor that failed once or twice
    /// then stored, or whose id was retired by a reprocess that did not drain the
    /// whole backlog) would leak for the worker's whole uptime. Pruning each pass
    /// against the peeked ids keeps the map bounded by the live missing set rather
    /// than by uptime. (Quarantined ids that are still missing are retained — they
    /// must stay filtered out — so this never resurrects a poison pill.)
    fn prune_anchor_failures(&mut self, live_ids: &std::collections::HashSet<i64>) {
        self.anchor_failures.retain(|id, _| live_ids.contains(id));
    }

    /// F6: whether this work pass should run the logging-only backlog count, on the
    /// coarse [`BACKLOG_COUNT_EVERY_N_WORK_PASSES`] cadence. Bumps the work-pass
    /// counter and returns `true` once every N work passes, so a fast drain logs
    /// progress periodically without a per-pass full COUNT(*) on the capture pool.
    fn should_count_backlog_this_pass(&mut self) -> bool {
        self.work_passes_since_count = self.work_passes_since_count.saturating_add(1);
        if self.work_passes_since_count >= BACKLOG_COUNT_EVERY_N_WORK_PASSES {
            self.work_passes_since_count = 0;
            true
        } else {
            false
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

    // The debug surface's window onto this worker (`get_semantic_index_status`):
    // the sweep publishes a snapshot of its otherwise task-local health here after
    // every pass. Write-only from the worker's side — nothing here feeds back into
    // the sweep.
    let health: crate::debug_status::SemanticWorkerHealthState =
        app_handle.state::<crate::debug_status::SemanticWorkerHealthState>().inner().clone();

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
            *health.lock().unwrap_or_else(|poison| poison.into_inner()) = state.health_snapshot();

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
        state.clear_load_failures();
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
        // Caught up: forget every quarantine entry (the backlog is fully drained, so
        // no id is still missing) and the clustered-poison detector. This bulk clear
        // is the *empty-backlog* prune only — continuous capture can keep the backlog
        // non-empty for the whole uptime, so it is NOT a sufficient bound on the map
        // by itself (F18); the per-pass prune below keeps it bounded by the live
        // missing set whether or not the backlog ever reaches zero.
        state.anchor_failures.clear();
        state.clear_transient_stuck();
        return SweepPass::Idle;
    }

    // F18: prune the L3 quarantine map against the live peeked ids each pass, so a
    // sub-cap entry whose anchor stored or was reprocessed (its id retired) cannot
    // leak for the worker's whole uptime when continuous capture keeps the backlog
    // from ever reaching the empty-backlog bulk clear. Quarantined ids that are still
    // missing remain in the map (they must stay filtered out below), so this never
    // resurrects a poison pill. Bounded by the peek window, so this is a small set.
    let live_ids: std::collections::HashSet<i64> =
        raw_batch.iter().map(|anchor| anchor.anchor_id).collect();
    state.prune_anchor_failures(&live_ids);

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

    // F3: the anchor-id set this pass will attempt, used to drive the clustered-
    // poison detector AFTER the pass completes (a transient-only outcome over the
    // same set, repeated, means the window is stuck). When the detector says we have
    // been stuck on this exact set, embed it ONE ANCHOR AT A TIME so every genuine
    // poison anchor fails in isolation (and is credited toward quarantine), rather
    // than the batched embed misclassifying the whole cluster transient forever.
    let batch_ids: Vec<i64> = batch.iter().map(|anchor| anchor.anchor_id).collect();
    let per_anchor_isolation = state.is_stuck_on(&batch_ids);

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
        let out: Vec<(i64, std::result::Result<Vec<f32>, String>)> = if per_anchor_isolation {
            // F3 per-anchor isolation: the batched embed has kept misclassifying this
            // exact window transient over repeated passes (the >=2-anchor poison
            // cluster the whole-batch failure-count heuristic cannot pin to a single
            // anchor). Embed each text in its OWN `embed_texts` call so every genuine
            // poison anchor fails ALONE — the caller then credits each one a per-anchor
            // L3 failure (every failure is isolated by construction here), so the
            // cluster is quarantined and the window slides past it. Slower (one forward
            // per anchor) but only entered when the fast batched path is stuck.
            texts
                .iter()
                .map(|(anchor_id, body)| {
                    let mut result =
                        loaded.embedder.embed_texts(&[body.as_str()], EmbedKind::Document);
                    // `embed_texts` returns exactly one result per input, so a non-empty
                    // input yields one slot; defend against an unexpected empty return.
                    let single: std::result::Result<Vec<f32>, String> = match result.pop() {
                        Some(slot) => slot.map_err(|error| error.to_string()),
                        None => Err(
                            "embed_texts returned no result for a single input".to_string()
                        ),
                    };
                    (*anchor_id, single)
                })
                .collect()
        } else {
            // One batched candle call for the whole batch (vs one `embed_text` per
            // anchor): fewer total forward passes, so the backlog drains sooner.
            // `embed_texts` returns exactly one result per text, in order, with the
            // same overflow-split/single-passthrough/multi-mean-pool semantics as
            // `embed_text`. `bodies` borrows `texts`, so build `out` after it returns.
            let bodies: Vec<&str> = texts.iter().map(|(_, body)| body.as_str()).collect();
            let results = loaded.embedder.embed_texts(&bodies, EmbedKind::Document);
            texts
                .iter()
                .map(|(anchor_id, _)| *anchor_id)
                .zip(results)
                .map(|(anchor_id, result)| (anchor_id, result.map_err(|error| error.to_string())))
                .collect()
        };
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
                //
                // F3 panic-path: a `spawn_blocking` join `Err` is a candle panic (an
                // abort inside the forward), which bypasses the per-anchor result Vec
                // entirely — without crediting anything, a deterministically-panicking
                // input would re-peek the same window and re-panic every 30s forever,
                // never quarantined. So credit each anchor in the peeked batch a
                // per-anchor L3 failure: after `MAX_CONSECUTIVE_ANCHOR_FAILURES` such
                // panics the whole window is quarantined and the sweep slides past it.
                // (A panic cannot be localized to one anchor, so the whole batch is
                // charged; a genuinely-healthy anchor caught in a panicking batch is
                // retried on its next peek and its streak only advances on a repeat.)
                for anchor_id in &batch_ids {
                    state.record_anchor_embed_failure(*anchor_id);
                }
                // The detector is for transient (non-panic) whole-batch faults; a panic
                // is charged per-anchor above, so this is progress toward quarantine —
                // clear the stuck streak so it does not also trip per-anchor isolation.
                state.clear_transient_stuck();
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
            state.clear_load_failures();
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
            // The error string is also RETAINED on the state (not just logged) so the
            // debug surface can show why the model will not load.
            let load_failures = state.record_load_failure(&error);
            crate::native_capture::debug_log::log_error(format!(
                "semantic index backfill failed to load model '{}/{}' (consecutive load failures: {load_failures}): {error}",
                descriptor.provider, descriptor.model_id
            ));
            if load_failures >= MAX_CONSECUTIVE_LOAD_FAILURES {
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
    // F3: in per-anchor isolation mode each anchor was embedded in its OWN
    // `embed_texts` call, so a failure here IS isolated by construction — credit
    // every failure toward quarantine regardless of how many failed this pass. That
    // is the whole point of dropping to per-anchor embedding: it re-isolates the
    // genuine poison in a >=2-anchor cluster the batched failure-count heuristic
    // (`is_isolated_embed_failure`) keeps misclassifying transient. In the normal
    // batched path the heuristic still applies (one failure among healthy siblings =
    // poison; several together = transient whole-batch fault).
    let credit_failures_per_anchor =
        per_anchor_isolation || is_isolated_embed_failure(embed_failure_count);

    let mut stored = 0u64;
    // Transient errors (DB re-check / store failures, OR a whole-batch embed fault):
    // worth a 30s retry.
    let mut transient_errors = 0u64;
    // Deterministic embed failures (a candle error on this exact input):
    // counted toward per-anchor quarantine, NOT toward the transient retry loop.
    let mut embed_failures = 0u64;
    let mut quarantined = 0u64;
    let mut dimension_skips = 0u64;
    // Vectors that passed the per-anchor re-check, deferred so the whole batch is
    // written in ONE transaction after the loop (one writer-lock acquisition for the
    // batch instead of one per anchor — the churn that starved capture/finalize
    // writes and inflated stop latency).
    let mut to_store: Vec<(i64, Vec<f32>)> = Vec::new();
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
                    Ok(true) => {
                        // Defer the write: collect this anchor's vector and store the
                        // whole batch in one transaction after the loop (see
                        // `to_store`). The atomic row-conditioned INSERT remains the
                        // correctness boundary; this re-check was always just an
                        // early-out, so deferring the store does not weaken it.
                        to_store.push((anchor_id, vector));
                    }
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
                if !credit_failures_per_anchor {
                    // More than one anchor in this batch failed to embed in the NORMAL
                    // batched path. `embed_texts` already isolates a true poison input
                    // to its own text (a failing chunk fails only its own text, after a
                    // per-chunk retry), so multiple anchors failing TOGETHER is the
                    // signature of a transient whole-batch fault (e.g. a GPU OOM that
                    // recurs even at the single-chunk shape), NOT several independent
                    // poison pills. Treat it as transient — back off and retry the
                    // whole batch later — WITHOUT crediting any anchor a deterministic
                    // L3 failure, so a transient OOM never quarantines a window of
                    // healthy anchors. Do not bump the anchor's streak. (F3: when this
                    // stays stuck over the same window for repeated passes, the next
                    // pass drops to per-anchor isolation, where every failure IS
                    // credited — see `credit_failures_per_anchor`.)
                    transient_errors += 1;
                    crate::native_capture::debug_log::log_error(format!(
                        "semantic index backfill batch embed failure ({embed_failure_count} anchors in this batch); treating as transient (no quarantine); anchor {anchor_id} last error: {error}"
                    ));
                    continue;
                }
                // L3: a deterministic embed failure credited to this anchor — either it
                // failed IN ISOLATION in the batched path (the only anchor to fail,
                // siblings stored, so the fault is the input not the batch) OR we are in
                // F3 per-anchor isolation mode where each anchor was embedded alone so
                // its failure is isolated by construction. Bump its consecutive-failure
                // count; quarantine it once it hits the cap so it stops driving the
                // error loop and the window slides past it. This is NOT a transient
                // error.
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

    // Single-transaction write of every vector that passed its re-check. Folds the
    // per-anchor outcomes back into the same counters the per-anchor store updated:
    // `true` = stored (clear the failure streak), `false` = skipped (dimension
    // mismatch or the anchor vanished mid-embed — not a poison pill).
    if !to_store.is_empty() {
        match infra
            .semantic_search()
            .store_vectors_if_dimension_matches(&to_store)
            .await
        {
            Ok(outcomes) => {
                for ((anchor_id, _), was_stored) in to_store.iter().zip(outcomes) {
                    if was_stored {
                        stored += 1;
                        state.clear_anchor_failures(*anchor_id);
                    } else {
                        dimension_skips += 1;
                    }
                }
            }
            Err(error) => {
                // A real DB failure rolled the whole batch back: every anchor in it
                // retries (transient), matching the single-store `Err` arm.
                transient_errors += to_store.len() as u64;
                crate::native_capture::debug_log::log_error(format!(
                    "semantic index backfill failed to store {} vector(s) in batch: {error}",
                    to_store.len()
                ));
            }
        }
    }

    if stored > 0 {
        // F3: a store is progress, so the window slid — clear the clustered-poison
        // detector (a later transient-only stall starts a fresh streak, not a stale
        // one inherited from before the drain resumed).
        state.clear_transient_stuck();
        // F6: the backlog count is a logging-only full COUNT(*) on the shared capture
        // pool. Running it every work pass during a fast drain (a work pass can be ~1s)
        // contends with capture, so gate it behind a coarse cadence — log it only once
        // every N work passes. The peek that drives the actual work stays per-pass.
        let backlog = if state.should_count_backlog_this_pass() {
            // Best-effort: a count failure never affects the sweep. Render the
            // outcome as a human-readable token rather than a negative sentinel so
            // the log line is self-documenting (no magic `-1`/`-2` to decode).
            match infra.semantic_search().count_anchors_missing_vector().await {
                Ok(remaining) => remaining.to_string(),
                Err(_) => "count-failed".to_string(),
            }
        } else {
            // Throttled to the coarse cadence above — not counted on this pass.
            "not-counted".to_string()
        };
        crate::native_capture::debug_log::log_info(format!(
            "semantic index backfill embedded {stored} anchor(s) (batch={}, embed_failures={embed_failures}, transient_errors={transient_errors}, backlog={backlog})",
            batch.len()
        ));
        // CPU pacing: scale the inter-batch cooldown off this batch's wall time and
        // clamp it, then loop. This replaces the old 0ms yield so a large backfill
        // paces itself instead of sustaining a back-to-back multi-core burn.
        return SweepPass::DidWork(backfill_batch_cooldown(embed_started_at.elapsed()));
    }
    if dimension_skips > 0 {
        // Every stored anchor was skipped on a dimension mismatch (a switch in
        // flight, or a table stuck at the old dimension after a failed rebuild).
        // Idle — not Error — so the worker does not burn a 30s retry loop; startup
        // reconciliation (or a successful rebuild) restores agreement. A dimension
        // skip is not a poison-cluster stall, so reset the detector.
        state.clear_transient_stuck();
        crate::native_capture::debug_log::log_info(format!(
            "semantic index backfill idled: {dimension_skips} anchor(s) skipped on a vector-dimension mismatch with the live index (awaiting re-index)"
        ));
        return SweepPass::Idle;
    }
    if quarantined > 0 {
        // At least one anchor was newly quarantined this pass: the window is being
        // resolved (a poison pill removed from future peeks), which is progress. This
        // is the F3 success arm: per-anchor isolation re-isolated the cluster and
        // quarantined an anchor, so the next peek slides past it. Reset the detector
        // UNLESS this was an isolation pass that still has OTHER un-quarantined poison
        // (`embed_failures > quarantined`): keep isolating those next pass rather than
        // bouncing back to the batched path that re-stalls on them.
        if !(per_anchor_isolation && embed_failures > quarantined) {
            state.clear_transient_stuck();
        }
        if embed_failures == quarantined {
            // The only non-stored outcome this pass was newly-quarantined poison-pills
            // (nothing left to retry sooner). Idle so the worker does not spin the 30s
            // error loop on anchors it has just decided to skip.
            return SweepPass::Idle;
        }
        // Some anchors quarantined but others are still under the cap: retry the rest.
        return SweepPass::Error;
    }
    if embed_failures > 0 {
        // Embed failures still under the quarantine cap (each credited per-anchor):
        // retry them after a backoff.
        if !per_anchor_isolation {
            // A normal-path isolated single failure (one poison among healthy
            // siblings) is progress toward quarantine, not a transient stall — reset
            // the detector. But when this pass WAS per-anchor isolation (F3), keep the
            // detector set so the NEXT pass re-isolates the same window again and the
            // cluster keeps accruing toward quarantine instead of bouncing back to the
            // batched path that misclassifies it transient. The streak already exceeds
            // the isolation threshold, so leaving it set keeps `is_stuck_on` true; once
            // ALL poison is quarantined the `quarantined > 0` arm above clears it.
            state.clear_transient_stuck();
        }
        return SweepPass::Error;
    }
    if transient_errors > 0 {
        // A whole-batch embed fault or a real DB hiccup with no per-anchor progress:
        // back off and retry the same batch. F3: record this transient-only pass in
        // the clustered-poison detector. If the worker keeps stalling on the SAME
        // anchor-id window (the signature of a >=2-anchor poison cluster the batched
        // failure-count heuristic misclassifies transient), the next pass drops to
        // per-anchor isolation to re-isolate and quarantine the genuine poison so the
        // window finally slides. A single transient blip clears on its own next pass
        // (a different/empty set restarts the streak) and never reaches isolation.
        state.record_transient_pass(batch_ids);
        return SweepPass::Error;
    }
    // The batch was non-empty but every anchor was skipped by the re-check (all
    // deleted/reprocessed mid-embed): treat as work so we loop and pick up the
    // replacements (with a minimal pace), but it is effectively idle if nothing
    // remains. The window changed, so reset the detector.
    state.clear_transient_stuck();
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

/// Advance the idle-drop decay for one non-work pass (an `Idle` or — since F3 — an
/// `Error`) and release the embedder if the grace period has been reached, logging
/// once. `reason` ("idle" / "error") only colors the log line. This is the shared
/// "decay to idle" handler for both non-work arms of the loop: a caught-up worker
/// (Idle) AND a worker stuck in a persistent error loop (Error) both release the
/// large model weights to the OS after the grace period instead of pinning them
/// resident — the F3 fix that the Error arm previously skipped, leaving the weights
/// resident for the whole spin.
fn maybe_release_embedder_on_idle_decay(state: &mut SweepState, reason: &str) {
    if advance_idle_drop(
        &mut state.consecutive_idles,
        false,
        state.embedder.is_some(),
    ) {
        state.embedder = None;
        crate::native_capture::debug_log::log_info(format!(
            "semantic index backfill released the embedder after {} {reason} passes (model weights returned to the OS; reloads on next anchor)",
            state.consecutive_idles
        ));
    }
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
    fn health_snapshot_records_a_load_failure_and_clears_it_on_a_successful_pass() {
        // The debug surface's whole view of this worker is this snapshot, so the two
        // transitions it must survive are: a load failure (streak + the error string
        // that used to be logged and dropped) and the recovery that clears them.
        let mut state = SweepState::new();

        // Nothing has happened yet: no model held, nothing failed, nothing quarantined.
        let fresh = state.health_snapshot();
        assert!(!fresh.model_loaded);
        assert_eq!(fresh.consecutive_load_failures, 0);
        assert_eq!(fresh.quarantined_count, 0);
        assert_eq!(fresh.last_load_error, None);

        // Two consecutive load failures: the streak climbs and the LATEST error is the
        // one surfaced.
        assert_eq!(state.record_load_failure("weights are truncated"), 1);
        assert_eq!(state.record_load_failure("device init failed"), 2);
        let failed = state.health_snapshot();
        assert_eq!(failed.consecutive_load_failures, 2);
        assert_eq!(failed.last_load_error.as_deref(), Some("device init failed"));
        assert!(!failed.model_loaded, "a failed load holds no embedder");

        // A successful pass (the `Ok(pair)` arm) clears the streak AND the stale error,
        // so the debug page stops showing a load error the worker has recovered from.
        state.clear_load_failures();
        let recovered = state.health_snapshot();
        assert_eq!(recovered.consecutive_load_failures, 0);
        assert_eq!(recovered.last_load_error, None);
        assert_eq!(state.corrupt_model_signalled, None, "the CT3 latch clears too");
    }

    #[test]
    fn health_snapshot_counts_only_quarantined_anchors_not_sub_cap_streaks() {
        // `anchor_failures` holds every failing anchor's streak, but only those AT the
        // cap are actually excluded from the sweep — the snapshot must report the
        // quarantine, not the raw map size, or the debug page overstates the damage.
        let mut state = SweepState::new();

        // One anchor short of the cap: failing, but still being retried.
        state.record_anchor_embed_failure(1);
        assert_eq!(
            state.health_snapshot().quarantined_count,
            0,
            "a sub-cap failure streak is not a quarantine"
        );

        // A second anchor all the way to the cap: quarantined.
        for _ in 0..MAX_CONSECUTIVE_ANCHOR_FAILURES {
            state.record_anchor_embed_failure(2);
        }
        assert_eq!(state.health_snapshot().quarantined_count, 1);

        // A clean store on the quarantined id releases it from the count.
        state.clear_anchor_failures(2);
        assert_eq!(state.health_snapshot().quarantined_count, 0);
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

    #[test]
    fn clustered_poison_trips_per_anchor_isolation_after_repeated_transient_passes() {
        // F3: a >=2-anchor poison cluster sharing the newest-first window fails the
        // batched embed as a whole-batch (transient) fault every pass, so it never
        // shrinks. After a bounded number of transient-only passes over the SAME set,
        // the detector must flip the next pass to per-anchor isolation so each poison
        // anchor fails alone and can be credited toward quarantine.
        let mut state = SweepState::new();
        let window = vec![10i64, 11, 12]; // a clustered window (order should not matter)

        // The first transient passes stay on the batched path (not yet stuck).
        for _ in 0..MAX_TRANSIENT_PASSES_BEFORE_PER_ANCHOR_ISOLATION {
            assert!(
                !state.is_stuck_on(&window),
                "must not isolate before the transient-pass threshold"
            );
            state.record_transient_pass(window.clone());
        }
        // Now stuck on this exact window => isolate next pass (order-insensitive).
        assert!(state.is_stuck_on(&window), "stuck after the threshold");
        let reordered = vec![12i64, 10, 11];
        assert!(
            state.is_stuck_on(&reordered),
            "the signature is the SET, not the order"
        );
    }

    #[test]
    fn a_different_transient_window_restarts_the_isolation_streak() {
        // A single transient blip (or a window that slid because progress was made
        // elsewhere) must NOT accrue toward isolation: a different set restarts the
        // streak, so only a genuinely STUCK window ever reaches per-anchor isolation.
        let mut state = SweepState::new();

        for _ in 0..MAX_TRANSIENT_PASSES_BEFORE_PER_ANCHOR_ISOLATION {
            state.record_transient_pass(vec![1, 2, 3]);
        }
        assert!(state.is_stuck_on(&[1, 2, 3]), "stuck on the original window");

        // The window changes (the backlog slid): the streak restarts from this set.
        state.record_transient_pass(vec![4, 5, 6]);
        assert!(
            !state.is_stuck_on(&[4, 5, 6]),
            "a fresh window is not immediately stuck"
        );
        assert!(
            !state.is_stuck_on(&[1, 2, 3]),
            "the old window is no longer the tracked signature"
        );
    }

    #[test]
    fn clearing_the_detector_after_progress_resets_isolation() {
        // Any progress pass (store / quarantine) clears the detector, so a later
        // transient stall starts a fresh streak rather than inheriting a stale one.
        let mut state = SweepState::new();
        for _ in 0..MAX_TRANSIENT_PASSES_BEFORE_PER_ANCHOR_ISOLATION {
            state.record_transient_pass(vec![7, 8]);
        }
        assert!(state.is_stuck_on(&[7, 8]));

        state.clear_transient_stuck();
        assert!(!state.is_stuck_on(&[7, 8]), "progress resets the isolation streak");
    }

    #[test]
    fn switching_models_clears_the_per_anchor_quarantine_map() {
        // F4: a quarantine recorded under model A is model-specific (a
        // tokenizer-incompatible input under A may embed fine under B). On a model
        // switch the vec0 table is rebuilt and every anchor re-enters the missing set,
        // so a stale quarantine from A would wrongly filter that anchor from every
        // batch under B until restart. Reconciling to a different selection must clear
        // the map (and the clustered-poison detector, and the idle streak).
        let mut state = SweepState::new();
        let provider = "mnema";
        let model_a = "model-a";
        let model_b = "model-b";

        // Quarantine an anchor under model A and arrive at a stuck/idle posture.
        for _ in 0..MAX_CONSECUTIVE_ANCHOR_FAILURES {
            state.record_anchor_embed_failure(999);
        }
        assert!(state.is_anchor_quarantined(999), "quarantined under model A");
        state.record_transient_pass(vec![1, 2]);
        state.consecutive_idles = IDLE_PASSES_BEFORE_EMBEDDER_DROP;
        // The latch must be set for the different-selection branch to run.
        state.corrupt_model_signalled = Some((provider.to_string(), model_a.to_string()));

        // Switch to model B.
        state.reconcile_selection(provider, model_b);
        assert!(
            !state.is_anchor_quarantined(999),
            "the quarantine map is cleared on a model switch so B retries every anchor"
        );
        assert!(
            !state.is_stuck_on(&[1, 2]),
            "the clustered-poison detector is cleared on a model switch"
        );
        assert_eq!(
            state.consecutive_idles, 0,
            "the idle streak resets on a model switch"
        );
    }

    #[test]
    fn a_clean_model_switch_without_a_corrupt_latch_still_clears_the_quarantine_map() {
        // F4 (the common path): the per-anchor quarantine map must be cleared on ANY
        // model switch, not only one where a corrupt-model latch happened to be set.
        // `reconcile_selection` keys off the tracked previous selection, so a clean
        // switch (no latch ever raised) is detected and the model-specific state is
        // cleared for the new model.
        let mut state = SweepState::new();
        let provider = "mnema";

        // Prime the previous selection by reconciling once to model A (first pass: no
        // switch detected, no state touched).
        state.reconcile_selection(provider, "model-a");
        for _ in 0..MAX_CONSECUTIVE_ANCHOR_FAILURES {
            state.record_anchor_embed_failure(321);
        }
        assert!(state.is_anchor_quarantined(321), "quarantined under model A");
        assert!(
            state.corrupt_model_signalled.is_none(),
            "no corrupt latch in this scenario"
        );

        // A clean switch to model B (still no latch) must clear A's quarantine.
        state.reconcile_selection(provider, "model-b");
        assert!(
            !state.is_anchor_quarantined(321),
            "a clean model switch clears the quarantine map keyed to the old model"
        );

        // Reconciling to the SAME model B again is a no-op (no spurious clear).
        for _ in 0..MAX_CONSECUTIVE_ANCHOR_FAILURES {
            state.record_anchor_embed_failure(654);
        }
        state.reconcile_selection(provider, "model-b");
        assert!(
            state.is_anchor_quarantined(654),
            "an unchanged selection preserves the quarantine map"
        );
    }

    #[test]
    fn reconcile_to_the_same_model_keeps_the_quarantine_map() {
        // The same-selection branch is a no-op: reconciling to the SAME model must NOT
        // wipe the quarantine map (the model has not changed, so the failures still
        // apply). Only a genuine switch clears it.
        let mut state = SweepState::new();
        let provider = "mnema";
        let model = "model-a";
        for _ in 0..MAX_CONSECUTIVE_ANCHOR_FAILURES {
            state.record_anchor_embed_failure(5);
        }
        state.corrupt_model_signalled = Some((provider.to_string(), model.to_string()));

        state.reconcile_selection(provider, model);
        assert!(
            state.is_anchor_quarantined(5),
            "reconciling to the same model preserves its quarantine map"
        );
    }

    #[test]
    fn prune_anchor_failures_drops_entries_absent_from_the_live_missing_set() {
        // F18: the per-pass prune keeps the quarantine map bounded by the live missing
        // set even when continuous capture never lets the backlog reach the empty-
        // backlog bulk clear. An entry whose id is no longer peeked (it stored, or its
        // id was retired by a reprocess) is dropped; an id still peeked is retained.
        let mut state = SweepState::new();
        state.record_anchor_embed_failure(1); // still missing this pass
        state.record_anchor_embed_failure(2); // gone (stored / reprocessed)
        for _ in 0..MAX_CONSECUTIVE_ANCHOR_FAILURES {
            state.record_anchor_embed_failure(3); // quarantined AND still missing
        }

        let live: std::collections::HashSet<i64> = [1i64, 3].into_iter().collect();
        state.prune_anchor_failures(&live);

        assert!(state.anchor_failures.contains_key(&1), "a still-missing id is kept");
        assert!(
            !state.anchor_failures.contains_key(&2),
            "an id no longer in the missing set is pruned (no uptime-scaled leak)"
        );
        assert!(
            state.is_anchor_quarantined(3),
            "a still-missing quarantined id is retained so it stays filtered out"
        );
    }

    #[test]
    fn backlog_count_runs_only_on_the_coarse_cadence() {
        // F6: the logging-only backlog count must not run every work pass (it is a full
        // COUNT(*) on the shared capture pool). It fires once every N work passes.
        let mut state = SweepState::new();
        let mut counted = 0u32;
        // Run several full cadences worth of work passes.
        let passes = BACKLOG_COUNT_EVERY_N_WORK_PASSES * 3;
        for _ in 0..passes {
            if state.should_count_backlog_this_pass() {
                counted += 1;
            }
        }
        assert_eq!(
            counted, 3,
            "exactly one count per BACKLOG_COUNT_EVERY_N_WORK_PASSES work passes"
        );
    }

    #[test]
    fn error_passes_decay_the_embedder_via_the_idle_drop() {
        // F3 (part b): a persistent error loop must release the model weights after the
        // grace period instead of pinning them resident — the Error arm advances the
        // same idle-drop decay as Idle. `advance_idle_drop` is the shared mechanism;
        // an Error pass calls it with `did_work = false`, so the streak grows and the
        // embedder is released exactly at the grace threshold.
        let mut idles = 0u32;
        for _ in 1..IDLE_PASSES_BEFORE_EMBEDDER_DROP {
            assert!(
                !advance_idle_drop(&mut idles, false, true),
                "an error pass under the grace threshold keeps the embedder warm"
            );
        }
        assert!(
            advance_idle_drop(&mut idles, false, true),
            "a sustained error loop releases the embedder at the grace threshold"
        );
    }
}
