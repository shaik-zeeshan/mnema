//! The **Subject Vector Backfill** worker (slice 4): a single self-healing
//! sweep-loop on the `mnema-deferred-startup` seam that embeds every distinct,
//! non-dismissed **User-Context Subject** into the `user_context_subject_vectors`
//! table (migration `0043`, slice 3), so slice 5's pre-retrieval candidate
//! selection has vectors to KNN against.
//!
//! It is the User-Context twin of [`crate::semantic_search_worker`]'s Semantic
//! Index Backfill, and reuses that module's model-gate / embedder-load helpers
//! (`effective_semantic_search_settings`, `selected_model_available`,
//! `resolve_selected_descriptor`, `load_embedder`, `LoadedEmbedder`) rather than
//! duplicating the gating logic. The two workers share ONE installed **Semantic
//! Search Model**: the subjects are embedded with the same model the search
//! anchors are, so a single KNN space spans both.
//!
//! Like the search backfill it is **default-on but model-gated**: with no model
//! installed it is a silent idle-poll no-op (never an error, never auto-download),
//! and it self-starts the day a model is installed — embedding the existing
//! subjects retroactively and any new subject as it appears. Progress lives
//! entirely in the DB (the presence/absence of a non-NULL vector row), so the
//! sweep is resumable across restarts with no in-memory cursor:
//! [`SubjectVectorStore::list_subjects_needing_embedding`] returns exactly the
//! distinct non-dismissed subjects still lacking a vector under the *active*
//! model — so a model switch (Settings) re-embeds every vector that was stored
//! under the old model rather than leaving it stranded.
//!
//! Statement enrichment: a bare handle like "Apple" is ambiguous, so the embed
//! text is the handle joined with its canonical statement (the highest-confidence
//! non-dismissed Conclusion, ties → lowest id) — `"{subject}: {statement}"` — so
//! the vector carries context. When the subject has no usable statement the bare
//! handle is embedded.
//!
//! Compute placement mirrors the search backfill: the candle embed is synchronous
//! model work (Metal GPU on macOS / candle-CPU elsewhere) that must never occupy
//! the tokio reactor, so it runs on a blocking thread; DB reads/writes stay on the
//! async loop. Each subject is embedded in its OWN `embed_text` call, so a poison
//! input fails in isolation and is credited toward a per-subject quarantine
//! directly (no whole-batch failure-count heuristic is needed here).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{
    future::{select, Either},
    pin_mut,
};
use semantic_search::EmbedKind;
use tauri::Manager;
use tokio::sync::watch;

use crate::app_infra::{shutdown_aware_sleep, AppInfraState, BackgroundWorkersState};
use crate::semantic_search_worker::{
    effective_semantic_search_settings, load_embedder, resolve_selected_descriptor,
    selected_model_available, LoadedEmbedder,
};

/// How many subjects to drain per pass. Bounded so the worker stays responsive to
/// shutdown between passes and caps the per-pass blocking-thread cost.
const SWEEP_BATCH_SIZE: i64 = 16;

/// Idle poll interval when there is nothing to embed (caught up, or no model is
/// installed). Modest so a freshly-installed model and freshly-derived subjects
/// are picked up promptly, but no work is done on these ticks.
const IDLE_POLL_INTERVAL: Duration = Duration::from_secs(30);

/// Backoff after a recoverable error this pass (a DB hiccup or an embed failure).
/// Embedding subjects never blocks any user-facing path, so a failure just retries
/// later rather than surfacing.
const ERROR_RETRY_INTERVAL: Duration = Duration::from_secs(30);

/// Consecutive idle passes (backlog drained) before the loaded embedder is dropped
/// to return its resident model weights to the OS — the "decay to idle" lever the
/// search backfill uses. A caught-up worker that keeps the embedder loaded pins
/// that memory floor forever; candle has no process-global arena to retain, so the
/// drop actually reclaims it and the next subject pays one reload.
const IDLE_PASSES_BEFORE_EMBEDDER_DROP: u32 = 3;

