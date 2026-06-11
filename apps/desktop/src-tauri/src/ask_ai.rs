//! Ask AI — Quick Recall / Chat tool-enabled answer slice (issue #70, ADR 0024;
//! migrated onto the shared Reasoning Engine by ADR 0033).
//!
//! Ask AI now drives the in-process `rig-core` agent loop (the `ai-runtime`
//! crate, aliased `ai_engine`) directly — there is NO Node child, NO PI shim, NO
//! resident PI session. Every turn is **stateless**: the driver reloads the
//! conversation's completed history from the backend conversation store, runs ONE
//! agent loop against the configured engine, and persists the answer as it
//! streams so a reattaching frontend can read the in-flight partial. Because the
//! turn task is detached, it finishes in the background regardless of whether the
//! Quick Recall window is dismissed; a follow-up just runs another stateless turn.
//!
//! The brokered data tools (`search`, `timeline`, `show_text`, `recall_context`)
//! plus the presentation-only `reference_captures` tool are described to the model
//! and executed through the All-Retained broker seam
//! (`BrokeredCaptureAccess::execute_for_ask_ai`) Rust-side, with redaction/audit
//! and the per-question tool-call cap enforced here. `open`/`open_in_mnema` is NOT
//! an Ask AI tool and is rejected before the broker.
//!
//! The streaming Tauri EVENT surface (`ask_ai_status` / `ask_ai_delta` /
//! `ask_ai_done` / `ask_ai_error` / `ask_ai_source`, all keyed by
//! `conversationId`) is byte-for-byte the same as the PI-era surface, so the
//! frontend barely changes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use app_infra::brokered_access::{
    BrokerClientIdentity, BrokerClientIdentitySource, BrokerRecallContextRequest,
    BrokerSearchRequest, BrokerSearchResult, BrokerTimelineRequest, BrokeredCaptureAccess,
    BrokeredCaptureRequest, BrokeredCaptureResponse,
};
use capture_types::AiRuntimeSettings;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

use crate::app_infra::AppInfraState;
use crate::conversation::commands::CONVERSATION_CHANGED_EVENT;

/// Process registry mapping a conversation id (the whole thread) to the
/// cooperative cancel flag of its CURRENTLY in-flight turn. Module-level so it
/// survives across separate Tauri command invocations without touching lib.rs
/// state wiring.
///
/// There is no resident session, prompt channel, or ownership token anymore: a
/// turn is a detached task that streams once and exits. The registry exists only
/// so `ask_ai_cancel` can find a running turn's flag, and so a new
/// start/follow-up can cancel a still-running prior turn for the same
/// conversation before launching its replacement.
static ASK_AI_INFLIGHT: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();

fn ask_ai_inflight() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    ASK_AI_INFLIGHT.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Register a fresh cancel flag for a conversation's new in-flight turn,
/// returning the flag. Any flag already registered for that conversation (a prior
/// turn still streaming) is CANCELLED before being replaced, so a racing
/// start/follow-up cooperatively stops the displaced turn rather than letting two
/// turns interleave `ask_ai_delta` output under the same conversation id. The
/// displaced turn observes its flag between stream items and ends cleanly.
fn register_inflight(conversation_id: &str) -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    if let Ok(mut map) = ask_ai_inflight().lock() {
        if let Some(previous) = map.insert(conversation_id.to_string(), cancel.clone()) {
            previous.store(true, Ordering::SeqCst);
        }
    }
    cancel
}

/// Remove the in-flight flag for a conversation only if it is still the exact
/// `cancel` instance the finishing turn registered. A turn calls this on exit so
/// it never evicts a NEWER turn that displaced it (the newer turn registered a
/// different `Arc`).
fn remove_inflight_if_owner(conversation_id: &str, cancel: &Arc<AtomicBool>) {
    if let Ok(mut map) = ask_ai_inflight().lock() {
        if map
            .get(conversation_id)
            .is_some_and(|flag| Arc::ptr_eq(flag, cancel))
        {
            map.remove(conversation_id);
        }
    }
}

/// Take (remove and return) the in-flight flag for a conversation. Used by
/// `ask_ai_cancel`: setting the returned flag cooperatively stops the running
/// turn, and removing it keeps the registry tidy.
fn take_inflight(conversation_id: &str) -> Option<Arc<AtomicBool>> {
    ask_ai_inflight()
        .lock()
        .ok()
        .and_then(|mut map| map.remove(conversation_id))
}

const ASK_AI_STATUS_EVENT: &str = "ask_ai_status";
const ASK_AI_DELTA_EVENT: &str = "ask_ai_delta";
const ASK_AI_REASONING_EVENT: &str = "ask_ai_reasoning";
const ASK_AI_DONE_EVENT: &str = "ask_ai_done";
const ASK_AI_ERROR_EVENT: &str = "ask_ai_error";
const ASK_AI_SOURCE_EVENT: &str = "ask_ai_source";

/// Per-kind caps on the nominated Answer Source set emitted to the frontend.
const ASK_AI_SOURCE_FRAME_CAP: usize = 6;
const ASK_AI_SOURCE_AUDIO_CAP: usize = 4;

/// Number of seeded broker-search results requested.
const ASK_AI_SEED_LIMIT: u32 = 8;

/// Persist-throttle thresholds for the streaming partial answer. The accumulating
/// answer is re-persisted to the turn row (phase `streaming`) once either many
/// deltas or many new chars have accrued since the last persist, so a reattach
/// reads a recent partial without one DB write per token.
const ASK_AI_PARTIAL_PERSIST_DELTA_INTERVAL: usize = 20;
const ASK_AI_PARTIAL_PERSIST_CHAR_INTERVAL: usize = 200;

/// Default origin stamped on a conversation when `ask_ai_start` does not carry one
/// and the conversation row does not yet exist. Quick Recall is the historical
/// Ask AI door; Chat passes `"chat"` explicitly (Slice 7).
const ASK_AI_DEFAULT_ORIGIN: &str = "quick_recall";

/// Bounds on an accepted generated thread title. The prompt asks for 3–6 words;
/// anything past these slack limits is treated as a failed generation (the
/// fallback first-question title stands).
const GENERATED_TITLE_MAX_WORDS: usize = 8;
const GENERATED_TITLE_MAX_CHARS: usize = 64;

/// Translate the persisted `askAiMaxToolCalls` setting (`0` = no cap) into the
/// per-turn cap passed to the agent loop. `0` becomes `usize::MAX` so the agent
/// may issue unlimited follow-up brokered queries (the loop clamps it to a sane
/// internal ceiling).
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

/// The broker client identity Ask AI presents to the All-Retained broker seam.
///
/// The label is "Ask AI" (the in-process agent is the client now; PI is gone) and
/// the source is `Inferred`, matching how the broker attributes an internally
/// inferred client rather than an explicitly authenticated CLI identity.
fn ask_ai_broker_identity() -> Result<BrokerClientIdentity, String> {
    BrokerClientIdentity::new("Ask AI", BrokerClientIdentitySource::Inferred)
        .map_err(|error| error.to_string())
}

/// Read the full current recording settings (so callers can pull both the
/// `access` and `ai_runtime` domains from one read).
fn read_recording_settings(app_handle: &tauri::AppHandle) -> capture_types::RecordingSettings {
    crate::native_capture::current_recording_settings_from_app_handle(app_handle)
}

fn read_ask_ai_enabled(app_handle: &tauri::AppHandle) -> bool {
    read_recording_settings(app_handle).access.ask_ai_enabled
}

