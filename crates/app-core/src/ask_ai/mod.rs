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
//! The brokered data tools (`search`, `timeline`, `show_text`, `recall_context`,
//! `activities`)
//! plus the presentation-only `reference_captures` tool are described to the model
//! and executed through the All-Retained broker seam
//! (`BrokeredCaptureAccess::execute_for_ask_ai`) Rust-side, with redaction/audit
//! and the per-question tool-call cap enforced here. `open`/`open_in_mnema` is NOT
//! an Ask AI tool and is rejected before the broker.
//!
//! The streaming EVENT surface is the single versioned `ask_ai_update` event
//! ([`CoreEvent::AskAiUpdate`]: one [`TurnUpdate`] per emit, keyed by
//! `conversationId` + `turnIndex`),
//! self-healable via the `ask_ai_snapshot` command. The legacy per-kind events
//! (`ask_ai_status`/`ask_ai_delta`/`ask_ai_reasoning`/`ask_ai_done`/
//! `ask_ai_error`/`ask_ai_source`) were removed once both frontends migrated.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use app_infra::brokered_access::{
    BrokerClientIdentity, BrokerClientIdentitySource, BrokerRecallContextRequest,
    BrokerSearchRequest, BrokerSearchResult, BrokerTimelineRequest, BrokeredCaptureAccess,
    BrokeredCaptureRequest, BrokeredCaptureResponse,
};
use capture_types::{AiRuntimeSettings, AskAiScope, TurnSnapshot, TurnUpdate, TurnView};
use serde::{Deserialize, Serialize};

use crate::ask_ai::answer_view::AnswerView;
use crate::{AppCore, AppInfraState, CoreEvent};

mod abandoned;
pub mod answer_view;
pub mod app_control;
mod broker;
mod live;
pub mod mcp;
mod persist;
mod prompt;
mod scope;
pub mod tool_activity;
mod turn;
mod turn_events;
mod turn_tools;
pub mod web_fetch;

use self::broker::*;
use self::live::*;
pub use self::live::{
    apply_update_to_view, get_ask_ai_last_turn_usage, turn_in_flight, AskAiTokenUsage,
};
use self::persist::*;
use self::prompt::*;
use self::turn::*;
use self::turn_events::*;
use self::turn_tools::*;

/// One `ask_ai_update` emit: a versioned [`TurnUpdate`] for one turn of one
/// conversation, the wire shape the frontend's reducer reads.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AskAiUpdateEvent {
    pub conversation_id: String,
    pub version: u64,
    pub turn_index: i64,
    pub update: TurnUpdate,
}

/// The unified view-model transport event (issue #110). Carries one versioned
/// [`TurnUpdate`] keyed by `conversationId` + `turnIndex`. This is the SOLE Ask AI
/// streaming surface — the legacy `ask_ai_status`/`ask_ai_delta`/`ask_ai_reasoning`
/// /`ask_ai_done`/`ask_ai_error`/`ask_ai_source` events were removed in Phase 2.
pub const ASK_AI_UPDATE_EVENT: &str = "ask_ai_update";

/// Per-kind caps on the nominated Answer Source set emitted to the frontend.
pub const ASK_AI_SOURCE_FRAME_CAP: usize = 6;
pub const ASK_AI_SOURCE_AUDIO_CAP: usize = 4;

/// Persist-throttle thresholds for the streaming partial answer. The accumulating
/// answer is re-persisted to the turn row (phase `streaming`) once either many
/// deltas or many new chars have accrued since the last persist, so a reattach
/// reads a recent partial without one DB write per token.
pub const ASK_AI_PARTIAL_PERSIST_DELTA_INTERVAL: usize = 20;
pub const ASK_AI_PARTIAL_PERSIST_CHAR_INTERVAL: usize = 200;

/// Default origin stamped on a conversation when `ask_ai_start` does not carry one
/// and the conversation row does not yet exist. Quick Recall is the historical
/// Ask AI door; Chat passes `"chat"` explicitly (Slice 7).
pub const ASK_AI_DEFAULT_ORIGIN: &str = "quick_recall";

/// Bounds on an accepted generated thread title. The prompt asks for 3–6 words;
/// anything past these slack limits is treated as a failed generation (the
/// fallback first-question title stands).
pub const GENERATED_TITLE_MAX_WORDS: usize = 8;
pub const GENERATED_TITLE_MAX_CHARS: usize = 64;

/// Translate the persisted `askAiMaxToolCalls` setting (`0` = no cap) into the
/// per-turn cap passed to the agent loop. `0` becomes `usize::MAX` so the agent
/// may issue unlimited follow-up brokered queries (the loop clamps it to a sane
/// internal ceiling).
pub fn resolve_tool_call_cap(setting: u32) -> usize {
    if setting == 0 {
        usize::MAX
    } else {
        setting as usize
    }
}

pub fn access_config_dir(core: &AppCore) -> Result<PathBuf, String> {
    Ok(core.paths().config.clone())
}