/// Maximum **consecutive** deterministic embed failures for a single Subject
/// before it is quarantined (excluded from the sweep until its content changes or
/// the worker restarts). Mirrors the search backfill's per-anchor cap and the User
/// Context `RetryPolicy.max_attempts = 3`: three genuine attempts on the same
/// poison input, then stop driving a 30s error loop on it forever. Per-subject and
/// in-memory by design — progress is otherwise DB-resident (a stored vector removes
/// the subject from the missing set), and a restart clears it for a fresh attempt.
const MAX_CONSECUTIVE_SUBJECT_FAILURES: u32 = 3;

/// Maximum **consecutive** embedder LOAD failures before the model is treated as
/// unavailable for the current selection and the worker stops the tight load-retry
/// loop, idling until the selection changes or a restart. A model marker says
/// "installed" on presence alone — it never validates the weights load — so a
/// truncated model would otherwise fail every retry forever. The search backfill
/// owns the user-facing "model appears corrupt — reinstall" telemetry for the same
/// shared model, so this twin worker only logs + idles rather than re-emitting it.
const MAX_CONSECUTIVE_LOAD_FAILURES: u32 = 3;

/// CPU pacing: a minimum inter-batch cooldown scaled off the just-finished batch's
/// wall time and clamped to a band, so a large retroactive backfill does not
/// sustain a back-to-back burn concurrent with capture/OCR. Mirrors the search
/// backfill's pacing band (kept under candle: on macOS the Metal forward leaves the
/// CPU idle, but on candle-CPU the forward is the burn this paces).
const BACKFILL_BATCH_COOLDOWN_MULTIPLIER: f64 = 1.0;
const BACKFILL_BATCH_COOLDOWN_MIN: Duration = Duration::from_millis(150);
const BACKFILL_BATCH_COOLDOWN_MAX: Duration = Duration::from_millis(2000);

/// The outcome of one sweep pass, deciding the loop's next sleep.
enum SweepPass {
    /// At least one subject was embedded + stored this pass. The carried `Duration`
    /// is the CPU-pacing cooldown before the next pass.
    DidWork(Duration),
    /// No subject needed a vector (caught up) OR no model is installed (silent
    /// no-op): sleep the idle interval.
    Idle,
    /// A recoverable error this pass: sleep the error-retry interval.
    Error,
    /// Shutdown was observed mid-pass (e.g. while a blocking embed was in flight):
    /// stop the loop now rather than waiting on in-flight work.
    Shutdown,
}

/// Mutable, in-memory worker state that outlives a single pass: the loaded
/// embedder plus the bounded-retry quarantine counters. All deliberately
/// non-persistent — see [`MAX_CONSECUTIVE_SUBJECT_FAILURES`].
#[derive(Default)]
struct SweepState {
    /// The loaded **Semantic Search Model**, reused across passes. `None` until the
    /// first pass that needs it with an installed model.
    embedder: Option<LoadedEmbedder>,
    /// Log the "no model installed" no-op only once per inert stretch.
    logged_no_model: bool,
    /// Per-subject **consecutive** deterministic-embed-failure counts. Keyed by the
    /// ASCII-folded subject (matching the table's `COLLATE NOCASE` key) so a poison
    /// subject is quarantined regardless of the casing it is peeked under.
    subject_failures: HashMap<String, u32>,
    /// Consecutive embedder LOAD failures for the current selection. Reset to 0 on a
    /// successful load or a model switch.
    consecutive_load_failures: u32,
    /// The `(provider, model_id)` whose load was given up on this stretch (after
    /// [`MAX_CONSECUTIVE_LOAD_FAILURES`] failures), so the worker idles quietly for
    /// THAT selection rather than re-attempting the doomed load every tick. Cleared
    /// when the model goes unavailable, a load later succeeds, or the user switches
    /// to a different selection.
    load_failed_latch: Option<(String, String)>,
    /// Consecutive idle passes since the last pass that embedded something. Drives
    /// the idle-drop ([`IDLE_PASSES_BEFORE_EMBEDDER_DROP`]).
    consecutive_idles: u32,
    /// The `(provider, model_id)` the previous pass reconciled against, so a model
    /// switch can be detected and the per-model state (quarantine map, load latch,
    /// cached embedder) cleared for the new model. `None` until the first pass.
    last_selection: Option<(String, String)>,
}

