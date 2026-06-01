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

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

/// A single streamed event parsed from one line of shim stdout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskAiStreamEvent {
    /// `{"type":"ready"}` — session created (optional; may be absent).
    Ready,
    /// `{"type":"delta","text":"..."}` — append text chunk to the streamed answer.
    Delta(String),
    /// `{"type":"done"}` — answer complete.
    Done,
    /// `{"type":"error","message":"..."}` — failure.
    Error(String),
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
/// Writes one JSON line `{"prompt":"..."}\n` to the child's stdin then closes
/// it. Reads newline-delimited JSON from stdout, parsing each line via
/// [`parse_shim_line`]. stderr is drained into a buffer used for diagnostics:
/// a non-zero exit without a terminal `done`/`error` event surfaces the stderr
/// tail as the error message. `cancel` kills the child when triggered.
pub async fn run_pi_ask_ai_session<F>(
    node_path: &Path,
    shim_path: &Path,
    pi_executable: Option<&str>,
    prompt: &str,
    mut on_event: F,
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
    // Pass through PI_CODING_AGENT_DIR only when the parent env already set it.
    if let Some(agent_dir) = std::env::var_os("PI_CODING_AGENT_DIR") {
        command.env("PI_CODING_AGENT_DIR", agent_dir);
    }

    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to spawn Ask AI shim via node: {error}"))?;

    // Write the prompt as one JSON line, then close stdin.
    if let Some(mut stdin) = child.stdin.take() {
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
        // Drop closes stdin.
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

    if let Some(stdout) = child.stdout.take() {
        let mut lines = BufReader::new(stdout).lines();
        loop {
            if cancel.is_cancelled() {
                let _ = child.start_kill();
                break;
            }

            let next = lines.next_line().await;
            match next {
                Ok(Some(line)) => {
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
}
