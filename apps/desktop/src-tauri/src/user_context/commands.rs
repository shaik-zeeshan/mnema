//! Tauri commands for **User Context**; the behaviour lives in
//! `app_core::user_context::commands`.

use std::sync::Arc;

use app_core::user_context::commands::{
    self as uc, AuthoredContextRow, UserContextDerivationRunResult,
};
use app_core::{AppCore, AppInfraState};
use capture_types::{
    Activity, ActivityCategory, ActivityMeta, AuthoredContext, Conclusion, DismissedView,
    FocusLevel, SubjectView, UserContextDigestResult, UserContextStatus,
};
use tauri::State;

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

#[tauri::command]
pub async fn wipe_user_context(
    core: State<'_, Arc<AppCore>>,
    infra: State<'_, AppInfraState>,
) -> Result<(), String> {
    uc::wipe_user_context(&core, &infra).await
}
