//! Spawn and stream the PI Ask AI Node shim.
//!
//! Mnema drives the user's installed PI runtime through an in-process SDK shim
//! (`resources/pi-ask-ai-shim.mjs`) rather than `pi --mode rpc`, so PI's builtin
//! bash/file tools are never exposed (ADR 0024). This module owns spawning that
//! shim via `node`, writing the seeded prompt to its stdin, and translating its
//! newline-delimited JSON stdout into [`AskAiStreamEvent`]s.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

/// One selectable Ask AI model reported by the shim's list mode.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AskAiModel {
    /// Stable `provider:id` value persisted in settings and reselected later.
    pub value: String,
    pub provider: String,
    pub id: String,
    pub name: String,
}

/// Boxed async result of one brokered tool invocation.
pub type AskAiToolFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<serde_json::Value, String>> + Send>>;

/// Invokes one brokered tool call (tool name + camelCase params object) and
/// resolves to the broker response value, or an error message string.
pub type AskAiToolInvoker =
    Box<dyn FnMut(String, serde_json::Value) -> AskAiToolFuture + Send>;

/// A single streamed event parsed from one line of shim stdout.
#[derive(Debug, Clone, PartialEq)]
pub enum AskAiStreamEvent {
    /// `{"type":"ready"}` — session created (optional; may be absent).
    Ready,
    /// `{"type":"delta","text":"..."}` — append text chunk to the streamed answer.
    Delta(String),
    /// A brokered tool call was received and is about to run.
    ToolCall {
        id: String,
        tool: String,
        params: serde_json::Value,
    },
    /// A brokered tool call finished (ok=false means it errored or was capped).
    ToolResult { id: String, tool: String, ok: bool },
    /// `{"type":"done"}` — answer complete.
    Done,
    /// `{"type":"error","message":"..."}` — failure.
    Error(String),
}

/// A parsed `tool_call` line from shim stdout.
#[derive(Debug, Clone, PartialEq)]
struct ToolCall {
    id: String,
    tool: String,
    params: serde_json::Value,
}

/// Parse a single line of shim stdout into a [`ToolCall`], if it is one.
///
/// Returns `None` for blank lines, malformed JSON, non-`tool_call` types, or
/// `tool_call` lines missing the required `id`/`tool` string fields. A missing
/// `params` defaults to an empty JSON object.
fn parse_tool_call_line(line: &str) -> Option<ToolCall> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let value: serde_json::Value = serde_json::from_str(trimmed).ok()?;
    if value.get("type").and_then(|t| t.as_str()) != Some("tool_call") {
        return None;
    }

    let id = value.get("id")?.as_str()?.to_string();
    let tool = value.get("tool")?.as_str()?.to_string();
    let params = value
        .get("params")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));

    Some(ToolCall { id, tool, params })
}

/// Whether a tool call spends one unit of the per-question data tool-call
/// budget. `reference_captures` is a presentation signal (it nominates Answer
/// Sources for the UI and returns no capture data), so it is exempt from the
/// cap and is never blocked by it; the three brokered data tools all count.
fn tool_counts_against_cap(tool: &str) -> bool {
    tool != "reference_captures"
}

/// Build the `tool_result` stdin line (without trailing newline) for a finished
/// tool call. `Ok` emits `ok:true` with the `result` value; `Err` emits
/// `ok:false` with the `error` message.
fn tool_result_line(id: &str, result: &Result<serde_json::Value, String>) -> String {
    let value = match result {
        Ok(value) => serde_json::json!({
            "type": "tool_result",
            "id": id,
            "ok": true,
            "result": value,
        }),
        Err(message) => serde_json::json!({
            "type": "tool_result",
            "id": id,
            "ok": false,
            "error": message,
        }),
    };
    value.to_string()
}

