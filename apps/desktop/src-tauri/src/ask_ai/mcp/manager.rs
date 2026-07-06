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
use tauri::Manager;
use tokio::sync::Mutex;

use super::oauth_flow::PendingOAuth;
use super::transport::{config_fingerprint, connect, McpClient};
use super::{
    bound_tool_description, bound_tool_schema, is_valid_model_tool_name, model_tool_name,
    offered_tools, truncate_tool_result, ToolInfo,
};

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

/// Project rmcp's `Tool` onto our trimmed [`ToolInfo`] (name, description,
/// schema) — or DROP it (`None`) when its model-facing `mcp__<id>__<name>` would
/// violate the provider tool-name contract (`^[a-zA-Z0-9_-]{1,64}$`): providers
/// reject the ENTIRE request over one bad name, killing every tool for the turn.
/// Never truncate — a rewritten name would no longer route (`parse_mcp_tool_name`).
fn tool_info_from_rmcp(server_id: &str, tool: rmcp::model::Tool) -> Option<ToolInfo> {
    let name = tool.name.into_owned();
    if !is_valid_model_tool_name(&model_tool_name(server_id, &name)) {
        tauri_plugin_log::log::info!(
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
                tauri_plugin_log::log::warn!(
                    "Ask AI MCP connector \"{}\" unavailable this turn: {error}",
                    cfg.label
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
            tauri_plugin_log::log::info!("Ask AI {line}");
            notes.push(line);
        }
        let (offered, note) = offered_tools(&discovered, cfg.enabled_tools.as_deref());
        if let Some(note) = note {
            // Non-silent trim: log AND surface one preamble note.
            let line = format!("MCP server \"{}\" {}", cfg.label, note);
            tauri_plugin_log::log::info!("Ask AI {line}");
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
fn enabled_servers(app_handle: &tauri::AppHandle) -> Vec<McpServerConfig> {
    super::super::read_ai_runtime_settings(app_handle)
        .mcp_servers
        .into_iter()
        .filter(|cfg| cfg.enabled && !cfg.id.trim().is_empty())
        .collect()
}

/// Persistent MCP connection manager, held in Tauri managed state. Cheap to
/// clone (an `Arc` inside), so the turn builder and the executor closure each
/// hold their own handle.
#[derive(Clone, Default)]
pub(crate) struct McpManager {
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
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
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
    pub(crate) fn warm(&self, app_handle: &tauri::AppHandle) {
        for cfg in enabled_servers(app_handle) {
            let manager = self.clone();
            tauri::async_runtime::spawn(async move {
                let slot = manager.inner.slot_for(&cfg).await;
                // Capture the reconnect generation BEFORE the connect: this task's
                // verdict is about the token held NOW. If an authorize/disconnect
                // lands while the connect is in flight, it bumps the generation
                // and the guarded write below is dropped — a stale warm failure
                // must not resurrect "Needs reconnect" on a freshly authorized
                // connector.
                let oauth_generation = (cfg.auth_mode == McpAuthMode::OAuth)
                    .then(|| lock_recover(&manager.inner.oauth_reconnect_needed).generation(&cfg.id));
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
                    tauri_plugin_log::log::info!(
                        "Ask AI MCP warm connect for \"{}\" failed (will retry on use): {error}",
                        cfg.label
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
    pub(crate) async fn tools_for_turn(
        &self,
        app_handle: &tauri::AppHandle,
    ) -> (Vec<ai_engine::AgentTool>, Vec<String>) {
        let servers = enabled_servers(app_handle);
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
                    tauri_plugin_log::log::info!(
                        "Ask AI MCP connector \"{}\" not ready within {}s; proceeding without it",
                        cfg.label,
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
    pub(crate) async fn call_tool(
        &self,
        app_handle: &tauri::AppHandle,
        server_id: &str,
        tool: &str,
        params: serde_json::Value,
    ) -> Result<String, String> {
        // Re-resolve the server from settings each call so a just-disabled or
        // just-edited connector is honored (and its config drives the redial).
        let cfg = enabled_servers(app_handle)
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
    pub(crate) async fn list_server_tools(
        &self,
        app_handle: &tauri::AppHandle,
        id: &str,
    ) -> Result<Vec<ToolInfo>, String> {
        let cfg = enabled_servers(app_handle)
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
    pub(crate) async fn reconcile(&self, app_handle: &tauri::AppHandle) {
        let keep: HashSet<String> = enabled_servers(app_handle)
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
#[tauri::command]
pub async fn mcp_warm_connectors(app_handle: tauri::AppHandle) -> Result<(), String> {
    app_handle.state::<McpManager>().warm(&app_handle);
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
#[tauri::command]
pub async fn mcp_list_server_tools(
    app_handle: tauri::AppHandle,
    id: String,
) -> Result<Vec<McpToolDescriptor>, String> {
    // Clone the manager out of managed state so the (possibly slow) connect await
    // does not hold the `State` guard.
    let manager = (*app_handle.state::<McpManager>()).clone();
    let tools = manager.list_server_tools(&app_handle, &id).await?;
    Ok(tools
        .into_iter()
        .map(|tool| McpToolDescriptor {
            name: tool.name,
            description: tool.description,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use capture_types::McpTransport;

    /// A stdio server config for tests: enabled, no env/secret/curation.
    fn stdio_cfg(id: &str, label: &str, command: &str, args: Vec<String>) -> McpServerConfig {
        McpServerConfig {
            id: id.to_string(),
            label: label.to_string(),
            enabled: true,
            transport: McpTransport::Stdio,
            auth_mode: capture_types::McpAuthMode::Bearer,
            command: Some(command.to_string()),
            args,
            env: Vec::new(),
            url: None,
            secret_env_name: None,
            enabled_tools: None,
        }
    }

    /// A fresh slot for `cfg`, as `slot_for` would build it.
    fn test_slot(cfg: &McpServerConfig) -> ServerSlot {
        ServerSlot {
            fingerprint: config_fingerprint(cfg),
            state: Mutex::new(SlotState::default()),
        }
    }

    /// A malicious/compromised MCP server ships a tool whose `inputSchema` is a
    /// multi-megabyte object (padding + `description` fields the model reads).
    /// `tool_info_from_rmcp` is the ONE projection of an rmcp `Tool` onto the
    /// model-facing `ToolInfo`; its schema is streamed to the model as the tool's
    /// parameter schema every turn. It caps the tool DESCRIPTION but must cap the
    /// SCHEMA too, or one rogue server floods the turn (and opens an unbounded
    /// prompt-injection channel via schema text) — the same INV-B5 bound the
    /// result/description caps enforce.
    #[test]
    fn tool_info_from_rmcp_bounds_a_giant_server_schema() {
        let mut schema = serde_json::Map::new();
        schema.insert("type".to_string(), serde_json::json!("object"));
        schema.insert(
            "description".to_string(),
            serde_json::json!("z".repeat(1_000_000)),
        );
        let tool = rmcp::model::Tool::new("echo", "does things", std::sync::Arc::new(schema));
        let info = tool_info_from_rmcp("srv", tool).expect("a valid tool name must survive");
        let schema_len = serde_json::to_string(&info.input_schema)
            .expect("schema serializes")
            .len();
        assert!(
            schema_len <= 20_000,
            "a server-controlled input schema reached the model unbounded ({schema_len} chars) — \
             it must be capped like the tool result and description (INV-B5)"
        );
    }

    /// A server tool name the provider would reject (a dot, or `mcp__<id>__<name>`
    /// over 64 chars) must be DROPPED at discovery — offered anyway, the provider
    /// rejects the ENTIRE request and every tool vanishes for the turn.
    #[test]
    fn tool_info_from_rmcp_drops_provider_invalid_tool_names() {
        let schema = || std::sync::Arc::new(serde_json::Map::new());
        let dotted = rmcp::model::Tool::new("list.files", "dotted", schema());
        assert!(tool_info_from_rmcp("srv", dotted).is_none());
        // `mcp__srv__` + 55 chars = 65 > 64.
        let long = rmcp::model::Tool::new("t".repeat(55), "long", schema());
        assert!(tool_info_from_rmcp("srv", long).is_none());
        let valid = rmcp::model::Tool::new("list_files", "fine", schema());
        assert!(tool_info_from_rmcp("srv", valid).is_some());
    }

    // ---- assemble_turn_tools: the pure per-turn assembly rules ----

    /// A discovered tool as `connect_and_list` would cache it.
    fn discovered_tool(name: &str) -> ToolInfo {
        ToolInfo {
            name: name.to_string(),
            description: Some(format!("does {name}")),
            input_schema: serde_json::json!({ "type": "object" }),
        }
    }

    /// One bad connector must cost only its own tools, never the turn.
    #[test]
    fn assemble_skips_an_errored_server_without_failing_the_turn() {
        let fetched = vec![
            (
                stdio_cfg("bad", "Bad", "sh", Vec::new()),
                Err("connection refused".to_string()),
            ),
            (
                stdio_cfg("good", "Good", "sh", Vec::new()),
                Ok((vec![discovered_tool("echo")], 0)),
            ),
        ];
        let (tools, notes) = assemble_turn_tools(fetched);
        assert_eq!(tools.len(), 1, "the healthy server's tools must survive");
        assert_eq!(tools[0].name, "mcp__good__echo");
        assert!(notes.is_empty(), "an errored server is skipped, not noted");
    }

    /// The default tool budget: an uncurated server over the 32-cap is trimmed
    /// to the first 32 in server order, and the trim is NON-silent (one note).
    #[test]
    fn assemble_caps_an_uncurated_server_and_notes_the_trim() {
        let discovered: Vec<ToolInfo> = (0..40)
            .map(|i| discovered_tool(&format!("t{i}")))
            .collect();
        let fetched = vec![(stdio_cfg("big", "Big", "sh", Vec::new()), Ok((discovered, 0)))];
        let (tools, notes) = assemble_turn_tools(fetched);
        assert_eq!(tools.len(), 32);
        assert_eq!(tools[0].name, "mcp__big__t0");
        assert_eq!(tools[31].name, "mcp__big__t31");
        assert_eq!(notes.len(), 1, "the trim must surface exactly one note");
        assert!(notes[0].contains("\"Big\""), "note names the server: {}", notes[0]);
        assert!(notes[0].contains("40") && notes[0].contains("32"));
    }

    /// Offered tools carry the model-facing `mcp__<id>__<name>` and a
    /// `(via <label>)` description prefix (with or without a server description).
    #[test]
    fn assemble_applies_model_naming_and_via_label_descriptions() {
        let mut undescribed = discovered_tool("bare");
        undescribed.description = None;
        let fetched = vec![(
            stdio_cfg("github-2", "GitHub", "sh", Vec::new()),
            Ok((vec![discovered_tool("create_issue"), undescribed], 0)),
        )];
        let (tools, _notes) = assemble_turn_tools(fetched);
        assert_eq!(tools[0].name, "mcp__github-2__create_issue");
        assert_eq!(tools[0].description, "(via GitHub) does create_issue");
        assert_eq!(tools[1].description, "(via GitHub)");
    }

    /// Offered schemas are normalized (a non-object becomes a permissive object)
    /// and bounded (a giant object is replaced) before reaching the model.
    #[test]
    fn assemble_normalizes_and_bounds_offered_schemas() {
        let mut non_object = discovered_tool("a");
        non_object.input_schema = serde_json::json!("not an object");
        let mut giant = discovered_tool("b");
        giant.input_schema =
            serde_json::json!({ "type": "object", "description": "z".repeat(50_000) });
        let fetched = vec![(
            stdio_cfg("srv", "Srv", "sh", Vec::new()),
            Ok((vec![non_object, giant], 0)),
        )];
        let (tools, _notes) = assemble_turn_tools(fetched);
        assert!(tools[0].parameters_schema.is_object(), "non-object must normalize");
        let bounded = serde_json::to_string(&tools[1].parameters_schema).expect("serializes");
        assert!(bounded.len() <= 20_000, "giant schema must be bounded: {} chars", bounded.len());
    }

    /// Invalid-name drops recorded at discovery surface as one preamble note per
    /// turn — non-silent, mirroring the 32-cap trim.
    #[test]
    fn assemble_notes_invalid_name_drops_from_discovery() {
        let fetched = vec![(
            stdio_cfg("srv", "Srv", "sh", Vec::new()),
            Ok((vec![discovered_tool("ok")], 2)),
        )];
        let (tools, notes) = assemble_turn_tools(fetched);
        assert_eq!(tools.len(), 1, "surviving tools are still offered");
        assert_eq!(notes.len(), 1, "the drop must surface exactly one note");
        assert!(notes[0].contains("\"Srv\"") && notes[0].contains('2'), "{}", notes[0]);
    }

    /// `reap_slots` (the pure core of `reconcile`) retains only enabled ids;
    /// a reaped slot's child dies with its last ref.
    #[test]
    fn reap_slots_retains_only_enabled_ids() {
        let keep_cfg = stdio_cfg("keep", "Keep", "sh", Vec::new());
        let gone_cfg = stdio_cfg("gone", "Gone", "sh", Vec::new());
        let mut slots = HashMap::new();
        slots.insert("keep".to_string(), Arc::new(test_slot(&keep_cfg)));
        slots.insert("gone".to_string(), Arc::new(test_slot(&gone_cfg)));
        let keep: HashSet<String> = ["keep".to_string()].into();
        reap_slots(&keep, &mut slots);
        assert_eq!(slots.len(), 1);
        assert!(slots.contains_key("keep"));
    }

    /// `slot_for` is the slot cache's whole policy: an unchanged config must
    /// REUSE the slot (no needless redial mid-session); a connect-relevant edit
    /// must REPLACE it (the old child dies on last ref, next use dials fresh).
    #[tokio::test]
    async fn slot_for_reuses_an_unchanged_config_and_replaces_an_edited_one() {
        let inner = Inner::default();
        let cfg = stdio_cfg("srv", "Srv", "sh", Vec::new());
        let first = inner.slot_for(&cfg).await;
        let second = inner.slot_for(&cfg).await;
        assert!(Arc::ptr_eq(&first, &second), "same config must reuse the slot");

        let mut edited = cfg.clone();
        edited.args.push("--flag".to_string());
        let third = inner.slot_for(&edited).await;
        assert!(!Arc::ptr_eq(&second, &third), "an edited config must replace the slot");
        let cached = Arc::clone(inner.slots.lock().await.get("srv").expect("slot present"));
        assert!(Arc::ptr_eq(&cached, &third), "the map must hold the NEW slot");
    }

    /// The bounded second-failure arm of the redial policy: a server that fails
    /// twice in a row (here: a command that cannot even spawn) must surface the
    /// readable "failed twice" error, not retry forever.
    #[tokio::test]
    async fn a_connect_that_fails_twice_surfaces_the_failed_twice_error() {
        let cfg = stdio_cfg(
            "test-gone-mcp-server",
            "Gone",
            "/nonexistent/mnema-mcp-no-such-binary",
            Vec::new(),
        );
        let slot = test_slot(&cfg);
        let result = tokio::time::timeout(
            Duration::from_secs(20),
            call_with_redial(&slot, &cfg, "echo", &serde_json::json!({}), CALL_TOOL_BUDGET),
        )
        .await
        .expect("a spawn failure must fail fast, not hang");
        let error = result.expect_err("an unspawnable server must fail the call");
        assert!(
            error.contains("failed this tool call twice"),
            "the redial's second failure must surface the bounded twice-failed arm: {error}"
        );
    }

    /// A unique scratch dir for one test's fixture signal files.
    fn fixture_dir(prefix: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create fixture dir");
        dir
    }

    /// `warm` and `list_server_tools` call `ensure_ready` with no timeout of their
    /// own, and `ensure_ready` holds the per-slot lock across the connect. A server
    /// that hangs during initialize must therefore be bounded here, or it poisons
    /// the slot forever (leaked child/task) and taxes every later turn.
    #[tokio::test]
    async fn ensure_ready_is_bounded_on_a_hung_server() {
        // `sleep` ignores stdin and never writes stdout, so the MCP initialize
        // handshake never completes (rmcp's `serve_client` has no internal timeout).
        let cfg = stdio_cfg("test-hung-mcp-server", "Hung", "sleep", vec!["60".to_string()]);
        let slot = test_slot(&cfg);
        // Beyond the discovery budget: a bounded `ensure_ready` returns an error
        // inside its budget; an unbounded one never returns and this outer window
        // elapses.
        let outcome = tokio::time::timeout(
            DISCOVERY_TURN_BUDGET + Duration::from_secs(3),
            slot.ensure_ready(&cfg),
        )
        .await;
        assert!(
            matches!(outcome, Ok(Err(_))),
            "ensure_ready on a hung server must return a bounded error, not hang"
        );
    }

    // Classification is the whole retry policy: `TransportSend` is the one
    // variant that provably never reached the server. `McpError`, `Timeout`,
    // `TransportClosed`, `UnexpectedResponse`, and `Cancelled` are all
    // constructible in rmcp 2.1.0 (`DynamicTransportError::from_parts` exists
    // for exactly this), so every variant is covered.

    fn transport_send_error() -> rmcp::ServiceError {
        rmcp::ServiceError::TransportSend(rmcp::transport::DynamicTransportError::from_parts(
            "test",
            std::any::TypeId::of::<()>(),
            Box::new(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pipe")),
        ))
    }

    #[test]
    fn classify_transport_send_is_retryable() {
        let failure = classify_service_error(transport_send_error());
        assert!(failure.retryable, "a failed transport write never reached the server");
        assert!(failure.text.contains("pipe"), "text must stay readable: {}", failure.text);
    }

    #[test]
    fn classify_post_send_errors_are_not_retryable() {
        let post_send = [
            rmcp::ServiceError::TransportClosed,
            rmcp::ServiceError::Timeout {
                timeout: Duration::from_secs(60),
            },
            rmcp::ServiceError::McpError(rmcp::model::ErrorData::new(
                rmcp::model::ErrorCode::INTERNAL_ERROR,
                "boom",
                None,
            )),
            rmcp::ServiceError::UnexpectedResponse,
            rmcp::ServiceError::Cancelled {
                reason: Some("test".to_string()),
            },
        ];
        for error in post_send {
            let text = error.to_string();
            let failure = classify_service_error(error);
            assert!(
                !failure.retryable,
                "{text}: may have executed server-side; a redial could duplicate side effects"
            );
            assert_eq!(failure.text, text, "error text must surface unchanged");
        }
    }

    /// A scripted stdio MCP server for the retryable-path test. It speaks just
    /// enough newline-delimited JSON-RPC for rmcp's client: answers
    /// `initialize` and `tools/list` (ignoring `notifications/initialized`).
    /// Run 1 then closes its STDIN but keeps running (stdout open), so the
    /// client's next send fails at the transport write — rmcp's service loop is
    /// still alive and reports `TransportSend`, the provably-pre-transmit
    /// failure. (Exiting instead would kill the loop via stdout EOF and yield
    /// `TransportClosed`, the non-retryable ambiguous case.) Run 2 is a healthy
    /// server that answers `tools/call`. Each startup appends to a counter file.
    fn retry_fixture_script(counter: &std::path::Path, stdin_closed: &std::path::Path) -> String {
        format!(
            r#"echo run >> "{counter}"
RUN=$(wc -l < "{counter}")
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '%s\n' '{{"jsonrpc":"2.0","id":'"$id"',"result":{{"protocolVersion":"2025-06-18","capabilities":{{"tools":{{}}}},"serverInfo":{{"name":"fixture","version":"0"}}}}}}'
      ;;
    *'"method":"tools/list"'*)
      printf '%s\n' '{{"jsonrpc":"2.0","id":'"$id"',"result":{{"tools":[{{"name":"echo","inputSchema":{{"type":"object"}}}}]}}}}'
      if [ "$RUN" -eq 1 ]; then
        exec 0<&-
        : > "{stdin_closed}"
        exec sleep 60
      fi
      ;;
    *'"method":"tools/call"'*)
      printf '%s\n' '{{"jsonrpc":"2.0","id":'"$id"',"result":{{"content":[{{"type":"text","text":"ok"}}],"isError":false}}}}'
      ;;
  esac
done"#,
            counter = counter.display(),
            stdin_closed = stdin_closed.display(),
        )
    }

    /// The retryable path end to end: a transport-write failure (request never
    /// transmitted) must redial exactly once and succeed on the fresh
    /// connection. The counter file proves the second connect happened.
    #[tokio::test]
    async fn transport_send_failure_redials_once() {
        let dir = fixture_dir("mnema-mcp-retry-test");
        let counter = dir.join("connects");
        let stdin_closed = dir.join("stdin-closed");
        let cfg = stdio_cfg(
            "test-retry-mcp-server",
            "Retry",
            "sh",
            vec![
                "-c".to_string(),
                retry_fixture_script(&counter, &stdin_closed),
            ],
        );
        let slot = test_slot(&cfg);

        // Warm the connection on run 1, then wait until the fixture has
        // PROVABLY closed its stdin — only then is the next write guaranteed to
        // fail pre-transmit rather than land in the pipe and hang.
        slot.ensure_ready(&cfg)
            .await
            .expect("run 1 handshake must succeed");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !stdin_closed.exists() {
            assert!(tokio::time::Instant::now() < deadline, "fixture never signaled stdin close");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        let result = tokio::time::timeout(
            Duration::from_secs(20),
            call_with_redial(&slot, &cfg, "echo", &serde_json::json!({}), CALL_TOOL_BUDGET),
        )
        .await
        .expect("call must not hang");
        assert!(result.is_ok(), "redial onto the healthy run-2 server must succeed: {result:?}");
        assert!(result.unwrap().contains("ok"), "run 2's tool result must come back");

        let connects = std::fs::read_to_string(&counter).expect("counter file");
        assert_eq!(connects.lines().count(), 2, "exactly one redial: two connects total");

        drop(slot); // kills run 2's child; run 1's died with its handle
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A scripted stdio MCP server (same framing as [`retry_fixture_script`])
    /// that answers the handshake + `tools/list`, then STALLS on `tools/call`:
    /// appends the invocation to a counter file and execs into a `sleep` —
    /// stdout stays open, no response ever comes, the call await hangs.
    fn stall_fixture_script(calls: &std::path::Path) -> String {
        format!(
            r#"while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '%s\n' '{{"jsonrpc":"2.0","id":'"$id"',"result":{{"protocolVersion":"2025-06-18","capabilities":{{"tools":{{}}}},"serverInfo":{{"name":"fixture","version":"0"}}}}}}'
      ;;
    *'"method":"tools/list"'*)
      printf '%s\n' '{{"jsonrpc":"2.0","id":'"$id"',"result":{{"tools":[{{"name":"echo","inputSchema":{{"type":"object"}}}}]}}}}'
      ;;
    *'"method":"tools/call"'*)
      echo call >> "{calls}"
      exec sleep 60
      ;;
  esac
done"#,
            calls = calls.display(),
        )
    }

    /// The execution budget end to end: a server that hangs on `tools/call`
    /// must yield a bounded, NON-retried error and leave the slot invalidated.
    /// The counter file proves exactly one `tools/call` reached the fixture (a
    /// timed-out call may have executed server-side, so a retry could duplicate
    /// a write — ADR 0048); the emptied slot proves the next call dials fresh.
    /// The budget is injected small: a real [`CALL_TOOL_BUDGET`] would take a
    /// minute, and tokio virtual time cannot bound a real child process
    /// (auto-advance races past real I/O, spuriously elapsing the connect too).
    #[tokio::test]
    async fn hung_tool_call_is_bounded_not_retried_and_invalidates_the_slot() {
        let dir = fixture_dir("mnema-mcp-stall-test");
        let calls = dir.join("calls");
        let cfg = stdio_cfg(
            "test-stall-mcp-server",
            "Stall",
            "sh",
            vec!["-c".to_string(), stall_fixture_script(&calls)],
        );
        let slot = test_slot(&cfg);

        let budget = Duration::from_secs(2);
        let result = tokio::time::timeout(
            budget + Duration::from_secs(8), // grace covers the (fast) connect
            call_with_redial(&slot, &cfg, "echo", &serde_json::json!({}), budget),
        )
        .await
        .expect("a hung tools/call must be bounded by the budget, not hang");

        let error = result.expect_err("a timed-out call must surface as an error");
        assert!(error.contains("timed out after 2 s"), "readable timeout text expected: {error}");
        let calls_seen = std::fs::read_to_string(&calls).expect("counter file");
        assert_eq!(calls_seen.lines().count(), 1, "a timed-out call must NOT be retried");
        assert!(
            slot.state.lock().await.established.is_none(),
            "timeout must invalidate the slot so the next call dials fresh"
        );

        // The stalled fixture already died with its handle (invalidate + return
        // dropped the last ref); only the scratch dir is left to clean.
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod mcp_error_result_bounds_review_security_b {
    use super::*;
    use capture_types::{McpEnvVar, McpServerConfig, McpTransport};

    fn fixture_dir(prefix: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create fixture dir");
        dir
    }

    /// A scripted stdio MCP server that answers the handshake + `tools/list`,
    /// then on every `tools/call` returns a JSON-RPC ERROR whose `message` is a
    /// giant server-controlled string (passed in via the `MNEMA_BIG_ERROR` env).
    /// This is the "error result from the server" that the 24k result cap must
    /// bound before it is streamed back to the model as a tool result.
    const HUGE_ERROR_SCRIPT: &str = r#"while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '%s\n' '{"jsonrpc":"2.0","id":'"$id"',"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"0"}}}'
      ;;
    *'"method":"tools/list"'*)
      printf '%s\n' '{"jsonrpc":"2.0","id":'"$id"',"result":{"tools":[{"name":"echo","inputSchema":{"type":"object"}}]}}'
      ;;
    *'"method":"tools/call"'*)
      printf '%s\n' '{"jsonrpc":"2.0","id":'"$id"',"error":{"code":-32000,"message":"'"$MNEMA_BIG_ERROR"'"}}'
      ;;
  esac
done"#;

    /// A malicious/compromised MCP server answers `tools/call` with a JSON-RPC
    /// error carrying a multi-ten-kilobyte `message`. That server-controlled
    /// error text is streamed to the model as the tool result, so it MUST be
    /// bounded by the same 24k cap as a successful result — otherwise one rogue
    /// server floods the turn (and opens an unbounded prompt-injection channel)
    /// on every failed call.
    #[tokio::test]
    async fn a_giant_server_error_result_is_bounded_before_reaching_the_model() {
        let dir = fixture_dir("mnema-mcp-huge-error");
        let big = "x".repeat(60_000);
        let cfg = McpServerConfig {
            id: "test-huge-error-mcp-server".to_string(),
            label: "HugeError".to_string(),
            enabled: true,
            transport: McpTransport::Stdio,
            auth_mode: capture_types::McpAuthMode::Bearer,
            command: Some("sh".to_string()),
            args: vec!["-c".to_string(), HUGE_ERROR_SCRIPT.to_string()],
            env: vec![McpEnvVar {
                name: "MNEMA_BIG_ERROR".to_string(),
                value: big,
            }],
            url: None,
            secret_env_name: None,
            enabled_tools: None,
        };
        let slot = ServerSlot {
            fingerprint: config_fingerprint(&cfg),
            state: Mutex::new(SlotState::default()),
        };

        let result = tokio::time::timeout(
            Duration::from_secs(20),
            call_with_redial(&slot, &cfg, "echo", &serde_json::json!({}), CALL_TOOL_BUDGET),
        )
        .await
        .expect("the call must not hang");

        let error = result.expect_err("a server JSON-RPC error must surface as an Err");
        assert!(
            error.chars().count() <= 24_600,
            "a server-controlled error result reached the model unbounded ({} chars) — the 24k \
             result cap must bound error results too, not only successful ones",
            error.chars().count()
        );

        drop(slot);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod is_error_truncation_review_logic_b {
    use super::*;
    use capture_types::{McpEnvVar, McpServerConfig, McpTransport};

    fn fixture_dir(prefix: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create fixture dir");
        dir
    }

    /// A scripted stdio MCP server that answers the handshake + `tools/list`,
    /// then on `tools/call` returns a SUCCESSFUL JSON-RPC result (not a JSON-RPC
    /// error) carrying `isError:true` and a huge `content` text (injected via the
    /// `MNEMA_BIG_CONTENT` env). This is an in-band tool-level error: rmcp yields
    /// `Ok(CallToolResult { is_error: Some(true), .. })`. Because `content`
    /// serializes BEFORE `isError`, once the content passes the 24k result cap the
    /// `"isError":true` flag is exactly what gets truncated away.
    const BIG_INBAND_ERROR_SCRIPT: &str = r#"while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '%s\n' '{"jsonrpc":"2.0","id":'"$id"',"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"0"}}}'
      ;;
    *'"method":"tools/list"'*)
      printf '%s\n' '{"jsonrpc":"2.0","id":'"$id"',"result":{"tools":[{"name":"echo","inputSchema":{"type":"object"}}]}}'
      ;;
    *'"method":"tools/call"'*)
      printf '%s\n' '{"jsonrpc":"2.0","id":'"$id"',"result":{"content":[{"type":"text","text":"'"$MNEMA_BIG_CONTENT"'"}],"isError":true}}'
      ;;
  esac
done"#;

    /// A tool that returns an in-band error (`isError:true`) whose content
    /// exceeds the 24k result cap must still tell the model it was an error.
    /// Because `content` serializes before `isError`, truncating the serialized
    /// result string drops the flag and the (truncated) error content reaches the
    /// model as an apparent success — poisoning the turn.
    #[tokio::test]
    async fn large_in_band_error_result_keeps_its_error_signal() {
        let dir = fixture_dir("mnema-mcp-inband-error");
        let big = "x".repeat(60_000);
        let cfg = McpServerConfig {
            id: "test-inband-error-mcp-server".to_string(),
            label: "InbandError".to_string(),
            enabled: true,
            transport: McpTransport::Stdio,
            auth_mode: capture_types::McpAuthMode::Bearer,
            command: Some("sh".to_string()),
            args: vec!["-c".to_string(), BIG_INBAND_ERROR_SCRIPT.to_string()],
            env: vec![McpEnvVar {
                name: "MNEMA_BIG_CONTENT".to_string(),
                value: big,
            }],
            url: None,
            secret_env_name: None,
            enabled_tools: None,
        };
        let slot = ServerSlot {
            fingerprint: config_fingerprint(&cfg),
            state: Mutex::new(SlotState::default()),
        };

        let result = tokio::time::timeout(
            Duration::from_secs(20),
            call_with_redial(&slot, &cfg, "echo", &serde_json::json!({}), CALL_TOOL_BUDGET),
        )
        .await
        .expect("the call must not hang")
        .expect("an in-band error result is a successful transport response (Ok)");

        assert!(
            result.to_lowercase().contains("error"),
            "a truncated in-band error result must still signal the error to the \
             model, else a tool error is read as a success; got prefix: {}",
            &result.chars().take(80).collect::<String>()
        );

        drop(slot);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
