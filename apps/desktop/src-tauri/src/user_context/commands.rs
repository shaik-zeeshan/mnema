//! Tauri command surface for **User Context** (issue #93).
//!
//! These three commands drive the "User Context" subsection of the Settings →
//! Access "Reasoning Engine" card: a status readout, a read-only recent-Activity
//! preview list, and a manual "run derivation now" trigger (for verification and
//! the settings button). Conclusion / dismiss / pin / wipe commands land with
//! their own slices (#94/#97/#99) on this same module.

use std::sync::Arc;

use capture_types::{Activity, UserContextStatus, UserContextTokenUsage};
use serde::Serialize;
use tauri::Emitter;

use crate::app_infra::AppInfraState;
use crate::native_capture::{read_recording_settings, RecordingSettingsState};

use super::worker::{run_forward_activity_window, USER_CONTEXT_CHANGED_EVENT};

/// Result of a manual one-window derivation pass (the "Run derivation now"
/// button). camelCase to match the rest of the wire DTOs.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserContextDerivationRunResult {
    pub activities_derived: i64,
    pub window_start_ms: i64,
    pub window_end_ms: i64,
    pub items_read: i64,
    pub message: String,
}

/// Availability + counts + token usage for the User Context settings surface.
///
/// `conclusion_count` is 0 and `backfilling` is false in this slice; they are
/// owned by #94 (Conclusion derivation) and #98 (History Backfill) respectively.
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
        conclusion_count: 0, // TODO(#94): wire count_conclusions.
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

/// Manually run ONE forward Activity-derivation window immediately (ignoring the
/// worker's tier pacing). Used by the settings "Run derivation now" button and
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
    let provider_label = super::worker::provider_label_for(&ai_runtime);
    let model_label = super::worker::model_label_for(&ai_runtime);
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

    Ok(UserContextDerivationRunResult {
        activities_derived: run.activities_derived,
        window_start_ms: run.window_start_ms,
        window_end_ms: run.window_end_ms,
        items_read: run.items_read,
        message: run.message,
    })
}