/// Parse a single line of shim stdout into an [`AskAiStreamEvent`].
///
/// Returns `None` for blank lines, malformed JSON, or unknown/unsupported
/// `type` values so the reader loop can skip them gracefully.
pub fn parse_shim_line(line: &str) -> Option<AskAiStreamEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let value: serde_json::Value = serde_json::from_str(trimmed).ok()?;
    let event_type = value.get("type")?.as_str()?;

    match event_type {
        "ready" => Some(AskAiStreamEvent::Ready),
        "delta" => {
            let text = value.get("text").and_then(|t| t.as_str()).unwrap_or("");
            Some(AskAiStreamEvent::Delta(text.to_string()))
        }
        "done" => Some(AskAiStreamEvent::Done),
        "error" => {
            let message = value
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Ask AI shim reported an error")
                .to_string();
            Some(AskAiStreamEvent::Error(message))
        }
        _ => None,
    }
}

/// A cheap cancellation handle shared with a running session.
///
/// Cancellation is cooperative: the reader loop checks this flag between lines
/// and kills the child when it is set. Callers may also be killed promptly via
/// the same flag because the loop polls it while reading.
#[derive(Debug, Clone, Default)]
pub struct AskAiCancel {
    flag: Arc<AtomicBool>,
}

impl AskAiCancel {
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Request cancellation of the associated session.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

/// Spawn `node <shim_path>`, stream its events, and drive `on_event` per event.
///
/// Writes one JSON line `{"prompt":"..."}\n` to the child's stdin and then keeps
/// stdin open for the whole session so brokered `tool_result` lines can be
/// written back. Reads newline-delimited JSON from stdout: `tool_call` lines are
/// executed via `tool_invoker` (capped at `max_tool_calls`) and answered with a
/// `tool_result` line, while everything else is parsed via [`parse_shim_line`].
/// stderr is drained into a buffer used for diagnostics: a non-zero exit without
/// a terminal `done`/`error` event surfaces the stderr tail as the error
/// message. `cancel` kills the child when triggered.
#[allow(clippy::too_many_arguments)]
pub async fn run_pi_ask_ai_session<F>(
    node_path: &Path,
    shim_path: &Path,
    pi_executable: Option<&str>,
    model: Option<&str>,
    prompt: &str,
    max_tool_calls: usize,
    mut on_event: F,
    mut tool_invoker: AskAiToolInvoker,
    cancel: AskAiCancel,
) -> Result<(), String>
where
    F: FnMut(AskAiStreamEvent),
{
    let mut command = Command::new(node_path);
    command.arg(shim_path);
    command.stdin(std::process::Stdio::piped());
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());

    if let Some(pi_executable) = pi_executable {
        command.env("MNEMA_PI_EXECUTABLE", pi_executable);
    }
    // The selected Quick Recall model ("provider:id"); the shim falls back to the
    // PI default when this is absent or does not resolve.
    if let Some(model) = model.map(str::trim).filter(|m| !m.is_empty()) {
        command.env("MNEMA_PI_ASK_AI_MODEL", model);
    }
    // Pass through PI_CODING_AGENT_DIR only when the parent env already set it.
    if let Some(agent_dir) = std::env::var_os("PI_CODING_AGENT_DIR") {
        command.env("PI_CODING_AGENT_DIR", agent_dir);
    }

    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to spawn Ask AI shim via node: {error}"))?;