impl SweepState {
    /// Fold a subject to its quarantine-map key. ASCII-only to match SQLite
    /// `COLLATE NOCASE` (full Unicode `to_lowercase` would over-fold).
    fn quarantine_key(subject: &str) -> String {
        subject.to_ascii_lowercase()
    }

    fn is_subject_quarantined(&self, subject: &str) -> bool {
        self.subject_failures
            .get(&Self::quarantine_key(subject))
            .is_some_and(|&failures| failures >= MAX_CONSECUTIVE_SUBJECT_FAILURES)
    }

    /// Record one deterministic embed failure for `subject`, returning the new
    /// consecutive-failure count and whether it has now reached the quarantine cap.
    fn record_subject_failure(&mut self, subject: &str) -> (u32, bool) {
        let failures = self
            .subject_failures
            .entry(Self::quarantine_key(subject))
            .and_modify(|count| *count = count.saturating_add(1))
            .or_insert(1);
        (*failures, *failures >= MAX_CONSECUTIVE_SUBJECT_FAILURES)
    }

    fn clear_subject_failures(&mut self, subject: &str) {
        self.subject_failures.remove(&Self::quarantine_key(subject));
    }

    /// Whether the currently-selected `(provider, model_id)` has been given up on
    /// for loading this stretch (so the worker idles instead of load-looping it).
    fn load_latch_matches(&self, provider: &str, model_id: &str) -> bool {
        self.load_failed_latch
            .as_ref()
            .is_some_and(|(p, m)| p == provider && m == model_id)
    }

    /// Reconcile all model-keyed in-memory state against the currently-selected
    /// `(provider, model_id)` at the top of a pass. On ANY model switch (detected
    /// via `last_selection`), clear the per-model state so the new model starts
    /// fresh: the load latch + load-failure counter, the cached embedder (it is for
    /// the old model), the per-subject quarantine map (a tokenizer-incompatible
    /// input under model A may embed fine under B), and the idle-drop streak. A
    /// no-op when the selection is unchanged.
    fn reconcile_selection(&mut self, provider: &str, model_id: &str) {
        let selection = (provider.to_string(), model_id.to_string());
        let switched_from_last = self
            .last_selection
            .as_ref()
            .is_some_and(|previous| previous != &selection);
        let latch_names_other_model = matches!(
            &self.load_failed_latch,
            Some((p, m)) if (p.as_str(), m.as_str()) != (provider, model_id)
        );
        let switched = switched_from_last || latch_names_other_model;
        self.last_selection = Some(selection);
        if !switched {
            return;
        }
        self.load_failed_latch = None;
        self.consecutive_load_failures = 0;
        self.embedder = None;
        self.subject_failures.clear();
        self.consecutive_idles = 0;
    }

    /// Whether the cached embedder is already for `provider`/`model_id` (so a
    /// Settings model switch reloads it).
    fn embedder_matches(&self, provider: &str, model_id: &str) -> bool {
        self.embedder
            .as_ref()
            .is_some_and(|loaded| loaded.provider == provider && loaded.model_id == model_id)
    }
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
    crate::native_capture::debug_log::log_info(format!(
        "starting user-context subject vector worker (batch={}, idle_poll_ms={}, error_retry_ms={})",
        SWEEP_BATCH_SIZE,
        IDLE_POLL_INTERVAL.as_millis(),
        ERROR_RETRY_INTERVAL.as_millis(),
    ));

