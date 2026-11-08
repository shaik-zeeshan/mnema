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
//! `spawn_retention_cleanup_worker`: one `AppCore::spawn` loop (`spawn.rs`),
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
use std::sync::{Arc, Mutex, Weak};
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
use tokio::sync::watch;

use crate::debug_status::SemanticWorkerHealth;
use crate::semantic_search_models::{
    SemanticSearchModelDownloadProgressDto, SemanticSearchModelDownloadStatusDto,
};
use crate::{AppCore, AppInfraState, CoreEvent};

mod embedder;
mod spawn;
mod sweep;
#[cfg(test)]
mod tests;

pub use embedder::{load_embedder, load_embedder_private, LoadedEmbedder};
pub use sweep::run_sweep_pass;
pub use spawn::spawn_semantic_index_backfill_worker;

/// How many anchors to drain per batch. A bounded batch keeps the worker
/// responsive to shutdown between batches and caps the blocking-thread hop cost,
/// while still amortizing the per-batch DB round-trips.
pub const SWEEP_BATCH_SIZE: i64 = 16;

/// Idle poll interval when there is nothing to embed (caught up, or the model is
/// not installed). Kept modest so the worker notices freshly captured anchors and
/// a just-installed model promptly, but it does no work on these ticks.
pub const IDLE_POLL_INTERVAL: Duration = Duration::from_secs(20);

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
/// costs a model read + device init. At [`IDLE_POLL_INTERVAL`] = 20 s, 30 passes
/// is ~10 min of being caught up before the weights are released; the next anchor
/// pays one reload. This is the cheap, in-process approximation of a sidecar (a
/// process that exits when drained returns *everything*, including the base
/// runtime — this only frees the model).
const IDLE_PASSES_BEFORE_EMBEDDER_DROP: u32 = 30;

/// Maximum time a loaded embedder stays warm across *consecutive working* passes
/// before it is recycled (dropped and reloaded on the next batch).
///
/// [`IDLE_PASSES_BEFORE_EMBEDDER_DROP`] only fires when the backlog is *drained*,
/// so a long historical backfill keeps `did_work = true` for hours and never
/// releases. That matters because of what the embedder holds on macOS: candle
/// 0.10.2's Metal backend pools every buffer it allocates in
/// `MetalDevice::{buffers, private_buffers}`, and its only reclaim path
/// (`drop_unused_buffers`) sweeps `buffers` alone — `private_buffers`, which the
/// StorageModePrivate intermediate allocator (`new_buffer`) fills on every forward
/// pass, is never swept. So GPU memory only comes back when the whole `Device`
/// drops. Measured on a 10h-old session: 574 MB of unmapped-graphics footprint
/// returned the instant this worker released its embedder; the process peaked at
/// 2.0 GB with embedders warm. Upstream: huggingface/candle#3464, #2271
/// (PR #3755 open, unmerged).
///
/// Recycling on a wall-clock bound turns that unbounded growth into a sawtooth
/// with a known ceiling, for the price of one model reload per interval.
///
/// ponytail: a wall-clock bound, not a per-batch counter — pool growth tracks
/// forward passes, and batch wall time already varies with the CPU-pacing
/// cooldown. Upgrade path: if candle ever sweeps `private_buffers`, delete this
/// and the recycle check that reads it.
pub const MAX_EMBEDDER_WARM_LIFETIME: Duration = Duration::from_secs(3600);

