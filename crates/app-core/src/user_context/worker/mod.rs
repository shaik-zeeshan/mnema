//! Background **Activity** derivation worker (issue #93).
//!
//! The host spawns the loop (`spawn_user_context_worker`, mirroring
//! `spawn_retention_cleanup_worker`) and calls [`worker_tick`] from it: one
//! loop, tracked for graceful shutdown, that selects between a tier-paced sleep
//! and the shutdown watch. The loop runs the **OCR Catch-Up** pattern — it walks
//! the forward (newest) un-derived capture window opportunistically off the
//! capture hot path and asks the configured Reasoning Engine to segment it into
//! semantic Activities.
//!
//! Cheapness when disabled is load-bearing: continuous derivation is gated on
//! the **User Context** opt-in (`user_context.enabled`, off by default) AND the
//! shared engine-configured prerequisite, so a tick with the opt-in off (or an
//! unresolved/unreachable engine) does nothing but sleep the idle interval. No
//! store/LLM work happens.
//!
//! Conclusion distillation (#94) runs on a slower beat after the Activity window
//! each tick (see [`WorkerCadence`]); confidence decay (#95) extends the loop on
//! a slower beat still — see the cadence note near [`run_forward_activity_window`].
//!
//! GAP CATCH-UP. The forward beat covers at most one `MAX_WINDOW_MS` window per
//! tick, so a long time behind realtime would otherwise drain one window per
//! tick-interval — hours of crawl after the worker (or the engine) was idle.
//! Two mechanisms keep catch-up fast:
//!   - **Empty-gap jump (A):** an *empty* forward window (no captures — e.g. an
//!     away-from-keyboard gap) jumps the cursor straight to the next raw capture
//!     (or to `now` if there is none) in a SINGLE tick instead of stepping one
//!     30-minute window. See [`empty_window_jump_target`] and the empty-window
//!     branch of [`run_forward_activity_window`].
//!   - **Catch-up cadence (C):** when the forward beat actually covered a window
//!     this tick AND the cursor is still more than one window behind `now`, the
//!     tick sleeps [`CATCHUP_INTERVAL`] (a few seconds) instead of the full tier
//!     interval, so windows-with-captures drain back-to-back until within one
//!     window of realtime. See [`is_catching_up`] and `worker_tick`.

use std::sync::OnceLock;
use std::time::Duration;

use capture_types::{AiProviderKind, DerivationBudgetTier};

use app_infra::retry_policy::RetryPolicy;
use app_infra::user_context::confidence;
use app_infra::{DistillationGateDrops, NewDerivationRun, UserContextStore};

use crate::recording_settings::{current_recording_settings, RecordingSettingsState};
use crate::{AppCore, AppInfraState, CoreEvent};

use super::derivation;

mod backfill;
mod tick;
pub(crate) use backfill::*;
pub use tick::*;

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

/// Catch-up sleep: when the forward beat did real work but the cursor is still
/// more than one window behind `now`, the worker ticks again after this short
/// interval instead of the full tier interval, draining the backlog back-to-back
/// until it is within one window of realtime. (Empty gaps are already collapsed
/// in one tick by the cursor jump; this drains a backlog of windows that have captures.)
const CATCHUP_INTERVAL: Duration = Duration::from_secs(5);
/// Forward-backlog threshold for entering catch-up cadence: more than one full
/// window (`MAX_WINDOW_MS`) of un-derived time still ahead of the cursor.
const CATCHUP_BACKLOG_MS: i64 = MAX_WINDOW_MS;

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
/// The engine is keyed by the global default model's provider (ADR 0034).
fn backfill_windows_per_tick(provider: AiProviderKind, tier: DerivationBudgetTier) -> u32 {
    if provider.is_local() {
        1
    } else {
        match tier {
            DerivationBudgetTier::Light => 1,
            DerivationBudgetTier::Balanced => 2,
            DerivationBudgetTier::Thorough => 4,
        }
    }
}

