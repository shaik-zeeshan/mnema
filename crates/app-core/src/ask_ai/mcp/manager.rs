//! The persistent [`McpManager`] — lazy connect, warm-on-open discovery, and a
//! per-app-session connection cache for user-configured MCP tool connectors
//! (Workstream C, ADR 0048).
//!
//! # Lifecycle
//! Nothing connects at app launch (the deferred-startup invariant holds): the
//! first `warm`/`tools_for_turn`/`call_tool` for a server dials it, and the
//! handle is cached for the app session. Both chat doors call `mcp_warm_connectors`
//! on open, so a turn build usually finds discovery already done — or in-flight,
//! which it AWAITS (up to [`DISCOVERY_TURN_BUDGET`]) rather than re-dialing,
//! because a second caller simply waits on the same slot lock.
//!
//! # Health / failure seam
//! This module is the ONE place MCP connection health is managed, and the policy
//! is deliberately minimal (ADR 0048 documented deferral): a failed call/connect
//! drops the handle and redials ONCE on next use; a second consecutive failure
//! surfaces readable error text to the model, whose remedy is the server's
//! `enabled` toggle. Health checks, auto-restart, backoff, and a live per-server
//! status indicator would all be added HERE — do NOT build them until real
//! dead-server behavior confuses users.
//!
//! # Teardown
//! Removed/disabled servers are reaped by `reconcile` (wired into the AI-runtime
//! settings save); an EDITED server (changed connect fingerprint) is reaped
//! lazily at `slot_for` on next use. On app exit, dropping a cached handle kills
//! its child's whole process GROUP on Unix — launcher AND grandchildren — via the
//! child transport's `Drop` (see [`McpClient`]; macOS exercised — SUPPORTS.md).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use capture_types::{McpAuthMode, McpServerConfig};
use futures_util::future::join_all;
use tokio::sync::Mutex;

use super::oauth_flow::PendingOAuth;
use super::transport::{config_fingerprint, connect, McpClient};
use super::{
    bound_tool_description, bound_tool_schema, is_valid_model_tool_name, model_tool_name,
    offered_tools, truncate_tool_result, ToolInfo,
};
use crate::AppCore;

/// Total budget a turn build waits for in-flight discovery before proceeding
/// with only the servers that are ready — npx cold boot can be slow, and a turn
/// must never block forever on a connector.
const DISCOVERY_TURN_BUDGET: Duration = Duration::from_secs(15);

/// Hard cap on ONE tool call's execution (the `call_tool` await itself; the
/// connect is already bounded by [`DISCOVERY_TURN_BUDGET`] inside `ensure_ready`).
/// A hung MCP server must never freeze a chat turn. Flat, not per-server
/// configurable, until a real connector proves slower (YAGNI, ADR 0048).
const CALL_TOOL_BUDGET: Duration = Duration::from_secs(60);

/// One server's live connection plus the tools it advertised at connect. Held
/// behind an `Arc` so a mid-call clone keeps the child alive even if the slot is
/// invalidated concurrently.
struct Established {
    client: McpClient,
    tools: Vec<ToolInfo>,
    /// Tools DROPPED at discovery because their model-facing `mcp__<id>__<name>`
    /// violates the provider tool-name contract (see [`is_valid_model_tool_name`]).
    /// Surfaced as one preamble note per turn — non-silent, like the 32-cap trim.
    dropped_invalid_names: usize,
}

/// Per-server connection slot, keyed by instance id. Its inner `Mutex` serializes
/// connect/redial AND caches the established connection, so a second caller
/// (warm vs. turn) that arrives mid-connect simply AWAITS the in-flight connect
/// on the lock — that is how "await in-flight discovery" is realized without a
/// separate task handle.
pub(super) struct ServerSlot {
    /// The connect-relevant config fingerprint this slot was built for; a Settings
    /// edit changes it and the manager replaces the slot (dropping the old child).
    fingerprint: String,
    state: Mutex<SlotState>,
}

#[derive(Default)]
struct SlotState {
    established: Option<Arc<Established>>,
    /// Consecutive connect/list/call failures, for logging only (the failure
    /// policy's redial-once falls out of invalidate + reconnect-on-next-use).
    consecutive_failures: u8,
}

