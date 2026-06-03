mod pi_agent_session;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use app_infra::brokered_access::{
    BrokerClientIdentity, BrokerClientIdentitySource, BrokerSearchRequest, BrokerSearchResult,
    BrokerTimelineRequest, BrokeredCaptureAccess, BrokeredCaptureRequest, BrokeredCaptureResponse,
};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

use pi_agent_session::AskAiCancel;

/// Monotonic counter minting a unique ownership token per session registration.
/// Lets a finishing session task remove only its own registry entry and never
/// evict a newer session that reused the same conversation id.
static ASK_AI_SESSION_TOKEN: AtomicU64 = AtomicU64::new(0);

/// A live Ask AI thread's control handles: its cancellation flag and the
/// follow-up prompt sender that feeds raw follow-up questions into the resident
/// PI session. Dropping `prompt_tx` (by removing the handle) makes the session
/// task's `prompt_rx.recv()` return `None` and tear the thread down between
/// turns; `cancel` hard-kills it mid-turn.
///
/// `token` is a per-registration ownership stamp assigned by
/// `register_ask_ai_session` (the value passed at construction is overwritten):
/// a session task only removes the registry entry whose token matches the one
/// register returned, so two `ask_ai_start` calls sharing a conversation id
/// don't let the first to finish evict the second's still-live handle.
struct AskAiSessionHandle {
    cancel: AskAiCancel,
    prompt_tx: tokio::sync::mpsc::UnboundedSender<String>,
    token: u64,
}

/// Process registry mapping a conversation id (the whole thread/session) to its
/// control handles, so `ask_ai_cancel` can kill a streaming thread started by
/// `ask_ai_start` and `ask_ai_followup` can route a follow-up prompt into the
/// live session. Module-level so it survives across separate Tauri command
/// invocations without touching lib.rs state wiring.
static ASK_AI_SESSIONS: OnceLock<Mutex<HashMap<String, AskAiSessionHandle>>> = OnceLock::new();

fn ask_ai_sessions() -> &'static Mutex<HashMap<String, AskAiSessionHandle>> {
    ASK_AI_SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Register (or replace) the handle for a conversation, stamping it with a fresh
/// unique ownership token that the caller records for a later owner-checked
/// removal. If an existing live handle is overwritten — two `ask_ai_start` calls
/// racing on one conversation id — the prior handle is hard-cancelled here (its
/// `cancel` flag set AND its `prompt_tx` dropped) so the displaced session winds
/// down rather than running orphaned alongside the newer one. Dropping
/// `prompt_tx` alone only tears the old session down BETWEEN turns
/// (`prompt_rx.recv()` returns `None`); a session that is mid-turn is blocked
/// reading shim stdout and is only stopped by the cancel flag, which the spawned
/// task checks at the top of its read loop. Without setting it, the displaced
/// streamer would keep emitting `ask_ai_delta`/`ask_ai_done` events tagged with
/// the SAME conversation id as the newer session, interleaving output, and would
/// be unreachable by `ask_ai_cancel` (which now resolves the newer handle).
/// Returns the minted token.
fn register_ask_ai_session(conversation_id: &str, mut handle: AskAiSessionHandle) -> u64 {
    // Stamp the registration with a fresh unique token so a finishing session can
    // later remove only its own entry.
    let token = next_ask_ai_session_token();
    handle.token = token;
    if let Ok(mut sessions) = ask_ai_sessions().lock() {
        // `insert` returns any prior handle. Hard-cancel it (set its `cancel`
        // flag) before dropping it, so a displaced session that is mid-turn is
        // killed rather than only winding down between turns; dropping the
        // returned handle also releases its `prompt_tx`. The flag is the same
        // `Arc<AtomicBool>` the displaced streaming task polls, so this reaches a
        // task that is no longer in the registry.
        if let Some(previous) = sessions.insert(conversation_id.to_string(), handle) {
            previous.cancel.cancel();
        }
    }
    token
}

/// Mint the next unique session ownership token.
fn next_ask_ai_session_token() -> u64 {
    ASK_AI_SESSION_TOKEN.fetch_add(1, Ordering::Relaxed)
}

/// Remove the registry entry for a conversation only if it still holds the
/// handle stamped with `token`. A finishing session task calls this so it never
/// evicts a newer session that reused the same conversation id (the newer
/// registration carries a different token).
fn remove_ask_ai_session_if_owner(conversation_id: &str, token: u64) {
    if let Ok(mut sessions) = ask_ai_sessions().lock() {
        if sessions
            .get(conversation_id)
            .is_some_and(|handle| handle.token == token)
        {
            sessions.remove(conversation_id);
        }
    }
}

#[cfg(test)]
fn remove_ask_ai_session(conversation_id: &str) {
    if let Ok(mut sessions) = ask_ai_sessions().lock() {
        sessions.remove(conversation_id);
    }
}

