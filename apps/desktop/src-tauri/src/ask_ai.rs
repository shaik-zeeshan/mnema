mod pi_agent_session;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use app_infra::brokered_access::{
    BrokerClientIdentity, BrokerClientIdentitySource, BrokerSearchRequest, BrokerSearchResult,
    BrokerTimelineRequest, BrokeredCaptureAccess, BrokeredCaptureRequest, BrokeredCaptureResponse,
};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

use pi_agent_session::AskAiCancel;

/// Process registry mapping a conversation id to its cancellation handle, so
/// `ask_ai_cancel` can kill a streaming session started by `ask_ai_start`.
/// Module-level so it survives across separate Tauri command invocations
/// without touching lib.rs state wiring.
static ASK_AI_SESSIONS: OnceLock<Mutex<HashMap<String, AskAiCancel>>> = OnceLock::new();

fn ask_ai_sessions() -> &'static Mutex<HashMap<String, AskAiCancel>> {
    ASK_AI_SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register_ask_ai_session(conversation_id: &str, cancel: AskAiCancel) {
    if let Ok(mut sessions) = ask_ai_sessions().lock() {
        sessions.insert(conversation_id.to_string(), cancel);
    }
}

fn remove_ask_ai_session(conversation_id: &str) {
    if let Ok(mut sessions) = ask_ai_sessions().lock() {
        sessions.remove(conversation_id);
    }
}

fn take_ask_ai_session(conversation_id: &str) -> Option<AskAiCancel> {
    ask_ai_sessions()
        .lock()
        .ok()
        .and_then(|mut sessions| sessions.remove(conversation_id))
}

const ASK_AI_STATUS_EVENT: &str = "ask_ai_status";
const ASK_AI_DELTA_EVENT: &str = "ask_ai_delta";
const ASK_AI_DONE_EVENT: &str = "ask_ai_done";
const ASK_AI_ERROR_EVENT: &str = "ask_ai_error";

/// Translate the persisted `askAiMaxToolCalls` setting (`0` = no cap) into the
/// per-session cap passed to the agent loop. `0` becomes `usize::MAX` so the
/// agent may issue unlimited follow-up brokered queries.
fn resolve_tool_call_cap(setting: u32) -> usize {
    if setting == 0 {
        usize::MAX
    } else {
        setting as usize
    }
}

fn access_config_dir(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    app_handle
        .path()
        .app_config_dir()
        .map_err(|error| format!("failed to resolve app config dir: {error}"))
}

fn broker_access(app_handle: &tauri::AppHandle) -> Result<BrokeredCaptureAccess, String> {
    Ok(BrokeredCaptureAccess::from_config_dir(access_config_dir(
        app_handle,
    )?))
}

fn pi_broker_identity() -> Result<BrokerClientIdentity, String> {
    BrokerClientIdentity::new("PI", BrokerClientIdentitySource::Inferred)
        .map_err(|error| error.to_string())
}

fn validate_ask_ai_access_ready(
    ask_ai_enabled: bool,
    status: &crate::app_infra::PiRuntimeStatus,
) -> Result<(), String> {
    if !ask_ai_enabled {
        return Err("Ask AI access is disabled in settings".to_string());
    }
    if !status.ready {
        let reason = status
            .reason
            .as_deref()
            .unwrap_or("pi_unavailable");
        return Err(format!("Ask AI requires a ready PI runtime ({reason})"));
    }

    Ok(())
}

fn read_ask_ai_enabled(app_handle: &tauri::AppHandle) -> Result<bool, String> {
    let Some(settings_state) =
        app_handle.try_state::<crate::native_capture::RecordingSettingsState>()
    else {
        return Err("Ask AI settings are unavailable".to_string());
    };
    let enabled = settings_state
        .lock()
        .map_err(|_| "Ask AI settings are unavailable".to_string())?
        .settings
        .access
        .ask_ai_enabled;
    Ok(enabled)
}