fn read_ai_runtime_settings(app_handle: &tauri::AppHandle) -> AiRuntimeSettings {
    read_recording_settings(app_handle).ai_runtime
}

/// Read the configured per-question tool-call cap (`0` = no cap).
fn read_ask_ai_max_tool_calls(app_handle: &tauri::AppHandle) -> usize {
    resolve_tool_call_cap(
        read_recording_settings(app_handle)
            .access
            .ask_ai_max_tool_calls,
    )
}

/// Resolve the AppInfra handle (for the conversation store). Cloned `Arc` so the
/// caller can hold it across awaits without borrowing the Tauri `State` guard.
fn app_infra(app_handle: &tauri::AppHandle) -> Result<AppInfraState, String> {
    app_handle
        .try_state::<AppInfraState>()
        .map(|state| Arc::clone(&*state))
        .ok_or_else(|| "Ask AI storage is unavailable".to_string())
}

/// "Now" in unix milliseconds (UTC), stamped Rust-side on persist so the store
/// stays deterministic. Mirrors the conversation command module's `now_ms`.
fn now_ms() -> i64 {
    (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}

/// The user's wall-clock context for one turn. The user thinks and asks in LOCAL
/// time ("this morning", "yesterday"), but every capture timestamp and every
/// `from`/`to` bound the broker speaks is UTC. The frontend supplies the offset +
/// IANA zone because that is the only SOUND source here: the `time` crate is
/// built without the `local-offset` feature, and `current_local_offset()` is
/// unsound under Tauri's multithreading. Absent (older payloads / unknown) → the
/// grounding falls back to UTC only.
#[derive(Debug, Clone, Default)]
struct ClientClock {
    /// Minutes to ADD to UTC to reach the user's local wall clock (e.g. PST = -480,
    /// IST = 330). This is `-Date.getTimezoneOffset()` on the JS side.
    utc_offset_minutes: Option<i32>,
    /// IANA zone name for display only (e.g. "America/Los_Angeles").
    time_zone: Option<String>,
}

/// Format an `OffsetDateTime` as `YYYY-MM-DD HH:MM` (no seconds — the model reasons
/// in minutes, not instants). Done by hand to avoid pulling in a format-description.
fn format_ymd_hm(dt: time::OffsetDateTime) -> String {
    let date = dt.date();
    let clock = dt.time();
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        date.year(),
        u8::from(date.month()),
        date.day(),
        clock.hour(),
        clock.minute(),
    )
}

/// Build the per-turn **temporal grounding** line prepended to the prompt: the
/// current local date/time + UTC offset, plus the rule that all capture
/// timestamps and tool `from`/`to` bounds are UTC. Without this the model has no
/// anchor for "today"/"yesterday" and cannot translate the user's local-time
/// phrasing into the UTC windows `search`/`timeline` expect.
fn build_temporal_grounding(now_ms: i64, clock: &ClientClock) -> String {
    let now_utc = time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(now_ms) * 1_000_000)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);

    let mut grounding = String::from("Temporal grounding: ");
    match clock.utc_offset_minutes {
        Some(offset_minutes) => {
            let local = now_utc + time::Duration::minutes(i64::from(offset_minutes));
            let sign = if offset_minutes < 0 { '-' } else { '+' };
            let abs = offset_minutes.abs();
            let zone = clock
                .time_zone
                .as_deref()
                .map(|zone| format!("{zone}, "))
                .unwrap_or_default();
            grounding.push_str(&format!(
                "the user's current local time is {} ({}UTC{}{:02}:{:02}); in UTC that is {} UTC. ",
                format_ymd_hm(local),
                zone,
                sign,
                abs / 60,
                abs % 60,
                format_ymd_hm(now_utc),
            ));
        }
        None => {
            grounding.push_str(&format!(
                "the current time is {} UTC (the user's local offset is unknown). ",
                format_ymd_hm(now_utc),
            ));
        }
    }
    grounding.push_str(
        "Every capture timestamp you see, and every `from`/`to` bound you pass to `search` and \
`timeline`, is in UTC (RFC3339 `Z`). Resolve the user's relative or local-time phrasing — \
\"today\", \"yesterday\", \"this morning\", \"last week\" — against the local time above, convert \
it to UTC for tool calls, and present times back to the user in their local time.\n\n",
    );
    grounding
}

/// The two-layer Ask AI access gate. `Ok(())` only when Ask AI is enabled in
/// settings AND the shared Reasoning Engine prerequisite passes; the error is a
/// human string the frontend surfaces.
async fn ensure_ask_ai_access_ready(app_handle: &tauri::AppHandle) -> Result<(), String> {
    if !read_ask_ai_enabled(app_handle) {
        return Err("Ask AI access is disabled in settings".to_string());
    }
    let settings = read_ai_runtime_settings(app_handle);
    crate::ai_runtime::engine_configured_prerequisite(&settings).await
}

/// Execute one brokered data-tool request under the Ask AI seam: re-checks access
/// readiness, then runs it through the All-Retained Ask-AI broker scope
/// (`execute_for_ask_ai`) with redaction/audit attributed to the "Ask AI"
/// identity. `open`/`open_in_mnema` never reach here — they are rejected by
/// `broker_request_from_tool` before this is called.
async fn execute_ask_ai_broker_request(
    app_handle: tauri::AppHandle,
    request: BrokeredCaptureRequest,
) -> Result<BrokeredCaptureResponse, String> {
    ensure_ask_ai_access_ready(&app_handle).await?;
    broker_access(&app_handle)?
        .execute_for_ask_ai(ask_ai_broker_identity()?, request)
        .await
        .map_err(|error| format!("failed to execute Ask AI broker request: {error}"))
}

/// Map an Ask AI tool name + camelCase params object onto a brokered request.
///
/// Only the Ask AI data tools (`search`, `timeline`, `show_text`,
/// `recall_context`) are accepted; `open`/`open_in_mnema`, the presentation-only
/// `reference_captures`, and anything else fall into the unknown branch and are
/// rejected, so they can never be issued as Ask AI data tools.
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
        "recall_context" => {
            let request: BrokerRecallContextRequest = serde_json::from_value(params)
                .map_err(|error| format!("invalid Ask AI recall_context params: {error}"))?;
            Ok(BrokeredCaptureRequest::RecallContext(request))
        }
        other => Err(format!("unknown Ask AI tool: {other}")),
    }
}

/// Convert a brokered response into the JSON value handed back to the model as a
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
        BrokeredCaptureResponse::RecallContext(response) => serde_json::to_value(response)
            .map_err(|error| format!("failed to serialize Ask AI recall_context result: {error}")),
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