/// Remove and return the handle for a conversation, dropping its `prompt_tx`
/// (which tears down the resident session between turns). Used by cancel.
fn take_ask_ai_session(conversation_id: &str) -> Option<AskAiSessionHandle> {
    ask_ai_sessions()
        .lock()
        .ok()
        .and_then(|mut sessions| sessions.remove(conversation_id))
}

/// Clone the follow-up prompt sender for a live conversation WITHOUT removing
/// it, so a follow-up can be routed into the resident session while the thread
/// stays registered. Returns `None` for an unknown/dead conversation id.
fn ask_ai_session_prompt_sender(
    conversation_id: &str,
) -> Option<tokio::sync::mpsc::UnboundedSender<String>> {
    ask_ai_sessions().lock().ok().and_then(|sessions| {
        sessions
            .get(conversation_id)
            .map(|handle| handle.prompt_tx.clone())
    })
}

const ASK_AI_STATUS_EVENT: &str = "ask_ai_status";
const ASK_AI_DELTA_EVENT: &str = "ask_ai_delta";
const ASK_AI_DONE_EVENT: &str = "ask_ai_done";
const ASK_AI_ERROR_EVENT: &str = "ask_ai_error";
const ASK_AI_SOURCE_EVENT: &str = "ask_ai_source";

/// Per-kind caps on the nominated Answer Source set emitted to the frontend.
const ASK_AI_SOURCE_FRAME_CAP: usize = 6;
const ASK_AI_SOURCE_AUDIO_CAP: usize = 4;

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
        let reason = status.reason.as_deref().unwrap_or("pi_unavailable");
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

