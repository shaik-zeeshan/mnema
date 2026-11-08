//! The User Context edit commands: digest reads, dismissals, pins, corrections
//! and Authored Context.

use super::*;

/// The **User Context Digest** (#89): the engine-written short (at most 2 sentences) narrative
/// lede the Insights Overview shows for one local-calendar range. Lazy +
/// fingerprint-cached: an unchanged range returns the stored narrative with no
/// engine call. Returns `{ digest }`, or `{ reason }` — `derivation_off` (User
/// Context off), `engine_not_ready`, or `too_few` (under two Activities). `Err` for a failed
/// generation or `digest::DIGEST_SENSITIVE_HOLD` — replayed from a 15-minute
/// in-memory cooldown on an unchanged input set, so a failure doesn't re-bill
/// the engine on every visit. `range_kind` is `"day"` | `"week"` | `"month"`; `[start_ms,
/// end_ms)` is the half-open local-calendar window the frontend computed
/// (invoked as `{ rangeKind, startMs, endMs }`).
pub async fn get_user_context_digest(
    core: &AppCore,
    infra: &AppInfraState,
    range_kind: String,
    start_ms: i64,
    end_ms: i64,
) -> Result<UserContextDigestResult, String> {
    let settings = current_recording_settings(&core.state::<RecordingSettingsState>());
    crate::user_context::digest::get_or_generate_digest(
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
/// Same outcomes as [`get_user_context_digest`], but it bypasses the failure
/// cooldown: a re-read always calls the engine.
pub async fn regenerate_user_context_digest(
    core: &AppCore,
    infra: &AppInfraState,
    range_kind: String,
    start_ms: i64,
    end_ms: i64,
) -> Result<UserContextDigestResult, String> {
    let settings = current_recording_settings(&core.state::<RecordingSettingsState>());
    crate::user_context::digest::get_or_generate_digest(
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
pub async fn user_context_dismiss_conclusion(
    core: &AppCore,
    infra: &AppInfraState,
    id: i64,
) -> Result<(), String> {
    let dismissed_subject = infra
        .user_context()
        .dismiss_conclusion(id)
        .await
        .map_err(|e| e.to_string())?;
    // Lazy re-embed (slice 4): dismissing a Conclusion can change which row is the
    // Subject's canonical statement, so the embedding text the Subject Vector
    // worker derives may have changed. Mark the Subject's vector stale (NULL it) so
    // the backfill worker re-claims and re-embeds it. Best-effort: a failure here
    // never blocks the dismissal (the worker also refills genuinely-missing
    // vectors, the load-bearing mechanism). NULL-ing is pure storage, not embedding
    // work, so app-infra stays embedding-free.
    if let Some(subject) = dismissed_subject {
        if let Err(error) = infra
            .subject_vectors()
            .mark_subject_vector_stale(&subject)
            .await
        {
            crate::debug_log::log_warn(format!(
                "failed to mark subject vector stale after dismissing conclusion {id}: {error}"
            ));
        }
    }
    core.host().emit(CoreEvent::UserContextChanged);
    Ok(())
}

/// Collapse the raw, newest-first dismissal list into one render-only
/// [`DismissedView`] per belief. A belief dismissed more than once accrues
/// duplicate veto rows; keyed case-insensitively on `(subject, statement)` (the
/// resurface-gate identity), the first occurrence — the newest, since the input
/// is `dismissed_at_ms DESC` — wins, preserving newest-first order.
pub(super) fn dedupe_dismissed(dismissals: Vec<DismissalState>) -> Vec<DismissedView> {
    let mut seen = std::collections::HashSet::new();
    let mut views = Vec::new();
    for dismissal in dismissals {
        // ASCII-only folding to match SQLite `COLLATE NOCASE` — the identity used
        // by the conclusions unique index, the resurface gate, and `undismiss`.
        // Full Unicode `to_lowercase` over-folds (e.g. É == é), collapsing two
        // distinct vetoes that `undismiss` can only delete one casing at a time,
        // leaving a stale, un-restorable suppression behind.
        let key = (
            dismissal.subject.to_ascii_lowercase(),
            dismissal.statement.to_ascii_lowercase(),
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
pub async fn user_context_list_dismissed(
    infra: &AppInfraState,
) -> Result<Vec<DismissedView>, String> {
    let dismissals = infra
        .user_context()
        .list_user_dismissals()
        .await
        .map_err(|e| e.to_string())?;
    Ok(dedupe_dismissed(dismissals))
}

/// **Restore** a dismissed belief: lift the suppression veto (all matching rows)
/// so the Conclusion can re-form on the next derivation pass IF its evidence
/// still supports it. It does NOT resurrect the old Conclusion (that row was
/// deleted at dismiss time). Emits `user_context_changed` so the archive
/// refreshes.
pub async fn user_context_restore_dismissed(
    core: &AppCore,
    infra: &AppInfraState,
    subject: String,
    statement: String,
) -> Result<(), String> {
    infra
        .user_context()
        .undismiss(&subject, &statement)
        .await
        .map_err(|e| e.to_string())?;
    core.host().emit(CoreEvent::UserContextChanged);
    Ok(())
}

/// **Pin** / unpin a Conclusion (#99): a pinned Conclusion is exempt from
/// confidence decay so it does not quietly fade during a quiet stretch. Emits
/// `user_context_changed` so the surface reflects the new pinned state.
pub async fn user_context_set_pinned(
    core: &AppCore,
    infra: &AppInfraState,
    id: i64,
    pinned: bool,
) -> Result<(), String> {
    infra
        .user_context()
        .set_pinned(id, pinned)
        .await
        .map_err(|e| e.to_string())?;
    core.host().emit(CoreEvent::UserContextChanged);
    Ok(())
}

/// **Correct** an Activity's Category (#108). The user override always WINS over
/// the engine label on read, and is fed back into the next derivation pass so the
/// engine is biased away from regenerating the corrected-away label. `category`
/// is the new effective Category; `null`/absent corrects it to "unset" (an
/// intentional clear, distinct from "never corrected"). Emits
/// `user_context_changed` so the surface re-reads the effective label.
pub async fn user_context_correct_activity_category(
    core: &AppCore,
    infra: &AppInfraState,
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
    core.host().emit(CoreEvent::UserContextChanged);
    Ok(())
}

/// **Correct** an Activity's Focus Classification (#108). The user override always
/// WINS over the engine label on read and feeds back into the next derivation
/// pass. `focus` is the new effective Focus; `null`/absent corrects it to "unset".
/// Emits `user_context_changed`.
pub async fn user_context_correct_activity_focus(
    core: &AppCore,
    infra: &AppInfraState,
    id: i64,
    focus: Option<FocusLevel>,
) -> Result<(), String> {
    infra
        .user_context()
        .correct_activity(id, None, Some(focus))
        .await
        .map_err(|e| e.to_string())?;
    core.host().emit(CoreEvent::UserContextChanged);
    Ok(())
}

/// One authored statement plus whether the engine actually reads it: `in_prompt`
/// is false for statements past the 4,000-char authored-context cap (CX-06).
#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AuthoredContextRow {
    #[serde(flatten)]
    pub item: AuthoredContext,
    pub in_prompt: bool,
}

/// List every **user-authored Context** statement (#107), most recently edited
/// first. These are
/// standing statements the user wrote about themselves; they are user-asserted, so
/// they carry no confidence and never decay — but only the newest that fit the
/// prompt cap are read, flagged per row via `inPrompt`.
pub async fn list_user_context_authored(
    infra: &AppInfraState,
) -> Result<Vec<AuthoredContextRow>, String> {
    let authored = infra
        .user_context()
        .list_authored_context()
        .await
        .map_err(|e| e.to_string())?;
    let lines = crate::user_context::derivation::authored_prompt_lines(&authored);
    Ok(authored
        .into_iter()
        .zip(lines)
        .map(|(item, line)| AuthoredContextRow {
            item,
            in_prompt: line.is_some(),
        })
        .collect())
}

/// Longest authored statement, in chars (ruling #15).
pub(super) const AUTHORED_STATEMENT_CHAR_LIMIT: usize = 280;

/// The fixed authored topics (ruling #9). The column stays free TEXT.
pub(super) const AUTHORED_TOPICS: [&str; 4] = ["Work", "Preferences", "People", "Goals"];

/// Shared add/update validation: trimmed, non-empty, at most
/// [`AUTHORED_STATEMENT_CHAR_LIMIT`] chars; topic one of [`AUTHORED_TOPICS`] or
/// none (blank counts as none).
pub(super) fn validate_authored<'a>(
    text: &'a str,
    topic: Option<&'a str>,
) -> Result<(&'a str, Option<&'a str>), String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Authored context text cannot be empty.".to_string());
    }
    if text.chars().count() > AUTHORED_STATEMENT_CHAR_LIMIT {
        return Err(format!(
            "Keep a statement to {AUTHORED_STATEMENT_CHAR_LIMIT} characters or fewer."
        ));
    }
    match topic.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) if !AUTHORED_TOPICS.contains(&t) => Err(format!(
            "Unknown topic \"{t}\": use Work, Preferences, People or Goals."
        )),
        topic => Ok((text, topic)),
    }
}

/// Add a **user-authored Context** statement (#107). Stored verbatim with an
/// optional `topic` (one of [`AUTHORED_TOPICS`]), at most
/// [`AUTHORED_STATEMENT_CHAR_LIMIT`] chars; stamped with the same `now_ms` time source the
/// rest of the module uses. Returns the persisted row and emits
/// `user_context_changed` so the surface refreshes.
pub async fn user_context_add_authored(
    core: &AppCore,
    infra: &AppInfraState,
    text: String,
    topic: Option<String>,
) -> Result<AuthoredContext, String> {
    let (text, topic) = validate_authored(&text, topic.as_deref())?;
    let now = now_ms();
    let store = infra.user_context();
    let id = store
        .add_authored_context(text, topic, now)
        .await
        .map_err(|e| e.to_string())?;
    core.host().emit(CoreEvent::UserContextChanged);
    Ok(AuthoredContext {
        id,
        text: text.to_string(),
        topic: topic.map(str::to_string),
        created_at_ms: now,
        updated_at_ms: now,
    })
}

/// Update a **user-authored Context** statement's text/topic (#107), bumping its
/// `updated_at_ms`. Emits `user_context_changed`.
pub async fn user_context_update_authored(
    core: &AppCore,
    infra: &AppInfraState,
    id: i64,
    text: String,
    topic: Option<String>,
) -> Result<(), String> {
    let (text, topic) = validate_authored(&text, topic.as_deref())?;
    infra
        .user_context()
        .update_authored_context(id, text, topic, now_ms())
        .await
        .map_err(|e| e.to_string())?;
    core.host().emit(CoreEvent::UserContextChanged);
    Ok(())
}

/// Delete a **user-authored Context** statement (#107). Emits
/// `user_context_changed`.
pub async fn user_context_delete_authored(
    core: &AppCore,
    infra: &AppInfraState,
    id: i64,
) -> Result<(), String> {
    infra
        .user_context()
        .delete_authored_context(id)
        .await
        .map_err(|e| e.to_string())?;
    core.host().emit(CoreEvent::UserContextChanged);
    Ok(())
}

/// Record the frontend's current local UTC offset (minutes to ADD to UTC) so
/// the distillation worker can label Activity times in the user's local clock.
/// The Rust `time` crate can't soundly read the local offset under Tauri, so the
/// frontend is the sound source (mirrors `askAiClock`). Invoked on app start and
/// window focus; DST staleness between stamps is accepted.
pub async fn user_context_stamp_local_offset(
    infra: &AppInfraState,
    offset_minutes: i64,
) -> Result<(), String> {
    infra
        .user_context()
        .set_local_offset_minutes(offset_minutes)
        .await
        .map_err(|e| e.to_string())
}
