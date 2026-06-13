//! A tool-agnostic streaming agent loop layered over `rig-core`.
//!
//! Where [`crate::extract_with_preamble`] runs one structured-extraction round
//! trip, [`run_agent_loop`] drives a multi-turn conversation: the model streams
//! text deltas, may issue tool calls that are executed by a caller-supplied
//! [`ToolExecutor`] callback, and the loop honours a per-question tool-call cap
//! and a cooperative cancellation flag.
//!
//! The crate keeps its dependency posture: tools are injected as opaque async
//! callbacks (the "Reasoning Engine" never imports the capture broker,
//! `capture-types`, or `app-infra`). Each [`AgentTool`] is wrapped in a
//! [`BrokeredTool`] that implements rig's [`ToolDyn`] and forwards `call` to the
//! executor, so rig's own multi-turn machinery resolves tool calls and feeds the
//! results back to the model.

use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures_util::StreamExt;
use rig_core::agent::{Agent, MultiTurnStreamItem};
use rig_core::client::CompletionClient;
use rig_core::completion::{CompletionModel, GetTokenUsage, PromptError, ToolDefinition};
use rig_core::message::Message;
use rig_core::providers::{anthropic, llamafile, ollama, openai};
use rig_core::streaming::{StreamedAssistantContent, StreamingPrompt};
use rig_core::tool::{ToolDyn, ToolError};
use rig_core::wasm_compat::{WasmBoxedFuture, WasmCompatSend};

use crate::{AiRuntimeError, CloudProvider, EngineConfig, LocalKind};

/// Default `max_tokens` requested from the strict-validation cloud providers
/// (Anthropic, OpenAI).
///
/// Anthropic streaming *requires* `max_tokens`. Both Anthropic and OpenAI reject
/// a `max_tokens` that exceeds the chosen model's maximum output (e.g. legacy
/// `gpt-4-turbo` caps at 4096), so this stays at a value every offered model
/// accepts rather than reaching for headroom we can't guarantee.
const DEFAULT_MAX_TOKENS: u64 = 4096;

/// `max_tokens` for the lenient endpoints that host reasoning models —
/// OpenAI-compatible gateways (Fireworks, etc.), Ollama, and Llamafile.
///
/// Reasoning models (Kimi "thinking"/"code", DeepSeek-R1, qwq, …) stream their
/// chain-of-thought as output tokens *before* the answer. Under the 4096 ceiling
/// a verbose model can spend the entire budget thinking, hit `finish_reason=length`
/// mid-thought, and emit no answer at all — the turn then renders as only a
/// "Thought process". These endpoints clamp an oversized `max_tokens` to the
/// context window instead of erroring, so we give reasoning room to land the
/// answer. `ask_ai` still guards the residual truncation case.
const REASONING_MAX_TOKENS: u64 = 32768;

/// Upper bound on the multi-turn depth handed to rig.
///
/// `max_tool_calls` is caller-controlled (and may legitimately be very large, or
/// `usize::MAX` for "no cap"), but rig increments an internal turn counter per
/// round, so an unbounded value risks overflow / pathological loops. Clamping to
/// this ceiling keeps the bound sane while still being far larger than any
/// realistic tool-call budget.
const MULTI_TURN_CEILING: usize = 64;

/// A tool the agent may call, described to the model.
///
/// Execution is delegated to the caller's [`ToolExecutor`], keyed by `name`; this
/// struct only carries what the model needs to *decide* to call the tool.
pub struct AgentTool {
    /// The tool name, used both in the provider definition and to dispatch the
    /// executor callback.
    pub name: String,
    /// Human-readable description sent to the model.
    pub description: String,
    /// JSON Schema object describing the tool's params (the `parameters` field of
    /// the provider tool definition).
    pub parameters_schema: serde_json::Value,
}

/// Caller-supplied async tool executor.
///
/// Given `(tool_name, params_json)` it returns the tool result serialized as a
/// JSON string fed back to the model, or an error string surfaced to the model
/// as the tool result. Boxed future so it is object-safe and `Send + 'static`.
pub type ToolExecutor = Arc<
    dyn Fn(
            String,
            serde_json::Value,
        ) -> Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>
        + Send
        + Sync,