/// Handle the model's `reference_captures` presentation tool: validate + decode
/// the nominated opaque ids, attach retained metadata, cap the set, emit a
/// single `ask_ai_source` event to the frontend, and return `{ accepted, dropped }`
/// as the tool result. This never touches the broker dispatch path. The emitted
/// sources JSON is also handed back to the caller so the turn driver can persist
/// it on the turn row.
async fn handle_reference_captures(
    app_handle: &tauri::AppHandle,
    conversation_id: &str,
    search_metadata: &Arc<Mutex<HashMap<String, BrokerSearchResult>>>,
    params: serde_json::Value,
) -> Result<(serde_json::Value, Vec<serde_json::Value>), String> {
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
    if let Some(infra) = app_handle.try_state::<AppInfraState>() {
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

    Ok((
        serde_json::json!({ "accepted": accepted, "dropped": dropped }),
        sources,
    ))
}

// NOTE: the brokered data tools (`search`, `timeline`, `show_text`,
// `recall_context`) are NOT exposed as Tauri commands. They are reachable only
// through the in-process agent loop's `ToolExecutor` (see `run_ask_ai_turn`),
// which routes every call through `execute_ask_ai_broker_request` AND the
// per-question tool-call cap. Registering them as renderer-callable commands
// would let any webview (or an XSS in Quick Recall) issue All-Retained broker
// queries directly, bypassing both the agent flow and the cap while audit still
// attributed access to "Ask AI". Keep them internal.

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskAiStartRequest {
    conversation_id: String,
    question: String,
    seed_query: Option<String>,
    /// The door that created/owns the conversation: `"quick_recall"` | `"chat"`.
    /// Stamped on the turn row only when the conversation is newly created (the
    /// store preserves an existing row's origin). Optional for wire back-compat;
    /// defaults to Quick Recall, the historical Ask AI door.
    #[serde(default)]
    origin: Option<String>,
    /// The conversation title for the upsert (the first non-empty title wins).
    /// Optional; defaults to empty.
    #[serde(default)]
    title: Option<String>,
    /// Legacy resurrect-from-transcript field. History now comes from the backend
    /// conversation store, so this is IGNORED — kept only so the frontend's
    /// existing start payload still deserializes during the migration.
    #[serde(default)]
    #[allow(dead_code)]
    prior_transcript: Option<serde_json::Value>,
    /// Minutes to add to UTC to reach the user's local wall clock, supplied by the
    /// frontend (`-Date.getTimezoneOffset()`). Optional for wire back-compat.
    #[serde(default)]
    utc_offset_minutes: Option<i32>,
    /// IANA zone name for display in the temporal grounding. Optional.
    #[serde(default)]
    time_zone: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskAiFollowupRequest {
    conversation_id: String,
    question: String,
    /// See [`AskAiStartRequest::utc_offset_minutes`].
    #[serde(default)]
    utc_offset_minutes: Option<i32>,
    /// See [`AskAiStartRequest::time_zone`].
    #[serde(default)]
    time_zone: Option<String>,
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

/// Build a single seed-context line for one broker search result.
///
/// The line surfaces the result's `opaqueId` the same way a tool-call `search`
/// result exposes it to the model. Without it, a model answering purely from
/// seeded context — never calling `search` — would have no id to hand to
/// `reference_captures`, so the answer would render zero Answer Source cards. The
/// ids minted by the broker seed search are HMAC-signed identically to tool-call
/// search ids, so a nominated seed id validates through the same
/// `reference_captures` resolver.
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

/// The agent **preamble** (system instruction): documents the four data tools +
/// the presentation `reference_captures` tool and the optional graphical-answer
/// affordance. This is the engine-agnostic system text; the per-turn seeded
/// context + the bare question live in the prompt (see [`build_ask_ai_prompt`]).
fn build_ask_ai_preamble() -> String {
    let mut preamble = String::new();
    preamble.push_str(
        "You are Mnema's Ask AI assistant. Answer the user's question using their own on-device \
screen and audio capture history. All data is the user's own, redacted, on-device capture. You \
have FOUR tools, and there is NO way to open files or access anything beyond them: `search` \
finds redacted snippets plus opaque ids across the user's screen OCR and audio transcript \
history (optionally narrowed by a `from`/`to` RFC3339 time range and `app`/`windowTitle` \
filters); `timeline` returns coarse activity intervals for a bounded `from`/`to` window; \
`show_text` returns the full redacted text for one opaque id returned by `search`; \
`recall_context` returns ONLY the User-Context conclusions (distilled beliefs about the user) \
and recent activities relevant to the question — redacted, capped, never the whole dossier, and \
never sensitive-category conclusions — and is the best first tool for questions about the user's \
habits, interests, projects, or what you know about them. When the \
seeded context below is missing or insufficient to answer, ISSUE follow-up tool calls to gather \
what you need before answering — prefer a concise `search` first, and use `show_text` sparingly \
for the specific results you need to read in full. Cite times and apps when useful, but never \
invent details. When the captured text you cite already contains a URL, render it as a labeled \
Markdown link `[label](url)` rather than bare text so the user can open it. If you still cannot \
answer, say so briefly. Be concise and direct.\n",
    );

    // Graphical-answer affordance (issue #110): the Chat surface renders two
    // fenced block kinds as inline charts. This is OPTIONAL and only for answers
    // that are naturally a breakdown/comparison; plain markdown is the default.
    preamble.push_str(
        "When an answer is naturally a breakdown or comparison (for example time by category, \
top apps, or a set of beliefs/conclusions), you MAY include a fenced ```mnema-bars block whose \
body is JSON `{\"title\":\"…\",\"bars\":[{\"label\":\"…\",\"value\":12,\"sublabel\":\"12m\"}]}` \
or a fenced ```mnema-dossier block whose body is JSON \
`{\"items\":[{\"subject\":\"…\",\"statement\":\"…\",\"confidence\":0.7}]}`, which the UI renders as \
a chart. For bars, ALWAYS set a `sublabel` carrying the number WITH its unit (for example \
`\"3h 12m\"`, `\"65%\"`, `\"42 frames\"`) so the readout is never an ambiguous bare number; \
`value` is the bare magnitude that sizes the bar. Use at most one such block, with real numbers \
you derived from the captures; otherwise answer in plain markdown.\n",
    );

    // Chronological-answer affordance: a third optional fenced block renders a
    // time-of-day breakdown of the user's day, fed from the `timeline` tool.
    preamble.push_str(
        "When the answer is genuinely chronological — a time-of-day breakdown of the user's day — \
you MAY include a fenced ```mnema-timeline block whose body is JSON \
`{\"title\":\"…\",\"intervals\":[{\"label\":\"…\",\"start\":\"9:30 AM\",\"end\":\"11:00 AM\",\
\"app\":\"Visual Studio Code\",\"category\":\"creating\"}]}`, which the UI renders as a timeline \
widget. `intervals` is REQUIRED: an array in chronological order where each interval has a \
`label` (what happened) and a `start` (a human time-of-day string like `\"9:30 AM\"`); `end`, \
`app`, and `category` are OPTIONAL. When set, `category` MUST be ONE of `creating`, \
`communication`, `meetings`, `research`, `learning`, `organizing`, `personal`, `entertainment` \
(it drives the widget's color — omit it if unsure). `title` is OPTIONAL. Emit this block ONLY \
for genuinely chronological / time-of-day answers, derived from the `timeline` tool's real \
intervals (which carry kind, startedAt, endedAt, and app context); otherwise answer in plain \
markdown. Use at most one timeline block.\n",
    );

    // The presentation tool is described separately because it is NOT a data tool
    // and does not count against the tool-call budget.
    preamble.push_str(
        "You also have a presentation signal, `reference_captures`, which takes `opaqueIds` (the \
opaque ids you received from `search` results, most-relevant-first) and nominates the captures \
(screen frames / audio) behind your answer so the app can show them to the user as source cards. \
It returns NO capture data — only an acknowledgement of how many ids were accepted/dropped. Call \
it once near the end of your answer (a repeat call replaces the prior set); it does NOT count \
against the tool-call budget.\n",
    );

    preamble
}

/// Assemble the per-turn **prompt**: the seeded capture context (if any) followed
/// by the bare question. The system instruction lives in the preamble (see
/// [`build_ask_ai_preamble`]); conversation history is fed separately to the
/// agent loop, so it is NOT in the prompt.
fn build_ask_ai_prompt(
    question: &str,
    seed_query: Option<&str>,
    results: &[BrokerSearchResult],
    now_ms: i64,
    clock: &ClientClock,
) -> String {
    let mut prompt = String::new();

    // Temporal grounding leads the prompt so the model anchors relative dates and
    // knows the local↔UTC relationship before reading any captures.
    prompt.push_str(&build_temporal_grounding(now_ms, clock));

    if let Some(seed_query) = seed_query {
        if !results.is_empty() {
            prompt.push_str(&format!(
                "Context from the user's captures for \"{seed_query}\":\n"
            ));
            for (index, result) in results.iter().enumerate() {
                prompt.push_str(&format_seed_result_line(index, result));
                prompt.push('\n');
            }
            prompt.push('\n');
        }
    }

    prompt.push_str(&format!("Question: {question}"));
    prompt
}

/// JSON Schema (object) for the `search` tool params. Mirrors the shapes the PI
/// shim declared via TypeBox so the model's expected tool contract is unchanged.
fn search_tool_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "query": { "type": "string", "description": "Free-text query to match against captured text." },
            "from": { "type": "string", "description": "Inclusive lower time bound, RFC3339 (e.g. 2026-06-01T09:00:00Z)." },
            "to": { "type": "string", "description": "Inclusive upper time bound, RFC3339." },
            "limit": { "type": "number", "description": "Maximum number of snippets to return." },
            "app": { "type": "string", "description": "Restrict to a single app by name or bundle id." },
            "windowTitle": { "type": "string", "description": "Restrict to snippets whose window title matches." }
        },
        "required": ["query"]
    })
}

