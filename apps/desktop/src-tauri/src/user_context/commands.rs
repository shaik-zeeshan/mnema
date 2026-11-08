//! Tauri commands for **User Context**; the behaviour lives in
//! `app_core::user_context::commands`. `wipe_user_context` stays here whole
//! because it turns the engine off through `native_capture`'s settings update,
//! which also reconciles the Ask AI MCP connectors; it moves with S3.

use std::sync::Arc;

use app_core::recording_settings::RecordingSettingsState;
use app_core::user_context::commands::{
    self as uc, AuthoredContextRow, UserContextDerivationRunResult,
};
use app_core::{AppCore, AppInfraState, CoreEvent};
use capture_types::{
    Activity, ActivityCategory, ActivityMeta, AuthoredContext, Conclusion, DismissedView,
    FocusLevel, SubjectView, UpdateAiRuntimeSettingsRequest, UserContextDigestResult,
    UserContextStatus,
};
use tauri::{Manager, State};

#[tauri::command]
pub async fn get_user_context_status(
    core: State<'_, Arc<AppCore>>,
    infra: State<'_, AppInfraState>,
) -> Result<UserContextStatus, String> {
    uc::get_user_context_status(&core, &infra).await
}

#[tauri::command]
pub async fn list_user_context_activities(
    infra: State<'_, AppInfraState>,
    limit: Option<i64>,
    offset: Option<i64>,
    start_ms: Option<i64>,
    end_ms: Option<i64>,
) -> Result<Vec<Activity>, String> {
    uc::list_user_context_activities(&infra, limit, offset, start_ms, end_ms).await
}

#[tauri::command]
pub async fn list_user_context_activity_meta(
    infra: State<'_, AppInfraState>,
    start_ms: i64,
    end_ms: i64,
) -> Result<Vec<ActivityMeta>, String> {
    uc::list_user_context_activity_meta(&infra, start_ms, end_ms).await
}

#[tauri::command]
pub async fn list_failed_derivation_windows(
    infra: State<'_, AppInfraState>,
    start_ms: i64,
    end_ms: i64,
) -> Result<Vec<(i64, i64)>, String> {
    uc::list_failed_derivation_windows(&infra, start_ms, end_ms).await
}

#[tauri::command]
pub async fn list_user_context_conclusions(
    infra: State<'_, AppInfraState>,
    include_faded: Option<bool>,
    start_ms: Option<i64>,
    end_ms: Option<i64>,
) -> Result<Vec<Conclusion>, String> {
    uc::list_user_context_conclusions(&infra, include_faded, start_ms, end_ms).await
}

#[tauri::command]
pub async fn get_user_context_subject(
    infra: State<'_, AppInfraState>,
    subject: String,
) -> Result<SubjectView, String> {
    uc::get_user_context_subject(&infra, subject).await
}

#[tauri::command]
pub async fn user_context_run_derivation_now(
    core: State<'_, Arc<AppCore>>,
    infra: State<'_, AppInfraState>,
) -> Result<UserContextDerivationRunResult, String> {
    uc::user_context_run_derivation_now(&core, &infra).await
}

#[tauri::command]
pub async fn get_user_context_digest(
    core: State<'_, Arc<AppCore>>,
    infra: State<'_, AppInfraState>,
    range_kind: String,
    start_ms: i64,
    end_ms: i64,
) -> Result<UserContextDigestResult, String> {
    uc::get_user_context_digest(&core, &infra, range_kind, start_ms, end_ms).await
}

#[tauri::command]
pub async fn regenerate_user_context_digest(
    core: State<'_, Arc<AppCore>>,
    infra: State<'_, AppInfraState>,
    range_kind: String,
    start_ms: i64,
    end_ms: i64,
) -> Result<UserContextDigestResult, String> {
    uc::regenerate_user_context_digest(&core, &infra, range_kind, start_ms, end_ms).await
}

#[tauri::command]
pub async fn user_context_dismiss_conclusion(
    core: State<'_, Arc<AppCore>>,
    infra: State<'_, AppInfraState>,
    id: i64,
) -> Result<(), String> {
    uc::user_context_dismiss_conclusion(&core, &infra, id).await
}

#[tauri::command]
pub async fn user_context_list_dismissed(
    infra: State<'_, AppInfraState>,
) -> Result<Vec<DismissedView>, String> {
    uc::user_context_list_dismissed(&infra).await
}

#[tauri::command]
pub async fn user_context_restore_dismissed(
    core: State<'_, Arc<AppCore>>,
    infra: State<'_, AppInfraState>,
    subject: String,
    statement: String,
) -> Result<(), String> {
    uc::user_context_restore_dismissed(&core, &infra, subject, statement).await
}

#[tauri::command]
pub async fn user_context_set_pinned(
    core: State<'_, Arc<AppCore>>,
    infra: State<'_, AppInfraState>,
    id: i64,
    pinned: bool,
) -> Result<(), String> {
    uc::user_context_set_pinned(&core, &infra, id, pinned).await
}

#[tauri::command]
pub async fn user_context_correct_activity_category(
    core: State<'_, Arc<AppCore>>,
    infra: State<'_, AppInfraState>,
    id: i64,
    category: Option<ActivityCategory>,
) -> Result<(), String> {
    uc::user_context_correct_activity_category(&core, &infra, id, category).await
}

