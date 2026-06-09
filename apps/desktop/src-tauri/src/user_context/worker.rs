//! Background **Activity** derivation worker (issue #93).
//!
//! Mirrors `spawn_retention_cleanup_worker`: one `tauri::async_runtime::spawn`
//! loop, tracked for graceful shutdown, that selects between a tier-paced sleep
//! and the shutdown watch. The loop runs the **OCR Catch-Up** pattern — it walks
//! the forward (newest) un-derived capture window opportunistically off the
//! capture hot path and asks the configured Reasoning Engine to segment it into
//! semantic Activities.
//!
//! Cheapness when disabled is load-bearing: the Reasoning Engine is **off by
//! default**, so a tick with `ai_runtime.enabled == false` (or an unresolved
//! engine) does nothing but sleep the idle interval. No store/LLM work happens.
//!
//! Conclusion distillation (#94) runs on a slower beat after the Activity window
//! each tick (see [`WorkerCadence`]); confidence decay (#95) extends the loop on
//! a slower beat still — see the cadence note near [`run_forward_activity_window`].

use std::sync::Arc;
use std::time::Duration;

use capture_types::{AiEngineKind, DerivationBudgetTier};
use tauri::{Emitter, Manager};

use app_infra::user_context::confidence;
use app_infra::{NewDerivationRun, UserContextStore};

use crate::app_infra::{shutdown_aware_sleep, AppInfraState, BackgroundWorkersState};
use crate::native_capture::{read_recording_settings, RecordingSettingsState};

use super::derivation;

/// Frontend event emitted after a derivation pass changes the dossier, so the
/// settings preview list + status can refresh. Carries no payload.
pub const USER_CONTEXT_CHANGED_EVENT: &str = "user_context_changed";

/// Idle interval when the engine is disabled / unresolved. Kept modest so the
/// worker notices an enable promptly, but it does no work on these ticks.
const IDLE_INTERVAL: Duration = Duration::from_secs(300);

/// Tier-based poll intervals for a cloud engine (the Derivation Budget knob).
const LIGHT_INTERVAL: Duration = Duration::from_secs(600);
const BALANCED_INTERVAL: Duration = Duration::from_secs(300);
const THOROUGH_INTERVAL: Duration = Duration::from_secs(120);

/// Local engine pacing is fixed (resource pacing only; the tier knob is
/// cloud-only because it is about token spend).
const LOCAL_INTERVAL: Duration = Duration::from_secs(300);

/// How far back the very first forward window reaches when nothing has been
/// derived yet (6h). Older history is the job of History Backfill (#98); this
/// slice only walks forward.
const INITIAL_LOOKBACK_MS: i64 = 6 * 60 * 60 * 1000;

/// Don't derive until at least this much new, un-derived time has accumulated,
/// so the engine sees a real stretch of activity (intent shifts need context).
const MIN_WINDOW_MS: i64 = 120 * 1000;

/// Cap one window so a long gap since the last run is split across ticks rather
/// than derived in a single oversized prompt (30 min).
const MAX_WINDOW_MS: i64 = 30 * 60 * 1000;

/// Cap items per window so a busy stretch stays within a sane prompt budget.
const MAX_ITEMS: i64 = 80;

/// One **History Backfill** window walks this far back per pass (30 min, matching
/// the forward `MAX_WINDOW_MS`). Backfill extends coverage backward, newest-first,
/// one bounded window at a time — a background trickle paced by the Derivation
/// Budget, never a synchronous whole-history bill.
const BACKFILL_WINDOW_MS: i64 = 30 * 60 * 1000;

/// Per-tick **backfill intensity** — how many backward windows one tick may
/// derive — as a function of the resolved engine + tier. This is what makes the
/// named cloud tier a *real* intensity knob (not just a sleep): Thorough chews
/// through more history per pass, Light at most one. A LOCAL engine ignores the
/// tier entirely (fixed resource pacing, like OCR) and does one window per tick.
///
/// Cloud: Light = 1, Balanced = 2, Thorough = 4 windows/tick (paired with the
/// tier sleeps `LIGHT_INTERVAL` 600s / `BALANCED_INTERVAL` 300s / `THOROUGH_INTERVAL`
/// 120s, so the windows-per-hour spread is wide: ~6 / ~24 / ~120). Local = 1.
fn backfill_windows_per_tick(engine_kind: AiEngineKind, tier: DerivationBudgetTier) -> u32 {
    match engine_kind {
        AiEngineKind::Local => 1,
        AiEngineKind::Cloud => match tier {
            DerivationBudgetTier::Light => 1,
            DerivationBudgetTier::Balanced => 2,
            DerivationBudgetTier::Thorough => 4,
        },
    }
}