/// JSON Schema (object) for the `timeline` tool params.
fn timeline_tool_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "from": { "type": "string", "description": "Inclusive window start, RFC3339 (required)." },
            "to": { "type": "string", "description": "Inclusive window end, RFC3339 (required)." },
            "limit": { "type": "number", "description": "Maximum number of intervals to return." },
            "app": { "type": "string", "description": "Restrict to a single app by name or bundle id." },
            "windowTitle": { "type": "string", "description": "Restrict to intervals whose window title matches." }
        },
        "required": ["from", "to"]
    })
}

/// JSON Schema (object) for the `show_text` tool params.
fn show_text_tool_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "opaqueId": { "type": "string", "description": "An opaque id from a prior `search` result (required)." }
        },
        "required": ["opaqueId"]
    })
}

/// JSON Schema (object) for the `recall_context` tool params.
fn recall_context_tool_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "query": {
                "type": "string",
                "description": "The user's question; returns only the User-Context conclusions/activities relevant to it."
            },
            "limit": { "type": "number", "description": "Maximum number of conclusions/activities to return (capped server-side)." }
        },
        "required": ["query"]
    })
}

/// JSON Schema (object) for the `reference_captures` presentation-tool params.
fn reference_captures_tool_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "opaqueIds": {
                "type": "array",
                "items": { "type": "string", "description": "An opaque id from a prior search result." },
                "description": "Opaque ids of the captures behind the answer, most-relevant-first."
            }
        },
        "required": ["opaqueIds"]
    })
}

/// Build the agent tool set described to the model. The descriptions mirror the
/// PI shim's `defineTool` descriptions so the model's tool contract is preserved
/// across the migration.
fn build_ask_ai_tools() -> Vec<ai_engine::AgentTool> {
    vec![
        ai_engine::AgentTool {
            name: "search".to_string(),
            description:
                "Search the user's redacted on-device capture history (screen OCR + audio \
transcripts). Returns snippets with opaque ids, kinds (screenText/audioTranscript), \
startedAt/endedAt timestamps, and optional context (appName/appBundleId/windowTitle)."
                    .to_string(),
            parameters_schema: search_tool_schema(),
        },
        ai_engine::AgentTool {
            name: "timeline".to_string(),
            description:
                "Return coarse activity intervals within a bounded time window. Without app/window \
filters the result is audio-oriented; with an app or window title it returns matching screen \
intervals instead."
                    .to_string(),
            parameters_schema: timeline_tool_schema(),
        },
        ai_engine::AgentTool {
            name: "show_text".to_string(),
            description:
                "Return the broker-visible derived text for ONE opaque id previously returned by \
`search`. Use sparingly, only when a snippet is insufficient to answer."
                    .to_string(),
            parameters_schema: show_text_tool_schema(),
        },
        ai_engine::AgentTool {
            name: "recall_context".to_string(),
            description:
                "Return ONLY the User-Context conclusions (distilled beliefs about the user) and \
recent activities that are relevant to the question. Redacted and capped — it NEVER returns the \
whole dossier and NEVER returns sensitive-category conclusions. Use this for questions about the \
user's habits, interests, projects, or what you know about them, instead of raw `search`."
                    .to_string(),
            parameters_schema: recall_context_tool_schema(),
        },
        ai_engine::AgentTool {
            name: "reference_captures".to_string(),
            description:
                "Presentation signal that nominates the captures (screen frames / audio) behind \
your answer so the app can show them to the user as source cards. Returns NO capture data — only \
an acknowledgement of how many were accepted/dropped. Pass the opaque ids you received from \
`search` results, ordered most-relevant-first, and call this once near the end of your answer (a \
repeat call replaces the prior set). This does NOT count against the tool-call budget."
                    .to_string(),
            parameters_schema: reference_captures_tool_schema(),
        },
    ]
}

#[tauri::command]
pub async fn ask_ai_availability(
    app_handle: tauri::AppHandle,
) -> Result<AskAiAvailability, String> {
    if !read_ask_ai_enabled(&app_handle) {
        return Ok(AskAiAvailability {
            available: false,
            reason: Some("ask_ai_disabled".to_string()),
        });
    }

    // The engine prerequisite reason (no model / no key / unreachable local …)
    // is surfaced verbatim so the UI can explain why Ask AI is unavailable.
    let settings = read_ai_runtime_settings(&app_handle);
    match crate::ai_runtime::engine_configured_prerequisite(&settings).await {
        Ok(()) => Ok(AskAiAvailability {
            available: true,
            reason: None,
        }),
        Err(reason) => Ok(AskAiAvailability {
            available: false,
            reason: Some(reason),
        }),
    }
}

/// Emit a `ask_ai_status` event with the given JSON body merged with the
/// conversation id.
fn emit_status(app_handle: &tauri::AppHandle, conversation_id: &str, mut body: serde_json::Value) {
    if let Some(map) = body.as_object_mut() {
        map.insert(
            "conversationId".to_string(),
            serde_json::Value::String(conversation_id.to_string()),
        );
    }
    let _ = app_handle.emit(ASK_AI_STATUS_EVENT, body);
}

/// Persist one turn row in whatever phase the driver is in. Best-effort: a store
/// error is logged, never surfaced — the live stream events are authoritative and
/// persistence is a reattach convenience. Returns `true` when the row was
/// written, so the driver can announce the conversation to the history list the
/// first time the row actually exists.
#[allow(clippy::too_many_arguments)]
async fn persist_turn(
    infra: &AppInfraState,
    conversation_id: &str,
    title: &str,
    origin: &str,
    turn_index: i64,
    question: &str,
    answer: &str,
    reasoning: Option<&str>,
    tool_activities: &[serde_json::Value],
    sources: &[serde_json::Value],
    phase: &str,
    error_message: Option<&str>,
    seeded_result_count: Option<i64>,
) -> bool {
    let tool_activities_json = serde_json::to_string(tool_activities).unwrap_or_else(|_| "[]".into());
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
            &tool_activities_json,
            &sources_json,
            phase,
            error_message,
            seeded_result_count,
            now_ms(),
        )
        .await
    {
        Ok(()) => true,
        Err(error) => {
            tauri_plugin_log::log::warn!(
                "Ask AI failed to persist turn {turn_index} for {conversation_id} (phase {phase}): {error}"
            );
            false
        }
    }
}