    // Write the prompt as one JSON line, then keep stdin open for the whole
    // session so brokered `tool_result` lines can be written back.
    let mut child_stdin = child.stdin.take();
    if let Some(stdin) = child_stdin.as_mut() {
        let line = serde_json::json!({ "prompt": prompt }).to_string();
        stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|error| format!("failed to write Ask AI prompt to shim: {error}"))?;
        stdin
            .write_all(b"\n")
            .await
            .map_err(|error| format!("failed to write Ask AI prompt to shim: {error}"))?;
        stdin
            .flush()
            .await
            .map_err(|error| format!("failed to flush Ask AI prompt to shim: {error}"))?;
    }

    // Drain stderr into a shared buffer on a background task.
    let stderr_buffer = Arc::new(tokio::sync::Mutex::new(String::new()));
    let stderr_task = child.stderr.take().map(|stderr| {
        let buffer = Arc::clone(&stderr_buffer);
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            let mut chunk = String::new();
            // Read to end best-effort; ignore errors.
            let _ = reader.read_to_string(&mut chunk).await;
            let mut guard = buffer.lock().await;
            guard.push_str(&chunk);
        })
    });

    let mut saw_terminal = false;
    let mut tool_call_count: usize = 0;

    if let Some(stdout) = child.stdout.take() {
        let mut lines = BufReader::new(stdout).lines();
        'read: loop {
            if cancel.is_cancelled() {
                let _ = child.start_kill();
                break;
            }

            let next = lines.next_line().await;
            match next {
                Ok(Some(line)) => {
                    // A tool_call is handled in-loop (run + answer) rather than
                    // through the normal `parse_shim_line` event path.
                    if let Some(call) = parse_tool_call_line(&line) {
                        let ToolCall { id, tool, params } = call;
                        // Presentation tools (e.g. `reference_captures`) never
                        // count against or are blocked by the data tool-call cap.
                        let counts = tool_counts_against_cap(&tool);
                        if counts {
                            tool_call_count += 1;
                        }
                        on_event(AskAiStreamEvent::ToolCall {
                            id: id.clone(),
                            tool: tool.clone(),
                            params: params.clone(),
                        });

                        let result: Result<serde_json::Value, String> =
                            if counts && tool_call_count > max_tool_calls {
                                Err(format!(
                                    "Ask AI tool-call limit reached ({max_tool_calls}). Answer using the information already gathered."
                                ))
                            } else {
                                tool_invoker(tool.clone(), params).await
                            };
                        let ok = result.is_ok();

                        // Answer the child. If stdin is gone or the write fails,
                        // treat it like EOF (unless we are cancelling).
                        let mut wrote = false;
                        if let Some(stdin) = child_stdin.as_mut() {
                            let mut payload = tool_result_line(&id, &result);
                            payload.push('\n');
                            if stdin.write_all(payload.as_bytes()).await.is_ok()
                                && stdin.flush().await.is_ok()
                            {
                                wrote = true;
                            }
                        }

                        on_event(AskAiStreamEvent::ToolResult { id, tool, ok });

                        if !wrote {
                            if cancel.is_cancelled() {
                                let _ = child.start_kill();
                            }
                            break 'read;
                        }
                        continue;
                    }

                    if let Some(event) = parse_shim_line(&line) {
                        let terminal =
                            matches!(event, AskAiStreamEvent::Done | AskAiStreamEvent::Error(_));
                        on_event(event);
                        if terminal {
                            saw_terminal = true;
                            break;
                        }
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    return Err(format!("failed to read Ask AI shim output: {error}"));
                }
            }
        }
    }

    // Done reading; close stdin so the child can exit cleanly.
    drop(child_stdin.take());

    if cancel.is_cancelled() {
        let _ = child.start_kill();
        let _ = child.wait().await;
        if let Some(task) = stderr_task {
            let _ = task.await;
        }
        return Ok(());
    }

    let status = child
        .wait()
        .await
        .map_err(|error| format!("failed to await Ask AI shim: {error}"))?;

    if let Some(task) = stderr_task {
        let _ = task.await;
    }

    if saw_terminal {
        return Ok(());
    }

    // No terminal event was seen. A clean exit with no output is itself an
    // error; a non-zero exit surfaces the stderr tail.
    let stderr_tail = {
        let guard = stderr_buffer.lock().await;
        stderr_tail(&guard)
    };

    if status.success() {
        let message = if stderr_tail.is_empty() {
            "Ask AI shim exited without producing an answer".to_string()
        } else {
            format!("Ask AI shim exited without producing an answer: {stderr_tail}")
        };
        return Err(message);
    }

    let message = if stderr_tail.is_empty() {
        format!("Ask AI shim failed (exit {})", describe_exit(&status))
    } else {
        format!(
            "Ask AI shim failed (exit {}): {stderr_tail}",
            describe_exit(&status)
        )
    };
    Err(message)
}