pub fn broker_access(core: &AppCore) -> Result<BrokeredCaptureAccess, String> {
    Ok(BrokeredCaptureAccess::from_config_dir(access_config_dir(
        core,
    )?))
}

/// The broker client identity Ask AI presents to the All-Retained broker seam.
///
/// The label is "Ask AI" (the in-process agent is the client now; PI is gone) and
/// the source is `Inferred`, matching how the broker attributes an internally
/// inferred client rather than an explicitly authenticated CLI identity.
pub fn ask_ai_broker_identity() -> Result<BrokerClientIdentity, String> {
    BrokerClientIdentity::new("Ask AI", BrokerClientIdentitySource::Inferred)
        .map_err(|error| error.to_string())
}

/// Read the full current recording settings (so callers can pull both the
/// `access` and `ai_runtime` domains from one read).
pub fn read_recording_settings(core: &AppCore) -> capture_types::RecordingSettings {
    crate::native_capture::control::current_recording_settings_from_core(core)
}

pub fn read_ask_ai_enabled(core: &AppCore) -> bool {
    read_recording_settings(core).access.ask_ai_enabled
}

/// Whether the opt-in `fetch_url` web-fetch tool is enabled (default off). Read
/// per turn so a Settings toggle takes effect on the next question.
pub fn read_ask_ai_web_fetch_enabled(core: &AppCore) -> bool {
    read_recording_settings(core)
        .access
        .ask_ai_web_fetch_enabled
}

pub fn read_ai_runtime_settings(core: &AppCore) -> AiRuntimeSettings {
    read_recording_settings(core).ai_runtime
}

/// Read the configured per-question tool-call cap (`0` = no cap).
pub fn read_ask_ai_max_tool_calls(core: &AppCore) -> usize {
    resolve_tool_call_cap(read_recording_settings(core).access.ask_ai_max_tool_calls)
}