impl ServerSlot {
    /// Ensure this slot has a live connection, returning it. Reuses the cached
    /// connection if present; otherwise connects (the "redial on next use" of the
    /// failure policy falls out of this — an invalidated slot reconnects here).
    async fn ensure_ready(&self, cfg: &McpServerConfig) -> Result<Arc<Established>, String> {
        let mut state = self.state.lock().await;
        if let Some(established) = state.established.as_ref() {
            return Ok(Arc::clone(established));
        }
        // Bound the connect. rmcp's `serve` runs an UNBOUNDED initialize handshake
        // (no internal timeout), and this lock is held across it — so without a cap
        // the untimed `warm`/`list_server_tools` callers would hold the slot lock
        // forever on a server that hangs mid-handshake, leaking the child + task and
        // taxing every later turn (which waits on this lock). On elapse the dropped
        // future drops the transport → the child is killed and the slot is freed.
        match tokio::time::timeout(DISCOVERY_TURN_BUDGET, connect_and_list(cfg)).await {
            Ok(Ok(established)) => {
                let established = Arc::new(established);
                state.established = Some(Arc::clone(&established));
                state.consecutive_failures = 0;
                Ok(established)
            }
            Ok(Err(error)) => {
                state.consecutive_failures = state.consecutive_failures.saturating_add(1);
                Err(error)
            }
            Err(_elapsed) => {
                state.consecutive_failures = state.consecutive_failures.saturating_add(1);
                Err(format!(
                    "timed out connecting to \"{}\" after {}s",
                    cfg.label,
                    DISCOVERY_TURN_BUDGET.as_secs()
                ))
            }
        }
    }

    /// Drop the cached connection (kills the child on last ref) so the next
    /// `ensure_ready` redials. Called after a call/list error.
    async fn invalidate(&self) {
        self.state.lock().await.established = None;
    }
}

/// Connect + discover in one step: dial the server, run the MCP handshake, and
/// list all its tools.
async fn connect_and_list(cfg: &McpServerConfig) -> Result<Established, String> {
    let client = connect(cfg).await?;
    let tools = client
        .list_all_tools()
        .await
        .map_err(|error| format!("failed to list tools for \"{}\": {error}", cfg.label))?;
    let listed = tools.len();
    let tools: Vec<ToolInfo> = tools
        .into_iter()
        .filter_map(|tool| tool_info_from_rmcp(&cfg.id, tool))
        .collect();
    let dropped_invalid_names = listed - tools.len();
    Ok(Established {
        client,
        tools,
        dropped_invalid_names,
    })
}

/// The loggable head of a connect/list failure: our own sentence, cut before
/// the first `": "`, which is where every wrapped transport error starts.
/// rmcp's reqwest errors quote the connector URL, and a URL can carry a token;
/// the error itself is not reachable as a `reqwest::Error` to strip it.
fn connect_failure_log_head(error: &str) -> &str {
    error.split_once(": ").map_or(error, |(head, _)| head)
}

/// Project rmcp's `Tool` onto our trimmed [`ToolInfo`] (name, description,
/// schema) — or DROP it (`None`) when its model-facing `mcp__<id>__<name>` would
/// violate the provider tool-name contract (`^[a-zA-Z0-9_-]{1,64}$`): providers
/// reject the ENTIRE request over one bad name, killing every tool for the turn.
/// Never truncate — a rewritten name would no longer route (`parse_mcp_tool_name`).
fn tool_info_from_rmcp(server_id: &str, tool: rmcp::model::Tool) -> Option<ToolInfo> {
    let name = tool.name.into_owned();
    if !is_valid_model_tool_name(&model_tool_name(server_id, &name)) {
        log::debug!(
            "Ask AI MCP tool \"{name}\" on \"{server_id}\" dropped: its model-facing name \
             violates the provider tool-name contract (^[a-zA-Z0-9_-]{{1,64}}$)"
        );
        return None;
    }
    Some(ToolInfo {
        name,
        description: tool
            .description
            .map(|description| bound_tool_description(description.into_owned())),
        input_schema: bound_tool_schema(serde_json::Value::Object((*tool.input_schema).clone())),
    })
}

/// Guarantee a tool's params schema is a JSON object (MCP servers always send
/// one, but a malformed/absent schema would otherwise reach the model as a
/// non-object and could break tool declaration). Non-objects become a permissive
/// empty object schema.
fn normalize_schema(schema: serde_json::Value) -> serde_json::Value {
    if schema.is_object() {
        schema
    } else {
        serde_json::json!({ "type": "object", "additionalProperties": true, "properties": {} })
    }
}