>;

/// The role of a prior conversation turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRole {
    User,
    Assistant,
}

/// One prior turn of conversation history (role + text), oldest first.
pub struct AgentHistoryTurn {
    pub role: AgentRole,
    pub text: String,
}

/// Events observed while the loop runs, for streaming UI.
///
/// Tool *execution* happens via the [`ToolExecutor`]; [`AgentLoopEvent::ToolCall`]
/// is observational, emitted as the model issues a call so a UI can show
/// tool-activity.
pub enum AgentLoopEvent {
    /// A streamed text chunk from the assistant.
    Delta(String),
    /// A streamed reasoning/thinking chunk from the assistant (provider-native;
    /// only emitted when the model/provider yields reasoning). Interleaves before
    /// and between `Delta` text. The caller decides whether/how to surface it.
    Reasoning(String),
    /// The model issued a tool call (emitted as it is dispatched to the executor).
    ToolCall {
        name: String,
        params: serde_json::Value,
    },
    /// The loop finished (clean completion or cooperative cancellation).
    Done,
}

/// A [`ToolDyn`] wrapper that forwards execution to a caller-supplied
/// [`ToolExecutor`].
///
/// rig owns tool dispatch inside its multi-turn loop: when the model calls a
/// tool by name, rig looks it up in the agent's tool set and invokes
/// [`ToolDyn::call`]. This wrapper parses the JSON args, runs the executor, and
/// maps the `Result<String, String>` onto rig's `Result<String, ToolError>` so
/// the result (or error text) is streamed back to the model as a tool result.
struct BrokeredTool {
    name: String,
    description: String,
    parameters_schema: serde_json::Value,
    executor: ToolExecutor,
}

impl ToolDyn for BrokeredTool {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn definition<'a>(&'a self, _prompt: String) -> WasmBoxedFuture<'a, ToolDefinition> {
        let definition = ToolDefinition {
            name: self.name.clone(),
            description: self.description.clone(),
            parameters: self.parameters_schema.clone(),
        };
        Box::pin(async move { definition })
    }

    fn call<'a>(&'a self, args: String) -> WasmBoxedFuture<'a, Result<String, ToolError>> {
        let name = self.name.clone();
        let executor = self.executor.clone();
        Box::pin(async move {
            // The model usually sends a JSON object; tolerate an empty/`null`
            // payload by treating it as an empty object so all-optional tools
            // still dispatch.
            let params: serde_json::Value = if args.trim().is_empty() || args.trim() == "null" {
                serde_json::Value::Object(serde_json::Map::new())
            } else {
                serde_json::from_str(&args).map_err(ToolError::JsonError)?
            };

            executor(name, params).await.map_err(|message| {
                ToolError::ToolCallError(Box::<dyn std::error::Error + Send + Sync>::from(message))
            })
        })
    }
}

/// Build the boxed dynamic-tool set handed to the agent builder.
fn brokered_tools(tools: Vec<AgentTool>, executor: &ToolExecutor) -> Vec<Box<dyn ToolDyn>> {
    tools
        .into_iter()
        .map(|tool| {
            Box::new(BrokeredTool {
                name: tool.name,
                description: tool.description,
                parameters_schema: tool.parameters_schema,
                executor: executor.clone(),
            }) as Box<dyn ToolDyn>
        })
        .collect()
}

/// Map an [`AgentHistoryTurn`] onto a rig chat-history [`Message`].
fn history_messages(history: &[AgentHistoryTurn]) -> Vec<Message> {
    history
        .iter()
        .map(|turn| match turn.role {
            AgentRole::User => Message::user(turn.text.clone()),
            AgentRole::Assistant => Message::assistant(turn.text.clone()),
        })
        .collect()
}