/// Read the configured per-question tool-call cap (`0` = no cap). Falls back to
/// the default cap if the settings state is unavailable.
fn read_ask_ai_max_tool_calls(app_handle: &tauri::AppHandle) -> usize {
    let setting = app_handle
        .try_state::<crate::native_capture::RecordingSettingsState>()
        .and_then(|state| {
            state
                .lock()
                .ok()
                .map(|guard| guard.settings.access.ask_ai_max_tool_calls)
        })
        .unwrap_or_else(capture_types::default_ask_ai_max_tool_calls);
    resolve_tool_call_cap(setting)
}

async fn ensure_ask_ai_access_ready(app_handle: &tauri::AppHandle) -> Result<(), String> {
    let ask_ai_enabled = read_ask_ai_enabled(app_handle)?;
    let status = crate::app_infra::get_pi_runtime_status_inner(app_handle.clone()).await?;
    validate_ask_ai_access_ready(ask_ai_enabled, &status)?;

    Ok(())
}

async fn execute_pi_broker_request(
    app_handle: tauri::AppHandle,
    request: BrokeredCaptureRequest,
) -> Result<BrokeredCaptureResponse, String> {
    ensure_ask_ai_access_ready(&app_handle).await?;
    broker_access(&app_handle)?
        .execute_for_ask_ai(pi_broker_identity()?, request)
        .await
        .map_err(|error| format!("failed to execute Ask AI broker request: {error}"))
}

/// Map an Ask AI tool name + camelCase params object onto a brokered request.
///
/// Only the three Ask AI tools (`search`, `timeline`, `show_text`) are
/// accepted; `open`/`open_in_mnema` and anything else fall into the unknown
/// branch and are rejected, so they can never be issued as Ask AI data tools.
fn broker_request_from_tool(
    tool: &str,
    params: serde_json::Value,
) -> Result<BrokeredCaptureRequest, String> {
    match tool {
        "search" => {
            let request: BrokerSearchRequest = serde_json::from_value(params)
                .map_err(|error| format!("invalid Ask AI search params: {error}"))?;
            Ok(BrokeredCaptureRequest::Search(request))
        }
        "timeline" => {
            let request: BrokerTimelineRequest = serde_json::from_value(params)
                .map_err(|error| format!("invalid Ask AI timeline params: {error}"))?;
            Ok(BrokeredCaptureRequest::Timeline(request))
        }
        "show_text" => {
            let opaque_id = params
                .get("opaqueId")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "Ask AI show_text requires a non-empty opaqueId".to_string())?
                .to_string();
            Ok(BrokeredCaptureRequest::ShowText { opaque_id })
        }
        other => Err(format!("unknown Ask AI tool: {other}")),
    }
}

