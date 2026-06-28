//! Tauri command surface for **User Context** (issues #93/#94).
//!
//! These commands drive the "User Context" subsection of the Settings → Access
//! "Reasoning Engine" card: a status readout, read-only Activity + Conclusion
//! preview lists, a Subject view, and a manual "run derivation now" trigger (for
//! verification and the settings button). Dismiss / pin / wipe commands land
//! with their own slices (#97/#99) on this same module.

use std::sync::Arc;

use capture_types::{
    Activity, ActivityCategory, AuthoredContext, Conclusion, DismissalState, DismissedView,
    FocusLevel, SubjectTrajectory, SubjectView, UpdateAiRuntimeSettingsRequest, UserContextDigest,
    UserContextDistillationSummary, UserContextStatus, UserContextTokenUsage,
};
use serde::Serialize;
use tauri::Emitter;

use crate::app_infra::AppInfraState;
use crate::native_capture::{read_recording_settings, RecordingSettingsState};

use super::worker::{
    backfill_floor_ms, model_label_for, now_ms, provider_label_for, run_conclusion_distillation,
    run_forward_activity_window, USER_CONTEXT_CHANGED_EVENT,
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
/// `backfilling` (#98) reflects whether the background **History Backfill** still
/// has older history to cover — it drives the "building your understanding…"
/// progress line. It is the engine-available AND not-yet-at-the-floor condition;
/// see the inline computation. The floor is resolved from the SAME
/// `user_context` settings the worker uses (`backfill_floor_ms`).
#[tauri::command]
pub async fn get_user_context_status(
    state: tauri::State<'_, RecordingSettingsState>,
    infra: tauri::State<'_, AppInfraState>,
) -> Result<UserContextStatus, String> {
    let settings = read_recording_settings(state.inner());
    let ai_runtime = settings.ai_runtime;
    let user_context = settings.user_context;

    // Two-layer gate, mirroring the reason-code shape: continuous derivation is
    // "available" only when User Context's own opt-in is on AND the shared
    // engine-configured prerequisite is satisfied. The opt-in is checked first so
    // a user who configured the engine for Ask AI but not User Context sees the
    // distinct `user_context_disabled` reason rather than an engine reason.
    let (engine_available, reason) = if !user_context.enabled {
        (false, Some("user_context_disabled".to_string()))
    } else {
        match crate::ai_runtime::engine_configured_prerequisite(&ai_runtime).await {
            Ok(()) => (true, None),
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

    // The most recent completed distillation pass with its per-gate withheld
    // counts — lets the readout explain a thin dossier instead of staying mute.
    let last_distillation = store
        .latest_distillation_summary()
        .await
        .ok()
        .flatten()
        .map(|(at_ms, conclusions_derived, drops)| UserContextDistillationSummary {
            at_ms,
            conclusions_derived,
            ungrounded: drops.ungrounded,
            guardrail_suppressed: drops.guardrail_suppressed,
            below_formation_bar: drops.below_formation_bar,
            resurface_blocked: drops.resurface_blocked,
        });

    // --- backfilling progress (#98) ---
    // Cheap (two store reads): backfilling is true only when the engine is
    // available AND the background History Backfill still has older history to
    // reach. Two shapes count as "still backfilling":
    //   1. No windowed coverage exists yet but there ARE captures — the forward
    //      pass has not even seeded coverage, so older history is pending.
    //   2. Coverage exists but its oldest-covered edge is still above the floor,
    //      AND there is older captured history below that edge to derive
    //      (earliest_capture < oldest_covered). Without older captures there is
    //      nothing to backfill even if the floor is lower.
    // The floor is resolved from the SAME settings the worker uses.
    let backfilling = if !engine_available {
        false
    } else {
        let oldest_covered = store
            .oldest_derivation_run_window_start()
            .await
            .map_err(|e| e.to_string())?;
        let earliest_capture = store
            .earliest_capture_at_ms()
            .await
            .map_err(|e| e.to_string())?;
        match oldest_covered {
            // Shape 1: nothing windowed has run yet — backfilling iff captures exist.
            None => earliest_capture.is_some(),
            // Shape 2: coverage exists — older history remains iff the floor is
            // below the trailing edge AND there are captures older than it.
            Some(oldest) => {
                let floor_ms = backfill_floor_ms(
                    now_ms(),
                    user_context.backfill_window_days,
                    user_context.backfill_go_deeper,
                    earliest_capture,
                );
                oldest > floor_ms && earliest_capture.map_or(false, |earliest| earliest < oldest)
            }
        }
    };

    Ok(UserContextStatus {
        engine_available,
        reason,
        activity_count,
        conclusion_count,
        last_derived_at_ms,
        backfilling,
        token_usage,
        budget_tier: user_context.derivation_budget_tier,
        last_distillation,
    })
}

/// The most-recently-derived Activities (newest first) for the preview list.
///
/// When BOTH `start_ms` and `end_ms` are supplied, returns EVERY Activity
/// overlapping the half-open `[start_ms, end_ms)` window (oldest-first), so a
/// range-scoped view (e.g. Overview's selected day/week/month) gets the whole
/// period rather than a recency-capped slice — fixing the case where a busy
/// month exceeds the limit or a past range falls outside the newest page.
/// Range results are NOT evidence-hydrated (the range consumers read only the
/// Activity's own fields; skipping hydration keeps a month-wide fetch cheap).
///
/// With no bounds it keeps the legacy `limit`/`offset` recency-paged behavior
/// (newest-first, evidence hydrated) for the preview / evidence-resolution
/// callers that depend on `activity.evidence`.
#[tauri::command]
pub async fn list_user_context_activities(
    infra: tauri::State<'_, AppInfraState>,
    limit: Option<i64>,
    offset: Option<i64>,
    start_ms: Option<i64>,
    end_ms: Option<i64>,
) -> Result<Vec<Activity>, String> {
    if let (Some(start_ms), Some(end_ms)) = (start_ms, end_ms) {
        return infra
            .user_context()
            .list_activities_in_range(start_ms, end_ms)
            .await
            .map_err(|e| e.to_string());
    }
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
    start_ms: Option<i64>,
    end_ms: Option<i64>,
) -> Result<Vec<Conclusion>, String> {
    let include_faded = include_faded.unwrap_or(false);
    // Range-scoped (Overview feed) returns only Conclusions with a delta in the
    // window — bounded as the dossier grows. Absent bounds (Subjects, brokered
    // access) return the whole dossier, as before.
    if let (Some(start_ms), Some(end_ms)) = (start_ms, end_ms) {
        return infra
            .user_context()
            .list_conclusions_in_range(include_faded, start_ms, end_ms)
            .await
            .map_err(|e| e.to_string());
    }
    infra
        .user_context()
        .list_conclusions(include_faded)
        .await
        .map_err(|e| e.to_string())
}

/// The Subject page: every non-dismissed **Conclusion** about a Subject (faded
/// included) plus its confidence trajectories — the per-Conclusion
/// confidence-over-time lines drawn from **Confidence History** (#95). This is
/// the literal "warming up to a thing then cooling off" picture: each trajectory
/// is one Conclusion's snapshot series.
#[tauri::command]
pub async fn get_user_context_subject(
    infra: tauri::State<'_, AppInfraState>,
    subject: String,
) -> Result<SubjectView, String> {
    let store = infra.user_context();
    let conclusions = store
        .list_conclusions_for_subject(&subject)
        .await
        .map_err(|e| e.to_string())?;

    // One trajectory per Conclusion, built from its Confidence History snapshots.
    let mut trajectories: Vec<SubjectTrajectory> = Vec::with_capacity(conclusions.len());
    for conclusion in &conclusions {
        let history = store
            .list_confidence_history(conclusion.id)
            .await
            .map_err(|e| e.to_string())?;
        trajectories.push(SubjectTrajectory {
            conclusion_id: conclusion.id,
            statement: conclusion.statement.clone(),
            history,
        });
    }

    Ok(SubjectView {
        subject,
        conclusions,
        trajectories,
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
    let user_context = settings.user_context;

    // Same two-layer gate as the worker: the User Context opt-in must be on, and
    // the shared engine-configured prerequisite must be satisfied. A manual run
    // honours the opt-in so it never derives behind a user who has not opted in.
    if !user_context.enabled {
        return Ok(UserContextDerivationRunResult {
            activities_derived: 0,
            conclusions_derived: 0,
            window_start_ms: 0,
            window_end_ms: 0,
            items_read: 0,
            message: "User Context is off. Turn it on to derive Activities.".to_string(),
        });
    }

    if let Err(reason) = crate::ai_runtime::engine_configured_prerequisite(&ai_runtime).await {
        return Ok(UserContextDerivationRunResult {
            activities_derived: 0,
            conclusions_derived: 0,
            window_start_ms: 0,
            window_end_ms: 0,
            items_read: 0,
            message: if reason == "ai_runtime_disabled" {
                "The Reasoning Engine is off. Enable it to derive Activities.".to_string()
            } else {
                format!("The Reasoning Engine is not ready ({reason}).")
            },
        });
    }

    // Background derivation always runs on the global default model (no pin,
    // no feature override).
    let engine = match crate::ai_runtime::resolve_engine_config(&ai_runtime, None, None) {
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
    let distillation = run_conclusion_distillation(
        &engine,
        infra.user_context(),
        provider_label,
        model_label,
    )
    .await;
    // "Conclusions upserted" = inserts AND updates. A distillation pass that
    // refreshes existing Conclusions on fresh evidence upserts via UPDATE (no new
    // row), so a count_conclusions before/after delta would report 0 while real
    // work happened. Use the outcome's own upserted count (also what the worker
    // stamps on its ledger row).
    let conclusions_derived = distillation.map_or(0, |outcome| outcome.upserted as i64);

    let distilled_changed = distillation.is_some_and(|outcome| outcome.upserted > 0);
    if run.changed || distilled_changed {
        let _ = app_handle.emit(USER_CONTEXT_CHANGED_EVENT, ());
    }

    // Surface what distillation withheld so a manual run that "produced
    // nothing" tells the user why instead of looking like a silent no-op.
    let mut message = run.message;
    if let Some(summary) = distillation
        .map(|outcome| outcome.gate_drops)
        .filter(|drops| drops.total() > 0)
        .map(withheld_summary)
    {
        message.push(' ');
        message.push_str(&summary);
    }

    Ok(UserContextDerivationRunResult {
        activities_derived: run.activities_derived,
        conclusions_derived,
        window_start_ms: run.window_start_ms,
        window_end_ms: run.window_end_ms,
        items_read: run.items_read,
        message,
    })
}

/// Plain-language sentence for the per-gate withheld counts of one distillation
/// pass, e.g. "Withheld 3 conclusion drafts: 1 by the privacy guardrail, 2
/// needing more evidence." Caller guarantees `drops.total() > 0`.
fn withheld_summary(drops: app_infra::DistillationGateDrops) -> String {
    let mut reasons: Vec<String> = Vec::new();
    if drops.guardrail_suppressed > 0 {
        reasons.push(format!("{} by the privacy guardrail", drops.guardrail_suppressed));
    }
    if drops.below_formation_bar > 0 {
        reasons.push(format!("{} needing more evidence", drops.below_formation_bar));
    }
    if drops.resurface_blocked > 0 {
        reasons.push(format!("{} honoring a dismissal", drops.resurface_blocked));
    }
    if drops.ungrounded > 0 {
        reasons.push(format!("{} without grounding", drops.ungrounded));
    }
    let total = drops.total();
    format!(
        "Withheld {total} conclusion draft{}: {}.",
        if total == 1 { "" } else { "s" },
        reasons.join(", ")
    )
}

/// The **User Context Digest** (#89): the engine-written 2–4 sentence narrative
/// lede the Insights Overview shows for one local-calendar range. Lazy +
/// fingerprint-cached: an unchanged range returns the stored narrative with no
/// engine call. `Ok(None)` — never an error — when User Context is off, the
/// engine is off/unready, or the range holds fewer than two Activities, so the
/// frontend silently omits the lede. `range_kind` is `"day"` | `"week"` | `"month"`; `[start_ms,
/// end_ms)` is the half-open local-calendar window the frontend computed
/// (invoked as `{ rangeKind, startMs, endMs }`).
#[tauri::command]
pub async fn get_user_context_digest(
    state: tauri::State<'_, RecordingSettingsState>,
    infra: tauri::State<'_, AppInfraState>,
    range_kind: String,
    start_ms: i64,
    end_ms: i64,
) -> Result<Option<UserContextDigest>, String> {
    let settings = read_recording_settings(state.inner());
    super::digest::get_or_generate_digest(
        &settings.ai_runtime,
        settings.user_context.enabled,
        infra.user_context(),
        &range_kind,
        start_ms,
        end_ms,
        false,
    )
    .await
}

/// **Re-read**: force a fresh User Context Digest for one Insights Overview
/// range, ignoring the fingerprint cache and the freshness floor. Backs the
/// Overview's re-digest button.
///
/// Unlike [`get_user_context_digest`], which collapses any failure into a silent
/// `Ok(None)` lede omission, this is an explicit user action — an `Err` here is
/// surfaced to the user (e.g. "The AI provider rejected your API key") so a
/// digest that never appears stops being a mystery. `Ok(None)` still means the
/// range genuinely has no read to write (User Context off, engine unready, or
/// fewer than two Activities).
#[tauri::command]
pub async fn regenerate_user_context_digest(
    state: tauri::State<'_, RecordingSettingsState>,
    infra: tauri::State<'_, AppInfraState>,
    range_kind: String,
    start_ms: i64,
    end_ms: i64,
) -> Result<Option<UserContextDigest>, String> {
    let settings = read_recording_settings(state.inner());
    super::digest::get_or_generate_digest(
        &settings.ai_runtime,
        settings.user_context.enabled,
        infra.user_context(),
        &range_kind,
        start_ms,
        end_ms,
        true,
    )
    .await
}

/// **Dismiss** a Conclusion (#99): record its **Dismissal State** (which evidence,
/// when) and remove it from the dossier in one transaction. A dismissed Conclusion
/// is a user correction ("you're wrong") with a high resurface bar — it returns
/// only on substantially more *fresh* evidence, never from the same evidence just
/// rejected (enforced at derivation). Emits `user_context_changed` so the surface
/// refreshes and the row disappears.
#[tauri::command]
pub async fn user_context_dismiss_conclusion(
    app_handle: tauri::AppHandle,
    infra: tauri::State<'_, AppInfraState>,
    id: i64,
) -> Result<(), String> {
    infra
        .user_context()
        .dismiss_conclusion(id)
        .await
        .map_err(|e| e.to_string())?;
    let _ = app_handle.emit(USER_CONTEXT_CHANGED_EVENT, ());
    Ok(())
}

/// Collapse the raw, newest-first dismissal list into one render-only
/// [`DismissedView`] per belief. A belief dismissed more than once accrues
/// duplicate veto rows; keyed case-insensitively on `(subject, statement)` (the
/// resurface-gate identity), the first occurrence — the newest, since the input
/// is `dismissed_at_ms DESC` — wins, preserving newest-first order.
fn dedupe_dismissed(dismissals: Vec<DismissalState>) -> Vec<DismissedView> {
    let mut seen = std::collections::HashSet::new();
    let mut views = Vec::new();
    for dismissal in dismissals {
        let key = (
            dismissal.subject.to_lowercase(),
            dismissal.statement.to_lowercase(),
        );
        if seen.insert(key) {
            views.push(DismissedView {
                subject: dismissal.subject,
                statement: dismissal.statement,
                dismissed_at_ms: dismissal.dismissed_at_ms,
            });
        }
    }
    views
}

/// List the user's **dismissed beliefs** for the Context "Dismissed" archive —
/// the negative space of the inferred dossier ("what you told Mnema you're
/// not"). Deduplicated by `(subject, statement)`, newest first.
#[tauri::command]
pub async fn user_context_list_dismissed(
    infra: tauri::State<'_, AppInfraState>,
) -> Result<Vec<DismissedView>, String> {
    let dismissals = infra
        .user_context()
        .list_dismissals()
        .await
        .map_err(|e| e.to_string())?;
    Ok(dedupe_dismissed(dismissals))
}

/// **Restore** a dismissed belief: lift the suppression veto (all matching rows)
/// so the Conclusion can re-form on the next derivation pass IF its evidence
/// still supports it. It does NOT resurrect the old Conclusion (that row was
/// deleted at dismiss time). Emits `user_context_changed` so the archive
/// refreshes.
#[tauri::command]
pub async fn user_context_restore_dismissed(
    app_handle: tauri::AppHandle,
    infra: tauri::State<'_, AppInfraState>,
    subject: String,
    statement: String,
) -> Result<(), String> {
    infra
        .user_context()
        .undismiss(&subject, &statement)
        .await
        .map_err(|e| e.to_string())?;
    let _ = app_handle.emit(USER_CONTEXT_CHANGED_EVENT, ());
    Ok(())
}

/// **Pin** / unpin a Conclusion (#99): a pinned Conclusion is exempt from
/// confidence decay so it does not quietly fade during a quiet stretch. Emits
/// `user_context_changed` so the surface reflects the new pinned state.
#[tauri::command]
pub async fn user_context_set_pinned(
    app_handle: tauri::AppHandle,
    infra: tauri::State<'_, AppInfraState>,
    id: i64,
    pinned: bool,
) -> Result<(), String> {
    infra
        .user_context()
        .set_pinned(id, pinned)
        .await
        .map_err(|e| e.to_string())?;
    let _ = app_handle.emit(USER_CONTEXT_CHANGED_EVENT, ());
    Ok(())
}

/// **Correct** an Activity's Category (#108). The user override always WINS over
/// the engine label on read, and is fed back into the next derivation pass so the
/// engine is biased away from regenerating the corrected-away label. `category`
/// is the new effective Category; `null`/absent corrects it to "unset" (an
/// intentional clear, distinct from "never corrected"). Emits
/// `user_context_changed` so the surface re-reads the effective label.
#[tauri::command]
pub async fn user_context_correct_activity_category(
    app_handle: tauri::AppHandle,
    infra: tauri::State<'_, AppInfraState>,
    id: i64,
    category: Option<ActivityCategory>,
) -> Result<(), String> {
    infra
        .user_context()
        // `Some(category)` = "set this correction" (where `category` may itself be
        // None = corrected to unset); the focus arg is `None` = "leave unchanged".
        .correct_activity(id, Some(category), None)
        .await
        .map_err(|e| e.to_string())?;
    let _ = app_handle.emit(USER_CONTEXT_CHANGED_EVENT, ());
    Ok(())
}

/// **Correct** an Activity's Focus Classification (#108). The user override always
/// WINS over the engine label on read and feeds back into the next derivation
/// pass. `focus` is the new effective Focus; `null`/absent corrects it to "unset".
/// Emits `user_context_changed`.
#[tauri::command]
pub async fn user_context_correct_activity_focus(
    app_handle: tauri::AppHandle,
    infra: tauri::State<'_, AppInfraState>,
    id: i64,
    focus: Option<FocusLevel>,
) -> Result<(), String> {
    infra
        .user_context()
        .correct_activity(id, None, Some(focus))
        .await
        .map_err(|e| e.to_string())?;
    let _ = app_handle.emit(USER_CONTEXT_CHANGED_EVENT, ());
    Ok(())
}

/// List every **user-authored Context** statement (#107), newest first. These are
/// standing statements the user wrote about themselves; they are user-asserted, so
/// they carry no confidence and never decay.
#[tauri::command]
pub async fn list_user_context_authored(
    infra: tauri::State<'_, AppInfraState>,
) -> Result<Vec<AuthoredContext>, String> {
    infra
        .user_context()
        .list_authored_context()
        .await
        .map_err(|e| e.to_string())
}

/// Add a **user-authored Context** statement (#107). Stored verbatim with an
/// optional `topic` grouping handle; stamped with the same `now_ms` time source the
/// rest of the module uses. Returns the persisted row and emits
/// `user_context_changed` so the surface refreshes.
#[tauri::command]
pub async fn user_context_add_authored(
    app_handle: tauri::AppHandle,
    infra: tauri::State<'_, AppInfraState>,
    text: String,
    topic: Option<String>,
) -> Result<AuthoredContext, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Authored context text cannot be empty.".to_string());
    }
    let now = now_ms();
    let topic = topic.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    let store = infra.user_context();
    let id = store
        .add_authored_context(text, topic.as_deref(), now)
        .await
        .map_err(|e| e.to_string())?;
    let _ = app_handle.emit(USER_CONTEXT_CHANGED_EVENT, ());
    Ok(AuthoredContext {
        id,
        text: text.to_string(),
        topic,
        created_at_ms: now,
        updated_at_ms: now,
    })
}

/// Update a **user-authored Context** statement's text/topic (#107), bumping its
/// `updated_at_ms`. Emits `user_context_changed`.
#[tauri::command]
pub async fn user_context_update_authored(
    app_handle: tauri::AppHandle,
    infra: tauri::State<'_, AppInfraState>,
    id: i64,
    text: String,
    topic: Option<String>,
) -> Result<(), String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Authored context text cannot be empty.".to_string());
    }
    let topic = topic.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    infra
        .user_context()
        .update_authored_context(id, text, topic.as_deref(), now_ms())
        .await
        .map_err(|e| e.to_string())?;
    let _ = app_handle.emit(USER_CONTEXT_CHANGED_EVENT, ());
    Ok(())
}

/// Delete a **user-authored Context** statement (#107). Emits
/// `user_context_changed`.
#[tauri::command]
pub async fn user_context_delete_authored(
    app_handle: tauri::AppHandle,
    infra: tauri::State<'_, AppInfraState>,
    id: i64,
) -> Result<(), String> {
    infra
        .user_context()
        .delete_authored_context(id)
        .await
        .map_err(|e| e.to_string())?;
    let _ = app_handle.emit(USER_CONTEXT_CHANGED_EVENT, ());
    Ok(())
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
///    single shared store, so the same control erases every derived/recalled
///    surface. Both data wipes run even if the first one errors, so a single
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
    wipe_context.map_err(|e| e.to_string())?;
    wipe_conversations.map_err(|e| e.to_string())?;

    // 4. Refresh the (now empty) User Context surface.
    let _ = app_handle.emit(USER_CONTEXT_CHANGED_EVENT, ());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use capture_types::DismissalState;

    fn dismissal(subject: &str, statement: &str, dismissed_at_ms: i64) -> DismissalState {
        DismissalState {
            subject: subject.to_string(),
            statement: statement.to_string(),
            evidence_fingerprint: "fp".to_string(),
            evidence_activity_count: 1,
            dismissed_at_ms,
        }
    }

    #[test]
    fn dedupe_dismissed_collapses_duplicates_keeping_newest() {
        // Newest-first global order, as `list_dismissals` returns it.
        let input = vec![
            dismissal("Apple", "Interested in Apple", 200),
            dismissal("Rust", "Learning Rust", 150),
            dismissal("Apple", "Interested in Apple", 100),
        ];

        let views = dedupe_dismissed(input);

        // One entry per (subject, statement); the Apple duplicate collapses to its
        // newest dismissal; newest-first order is preserved.
        assert_eq!(views.len(), 2);
        assert_eq!(views[0].subject, "Apple");
        assert_eq!(views[0].dismissed_at_ms, 200);
        assert_eq!(views[1].subject, "Rust");
    }

    #[test]
    fn dedupe_dismissed_keys_case_insensitively() {
        let input = vec![
            dismissal("Apple", "Interested in Apple", 200),
            dismissal("apple", "INTERESTED IN APPLE", 100),
        ];

        let views = dedupe_dismissed(input);

        assert_eq!(views.len(), 1, "case variants are the same belief");
        assert_eq!(views[0].dismissed_at_ms, 200);
    }
}