/// Process-wide serialization for the forward Activity beat (#12). BOTH the
/// background worker tick AND the "Run derivation now" command call
/// [`run_forward_activity_window`]; without a guard they could read the same
/// cursor, pick the same `[start, end]` window, and derive it twice — DUPLICATE
/// Activities and a second cloud bill for that span. This `tokio::Mutex` is
/// `try_lock`'d inside [`run_forward_activity_window`], so whichever caller is
/// already mid-window wins and the other returns a no-op for this pass (the
/// worker simply retries next tick). Paired with the `derived_window_exists`
/// idempotency guard below, which extinguishes the race even across the
/// select-window gap (e.g. a run that committed between lock release and the next
/// caller's window selection).
static FORWARD_DERIVATION_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

fn forward_derivation_lock() -> &'static tokio::sync::Mutex<()> {
    FORWARD_DERIVATION_LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Current unix time in milliseconds (matches the user_context `*_at_ms`
/// convention; no `Date.now()`-style nondeterminism).
pub(crate) fn now_ms() -> i64 {
    (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}

/// The tier-paced sleep between ticks for a resolved engine (keyed by the
/// global default model's provider: local = fixed pacing, cloud = the tier).
fn tick_interval(provider: AiProviderKind, tier: DerivationBudgetTier) -> Duration {
    if provider.is_local() {
        LOCAL_INTERVAL
    } else {
        match tier {
            DerivationBudgetTier::Light => LIGHT_INTERVAL,
            DerivationBudgetTier::Balanced => BALANCED_INTERVAL,
            DerivationBudgetTier::Thorough => THOROUGH_INTERVAL,
        }
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

/// Where to advance the forward cursor when a window read empty (A). Rather than
/// stepping one `MAX_WINDOW_MS` window across a multi-hour away-from-keyboard gap
/// (one tick per 30 min), jump in O(1) to the next raw capture — or straight to
/// `now` when nothing has been captured since `end`.
///
/// `next_raw_capture` is `store.next_raw_capture_at_ms(end)`: the earliest RAW
/// (NOT OCR-completeness-filtered, on purpose) frame/audio timestamp at-or-after
/// `end`, or `None` if there is none. The RAW-ness matters: if a capture sits
/// *within* the just-read window its OCR may still be pending, so we must NOT
/// leap past it — we advance one window as before and re-derive it next tick.
///
/// Invariant: the result is always `>= end` (the cursor never goes backward)
/// and `<= now`.
fn empty_window_jump_target(end: i64, now: i64, next_raw_capture: Option<i64>) -> i64 {
    let target = match next_raw_capture {
        // Empty gap [end, next): jump past it (clamped to `now`).
        Some(next) if next > end => next.min(now),
        // A raw capture sits within the just-read window (OCR pending): do NOT
        // leap past it — advance one window as before.
        Some(_) => end,
        // No raw captures at/after `end`: safe to jump straight to `now`.
        None => now,
    };
    // Defensive clamp: `end <= result <= now` (note `end <= now` always holds here).
    target.max(end).min(now)
}

/// Whether the worker should use the short catch-up cadence after this tick (C):
/// only when the forward beat actually covered a window this tick
/// (`derived_window`) AND the cursor is still more than `CATCHUP_BACKLOG_MS`
/// behind `now`.
fn is_catching_up(derived_window: bool, forward_backlog_ms: i64) -> bool {
    derived_window && forward_backlog_ms > CATCHUP_BACKLOG_MS
}

/// Where an EMPTY backfill window's coverage starts — the backward twin of
/// [`empty_window_jump_target`]: just above the latest earlier capture (so the
/// next window ends on it), or the floor when nothing older exists (backfill
/// done). Without it an empty night, or the days below the first capture, are
/// walked 30 minutes per window. Always within `[floor, start]`.
fn backfill_jump_start(start: i64, floor: i64, prev_capture: Option<i64>) -> i64 {
    prev_capture
        .map_or(floor, |prev| prev + 1)
        .max(floor)
        .min(start)
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
    /// is caught up — there was no new recent window to derive. Feeds
    /// [`is_catching_up`], which gates both the catch-up cadence and the backward
    /// **History Backfill** pass (newest-first). Note this is `true` even
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
/// EMPTY-GAP JUMP (A): when the window reads empty (no captures — an
/// away-from-keyboard gap), the recorded `skipped` run advances the cursor in
/// O(1) to the next raw capture (or to `now` if there is none) via
/// [`empty_window_jump_target`], instead of stepping a single `MAX_WINDOW_MS`.
/// This collapses a multi-hour zero-capture gap into one tick rather than one
/// tick per 30 minutes. A capture sitting *within* the just-read window (OCR
/// still pending) is never leapt past — the cursor advances one window as before.
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

    // (#12) Serialize the forward beat: if another caller (the worker tick or the
    // run-now command) is already mid-window, do not race it onto the same cursor.
    // `try_lock` — neither caller blocks; the loser returns a no-op (the worker
    // retries next tick; the manual command reports it).
    let _guard = match forward_derivation_lock().try_lock() {
        Ok(guard) => guard,
        Err(_) => {
            let last_covered_end = store
                .latest_derivation_run_window()
                .await
                .ok()
                .flatten()
                .map(|(_, end)| end);
            return ForwardWindowRun {
                activities_derived: 0,
                window_start_ms: last_covered_end.unwrap_or(now),
                window_end_ms: now,
                items_read: 0,
                message: "A derivation pass is already running.".to_string(),
                changed: false,
                derived_window: false,
            };
        }
    };

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

    // Idempotency backstop (#12): a run may already cover exactly this window — a
    // racing caller could have committed it between our lock acquisition and the
    // cursor read above (the cursor read is not itself under any longer-held lock).
    // `derived_window_exists` is the guard that was built for this and never wired.
    // Treat an already-covered window as a no-op rather than re-billing it.
    if store
        .derived_window_exists(start, end)
        .await
        .unwrap_or(false)
    {
        return ForwardWindowRun {
            activities_derived: 0,
            window_start_ms: start,
            window_end_ms: end,
            items_read: 0,
            message: "This window was already derived.".to_string(),
            changed: false,
            derived_window: false,
        };
    }

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
                    gate_drops: DistillationGateDrops::default(),
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
    // worker does not re-pick the same empty window forever. Rather than stepping
    // a single window, jump the cursor in O(1) past the empty gap to the next raw
    // capture (A) — see `empty_window_jump_target`. The `Err` fallback stays here
    // as `end` (a failed jump query degrades to the old one-window step).
    if window.items.is_empty() {
        let jump_to = match store.next_raw_capture_at_ms(end).await {
            Ok(next) => empty_window_jump_target(end, now, next),
            Err(_) => end, // query failed: fall back to the current 30-min step
        };
        let _ = store
            .insert_derivation_run(NewDerivationRun {
                kind: "activity".to_string(),
                window_start_ms: Some(start),
                window_end_ms: Some(jump_to),
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
        let message = if jump_to > end {
            "No captures in range; jumped the derivation cursor past the empty gap.".to_string()
        } else {
            "No captures in range; advanced the derivation cursor.".to_string()
        };
        return ForwardWindowRun {
            activities_derived: 0,
            window_start_ms: start,
            window_end_ms: jump_to,
            items_read: 0,
            message,
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
                    gate_drops: DistillationGateDrops::default(),
                })
                .await;
            let message = if outcome.inserted == 0 && outcome.extended > 0 {
                format!(
                    "Extended {} ongoing {} from {} capture item(s).",
                    outcome.extended,
                    if outcome.extended == 1 {
                        "Activity"
                    } else {
                        "Activities"
                    },
                    items_read
                )
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
                changed: outcome.inserted + outcome.extended > 0,
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
                    gate_drops: DistillationGateDrops::default(),
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

#[cfg(test)]
mod tests;
