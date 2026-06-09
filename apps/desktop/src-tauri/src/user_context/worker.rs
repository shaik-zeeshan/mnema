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
//! Conclusion distillation (#94) and confidence decay (#95) extend this loop on
//! slower beats — see the cadence note near [`run_forward_activity_window`].

use std::sync::Arc;
use std::time::Duration;

use capture_types::{AiEngineKind, DerivationBudgetTier};
use tauri::{Emitter, Manager};

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

/// One worker tick. Returns the sleep to wait before the next tick. All errors
/// are logged, never propagated.
async fn worker_tick(infra: &AppInfraState, app_handle: &tauri::AppHandle) -> Duration {
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

    let run = run_forward_activity_window(
        &engine,
        infra.user_context(),
        provider_label,
        model_label,
    )
    .await;

    if run.changed {
        let _ = app_handle.emit(USER_CONTEXT_CHANGED_EVENT, ());
    }
    if run.activities_derived > 0 {
        crate::native_capture::debug_log::log_info(format!(
            "user context worker derived {} activities (window=[{},{}], items={})",
            run.activities_derived, run.window_start_ms, run.window_end_ms, run.items_read
        ));
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
        loop {
            if *shutdown_rx.borrow() {
                break;
            }

            // Every tick path is error-recording (no `?`, no `unwrap`), so a
            // tick returns an idle/paced sleep rather than panicking the worker.
            let next_sleep = worker_tick(&infra, &app_handle).await;

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
