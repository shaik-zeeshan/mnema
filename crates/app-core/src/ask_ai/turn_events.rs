//! The turn's agent-loop event handler: streamed deltas, reasoning, tool calls
//! and usage become live view updates and throttled partial persists.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn turn_event_handler(
    core: Arc<AppCore>,
    conversation_id: String,
    turn_index: i64,
    turn_token: u64,
    scope: Option<AskAiScope>,
    infra: AppInfraState,
    conversation_announced: Arc<AtomicBool>,
    answer: Arc<Mutex<String>>,
    partial_persists: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    answer_view: Arc<Mutex<AnswerView>>,
    reasoning: Arc<Mutex<String>>,
    tool_activities: Arc<Mutex<Vec<serde_json::Value>>>,
    sources: Arc<Mutex<Vec<serde_json::Value>>>,
    title: String,
    origin: String,
    question: String,
    mut deltas_since_persist: usize,
    mut chars_since_persist: usize,
    label_utc_offset_minutes: Option<i32>,
    mcp_labels: HashMap<String, String>,
    turn_started_at_ms: i64,
    turn_tool_calls: Arc<AtomicU64>,
) -> impl FnMut(ai_engine::AgentLoopEvent) + Send {
    let core = core.clone();
    let conversation_id = conversation_id.clone();
    let turn_tool_calls = Arc::clone(&turn_tool_calls);
    let infra = Arc::clone(&infra);
    let conversation_announced = Arc::clone(&conversation_announced);
    let answer = Arc::clone(&answer);
    let partial_persists = Arc::clone(&partial_persists);
    let answer_view = Arc::clone(&answer_view);
    let reasoning = Arc::clone(&reasoning);
    let tool_activities = Arc::clone(&tool_activities);
    let sources = Arc::clone(&sources);
    let title = title.clone();
    let origin = origin.clone();
    let question = question.clone();
    move |event: ai_engine::AgentLoopEvent| match event {
        ai_engine::AgentLoopEvent::Delta(text) => {
            // On the FIRST visible delta advance the phase to "streaming" and
            // clear any live tool-activity line. Whitespace doesn't count: a
            // model often sends a bare newline before its tool calls, and
            // flipping then hid the step line behind an empty answer. Reading
            // the current live_activity first avoids a no-op version bump
            // when there is nothing to clear.
            if !text.trim().is_empty() {
                let (was_thinking, had_live) = ask_ai_live_turns()
                    .lock()
                    .ok()
                    .and_then(|map| {
                        map.get(&conversation_id).map(|live| {
                            (
                                live.view.phase == "thinking",
                                live.view.live_activity.is_some(),
                            )
                        })
                    })
                    .unwrap_or((false, false));
                if was_thinking {
                    emit_live_update(
                        &core,
                        &conversation_id,
                        turn_token,
                        TurnUpdate::Phase {
                            phase: "streaming".to_string(),
                        },
                    );
                    if had_live {
                        emit_live_update(
                            &core,
                            &conversation_id,
                            turn_token,
                            TurnUpdate::LiveActivity { entry: None },
                        );
                    }
                }
            }
            // Parse the delta into render-ready ops and emit each as a view
            // update (AppendProse / OpenBlock). The lock is released before the
            // emit loop so `emit_live_update`'s own map lock never nests under
            // the parser lock.
            let ops = {
                let mut parser = answer_view.lock().unwrap_or_else(|e| e.into_inner());
                parser.push_delta(&text)
            };
            for op in ops {
                emit_live_update(&core, &conversation_id, turn_token, op);
            }
            let answer_so_far = {
                let mut guard = answer.lock().unwrap_or_else(|e| e.into_inner());
                guard.push_str(&text);
                deltas_since_persist += 1;
                chars_since_persist += text.len();
                if deltas_since_persist >= ASK_AI_PARTIAL_PERSIST_DELTA_INTERVAL
                    || chars_since_persist >= ASK_AI_PARTIAL_PERSIST_CHAR_INTERVAL
                {
                    deltas_since_persist = 0;
                    chars_since_persist = 0;
                    Some(guard.clone())
                } else {
                    None
                }
            };
            // Throttled partial persist for reattach. Spawned so persistence
            // never blocks the synchronous stream callback.
            // Blank text is never persisted; non-blank partials are tracked
            // in `partial_persists` so a discard can wait them out.
            if let Some(answer_so_far) = answer_so_far.filter(|a| !a.trim().is_empty()) {
                let infra = Arc::clone(&infra);
                let core = core.clone();
                let conversation_announced = Arc::clone(&conversation_announced);
                let conversation_id = conversation_id.clone();
                let title = title.clone();
                let origin = origin.clone();
                let question = question.clone();
                let tool_activities_snapshot = tool_activities
                    .lock()
                    .map(|guard| guard.clone())
                    .unwrap_or_default();
                let sources_snapshot = sources
                    .lock()
                    .map(|guard| guard.clone())
                    .unwrap_or_default();
                // Snapshot the reasoning buffer too so the partial persist
                // carries any thinking captured so far (empty → NULL).
                let reasoning_snapshot = reasoning
                    .lock()
                    .map(|guard| guard.clone())
                    .unwrap_or_default();
                // Snapshot the parser's render-ready blocks so the persisted
                // row matches the live view; `Some(..)` keeps the turn
                // non-legacy even before any block has been committed.
                let blocks_snapshot = answer_view
                    .lock()
                    .map(|v| v.blocks().to_vec())
                    .unwrap_or_default();
                let handle = core.runtime_handle().spawn(async move {
                    let persisted = persist_turn(
                        &infra,
                        &conversation_id,
                        &title,
                        &origin,
                        turn_index,
                        &question,
                        &answer_so_far,
                        if reasoning_snapshot.is_empty() {
                            None
                        } else {
                            Some(reasoning_snapshot.as_str())
                        },
                        Some(&blocks_snapshot),
                        &tool_activities_snapshot,
                        &sources_snapshot,
                        "streaming",
                        None,
                    )
                    .await;
                    // Announce only the FIRST successful persist of the turn
                    // (the initial persist normally wins; this covers it
                    // having failed), never every partial.
                    if persisted && !conversation_announced.swap(true, Ordering::SeqCst) {
                        core.host().emit(CoreEvent::ConversationChanged);
                    }
                });
                if let Ok(mut handles) = partial_persists.lock() {
                    handles.retain(|h| !h.is_finished());
                    handles.push(handle);
                }
            }
        }
        ai_engine::AgentLoopEvent::Reasoning(text) => {
            if let Ok(mut guard) = reasoning.lock() {
                guard.push_str(&text);
            }
            // Append reasoning to the view model.
            emit_live_update(
                &core,
                &conversation_id,
                turn_token,
                TurnUpdate::Reasoning { text },
            );
        }
        ai_engine::AgentLoopEvent::ToolCall { name, params } => {
            // `reference_captures` is a presentation signal, not a data
            // activity, so it must not appear in the activity working-line.
            if name == "reference_captures" {
                return;
            }
            turn_tool_calls.fetch_add(1, Ordering::SeqCst);
            // Build the render-ready entry SYNCHRONOUSLY (this
            // callback is sync) without the icon, record it in the rail, and
            // set it as the live activity line. Icon resolution is async, so it
            // is spawned and re-emits an enriched live line if/when it resolves.
            let now = now_ms();
            let mut entry = scope::scoped_tool_activity(
                scope.as_ref(),
                &name,
                &params,
                now,
                label_utc_offset_minutes,
            );
            // Runtime enrich (settings-dependent, like the icon step below):
            // suffix an MCP tool's line with its connector's display label, e.g.
            // "Running pull request read from GitHub".
            if let Some(suffix) = mcp_connector_suffix(&name, &mcp_labels) {
                entry.label.push_str(&suffix);
            }
            emit_live_update(
                &core,
                &conversation_id,
                turn_token,
                TurnUpdate::ToolActivity {
                    entry: entry.clone(),
                },
            );
            emit_live_update(
                &core,
                &conversation_id,
                turn_token,
                TurnUpdate::LiveActivity {
                    entry: Some(entry.clone()),
                },
            );
            if let Some(app) = entry.app.clone() {
                let core = core.clone();
                let conversation_id = conversation_id.clone();
                core.runtime_handle().spawn(async move {
                    if let Some(icon_path) = tool_activity::resolve_app_icon_path(&core, &app).await
                    {
                        let mut enriched = entry;
                        enriched.app_icon_path = Some(icon_path);
                        // Goes through `apply_live_update` under the lock, so the
                        // version stays monotonic even though this is a later
                        // spawned task.
                        emit_live_update(
                            &core,
                            &conversation_id,
                            turn_token,
                            TurnUpdate::LiveActivity {
                                entry: Some(enriched),
                            },
                        );
                    }
                });
            }
        }
        ai_engine::AgentLoopEvent::Usage {
            input_tokens,
            output_tokens,
            total_tokens,
        } => {
            let context = context_tokens(input_tokens, output_tokens, total_tokens);
            record_last_turn_usage(AskAiTokenUsage {
                input_tokens,
                output_tokens,
                context_tokens: context,
                started_at_ms: turn_started_at_ms,
                tool_calls: turn_tool_calls.load(Ordering::SeqCst),
                duration_ms: (now_ms() - turn_started_at_ms).max(0),
            });
            emit_live_update(
                &core,
                &conversation_id,
                turn_token,
                TurnUpdate::ContextTokens { tokens: context },
            );
        }
        // `Done` is handled after the loop returns; the loop emits it last.
        ai_engine::AgentLoopEvent::Done => {}
    }
}