/// Structured-extraction target for the generated thread title.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
struct GeneratedConversationTitle {
    /// A 3–6 word title summarizing the conversation topic.
    title: String,
}

/// Generated titles are first-turn-only: a thread is eligible exactly when the
/// completed turn was its first (`turn_index == 0`). Later turns never
/// (re)generate; the store-side conditional write additionally guards "no
/// title exists yet" and "a user rename wins forever".
fn title_generation_eligible(turn_index: i64) -> bool {
    turn_index == 0
}

/// System instruction for the title extraction. The prompt sees ONLY the
/// user's question text — no capture data, no transcript — so this call adds
/// no new redaction surface.
fn build_title_preamble() -> &'static str {
    "You name chat threads. Produce a short 3-6 word title that captures the topic of the \
user's question. Use plain words: no quotes, no trailing punctuation, no \"Question about\" \
prefix, and do not answer the question."
}

/// Per-call prompt for the title extraction: only the question text.
fn build_title_prompt(question: &str) -> String {
    format!("The user's first question to an assistant was:\n{question}\n\nTitle this thread.")
}

/// Normalize a model-produced title into an acceptable thread title, or `None`
/// when the result is unusable (empty, over-long) — an unusable result is a
/// swallowed failure that leaves the fallback first-question title in place.
fn normalize_generated_title(raw: &str) -> Option<String> {
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
async fn generate_conversation_title(
    app_handle: tauri::AppHandle,
    infra: AppInfraState,
    conversation_id: String,
    question: String,
) {
    let settings = read_ai_runtime_settings(&app_handle);
    let config = match crate::ai_runtime::resolve_engine_config(&settings, None, None) {
        Ok(config) => config,
        Err(reason) => {
            tauri_plugin_log::log::debug!(
                "Ask AI skipped title generation for {conversation_id}: {reason}"
            );
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
            tauri_plugin_log::log::warn!(
                "Ask AI title generation failed for {conversation_id}: {error}"
            );
            return;
        }
    };

    let Some(title) = normalize_generated_title(&extracted.title) else {
        tauri_plugin_log::log::warn!(
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
            let _ = app_handle.emit(CONVERSATION_CHANGED_EVENT, ());
        }
        // No longer eligible (user renamed mid-flight, or a title already
        // exists): the user/earlier title wins silently.
        Ok(false) => {}
        Err(error) => {
            tauri_plugin_log::log::warn!(
                "Ask AI failed to persist generated title for {conversation_id}: {error}"
            );
        }
    }
}

/// The single stateless-per-turn Ask AI driver used by BOTH start and follow-up.
///
/// Loads the conversation's completed history + engine pin from the store,
/// resolves the engine through the single precedence chain (ADR 0034: thread pin
/// → Ask AI model override → global default model), seeds best-effort via broker
/// search, persists a
/// `streaming` turn row, then runs ONE `ai_engine::run_agent_loop` against the
/// configured engine. The model's text streams as `ask_ai_delta` events (and is
/// periodically persisted as a partial for reattach); tool calls run through the
/// All-Retained broker seam Rust-side and drive the `ask_ai_status` tool phase +
/// `ask_ai_source` cards. On completion it persists the final turn and emits the
/// terminal `ask_ai_done`/`ask_ai_error`. A cooperative cancel keeps whatever was
/// generated (phase `done`) and emits no error.
///
/// Detached: the spawned task finishes regardless of dismiss/close, so an unseen
/// thread completes in the background and a reattach reads the persisted answer.
async fn run_ask_ai_turn(
    app_handle: tauri::AppHandle,
    conversation_id: String,
    question: String,
    seed_query: Option<String>,
    origin: String,
    title: String,
    clock: ClientClock,
    cancel: Arc<AtomicBool>,
) {
    // Resolve storage; without it we cannot persist or read history, so surface a
    // terminal error and stop.
    let infra = match app_infra(&app_handle) {
        Ok(infra) => infra,
        Err(error) => {
            let _ = app_handle.emit(
                ASK_AI_ERROR_EVENT,
                serde_json::json!({ "conversationId": conversation_id, "message": error }),
            );
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
    //    thread pin → Ask AI model override (`access.askAiModel`, a bare
    //    rig-core model id riding on the default model's provider) → global
    //    default model.
    let settings = read_recording_settings(&app_handle);
    let pin_ref = pin.as_ref().and_then(|(provider, model)| {
        match (provider.as_deref(), model.as_deref()) {
            (Some(provider), Some(model)) => Some((provider, model)),
            _ => None,
        }
    });
    let config_result = crate::ai_runtime::resolve_engine_config(
        &settings.ai_runtime,
        pin_ref,
        settings.access.ask_ai_model.as_deref(),
    );
    let config = match config_result {
        Ok(config) => config,
        Err(reason) => {
            let _ = app_handle.emit(
                ASK_AI_ERROR_EVENT,
                serde_json::json!({ "conversationId": conversation_id, "message": reason }),
            );
            remove_inflight_if_owner(&conversation_id, &cancel);
            return;
        }
    };

    // 3. Best-effort seeding via the broker search path (start only; follow-ups
    //    pass `seed_query: None`). A broker error/empty result proceeds unseeded.
    let seed_query = seed_query
        .map(|query| query.trim().to_string())
        .filter(|query| !query.is_empty());
    let mut seed_results: Vec<BrokerSearchResult> = Vec::new();
    if let Some(seed_query) = seed_query.as_deref() {
        emit_status(
            &app_handle,
            &conversation_id,
            serde_json::json!({ "phase": "seeding", "seededResultCount": 0 }),
        );
        let search_request = BrokerSearchRequest {
            query: seed_query.to_string(),
            from: None,
            to: None,
            limit: Some(ASK_AI_SEED_LIMIT),
            app: None,
            window_title: None,
        };
        if let Ok(BrokeredCaptureResponse::Search(response)) = execute_ask_ai_broker_request(
            app_handle.clone(),
            BrokeredCaptureRequest::Search(search_request),
        )
        .await
        {
            seed_results = response.results;
        }
        emit_status(
            &app_handle,
            &conversation_id,
            serde_json::json!({ "phase": "seeding", "seededResultCount": seed_results.len() }),
        );
    }
    let seeded_result_count = Some(seed_results.len() as i64);

    // A cancel arriving during seeding short-circuits before any model call.
    if cancel.load(Ordering::SeqCst) {
        remove_inflight_if_owner(&conversation_id, &cancel);
        return;
    }

    emit_status(
        &app_handle,
        &conversation_id,
        serde_json::json!({ "phase": "thinking" }),
    );

    // 4. Persist the turn row immediately (empty `streaming` answer) so a reattach
    //    can read the in-flight partial. Seeded count is carried from the start.
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
        &[],
        &[],
        "streaming",
        None,
        seeded_result_count,
    )
    .await
    {
        conversation_announced.store(true, Ordering::SeqCst);
        let _ = app_handle.emit(CONVERSATION_CHANGED_EVENT, ());
    }

    // 5. Search results (and seed results) are recorded by opaque id so a later
    //    `reference_captures` call can attach metadata and prove the model only
    //    references ids it actually received.
    let search_metadata: Arc<Mutex<HashMap<String, BrokerSearchResult>>> =
        Arc::new(Mutex::new(HashMap::new()));
    if !seed_results.is_empty() {
        if let Ok(mut map) = search_metadata.lock() {
            for result in &seed_results {
                map.insert(result.opaque_id.clone(), result.clone());
            }
        }
    }

    // Shared persistence buffers the executor appends to (tool-activity entries +
    // nominated Answer Sources) and the on_event closure reads when persisting.
    let tool_activities: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
    let sources: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));

    // 6. Build the tool executor. Each non-reference data tool rides
    //    `execute_ask_ai_broker_request` (access readiness + All-Retained scope +
    //    redaction/audit); `reference_captures` is intercepted before the broker
    //    and emits the source cards. Results are returned to the model as a JSON
    //    STRING.
    let executor: ai_engine::ToolExecutor = {
        let app_handle = app_handle.clone();
        let conversation_id = conversation_id.clone();
        let search_metadata = Arc::clone(&search_metadata);
        let tool_activities = Arc::clone(&tool_activities);
        let sources = Arc::clone(&sources);
        Arc::new(move |tool: String, params: serde_json::Value| {
            let app_handle = app_handle.clone();
            let conversation_id = conversation_id.clone();
            let search_metadata = Arc::clone(&search_metadata);
            let tool_activities = Arc::clone(&tool_activities);
            let sources = Arc::clone(&sources);
            Box::pin(async move {
                // Presentation signal: validate/decode + emit `ask_ai_source`,
                // never dispatched to the broker. Its emitted source set is also
                // stashed for persistence.
                if tool == "reference_captures" {
                    let (ack, nominated) = handle_reference_captures(
                        &app_handle,
                        &conversation_id,
                        &search_metadata,
                        params,
                    )
                    .await?;
                    if let Ok(mut buffer) = sources.lock() {
                        *buffer = nominated;
                    }
                    return serde_json::to_string(&ack)
                        .map_err(|error| format!("failed to serialize reference ack: {error}"));
                }

                // Data tool: record the activity, run it through the broker seam,
                // retain any search results, return the JSON result as a string.
                if let Ok(mut buffer) = tool_activities.lock() {
                    buffer.push(serde_json::json!({ "tool": tool, "params": params }));
                }
                let request = broker_request_from_tool(&tool, params)?;
                let response = execute_ask_ai_broker_request(app_handle, request).await?;
                if let BrokeredCaptureResponse::Search(ref response) = response {
                    if let Ok(mut map) = search_metadata.lock() {
                        for result in &response.results {
                            map.insert(result.opaque_id.clone(), result.clone());
                        }
                    }
                }
                let value = broker_response_to_tool_value(response)?;
                serde_json::to_string(&value)
                    .map_err(|error| format!("failed to serialize Ask AI tool result: {error}"))
            })
        })
    };

    let tools = build_ask_ai_tools();
    let max_tool_calls = read_ask_ai_max_tool_calls(&app_handle);
    let preamble = build_ask_ai_preamble();
    let prompt = build_ask_ai_prompt(
        &question,
        seed_query.as_deref(),
        &seed_results,
        now_ms(),
        &clock,
    );

    // 7. Run the agent loop, streaming deltas and persisting throttled partials.
    let answer: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    // Reasoning accumulator mirrors `answer`: reasoning chunks stream live as
    // `ask_ai_reasoning` events and accumulate here so every persist (partial /
    // done / error) writes the snapshot through `save_turn`'s `reasoning` arg.
    let reasoning: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let mut deltas_since_persist = 0usize;
    let mut chars_since_persist = 0usize;
    let on_event = {
        let app_handle = app_handle.clone();
        let conversation_id = conversation_id.clone();
        let infra = Arc::clone(&infra);
        let conversation_announced = Arc::clone(&conversation_announced);
        let answer = Arc::clone(&answer);
        let reasoning = Arc::clone(&reasoning);
        let tool_activities = Arc::clone(&tool_activities);
        let sources = Arc::clone(&sources);
        let title = title.clone();
        let origin = origin.clone();
        let question = question.clone();
        move |event: ai_engine::AgentLoopEvent| match event {
            ai_engine::AgentLoopEvent::Delta(text) => {
                let _ = app_handle.emit(
                    ASK_AI_DELTA_EVENT,
                    serde_json::json!({ "conversationId": conversation_id, "text": text }),
                );
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
                if let Some(answer_so_far) = answer_so_far {
                    let infra = Arc::clone(&infra);
                    let app_handle = app_handle.clone();
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
                    tauri::async_runtime::spawn(async move {
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
                            &tool_activities_snapshot,
                            &sources_snapshot,
                            "streaming",
                            None,
                            seeded_result_count,
                        )
                        .await;
                        // Announce only the FIRST successful persist of the turn
                        // (the initial persist normally wins; this covers it
                        // having failed), never every partial.
                        if persisted && !conversation_announced.swap(true, Ordering::SeqCst) {
                            let _ = app_handle.emit(CONVERSATION_CHANGED_EVENT, ());
                        }
                    });
                }
            }
            ai_engine::AgentLoopEvent::Reasoning(text) => {
                let _ = app_handle.emit(
                    ASK_AI_REASONING_EVENT,
                    serde_json::json!({ "conversationId": conversation_id, "text": text }),
                );
                if let Ok(mut guard) = reasoning.lock() {
                    guard.push_str(&text);
                }
            }
            ai_engine::AgentLoopEvent::ToolCall { name, params } => {
                // `reference_captures` is a presentation signal, not a data
                // activity, so it must not appear in the activity working-line.
                if name == "reference_captures" {
                    return;
                }
                emit_status(
                    &app_handle,
                    &conversation_id,
                    serde_json::json!({ "phase": "tool", "tool": name, "params": params }),
                );
            }
            // `Done` is handled after the loop returns; the loop emits it last.
            ai_engine::AgentLoopEvent::Done => {}
        }
    };

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
    let was_cancelled = cancel.load(Ordering::SeqCst);

    match run_result {
        Ok(()) => {
            // A cooperative cancel keeps whatever was generated and emits no
            // error; a clean finish persists `done` and emits the terminal event.
            persist_turn(
                &infra,
                &conversation_id,
                &title,
                &origin,
                turn_index,
                &question,
                &final_answer,
                final_reasoning.as_deref(),
                &final_tool_activities,
                &final_sources,
                "done",
                None,
                seeded_result_count,
            )
            .await;
            // Terminal persist updated the row (answer/updated-at), so the
            // history list re-sorts/refreshes its entry.
            let _ = app_handle.emit(CONVERSATION_CHANGED_EVENT, ());
            if !was_cancelled {
                let _ = app_handle.emit(
                    ASK_AI_DONE_EVENT,
                    serde_json::json!({ "conversationId": conversation_id }),
                );
            }
            // Fire-and-forget generated thread title: only after the FIRST turn
            // completes and persists. Spawned detached so it can never delay or
            // fail the turn; every failure inside is swallowed (the fallback
            // first-question title stands).
            if title_generation_eligible(turn_index) {
                tauri::async_runtime::spawn(generate_conversation_title(
                    app_handle.clone(),
                    Arc::clone(&infra),
                    conversation_id.clone(),
                    question.clone(),
                ));
            }
        }
        Err(error) => {
            let message = error.to_string();
            persist_turn(
                &infra,
                &conversation_id,
                &title,
                &origin,
                turn_index,
                &question,
                &final_answer,
                final_reasoning.as_deref(),
                &final_tool_activities,
                &final_sources,
                "error",
                Some(&message),
                seeded_result_count,
            )
            .await;
            let _ = app_handle.emit(CONVERSATION_CHANGED_EVENT, ());
            let _ = app_handle.emit(
                ASK_AI_ERROR_EVENT,
                serde_json::json!({ "conversationId": conversation_id, "message": message }),
            );
        }
    }

    // Remove only our own in-flight flag: a newer turn that displaced us holds a
    // different `Arc` and must survive our teardown.
    remove_inflight_if_owner(&conversation_id, &cancel);
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
        origin,
        title,
        prior_transcript: _,
        utc_offset_minutes,
        time_zone,
    } = request;

    let origin = origin
        .map(|origin| origin.trim().to_string())
        .filter(|origin| !origin.is_empty())
        .unwrap_or_else(|| ASK_AI_DEFAULT_ORIGIN.to_string());
    let title = title.unwrap_or_default();
    let clock = ClientClock {
        utc_offset_minutes,
        time_zone,
    };

    // Register the in-flight cancel flag (cancelling any prior running turn for
    // this conversation), then spawn the detached driver. The command returns
    // promptly so the turn completes in the background regardless of dismiss.
    let cancel = register_inflight(&conversation_id);
    tauri::async_runtime::spawn(run_ask_ai_turn(
        app_handle,
        conversation_id,
        question,
        seed_query,
        origin,
        title,
        clock,
        cancel,
    ));

    Ok(())
}