/// Assemble the offered tool set for one turn from already-fetched per-server
/// discovery results — for each server, either its `(discovered tools, count of
/// invalid-name drops)` or the connect/list error. Applies curation + the
/// 32-cap (noting a trim), the invalid-name drop note, the `(via <label>)`
/// description prefix, model-facing naming, and schema normalization/bounding.
/// Pure (no AppHandle, no awaits) so the assembly rules are unit-testable; an
/// errored server is skipped, never failing the whole turn.
fn assemble_turn_tools(
    fetched: Vec<(McpServerConfig, Result<(Vec<ToolInfo>, usize), String>)>,
) -> (Vec<ai_engine::AgentTool>, Vec<String>) {
    let mut tools: Vec<ai_engine::AgentTool> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    for (cfg, fetched) in fetched {
        let (discovered, dropped_invalid_names) = match fetched {
            Ok(fetched) => fetched,
            Err(error) => {
                // Connect/list failed within the budget: skip this server this
                // turn (remedy is the enabled toggle; next turn retries).
                log::warn!(
                    "Ask AI MCP connector \"{}\" unavailable this turn: {}",
                    cfg.id,
                    connect_failure_log_head(&error)
                );
                continue;
            }
        };

        if dropped_invalid_names > 0 {
            // Non-silent drop (mirrors the 32-cap trim): log AND one preamble note.
            let line = format!(
                "MCP server \"{}\" has {dropped_invalid_names} tool(s) whose names the model \
                 cannot accept; they are unavailable",
                cfg.label
            );
            log::debug!(
                "Ask AI MCP server \"{}\" dropped {dropped_invalid_names} invalid tool name(s)",
                cfg.id
            );
            notes.push(line);
        }
        let (offered, note) = offered_tools(&discovered, cfg.enabled_tools.as_deref());
        if let Some(note) = note {
            // Non-silent trim: log AND surface one preamble note.
            let line = format!("MCP server \"{}\" {}", cfg.label, note);
            log::debug!("Ask AI MCP server \"{}\" {}", cfg.id, note);
            notes.push(line);
        }
        for tool in offered {
            let description = match tool.description {
                Some(description) => format!("(via {}) {description}", cfg.label),
                None => format!("(via {})", cfg.label),
            };
            tools.push(ai_engine::AgentTool {
                name: model_tool_name(&cfg.id, &tool.name),
                description,
                parameters_schema: bound_tool_schema(normalize_schema(tool.input_schema)),
            });
        }
    }
    (tools, notes)
}

/// Drop every slot whose id is not in `keep` (a reaped slot's child dies on
/// last ref). The pure core of [`McpManager::reconcile`].
fn reap_slots(keep: &HashSet<String>, slots: &mut HashMap<String, Arc<ServerSlot>>) {
    slots.retain(|id, _slot| keep.contains(id));
}

/// The ENABLED MCP servers from current settings (with a non-empty id). Only
/// enabled servers ever connect (ADR 0048); a per-server `enabled` toggle means
/// disabling ≠ deleting.
fn enabled_servers(core: &AppCore) -> Vec<McpServerConfig> {
    super::super::read_ai_runtime_settings(core)
        .mcp_servers
        .into_iter()
        .filter(|cfg| cfg.enabled && !cfg.id.trim().is_empty())
        .collect()
}

/// Persistent MCP connection manager, held in Tauri managed state. Cheap to
/// clone (an `Arc` inside), so the turn builder and the executor closure each
/// hold their own handle.
#[derive(Clone, Default)]
pub struct McpManager {
    inner: Arc<Inner>,
}

