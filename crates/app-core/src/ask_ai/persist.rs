//! The live update emit, turn persistence and conversation title generation.

use super::*;

/// Apply `update` to the conversation's LiveTurn and, when this turn still owns
/// it, emit the versioned [`ASK_AI_UPDATE_EVENT`]. Returns the version emitted (so
/// the driver can track `last_version` for the displaced-terminal path), or `None`
/// when this turn was displaced — in which case NOTHING is emitted (the newer turn
/// owns the conversation's update stream).
pub fn emit_live_update(
    core: &AppCore,
    conversation_id: &str,
    turn_token: u64,
    update: TurnUpdate,
) -> Option<u64> {
    let (version, turn_index, update) = apply_live_update(conversation_id, turn_token, update)?;
    core.host().emit(CoreEvent::AskAiUpdate(AskAiUpdateEvent {
        conversation_id: conversation_id.to_string(),
        version: version,
        turn_index: turn_index,
        update: update,
    }));
    Some(version)
}

/// Persist one turn row in whatever phase the driver is in. Best-effort: a store
/// error is logged, never surfaced — the live stream events are authoritative and
/// persistence is a reattach convenience. Returns `true` when the row was
/// written, so the driver can announce the conversation to the history list the
/// first time the row actually exists.
#[allow(clippy::too_many_arguments)]
pub async fn persist_turn(
    infra: &AppInfraState,
    conversation_id: &str,
    title: &str,
    origin: &str,
    turn_index: i64,
    question: &str,
    answer: &str,
    reasoning: Option<&str>,
    blocks: Option<&[capture_types::AnswerBlock]>,
    tool_activities: &[serde_json::Value],
    sources: &[serde_json::Value],
    phase: &str,
    error: Option<(&str, &str)>,
) -> bool {
    let tool_activities_json =
        serde_json::to_string(tool_activities).unwrap_or_else(|_| "[]".into());
    let sources_json = serde_json::to_string(sources).unwrap_or_else(|_| "[]".into());
    match infra
        .conversation()
        .save_turn(
            conversation_id,
            title,
            origin,
            turn_index,
            question,
            answer,
            reasoning,
            blocks,
            &tool_activities_json,
            &sources_json,
            phase,
            error,
            now_ms(),
        )
        .await
    {
        Ok(()) => true,
        Err(error) => {
            log::error!(
                "Ask AI failed to persist turn {turn_index} for {conversation_id} (phase {phase}): {error}"
            );
            false
        }
    }
}

/// Structured-extraction target for the generated thread title.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct GeneratedConversationTitle {
    /// A 3–6 word title summarizing the conversation topic.
    pub title: String,
}

/// Generated titles are first-turn-only: a thread is eligible exactly when the
/// completed turn was its first (`turn_index == 0`). Later turns never
/// (re)generate; the store-side conditional write additionally guards "no
/// title exists yet" and "a user rename wins forever".
pub fn title_generation_eligible(turn_index: i64) -> bool {
    turn_index == 0
}

/// System instruction for the title extraction. The prompt sees ONLY the
/// user's question text — no capture data, no transcript — so this call adds
/// no new redaction surface.
pub fn build_title_preamble() -> &'static str {
    "You name chat threads. Produce a short 3-6 word title that captures the topic of the \
user's question. Use plain words: no quotes, no trailing punctuation, no \"Question about\" \
prefix, and do not answer the question."
}

/// Per-call prompt for the title extraction: only the question text.
pub fn build_title_prompt(question: &str) -> String {
    format!("The user's first question to an assistant was:\n{question}\n\nTitle this thread.")
}

/// Normalize a model-produced title into an acceptable thread title, or `None`
/// when the result is unusable (empty, over-long) — an unusable result is a
/// swallowed failure that leaves the fallback first-question title in place.
pub fn normalize_generated_title(raw: &str) -> Option<String> {
    let trimmed = raw
        .trim()
        .trim_matches(|ch| matches!(ch, '"' | '\'' | '`' | '\u{201c}' | '\u{201d}'))
        .trim()
        .trim_end_matches(['.', '!'])
        .trim();
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    if words.is_empty() || words.len() > GENERATED_TITLE_MAX_WORDS {
        return None;
    }
    let title = words.join(" ");
    if title.chars().count() > GENERATED_TITLE_MAX_CHARS {
        return None;
    }
    Some(title)
}