    let handle = tauri::async_runtime::spawn(async move {
        let infra = Arc::clone(&infra);
        let mut state = SweepState::default();

        loop {
            if *shutdown_rx.borrow() {
                break;
            }

            let pass = run_sweep_pass(&infra, &app_handle, &mut state, &mut shutdown_rx).await;
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

        crate::native_capture::debug_log::log_info("stopped user-context subject vector worker");
    });
    background_workers.track(handle);
}

/// Run one sweep pass: gate on the installed model, drain up to one batch of
/// subjects (skipping quarantined poison), embed each on a blocking thread with its
/// canonical-statement-enriched text, and upsert the vectors. Never panics; any
/// error is logged and turned into [`SweepPass::Error`].
async fn run_sweep_pass(
    infra: &AppInfraState,
    app_handle: &tauri::AppHandle,
    state: &mut SweepState,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> SweepPass {
    let settings = effective_semantic_search_settings(app_handle);

    let app_data_dir = match app_handle.path().app_data_dir() {
        Ok(dir) => dir,
        Err(error) => {
            crate::native_capture::debug_log::log_error(format!(
                "subject vector backfill could not resolve app data dir: {error}"
            ));
            return SweepPass::Error;
        }
    };

    // Model-gating: silent no-op when no model is installed (matches the search
    // backfill). Drop any cached embedder and reset the load latch so a fresh
    // (re)install loads cleanly.
    if !selected_model_available(&app_data_dir, &settings) {
        state.embedder = None;
        state.consecutive_load_failures = 0;
        state.load_failed_latch = None;
        if !state.logged_no_model {
            crate::native_capture::debug_log::log_info(
                "subject vector backfill skipped: no Semantic Search Model installed (silent no-op)",
            );
            state.logged_no_model = true;
        }
        return SweepPass::Idle;
    }
    state.logged_no_model = false;

    let Some(descriptor) = resolve_selected_descriptor(&settings) else {
        state.embedder = None;
        return SweepPass::Idle;
    };

    // Reconcile per-model in-memory state against the current selection; a switch
    // clears the load latch, the quarantine map, and the stale cached embedder.
    state.reconcile_selection(&descriptor.provider, &descriptor.model_id);

    // The active model's identity string (`provider/model_id`): keys both the
    // "needs embedding" backlog query (so stale-model vectors are re-claimed) and
    // each upsert's `embedded_model` stamp.
    let active_model = format!("{}/{}", descriptor.provider, descriptor.model_id);

    // If the current selection has been given up on for loading this stretch, idle
    // quietly rather than re-attempting the doomed load every tick.
    if state.load_latch_matches(&descriptor.provider, &descriptor.model_id) {
        return SweepPass::Idle;
    }

    // Peek the backlog before paying for an embedder load: nothing to embed → idle
    // without touching the model.
    let subjects = match infra
        .subject_vectors()
        .list_subjects_needing_embedding(&active_model, SWEEP_BATCH_SIZE)
        .await
    {
        Ok(subjects) => subjects,
        Err(error) => {
            crate::native_capture::debug_log::log_error(format!(
                "subject vector backfill failed to list subjects without a vector: {error}"
            ));
            return SweepPass::Error;
        }
    };
    if subjects.is_empty() {
        // Caught up: forget every quarantine entry (no subject is still missing).
        state.subject_failures.clear();
        return SweepPass::Idle;
    }

    // Quarantine: drop subjects that have already failed deterministically. The
    // backlog drains around them; a restart or a content change retries them.
    let pending: Vec<String> = subjects
        .into_iter()
        .filter(|subject| !state.is_subject_quarantined(subject))
        .collect();
    if pending.is_empty() {
        return SweepPass::Idle;
    }

    // Build the embedding text per subject (handle + canonical statement) on the
    // async loop, BEFORE the blocking embed. A per-subject read failure is treated
    // as transient for that subject; it is simply omitted from this pass and retried
    // next pass (it is still missing a vector).
    let mut texts: Vec<(String, String)> = Vec::with_capacity(pending.len());
    for subject in pending {
        match infra
            .user_context()
            .canonical_statement_for_subject(&subject)
            .await
        {
            Ok(statement) => {
                let text = compose_embed_text(&subject, statement.as_deref());
                texts.push((subject, text));
            }
            Err(error) => {
                crate::native_capture::debug_log::log_error(format!(
                    "subject vector backfill failed to read canonical statement for '{subject}': {error}"
                ));
            }
        }
    }
    if texts.is_empty() {
        // Every subject's statement read failed this pass: transient, retry later.
        return SweepPass::Error;
    }

    // Embed on a blocking thread: BOTH the (conditional) embedder load AND the
    // candle forward are synchronous model work that must stay off the tokio
    // reactor. Each subject is embedded in its OWN `embed_text` call so a poison
    // input fails in isolation. The cached embedder is moved into the task and back
    // out so it survives across passes.
    let cached = if state.embedder_matches(&descriptor.provider, &descriptor.model_id) {
        state.embedder.take()
    } else {
        state.embedder = None;
        None
    };

    let embed_started_at = Instant::now();
    let app_data_dir_for_task = app_data_dir.clone();
    let descriptor_for_task = descriptor.clone();
    let texts_for_task = texts.clone();
    // Race the blocking load+embed against the shutdown watch so a quit mid-batch
    // does not wait on a full batch of model work. If shutdown wins, the task is
    // abandoned (it only computes in memory — no partial DB state) and the worker
    // stops.
    let embed_task = tauri::async_runtime::spawn_blocking(move || {
        let loaded = match cached {
            Some(loaded) => loaded,
            None => match load_embedder(&app_data_dir_for_task, &descriptor_for_task) {
                Ok(loaded) => loaded,
                Err(error) => return Err(error),
            },
        };
        let out: Vec<(String, std::result::Result<Vec<f32>, String>)> = texts_for_task
            .iter()
            .map(|(subject, text)| {
                let result = loaded
                    .embedder
                    .embed_text(text, EmbedKind::Document)
                    .map_err(|error| error.to_string());
                (subject.clone(), result)
            })
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
                    "subject vector backfill embed task panicked/cancelled: {error}"
                ));
                // A `spawn_blocking` join `Err` is a candle panic that bypasses the
                // per-subject result vec; credit each subject in the batch a failure so
                // a deterministically-panicking input is eventually quarantined instead
                // of re-panicking every 30s forever.
                for (subject, _) in &texts {
                    state.record_subject_failure(subject);
                }
                return SweepPass::Error;
            }
        },
        Either::Right((_, _)) => return SweepPass::Shutdown,
    };

    let (loaded, embedded) = match load_embed {
        Ok(pair) => {
            // A successful load (or a reuse of the cached embedder) clears the load
            // latch + counter.
            state.consecutive_load_failures = 0;
            state.load_failed_latch = None;
            pair
        }
        Err(error) => {
            state.consecutive_load_failures = state.consecutive_load_failures.saturating_add(1);
            crate::native_capture::debug_log::log_error(format!(
                "subject vector backfill failed to load model '{}/{}' (consecutive load failures: {}): {error}",
                descriptor.provider, descriptor.model_id, state.consecutive_load_failures
            ));
            if state.consecutive_load_failures >= MAX_CONSECUTIVE_LOAD_FAILURES {
                crate::native_capture::debug_log::log_warn(format!(
                    "subject vector backfill giving up on loading model '{}/{}' after {MAX_CONSECUTIVE_LOAD_FAILURES} consecutive failures; idling until the selection changes (the search backfill owns the reinstall signal for this model)",
                    descriptor.provider, descriptor.model_id
                ));
                state.load_failed_latch =
                    Some((descriptor.provider.clone(), descriptor.model_id.clone()));
                return SweepPass::Idle;
            }
            return SweepPass::Error;
        }
    };
    // Restore the embedder for the next pass.
    state.embedder = Some(loaded);

    let now = super::worker::now_ms();
    let mut stored = 0u64;
    let mut embed_failures = 0u64;
    let mut quarantined = 0u64;
    let mut transient_errors = 0u64;
    for (subject, result) in embedded {
        match result {
            Ok(vector) => {
                // Finite-guard (mirrors the query/write-path guard): a NaN/inf
                // component yields non-deterministic KNN ordering, so skip it. Treat
                // it as a per-subject deterministic failure so a persistently
                // non-finite input is eventually quarantined rather than retried
                // forever.
                if vector.iter().any(|component| !component.is_finite()) {
                    crate::native_capture::debug_log::log_error(format!(
                        "subject vector backfill produced a non-finite component for '{subject}'; skipping"
                    ));
                    embed_failures += 1;
                    let (failures, now_quarantined) = state.record_subject_failure(&subject);
                    if now_quarantined {
                        quarantined += 1;
                        crate::native_capture::debug_log::log_warn(format!(
                            "subject vector backfill quarantined '{subject}' after {failures} consecutive failures (excluded until content change/restart)"
                        ));
                    }
                    continue;
                }
                match infra
                    .subject_vectors()
                    .upsert_subject_vector(&subject, &vector, now, &active_model)
                    .await
                {
                    Ok(()) => {
                        stored += 1;
                        state.clear_subject_failures(&subject);
                    }
                    Err(error) => {
                        transient_errors += 1;
                        crate::native_capture::debug_log::log_error(format!(
                            "subject vector backfill failed to store vector for '{subject}': {error}"
                        ));
                    }
                }
            }
            Err(error) => {
                // Each subject was embedded alone, so a failure is isolated by
                // construction: credit it toward quarantine.
                embed_failures += 1;
                let (failures, now_quarantined) = state.record_subject_failure(&subject);
                if now_quarantined {
                    quarantined += 1;
                    crate::native_capture::debug_log::log_warn(format!(
                        "subject vector backfill quarantined '{subject}' after {failures} consecutive embed failures (excluded until content change/restart); last error: {error}"
                    ));
                } else {
                    crate::native_capture::debug_log::log_error(format!(
                        "subject vector backfill failed to embed '{subject}' (failure {failures}/{MAX_CONSECUTIVE_SUBJECT_FAILURES}): {error}"
                    ));
                }
            }
        }
    }

    if stored > 0 {
        crate::native_capture::debug_log::log_info(format!(
            "subject vector backfill embedded {stored} subject(s) (batch={}, embed_failures={embed_failures}, quarantined={quarantined}, transient_errors={transient_errors})",
            texts.len()
        ));
        return SweepPass::DidWork(backfill_batch_cooldown(embed_started_at.elapsed()));
    }
    if quarantined > 0 && embed_failures == quarantined && transient_errors == 0 {
        // The only non-stored outcomes were newly-quarantined poison: idle rather than
        // spin the 30s error loop on subjects we just decided to skip.
        return SweepPass::Idle;
    }
    if embed_failures > 0 || transient_errors > 0 {
        return SweepPass::Error;
    }
    // Non-empty batch but nothing stored and nothing failed (shouldn't happen):
    // treat as a minimal-pace work pass so the loop continues.
    SweepPass::DidWork(BACKFILL_BATCH_COOLDOWN_MIN)
}

