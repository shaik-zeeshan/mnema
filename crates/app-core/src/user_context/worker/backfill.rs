use super::*;

/// The outcome of deriving one bounded window (shared by the backward
/// **History Backfill** pass and the failed-window **Retry** pass, #113).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BackfillStep {
    /// Coverage already reaches the floor — nothing left to backfill this pass.
    Complete,
    /// A window was read but had no captures; a `skipped` run advanced the
    /// cursor. Keep going (more windows may have content).
    Advanced,
    /// A window was derived (≥1 Activity inserted or extended), recorded as a
    /// completed run.
    Derived,
    /// A window read empty-of-engine-work or failed; the outcome was still
    /// recorded via a run. Keep going.
    NoChange,
}

/// Derive ONE explicit `[start, end]` window and stamp a `derivation_run` of
/// `kind` with the outcome (empty → `skipped`, derived → `completed`, engine
/// error → `failed`). This is the shared engine-facing half of both the
/// History Backfill step (#98) and the failed-window Retry step (#113); the
/// callers own window *selection*. Resilient: an error records a `failed` run
/// and returns `BackfillStep::NoChange`; it never panics.
pub(super) async fn derive_window_and_record(
    engine: &ai_engine::EngineConfig,
    store: &UserContextStore,
    kind: &str,
    start: i64,
    end: i64,
    jump_floor_ms: Option<i64>,
    provider_label: Option<String>,
    model_label: Option<String>,
) -> BackfillStep {
    let window = match store.read_capture_window(start, end, MAX_ITEMS).await {
        Ok(window) => window,
        Err(error) => {
            // Record a failed run so the outcome stays visible in the ledger.
            let _ = store
                .insert_derivation_run(NewDerivationRun {
                    kind: kind.to_string(),
                    window_start_ms: Some(start),
                    window_end_ms: Some(end),
                    status: "failed".to_string(),
                    activities_derived: 0,
                    conclusions_derived: 0,
                    input_tokens: 0,
                    output_tokens: 0,
                    provider: provider_label,
                    model: model_label,
                    error: Some(error.to_string()),
                    gate_drops: DistillationGateDrops::default(),
                })
                .await;
            return BackfillStep::NoChange;
        }
    };

    // No captures in this window: record a `skipped` run to advance the cursor
    // (and, for a retry, extinguish the hole — the captures are gone). Backfill
    // stretches the skipped run down to the next older capture in one step.
    if window.items.is_empty() {
        let start = match jump_floor_ms {
            Some(floor) => store
                .prev_raw_capture_before_ms(start)
                .await
                .map_or(start, |prev| backfill_jump_start(start, floor, prev)),
            None => start,
        };
        let _ = store
            .insert_derivation_run(NewDerivationRun {
                kind: kind.to_string(),
                window_start_ms: Some(start),
                window_end_ms: Some(end),
                status: "skipped".to_string(),
                activities_derived: 0,
                conclusions_derived: 0,
                input_tokens: 0,
                output_tokens: 0,
                provider: provider_label,
                model: model_label,
                error: None,
                gate_drops: DistillationGateDrops::default(),
            })
            .await;
        return BackfillStep::Advanced;
    }

    // On success the `completed` run is written with the Activities, atomically.
    let completed_run = NewDerivationRun {
        kind: kind.to_string(),
        window_start_ms: Some(start),
        window_end_ms: Some(end),
        status: "completed".to_string(),
        activities_derived: 0,
        conclusions_derived: 0,
        input_tokens: 0,
        output_tokens: 0,
        provider: provider_label.clone(),
        model: model_label.clone(),
        error: None,
        gate_drops: DistillationGateDrops::default(),
    };
    match derivation::derive_activities(engine, store, window, completed_run).await {
        Ok(outcome) => {
            if outcome.inserted + outcome.extended > 0 {
                BackfillStep::Derived
            } else {
                BackfillStep::NoChange
            }
        }
        Err(error) => {
            let _ = store
                .insert_derivation_run(NewDerivationRun {
                    kind: kind.to_string(),
                    window_start_ms: Some(start),
                    window_end_ms: Some(end),
                    status: "failed".to_string(),
                    activities_derived: 0,
                    conclusions_derived: 0,
                    input_tokens: 0,
                    output_tokens: 0,
                    provider: provider_label,
                    model: model_label,
                    error: Some(error),
                    gate_drops: DistillationGateDrops::default(),
                })
                .await;
            BackfillStep::NoChange
        }
    }
}