/// Run a tool-agnostic streaming agent loop against the selected engine.
///
/// Builds the appropriate provider client+agent for `config` (mirroring the
/// per-provider arms of [`crate::extract_with_preamble`]) with `preamble` as the
/// system instruction and `tools` attached as dynamic [`ToolDyn`] wrappers whose
/// `call` invokes `executor(name, params)`. `history` is fed as the agent's chat
/// history (oldest first). The model streams text deltas (surfaced as
/// [`AgentLoopEvent::Delta`]); when it issues a tool call, rig executes it via the
/// matching [`ToolExecutor`] and an observational [`AgentLoopEvent::ToolCall`] is
/// emitted. `max_tool_calls` bounds the multi-turn depth (clamped to a sane
/// ceiling so `usize::MAX` cannot overflow rig's turn counter), and `cancel` is
/// checked between streamed items so a set flag stops consumption and returns
/// `Ok(())`. [`AgentLoopEvent::Done`] is emitted once the loop ends.
#[allow(clippy::too_many_arguments)]
pub async fn run_agent_loop(
    config: &EngineConfig,
    preamble: &str,
    prompt: &str,
    history: &[AgentHistoryTurn],
    tools: Vec<AgentTool>,
    executor: ToolExecutor,
    max_tool_calls: usize,
    cancel: Arc<AtomicBool>,
    on_event: impl FnMut(AgentLoopEvent) + Send,
) -> Result<(), AiRuntimeError> {
    let tool_set = brokered_tools(tools, &executor);
    let history = history_messages(history);

    match config {
        EngineConfig::Cloud {
            provider,
            model,
            api_key,
            base_url,
        } => {
            if model.trim().is_empty() {
                return Err(AiRuntimeError::MissingModel);
            }
            if api_key.trim().is_empty() {
                return Err(AiRuntimeError::MissingKey);
            }

            match provider {
                CloudProvider::Anthropic => {
                    let client = anthropic::Client::builder().api_key(api_key).build()?;
                    let agent = client
                        .agent(model.as_str())
                        .preamble(preamble)
                        .max_tokens(DEFAULT_MAX_TOKENS)
                        .tools(tool_set)
                        .build();
                    drive_agent_stream(agent, prompt, history, max_tool_calls, cancel, on_event)
                        .await
                }
                CloudProvider::Openai => {
                    let client = openai::Client::builder().api_key(api_key).build()?;
                    let agent = client
                        .agent(model.as_str())
                        .preamble(preamble)
                        .max_tokens(DEFAULT_MAX_TOKENS)
                        .tools(tool_set)
                        .build();
                    drive_agent_stream(agent, prompt, history, max_tool_calls, cancel, on_event)
                        .await
                }
                CloudProvider::OpenAiCompatible => {
                    // OpenAI-compatible providers implement the Chat Completions
                    // API, so build a `CompletionsClient` pointed at the
                    // user-supplied base URL (mirrors `extract_with_preamble`).
                    let base_url = base_url
                        .as_deref()
                        .map(str::trim)
                        .filter(|url| !url.is_empty())
                        .ok_or(AiRuntimeError::MissingBaseUrl)?;
                    let client = openai::CompletionsClient::builder()
                        .api_key(api_key)
                        .base_url(base_url)
                        .build()?;
                    let agent = client
                        .agent(model.as_str())
                        .preamble(preamble)
                        .max_tokens(REASONING_MAX_TOKENS)
                        .tools(tool_set)
                        .build();
                    drive_agent_stream(agent, prompt, history, max_tool_calls, cancel, on_event)
                        .await
                }
            }
        }
        EngineConfig::Local {
            kind,
            endpoint,
            model,
        } => {
            if model.trim().is_empty() {
                return Err(AiRuntimeError::MissingModel);
            }

            match kind {
                LocalKind::Ollama => {
                    let client = ollama::Client::builder()
                        .api_key(rig_core::client::Nothing)
                        .base_url(endpoint)
                        .build()?;
                    let agent = client
                        .agent(model.as_str())
                        .preamble(preamble)
                        .max_tokens(REASONING_MAX_TOKENS)
                        .tools(tool_set)
                        .build();
                    drive_agent_stream(agent, prompt, history, max_tool_calls, cancel, on_event)
                        .await
                }
                LocalKind::Llamafile => {
                    let client = llamafile::Client::from_url(endpoint)?;
                    let agent = client
                        .agent(model.as_str())
                        .preamble(preamble)
                        .max_tokens(REASONING_MAX_TOKENS)
                        .tools(tool_set)
                        .build();
                    drive_agent_stream(agent, prompt, history, max_tool_calls, cancel, on_event)
                        .await
                }
            }
        }
    }
}