/// Backoff after a batch error (a DB hiccup or an embed failure). Embedding never
/// blocks capture, so a failure just retries later rather than surfacing.
pub const ERROR_RETRY_INTERVAL: Duration = Duration::from_secs(30);

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
pub const MAX_CONSECUTIVE_LOAD_FAILURES: u32 = 3;

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
/// The multiplier is what actually sets the steady-state duty cycle:
/// `duty = 1 / (1 + multiplier)`. At 3.0 the worker embeds ~25% of the time.
///
/// It was 1.0 (a 50% target) — but measurement showed the multiplier was inert in
/// practice, because a real batch of 16 anchors takes ~3.5s while
/// [`BACKFILL_BATCH_COOLDOWN_MAX`] was 2s, so the CAP bound on every single pass
/// and the multiplier never applied. Two consequences, both measured on an M4 Air
/// against real capture text: the GPU sat at 65% duty / 7.4W sustained, and — worse
/// — duty *rose* when the GPU slowed (to 86% under Low Power Mode), because a
/// longer batch still got the same fixed 2s rest. A throttled machine was made to
/// press HARDER.
///
/// With the cap raised out of the way (below) the multiplier governs again and the
/// duty cycle is **self-correcting**: a slower forward earns proportionally more
/// rest, so thermal throttling backs the worker off instead of amplifying it. This
/// is why the worker deliberately does NOT read `NSProcessInfo.thermalState` or the
/// power source — proportional pacing already responds to a throttled GPU, and a
/// thermal-state reader would duplicate it.
const BACKFILL_BATCH_COOLDOWN_MULTIPLIER: f64 = 3.0;
/// Lower bound of the inter-batch cooldown — a real yield even for a trivially
/// fast batch (the old 0ms gave none), so the sweep never busy-loops the cores.
const BACKFILL_BATCH_COOLDOWN_MIN: Duration = Duration::from_millis(150);
/// Safety bound only — NOT the governor. It exists so a pathologically slow batch
/// cannot park the sweep for minutes; it must stay well above
/// `multiplier * typical_batch` (~10.5s today) or it silently becomes the governor
/// again and reintroduces the throttle-amplifying bug described above.
const BACKFILL_BATCH_COOLDOWN_MAX: Duration = Duration::from_secs(30);

/// Serialises the two BACKGROUND embed workers — this one and
/// `user_context::subject_vector_worker` — onto one GPU slot, so they never run
/// forwards concurrently.
///
/// Each worker holds its own embedder and paces itself independently, with nothing
/// coordinating them. Measured on an M4 Air against real capture text, two
/// concurrent workers cost **+38% GPU power (7.4W → 10.2W) and +9°C** for only +45%
/// throughput — and 10.2W is 86% of the device's saturation ceiling, i.e. two
/// background embedders effectively saturate the GPU. Serialising them removes that
/// worst case; the pacing cooldowns then compose instead of stacking.
///
/// The QUERY path (`semantic_search_query.rs`) deliberately does NOT take this
/// permit: it is user-facing and latency-sensitive, and making a search wait behind
/// a backfill batch (~3.5s) to save background watts is the wrong trade. Only the
/// two background sweeps share the slot.
pub static BACKGROUND_EMBED_SLOT: tokio::sync::Semaphore =
    tokio::sync::Semaphore::const_new(1);

/// The outcome of one sweep pass, deciding the loop's next sleep.
pub enum SweepPass {
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
pub struct SweepState {
    /// The loaded **Semantic Search Model**, reused across passes. `None` until
    /// the first pass that needs it with an installed model. An `Arc` because the
    /// instance is shared with the other holders via [`SHARED_EMBEDDER`]; holding
    /// it here is what keeps the weights resident, and clearing it is this
    /// worker's half of freeing them.
    pub embedder: Option<Arc<LoadedEmbedder>>,
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
    pub consecutive_idles: u32,
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
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
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
    pub fn health_snapshot(&self) -> SemanticWorkerHealth {
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

/// The effective **Semantic Search** selection for the worker and the query
/// embedder: the user's `RecordingSettings.semantic_search`. Falls back to the
/// default-on English-tier selection when the settings state is not yet managed
/// (early startup) so historical capture still gets vectored once a model is
/// installed.
pub fn effective_semantic_search_settings(
    core: &AppCore,
) -> SemanticSearchSettings {
    match core.try_state::<crate::recording_settings::RecordingSettingsState>() {
        Some(state) => {
            crate::recording_settings::current_recording_settings(&state).semantic_search
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
pub fn resolve_selected_descriptor(
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
pub fn selected_model_available(
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
    slot: &Option<Arc<LoadedEmbedder>>,
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
pub fn advance_idle_drop(consecutive_idles: &mut u32, did_work: bool, has_embedder: bool) -> bool {
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
pub fn maybe_release_embedder_on_idle_decay(state: &mut SweepState, reason: &str) {
    if advance_idle_drop(
        &mut state.consecutive_idles,
        false,
        state.embedder.is_some(),
    ) {
        state.embedder = None;
        crate::debug_log::log(format!(
            "semantic index backfill released the embedder after {} {reason} passes (model weights returned to the OS; reloads on next anchor)",
            state.consecutive_idles
        ));
    }
}