#[derive(Default)]
pub(super) struct Inner {
    /// server id → its connection slot. The `tokio::Mutex` guards the map itself;
    /// each slot has its own inner mutex for its connect/redial, so per-server
    /// connects never serialize behind one another.
    pub(super) slots: Mutex<HashMap<String, Arc<ServerSlot>>>,
    /// In-flight browser OAuth flows, keyed by the CSRF `state` param the authorize
    /// URL carries (and the deep-link callback echoes). A plain `std::sync::Mutex`:
    /// the critical sections are tiny, purely in-memory map ops with no `.await`
    /// held across the lock. Evicted on claim, on the TTL sweep at the next
    /// `begin_oauth`, and on disconnect.
    pub(super) oauth_pending: std::sync::Mutex<HashMap<String, PendingOAuth>>,
    /// Connector ids whose stored Token Set failed to refresh at the last
    /// warm-on-open — the *Needs reconnect* signal Settings (slice 6) reads.
    /// Refreshed at warm, cleared on a successful (re)authorization or
    /// disconnect. Carries a per-id generation counter so a warm task that
    /// resolves AFTER a newer authorize/disconnect cannot resurrect the flag
    /// (see [`super::oauth_flow::ReconnectState`]).
    pub(super) oauth_reconnect_needed: std::sync::Mutex<super::oauth_flow::ReconnectState>,
}

/// Lock a `std::sync::Mutex`, recovering the guard from a poisoned lock (a thread
/// panicked while holding it) — these maps carry no invariant a panic could
/// corrupt, so the data stays usable.
pub(super) fn lock_recover<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Inner {
    /// Get the slot for `cfg`, creating it — or replacing an EDITED one whose
    /// connect fingerprint changed — under the map lock. Dropping a replaced
    /// slot's established connection kills its child (last ref).
    async fn slot_for(&self, cfg: &McpServerConfig) -> Arc<ServerSlot> {
        let fingerprint = config_fingerprint(cfg);
        let mut slots = self.slots.lock().await;
        if let Some(existing) = slots.get(&cfg.id) {
            if existing.fingerprint == fingerprint {
                return Arc::clone(existing);
            }
            // Edited: fingerprint changed → fall through to a fresh slot; the
            // `insert` below drops the old one (killing its child on last ref).
        }
        let slot = Arc::new(ServerSlot {
            fingerprint,
            state: Mutex::new(SlotState::default()),
        });
        slots.insert(cfg.id.clone(), Arc::clone(&slot));
        slot
    }
}

impl McpManager {
    /// Borrow the shared [`Inner`] state — the sibling [`super::oauth_flow`] module
    /// reaches the pending-flow map, reconnect-needed set, and slot cache through
    /// this so the OAuth lifecycle can live in its own file.
    pub(super) fn inner(&self) -> &Inner {
        &self.inner
    }

    /// Warm-on-open discovery: for every enabled server, kick off a BACKGROUND
    /// connect + `list_tools` and return immediately. Both chat doors call this on
    /// mount so a turn build usually finds discovery already done (or in-flight,
    /// which it awaits). Nothing here blocks; a failed warm is logged and left for
    /// the turn / next-use retry. Runs only on chat-surface open, never at app
    /// launch (deferred-startup invariant).
    ///
    /// Documented deferral (ADR 0048, same minimalism as the health seam): a warm
    /// task racing a settings save can resurrect a JUST-disabled server — if
    /// `reconcile` reaps the slot first, this task's `slot_for` re-inserts it and
    /// connects a child that lingers (holding its keychain secret) until the next
    /// settings save or app exit. The window is milliseconds at chat-door open,
    /// and `tools_for_turn`/`call_tool` re-filter on `enabled_servers`, so the
    /// stale child is never offered or called — it only idles. Closing it needs a
    /// settings re-read inside each spawned task; do NOT add that until a real
    /// lingering child bothers someone.
    pub fn warm(&self, core: &AppCore) {
        for cfg in enabled_servers(core) {
            let manager = self.clone();
            core.spawn(async move {
                let slot = manager.inner.slot_for(&cfg).await;
                // Capture the reconnect generation BEFORE the connect: this task's
                // verdict is about the token held NOW. If an authorize/disconnect
                // lands while the connect is in flight, it bumps the generation
                // and the guarded write below is dropped — a stale warm failure
                // must not resurrect "Needs reconnect" on a freshly authorized
                // connector.
                let oauth_generation = (cfg.auth_mode == McpAuthMode::OAuth).then(|| {
                    lock_recover(&manager.inner.oauth_reconnect_needed).generation(&cfg.id)
                });
                let ready = slot.ensure_ready(&cfg).await;
                // ponytail: the OAuth "needs reconnect" flag is refreshed HERE, at
                // warm-on-open — not continuously. A connector that HELD a token
                // whose refresh now fails gets flagged; a clean connect clears it.
                // That warm-time refresh is the documented ceiling — slice 6 reads
                // the flag for the status surface, and the transport OAuth branch
                // already surfaces the readable "needs reconnecting" error at turn
                // time. A continuous health probe would be added in the manager's
                // failure seam, not here, and only if dead-token status confuses.
                if let Some(generation) = oauth_generation {
                    use super::oauth_flow::{oauth_token_present, reconnect_flag_update};
                    // Keychain read only on the Err path — Ok clears regardless.
                    let has_token = ready.is_err() && oauth_token_present(&cfg.id);
                    lock_recover(&manager.inner.oauth_reconnect_needed).apply_if_current(
                        &cfg.id,
                        generation,
                        reconnect_flag_update(ready.is_ok(), has_token),
                    );
                }
                if let Err(error) = ready {
                    log::warn!(
                        "Ask AI MCP warm connect for \"{}\" failed (will retry on use): {}",
                        cfg.id,
                        connect_failure_log_head(&error)
                    );
                }
            });
        }
    }