/// Spawn `node <shim_path>` in list mode and return the selectable Ask AI models.
///
/// Sets `MNEMA_PI_LIST_MODELS=1` so the shim builds the PI model registry, emits
/// one `{"type":"models","models":[...]}` line, and exits. Returns the parsed
/// list, or an error message (with the stderr tail) when the shim fails or emits
/// no models line.
pub async fn list_pi_models(
    node_path: &Path,
    shim_path: &Path,
    pi_executable: Option<&str>,
) -> Result<Vec<AskAiModel>, String> {
    let mut command = Command::new(node_path);
    command.arg(shim_path);
    command.stdin(std::process::Stdio::null());
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    command.env("MNEMA_PI_LIST_MODELS", "1");
    if let Some(pi_executable) = pi_executable {
        command.env("MNEMA_PI_EXECUTABLE", pi_executable);
    }
    if let Some(agent_dir) = std::env::var_os("PI_CODING_AGENT_DIR") {
        command.env("PI_CODING_AGENT_DIR", agent_dir);
    }

    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to spawn Ask AI shim via node: {error}"))?;

    // Drain stderr for diagnostics.
    let stderr_buffer = Arc::new(tokio::sync::Mutex::new(String::new()));
    let stderr_task = child.stderr.take().map(|stderr| {
        let buffer = Arc::clone(&stderr_buffer);
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            let mut chunk = String::new();
            let _ = reader.read_to_string(&mut chunk).await;
            buffer.lock().await.push_str(&chunk);
        })
    });

    let mut models: Option<Vec<AskAiModel>> = None;
    if let Some(stdout) = child.stdout.take() {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(parsed) = parse_models_line(&line) {
                models = Some(parsed);
                break;
            }
        }
    }

    let status = child
        .wait()
        .await
        .map_err(|error| format!("failed to await Ask AI shim: {error}"))?;
    if let Some(task) = stderr_task {
        let _ = task.await;
    }

    if let Some(models) = models {
        return Ok(models);
    }

    let stderr_tail = {
        let guard = stderr_buffer.lock().await;
        stderr_tail(&guard)
    };
    let detail = if stderr_tail.is_empty() {
        String::new()
    } else {
        format!(": {stderr_tail}")
    };
    Err(format!(
        "Ask AI shim did not report any models (exit {}){detail}",
        describe_exit(&status)
    ))
}

/// Parse a `{"type":"models","models":[...]}` line into the model list. Returns
/// `None` for blank lines, malformed JSON, or non-`models` types.
fn parse_models_line(line: &str) -> Option<Vec<AskAiModel>> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(trimmed).ok()?;
    if value.get("type").and_then(|t| t.as_str()) != Some("models") {
        return None;
    }
    serde_json::from_value(value.get("models")?.clone()).ok()
}

fn describe_exit(status: &std::process::ExitStatus) -> String {
    match status.code() {
        Some(code) => code.to_string(),
        None => "signal".to_string(),
    }
}