/// Drive a built agent's streaming multi-turn loop.
///
/// Split out from [`run_agent_loop`] so it is generic over the concrete
/// [`CompletionModel`] and can be unit-tested against a scripted mock model. The
/// public function builds the provider agent and delegates here.
///
/// Consumes the [`MultiTurnStreamItem`] stream: text deltas become
/// [`AgentLoopEvent::Delta`], assistant tool calls become observational
/// [`AgentLoopEvent::ToolCall`] (rig executes the call itself via the agent's
/// tool set), and [`AgentLoopEvent::Done`] is emitted when the stream ends.
/// `cancel` is polled between items; a set flag drops the stream and returns
/// `Ok(())`. A `MaxTurnsError` from rig (the tool-call cap being hit) is *not* an
/// error — it is the expected bound, so the loop ends cleanly.
async fn drive_agent_stream<M>(
    agent: Agent<M>,
    prompt: &str,
    history: Vec<Message>,
    max_tool_calls: usize,
    cancel: Arc<AtomicBool>,
    mut on_event: impl FnMut(AgentLoopEvent) + Send,
) -> Result<(), AiRuntimeError>
where
    M: CompletionModel + 'static,
    <M as CompletionModel>::StreamingResponse: WasmCompatSend + GetTokenUsage,
{
    // Clamp the caller's cap onto rig's multi-turn depth: `0`/`usize::MAX` (the
    // "no cap" sentinels) collapse to the ceiling rather than overflowing.
    let max_turns = max_tool_calls.clamp(1, MULTI_TURN_CEILING);

    // Cancelled before we even started: emit Done and return.
    if cancel.load(Ordering::SeqCst) {
        on_event(AgentLoopEvent::Done);
        return Ok(());
    }

    let mut stream = agent
        .stream_prompt(prompt.to_string())
        .with_history(history)
        .multi_turn(max_turns)
        .await;

    while let Some(item) = stream.next().await {
        if cancel.load(Ordering::SeqCst) {
            // Drop the stream by breaking; cooperative cancellation is a clean
            // termination, not an error.
            break;
        }

        match item {
            Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(text))) => {
                on_event(AgentLoopEvent::Delta(text.text));
            }
            Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::ToolCall {
                tool_call,
                ..
            })) => {
                on_event(AgentLoopEvent::ToolCall {
                    name: tool_call.function.name,
                    params: tool_call.function.arguments,
                });
            }
            Ok(MultiTurnStreamItem::StreamAssistantItem(
                StreamedAssistantContent::ReasoningDelta { reasoning, .. },
            )) => {
                on_event(AgentLoopEvent::Reasoning(reasoning));
            }
            Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Reasoning(
                reasoning,
            ))) => {
                on_event(AgentLoopEvent::Reasoning(reasoning.display_text()));
            }
            // Remaining bookkeeping items (CompletionCall, FinalResponse,
            // ToolCallDelta, Final, user/tool-result items) are not surfaced to
            // the caller.
            Ok(_) => {}
            Err(err) => {
                // Hitting the multi-turn bound is the expected effect of the
                // tool-call cap, not a failure.
                if matches!(err, rig_core::agent::StreamingError::Prompt(ref prompt_err)
                    if matches!(**prompt_err, PromptError::MaxTurnsError { .. }))
                {
                    break;
                }
                return Err(AiRuntimeError::AgentLoop(err.to_string()));
            }
        }
    }

    on_event(AgentLoopEvent::Done);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig_core::agent::AgentBuilder;
    use rig_core::test_utils::{MockCompletionModel, MockStreamEvent};
    use std::sync::Mutex;

    /// Build an agent around a scripted mock model with the given brokered tools.
    fn mock_agent(model: MockCompletionModel, tools: Vec<AgentTool>, executor: &ToolExecutor) -> Agent<MockCompletionModel> {
        AgentBuilder::new(model)
            .preamble("test preamble")
            .tools(brokered_tools(tools, executor))
            .build()
    }

    /// Shared log of every `(tool_name, params)` the recording executor saw.
    type RecordedCalls = Arc<Mutex<Vec<(String, serde_json::Value)>>>;

    /// An executor that records every (name, params) call and returns a fixed
    /// JSON result.
    fn recording_executor() -> (ToolExecutor, RecordedCalls) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let calls_for_closure = calls.clone();
        let executor: ToolExecutor = Arc::new(move |name, params| {
            let calls = calls_for_closure.clone();
            Box::pin(async move {
                calls.lock().unwrap().push((name, params));
                Ok("{\"ok\":true}".to_string())
            })
        });
        (executor, calls)
    }

    fn search_tool() -> AgentTool {
        AgentTool {
            name: "search".to_string(),
            description: "search the index".to_string(),
            parameters_schema: serde_json::json!({
                "type": "object",
                "properties": { "query": { "type": "string" } }
            }),
        }
    }

    #[tokio::test]
    async fn tool_call_dispatches_executor_and_completes() {
        let (executor, calls) = recording_executor();
        // Turn 1: model issues a tool call. Turn 2: model answers with text.
        let model = MockCompletionModel::from_stream_turns([
            vec![
                MockStreamEvent::tool_call(
                    "call_1",
                    "search",
                    serde_json::json!({ "query": "hello" }),
                ),
                MockStreamEvent::final_response_with_default_usage(),
            ],
            vec![
                MockStreamEvent::text("done"),
                MockStreamEvent::final_response_with_default_usage(),
            ],
        ]);
        let agent = mock_agent(model, vec![search_tool()], &executor);

        let events = Arc::new(Mutex::new(Vec::new()));
        let events_sink = events.clone();
        let cancel = Arc::new(AtomicBool::new(false));

        drive_agent_stream(
            agent,
            "find hello",
            vec![],
            8,
            cancel,
            move |event| events_sink.lock().unwrap().push(event),
        )
        .await
        .expect("loop should complete");

        let recorded = calls.lock().unwrap();
        assert_eq!(recorded.len(), 1, "executor should be invoked once");
        assert_eq!(recorded[0].0, "search");
        assert_eq!(recorded[0].1, serde_json::json!({ "query": "hello" }));

        let events = events.lock().unwrap();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentLoopEvent::ToolCall { name, .. } if name == "search")),
            "a ToolCall event should be observed"
        );
        assert!(
            matches!(events.last(), Some(AgentLoopEvent::Done)),
            "loop should end with Done"
        );
    }

    #[tokio::test]
    async fn streamed_deltas_arrive_in_order() {
        let (executor, _calls) = recording_executor();
        let model = MockCompletionModel::from_stream_turns([vec![
            MockStreamEvent::text("Hello, "),
            MockStreamEvent::text("world"),
            MockStreamEvent::text("!"),
            MockStreamEvent::final_response_with_default_usage(),
        ]]);
        let agent = mock_agent(model, vec![], &executor);

        let deltas = Arc::new(Mutex::new(Vec::new()));
        let deltas_sink = deltas.clone();
        let cancel = Arc::new(AtomicBool::new(false));

        drive_agent_stream(
            agent,
            "greet",
            vec![],
            4,
            cancel,
            move |event| {
                if let AgentLoopEvent::Delta(text) = event {
                    deltas_sink.lock().unwrap().push(text);
                }
            },
        )
        .await
        .expect("loop should complete");

        let deltas = deltas.lock().unwrap();
        assert_eq!(
            deltas.join(""),
            "Hello, world!",
            "deltas should arrive in streamed order"
        );
    }

    /// Run a model that issues a tool call every turn and never answers, scripted
    /// with `scripted_turns` tool turns, under the given `cap`. Returns how many
    /// times the executor was invoked. Because the model never produces a final
    /// text answer, the only thing that ends the loop is the multi-turn cap, so
    /// the returned count is a direct function of `cap`.
    async fn tool_calls_under_cap(cap: usize, scripted_turns: usize) -> usize {
        let (executor, calls) = recording_executor();
        let turns: Vec<Vec<MockStreamEvent>> = (0..scripted_turns)
            .map(|_| {
                vec![
                    MockStreamEvent::tool_call(
                        "call",
                        "search",
                        serde_json::json!({ "query": "x" }),
                    ),
                    MockStreamEvent::final_response_with_default_usage(),
                ]
            })
            .collect();
        let model = MockCompletionModel::from_stream_turns(turns);
        let agent = mock_agent(model, vec![search_tool()], &executor);

        let cancel = Arc::new(AtomicBool::new(false));
        drive_agent_stream(agent, "loop", vec![], cap, cancel, |_| {})
            .await
            .expect("hitting the cap is a clean stop, not an error");

        let count = calls.lock().unwrap().len();
        count // executor invocations == tool calls the model issued
    }

    #[tokio::test]
    async fn tool_call_cap_bounds_tool_calls() {
        // Script far more tool turns than either cap, so the cap — not turn
        // exhaustion — is what ends the loop.
        let small = tool_calls_under_cap(1, 8).await;
        let larger = tool_calls_under_cap(4, 8).await;

        // Hitting the cap is a clean stop (no error) and bounds the tool calls
        // below the scripted turn count.
        assert!(small >= 1, "at least one tool call should run, got {small}");
        assert!(
            small < 8 && larger < 8,
            "tool calls should be bounded by the cap below the 8 scripted turns: small={small}, larger={larger}"
        );
        // The bound scales with the cap: a larger cap permits strictly more tool
        // rounds, proving the cap (not the script length) is the limiter.
        assert!(
            larger > small,
            "a larger cap should allow more tool calls: small={small}, larger={larger}"
        );
    }

    #[tokio::test]
    async fn reasoning_chunks_are_surfaced() {
        let (executor, _calls) = recording_executor();
        // Script a reasoning delta and a complete reasoning block interleaved with
        // text deltas. Real providers emit EITHER deltas OR a full block for the
        // same content; scripting both here just exercises both new match arms.
        let model = MockCompletionModel::from_stream_turns([vec![
            MockStreamEvent::reasoning_delta(Some("rs_1"), "let me think"),
            MockStreamEvent::text("partial "),
            MockStreamEvent::reasoning("done thinking"),
            MockStreamEvent::text("answer"),
            MockStreamEvent::final_response_with_default_usage(),
        ]]);
        let agent = mock_agent(model, vec![], &executor);

        let reasoning = Arc::new(Mutex::new(Vec::new()));
        let reasoning_sink = reasoning.clone();
        let cancel = Arc::new(AtomicBool::new(false));

        drive_agent_stream(
            agent,
            "think",
            vec![],
            4,
            cancel,
            move |event| {
                if let AgentLoopEvent::Reasoning(text) = event {
                    reasoning_sink.lock().unwrap().push(text);
                }
            },
        )
        .await
        .expect("loop should complete");

        let reasoning = reasoning.lock().unwrap();
        // Both the streamed delta and the complete block's display text surface as
        // `Reasoning` events, in streamed order.
        assert_eq!(
            *reasoning,
            vec!["let me think".to_string(), "done thinking".to_string()],
            "reasoning delta and full block should both be surfaced as Reasoning events"
        );
    }

    #[tokio::test]
    async fn cancellation_mid_stream_stops_and_returns_ok() {
        let (executor, _calls) = recording_executor();
        let model = MockCompletionModel::from_stream_turns([vec![
            MockStreamEvent::text("one"),
            MockStreamEvent::text("two"),
            MockStreamEvent::text("three"),
            MockStreamEvent::final_response_with_default_usage(),
        ]]);
        let agent = mock_agent(model, vec![], &executor);

        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_for_event = cancel.clone();
        let deltas = Arc::new(Mutex::new(Vec::new()));
        let deltas_sink = deltas.clone();

        drive_agent_stream(
            agent,
            "count",
            vec![],
            4,
            cancel,
            move |event| {
                if let AgentLoopEvent::Delta(text) = event {
                    deltas_sink.lock().unwrap().push(text);
                    // Cancel after observing the first delta.
                    cancel_for_event.store(true, Ordering::SeqCst);
                }
            },
        )
        .await
        .expect("cancellation should return Ok");

        let deltas = deltas.lock().unwrap();
        assert!(
            deltas.len() < 3,
            "cancellation should stop emitting before all deltas, got {:?}",
            *deltas
        );
        assert_eq!(deltas.first().map(String::as_str), Some("one"));
    }
}