/// Resolve the AppInfra handle (for the conversation store). Cloned `Arc` so the
/// caller can hold it across awaits without borrowing the Tauri `State` guard.
pub fn app_infra(core: &AppCore) -> Result<AppInfraState, String> {
    core.try_state::<AppInfraState>()
        .map(|state| Arc::clone(&*state))
        .ok_or_else(|| "Ask AI storage is unavailable".to_string())
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
    pub conversation_id: String,
    pub question: String,
    /// The door that created/owns the conversation: `"quick_recall"` | `"chat"`.
    /// Stamped on the turn row only when the conversation is newly created (the
    /// store preserves an existing row's origin). Optional for wire back-compat;
    /// defaults to Quick Recall, the historical Ask AI door.
    #[serde(default)]
    pub origin: Option<String>,
    /// The conversation title for the upsert (the first non-empty title wins).
    /// Optional; defaults to empty.
    #[serde(default)]
    pub title: Option<String>,
    /// Legacy resurrect-from-transcript field. History now comes from the backend
    /// conversation store, so this is IGNORED — kept only so the frontend's
    /// existing start payload still deserializes during the migration.
    #[serde(default)]
    #[allow(dead_code)]
    pub prior_transcript: Option<serde_json::Value>,
    /// Minutes to add to UTC to reach the user's local wall clock, supplied by the
    /// frontend (`-Date.getTimezoneOffset()`). Optional for wire back-compat.
    #[serde(default)]
    pub utc_offset_minutes: Option<i32>,
    /// IANA zone name for display in the temporal grounding. Optional.
    #[serde(default)]
    pub time_zone: Option<String>,
    /// Chat's scope (CH3), resent every turn; absent = unscoped.
    #[serde(default)]
    pub scope: Option<AskAiScope>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskAiFollowupRequest {
    pub conversation_id: String,
    pub question: String,
    /// See [`AskAiStartRequest::utc_offset_minutes`].
    #[serde(default)]
    pub utc_offset_minutes: Option<i32>,
    /// See [`AskAiStartRequest::time_zone`].
    #[serde(default)]
    pub time_zone: Option<String>,
    /// See [`AskAiStartRequest::scope`].
    #[serde(default)]
    pub scope: Option<AskAiScope>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskAiCancelRequest {
    pub conversation_id: String,
    /// The question was edited into a new one (Quick Recall): drop the thread
    /// even if a partial answer already arrived (see `abandoned`).
    #[serde(default)]
    pub discard: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AskAiAvailability {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

pub async fn ask_ai_availability(core: &AppCore) -> Result<AskAiAvailability, String> {
    if !read_ask_ai_enabled(core) {
        return Ok(AskAiAvailability {
            available: false,
            reason: Some("ask_ai_disabled".to_string()),
        });
    }

    // The engine prerequisite reason (no model / no key / unreachable local …)
    // is surfaced verbatim so the UI can explain why Ask AI is unavailable.
    let settings = read_recording_settings(core);
    match crate::ai_runtime::ask_ai_engine_prerequisite(&settings.ai_runtime, &settings.access)
        .await
    {
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

pub async fn ask_ai_start(core: &Arc<AppCore>, request: AskAiStartRequest) -> Result<(), String> {
    let AskAiStartRequest {
        conversation_id,
        question,
        origin,
        title,
        prior_transcript: _,
        utc_offset_minutes,
        time_zone,
        scope,
    } = request;

    // Register the in-flight cancel flag FIRST — before the async readiness check —
    // so a Stop that races the request start is captured (consumed from the
    // pending-cancel set) rather than dropped, and so a pending entry can never
    // leak past a failed readiness check. If readiness fails no turn is spawned, so
    // clean the flag back up. Registering also cancels any prior running turn for
    // this conversation, which is the intended displacement on a new start.
    let cancel = register_inflight(&conversation_id);
    if let Err(err) = ensure_ask_ai_access_ready(core).await {
        remove_inflight_if_owner(&conversation_id, &cancel);
        return Err(err);
    }

    let origin = origin
        .map(|origin| origin.trim().to_string())
        .filter(|origin| !origin.is_empty())
        .unwrap_or_else(|| ASK_AI_DEFAULT_ORIGIN.to_string());
    let title = title.unwrap_or_default();
    let clock = ClientClock {
        utc_offset_minutes,
        time_zone,
    };

    // Spawn the detached driver. The command returns promptly so the turn completes
    // in the background regardless of dismiss.
    core.spawn(run_ask_ai_turn(
        core.clone(),
        conversation_id,
        question,
        origin,
        title,
        clock,
        scope,
        cancel,
    ));

    Ok(())
}

/// Run a follow-up question as another stateless turn on an existing thread.
///
/// `conversationId` identifies the whole thread: the prior turns' completed
/// history is reloaded from the
/// store by [`run_ask_ai_turn`] and fed to the agent loop as conversation
/// history. A follow-up always works — there is no resident session to be "no
/// longer active".
pub async fn ask_ai_followup(
    core: &Arc<AppCore>,
    request: AskAiFollowupRequest,
) -> Result<(), String> {
    let AskAiFollowupRequest {
        conversation_id,
        question,
        utc_offset_minutes,
        time_zone,
        scope,
    } = request;
    let question = question.trim().to_string();
    if question.is_empty() {
        return Err("Ask AI follow-up question is empty".to_string());
    }
    let clock = ClientClock {
        utc_offset_minutes,
        time_zone,
    };

    // Register the in-flight cancel flag before the async readiness check so a Stop
    // racing the request start is honored, not dropped (see `ask_ai_start`). Clean
    // up if readiness fails, since no turn is spawned. A follow-up reuses the
    // conversation's existing origin/title (the store preserves an existing row's
    // origin and first non-empty title regardless of what is passed), so default
    // values are fine here.
    let cancel = register_inflight(&conversation_id);
    if let Err(err) = ensure_ask_ai_access_ready(core).await {
        remove_inflight_if_owner(&conversation_id, &cancel);
        return Err(err);
    }
    core.spawn(run_ask_ai_turn(
        core.clone(),
        conversation_id,
        question,
        ASK_AI_DEFAULT_ORIGIN.to_string(),
        String::new(),
        clock,
        scope,
        cancel,
    ));

    Ok(())
}

pub async fn ask_ai_cancel(request: AskAiCancelRequest) -> Result<(), String> {
    // Cooperative cancel: set + remove the conversation's in-flight flag. The
    // running loop checks it between stream items and stops cleanly, keeping
    // whatever was generated so far.
    //
    // If no flag is registered yet, the cancel raced ahead of the turn's
    // registration (the Stop button is live before `register_inflight` runs, which
    // sits behind an async access-readiness check). Record it as pending so the
    // imminent registration brings the turn up already cancelled instead of
    // dropping the cancel and letting the turn run to completion.
    // A pre-registration cancel never writes a row, so only a running turn
    // needs the discard mark. It is set under the in-flight lock, before the
    // flag: the turn either sees it at finalize or clears it once its own flag
    // is gone (see `remove_inflight_if_owner`), so a late mark never lingers.
    let taken = ask_ai_inflight().lock().ok().and_then(|mut map| {
        let cancel = map.remove(&request.conversation_id)?;
        if request.discard {
            abandoned::mark_superseded(&request.conversation_id);
        }
        Some(cancel)
    });
    match taken {
        Some(cancel) => cancel.store(true, Ordering::SeqCst),
        None => record_cancel_before_register(&request.conversation_id),
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskAiSnapshotRequest {
    pub conversation_id: String,
}

/// Return the current versioned [`TurnSnapshot`] for a conversation's in-flight
/// turn, or `None` when no turn is live. A reattaching frontend calls this to
/// self-heal to the exact current view + version, then ignores any live
/// `ask_ai_update` it had already applied at or below that version.
pub async fn ask_ai_snapshot(
    request: AskAiSnapshotRequest,
) -> Result<Option<TurnSnapshot>, String> {
    Ok(snapshot_live_turn(&request.conversation_id))
}

#[cfg(test)]
mod tests;