/// Convert a brokered response into the JSON value handed back to the shim as a
/// tool result, or an error message (broker error envelopes become `Err`).
fn broker_response_to_tool_value(
    response: BrokeredCaptureResponse,
) -> Result<serde_json::Value, String> {
    match response {
        BrokeredCaptureResponse::Search(response) => serde_json::to_value(response)
            .map_err(|error| format!("failed to serialize Ask AI search result: {error}")),
        BrokeredCaptureResponse::Timeline(response) => serde_json::to_value(response)
            .map_err(|error| format!("failed to serialize Ask AI timeline result: {error}")),
        BrokeredCaptureResponse::ShowText(response) => serde_json::to_value(response)
            .map_err(|error| format!("failed to serialize Ask AI show_text result: {error}")),
        BrokeredCaptureResponse::Error(error) => Err(error.message),
        BrokeredCaptureResponse::AuthStatus(_) | BrokeredCaptureResponse::OpenInMnema(_) => {
            Err("unexpected Ask AI broker response".to_string())
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskAiShowTextRequest {
    opaque_id: String,
}

#[tauri::command]
pub async fn get_pi_runtime_status(
    app_handle: tauri::AppHandle,
) -> Result<crate::app_infra::PiRuntimeStatus, String> {
    crate::app_infra::get_pi_runtime_status_inner(app_handle).await
}

#[tauri::command]
pub async fn ask_ai_broker_search(
    app_handle: tauri::AppHandle,
    request: BrokerSearchRequest,
) -> Result<BrokeredCaptureResponse, String> {
    execute_pi_broker_request(app_handle, BrokeredCaptureRequest::Search(request)).await
}

#[tauri::command]
pub async fn ask_ai_broker_timeline(
    app_handle: tauri::AppHandle,
    request: BrokerTimelineRequest,
) -> Result<BrokeredCaptureResponse, String> {
    execute_pi_broker_request(app_handle, BrokeredCaptureRequest::Timeline(request)).await
}

#[tauri::command]
pub async fn ask_ai_broker_show_text(
    app_handle: tauri::AppHandle,
    request: AskAiShowTextRequest,
) -> Result<BrokeredCaptureResponse, String> {
    execute_pi_broker_request(
        app_handle,
        BrokeredCaptureRequest::ShowText {
            opaque_id: request.opaque_id,
        },
    )
    .await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskAiStartRequest {
    conversation_id: String,
    question: String,
    seed_query: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskAiCancelRequest {
    conversation_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AskAiAvailability {
    available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

/// Resolve `node` against the user's real terminal shell PATH (never bare
/// `Command::new("node")`; packaged macOS apps lack the Homebrew PATH).
fn resolve_node_executable() -> Result<PathBuf, String> {
    crate::app_infra::executable_in_shell_path("node")
        .ok_or_else(|| "Ask AI requires Node.js (from your PI install) on PATH".to_string())
}

/// Resolve the Ask AI shim path: production resource dir first, then the dev
/// `CARGO_MANIFEST_DIR` fallback.
fn resolve_shim_path(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    if let Ok(resource_dir) = app_handle.path().resource_dir() {
        let candidate = resource_dir.join("resources/pi-ask-ai-shim.mjs");
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    let dev_candidate =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/pi-ask-ai-shim.mjs");
    if dev_candidate.is_file() {
        return Ok(dev_candidate);
    }

    Err("Ask AI shim is missing".to_string())
}

/// Build a single seed-context line for one broker search result.
fn format_seed_result_line(index: usize, result: &BrokerSearchResult) -> String {
    let app_label = result
        .context
        .as_ref()
        .and_then(|context| {
            context
                .app_name
                .clone()
                .or_else(|| context.app_bundle_id.clone())
        })
        .unwrap_or_else(|| "unknown app".to_string());

    let window_segment = result
        .context
        .as_ref()
        .and_then(|context| context.window_title.as_ref())
        .map(|title| format!(" · \"{title}\""))
        .unwrap_or_default();

    format!(
        "{}. [{} · {}{} · {}–{}] {}",
        index + 1,
        result.kind,
        app_label,
        window_segment,
        result.started_at,
        result.ended_at,
        result.snippet
    )
}

/// Assemble the full seeded prompt string sent to the shim over stdin.
fn build_ask_ai_prompt(
    question: &str,
    seed_query: Option<&str>,
    results: &[BrokerSearchResult],
) -> String {
    let mut prompt = String::new();
    prompt.push_str(
        "You are Mnema's Ask AI assistant. Answer the user's question using their own on-device \
screen and audio capture history. All data is the user's own, redacted, on-device capture. You \
have THREE tools, and there is NO way to open files or access anything beyond them: `search` \
finds redacted snippets plus opaque ids across the user's screen OCR and audio transcript \
history (optionally narrowed by a `from`/`to` RFC3339 time range and `app`/`windowTitle` \
filters); `timeline` returns coarse activity intervals for a bounded `from`/`to` window; \
`show_text` returns the full redacted text for one opaque id returned by `search`. When the \
seeded context below is missing or insufficient to answer, ISSUE follow-up tool calls to gather \
what you need before answering — prefer a concise `search` first, and use `show_text` sparingly \
for the specific results you need to read in full. Cite times and apps when useful, but never \
invent details. If you still cannot answer, say so briefly. Be concise and direct.\n",
    );

    if let Some(seed_query) = seed_query {
        if !results.is_empty() {
            prompt.push('\n');
            prompt.push_str(&format!(
                "Context from the user's captures for \"{seed_query}\":\n"
            ));
            for (index, result) in results.iter().enumerate() {
                prompt.push_str(&format_seed_result_line(index, result));
                prompt.push('\n');
            }
        }
    }

    prompt.push('\n');
    prompt.push_str(&format!("Question: {question}"));
    prompt
}

#[tauri::command]
pub async fn ask_ai_availability(
    app_handle: tauri::AppHandle,
) -> Result<AskAiAvailability, String> {
    let ask_ai_enabled = read_ask_ai_enabled(&app_handle)?;
    if !ask_ai_enabled {
        return Ok(AskAiAvailability {
            available: false,
            reason: Some("ask_ai_disabled".to_string()),
        });
    }

    let status = crate::app_infra::get_pi_runtime_status_inner(app_handle.clone()).await?;
    if status.ready {
        Ok(AskAiAvailability {
            available: true,
            reason: None,
        })
    } else {
        Ok(AskAiAvailability {
            available: false,
            reason: Some(status.reason.unwrap_or_else(|| "pi_unavailable".to_string())),
        })
    }
}

#[tauri::command]
pub async fn ask_ai_start(
    app_handle: tauri::AppHandle,
    request: AskAiStartRequest,
) -> Result<(), String> {
    ensure_ask_ai_access_ready(&app_handle).await?;

    let AskAiStartRequest {
        conversation_id,
        question,
        seed_query,
    } = request;

    // Resolve the seed query (trimmed, non-empty).
    let seed_query = seed_query
        .map(|query| query.trim().to_string())
        .filter(|query| !query.is_empty());

    // Best-effort seeding via the broker search path.
    let mut seed_results: Vec<BrokerSearchResult> = Vec::new();
    if let Some(seed_query) = seed_query.as_deref() {
        let _ = app_handle.emit(
            ASK_AI_STATUS_EVENT,
            serde_json::json!({
                "conversationId": conversation_id,
                "phase": "seeding",
                "seededResultCount": 0,
            }),
        );

        let search_request = BrokerSearchRequest {
            query: seed_query.to_string(),
            from: None,
            to: None,
            limit: Some(8),
            app: None,
            window_title: None,
        };

        match execute_pi_broker_request(
            app_handle.clone(),
            BrokeredCaptureRequest::Search(search_request),
        )
        .await
        {
            Ok(BrokeredCaptureResponse::Search(response)) => {
                seed_results = response.results;
            }
            // Broker error envelope or any other response: proceed unseeded.
            Ok(_) | Err(_) => {}
        }

        let _ = app_handle.emit(
            ASK_AI_STATUS_EVENT,
            serde_json::json!({
                "conversationId": conversation_id,
                "phase": "seeding",
                "seededResultCount": seed_results.len(),
            }),
        );
    }

    let _ = app_handle.emit(
        ASK_AI_STATUS_EVENT,
        serde_json::json!({
            "conversationId": conversation_id,
            "phase": "thinking",
        }),
    );

    let prompt = build_ask_ai_prompt(&question, seed_query.as_deref(), &seed_results);

    // Resolve node, shim, and the pi executable path (for SDK resolution in the shim).
    let node_path = resolve_node_executable()?;
    let shim_path = resolve_shim_path(&app_handle)?;
    let status = crate::app_infra::get_pi_runtime_status_inner(app_handle.clone()).await?;
    let pi_executable = status.executable_path;

    // Resolve the per-question tool-call cap from settings (0 => unlimited).
    let max_tool_calls = read_ask_ai_max_tool_calls(&app_handle);

    let cancel = AskAiCancel::new();
    register_ask_ai_session(&conversation_id, cancel.clone());

    // Build the brokered tool invoker. Every tool call rides
    // `execute_pi_broker_request`, which enforces Ask-AI access readiness plus
    // the All-Retained broker scope and redaction/audit Rust-side; do not
    // bypass it.
    let invoker_app_handle = app_handle.clone();
    let tool_invoker: pi_agent_session::AskAiToolInvoker =
        Box::new(move |tool: String, params: serde_json::Value| {
            let app_handle = invoker_app_handle.clone();
            Box::pin(async move {
                let request = broker_request_from_tool(&tool, params)?;
                let response = execute_pi_broker_request(app_handle, request).await?;
                broker_response_to_tool_value(response)
            })
        });

    // Stream on a background task so the command returns promptly after launch.
    let task_app_handle = app_handle.clone();
    let task_conversation_id = conversation_id.clone();
    tauri::async_runtime::spawn(async move {
        let mut saw_terminal = false;
        let emit_handle = task_app_handle.clone();
        let emit_conversation_id = task_conversation_id.clone();
        let run_result = pi_agent_session::run_pi_ask_ai_session(
            &node_path,
            &shim_path,
            pi_executable.as_deref(),
            &prompt,
            max_tool_calls,
            |event| match event {
                pi_agent_session::AskAiStreamEvent::Ready => {}
                pi_agent_session::AskAiStreamEvent::Delta(text) => {
                    let _ = emit_handle.emit(
                        ASK_AI_DELTA_EVENT,
                        serde_json::json!({
                            "conversationId": emit_conversation_id,
                            "text": text,
                        }),
                    );
                }
                pi_agent_session::AskAiStreamEvent::ToolCall { tool, .. } => {
                    let friendly = match tool.as_str() {
                        "search" => "Searching your captures",
                        "timeline" => "Scanning your timeline",
                        "show_text" => "Reading a capture",
                        other => other,
                    };
                    let _ = emit_handle.emit(
                        ASK_AI_STATUS_EVENT,
                        serde_json::json!({
                            "conversationId": emit_conversation_id,
                            "phase": "tool",
                            "tool": friendly,
                        }),
                    );
                }
                pi_agent_session::AskAiStreamEvent::ToolResult { .. } => {}
                pi_agent_session::AskAiStreamEvent::Done => {
                    saw_terminal = true;
                    let _ = emit_handle.emit(
                        ASK_AI_DONE_EVENT,
                        serde_json::json!({ "conversationId": emit_conversation_id }),
                    );
                }
                pi_agent_session::AskAiStreamEvent::Error(message) => {
                    saw_terminal = true;
                    let _ = emit_handle.emit(
                        ASK_AI_ERROR_EVENT,
                        serde_json::json!({
                            "conversationId": emit_conversation_id,
                            "message": message,
                        }),
                    );
                }
            },
            tool_invoker,
            cancel,
        )
        .await;

        if let Err(error) = run_result {
            if !saw_terminal {
                let _ = task_app_handle.emit(
                    ASK_AI_ERROR_EVENT,
                    serde_json::json!({
                        "conversationId": task_conversation_id,
                        "message": error,
                    }),
                );
            }
        }

        remove_ask_ai_session(&task_conversation_id);
    });

    Ok(())
}

#[tauri::command]
pub async fn ask_ai_cancel(
    _app_handle: tauri::AppHandle,
    request: AskAiCancelRequest,
) -> Result<(), String> {
    if let Some(cancel) = take_ask_ai_session(&request.conversation_id) {
        cancel.cancel();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use app_infra::brokered_access::{
        BrokerAuthStatusKind, BrokerErrorResponse, BrokerSearchResponse,
        BrokerSearchResultContext, BrokerShowTextResponse,
    };

    fn ready_pi_status() -> crate::app_infra::PiRuntimeStatus {
        crate::app_infra::PiRuntimeStatus {
            source: crate::app_infra::PiRuntimeSource::Path,
            executable_path: Some("/usr/local/bin/pi".to_string()),
            version: Some("0.65.0".to_string()),
            minimum_version: "0.65.0".to_string(),
            version_ok: true,
            auth_json_path: "/Users/tester/.pi/agent/auth.json".to_string(),
            auth_json_exists: true,
            provider_configured: true,
            provider_count: 1,
            ready: true,
            reason: None,
        }
    }

    #[test]
    fn pi_broker_identity_matches_existing_pi_client_label() {
        let identity = pi_broker_identity().expect("PI identity should be valid");

        assert_eq!(identity.label, "PI");
        assert_eq!(identity.normalized_label, "pi");
        assert_eq!(identity.source, BrokerClientIdentitySource::Inferred);
    }

    #[test]
    fn ask_ai_access_ready_rejects_disabled_setting() {
        let error = validate_ask_ai_access_ready(false, &ready_pi_status())
            .expect_err("disabled Ask AI should be rejected");

        assert_eq!(error, "Ask AI access is disabled in settings");
    }

    #[test]
    fn ask_ai_access_ready_rejects_unready_pi() {
        let mut status = ready_pi_status();
        status.ready = false;
        status.reason = Some("pi_auth_missing".to_string());

        let error = validate_ask_ai_access_ready(true, &status)
            .expect_err("unready PI should be rejected");

        assert_eq!(error, "Ask AI requires a ready PI runtime (pi_auth_missing)");
    }

    #[test]
    fn ask_ai_access_ready_accepts_enabled_setting_and_ready_pi() {
        validate_ask_ai_access_ready(true, &ready_pi_status())
            .expect("enabled Ask AI with ready PI should be accepted");
    }

    fn sample_result() -> BrokerSearchResult {
        BrokerSearchResult {
            opaque_id: "op-1".to_string(),
            kind: "frame".to_string(),
            snippet: "build passed".to_string(),
            started_at: "2026-01-01T10:00:00Z".to_string(),
            ended_at: "2026-01-01T10:01:00Z".to_string(),
            context: Some(BrokerSearchResultContext {
                app_bundle_id: Some("com.apple.dt.Xcode".to_string()),
                app_name: Some("Xcode".to_string()),
                window_title: Some("ContentView.swift".to_string()),
            }),
        }
    }

    #[test]
    fn prompt_unseeded_omits_context_block() {
        let prompt = build_ask_ai_prompt("What did I do?", None, &[]);
        assert!(!prompt.contains("Context from the user's captures"));
        assert!(prompt.ends_with("Question: What did I do?"));
    }

    #[test]
    fn prompt_with_empty_results_omits_context_block() {
        let prompt = build_ask_ai_prompt("Q?", Some("build"), &[]);
        assert!(!prompt.contains("Context from the user's captures"));
    }

    #[test]
    fn prompt_seeded_includes_numbered_context() {
        let prompt = build_ask_ai_prompt("Did the build pass?", Some("build"), &[sample_result()]);
        assert!(prompt.contains("Context from the user's captures for \"build\":"));
        assert!(prompt.contains(
            "1. [frame · Xcode · \"ContentView.swift\" · 2026-01-01T10:00:00Z–2026-01-01T10:01:00Z] build passed"
        ));
        assert!(prompt.ends_with("Question: Did the build pass?"));
    }

    #[test]
    fn seed_line_falls_back_to_bundle_id_then_unknown() {
        let mut result = sample_result();
        result.context = Some(BrokerSearchResultContext {
            app_bundle_id: Some("com.example.app".to_string()),
            app_name: None,
            window_title: None,
        });
        let line = format_seed_result_line(0, &result);
        assert!(line.contains("· com.example.app ·"));
        assert!(!line.contains("\""));

        result.context = None;
        let line = format_seed_result_line(2, &result);
        assert!(line.starts_with("3. [frame · unknown app ·"));
    }

    #[test]
    fn availability_serializes_camel_case() {
        let json = serde_json::to_string(&AskAiAvailability {
            available: false,
            reason: Some("ask_ai_disabled".to_string()),
        })
        .expect("serialize");
        assert_eq!(json, r#"{"available":false,"reason":"ask_ai_disabled"}"#);

        let json = serde_json::to_string(&AskAiAvailability {
            available: true,
            reason: None,
        })
        .expect("serialize");
        assert_eq!(json, r#"{"available":true}"#);
    }

    #[test]
    fn resolve_tool_call_cap_treats_zero_as_unlimited() {
        assert_eq!(resolve_tool_call_cap(0), usize::MAX);
        assert_eq!(resolve_tool_call_cap(1), 1);
        assert_eq!(resolve_tool_call_cap(12), 12);
        assert_eq!(resolve_tool_call_cap(250), 250);
    }

    #[test]
    fn broker_request_from_tool_search_maps_to_search_variant() {
        let request = broker_request_from_tool(
            "search",
            serde_json::json!({ "query": "build", "limit": 5 }),
        )
        .expect("search params should parse");

        match request {
            BrokeredCaptureRequest::Search(search) => {
                assert_eq!(search.query, "build");
                assert_eq!(search.limit, Some(5));
            }
            other => panic!("expected Search, got {other:?}"),
        }
    }

    #[test]
    fn broker_request_from_tool_timeline_maps_to_timeline_variant() {
        let request = broker_request_from_tool(
            "timeline",
            serde_json::json!({
                "from": "2026-01-01T00:00:00Z",
                "to": "2026-01-01T01:00:00Z",
            }),
        )
        .expect("timeline params should parse");

        match request {
            BrokeredCaptureRequest::Timeline(timeline) => {
                assert_eq!(timeline.from, "2026-01-01T00:00:00Z");
                assert_eq!(timeline.to, "2026-01-01T01:00:00Z");
            }
            other => panic!("expected Timeline, got {other:?}"),
        }
    }

    #[test]
    fn broker_request_from_tool_show_text_extracts_opaque_id() {
        let request =
            broker_request_from_tool("show_text", serde_json::json!({ "opaqueId": "op-7" }))
                .expect("show_text params should parse");

        match request {
            BrokeredCaptureRequest::ShowText { opaque_id } => assert_eq!(opaque_id, "op-7"),
            other => panic!("expected ShowText, got {other:?}"),
        }
    }

    #[test]
    fn broker_request_from_tool_rejects_unknown_tool() {
        let error = broker_request_from_tool("open", serde_json::json!({ "opaqueId": "op-1" }))
            .expect_err("open is not an Ask AI tool");
        assert_eq!(error, "unknown Ask AI tool: open");

        let error = broker_request_from_tool("open_in_mnema", serde_json::json!({}))
            .expect_err("open_in_mnema is not an Ask AI tool");
        assert_eq!(error, "unknown Ask AI tool: open_in_mnema");
    }

    #[test]
    fn broker_request_from_tool_rejects_missing_opaque_id() {
        let error = broker_request_from_tool("show_text", serde_json::json!({}))
            .expect_err("missing opaqueId should error");
        assert_eq!(error, "Ask AI show_text requires a non-empty opaqueId");

        let error = broker_request_from_tool("show_text", serde_json::json!({ "opaqueId": "  " }))
            .expect_err("blank opaqueId should error");
        assert_eq!(error, "Ask AI show_text requires a non-empty opaqueId");
    }

    #[test]
    fn broker_response_to_tool_value_serializes_search() {
        let response = BrokeredCaptureResponse::Search(BrokerSearchResponse {
            results: vec![sample_result()],
            limit: 8,
        });

        let value = broker_response_to_tool_value(response).expect("search serializes");
        assert_eq!(value["limit"], serde_json::json!(8));
        assert_eq!(value["results"][0]["opaqueId"], serde_json::json!("op-1"));
    }

    #[test]
    fn broker_response_to_tool_value_serializes_show_text() {
        let response = BrokeredCaptureResponse::ShowText(BrokerShowTextResponse {
            opaque_id: "op-1".to_string(),
            kind: "frame".to_string(),
            text: "full redacted text".to_string(),
        });

        let value = broker_response_to_tool_value(response).expect("show_text serializes");
        assert_eq!(value["opaqueId"], serde_json::json!("op-1"));
        assert_eq!(value["text"], serde_json::json!("full redacted text"));
    }

    #[test]
    fn broker_response_to_tool_value_error_returns_message() {
        let response = BrokeredCaptureResponse::Error(BrokerErrorResponse {
            error: BrokerAuthStatusKind::AuthorizationRequired,
            message: "result is unavailable or outside the grant scope".to_string(),
        });

        let error = broker_response_to_tool_value(response)
            .expect_err("error envelope should become Err");
        assert_eq!(error, "result is unavailable or outside the grant scope");
    }
}