/// Fire-and-forget generated thread title (PLAN slice 3, ADR 0034 chat rail).
///
/// Spawned AFTER a thread's first turn persists as terminal, so it can never
/// delay or fail the turn: every failure here (engine unresolved, extraction
/// error, unusable result, store error) is swallowed with at most a log line,
/// leaving the fallback first-question title in place. One cheap structured
/// `extract` against the GLOBAL DEFAULT engine (no thread pin, no Ask-AI model
/// override — titles are cosmetic, not part of the pinned conversation), whose
/// prompt sees ONLY the user's question text. The persist is the store's
/// conditional write, so a user rename that raced this generation wins and the
/// late generated title is dropped; only an actual write announces
/// `conversation_changed`.
pub async fn generate_conversation_title(
    core: Arc<AppCore>,
    infra: AppInfraState,
    conversation_id: String,
    question: String,
) {
    let settings = read_ai_runtime_settings(&core);
    let config = match crate::ai_runtime::resolve_engine_config_live(&settings, None, None).await {
        Ok(config) => config,
        Err(reason) => {
            log::debug!("Ask AI skipped title generation for {conversation_id}: {reason}");
            return;
        }
    };

    let prompt = build_title_prompt(&question);
    let extracted = match ai_engine::extract_with_preamble::<GeneratedConversationTitle>(
        &config,
        build_title_preamble(),
        &prompt,
    )
    .await
    {
        Ok(extracted) => extracted,
        Err(error) => {
            // Kind + the error's head only: the raw provider error can echo
            // prompt fragments, and user logs leave the machine.
            log::warn!(
                "Ask AI title generation failed for {conversation_id} ({}): {}",
                error.failure_kind(),
                error.log_detail()
            );
            return;
        }
    };

    let Some(title) = normalize_generated_title(&extracted.title) else {
        log::warn!(
            "Ask AI title generation for {conversation_id} returned an unusable title; keeping fallback"
        );
        return;
    };

    match infra
        .conversation()
        .set_generated_title_if_unset(&conversation_id, &title)
        .await
    {
        // Written: refresh the history list so the new title appears.
        Ok(true) => {
            core.host().emit(CoreEvent::ConversationChanged);
        }
        // No longer eligible (user renamed mid-flight, or a title already
        // exists): the user/earlier title wins silently.
        Ok(false) => {}
        Err(error) => {
            log::warn!("Ask AI failed to persist generated title for {conversation_id}: {error}");
        }
    }
}

/// The `TurnUpdate::Error` kind for an engine-resolve reason code: a rejected
/// ChatGPT login, an engine that didn't answer, or (everything else — missing
/// key/model/base URL, denied vault) a config problem fixed in Settings.
pub fn resolve_failure_kind(reason: &str) -> &'static str {
    if reason.starts_with("needs_reconnect:") {
        "reconnect"
    } else if reason.starts_with("provider_unreachable:") || reason == "local_endpoint_unreachable"
    {
        "unreachable"
    } else {
        "settings"
    }
}

/// Saves a turn whose engine resolve failed as a terminal `error` row, so the
/// backend's turn count matches the frontend's (CH-04).
pub async fn persist_resolve_failure(
    infra: &AppInfraState,
    conversation_id: &str,
    title: &str,
    origin: &str,
    turn_index: i64,
    question: &str,
    reason: &str,
) -> bool {
    persist_turn(
        infra,
        conversation_id,
        title,
        origin,
        turn_index,
        question,
        "",
        None,
        Some(&[]),
        &[],
        &[],
        "error",
        Some((reason, resolve_failure_kind(reason))),
    )
    .await
}

/// Saves a turn stopped before the model ran as a `done` row with no answer —
/// what finalize saves for a Stop that lands mid-turn — so the frontend's turn
/// count and Retry (`delete_last_turn`) match the store (CH-04). An abandoned
/// Quick Recall first question is deleted instead, exactly as at finalize.
pub async fn persist_stopped_before_model(
    infra: &AppInfraState,
    conversation_id: &str,
    title: &str,
    origin: &str,
    stored_origin: Option<&str>,
    turn_index: i64,
    question: &str,
) {
    persist_turn(
        infra,
        conversation_id,
        title,
        origin,
        turn_index,
        question,
        "",
        None,
        Some(&[]),
        &[],
        &[],
        "done",
        None,
    )
    .await;
    abandoned::discard_if_abandoned(
        infra,
        conversation_id,
        origin,
        stored_origin,
        turn_index,
        false,
        "",
        &[],
    )
    .await;
}
