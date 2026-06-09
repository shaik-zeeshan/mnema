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

/// Current unix time in milliseconds (matches the user_context `*_at_ms`
/// convention; no `Date.now()`-style nondeterminism).
fn now_ms() -> i64 {
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
            }
        }
    }
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
}
