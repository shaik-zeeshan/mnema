//! Tauri command surface for **User Context** (issues #93/#94).
//!
//! These commands drive the "User Context" subsection of the Settings → Access
//! "Reasoning Engine" card: a status readout, read-only Activity + Conclusion
//! preview lists, a Subject view, and a manual "run derivation now" trigger (for
//! verification and the settings button). Dismiss / pin / wipe commands land
//! with their own slices (#97/#99) on this same module.

use std::sync::Arc;

use capture_types::{Activity, Conclusion, SubjectView, UserContextStatus, UserContextTokenUsage};
use serde::Serialize;
use tauri::Emitter;

use crate::app_infra::AppInfraState;
use crate::native_capture::{read_recording_settings, RecordingSettingsState};

use super::worker::{
    model_label_for, provider_label_for, run_conclusion_distillation, run_forward_activity_window,
    USER_CONTEXT_CHANGED_EVENT,
};

/// Result of a manual one-window derivation pass (the "Run derivation now"
/// button). camelCase to match the rest of the wire DTOs.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserContextDerivationRunResult {
    pub activities_derived: i64,
    /// Conclusions upserted by the optional distillation pass that follows the
    /// Activity window (#94). 0 when distillation did not run / produced nothing.
    pub conclusions_derived: i64,
    pub window_start_ms: i64,
    pub window_end_ms: i64,
    pub items_read: i64,
    pub message: String,
}

/// Availability + counts + token usage for the User Context settings surface.
///
/// `backfilling` is false in this slice; it is owned by #98 (History Backfill).
#[tauri::command]
pub async fn get_user_context_status(
    state: tauri::State<'_, RecordingSettingsState>,
    infra: tauri::State<'_, AppInfraState>,
) -> Result<UserContextStatus, String> {
    let settings = read_recording_settings(state.inner());
    let ai_runtime = settings.ai_runtime;
    let user_context = settings.user_context;

    // Engine availability mirrors `ai_runtime`'s reason-code shape: disabled is
    // its own reason; otherwise resolve_engine_config tells us ready / why-not.
    let (engine_available, reason) = if !ai_runtime.enabled {
        (false, Some("ai_runtime_disabled".to_string()))
    } else {
        match crate::ai_runtime::resolve_engine_config(&ai_runtime) {
            Ok(_) => (true, None),
            Err(reason) => (false, Some(reason)),
        }
    };

    let store = infra.user_context();
    let activity_count = store.count_activities().await.map_err(|e| e.to_string())?;
    let conclusion_count = store.count_conclusions().await.map_err(|e| e.to_string())?;
    let last_derived_at_ms = store.last_derived_at_ms().await.map_err(|e| e.to_string())?;
    let token_usage = store
        .token_usage_totals()
        .await
        .map_err(|e| e.to_string())
        .unwrap_or(UserContextTokenUsage {
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            run_count: 0,
        });

    Ok(UserContextStatus {
        engine_available,
        reason,
        activity_count,
        conclusion_count,
        last_derived_at_ms,
        backfilling: false, // TODO(#98): wire backfill progress.
        token_usage,
        budget_tier: user_context.derivation_budget_tier,
    })
}

/// The most-recently-derived Activities (newest first) for the preview list.
#[tauri::command]
pub async fn list_user_context_activities(
    infra: tauri::State<'_, AppInfraState>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<Activity>, String> {
    let limit = limit.unwrap_or(50).clamp(1, 500);
    let offset = offset.unwrap_or(0).max(0);
    infra
        .user_context()
        .list_recent_activities(limit, offset)
        .await
        .map_err(|e| e.to_string())
}

/// The derived **Conclusion** dossier (highest-confidence first) for the preview
/// list. `visible` Conclusions always appear; `faded` (below the display floor)
/// are included only when `include_faded` is true; `dismissed` never appear.
#[tauri::command]
pub async fn list_user_context_conclusions(
    infra: tauri::State<'_, AppInfraState>,
    include_faded: Option<bool>,
) -> Result<Vec<Conclusion>, String> {
    let include_faded = include_faded.unwrap_or(false);
    infra
        .user_context()
        .list_conclusions(include_faded)
        .await
        .map_err(|e| e.to_string())
}

