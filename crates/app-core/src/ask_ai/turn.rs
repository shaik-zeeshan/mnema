//! One stateless Ask AI turn, start to terminal update.

use super::*;

/// The single stateless-per-turn Ask AI driver used by BOTH start and follow-up.
///
/// Loads the conversation's completed history + engine pin from the store,
/// resolves the engine through the single precedence chain (ADR 0034: thread pin
/// → Ask AI model override → global default model), persists a
/// `streaming` turn row, then runs ONE `ai_engine::run_agent_loop` against the
/// configured engine. The model's text streams as versioned `ask_ai_update`
/// view updates (and is periodically persisted as a partial for reattach); tool
/// calls run through the All-Retained broker seam Rust-side and drive the live
/// tool-activity line + source cards via the same update stream. On completion it
/// persists the final turn and emits the terminal `Done`/`Error` update. A
/// cooperative cancel keeps whatever was generated (phase `done`) and still
/// settles the view with a terminal `Done`.
///
/// Detached: the spawned task finishes regardless of dismiss/close, so an unseen
/// thread completes in the background and a reattach reads the persisted answer.
pub async fn run_ask_ai_turn(
    core: Arc<AppCore>,
    conversation_id: String,
    question: String,
    origin: String,
    title: String,
    clock: ClientClock,
    scope: Option<AskAiScope>,
    cancel: Arc<AtomicBool>,
) {
    // Mint this turn's unique LiveTurn ownership token. Held for the turn's life so
    // ownership checks (apply/remove) can tell THIS turn apart from a newer turn
    // that displaces it for the same conversation. The LiveTurn analogue of the
    // `cancel` Arc identity in the inflight registry.
    let turn_token = next_turn_token();

    // Resolve storage; without it we cannot persist or read history, so surface a
    // terminal error and stop. This is BEFORE any LiveTurn is registered, so
    // `emit_live_update` would be a no-op — emit a DIRECT terminal `ask_ai_update`
    // error so the frontend (which only listens to `ask_ai_update`) settles the
    // turn instead of hanging. `turn_index` is unknown here (infra is what loads
    // it), so use 0.
    let infra = match app_infra(&core) {
        Ok(infra) => infra,
        Err(error) => {
            core.host().emit(CoreEvent::AskAiUpdate(AskAiUpdateEvent {
                conversation_id: conversation_id.to_string(),
                version: 1u64,
                turn_index: 0i64,
                update: TurnUpdate::Error {
                    message: error,
                    kind: None,
                },
            }));
            remove_inflight_if_owner(&conversation_id, &cancel);
            return;
        }
    };

    // 1. Load prior conversation: completed Q/A turns become history (oldest
    //    first), and the count of existing turns is the next turn index. The
    //    engine pin (if any) is read alongside.
    let existing = infra
        .conversation()
        .get_conversation(&conversation_id)
        .await
        .ok()
        .flatten();
    let mut history: Vec<ai_engine::AgentHistoryTurn> = Vec::new();
    let mut turn_index: i64 = 0;
    if let Some(conversation) = existing.as_ref() {
        turn_index = conversation.turns.len() as i64;
        for turn in &conversation.turns {
            // Only completed turns with a real answer become history; an
            // in-flight/errored turn is not established context.
            if turn.phase == "done" && !turn.answer.trim().is_empty() {
                history.push(ai_engine::AgentHistoryTurn {
                    role: ai_engine::AgentRole::User,
                    text: turn.question.clone(),
                });
                history.push(ai_engine::AgentHistoryTurn {
                    role: ai_engine::AgentRole::Assistant,
                    text: turn.answer.clone(),
                });
            }
        }
    }
    let pin = infra
        .conversation()
        .get_conversation_engine(&conversation_id)
        .await
        .ok()
        .flatten();

    // 2. Resolve the engine through the single precedence chain (ADR 0034):
    //    thread pin → Ask AI model override (`access.askAiModel`, on its saved
    //    provider, or riding on the default model's provider when it has none)
    //    → global default model.
    let settings = read_recording_settings(&core);
    let pin_ref =
        pin.as_ref().and_then(
            |(provider, model)| match (provider.as_deref(), model.as_deref()) {
                (Some(provider), Some(model)) => Some((provider, model)),
                _ => None,
            },
        );
    let (pin_ref, override_model) =
        crate::ai_runtime::ask_ai_resolver_inputs(&settings.ai_runtime, &settings.access, pin_ref);
    let config_result =
        crate::ai_runtime::resolve_engine_ref_live(&settings.ai_runtime, pin_ref, override_model)
            .await;
    let (engine_provider, config) = match config_result {
        Ok(resolved) => resolved,
        Err(reason) => {
            // Still BEFORE the LiveTurn is registered, so emit a DIRECT terminal
            // `ask_ai_update` error (no live view exists for `emit_live_update` to
            // mutate). Here `turn_index` is known (history was loaded above).
            // Persist it as an `error` turn row first (CH-04): the frontend
            // already counts this turn, so an unsaved one desyncs every later
            // `turn_index` (and a failed first message would lose origin/title).
            persist_resolve_failure(
                &infra,
                &conversation_id,
                &title,
                &origin,
                turn_index,
                &question,
                &reason,
            )
            .await;
            core.host().emit(CoreEvent::ConversationChanged);
            // Clear the flag before the terminal emit, so a Retry on this row
            // never meets "a turn is still running" (see `emit_terminal`).
            remove_inflight_if_owner(&conversation_id, &cancel);
            core.host().emit(CoreEvent::AskAiUpdate(AskAiUpdateEvent {
                conversation_id: conversation_id.to_string(),
                version: 1u64,
                turn_index: turn_index,
                update: TurnUpdate::Error {
                    kind: Some(resolve_failure_kind(&reason).to_string()),
                    message: reason,
                },
            }));
            return;
        }
    };

    // 3. A cancel arriving before the model call short-circuits here. No LiveTurn
    //    is registered yet (that happens at step 4), so emit a direct terminal
    //    `ask_ai_update` Done — version 1, this turn's index — so any
    //    already-attached frontend view for this turn settles instead of hanging.
    //    The row is saved first (CH-04), so the frontend's turn count and a
    //    Retry still match the store.
    if cancel.load(Ordering::SeqCst) {
        persist_stopped_before_model(
            &infra,
            &conversation_id,
            &title,
            &origin,
            existing.as_ref().map(|c| c.origin.as_str()),
            turn_index,
            &question,
        )
        .await;
        core.host().emit(CoreEvent::ConversationChanged);
        remove_inflight_if_owner(&conversation_id, &cancel);
        core.host().emit(CoreEvent::AskAiUpdate(AskAiUpdateEvent {
            conversation_id: conversation_id.to_string(),
            version: 1u64,
            turn_index: turn_index,
            update: TurnUpdate::Done,
        }));
        return;
    }

    let engine_model = match &config {
        ai_engine::EngineConfig::Cloud { model, .. }
        | ai_engine::EngineConfig::Local { model, .. } => model.clone(),
    };

    // 4. Persist the turn row immediately (empty `streaming` answer) so a reattach
    //    can read the in-flight partial.
    //    The FIRST successful persist is when the conversation row exists, so it
    //    announces `conversation_changed` once — a brand-new chat appears in the
    //    history list while still streaming. The flag guards against re-announcing
    //    on every throttled partial persist; the terminal persist re-emits anyway.
    let conversation_announced = Arc::new(AtomicBool::new(false));
    if persist_turn(
        &infra,
        &conversation_id,
        &title,
        &origin,
        turn_index,
        &question,
        "",
        None,
        // A brand-new turn is never "legacy NULL" — an EMPTY parsed set.
        Some(&[]),
        &[],
        &[],
        "streaming",
        None,
    )
    .await
    {
        conversation_announced.store(true, Ordering::SeqCst);
        core.host().emit(CoreEvent::ConversationChanged);
        // Stamp the engine answering this turn, so a later model switch never
        // relabels this answer. Best-effort, like every persist here.
        if let Err(error) = infra
            .conversation()
            .set_turn_engine(
                &conversation_id,
                turn_index,
                &engine_provider,
                &engine_model,
            )
            .await
        {
            log::warn!(
                "Ask AI failed to stamp the engine on turn {turn_index} for {conversation_id}: {error}"
            );
        }
    }

    // Register the in-memory LiveTurn alongside the initial streaming row, at the
    // point `turn_index` is known. This view is what
    // `ask_ai_snapshot` returns to a (re)attaching frontend, and what the live
    // update stream mutates. The registered view already carries phase "thinking",
    // so an initial Phase emit is unnecessary — the snapshot covers a fresh attach.
    register_live_turn(
        &conversation_id,
        turn_token,
        TurnView {
            turn_index,
            question: question.clone(),
            phase: "thinking".to_string(),
            blocks: vec![],
            reasoning: None,
            tool_activities: vec![],
            live_activity: None,
            sources: serde_json::json!([]),
            error_message: None,
            provider: Some(engine_provider.clone()),
            model: Some(engine_model.clone()),
            context_tokens: None,
        },
    );
    // The frontend's own turn starts without the engine; tell it which engine
    // answers, so its label never falls back to the picker's current model.
    emit_live_update(
        &core,
        &conversation_id,
        turn_token,
        TurnUpdate::Engine {
            provider: engine_provider,
            model: engine_model,
        },
    );

    // 5. Search results are recorded by opaque id so a later `reference_captures`
    //    call can attach metadata and prove the model only references ids it
    //    actually received.
    let search_metadata: Arc<Mutex<HashMap<String, BrokerSearchResult>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // Shared persistence buffers the executor appends to (tool-activity entries +
    // nominated Answer Sources) and the on_event closure reads when persisting.
    let tool_activities: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
    let sources: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));

    // 6. Build the tool executor. Each non-reference data tool rides
    //    `execute_ask_ai_broker_request` (access readiness + All-Retained scope +
    //    redaction/audit); `reference_captures` is intercepted before the broker
    //    and emits the source cards. Results are returned to the model as a JSON
    //    STRING.
    let utc_offset_minutes = clock.utc_offset_minutes;
    let executor: ai_engine::ToolExecutor = build_tool_executor(
        core.clone(),
        conversation_id.clone(),
        turn_token,
        scope,
        utc_offset_minutes,
        Arc::clone(&search_metadata),
        Arc::clone(&tool_activities),
        Arc::clone(&sources),
    );

    // MCP connectors (Workstream C): await any in-flight warm-on-open discovery
    // (≤15s) and offer the ready servers' curated tools + preamble notes this
    // turn. Cheap Arc clone out of managed state so no State guard is held across
    // the await.
    let mcp_manager = (*core.state::<mcp::McpManager>()).clone();
    let (mcp_tools, mcp_notes) = mcp_manager.tools_for_turn(&core).await;

    let mut tools = build_ask_ai_tools(read_ask_ai_web_fetch_enabled(&core), mcp_tools);
    if scope.is_some_and(|scope| !scope.about_you) {
        tools.retain(|tool| tool.name != "recall_context");
    }
    let max_tool_calls = read_ask_ai_max_tool_calls(&core);
    let mut preamble = build_ask_ai_preamble(&mcp_notes);
    // Quick Recall only ever asks a thread's first turn; later turns come from
    // Chat (a follow-up still sends the default `quick_recall` origin), which
    // gets full-length answers.
    if origin == "quick_recall" && turn_index == 0 {
        preamble.push_str(QUICK_RECALL_GLANCE);
    }
    let prompt = build_ask_ai_prompt(&question, now_ms(), &clock, scope.as_ref());

    // 7. Run the agent loop, streaming deltas and persisting throttled partials.
    let answer: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    // Streaming answer parser (Slice 2) shared between the `on_event` Delta arm
    // (which pushes deltas → `AppendProse`/`OpenBlock` ops) and the post-loop
    // finalize (which flushes any unterminated fence to prose). The same parser
    // instance must back both so its committed/mode state is continuous.
    let answer_view: Arc<Mutex<AnswerView>> = Arc::new(Mutex::new(AnswerView::new()));
    // Reasoning accumulator mirrors `answer`: reasoning chunks stream live as
    // `ask_ai_reasoning` events and accumulate here so every persist (partial /
    // done / error) writes the snapshot through `save_turn`'s `reasoning` arg.
    let reasoning: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    // Spawned partial persists, awaited before a discard so none lands after
    // the delete and re-creates the row.
    let partial_persists: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>> =
        Arc::new(Mutex::new(Vec::new()));
    let deltas_since_persist = 0usize;
    let chars_since_persist = 0usize;
    // Captured for the `ToolCall` arm's tool-activity labelling: the recall-range
    // suffix renders its `from`/`to` window in the user's LOCAL time. `clock`'s
    // last use is `build_ask_ai_prompt(&clock)` above, before this closure, so we
    // copy out the offset (a `Copy` `Option<i32>`).
    let label_utc_offset_minutes = clock.utc_offset_minutes;
    // Connector display labels (server-id → label) so an MCP tool's activity line
    // can read "… from GitHub" instead of only the tool name. Moved into the
    // closure; unknown/non-MCP tools just don't get a suffix.
    let mcp_labels: std::collections::HashMap<String, String> = read_ai_runtime_settings(&core)
        .mcp_servers
        .into_iter()
        .map(|server| (server.id, server.label))
        .collect();
    // Turn timing + visible-tool-call count for the debug page's last-turn row.
    let turn_started_at_ms = now_ms();
    let turn_tool_calls = Arc::new(AtomicU64::new(0));
    let on_event = turn_event_handler(
        core.clone(),
        conversation_id.clone(),
        turn_index,
        turn_token,
        scope,
        Arc::clone(&infra),
        Arc::clone(&conversation_announced),
        Arc::clone(&answer),
        Arc::clone(&partial_persists),
        Arc::clone(&answer_view),
        Arc::clone(&reasoning),
        Arc::clone(&tool_activities),
        Arc::clone(&sources),
        title.clone(),
        origin.clone(),
        question.clone(),
        deltas_since_persist,
        chars_since_persist,
        label_utc_offset_minutes,
        mcp_labels,
        turn_started_at_ms,
        Arc::clone(&turn_tool_calls),
    );

    let run_result = ai_engine::run_agent_loop(
        &config,
        &preamble,
        &prompt,
        &history,
        tools,
        executor,
        max_tool_calls,
        cancel.clone(),
        on_event,
    )
    .await;

    // 8. Finalize. Snapshot the accumulated answer + reasoning + persistence
    //    buffers. Empty reasoning maps to NULL on persist (turns with no thinking).
    let final_answer = answer.lock().map(|g| g.clone()).unwrap_or_default();
    let final_reasoning = reasoning.lock().map(|g| g.clone()).unwrap_or_default();
    let final_reasoning = if final_reasoning.is_empty() {
        None
    } else {
        Some(final_reasoning)
    };
    let final_tool_activities = tool_activities
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    let final_sources = sources.lock().map(|g| g.clone()).unwrap_or_default();

    // Flush the streaming parser so any unterminated `mnema-*`
    // fence degrades to prose, completing the view's blocks. Done for BOTH arms so
    // the terminal view matches the persisted answer. Emitting each flush op bumps
    // the version, which `apply_live_update` returns; track the highest so the
    // displaced-terminal path can continue THIS turn's own version sequence
    // without a gap.
    let mut last_version: u64 = 0;
    {
        let finalize_ops = {
            let mut parser = answer_view.lock().unwrap_or_else(|e| e.into_inner());
            parser.finalize()
        };
        for op in finalize_ops {
            if let Some(version) = emit_live_update(&core, &conversation_id, turn_token, op) {
                last_version = version;
            }
        }
    }

    // The parser is now finalized (any unterminated fence flushed to prose), so
    // its blocks are the render-ready terminal set persisted on BOTH arms. A
    // non-empty `Some(..)` keeps the terminal row non-legacy (never re-parsed).
    let final_blocks = answer_view
        .lock()
        .map(|v| v.blocks().to_vec())
        .unwrap_or_default();

    // Emit the terminal view update on EVERY exit path. If the live apply succeeds
    // (this turn still owns the conversation), it emits through the normal path and
    // we advance `last_version`. If it returns `None` we were DISPLACED by a newer
    // turn; emit the terminal DIRECTLY anyway — continuing THIS turn's own version
    // sequence (`last_version + 1`) so the frontend view for this `turn_index`
    // settles ("Writing…" resolves) with no version gap.
    // Every terminal path has persisted its final row by the time it emits, so
    // clear our in-flight flag FIRST: once the frontend sees Done/Error it may
    // `delete_last_turn` (Regenerate/Retry), which refuses while a turn is in
    // flight. Only our own flag goes — a displacing newer turn holds a
    // different `Arc` and must survive our teardown.
    let emit_terminal = |update: TurnUpdate, last_version: &mut u64| {
        remove_inflight_if_owner(&conversation_id, &cancel);
        match emit_live_update(&core, &conversation_id, turn_token, update.clone()) {
            Some(version) => *last_version = version,
            None => {
                let version = *last_version + 1;
                core.host().emit(CoreEvent::AskAiUpdate(AskAiUpdateEvent {
                    conversation_id: conversation_id.to_string(),
                    version: version,
                    turn_index: turn_index,
                    update: update,
                }));
                *last_version = version;
            }
        }
    };

    // A cancelled Quick Recall question that never got an answer, or was edited
    // into a new one, is deleted rather than kept (see `abandoned`); a
    // displacing newer turn owns the rows.
    let superseded = abandoned::take_superseded(&conversation_id);
    let cancelled = run_result.is_ok()
        && cancel.load(Ordering::SeqCst)
        && !abandoned::displaced(&conversation_id, &cancel);
    if cancelled {
        let handles = partial_persists
            .lock()
            .map(|mut h| std::mem::take(&mut *h))
            .unwrap_or_default();
        for handle in handles {
            let _ = handle.await;
        }
    }
    let discarded = cancelled
        && abandoned::discard_if_abandoned(
            &infra,
            &conversation_id,
            &origin,
            existing.as_ref().map(|c| c.origin.as_str()),
            turn_index,
            superseded,
            &final_answer,
            &final_blocks,
        )
        .await;

    match run_result {
        Ok(()) if !cancel.load(Ordering::SeqCst) && final_answer.trim().is_empty() => {
            // Clean finish that produced no answer. A reasoning model can spend
            // its entire token budget on chain-of-thought and stop
            // (`finish_reason=length`) before emitting any answer — the turn
            // would otherwise persist as `done` with an empty answer and render
            // as only a "Thought process". rig's stream items don't surface
            // `finish_reason`, so we detect the equivalent signal: a clean,
            // non-cancelled finish with an empty answer. (A user cancel
            // legitimately leaves an empty answer and is excluded by the guard.)
            // Surface it as an error rather than a silent blank turn.
            let message = if final_reasoning.is_some() {
                "The model used its whole response budget on reasoning and never reached an answer. Ask again, or pick a different model in Settings."
            } else {
                "The model returned an empty response. Ask again, or pick a different model in Settings."
            }
            .to_string();
            log::warn!(
                "Ask AI turn for {conversation_id} finished cleanly with an empty answer (reasoning present: {})",
                final_reasoning.is_some()
            );
            persist_turn(
                &infra,
                &conversation_id,
                &title,
                &origin,
                turn_index,
                &question,
                &final_answer,
                final_reasoning.as_deref(),
                Some(&final_blocks),
                &final_tool_activities,
                &final_sources,
                "error",
                Some((&message, "retryable")),
            )
            .await;
            core.host().emit(CoreEvent::ConversationChanged);
            emit_terminal(
                TurnUpdate::Error {
                    message,
                    kind: Some("retryable".to_string()),
                },
                &mut last_version,
            );
        }
        Ok(()) if discarded => {
            // An abandoned Quick Recall question: the conversation is gone, so
            // the history list drops it; the view still settles.
            core.host().emit(CoreEvent::ConversationChanged);
            emit_terminal(TurnUpdate::Done, &mut last_version);
        }
        Ok(()) => {
            // A cooperative cancel keeps whatever was generated and emits no
            // error; a clean finish persists `done` and emits the terminal update.
            persist_turn(
                &infra,
                &conversation_id,
                &title,
                &origin,
                turn_index,
                &question,
                &final_answer,
                final_reasoning.as_deref(),
                Some(&final_blocks),
                &final_tool_activities,
                &final_sources,
                "done",
                None,
            )
            .await;
            // Terminal persist updated the row (answer/updated-at), so the
            // history list re-sorts/refreshes its entry.
            core.host().emit(CoreEvent::ConversationChanged);
            // ALWAYS emit the terminal `Done` view update, even on a user cancel,
            // so the view settles ("Writing…" resolves).
            emit_terminal(TurnUpdate::Done, &mut last_version);
            // Fire-and-forget generated thread title: only after the FIRST turn
            // completes and persists. Spawned detached so it can never delay or
            // fail the turn; every failure inside is swallowed (the fallback
            // first-question title stands).
            if title_generation_eligible(turn_index) {
                core.spawn(generate_conversation_title(
                    core.clone(),
                    Arc::clone(&infra),
                    conversation_id.clone(),
                    question.clone(),
                ));
            }
        }
        Err(error) => {
            // Display a plain-language sentence; log the failure kind plus
            // the error's head (status code, error class) — never the body,
            // which can echo prompt or answer fragments, and user logs leave
            // the machine.
            log::warn!(
                "Ask AI agent loop failed for {conversation_id} ({}): {}",
                error.failure_kind(),
                error.log_detail()
            );
            // Worded for the engine that failed: on the ChatGPT subscription
            // backend a rejected credential is a sign-in problem, and there is
            // no API key field to send the user to.
            let message = error.user_facing_message_for(config.cloud_provider());
            let kind = error.failure_kind();
            persist_turn(
                &infra,
                &conversation_id,
                &title,
                &origin,
                turn_index,
                &question,
                &final_answer,
                final_reasoning.as_deref(),
                Some(&final_blocks),
                &final_tool_activities,
                &final_sources,
                "error",
                Some((&message, kind)),
            )
            .await;
            core.host().emit(CoreEvent::ConversationChanged);
            // Terminal `Error` view update.
            emit_terminal(
                TurnUpdate::Error {
                    message: message.clone(),
                    kind: Some(kind.to_string()),
                },
                &mut last_version,
            );
        }
    }

    // Remove our LiveTurn ONLY if we still own it — a displacing newer turn
    // registered a different token and its entry must survive our teardown.
    // (The in-flight flag already went in `emit_terminal`.)
    remove_live_turn_if_owner(&conversation_id, turn_token);
}