/// Compose the embedding text for a Subject: the handle joined with its canonical
/// statement (`"{subject}: {statement}"`) so a terse handle carries context, or the
/// bare handle when there is no usable statement.
fn compose_embed_text(subject: &str, statement: Option<&str>) -> String {
    match statement {
        Some(statement) if !statement.trim().is_empty() => format!("{subject}: {statement}"),
        _ => subject.to_string(),
    }
}

/// CPU-pacing cooldown between batches: the batch wall time scaled and clamped to
/// `[MIN, MAX]`, mirroring the search backfill's pacing.
fn backfill_batch_cooldown(batch_duration: Duration) -> Duration {
    let scaled = batch_duration.mul_f64(BACKFILL_BATCH_COOLDOWN_MULTIPLIER);
    scaled.clamp(BACKFILL_BATCH_COOLDOWN_MIN, BACKFILL_BATCH_COOLDOWN_MAX)
}

/// Advance the idle-drop streak for one non-work pass and release the embedder if
/// the grace period has been reached, logging once. The shared "decay to idle"
/// handler for both the Idle and Error arms: a caught-up worker AND a worker stuck
/// in an error loop both release the large model weights after the grace period
/// instead of pinning them resident. Returns nothing; mutates `state`.
fn maybe_release_embedder_on_idle_decay(state: &mut SweepState, reason: &str) {
    state.consecutive_idles = state.consecutive_idles.saturating_add(1);
    if state.consecutive_idles >= IDLE_PASSES_BEFORE_EMBEDDER_DROP && state.embedder.is_some() {
        state.embedder = None;
        crate::native_capture::debug_log::log_info(format!(
            "subject vector backfill released the embedder after {} {reason} passes (model weights returned to the OS; reloads on next subject)",
            state.consecutive_idles
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_embed_text_enriches_with_statement_else_bare_handle() {
        assert_eq!(
            compose_embed_text("Apple", Some("prefers the M-series laptops")),
            "Apple: prefers the M-series laptops"
        );
        // No statement, or a blank/whitespace one, falls back to the bare handle.
        assert_eq!(compose_embed_text("Apple", None), "Apple");
        assert_eq!(compose_embed_text("Apple", Some("   ")), "Apple");
    }

    #[test]
    fn subject_is_quarantined_only_after_n_consecutive_failures_case_insensitively() {
        let mut state = SweepState::default();
        assert!(!state.is_subject_quarantined("Rust"));

        for expected in 1..MAX_CONSECUTIVE_SUBJECT_FAILURES {
            let (failures, now_quarantined) = state.record_subject_failure("Rust");
            assert_eq!(failures, expected);
            assert!(!now_quarantined);
            assert!(!state.is_subject_quarantined("Rust"));
        }
        let (failures, now_quarantined) = state.record_subject_failure("Rust");
        assert_eq!(failures, MAX_CONSECUTIVE_SUBJECT_FAILURES);
        assert!(now_quarantined);
        // NOCASE folding: a different casing resolves to the same quarantine entry.
        assert!(state.is_subject_quarantined("rust"));
    }

    #[test]
    fn a_clean_store_resets_a_subject_failure_streak() {
        let mut state = SweepState::default();
        state.record_subject_failure("Tokio");
        state.record_subject_failure("Tokio");
        state.clear_subject_failures("tokio");
        assert!(!state.is_subject_quarantined("Tokio"));
        for _ in 1..MAX_CONSECUTIVE_SUBJECT_FAILURES {
            assert!(!state.record_subject_failure("Tokio").1);
        }
        assert!(state.record_subject_failure("Tokio").1);
    }

    #[test]
    fn switching_models_clears_the_quarantine_map_and_load_latch() {
        let mut state = SweepState::default();
        let provider = "mnema";
        // Prime the previous selection and quarantine a subject under model A.
        state.reconcile_selection(provider, "model-a");
        for _ in 0..MAX_CONSECUTIVE_SUBJECT_FAILURES {
            state.record_subject_failure("Poison");
        }
        state.load_failed_latch = Some((provider.to_string(), "model-a".to_string()));
        assert!(state.is_subject_quarantined("Poison"));

        // A switch to model B clears the per-model state.
        state.reconcile_selection(provider, "model-b");
        assert!(!state.is_subject_quarantined("Poison"));
        assert_eq!(state.load_failed_latch, None);
        assert_eq!(state.consecutive_load_failures, 0);

        // Reconciling to the SAME model is a no-op: the map is preserved.
        for _ in 0..MAX_CONSECUTIVE_SUBJECT_FAILURES {
            state.record_subject_failure("Other");
        }
        state.reconcile_selection(provider, "model-b");
        assert!(state.is_subject_quarantined("Other"));
    }

    #[test]
    fn load_latch_only_matches_the_failed_selection() {
        let mut state = SweepState::default();
        assert!(!state.load_latch_matches("mnema", "model-a"));
        state.load_failed_latch = Some(("mnema".to_string(), "model-a".to_string()));
        assert!(state.load_latch_matches("mnema", "model-a"));
        assert!(!state.load_latch_matches("mnema", "model-b"));
    }

    #[test]
    fn idle_decay_releases_the_embedder_after_the_grace_period() {
        let mut state = SweepState::default();
        // Pretend an embedder is loaded by giving the streak a head start; the release
        // only fires when `embedder.is_some()`, so with no embedder it never fires.
        for _ in 0..(IDLE_PASSES_BEFORE_EMBEDDER_DROP + 2) {
            maybe_release_embedder_on_idle_decay(&mut state, "idle");
        }
        // No embedder was ever loaded, so nothing to release and the streak just grows.
        assert!(state.embedder.is_none());
        assert!(state.consecutive_idles >= IDLE_PASSES_BEFORE_EMBEDDER_DROP);
    }

    #[test]
    fn backfill_cooldown_clamps_to_the_pacing_band() {
        assert_eq!(
            backfill_batch_cooldown(Duration::ZERO),
            BACKFILL_BATCH_COOLDOWN_MIN
        );
        assert_eq!(
            backfill_batch_cooldown(Duration::from_secs(60)),
            BACKFILL_BATCH_COOLDOWN_MAX
        );
        let mid = Duration::from_millis(500);
        assert_eq!(backfill_batch_cooldown(mid), mid);
    }
}