/// Current unix time in milliseconds (matches the user_context `*_at_ms`
/// convention; no `Date.now()`-style nondeterminism).
pub(crate) fn now_ms() -> i64 {
    (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}

/// The tier-paced sleep between ticks for a resolved engine.
fn tick_interval(engine_kind: AiEngineKind, tier: DerivationBudgetTier) -> Duration {
    match engine_kind {
        AiEngineKind::Local => LOCAL_INTERVAL,
        AiEngineKind::Cloud => match tier {
            DerivationBudgetTier::Light => LIGHT_INTERVAL,
            DerivationBudgetTier::Balanced => BALANCED_INTERVAL,
            DerivationBudgetTier::Thorough => THOROUGH_INTERVAL,
        },
    }
}

/// The next forward window `[start, end]` to derive, or `None` when not enough
/// new time has accumulated yet. `start` resumes from the newest covered window
/// (or `now - INITIAL_LOOKBACK_MS` on a cold store); `end` is `now`, clamped to
/// at most `MAX_WINDOW_MS` past `start`.
fn next_forward_window(now: i64, last_covered_end: Option<i64>) -> Option<(i64, i64)> {
    let start = last_covered_end.unwrap_or_else(|| now.saturating_sub(INITIAL_LOOKBACK_MS));
    if now.saturating_sub(start) < MIN_WINDOW_MS {
        return None;
    }
    let end = now.min(start.saturating_add(MAX_WINDOW_MS));
    Some((start, end))
}

/// The **History Backfill** floor (#98): how far back backfill is allowed to
/// reach, newest-first. When **go-deeper** is on, the floor is the true earliest
/// capture (`earliest_capture_at_ms`, all of history); otherwise it is the
/// bounded recent window `now - backfill_window_days`. A `None` earliest-capture
/// (no captures, or go-deeper but nothing recorded) collapses to `now`, which
/// makes backfill a no-op (the floor is at/after coverage) until captures exist.
///
/// This bounded-by-default-with-explicit-go-deeper shape is what caps the
/// "cost surprise" even at the Thorough tier: backfill stops at the floor.
///
/// Shared with the status command (`get_user_context_status`) so the worker's
/// backfill floor and the "backfilling" progress readout agree on exactly the
/// same floor for a given settings snapshot.
pub(crate) fn backfill_floor_ms(
    now: i64,
    window_days: u32,
    go_deeper: bool,
    earliest_capture: Option<i64>,
) -> i64 {
    if go_deeper {
        earliest_capture.unwrap_or(now)
    } else {
        now.saturating_sub((window_days as i64).saturating_mul(86_400_000))
    }
}

/// The next backward backfill window `[start, end]`, or `None` when backfill has
/// nothing to do this pass. `oldest_covered` is the trailing edge of coverage
/// (`oldest_derivation_run_window_start`); the window is
/// `[max(floor, oldest_covered - BACKFILL_WINDOW_MS) .. oldest_covered]`. When
/// `oldest_covered <= floor`, coverage already reaches the floor → `None`
/// (backfill complete).
fn next_backfill_window(floor_ms: i64, oldest_covered: i64) -> Option<(i64, i64)> {
    if oldest_covered <= floor_ms {
        return None;
    }
    let start = floor_ms.max(oldest_covered.saturating_sub(BACKFILL_WINDOW_MS));
    Some((start, oldest_covered))
}

/// The result of running one forward window (used by both the worker tick and
/// the manual run-now command).
#[derive(Debug, Clone)]
pub struct ForwardWindowRun {
    pub activities_derived: i64,
    pub window_start_ms: i64,
    pub window_end_ms: i64,
    pub items_read: i64,
    /// Human-readable outcome for the run-now button / logs.
    pub message: String,
    /// Whether the dossier changed (drives whether to emit the refresh event).
    pub changed: bool,
    /// Whether a forward window was actually *picked* this pass (i.e.
    /// `next_forward_window` returned a window). `false` means the forward catch-up
    /// is caught up — there was no new recent window to derive — which is the signal
    /// the worker uses to gate the backward **History Backfill** pass (newest-first:
    /// backfill only runs when forward has nothing new). Note this is `true` even
    /// when the window read empty / derived zero Activities — what matters for the
    /// interleave is whether forward had ground to cover, not the yield.
    pub derived_window: bool,
}

/// Run exactly one forward un-derived Activity window end-to-end: pick the
/// window, read the redacted capture text, ask the engine to segment it, persist
/// the Activities, and stamp a single `derivation_run` ledger row (which also
/// advances the newest-covered cursor). Never panics; an engine error records a
/// `failed` run so the cursor still advances and the loop/caller continues.
///
/// `provider_label` / `model_label` are recorded on the run row for the
/// tokens-used readout. The engine is already resolved (key sourced from the
/// keychain) — this fn never touches credentials.
///
/// CADENCE HOOK (for #94/#95): this is the Activity beat. A Conclusion
/// distillation pass (`distill_conclusions`) should run on a slower beat — e.g.
/// every K successful Activity ticks or when new Activities exist since the last
/// distillation — and a confidence-decay pass slower still. Add those as sibling
/// helpers called from `worker_tick` after `run_forward_activity_window`, each
/// recording its own `derivation_run` (kind `'conclusion'` / `'confidence'`).
pub async fn run_forward_activity_window(
    engine: &ai_engine::EngineConfig,
    store: &UserContextStore,
    provider_label: Option<String>,
    model_label: Option<String>,
) -> ForwardWindowRun {
    let now = now_ms();
    let last_covered_end = store
        .latest_derivation_run_window()
        .await
        .ok()
        .flatten()
        .map(|(_, end)| end);

    let Some((start, end)) = next_forward_window(now, last_covered_end) else {
        return ForwardWindowRun {
            activities_derived: 0,
            window_start_ms: last_covered_end.unwrap_or(now),
            window_end_ms: now,
            items_read: 0,
            message: "Not enough new captures yet to derive an Activity.".to_string(),
            changed: false,
            derived_window: false,
        };
    };

    let window = match store.read_capture_window(start, end, MAX_ITEMS).await {
        Ok(window) => window,
        Err(error) => {
            // Could not even read the window: record a failed run so the cursor
            // still advances, and report it.
            let _ = store
                .insert_derivation_run(NewDerivationRun {
                    kind: "activity".to_string(),
                    window_start_ms: Some(start),
                    window_end_ms: Some(end),
                    status: "failed".to_string(),
                    activities_derived: 0,
                    conclusions_derived: 0,
                    input_tokens: 0,
                    output_tokens: 0,
                    provider: provider_label.clone(),
                    model: model_label.clone(),
                    error: Some(error.to_string()),
                })
                .await;
            return ForwardWindowRun {
                activities_derived: 0,
                window_start_ms: start,
                window_end_ms: end,
                items_read: 0,
                message: format!("Failed to read capture window: {error}"),
                changed: false,
                derived_window: true,
            };
        }
    };

    let items_read = window.items.len() as i64;

    // No captures in range: record a `skipped` run to ADVANCE the cursor so the
    // worker does not re-pick the same empty window forever.
    if window.items.is_empty() {
        let _ = store
            .insert_derivation_run(NewDerivationRun {
                kind: "activity".to_string(),
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
            })
            .await;
        return ForwardWindowRun {
            activities_derived: 0,
            window_start_ms: start,
            window_end_ms: end,
            items_read: 0,
            message: "No captures in range; advanced the derivation cursor.".to_string(),
            changed: false,
            derived_window: true,
        };
    }

    // Run the LLM segmentation, then stamp ONE derivation_run with the final
    // outcome (status / count / estimated tokens / window). This single insert
    // both records the result and advances the newest-covered cursor.
    match derivation::derive_activities(
        engine,
        store,
        window,
        provider_label.clone(),
        model_label.clone(),
    )
    .await
    {
        Ok(outcome) => {
            let _ = store
                .insert_derivation_run(NewDerivationRun {
                    kind: "activity".to_string(),
                    window_start_ms: Some(start),
                    window_end_ms: Some(end),
                    status: "completed".to_string(),
                    activities_derived: outcome.inserted as i64,
                    conclusions_derived: 0,
                    input_tokens: outcome.input_tokens,
                    output_tokens: outcome.output_tokens,
                    provider: provider_label,
                    model: model_label,
                    error: None,
                })
                .await;
            let message = if outcome.inserted == 0 {
                "Derivation completed; no new Activities in this window.".to_string()
            } else {
                format!(
                    "Derived {} {} from {} capture item(s).",
                    outcome.inserted,
                    if outcome.inserted == 1 {
                        "Activity"
                    } else {
                        "Activities"
                    },
                    items_read
                )
            };
            ForwardWindowRun {
                activities_derived: outcome.inserted as i64,
                window_start_ms: start,
                window_end_ms: end,
                items_read,
                message,
                changed: outcome.inserted > 0,
                derived_window: true,
            }
        }
        Err(error) => {
            // Record a failed run (cursor still advances) and report it; never
            // panic the worker.
            let _ = store
                .insert_derivation_run(NewDerivationRun {
                    kind: "activity".to_string(),
                    window_start_ms: Some(start),
                    window_end_ms: Some(end),
                    status: "failed".to_string(),
                    activities_derived: 0,
                    conclusions_derived: 0,
                    input_tokens: 0,
                    output_tokens: 0,
                    provider: provider_label,
                    model: model_label,
                    error: Some(error.clone()),
                })
                .await;
            ForwardWindowRun {
                activities_derived: 0,
                window_start_ms: start,
                window_end_ms: end,
                items_read,
                message: format!("Derivation failed: {error}"),
                changed: false,
                derived_window: true,
            }
        }
    }
}

/// The outcome of one backward **History Backfill** window pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackfillStep {
    /// Coverage already reaches the floor — nothing left to backfill this pass.
    Complete,
    /// A window was read but had no captures; a `skipped` `backfill` run advanced
    /// the cursor backward. Keep going (more windows may have content).
    Advanced,
    /// A window was derived (≥1 Activity), recorded as a completed `backfill` run.
    Derived,
    /// A window read empty-of-engine-work or failed; the cursor still advanced via
    /// a recorded run. Keep going.
    NoChange,
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
async fn run_backfill_window(
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

    let window = match store.read_capture_window(start, end, MAX_ITEMS).await {
        Ok(window) => window,
        Err(error) => {
            // Record a failed backfill run so the cursor still advances backward.
            let _ = store
                .insert_derivation_run(NewDerivationRun {
                    kind: "backfill".to_string(),
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
                })
                .await;
            return BackfillStep::NoChange;
        }
    };

    // No captures in this older window: record a `skipped` backfill run to advance
    // the cursor backward so the next pass picks the next-older window.
    if window.items.is_empty() {
        let _ = store
            .insert_derivation_run(NewDerivationRun {
                kind: "backfill".to_string(),
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
            })
            .await;
        return BackfillStep::Advanced;
    }

    match derivation::derive_activities(
        engine,
        store,
        window,
        provider_label.clone(),
        model_label.clone(),
    )
    .await
    {
        Ok(outcome) => {
            let _ = store
                .insert_derivation_run(NewDerivationRun {
                    kind: "backfill".to_string(),
                    window_start_ms: Some(start),
                    window_end_ms: Some(end),
                    status: "completed".to_string(),
                    activities_derived: outcome.inserted as i64,
                    conclusions_derived: 0,
                    input_tokens: outcome.input_tokens,
                    output_tokens: outcome.output_tokens,
                    provider: provider_label,
                    model: model_label,
                    error: None,
                })
                .await;
            if outcome.inserted > 0 {
                BackfillStep::Derived
            } else {
                BackfillStep::NoChange
            }
        }
        Err(error) => {
            let _ = store
                .insert_derivation_run(NewDerivationRun {
                    kind: "backfill".to_string(),
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
                })
                .await;
            BackfillStep::NoChange
        }
    }
}

/// Run the per-tick backward **History Backfill** pass: up to
/// `backfill_windows_per_tick(engine_kind, tier)` windows, oldest-covered →
/// floor. Returns whether any window derived an Activity (so the caller can emit
/// `user_context_changed`). Stops early when backfill is complete. Re-reads the
/// oldest-covered cursor between windows so each window steps strictly older.
///
/// Newest-first ordering invariant (#98): the caller runs this ONLY after the
/// forward catch-up is caught up this tick, so recent windows are always covered
/// before older ones — recency-weighted Confidence means recent history drives
/// current conclusions.
async fn run_backfill_pass(
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

/// Run Conclusion distillation on the slower beat once every this many
/// successful Activity ticks, so the dossier (expensive, occasional) re-distills
/// less often than the diary (cheap, frequent). A distillation also runs early
/// whenever the Activity count has grown since the last one (see [`WorkerCadence`]).
const CONCLUSION_EVERY_K_ACTIVITY_TICKS: u32 = 3;

/// Run the confidence-decay beat once every this many ticks. This is the
/// *slowest* beat (M ≥ K, the Conclusion beat): confidence fades on a 30-day
/// half-life, so re-evaluating decay only every few ticks is plenty, and decay
/// uses **no LLM** (it is pure local math). Counting ticks (not successful
/// Activity ticks) means the dossier keeps fading during quiet stretches even
/// when no new Activities are arriving — silence is exactly when decay matters.
const CONFIDENCE_DECAY_EVERY_M_TICKS: u32 = 6;

/// Keep at most this many **Confidence History** snapshots per Conclusion. History
/// is aggressively prunable: recency-weighting means old snapshots stop mattering,
/// so the trajectory keeps only a recent tail. At one snapshot per decay beat this
/// is many days of arc, far more than the Subject line needs.
const MAX_SNAPSHOTS_PER_CONCLUSION: i64 = 64;

/// Cross-tick state for the slower Conclusion-distillation beat. The worker loop
/// owns one of these and threads it through each tick. Activity derivation stays
/// the frequent beat; this counts successful Activity ticks so distillation runs
/// on a slower cadence (or sooner when new Activities have accumulated).
#[derive(Debug, Default)]
struct WorkerCadence {
    /// Successful Activity ticks since the last distillation.
    activity_ticks_since_distillation: u32,
    /// `count_activities()` observed at the last distillation, so a tick can
    /// distill early when the Activity total has grown.
    last_distilled_activity_count: Option<i64>,
    /// Ticks (of any kind, including idle-but-resolved ones that reach the
    /// decay check) since the last confidence-decay pass.
    ticks_since_decay: u32,
}

impl WorkerCadence {
    /// Whether this tick should run a Conclusion distillation: every Kth
    /// successful Activity tick, OR as soon as new Activities exist since the
    /// last distillation.
    fn should_distill(&self, current_activity_count: i64) -> bool {
        let grew = self
            .last_distilled_activity_count
            .map(|last| current_activity_count > last)
            .unwrap_or(current_activity_count > 0);
        grew || self.activity_ticks_since_distillation >= CONCLUSION_EVERY_K_ACTIVITY_TICKS
    }

    /// Whether this tick should run the confidence-decay beat: every Mth tick.
    /// Decay is the slowest beat (M ≥ K) and unconditional on new Activities —
    /// silence is precisely when a Conclusion should fade.
    fn should_decay(&self) -> bool {
        self.ticks_since_decay >= CONFIDENCE_DECAY_EVERY_M_TICKS
    }
}

/// Run one Conclusion distillation pass over accumulated Activities and stamp a
/// single `derivation_run` (kind `'conclusion'`) with the outcome. Resilient:
/// any engine/store error records a `failed` run and returns `false`; it never
/// panics the worker. Returns whether the dossier changed (≥1 upsert).
///
/// Shared with the run-now command so manual and automatic distillation behave
/// identically and both stamp the same ledger row.
pub(crate) async fn run_conclusion_distillation(
    engine: &ai_engine::EngineConfig,
    store: &UserContextStore,
    provider_label: Option<String>,
    model_label: Option<String>,
) -> bool {
    match derivation::distill_conclusions(engine, store).await {
        Ok(outcome) => {
            let _ = store
                .insert_derivation_run(NewDerivationRun {
                    kind: "conclusion".to_string(),
                    window_start_ms: None,
                    window_end_ms: None,
                    status: "completed".to_string(),
                    activities_derived: 0,
                    conclusions_derived: outcome.upserted as i64,
                    input_tokens: outcome.input_tokens,
                    output_tokens: outcome.output_tokens,
                    provider: provider_label,
                    model: model_label,
                    error: None,
                })
                .await;
            if outcome.upserted > 0 {
                crate::native_capture::debug_log::log_info(format!(
                    "user context worker distilled {} conclusion(s)",
                    outcome.upserted
                ));
            }
            outcome.upserted > 0
        }
        Err(error) => {
            let _ = store
                .insert_derivation_run(NewDerivationRun {
                    kind: "conclusion".to_string(),
                    window_start_ms: None,
                    window_end_ms: None,
                    status: "failed".to_string(),
                    activities_derived: 0,
                    conclusions_derived: 0,
                    input_tokens: 0,
                    output_tokens: 0,
                    provider: provider_label,
                    model: model_label,
                    error: Some(error),
                })
                .await;
            false
        }
    }
}

/// Run one **confidence-decay** pass (#95): the slowest, LLM-free beat. For each
/// decayable Conclusion (`visible`/`faded`, never dismissed), apply the
/// recency-weighted half-life [`confidence::decay`] over elapsed silence,
/// recompute its [`confidence::status_for`] (below the display floor → `faded`,
/// leaving the visible dossier while its Confidence History is kept), persist the
/// new value + status + `last_decayed_at_ms`, and append a Confidence History
/// snapshot so the Subject trajectory can plot the arc. After the loop, prune the
/// history to [`MAX_SNAPSHOTS_PER_CONCLUSION`] per Conclusion. Records a single
/// `derivation_run` (kind `'confidence'`, **0 tokens** — decay uses no LLM) with
/// the count touched. Resilient: every error is swallowed-with-log; it never
/// panics the worker.
///
/// A **pinned** Conclusion is exempt from decay (#99): `list_decayable_conclusions`
/// already drops pinned rows from the set entirely, and the per-row `pinned` flag
/// is passed to `decay()`/`status_for()` (which also honor it) as a belt-and-braces
/// guard. So pinned Conclusions are never touched by this beat.
pub(crate) async fn run_confidence_decay(
    store: &UserContextStore,
    provider_label: Option<String>,
    model_label: Option<String>,
) -> bool {
    let now = now_ms();
    let conclusions = match store.list_decayable_conclusions().await {
        Ok(conclusions) => conclusions,
        Err(error) => {
            let _ = store
                .insert_derivation_run(NewDerivationRun {
                    kind: "confidence".to_string(),
                    window_start_ms: None,
                    window_end_ms: None,
                    status: "failed".to_string(),
                    activities_derived: 0,
                    conclusions_derived: 0,
                    input_tokens: 0,
                    output_tokens: 0,
                    provider: provider_label,
                    model: model_label,
                    error: Some(error.to_string()),
                })
                .await;
            return false;
        }
    };

    let mut touched = 0i64;
    for conclusion in &conclusions {
        // A pinned Conclusion is exempt from decay (#99). `list_decayable_conclusions`
        // already excludes pinned rows, so this is normally `false` here; passing the
        // real flag keeps `decay()`/`status_for()` correct as a guard regardless.
        let pinned = conclusion.pinned;
        let decayed = confidence::decay(
            conclusion.confidence,
            conclusion.last_supported_at_ms,
            now,
            pinned,
        );
        let status = confidence::status_for(decayed, pinned);

        if let Err(error) = store
            .update_conclusion_confidence(conclusion.id, decayed, status, now)
            .await
        {
            crate::native_capture::debug_log::log_info(format!(
                "user context confidence decay: update failed for conclusion {}: {error}",
                conclusion.id
            ));
            continue;
        }
        // Snapshot the (possibly-faded) value so the Subject trajectory keeps the
        // arc even below the display floor (faded is not deleted).
        if let Err(error) = store
            .insert_confidence_snapshot(conclusion.id, decayed, now)
            .await
        {
            crate::native_capture::debug_log::log_info(format!(
                "user context confidence decay: snapshot failed for conclusion {}: {error}",
                conclusion.id
            ));
        }
        touched += 1;
    }

    // Aggressively prune the history tail (recency-weighting means old snapshots
    // stop mattering). Best-effort: a prune error does not fail the beat.
    if let Err(error) = store
        .prune_confidence_history(MAX_SNAPSHOTS_PER_CONCLUSION)
        .await
    {
        crate::native_capture::debug_log::log_info(format!(
            "user context confidence decay: history prune failed: {error}"
        ));
    }

    let _ = store
        .insert_derivation_run(NewDerivationRun {
            kind: "confidence".to_string(),
            window_start_ms: None,
            window_end_ms: None,
            status: "completed".to_string(),
            activities_derived: 0,
            conclusions_derived: touched,
            input_tokens: 0,
            output_tokens: 0,
            provider: provider_label,
            model: model_label,
            error: None,
        })
        .await;

    if touched > 0 {
        crate::native_capture::debug_log::log_info(format!(
            "user context worker decayed {touched} conclusion(s)"
        ));
    }
    touched > 0
}

/// One worker tick. Returns the sleep to wait before the next tick. All errors
/// are logged, never propagated. `cadence` carries the slower Conclusion beat
/// across ticks.
async fn worker_tick(
    infra: &AppInfraState,
    app_handle: &tauri::AppHandle,
    cadence: &mut WorkerCadence,
) -> Duration {
    let Some(settings_state) = app_handle.try_state::<RecordingSettingsState>() else {
        return IDLE_INTERVAL;
    };
    let settings = read_recording_settings(settings_state.inner());
    let ai_runtime = settings.ai_runtime;
    let user_context = settings.user_context;

    // Cheap-when-disabled: the engine is off by default; do nothing but idle.
    if !ai_runtime.enabled {
        return IDLE_INTERVAL;
    }

    let engine = match crate::ai_runtime::resolve_engine_config(&ai_runtime) {
        Ok(engine) => engine,
        Err(_reason) => {
            // Not ready (no model / no key / no endpoint). Stay cheap; do not log
            // the reason at error level on every tick.
            return IDLE_INTERVAL;
        }
    };

    let provider_label = provider_label_for(&ai_runtime);
    let model_label = model_label_for(&ai_runtime);

    // --- Activity beat (frequent) ---
    let run = run_forward_activity_window(
        &engine,
        infra.user_context(),
        provider_label.clone(),
        model_label.clone(),
    )
    .await;

    let mut dossier_changed = run.changed;
    if run.activities_derived > 0 {
        crate::native_capture::debug_log::log_info(format!(
            "user context worker derived {} activities (window=[{},{}], items={})",
            run.activities_derived, run.window_start_ms, run.window_end_ms, run.items_read
        ));
        cadence.activity_ticks_since_distillation =
            cadence.activity_ticks_since_distillation.saturating_add(1);
    }

    // --- History Backfill beat (backward, newest-first) ---
    // (#98) STRICTLY newest-first: only extend coverage *backward* once the forward
    // catch-up has no new recent window to derive this tick (`!run.derived_window`).
    // Recency-weighted Confidence means recent history drives current conclusions,
    // so recent windows are always covered before older ones.
    //
    // This is a BACKGROUND TRICKLE paced by the Derivation Budget — one bounded
    // window per pass, up to a small per-tier count — never a synchronous
    // whole-history bill (there is deliberately no "derive all of history now"
    // path). It is bounded by default (`backfill_window_days`) and extends to the
    // true earliest capture only under the explicit go-deeper toggle.
    if !run.derived_window {
        let now = now_ms();
        let earliest_capture = if user_context.backfill_go_deeper {
            infra
                .user_context()
                .earliest_capture_at_ms()
                .await
                .ok()
                .flatten()
        } else {
            None
        };
        let floor_ms = backfill_floor_ms(
            now,
            user_context.backfill_window_days,
            user_context.backfill_go_deeper,
            earliest_capture,
        );
        let max_windows =
            backfill_windows_per_tick(ai_runtime.engine_kind, user_context.derivation_budget_tier);
        let backfilled = run_backfill_pass(
            &engine,
            infra.user_context(),
            floor_ms,
            max_windows,
            provider_label.clone(),
            model_label.clone(),
        )
        .await;
        if backfilled {
            dossier_changed = true;
            cadence.activity_ticks_since_distillation =
                cadence.activity_ticks_since_distillation.saturating_add(1);
            crate::native_capture::debug_log::log_info(
                "user context worker backfilled older history (background trickle)",
            );
        }
    }

    // --- Conclusion beat (slower) ---
    // CADENCE HOOK (spec §6 / §4): after the forward Activity window, run
    // Conclusion distillation on a slower beat — every Kth successful Activity
    // tick, or whenever the Activity count has grown since the last distillation.
    // It shares the SAME resolved engine and records its own `derivation_run`
    // (kind `'conclusion'`).
    let activity_count = infra
        .user_context()
        .count_activities()
        .await
        .unwrap_or(0);
    if cadence.should_distill(activity_count) {
        let changed = run_conclusion_distillation(
            &engine,
            infra.user_context(),
            provider_label.clone(),
            model_label.clone(),
        )
        .await;
        dossier_changed = dossier_changed || changed;
        cadence.activity_ticks_since_distillation = 0;
        cadence.last_distilled_activity_count = Some(activity_count);
    }

    // --- Confidence-decay beat (slowest, LLM-free) ---
    // (#95) Every Mth tick, fade Conclusions on their recency-weighted half-life,
    // recompute faded/visible status, snapshot Confidence History, and prune it.
    // This beat counts ticks (not Activities) and runs even when no new Activities
    // arrived — silence is exactly when a Conclusion should fade. It uses NO LLM,
    // so it records a 0-token `derivation_run` (kind `'confidence'`).
    cadence.ticks_since_decay = cadence.ticks_since_decay.saturating_add(1);
    if cadence.should_decay() {
        let changed = run_confidence_decay(
            infra.user_context(),
            provider_label,
            model_label,
        )
        .await;
        dossier_changed = dossier_changed || changed;
        cadence.ticks_since_decay = 0;
    }

    if dossier_changed {
        let _ = app_handle.emit(USER_CONTEXT_CHANGED_EVENT, ());
    }

    tick_interval(ai_runtime.engine_kind, user_context.derivation_budget_tier)
}

/// Provider label for the run ledger (cloud provider id / local kind name).
pub(crate) fn provider_label_for(settings: &capture_types::AiRuntimeSettings) -> Option<String> {
    match settings.engine_kind {
        AiEngineKind::Cloud => Some(
            match settings.cloud_provider {
                capture_types::AiCloudProvider::Anthropic => "anthropic",
                capture_types::AiCloudProvider::Openai => "openai",
                capture_types::AiCloudProvider::OpenaiCompatible => "openai_compatible",
            }
            .to_string(),
        ),
        AiEngineKind::Local => Some(
            match settings.local_kind {
                capture_types::AiLocalKind::Ollama => "ollama",
                capture_types::AiLocalKind::Llamafile => "llamafile",
            }
            .to_string(),
        ),
    }
}

/// Model label for the run ledger (the cloud/local model id).
pub(crate) fn model_label_for(settings: &capture_types::AiRuntimeSettings) -> Option<String> {
    let model = match settings.engine_kind {
        AiEngineKind::Cloud => settings.cloud_model.trim(),
        AiEngineKind::Local => settings.local_model.trim(),
    };
    if model.is_empty() {
        None
    } else {
        Some(model.to_string())
    }
}

/// Spawn the background User Context derivation worker. Mirrors
/// `spawn_retention_cleanup_worker`: tracks the handle for graceful shutdown and
/// selects between a tier-paced sleep and the shutdown watch.
pub fn spawn_user_context_worker(
    infra: AppInfraState,
    app_handle: tauri::AppHandle,
    background_workers: BackgroundWorkersState,
) {
    let mut shutdown_rx = background_workers.subscribe();
    crate::native_capture::debug_log::log_info(
        "starting user context derivation worker (idle_when_disabled)",
    );
    let handle = tauri::async_runtime::spawn(async move {
        let infra = Arc::clone(&infra);
        // The slower Conclusion-distillation beat is paced relative to the
        // frequent Activity beat; this state survives across ticks.
        let mut cadence = WorkerCadence::default();
        loop {
            if *shutdown_rx.borrow() {
                break;
            }

            // Every tick path is error-recording (no `?`, no `unwrap`), so a
            // tick returns an idle/paced sleep rather than panicking the worker.
            let next_sleep = worker_tick(&infra, &app_handle, &mut cadence).await;

            if shutdown_aware_sleep(&mut shutdown_rx, next_sleep).await {
                break;
            }
        }
        crate::native_capture::debug_log::log_info("stopped user context derivation worker");
    });
    background_workers.track(handle);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cold_store_window_reaches_back_initial_lookback() {
        let now = 10_000_000_000;
        let window = next_forward_window(now, None).expect("cold window");
        assert_eq!(window.0, now - INITIAL_LOOKBACK_MS);
        assert_eq!(window.1, (now - INITIAL_LOOKBACK_MS) + MAX_WINDOW_MS);
    }

    #[test]
    fn resumes_from_last_covered_end() {
        let now = 10_000_000_000;
        let last_end = now - (5 * 60 * 1000);
        let window = next_forward_window(now, Some(last_end)).expect("forward window");
        assert_eq!(window.0, last_end);
        assert_eq!(window.1, now);
    }

    #[test]
    fn waits_until_enough_new_time() {
        let now = 10_000_000_000;
        let last_end = now - (MIN_WINDOW_MS - 1);
        assert!(next_forward_window(now, Some(last_end)).is_none());
    }

    #[test]
    fn clamps_long_gap_to_max_window() {
        let now = 10_000_000_000;
        let last_end = now - (5 * MAX_WINDOW_MS);
        let window = next_forward_window(now, Some(last_end)).expect("clamped window");
        assert_eq!(window.1 - window.0, MAX_WINDOW_MS);
    }

    #[test]
    fn cadence_distills_when_activity_count_grows() {
        let mut cadence = WorkerCadence::default();
        // Cold: distills as soon as there is at least one Activity.
        assert!(cadence.should_distill(1));
        cadence.last_distilled_activity_count = Some(5);
        cadence.activity_ticks_since_distillation = 0;
        // No growth and below the K threshold => skip.
        assert!(!cadence.should_distill(5));
        // Growth => distill early even below the K threshold.
        assert!(cadence.should_distill(6));
    }

    #[test]
    fn cadence_distills_every_kth_tick_without_growth() {
        let cadence = WorkerCadence {
            activity_ticks_since_distillation: CONCLUSION_EVERY_K_ACTIVITY_TICKS,
            last_distilled_activity_count: Some(10),
            ticks_since_decay: 0,
        };
        // No growth, but the K-tick threshold is met => distill.
        assert!(cadence.should_distill(10));
    }

    #[test]
    fn cadence_decays_on_the_slowest_beat() {
        // The decay beat is strictly the slowest (M ≥ K).
        assert!(CONFIDENCE_DECAY_EVERY_M_TICKS >= CONCLUSION_EVERY_K_ACTIVITY_TICKS);
        let mut cadence = WorkerCadence::default();
        // Below the M threshold => no decay yet.
        cadence.ticks_since_decay = CONFIDENCE_DECAY_EVERY_M_TICKS - 1;
        assert!(!cadence.should_decay());
        // At the threshold => decay.
        cadence.ticks_since_decay = CONFIDENCE_DECAY_EVERY_M_TICKS;
        assert!(cadence.should_decay());
    }

    #[test]
    fn tier_paces_cloud_but_not_local() {
        assert_eq!(
            tick_interval(AiEngineKind::Cloud, DerivationBudgetTier::Light),
            LIGHT_INTERVAL
        );
        assert_eq!(
            tick_interval(AiEngineKind::Cloud, DerivationBudgetTier::Thorough),
            THOROUGH_INTERVAL
        );
        assert_eq!(
            tick_interval(AiEngineKind::Local, DerivationBudgetTier::Thorough),
            LOCAL_INTERVAL
        );
    }

    // --- #98 History Backfill window math --------------------------------------

    #[test]
    fn bounded_backfill_floor_is_now_minus_window_days() {
        let now = 10_000_000_000;
        // go-deeper OFF: floor is the bounded recent window, ignoring the earliest
        // capture even when there is much older history.
        let floor = backfill_floor_ms(now, 30, false, Some(0));
        assert_eq!(floor, now - 30 * 86_400_000);
    }

    #[test]
    fn go_deeper_floor_reaches_earliest_capture() {
        let now = 10_000_000_000;
        let earliest = 1_234;
        // go-deeper ON: floor is the true earliest capture (all of history).
        assert_eq!(backfill_floor_ms(now, 30, true, Some(earliest)), earliest);
        // go-deeper ON with no captures collapses to `now` (backfill no-op).
        assert_eq!(backfill_floor_ms(now, 30, true, None), now);
    }

    #[test]
    fn backfill_window_steps_one_window_older() {
        let oldest_covered = 10_000_000_000;
        let floor = oldest_covered - 10 * BACKFILL_WINDOW_MS;
        let (start, end) = next_backfill_window(floor, oldest_covered).expect("backfill window");
        // The window ends at the current trailing edge and reaches back exactly one
        // BACKFILL_WINDOW_MS (the floor is far below, so it does not clamp here).
        assert_eq!(end, oldest_covered);
        assert_eq!(start, oldest_covered - BACKFILL_WINDOW_MS);
    }

    #[test]
    fn backfill_window_clamps_to_floor() {
        let oldest_covered = 10_000_000_000;
        // Floor is closer than one full window: the window start clamps to the floor.
        let floor = oldest_covered - (BACKFILL_WINDOW_MS / 2);
        let (start, end) = next_backfill_window(floor, oldest_covered).expect("clamped window");
        assert_eq!(start, floor);
        assert_eq!(end, oldest_covered);
    }

    #[test]
    fn backfill_complete_when_coverage_reaches_floor() {
        let oldest_covered = 10_000_000_000;
        // Coverage already AT the floor → nothing to backfill.
        assert!(next_backfill_window(oldest_covered, oldest_covered).is_none());
        // Coverage already BELOW the floor (e.g. floor moved up) → nothing to do.
        assert!(next_backfill_window(oldest_covered + 1, oldest_covered).is_none());
    }

    #[test]
    fn backfill_intensity_is_a_real_cloud_tier_knob() {
        // The named cloud tier is a real intensity knob (windows/tick), not just a
        // sleep: Light < Balanced < Thorough.
        assert_eq!(
            backfill_windows_per_tick(AiEngineKind::Cloud, DerivationBudgetTier::Light),
            1
        );
        assert_eq!(
            backfill_windows_per_tick(AiEngineKind::Cloud, DerivationBudgetTier::Balanced),
            2
        );
        assert_eq!(
            backfill_windows_per_tick(AiEngineKind::Cloud, DerivationBudgetTier::Thorough),
            4
        );
        // Local ignores the tier entirely (fixed pacing): always one window/tick.
        for tier in [
            DerivationBudgetTier::Light,
            DerivationBudgetTier::Balanced,
            DerivationBudgetTier::Thorough,
        ] {
            assert_eq!(backfill_windows_per_tick(AiEngineKind::Local, tier), 1);
        }
    }
}