#[tauri::command]
pub async fn user_context_correct_activity_focus(
    core: State<'_, Arc<AppCore>>,
    infra: State<'_, AppInfraState>,
    id: i64,
    focus: Option<FocusLevel>,
) -> Result<(), String> {
    uc::user_context_correct_activity_focus(&core, &infra, id, focus).await
}

#[tauri::command]
pub async fn list_user_context_authored(
    infra: State<'_, AppInfraState>,
) -> Result<Vec<AuthoredContextRow>, String> {
    uc::list_user_context_authored(&infra).await
}

#[tauri::command]
pub async fn user_context_add_authored(
    core: State<'_, Arc<AppCore>>,
    infra: State<'_, AppInfraState>,
    text: String,
    topic: Option<String>,
) -> Result<AuthoredContext, String> {
    uc::user_context_add_authored(&core, &infra, text, topic).await
}

#[tauri::command]
pub async fn user_context_update_authored(
    core: State<'_, Arc<AppCore>>,
    infra: State<'_, AppInfraState>,
    id: i64,
    text: String,
    topic: Option<String>,
) -> Result<(), String> {
    uc::user_context_update_authored(&core, &infra, id, text, topic).await
}

#[tauri::command]
pub async fn user_context_delete_authored(
    core: State<'_, Arc<AppCore>>,
    infra: State<'_, AppInfraState>,
    id: i64,
) -> Result<(), String> {
    uc::user_context_delete_authored(&core, &infra, id).await
}

#[tauri::command]
pub async fn user_context_stamp_local_offset(
    infra: State<'_, AppInfraState>,
    offset_minutes: i64,
) -> Result<(), String> {
    uc::user_context_stamp_local_offset(&infra, offset_minutes).await
}

/// **Wipe User Context** (#97, ADR 0029): the explicit, full clear of the derived
/// dossier. Because that dossier deliberately outlives the raw-capture Retention
/// Policy window, this is the disclosed control for erasing it.
///
/// These are three independent writes (engine settings, the `user_context_*`
/// tables, the conversation store) with no enclosing transaction, so a mid-failure
/// is possible. We order them so the *only* possible partial state is the safe one:
///
/// 1. Turn the **Reasoning Engine** OFF FIRST, through the normal AI-runtime
///    settings flow (`enabled = false`), so it persists to
///    `recording-settings.json` and broadcasts
///    `recording_settings_changed`/`*_domain_changed`. Wiping implies "I'm done";
///    rebuilding is a deliberate re-opt-in. (Merely toggling the engine off — the
///    separate master toggle — is NOT a wipe: it stops new derivation but leaves
///    the dossier readable.) Disabling first means a later data-wipe failure leaves
///    "engine OFF + dossier present" (consistent, retryable) rather than the unsafe
///    "engine ON + data gone" the previous ordering risked. If disabling fails we
///    bail before touching any data, leaving everything intact.
/// 2. Clear every `user_context_*` table — all derived **Activity** /
///    **Conclusion** data AND **Dismissal State** and the derivation-run ledger
///    (`wipe_all`). Raw captures and other settings are untouched.
/// 3. Clear all persistent Quick Recall / Chat conversations (issue #102), the
///    single shared store, and Quick Recall's recent queries, so the same control
///    erases every derived/recalled surface. All data wipes run even if one errors, so a single
///    failure cannot strand the other store half-cleared with no retry; the first
///    error is surfaced afterwards.
/// 4. Emit `user_context_changed` so the now-empty surface refreshes.
#[tauri::command]
pub async fn wipe_user_context(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, RecordingSettingsState>,
    infra: tauri::State<'_, AppInfraState>,
) -> Result<(), String> {
    // 1. Disable the engine FIRST via the same settings-update path the toggle
    //    uses, so persistence + broadcasts (recording_settings_changed /
    //    domain_changed) are identical. Doing this before any data wipe makes the
    //    unsafe "engine ON + data gone" state unreachable: if this fails we bail
    //    with everything intact; if a later data wipe fails we are left with
    //    "engine OFF + dossier present", which is consistent and retryable.
    crate::native_capture::update_ai_runtime_settings(
        UpdateAiRuntimeSettingsRequest {
            enabled: Some(false),
            ..Default::default()
        },
        app_handle.clone(),
        state,
    )
    .map_err(|e| e.message)?;

    // 2 + 3. Storage half: clear the whole dossier AND all persistent conversations.
    //    Run both regardless of an error in the first so a single store failure
    //    cannot leave the other half-cleared with no retry path; report the first
    //    error after attempting both.
    let wipe_context = infra.user_context().wipe_all().await;
    let wipe_conversations = infra.conversation().wipe_all().await;
    // Quick Recall's recent queries share the conversations' contract.
    let wipe_recent_queries = ::app_infra::delete_recent_queries(infra.write_pool()).await;
    wipe_context.map_err(|e| e.to_string())?;
    wipe_conversations.map_err(|e| e.to_string())?;
    wipe_recent_queries.map_err(|e| e.to_string())?;

    // 4. Refresh the (now empty) User Context surface.
    app_handle
        .state::<Arc<AppCore>>()
        .host()
        .emit(CoreEvent::UserContextChanged);
    Ok(())
}