/// Run exactly ONE backward **History Backfill** window (#98): extend coverage
/// older by `[max(floor, oldest_covered - BACKFILL_WINDOW_MS) .. oldest_covered]`.
/// This walks history newest-first, one bounded window per call — a background
/// trickle, NOT a synchronous whole-history bill (there is deliberately no
/// "derive all of history now" path).
///
/// Every outcome records a `'backfill'`-kind `derivation_run` so the
/// oldest-covered cursor advances backward each pass (empty → `skipped`,
/// derived → `completed`, engine error → `failed`). Resilient: an error records
/// a `failed` run and returns `BackfillStep::NoChange`; it never panics.
///
/// `oldest_covered` is the caller-resolved `oldest_derivation_run_window_start()`;
/// the caller skips backfill entirely (forward seeds coverage first) when that is
/// `None`. `floor_ms` is `backfill_floor_ms(...)`.
pub(super) async fn run_backfill_window(
    engine: &ai_engine::EngineConfig,
    store: &UserContextStore,
    floor_ms: i64,
    oldest_covered: i64,
    provider_label: Option<String>,
    model_label: Option<String>,
) -> BackfillStep {
    let Some((start, end)) = next_backfill_window(floor_ms, oldest_covered) else {
        return BackfillStep::Complete;
    };
    derive_window_and_record(
        engine,
        store,
        "backfill",
        start,
        end,
        Some(floor_ms),
        provider_label,
        model_label,
    )
    .await
}

/// Run the per-tick backward **History Backfill** pass: up to
/// `backfill_windows_per_tick(provider, tier)` windows, oldest-covered →
/// floor. Returns whether any window derived an Activity (so the caller can emit
/// `user_context_changed`). Stops early when backfill is complete. Re-reads the
/// oldest-covered cursor between windows so each window steps strictly older.
///
/// Newest-first ordering invariant (#98): the caller runs this ONLY after the
/// forward catch-up is caught up this tick, so recent windows are always covered
/// before older ones — recency-weighted Confidence means recent history drives
/// current conclusions.
pub(super) async fn run_backfill_pass(
    engine: &ai_engine::EngineConfig,
    store: &UserContextStore,
    floor_ms: i64,
    max_windows: u32,
    provider_label: Option<String>,
    model_label: Option<String>,
) -> bool {
    let mut derived_anything = false;
    for _ in 0..max_windows {
        let oldest_covered = match store.oldest_derivation_run_window_start().await {
            Ok(Some(oldest)) => oldest,
            // No windowed coverage yet → the forward pass seeds it first; skip.
            Ok(None) => break,
            Err(_) => break,
        };
        let step = run_backfill_window(
            engine,
            store,
            floor_ms,
            oldest_covered,
            provider_label.clone(),
            model_label.clone(),
        )
        .await;
        match step {
            BackfillStep::Complete => break,
            BackfillStep::Derived => derived_anything = true,
            BackfillStep::Advanced | BackfillStep::NoChange => {}
        }
    }
    derived_anything
}

/// Retry policy for failed derivation windows: 3 total attempts (original + 2 retries),
/// flat 10-minute backoff between attempts. Mirrors the processing-queue caps in
/// `processing/store.rs`; a single-element `backoff_seconds` slice saturates to the same
/// value for every failure count, giving flat (non-stepped) spacing.
pub(crate) const WINDOW_RETRY_POLICY: RetryPolicy = RetryPolicy {
    max_attempts: 3,
    backoff_seconds: &[600],
};

/// At most this many failed windows are retried per tick. Retries are a
/// low-priority background trickle behind fresh forward work and backfill,
/// never a burst.
pub(super) const RETRY_WINDOWS_PER_TICK: i64 = 1;

/// Run the per-tick failed-window **Retry** pass (#113): re-derive up to
/// [`RETRY_WINDOWS_PER_TICK`] `failed` windows whose span was never covered by
/// a later `completed`/`skipped` run, newest-first. Eligibility (the
/// per-window [`MAX_WINDOW_DERIVATION_FAILURES`] cap and the
/// [`RETRY_BACKOFF_MS`] spacing) is resolved by the store query; a successful
/// or `skipped` retry run extinguishes the hole, while another failure counts
/// toward the cap and re-arms the backoff. Runs AFTER the forward beat (fresh
/// capture always derives first) so a transient engine hiccup heals itself
/// within a few beats instead of leaving a permanent Activity gap. Returns
/// whether any window derived an Activity.
pub(super) async fn run_failed_window_retry_pass(
    engine: &ai_engine::EngineConfig,
    store: &UserContextStore,
    provider_label: Option<String>,
    model_label: Option<String>,
) -> bool {
    let eligible = match store
        .failed_windows_eligible_for_retry(
            WINDOW_RETRY_POLICY.max_attempts,
            now_ms().saturating_sub(WINDOW_RETRY_POLICY.backoff_seconds(1) * 1000),
            RETRY_WINDOWS_PER_TICK,
        )
        .await
    {
        Ok(eligible) => eligible,
        Err(_) => return false,
    };

    let mut derived_anything = false;
    for window in eligible {
        let step = derive_window_and_record(
            engine,
            store,
            &window.kind,
            window.window_start_ms,
            window.window_end_ms,
            None,
            provider_label.clone(),
            model_label.clone(),
        )
        .await;
        if step == BackfillStep::Derived {
            derived_anything = true;
        }
        crate::debug_log::log_warn(format!(
            "user context worker retried failed window [{},{}] (attempt {} of {}): {:?}",
            window.window_start_ms,
            window.window_end_ms,
            window.failure_count + 1,
            WINDOW_RETRY_POLICY.max_attempts,
            step
        ));
    }
    derived_anything
}