    /// Build the MCP tool set for one turn: for every ENABLED server, await its
    /// in-flight (or start its) discovery within a shared [`DISCOVERY_TURN_BUDGET`]
    /// and, for the servers ready in time, produce the offered tools (curated +
    /// 32-capped) plus any preamble notes. Servers not ready (slow cold boot,
    /// connect error) are skipped this turn — never block the turn forever.
    ///
    /// Returns `(tools, notes)`: the tools carry model-facing `mcp__<id>__<tool>`
    /// names and a "(via <label>)" description prefix; the notes are human lines
    /// for the preamble (the non-silent 32-cap trim).
    pub async fn tools_for_turn(&self, core: &AppCore) -> (Vec<ai_engine::AgentTool>, Vec<String>) {
        let servers = enabled_servers(core);
        if servers.is_empty() {
            return (Vec::new(), Vec::new());
        }
        let deadline = tokio::time::Instant::now() + DISCOVERY_TURN_BUDGET;

        // Await each server's readiness concurrently under ONE overall deadline —
        // a fast server resolves even while a slow one is still booting.
        let ready = join_all(servers.into_iter().map(|cfg| async move {
            let slot = self.inner.slot_for(&cfg).await;
            let ready = tokio::time::timeout_at(deadline, slot.ensure_ready(&cfg)).await;
            (cfg, ready)
        }))
        .await;

        // Peel the AppHandle/await-bound part off here; the assembly rules are
        // pure in `assemble_turn_tools` so they stay unit-testable.
        let fetched = ready
            .into_iter()
            .filter_map(|(cfg, ready)| match ready {
                Ok(result) => Some((
                    cfg,
                    result.map(|established| {
                        (established.tools.clone(), established.dropped_invalid_names)
                    }),
                )),
                Err(_elapsed) => {
                    // Still connecting when the budget expired: proceed without it;
                    // the in-flight connect keeps running for the next turn.
                    log::warn!(
                        "Ask AI MCP connector \"{}\" not ready within {}s; proceeding without it",
                        cfg.id,
                        DISCOVERY_TURN_BUDGET.as_secs()
                    );
                    None
                }
            })
            .collect();
        assemble_turn_tools(fetched)
    }

    /// Execute one MCP tool call. `server_id`/`tool` are the parts the executor
    /// parsed from `mcp__<server_id>__<tool>`; `params` is the model's argument
    /// object. Returns the tool result serialized to a JSON string (truncated to
    /// the result cap), or readable error text the model relays.
    ///
    /// Failure policy (ADR 0048): a call error drops the handle and redials ONCE;
    /// a second consecutive failure returns error text.
    pub async fn call_tool(
        &self,
        core: &AppCore,
        server_id: &str,
        tool: &str,
        params: serde_json::Value,
    ) -> Result<String, String> {
        // Re-resolve the server from settings each call so a just-disabled or
        // just-edited connector is honored (and its config drives the redial).
        let cfg = enabled_servers(core)
            .into_iter()
            .find(|cfg| cfg.id == server_id)
            .ok_or_else(|| {
                format!("MCP server \"{server_id}\" is not enabled (enable it in Settings)")
            })?;
        let slot = self.inner.slot_for(&cfg).await;
        call_with_redial(&slot, &cfg, tool, &params, CALL_TOOL_BUDGET).await
    }