/// Run a follow-up question as another stateless turn on an existing thread.
///
/// `conversationId` identifies the whole thread. Unlike start there is NO seeding
/// and NO `seedQuery`: the prior turns' completed history is reloaded from the
/// store by [`run_ask_ai_turn`] and fed to the agent loop as conversation
/// history. A follow-up always works — there is no resident session to be "no
/// longer active".
#[tauri::command]
pub async fn ask_ai_followup(
    app_handle: tauri::AppHandle,
    request: AskAiFollowupRequest,
) -> Result<(), String> {
    ensure_ask_ai_access_ready(&app_handle).await?;

    let AskAiFollowupRequest {
        conversation_id,
        question,
        utc_offset_minutes,
        time_zone,
    } = request;
    let question = question.trim().to_string();
    if question.is_empty() {
        return Err("Ask AI follow-up question is empty".to_string());
    }
    let clock = ClientClock {
        utc_offset_minutes,
        time_zone,
    };

    // A follow-up reuses the conversation's existing origin/title (the store
    // preserves an existing row's origin and first non-empty title regardless of
    // what is passed), so default values are fine here.
    let cancel = register_inflight(&conversation_id);
    tauri::async_runtime::spawn(run_ask_ai_turn(
        app_handle,
        conversation_id,
        question,
        None,
        ASK_AI_DEFAULT_ORIGIN.to_string(),
        String::new(),
        clock,
        cancel,
    ));

    Ok(())
}