/// The Subject page: every non-dismissed **Conclusion** about a Subject (faded
/// included) plus its confidence trajectories. Trajectories are empty in this
/// slice — Confidence History (#95) fills them.
#[tauri::command]
pub async fn get_user_context_subject(
    infra: tauri::State<'_, AppInfraState>,
    subject: String,
) -> Result<SubjectView, String> {
    let conclusions = infra
        .user_context()
        .list_conclusions_for_subject(&subject)
        .await
        .map_err(|e| e.to_string())?;
    Ok(SubjectView {
        subject,
        conclusions,
        // TODO(#95): confidence history. The per-Conclusion confidence-over-time
        // trajectories are populated from `list_confidence_history` once the
        // Confidence History store + snapshotting (#95) land.
        trajectories: vec![],
    })
}

/// Manually run ONE forward Activity-derivation window immediately (ignoring the
/// worker's tier pacing), then one Conclusion distillation pass over the
/// accumulated Activities. Used by the settings "Run derivation now" button and
/// for end-to-end verification. Returns a helpful message if the engine is
/// unavailable or there are no captures in range; never errors on those.
#[tauri::command]
pub async fn user_context_run_derivation_now(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, RecordingSettingsState>,
    infra: tauri::State<'_, AppInfraState>,
) -> Result<UserContextDerivationRunResult, String> {
    let settings = read_recording_settings(state.inner());
    let ai_runtime = settings.ai_runtime;

    if !ai_runtime.enabled {
        return Ok(UserContextDerivationRunResult {
            activities_derived: 0,
            conclusions_derived: 0,
            window_start_ms: 0,
            window_end_ms: 0,
            items_read: 0,
            message: "The Reasoning Engine is off. Enable it to derive Activities.".to_string(),
        });
    }

    let engine = match crate::ai_runtime::resolve_engine_config(&ai_runtime) {
        Ok(engine) => engine,
        Err(reason) => {
            return Ok(UserContextDerivationRunResult {
                activities_derived: 0,
                conclusions_derived: 0,
                window_start_ms: 0,
                window_end_ms: 0,
                items_read: 0,
                message: format!("The Reasoning Engine is not ready ({reason})."),
            });
        }
    };

    // Reuse the same forward-window path the worker runs, so manual and
    // automatic derivation behave identically.
    let infra = Arc::clone(&*infra);
    let provider_label = provider_label_for(&ai_runtime);
    let model_label = model_label_for(&ai_runtime);
    let run = run_forward_activity_window(
        &engine,
        infra.user_context(),
        provider_label.clone(),
        model_label.clone(),
    )
    .await;

    // Also run one distillation pass after the Activity window so a manual run
    // surfaces fresh Conclusions immediately (helpful for verification). It
    // shares the same engine and stamps its own `derivation_run` (kind
    // `'conclusion'`); distillation no-ops below two Activities.
    let conclusions_before = infra.user_context().count_conclusions().await.unwrap_or(0);
    let distilled_changed = run_conclusion_distillation(
        &engine,
        infra.user_context(),
        provider_label,
        model_label,
    )
    .await;
    let conclusions_after = infra.user_context().count_conclusions().await.unwrap_or(0);
    let conclusions_derived = (conclusions_after - conclusions_before).max(0);

    if run.changed || distilled_changed {
        let _ = app_handle.emit(USER_CONTEXT_CHANGED_EVENT, ());
    }

    Ok(UserContextDerivationRunResult {
        activities_derived: run.activities_derived,
        conclusions_derived,
        window_start_ms: run.window_start_ms,
        window_end_ms: run.window_end_ms,
        items_read: run.items_read,
        message: run.message,
    })
}