/// Read the configured Quick Recall model (`provider:id`), or `None` to let the
/// PI runtime pick its default. Blank values normalize to `None`.
fn read_ask_ai_model(app_handle: &tauri::AppHandle) -> Option<String> {
    app_handle
        .try_state::<crate::native_capture::RecordingSettingsState>()
        .and_then(|state| {
            state
                .lock()
                .ok()
                .and_then(|guard| guard.settings.access.ask_ai_model.clone())
        })
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
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

/// One Answer Source resolved from a nominated opaque id: authoritative
/// frame/audio identity from the signed reference, plus retained metadata from
/// the search result the model actually received.
struct ResolvedAskAiSource {
    kind: String,
    frame_id: Option<i64>,
    audio_segment_id: Option<i64>,
    app_name: Option<String>,
    window_title: Option<String>,
    started_at: String,
    ended_at: String,
    // Audio Search Result Anchor: sub-segment match timing + aligned frame for
    // audio sources so the dashboard lands on the cited moment rather than the
    // segment start. Always `None` for frame sources.
    span_start_ms: Option<i64>,
    aligned_frame_id: Option<i64>,
}

/// Build the capped, de-duped, ordered Answer Source list from nominated opaque
/// ids. `resolve` returns `None` for any id that fails HMAC validation or has no
/// retained metadata (dropped). Returns `(sources_json, accepted, dropped)`,
/// capped to `ASK_AI_SOURCE_FRAME_CAP` frame + `ASK_AI_SOURCE_AUDIO_CAP` audio
/// sources, preserving nomination order.
fn build_ask_ai_sources<F>(
    opaque_ids: &[String],
    mut resolve: F,
) -> (Vec<serde_json::Value>, usize, usize)
where
    F: FnMut(&str) -> Option<ResolvedAskAiSource>,
{
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut sources: Vec<serde_json::Value> = Vec::new();
    let mut frame_count = 0usize;
    let mut audio_count = 0usize;

    for id in opaque_ids {
        // Skip duplicates so a repeated nomination is dropped, not double-counted.
        if !seen.insert(id.clone()) {
            continue;
        }
        let Some(source) = resolve(id) else {
            continue;
        };

        // Classify by authoritative kind; ignore anything that is not a known
        // frame/audio source, and skip once that kind's cap is full.
        match source.kind.as_str() {
            "frame" => {
                if frame_count >= ASK_AI_SOURCE_FRAME_CAP {
                    continue;
                }
                frame_count += 1;
            }
            "audio" => {
                if audio_count >= ASK_AI_SOURCE_AUDIO_CAP {
                    continue;
                }
                audio_count += 1;
            }
            _ => continue,
        }

        sources.push(serde_json::json!({
            "kind": source.kind,
            "frameId": source.frame_id,
            "audioSegmentId": source.audio_segment_id,
            "appName": source.app_name,
            "windowTitle": source.window_title,
            "startedAt": source.started_at,
            "endedAt": source.ended_at,
            // Audio Search Result Anchor: sub-segment match span + aligned frame
            // so the dashboard lands on the cited moment (frame sources: null).
            // Field names mirror `selectAudio`'s open payload so the dashboard
            // consumer (`payload.spanStartMs`, `payload.alignedFrameId`) works
            // unchanged.
            "spanStartMs": source.span_start_ms,
            "alignedFrameId": source.aligned_frame_id,
            // Microphone/system distinction for audio sources. The pure builder
            // never sets it; a best-effort async post-pass in
            // `handle_reference_captures` fills audio sources from the DB.
            "sourceKind": serde_json::Value::Null,
        }));
    }

    let accepted = sources.len();
    let dropped = opaque_ids.len().saturating_sub(accepted);
    (sources, accepted, dropped)
}

/// Handle the shim's `reference_captures` presentation tool: validate + decode
/// the nominated opaque ids, attach retained metadata, cap the set, emit a
/// single `ask_ai_source` event to the frontend, and ack the shim with
/// `{ accepted, dropped }`. This never touches the broker dispatch path.
async fn handle_reference_captures(
    app_handle: &tauri::AppHandle,
    conversation_id: &str,
    search_metadata: &Arc<Mutex<HashMap<String, BrokerSearchResult>>>,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let opaque_ids: Vec<String> = params
        .get("opaqueIds")
        .and_then(|value| value.as_array())
        .map(|array| {
            array
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let config_dir = access_config_dir(app_handle)?;

    // Snapshot the retained search-metadata map, then drop the guard so the
    // resolver closure does not hold the lock across decode work.
    let snapshot: HashMap<String, BrokerSearchResult> = search_metadata
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default();

    let (mut sources, accepted, dropped) = build_ask_ai_sources(&opaque_ids, |id| {
        // Authoritative frame/audio identity via the signed reference. A failed
        // HMAC validation or unparseable id yields `None` (dropped).
        let reference =
            app_infra::brokered_access::signed_opaque_capture_reference(&config_dir, id)
                .ok()
                .flatten()?;
        // The model may only reference ids it actually received from `search`.
        let result = snapshot.get(id)?;
        let context = result.context.as_ref();
        Some(ResolvedAskAiSource {
            kind: reference.kind,
            frame_id: reference.frame_id,
            audio_segment_id: reference.audio_segment_id,
            app_name: context.and_then(|context| {
                context
                    .app_name
                    .clone()
                    .or_else(|| context.app_bundle_id.clone())
            }),
            window_title: context.and_then(|context| context.window_title.clone()),
            started_at: result.started_at.clone(),
            ended_at: result.ended_at.clone(),
            // Audio Search Result Anchor retained from the search result the
            // model received. Present only for audio results; the search mapper
            // leaves these `None` for frames.
            span_start_ms: result.span_start_ms,
            aligned_frame_id: result.aligned_frame_id,
        })
    });

    // Best-effort enrichment: color each audio source by its real microphone vs
    // system-audio kind from the DB. The pure builder cannot do this (no async DB
    // access), so we patch `sourceKind` here. Capped naturally at the audio cap
    // (≤4 lookups); a missing AppInfra or a single failed lookup just leaves that
    // source's `sourceKind` null and never aborts the emit.
    //
    // The lookups are issued concurrently rather than sequentially: the audio cap
    // bounds them at ≤4, but a per-source await chain serializes them needlessly.
    // We first collect `(index, audio_segment_id)` from an immutable read of
    // `sources`, drive the cloned-`Arc` lookups through `join_all`, then apply the
    // resolved kinds back by index (the only mutable borrow of `sources`).
    if let Some(infra) = app_handle.try_state::<crate::app_infra::AppInfraState>() {
        // Own a cloned `Arc<AppInfra>` so each concurrent lookup future can hold it
        // for the life of its `await` without borrowing the Tauri `State` guard.
        let infra = Arc::clone(&*infra);
        let audio_lookups: Vec<(usize, i64)> = sources
            .iter()
            .enumerate()
            .filter_map(|(index, source)| {
                if source.get("kind").and_then(|kind| kind.as_str()) != Some("audio") {
                    return None;
                }
                let audio_segment_id = source
                    .get("audioSegmentId")
                    .and_then(|value| value.as_i64())?;
                Some((index, audio_segment_id))
            })
            .collect();

        let segments = futures_util::future::join_all(audio_lookups.into_iter().map(
            |(index, audio_segment_id)| {
                let infra = Arc::clone(&infra);
                async move { (index, infra.get_audio_segment(audio_segment_id).await) }
            },
        ))
        .await;

        for (index, lookup) in segments {
            let Ok(Some(segment)) = lookup else {
                continue;
            };
            let source_kind = match segment.source_kind.as_str() {
                "system_audio" => "system",
                // `microphone` (and any unexpected value) colors as microphone.
                _ => "microphone",
            };
            sources[index]["sourceKind"] = serde_json::json!(source_kind);
        }
    }

    let _ = app_handle.emit(
        ASK_AI_SOURCE_EVENT,
        serde_json::json!({
            "conversationId": conversation_id,
            "sources": sources,
        }),
    );

    Ok(serde_json::json!({ "accepted": accepted, "dropped": dropped }))
}

#[tauri::command]
pub async fn get_pi_runtime_status(
    app_handle: tauri::AppHandle,
) -> Result<crate::app_infra::PiRuntimeStatus, String> {
    // This command backs the explicit Settings → "Refresh PI status" action, so force a
    // fresh login-shell PATH read: a user who fixed pi/node (e.g. added a dir to their
    // shell profile) after launch must see it without restarting the app. Hot/background
    // callers use `get_pi_runtime_status_inner` (cached) instead.
    crate::app_infra::get_pi_runtime_status_inner_with_options(app_handle, true).await
}

// NOTE: the brokered data tools (`search`, `timeline`, `show_text`) are NOT
// exposed as Tauri commands. They are reachable only through the PI Ask AI
// session via the in-process `tool_invoker` (see `run_pi_ask_ai_session`), which
// routes every call through `execute_pi_broker_request` AND the per-question
// tool-call cap. Registering them as renderer-callable commands would let any
// webview (or an XSS in Quick Recall) issue All-Retained broker queries directly,
// bypassing both the PI flow and the cap while audit still attributed access to
// PI. Keep them internal.

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskAiStartRequest {
    conversation_id: String,
    question: String,
    seed_query: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskAiFollowupRequest {
    conversation_id: String,
    question: String,
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
///
/// The line surfaces the result's `opaqueId` the same way a tool-call `search`
/// result exposes it to the model (each `search` result JSON carries an
/// `opaqueId` field). Without it, a model answering purely from seeded context —
/// never calling `search` — would have no id to hand to `reference_captures`, so
/// the answer would render zero Answer Source cards. The ids minted by the
/// broker seed search are HMAC-signed identically to tool-call search ids, so a
/// nominated seed id validates through the same `reference_captures` resolver.
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
        "{}. [{} · {}{} · {}–{} · opaqueId={}] {}",
        index + 1,
        result.kind,
        app_label,
        window_segment,
        result.started_at,
        result.ended_at,
        result.opaque_id,
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
invent details. When the captured text you cite already contains a URL, render it as a labeled \
Markdown link `[label](url)` rather than bare text so the user can open it. If you still cannot \
answer, say so briefly. Be concise and direct.\n",
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
    if !status.ready {
        return Ok(AskAiAvailability {
            available: false,
            reason: Some(
                status
                    .reason
                    .unwrap_or_else(|| "pi_unavailable".to_string()),
            ),
        });
    }

    // `ask_ai_start` also needs a resolvable `node` on the shell PATH and the
    // bundled shim resource; a ready PI runtime alone is not enough. Surface
    // these as distinct unavailable reasons so the UI does not advertise Ask AI
    // and then fail at launch.
    if resolve_node_executable().is_err() {
        return Ok(AskAiAvailability {
            available: false,
            reason: Some("node_unavailable".to_string()),
        });
    }
    if resolve_shim_path(&app_handle).is_err() {
        return Ok(AskAiAvailability {
            available: false,
            reason: Some("shim_unavailable".to_string()),
        });
    }

    Ok(AskAiAvailability {
        available: true,
        reason: None,
    })
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

    // Register the cancellable session handle BEFORE the awaitable seeding so a
    // cancel arriving mid-seed (the user dismisses Quick Recall while the broker
    // search is still in flight) is honored: `ask_ai_cancel` finds this handle,
    // sets `cancel`, and removes the entry. Without early registration the cancel
    // would be a no-op against an unregistered conversation, and we would later
    // spawn a resident PI child nobody could stop. The follow-up prompt channel is
    // created here too so the sender can live in the handle; the receiver is moved
    // into the streaming task below.
    let cancel = AskAiCancel::new();
    let (prompt_tx, prompt_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    // `register_ask_ai_session` mints and returns this registration's ownership
    // token (overwriting the placeholder); the spawned task uses it to remove only
    // its own entry on completion.
    let session_token = register_ask_ai_session(
        &conversation_id,
        AskAiSessionHandle {
            cancel: cancel.clone(),
            prompt_tx,
            token: 0,
        },
    );

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

    // If a cancel arrived during seeding it set `cancel` and removed our handle.
    // Honor it now: skip the PI child spawn entirely so no resident process is
    // launched for a conversation the frontend already dropped. Drop our own
    // registry entry too in case the cancel raced just after registration.
    if cancel.is_cancelled() {
        remove_ask_ai_session_if_owner(&conversation_id, session_token);
        return Ok(());
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

    // Resolve the selected Quick Recall model (None => PI default).
    let model = read_ask_ai_model(&app_handle);

    // The cancellable session handle (`cancel` flag + follow-up `prompt_tx`) was
    // registered before seeding so a mid-seed cancel is honored; `prompt_rx` (the
    // receiver) lives in `run_pi_ask_ai_session` below. Removing the registry
    // handle drops the sender, which tears the thread down between turns.

    // Build the brokered tool invoker. The three data tools ride
    // `execute_pi_broker_request`, which enforces Ask-AI access readiness plus
    // the All-Retained broker scope and redaction/audit Rust-side; do not
    // bypass it. The `reference_captures` presentation tool is intercepted
    // before the broker dispatch and never builds a broker request.
    //
    // Search results are recorded into this map keyed by opaque id so a later
    // `reference_captures` call can attach metadata and prove the model only
    // references ids it actually received from `search`.
    let search_metadata: Arc<Mutex<HashMap<String, BrokerSearchResult>>> =
        Arc::new(Mutex::new(HashMap::new()));
    // Seed results are citable too: register them under the same opaque-id keying
    // the tool-call `search` path uses, so a model answering purely from seeded
    // context (never calling `search`) can still nominate them to
    // `reference_captures`. The seed line surfaces each `opaqueId` for exactly
    // this. Their ids are minted by the same broker search path, so they validate
    // identically in the resolver.
    if !seed_results.is_empty() {
        if let Ok(mut map) = search_metadata.lock() {
            for result in &seed_results {
                map.insert(result.opaque_id.clone(), result.clone());
            }
        }
    }
    let invoker_app_handle = app_handle.clone();
    let invoker_conversation_id = conversation_id.clone();
    let invoker_search_metadata = Arc::clone(&search_metadata);
    let tool_invoker: pi_agent_session::AskAiToolInvoker =
        Box::new(move |tool: String, params: serde_json::Value| {
            let app_handle = invoker_app_handle.clone();
            let conversation_id = invoker_conversation_id.clone();
            let search_metadata = Arc::clone(&invoker_search_metadata);
            Box::pin(async move {
                // Presentation signal: validate/decode + emit `ask_ai_source`,
                // never dispatched to the broker.
                if tool == "reference_captures" {
                    return handle_reference_captures(
                        &app_handle,
                        &conversation_id,
                        &search_metadata,
                        params,
                    )
                    .await;
                }

                let request = broker_request_from_tool(&tool, params)?;
                let response = execute_pi_broker_request(app_handle, request).await?;
                // Retain each search result by opaque id for later nomination.
                if let BrokeredCaptureResponse::Search(ref response) = response {
                    if let Ok(mut map) = search_metadata.lock() {
                        for r in &response.results {
                            map.insert(r.opaque_id.clone(), r.clone());
                        }
                    }
                }
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
            model.as_deref(),
            &prompt,
            max_tool_calls,
            prompt_rx,
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
                pi_agent_session::AskAiStreamEvent::ToolCall { tool, params, .. } => {
                    // `reference_captures` is a presentation signal, not a data
                    // activity, so it must not appear in the activity chip.
                    if tool == "reference_captures" {
                        return;
                    }
                    // Forward the raw tool name (`search`/`timeline`/`show_text`)
                    // plus its params; the frontend builds the humane working-line
                    // label from these (e.g. `Searching "invoice" · Jun 1`).
                    let _ = emit_handle.emit(
                        ASK_AI_STATUS_EVENT,
                        serde_json::json!({
                            "conversationId": emit_conversation_id,
                            "phase": "tool",
                            "tool": tool,
                            "params": params,
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

        // Remove only our own registration: if a newer `ask_ai_start` reused this
        // conversation id while we were running, it holds a different token and
        // must survive our teardown.
        remove_ask_ai_session_if_owner(&task_conversation_id, session_token);
    });

    Ok(())
}

/// Enumerate the PI models selectable for Quick Recall.
///
/// Runs the shim in list mode to read the user's PI model registry. Requires a
/// resolvable `node` + Ask AI shim + `pi` runtime; surfaces a string error the
/// frontend can degrade to the "PI default" option only.
#[tauri::command]
pub async fn ask_ai_list_models(
    app_handle: tauri::AppHandle,
) -> Result<Vec<pi_agent_session::AskAiModel>, String> {
    let node_path = resolve_node_executable()?;
    let shim_path = resolve_shim_path(&app_handle)?;
    let status = crate::app_infra::get_pi_runtime_status_inner(app_handle.clone()).await?;
    pi_agent_session::list_pi_models(&node_path, &shim_path, status.executable_path.as_deref())
        .await
}

/// Route a raw follow-up question into the resident PI session for an existing
/// thread. `conversationId` identifies the whole thread/session started by
/// `ask_ai_start`. Unlike start, there is NO seeding and NO `seedQuery`: the
/// resident session already holds turn 1's system instructions plus the prior
/// turns' history, so the raw trimmed question is fed straight in and the
/// answer streams back over the same `ask_ai_status`/`ask_ai_delta`/
/// `ask_ai_source`/`ask_ai_done` events carrying this `conversationId`.
#[tauri::command]
pub async fn ask_ai_followup(
    app_handle: tauri::AppHandle,
    request: AskAiFollowupRequest,
) -> Result<(), String> {
    // Validate access readiness for parity with start.
    ensure_ask_ai_access_ready(&app_handle).await?;

    let AskAiFollowupRequest {
        conversation_id,
        question,
    } = request;

    let question = question.trim().to_string();
    if question.is_empty() {
        return Err("Ask AI follow-up question is empty".to_string());
    }

    // Look up the live session's follow-up sender without removing it; an absent
    // handle means the thread was cancelled or already ended.
    let Some(prompt_tx) = ask_ai_session_prompt_sender(&conversation_id) else {
        return Err("Ask AI conversation is no longer active".to_string());
    };

    // Announce a new turn so the frontend re-enters the thinking phase. No
    // seeding, no broker search — follow-ups send the raw question.
    let _ = app_handle.emit(
        ASK_AI_STATUS_EVENT,
        serde_json::json!({
            "conversationId": conversation_id,
            "phase": "thinking",
        }),
    );

    // Feed the raw trimmed question into the resident session. A send error
    // means the receiver was dropped (the session task ended between the lookup
    // and the send), which is the same dead-thread case.
    prompt_tx
        .send(question)
        .map_err(|_| "Ask AI conversation is no longer active".to_string())?;

    Ok(())
}

#[tauri::command]
pub async fn ask_ai_cancel(
    _app_handle: tauri::AppHandle,
    request: AskAiCancelRequest,
) -> Result<(), String> {
    if let Some(handle) = take_ask_ai_session(&request.conversation_id) {
        handle.cancel.cancel();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use app_infra::brokered_access::{
        BrokerAuthStatusKind, BrokerErrorResponse, BrokerSearchResponse, BrokerSearchResultContext,
        BrokerShowTextResponse,
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

        let error =
            validate_ask_ai_access_ready(true, &status).expect_err("unready PI should be rejected");

        assert_eq!(
            error,
            "Ask AI requires a ready PI runtime (pi_auth_missing)"
        );
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
            span_start_ms: None,
            span_end_ms: None,
            aligned_frame_id: None,
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
            "1. [frame · Xcode · \"ContentView.swift\" · 2026-01-01T10:00:00Z–2026-01-01T10:01:00Z · opaqueId=op-1] build passed"
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
    fn seed_line_surfaces_opaque_id_for_nomination() {
        // The opaque id must appear in the seed line so a model answering from
        // seeded context alone can still nominate it to `reference_captures`.
        let line = format_seed_result_line(0, &sample_result());
        assert!(line.contains("opaqueId=op-1"));
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

    fn frame_source(started_at: &str, ended_at: &str) -> ResolvedAskAiSource {
        ResolvedAskAiSource {
            kind: "frame".to_string(),
            frame_id: Some(42),
            audio_segment_id: None,
            app_name: Some("Xcode".to_string()),
            window_title: Some("ContentView.swift".to_string()),
            started_at: started_at.to_string(),
            ended_at: ended_at.to_string(),
            span_start_ms: None,
            aligned_frame_id: None,
        }
    }

    fn audio_source(started_at: &str, ended_at: &str) -> ResolvedAskAiSource {
        ResolvedAskAiSource {
            kind: "audio".to_string(),
            frame_id: None,
            audio_segment_id: Some(7),
            app_name: Some("Zoom".to_string()),
            window_title: None,
            started_at: started_at.to_string(),
            ended_at: ended_at.to_string(),
            // Audio Search Result Anchor: a mid-segment match span + aligned
            // frame, as a real audio search result would carry.
            span_start_ms: Some(3_000),
            aligned_frame_id: Some(99),
        }
    }

    #[test]
    fn build_ask_ai_sources_caps_frames_and_audio() {
        let mut ids: Vec<String> = (0..8).map(|i| format!("frame-{i}")).collect();
        ids.extend((0..6).map(|i| format!("audio-{i}")));

        let (sources, accepted, dropped) = build_ask_ai_sources(&ids, |id| {
            if id.starts_with("frame-") {
                Some(frame_source("2026-01-01T10:00:00Z", "2026-01-01T10:01:00Z"))
            } else {
                Some(audio_source("2026-01-01T11:00:00Z", "2026-01-01T11:05:00Z"))
            }
        });

        assert_eq!(accepted, 10);
        assert_eq!(dropped, ids.len() - 10);
        let frame_total = sources
            .iter()
            .filter(|s| s["kind"] == serde_json::json!("frame"))
            .count();
        let audio_total = sources
            .iter()
            .filter(|s| s["kind"] == serde_json::json!("audio"))
            .count();
        assert_eq!(frame_total, 6);
        assert_eq!(audio_total, 4);
        // Frames are nominated first, so they appear before audio in order.
        assert_eq!(sources[0]["kind"], serde_json::json!("frame"));
        assert_eq!(sources[6]["kind"], serde_json::json!("audio"));
    }

    #[test]
    fn build_ask_ai_sources_drops_invalid_ids() {
        let ids = vec![
            "good-1".to_string(),
            "bad-1".to_string(),
            "good-2".to_string(),
            "bad-2".to_string(),
        ];

        let (sources, accepted, dropped) = build_ask_ai_sources(&ids, |id| {
            if id.starts_with("good-") {
                Some(frame_source("2026-01-01T10:00:00Z", "2026-01-01T10:01:00Z"))
            } else {
                None
            }
        });

        assert_eq!(accepted, 2);
        assert_eq!(dropped, 2);
        assert_eq!(sources.len(), 2);
    }

    #[test]
    fn build_ask_ai_sources_dedupes() {
        let ids = vec![
            "dup".to_string(),
            "dup".to_string(),
            "other".to_string(),
            "dup".to_string(),
        ];

        let (sources, accepted, dropped) = build_ask_ai_sources(&ids, |_id| {
            Some(frame_source("2026-01-01T10:00:00Z", "2026-01-01T10:01:00Z"))
        });

        // Two distinct ids resolve once each; the two extra `dup` repeats drop.
        assert_eq!(accepted, 2);
        assert_eq!(dropped, 2);
        assert_eq!(sources.len(), 2);
    }

    #[test]
    fn build_ask_ai_sources_source_shape() {
        let ids = vec!["frame-1".to_string(), "audio-1".to_string()];
        let (sources, _accepted, _dropped) = build_ask_ai_sources(&ids, |id| {
            if id.starts_with("frame-") {
                Some(frame_source("2026-01-01T10:00:00Z", "2026-01-01T10:01:00Z"))
            } else {
                Some(audio_source("2026-01-01T11:00:00Z", "2026-01-01T11:05:00Z"))
            }
        });

        let frame = &sources[0];
        assert_eq!(frame["kind"], serde_json::json!("frame"));
        assert_eq!(frame["frameId"], serde_json::json!(42));
        assert_eq!(frame["audioSegmentId"], serde_json::Value::Null);
        assert_eq!(frame["appName"], serde_json::json!("Xcode"));
        assert_eq!(frame["windowTitle"], serde_json::json!("ContentView.swift"));
        assert_eq!(
            frame["startedAt"],
            serde_json::json!("2026-01-01T10:00:00Z")
        );
        assert_eq!(frame["endedAt"], serde_json::json!("2026-01-01T10:01:00Z"));
        // The pure builder never resolves the mic/system distinction; that is the
        // async post-pass's job, so every source starts with a null `sourceKind`.
        assert!(frame.as_object().unwrap().contains_key("sourceKind"));
        assert_eq!(frame["sourceKind"], serde_json::Value::Null);
        // Frame sources carry no Audio Search Result Anchor.
        assert_eq!(frame["spanStartMs"], serde_json::Value::Null);
        assert_eq!(frame["alignedFrameId"], serde_json::Value::Null);

        let audio = &sources[1];
        assert_eq!(audio["kind"], serde_json::json!("audio"));
        assert_eq!(audio["frameId"], serde_json::Value::Null);
        assert_eq!(audio["audioSegmentId"], serde_json::json!(7));
        assert_eq!(audio["windowTitle"], serde_json::Value::Null);
        assert!(audio.as_object().unwrap().contains_key("sourceKind"));
        assert_eq!(audio["sourceKind"], serde_json::Value::Null);
        // Audio sources carry the anchor so the dashboard lands mid-segment.
        assert_eq!(audio["spanStartMs"], serde_json::json!(3_000));
        assert_eq!(audio["alignedFrameId"], serde_json::json!(99));
    }

    #[test]
    fn broker_request_from_tool_rejects_reference_captures() {
        let error =
            broker_request_from_tool("reference_captures", serde_json::json!({ "opaqueIds": [] }))
                .expect_err("reference_captures is not a broker data tool");
        assert_eq!(error, "unknown Ask AI tool: reference_captures");
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
    fn followup_request_deserializes_camel_case_without_seed_query() {
        let request: AskAiFollowupRequest = serde_json::from_str(
            r#"{"conversationId":"conv-1","question":"what about in Slack?"}"#,
        )
        .expect("follow-up request should deserialize");
        assert_eq!(request.conversation_id, "conv-1");
        assert_eq!(request.question, "what about in Slack?");

        // A follow-up carries no seedQuery: an extra field is ignored, not
        // required, and the struct exposes only conversation_id + question.
        let request: AskAiFollowupRequest = serde_json::from_str(
            r#"{"conversationId":"conv-2","question":"more","seedQuery":"ignored"}"#,
        )
        .expect("extra fields are ignored");
        assert_eq!(request.conversation_id, "conv-2");
        assert_eq!(request.question, "more");
    }

    fn test_session_handle() -> AskAiSessionHandle {
        let (prompt_tx, _prompt_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        AskAiSessionHandle {
            cancel: AskAiCancel::new(),
            prompt_tx,
            // `register_ask_ai_session` overwrites this with the minted token.
            token: 0,
        }
    }

    #[test]
    fn prompt_sender_is_none_for_unknown_conversation() {
        assert!(ask_ai_session_prompt_sender("missing-conv-xyz").is_none());
    }

    #[test]
    fn prompt_sender_present_after_register_then_gone_after_take() {
        let id = "registry-roundtrip-conv";
        // Keep the receiver alive so the cloned sender stays usable for the test.
        let (prompt_tx, _prompt_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        register_ask_ai_session(
            id,
            AskAiSessionHandle {
                cancel: AskAiCancel::new(),
                prompt_tx,
                token: 0,
            },
        );

        let sender = ask_ai_session_prompt_sender(id).expect("registered session should resolve");
        // The cloned sender routes into the live receiver.
        assert!(sender.send("hi".to_string()).is_ok());

        // Taking the session removes it, so the clone-helper no longer finds it.
        assert!(take_ask_ai_session(id).is_some());
        assert!(ask_ai_session_prompt_sender(id).is_none());
    }

    #[test]
    fn remove_session_clears_prompt_sender() {
        let id = "remove-clears-conv";
        register_ask_ai_session(id, test_session_handle());
        assert!(ask_ai_session_prompt_sender(id).is_some());
        remove_ask_ai_session(id);
        assert!(ask_ai_session_prompt_sender(id).is_none());
    }

    #[test]
    fn remove_if_owner_spares_session_that_reused_the_id() {
        let id = "owner-token-conv";

        // First registration; capture the token register mints for it. Keep the
        // receiver alive so the registered sender stays usable.
        let (first_tx, _first_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let first_token = register_ask_ai_session(
            id,
            AskAiSessionHandle {
                cancel: AskAiCancel::new(),
                prompt_tx: first_tx,
                token: 0,
            },
        );

        // A second `ask_ai_start` reuses the id, overwriting the entry with a new
        // token. Keep the receiver alive so the registered sender stays usable.
        let (second_tx, _second_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let second_token = register_ask_ai_session(
            id,
            AskAiSessionHandle {
                cancel: AskAiCancel::new(),
                prompt_tx: second_tx,
                token: 0,
            },
        );
        assert_ne!(first_token, second_token);

        // The first session finishing must NOT evict the newer registration.
        remove_ask_ai_session_if_owner(id, first_token);
        assert!(
            ask_ai_session_prompt_sender(id).is_some(),
            "stale owner removal should leave the newer session registered"
        );

        // The owning (newer) session's removal clears it.
        remove_ask_ai_session_if_owner(id, second_token);
        assert!(ask_ai_session_prompt_sender(id).is_none());
    }

    #[test]
    fn register_cancels_the_displaced_session_handle() {
        let id = "displaced-cancel-conv";

        // First registration: capture its cancel handle so we can observe whether
        // a racing second start hard-cancels it. Keep the receiver alive so the
        // first handle's sender stays valid until it is displaced.
        let first_cancel = AskAiCancel::new();
        let (first_tx, _first_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        register_ask_ai_session(
            id,
            AskAiSessionHandle {
                cancel: first_cancel.clone(),
                prompt_tx: first_tx,
                token: 0,
            },
        );
        assert!(!first_cancel.is_cancelled());

        // A second `ask_ai_start` reuses the id. Replacing the entry must set the
        // displaced handle's cancel flag so a mid-turn streamer (blocked reading
        // shim stdout, unreachable by dropping `prompt_tx` alone) is killed and
        // stops interleaving output under the now-newer conversation id.
        let (second_tx, _second_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        register_ask_ai_session(
            id,
            AskAiSessionHandle {
                cancel: AskAiCancel::new(),
                prompt_tx: second_tx,
                token: 0,
            },
        );

        assert!(
            first_cancel.is_cancelled(),
            "displacing a session must cancel the prior handle, not only drop its prompt_tx"
        );

        // Clean up the registry so the static map does not leak into other tests.
        let _ = take_ask_ai_session(id);
    }

    #[test]
    fn broker_response_to_tool_value_error_returns_message() {
        let response = BrokeredCaptureResponse::Error(BrokerErrorResponse {
            error: BrokerAuthStatusKind::AuthorizationRequired,
            message: "result is unavailable or outside the grant scope".to_string(),
        });

        let error =
            broker_response_to_tool_value(response).expect_err("error envelope should become Err");
        assert_eq!(error, "result is unavailable or outside the grant scope");
    }
}