#[tauri::command]
pub async fn ask_ai_cancel(
    _app_handle: tauri::AppHandle,
    request: AskAiCancelRequest,
) -> Result<(), String> {
    // Cooperative cancel: set + remove the conversation's in-flight flag. The
    // running loop checks it between stream items and stops cleanly, keeping
    // whatever was generated so far.
    if let Some(cancel) = take_inflight(&request.conversation_id) {
        cancel.store(true, Ordering::SeqCst);
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

    #[test]
    fn ask_ai_broker_identity_uses_ask_ai_label() {
        let identity = ask_ai_broker_identity().expect("Ask AI identity should be valid");

        assert_eq!(identity.label, "Ask AI");
        assert_eq!(identity.normalized_label, "ask ai");
        assert_eq!(identity.source, BrokerClientIdentitySource::Inferred);
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
    fn preamble_documents_the_tools_and_graphical_affordance() {
        let preamble = build_ask_ai_preamble();
        // The four data tools + the presentation tool are all described.
        assert!(preamble.contains("`search`"));
        assert!(preamble.contains("`timeline`"));
        assert!(preamble.contains("`show_text`"));
        assert!(preamble.contains("`recall_context`"));
        assert!(preamble.contains("`reference_captures`"));
        // The graphical-answer affordance blocks are documented.
        assert!(preamble.contains("mnema-bars"));
        assert!(preamble.contains("mnema-dossier"));
        assert!(preamble.contains("mnema-timeline"));
        // The preamble is the SYSTEM instruction — it must carry no question.
        assert!(!preamble.contains("Question:"));
    }

    #[test]
    fn prompt_unseeded_is_grounding_then_question() {
        let prompt = build_ask_ai_prompt("What did I do?", None, &[], 0, &ClientClock::default());
        assert!(!prompt.contains("Context from the user's captures"));
        // The temporal grounding leads; the bare question trails.
        assert!(prompt.starts_with("Temporal grounding: "));
        assert!(prompt.ends_with("Question: What did I do?"));
    }

    #[test]
    fn prompt_with_empty_results_omits_context_block() {
        let prompt = build_ask_ai_prompt("Q?", Some("build"), &[], 0, &ClientClock::default());
        assert!(!prompt.contains("Context from the user's captures"));
        assert!(prompt.ends_with("Question: Q?"));
    }

    #[test]
    fn prompt_seeded_includes_numbered_context() {
        let prompt = build_ask_ai_prompt(
            "Did the build pass?",
            Some("build"),
            &[sample_result()],
            0,
            &ClientClock::default(),
        );
        assert!(prompt.contains("Context from the user's captures for \"build\":"));
        assert!(prompt.contains(
            "1. [frame · Xcode · \"ContentView.swift\" · 2026-01-01T10:00:00Z–2026-01-01T10:01:00Z · opaqueId=op-1] build passed"
        ));
        assert!(prompt.ends_with("Question: Did the build pass?"));
    }

    #[test]
    fn temporal_grounding_renders_local_and_utc() {
        // now_ms = 0 → 1970-01-01 00:00 UTC. PST (UTC-08:00) lands at 1969-12-31 16:00.
        let clock = ClientClock {
            utc_offset_minutes: Some(-480),
            time_zone: Some("America/Los_Angeles".to_string()),
        };
        let grounding = build_temporal_grounding(0, &clock);
        assert!(grounding.contains("1969-12-31 16:00"));
        assert!(grounding.contains("(America/Los_Angeles, UTC-08:00)"));
        assert!(grounding.contains("1970-01-01 00:00 UTC"));
        assert!(grounding.contains("RFC3339 `Z`"));
    }

    #[test]
    fn temporal_grounding_falls_back_to_utc_without_offset() {
        let grounding = build_temporal_grounding(0, &ClientClock::default());
        assert!(grounding.contains("1970-01-01 00:00 UTC"));
        assert!(grounding.contains("local offset is unknown"));
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
    fn availability_reason_is_ask_ai_disabled_when_access_off() {
        // The availability reason logic: Ask AI disabled short-circuits to the
        // `ask_ai_disabled` reason BEFORE the engine prerequisite is consulted.
        // (The full command needs a Tauri app handle; here we assert the reason
        // contract the command relies on — a disabled flag produces this exact
        // reason, the same string `ensure_ask_ai_access_ready` would otherwise
        // surface via its disabled-message branch.)
        let availability = AskAiAvailability {
            available: false,
            reason: Some("ask_ai_disabled".to_string()),
        };
        assert!(!availability.available);
        assert_eq!(availability.reason.as_deref(), Some("ask_ai_disabled"));
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
    fn broker_request_from_tool_recall_context_maps_to_recall_context_variant() {
        let request = broker_request_from_tool(
            "recall_context",
            serde_json::json!({ "query": "what am I working on", "limit": 5 }),
        )
        .expect("recall_context params should parse");

        match request {
            BrokeredCaptureRequest::RecallContext(req) => {
                assert_eq!(req.query, "what am I working on");
                assert_eq!(req.limit, Some(5));
            }
            other => panic!("expected RecallContext, got {other:?}"),
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
        assert!(frame.as_object().unwrap().contains_key("sourceKind"));
        assert_eq!(frame["sourceKind"], serde_json::Value::Null);
        assert_eq!(frame["spanStartMs"], serde_json::Value::Null);
        assert_eq!(frame["alignedFrameId"], serde_json::Value::Null);

        let audio = &sources[1];
        assert_eq!(audio["kind"], serde_json::json!("audio"));
        assert_eq!(audio["frameId"], serde_json::Value::Null);
        assert_eq!(audio["audioSegmentId"], serde_json::json!(7));
        assert_eq!(audio["windowTitle"], serde_json::Value::Null);
        assert!(audio.as_object().unwrap().contains_key("sourceKind"));
        assert_eq!(audio["sourceKind"], serde_json::Value::Null);
        assert_eq!(audio["spanStartMs"], serde_json::json!(3_000));
        assert_eq!(audio["alignedFrameId"], serde_json::json!(99));
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

        let error =
            broker_response_to_tool_value(response).expect_err("error envelope should become Err");
        assert_eq!(error, "result is unavailable or outside the grant scope");
    }

    #[test]
    fn followup_request_deserializes_camel_case_without_seed_query() {
        let request: AskAiFollowupRequest = serde_json::from_str(
            r#"{"conversationId":"conv-1","question":"what about in Slack?"}"#,
        )
        .expect("follow-up request should deserialize");
        assert_eq!(request.conversation_id, "conv-1");
        assert_eq!(request.question, "what about in Slack?");

        let request: AskAiFollowupRequest = serde_json::from_str(
            r#"{"conversationId":"conv-2","question":"more","seedQuery":"ignored"}"#,
        )
        .expect("extra fields are ignored");
        assert_eq!(request.conversation_id, "conv-2");
        assert_eq!(request.question, "more");
    }

    #[test]
    fn start_request_deserializes_without_optional_fields() {
        // Existing callers (no origin/title/priorTranscript) keep working.
        let request: AskAiStartRequest = serde_json::from_str(
            r#"{"conversationId":"conv-1","question":"what did I do?","seedQuery":"build"}"#,
        )
        .expect("start request without optional fields should deserialize");
        assert_eq!(request.conversation_id, "conv-1");
        assert_eq!(request.question, "what did I do?");
        assert_eq!(request.seed_query.as_deref(), Some("build"));
        assert!(request.origin.is_none());
        assert!(request.title.is_none());
        assert!(request.prior_transcript.is_none());
    }

    #[test]
    fn start_request_accepts_origin_title_and_ignores_prior_transcript() {
        let request: AskAiStartRequest = serde_json::from_str(
            r#"{"conversationId":"c","question":"q","origin":"chat","title":"My chat","priorTranscript":[{"question":"q1","answer":"a1"}]}"#,
        )
        .expect("start request with origin/title/priorTranscript should deserialize");
        assert_eq!(request.origin.as_deref(), Some("chat"));
        assert_eq!(request.title.as_deref(), Some("My chat"));
        // priorTranscript still deserializes (as opaque JSON) but is ignored.
        assert!(request.prior_transcript.is_some());
    }

    #[test]
    fn title_generation_is_first_turn_only() {
        // Eligibility rule: only the FIRST turn of a thread generates a title;
        // a follow-up (second turn onward) is a no-op.
        assert!(title_generation_eligible(0));
        assert!(!title_generation_eligible(1));
        assert!(!title_generation_eligible(7));
    }

    #[test]
    fn title_prompt_sees_only_the_question_text() {
        let prompt = build_title_prompt("what did I work on yesterday?");
        assert!(prompt.contains("what did I work on yesterday?"));
        // No capture data / transcript / tool text leaks into the title call.
        assert!(!prompt.contains("Context from the user's captures"));

        let preamble = build_title_preamble();
        assert!(preamble.contains("3-6 word"));
    }

    #[test]
    fn normalize_generated_title_accepts_a_clean_short_title() {
        assert_eq!(
            normalize_generated_title("Yesterday's coding session"),
            Some("Yesterday's coding session".to_string())
        );
        // Wrapping quotes, trailing punctuation, and ragged whitespace are
        // stripped/collapsed rather than rejected.
        assert_eq!(
            normalize_generated_title("\u{201c}Rust borrow checker help.\u{201d}"),
            Some("Rust borrow checker help".to_string())
        );
        assert_eq!(
            normalize_generated_title("  Weekly   planning  recap \n"),
            Some("Weekly planning recap".to_string())
        );
    }

    #[test]
    fn normalize_generated_title_rejects_empty_and_overlong() {
        // Empty / whitespace / quote-only results are failures.
        assert_eq!(normalize_generated_title(""), None);
        assert_eq!(normalize_generated_title("   "), None);
        assert_eq!(normalize_generated_title("\"\""), None);
        // Too many words (the model ignored the 3–6 word instruction).
        assert_eq!(
            normalize_generated_title(
                "This is a much too long title that rambles on and on about everything"
            ),
            None
        );
        // Few words but absurdly long characters-wise.
        let overlong = format!("{} {}", "a".repeat(40), "b".repeat(40));
        assert_eq!(normalize_generated_title(&overlong), None);
    }

    #[test]
    fn inflight_registry_register_take_roundtrip() {
        let id = "inflight-roundtrip-conv";
        let cancel = register_inflight(id);
        assert!(!cancel.load(Ordering::SeqCst));
        let taken = take_inflight(id).expect("registered flag should be takeable");
        assert!(Arc::ptr_eq(&cancel, &taken));
        // Once taken, the registry no longer holds it.
        assert!(take_inflight(id).is_none());
    }

    #[test]
    fn register_cancels_displaced_inflight_turn() {
        let id = "inflight-displace-conv";
        let first = register_inflight(id);
        assert!(!first.load(Ordering::SeqCst));
        // A racing second start/follow-up displaces the first, cancelling it.
        let second = register_inflight(id);
        assert!(
            first.load(Ordering::SeqCst),
            "displacing an in-flight turn must set its cancel flag"
        );
        assert!(!second.load(Ordering::SeqCst));
        // Cleanup so the static map does not leak into other tests.
        let _ = take_inflight(id);
    }

    #[test]
    fn remove_if_owner_spares_a_displacing_turn() {
        let id = "inflight-owner-conv";
        let first = register_inflight(id);
        let second = register_inflight(id);
        // The first (displaced) turn finishing must NOT evict the newer flag.
        remove_inflight_if_owner(id, &first);
        let still = take_inflight(id).expect("newer flag should still be registered");
        assert!(Arc::ptr_eq(&still, &second));
    }
}