/// Keep only the last few lines of captured stderr for a concise message.
fn stderr_tail(buffer: &str) -> String {
    let trimmed = buffer.trim_end();
    if trimmed.is_empty() {
        return String::new();
    }
    let tail: Vec<&str> = trimmed.lines().rev().take(5).collect();
    tail.into_iter().rev().collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_delta_line() {
        let event = parse_shim_line(r#"{"type":"delta","text":"hello"}"#);
        assert_eq!(event, Some(AskAiStreamEvent::Delta("hello".to_string())));
    }

    #[test]
    fn parse_delta_line_with_empty_text() {
        let event = parse_shim_line(r#"{"type":"delta","text":""}"#);
        assert_eq!(event, Some(AskAiStreamEvent::Delta(String::new())));
    }

    #[test]
    fn parse_delta_line_missing_text_defaults_empty() {
        let event = parse_shim_line(r#"{"type":"delta"}"#);
        assert_eq!(event, Some(AskAiStreamEvent::Delta(String::new())));
    }

    #[test]
    fn parse_delta_preserves_unicode_and_newlines() {
        let event = parse_shim_line(r#"{"type":"delta","text":"a\nb é"}"#);
        assert_eq!(event, Some(AskAiStreamEvent::Delta("a\nb é".to_string())));
    }

    #[test]
    fn parse_done_line() {
        assert_eq!(
            parse_shim_line(r#"{"type":"done"}"#),
            Some(AskAiStreamEvent::Done)
        );
    }

    #[test]
    fn parse_ready_line() {
        assert_eq!(
            parse_shim_line(r#"{"type":"ready"}"#),
            Some(AskAiStreamEvent::Ready)
        );
    }

    #[test]
    fn parse_error_line() {
        assert_eq!(
            parse_shim_line(r#"{"type":"error","message":"boom"}"#),
            Some(AskAiStreamEvent::Error("boom".to_string()))
        );
    }

    #[test]
    fn parse_error_line_missing_message_uses_default() {
        assert_eq!(
            parse_shim_line(r#"{"type":"error"}"#),
            Some(AskAiStreamEvent::Error(
                "Ask AI shim reported an error".to_string()
            ))
        );
    }

    #[test]
    fn parse_blank_line_is_none() {
        assert_eq!(parse_shim_line(""), None);
        assert_eq!(parse_shim_line("   "), None);
        assert_eq!(parse_shim_line("\t\n"), None);
    }

    #[test]
    fn parse_malformed_json_is_none() {
        assert_eq!(parse_shim_line("{not json"), None);
        assert_eq!(parse_shim_line("garbage"), None);
        assert_eq!(parse_shim_line(r#"{"type":}"#), None);
    }

    #[test]
    fn parse_json_without_type_is_none() {
        assert_eq!(parse_shim_line(r#"{"text":"hi"}"#), None);
    }

    #[test]
    fn parse_non_string_type_is_none() {
        assert_eq!(parse_shim_line(r#"{"type":42}"#), None);
    }

    #[test]
    fn parse_unknown_type_is_none() {
        assert_eq!(parse_shim_line(r#"{"type":"heartbeat"}"#), None);
        assert_eq!(parse_shim_line(r#"{"type":"tool_call"}"#), None);
    }

    #[test]
    fn parse_tolerates_surrounding_whitespace() {
        assert_eq!(
            parse_shim_line("  {\"type\":\"done\"}  "),
            Some(AskAiStreamEvent::Done)
        );
    }

    #[test]
    fn cancel_handle_round_trips() {
        let cancel = AskAiCancel::new();
        assert!(!cancel.is_cancelled());
        cancel.cancel();
        assert!(cancel.is_cancelled());
    }

    #[test]
    fn stderr_tail_keeps_last_lines() {
        let buffer = "l1\nl2\nl3\nl4\nl5\nl6\nl7\n";
        assert_eq!(stderr_tail(buffer), "l3\nl4\nl5\nl6\nl7");
    }

    #[test]
    fn stderr_tail_empty_for_blank() {
        assert_eq!(stderr_tail("   \n  "), "");
    }

    #[test]
    fn parse_tool_call_line_well_formed() {
        let call = parse_tool_call_line(
            r#"{"type":"tool_call","id":"c1","tool":"search","params":{"query":"hi","maxResults":5}}"#,
        )
        .expect("tool_call should parse");
        assert_eq!(call.id, "c1");
        assert_eq!(call.tool, "search");
        assert!(call.params.is_object());
        assert_eq!(
            call.params.get("query").and_then(|v| v.as_str()),
            Some("hi")
        );
        assert_eq!(
            call.params.get("maxResults").and_then(|v| v.as_u64()),
            Some(5)
        );
    }

    #[test]
    fn parse_tool_call_line_missing_params_defaults_empty_object() {
        let call = parse_tool_call_line(r#"{"type":"tool_call","id":"c2","tool":"timeline"}"#)
            .expect("tool_call should parse");
        assert_eq!(call.id, "c2");
        assert_eq!(call.tool, "timeline");
        assert_eq!(call.params, serde_json::json!({}));
    }

    #[test]
    fn parse_tool_call_line_tolerates_surrounding_whitespace() {
        let call =
            parse_tool_call_line("  {\"type\":\"tool_call\",\"id\":\"c3\",\"tool\":\"show-text\"}  ")
                .expect("tool_call should parse");
        assert_eq!(call.id, "c3");
        assert_eq!(call.tool, "show-text");
    }

    #[test]
    fn parse_tool_call_line_non_tool_call_is_none() {
        assert_eq!(parse_tool_call_line(r#"{"type":"done"}"#), None);
        assert_eq!(
            parse_tool_call_line(r#"{"type":"delta","text":"hi"}"#),
            None
        );
        assert_eq!(parse_tool_call_line(r#"{"type":"ready"}"#), None);
    }

    #[test]
    fn parse_tool_call_line_missing_fields_is_none() {
        // Missing id.
        assert_eq!(
            parse_tool_call_line(r#"{"type":"tool_call","tool":"search"}"#),
            None
        );
        // Missing tool.
        assert_eq!(
            parse_tool_call_line(r#"{"type":"tool_call","id":"c1"}"#),
            None
        );
        // Non-string id/tool.
        assert_eq!(
            parse_tool_call_line(r#"{"type":"tool_call","id":1,"tool":"search"}"#),
            None
        );
    }

    #[test]
    fn parse_tool_call_line_blank_and_malformed_is_none() {
        assert_eq!(parse_tool_call_line(""), None);
        assert_eq!(parse_tool_call_line("   "), None);
        assert_eq!(parse_tool_call_line("{not json"), None);
    }

    #[test]
    fn tool_result_line_ok_shape() {
        let line = tool_result_line(&"c1".to_string(), &Ok(serde_json::json!({"hits": 3})));
        let value: serde_json::Value = serde_json::from_str(&line).expect("valid json");
        assert_eq!(
            value,
            serde_json::json!({
                "type": "tool_result",
                "id": "c1",
                "ok": true,
                "result": {"hits": 3},
            })
        );
    }

    #[test]
    fn tool_result_line_err_shape() {
        let line = tool_result_line(&"c2".to_string(), &Err("broker down".to_string()));
        let value: serde_json::Value = serde_json::from_str(&line).expect("valid json");
        assert_eq!(
            value,
            serde_json::json!({
                "type": "tool_result",
                "id": "c2",
                "ok": false,
                "error": "broker down",
            })
        );
    }

    #[test]
    fn tool_result_line_emits_single_line() {
        let line = tool_result_line(&"c3".to_string(), &Ok(serde_json::Value::Null));
        assert!(!line.contains('\n'));
    }

    #[test]
    fn tool_counts_against_cap_exempts_reference_captures() {
        assert!(!tool_counts_against_cap("reference_captures"));
        assert!(tool_counts_against_cap("search"));
        assert!(tool_counts_against_cap("timeline"));
        assert!(tool_counts_against_cap("show_text"));
    }

    #[test]
    fn parse_models_line_well_formed() {
        let models = parse_models_line(
            r#"{"type":"models","models":[{"value":"anthropic:claude-opus-4","provider":"anthropic","id":"claude-opus-4","name":"Claude Opus 4"}]}"#,
        )
        .expect("models line should parse");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].value, "anthropic:claude-opus-4");
        assert_eq!(models[0].provider, "anthropic");
        assert_eq!(models[0].id, "claude-opus-4");
        assert_eq!(models[0].name, "Claude Opus 4");
    }

    #[test]
    fn parse_models_line_empty_list() {
        let models =
            parse_models_line(r#"{"type":"models","models":[]}"#).expect("empty list parses");
        assert!(models.is_empty());
    }

    #[test]
    fn parse_models_line_rejects_other_types_and_garbage() {
        assert_eq!(parse_models_line(r#"{"type":"done"}"#), None);
        assert_eq!(parse_models_line(r#"{"type":"delta","text":"hi"}"#), None);
        assert_eq!(parse_models_line(""), None);
        assert_eq!(parse_models_line("{not json"), None);
        // Missing `models` field.
        assert_eq!(parse_models_line(r#"{"type":"models"}"#), None);
    }
}