    /// Connect the server ON DEMAND (reusing the app-session cache when it is
    /// already warm) and return its FULL discovered tool list — the backing call
    /// for the "See tool list" curation modal. Unlike [`tools_for_turn`] this
    /// applies NO curation or 32-cap: the modal shows every tool so the user can
    /// pick which to offer. A connect/list failure surfaces as readable error
    /// text the modal renders (with a retry button).
    pub async fn list_server_tools(
        &self,
        core: &AppCore,
        id: &str,
    ) -> Result<Vec<ToolInfo>, String> {
        let cfg = enabled_servers(core)
            .into_iter()
            .find(|cfg| cfg.id == id)
            .ok_or_else(|| {
                format!(
                    "MCP connector \"{id}\" is not enabled — enable it in Settings to list its tools"
                )
            })?;
        let slot = self.inner.slot_for(&cfg).await;
        let established = slot.ensure_ready(&cfg).await?;
        Ok(established.tools.clone())
    }

    /// Reconcile the connection cache against current settings: drop slots for
    /// servers that were REMOVED or DISABLED (kills their child on last ref).
    /// Edited servers (changed connect fingerprint) are reaped lazily at
    /// `slot_for`; this only reaps the ones that should no longer exist at all.
    /// Wired into the AI-runtime settings save (fire-and-forget).
    pub async fn reconcile(&self, core: &AppCore) {
        let keep: HashSet<String> = enabled_servers(core)
            .into_iter()
            .map(|cfg| cfg.id)
            .collect();
        reap_slots(&keep, &mut *self.inner.slots.lock().await);
    }
}

/// One call attempt's failure, classified by whether the request PROVABLY never
/// reached the server. Only `retryable` failures may be redialed — retrying a
/// call the server may have already executed would silently duplicate a write
/// tool's side effects (create issue, send message).
struct CallFailure {
    retryable: bool,
    text: String,
}

/// Classify an rmcp call error for the redial policy. `TransportSend` is the
/// one variant that guarantees the request never left the process (the write
/// to the transport itself failed), so it alone is retryable. Everything else
/// — `Timeout`, `TransportClosed`, `McpError`, `UnexpectedResponse`,
/// `Cancelled`, and any future variant (the enum is `#[non_exhaustive]`) —
/// may have executed server-side and must surface unretried.
fn classify_service_error(error: rmcp::ServiceError) -> CallFailure {
    CallFailure {
        retryable: matches!(error, rmcp::ServiceError::TransportSend(_)),
        text: error.to_string(),
    }
}

/// The failure policy around [`call_once`] (ADR 0048, amended 2026-07-04): a
/// RETRYABLE failure (never reached the server) drops the handle and redials
/// ONCE; a non-retryable failure — or a second consecutive one — surfaces as
/// readable error text the model relays. Split from `call_tool` so tests can
/// drive it without an `AppHandle`. `budget` is [`CALL_TOOL_BUDGET`] in production;
/// injectable so tests need not wait a real 60 s (tokio virtual time cannot drive
/// a real child process).
async fn call_with_redial(
    slot: &ServerSlot,
    cfg: &McpServerConfig,
    tool: &str,
    params: &serde_json::Value,
    budget: Duration,
) -> Result<String, String> {
    // First attempt on the (possibly cached) connection.
    match call_once(slot, cfg, tool, params, budget).await {
        Ok(result) => Ok(result),
        Err(failure) if failure.retryable => {
            // Provably pre-transmit: redial ONCE ( `call_once` already
            // invalidated any cached handle on the way out).
            call_once(slot, cfg, tool, params, budget)
                .await
                .map_err(|second_failure| {
                    // The failure text can be server-controlled and unbounded (an
                    // rmcp `McpError` embeds the server's JSON-RPC `message`/`data`
                    // verbatim), so bound it with the SAME cap as a successful
                    // result before it is streamed to the model as a tool result.
                    truncate_tool_result(format!(
                        "MCP server \"{}\" failed this tool call twice: {}",
                        cfg.label, second_failure.text
                    ))
                })
        }
        // Bound as above: a non-retryable failure's text is likewise server-
        // controlled and unbounded, and reaches the model as the tool result.
        Err(failure) => Err(truncate_tool_result(format!(
            "MCP server \"{}\" tool call failed: {}",
            cfg.label, failure.text
        ))),
    }
}

/// One connect-then-call attempt. A connect failure surfaces as a RETRYABLE
/// `Err` (the request was never formed); a call failure or `budget` elapse
/// invalidates the slot (so a redial dials fresh) and surfaces classified. On
/// success the result is serialized and truncated to the result cap.
async fn call_once(
    slot: &ServerSlot,
    cfg: &McpServerConfig,
    tool: &str,
    params: &serde_json::Value,
    budget: Duration,
) -> Result<String, CallFailure> {
    let established = slot.ensure_ready(cfg).await.map_err(|text| CallFailure {
        retryable: true,
        text,
    })?;

    let arguments = match params {
        serde_json::Value::Object(map) => Some(map.clone()),
        // A null / non-object payload is treated as no arguments.
        _ => None,
    };
    let mut request = rmcp::model::CallToolRequestParams::new(tool.to_string());
    if let Some(arguments) = arguments {
        request = request.with_arguments(arguments);
    }

    match tokio::time::timeout(budget, established.client.call_tool(request)).await {
        Ok(Ok(result)) => {
            let reported_error = result.is_error == Some(true);
            let json = serde_json::to_string(&result).map_err(|error| CallFailure {
                retryable: false,
                text: format!("failed to serialize MCP tool result: {error}"),
            })?;
            let body = truncate_tool_result(json);
            // `is_error` serializes AFTER `content`, so a tool-level (in-band)
            // error whose content exceeds the result cap would have its
            // `"isError":true` flag truncated away — the model would then read a
            // tool error as a success. Hoist an explicit marker that survives
            // truncation so the error signal always reaches the model.
            Ok(if reported_error {
                format!("[MCP tool reported an error] {body}")
            } else {
                body
            })
        }
        Ok(Err(error)) => {
            // Drop the handle so any redial dials a fresh connection.
            slot.invalidate().await;
            Err(classify_service_error(error))
        }
        Err(_elapsed) => {
            // The abandoned in-flight call leaves the shared connection in an
            // unknown state — drop it so the next call dials fresh. NON-retryable:
            // the call may have executed server-side, so a redial could duplicate
            // a write (ADR 0048 documents this residual risk).
            slot.invalidate().await;
            Err(CallFailure {
                retryable: false,
                text: format!(
                    "tool call \"{tool}\" on \"{}\" timed out after {} s",
                    cfg.label,
                    budget.as_secs(),
                ),
            })
        }
    }
}

/// Fire-and-forget warm-on-open discovery for enabled MCP connectors. Returns
/// immediately; the actual connects run in the background (spawned per server).
/// Both chat doors call this on mount.
pub async fn mcp_warm_connectors(core: &AppCore) -> Result<(), String> {
    core.state::<McpManager>().warm(core);
    Ok(())
}

/// One tool as the curation modal needs it — just name + description. The input
/// schema is deliberately omitted (the modal only lists + toggles, and schemas
/// can be large). camelCase to match the frontend `McpToolDescriptor` mirror.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolDescriptor {
    pub name: String,
    pub description: Option<String>,
}

/// On-demand connect + list backing the "See tool list" curation modal. Connects
/// the named server (reusing the app-session cache when already warm — an npx
/// cold boot can otherwise take seconds, hence the modal's spinner), lists its
/// tools, and returns name+description for each. A connect/list failure (or a
/// disabled/absent server) surfaces as a readable error string the modal renders
/// with a retry button.
pub async fn mcp_list_server_tools(
    core: &AppCore,
    id: String,
) -> Result<Vec<McpToolDescriptor>, String> {
    // Clone the manager out of managed state so the (possibly slow) connect await
    // does not hold the `State` guard.
    let manager = (*core.state::<McpManager>()).clone();
    let tools = manager.list_server_tools(core, &id).await?;
    Ok(tools
        .into_iter()
        .map(|tool| McpToolDescriptor {
            name: tool.name,
            description: tool.description,
        })
        .collect())
}

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "manager_tests_bounds.rs"]
mod mcp_error_result_bounds_review_security_b;

#[cfg(test)]
#[path = "manager_tests_truncation.rs"]
mod is_error_truncation_review_logic_b;
