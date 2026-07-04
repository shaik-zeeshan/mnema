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
//! its child via the child transport's `Drop` (see [`McpClient`]); this is
//! macOS-only verified on this branch (SUPPORTS.md).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use capture_types::McpServerConfig;
use futures_util::future::join_all;
use tauri::Manager;
use tokio::sync::Mutex;

use super::transport::{config_fingerprint, connect, McpClient};
use super::{
    bound_tool_description, model_tool_name, offered_tools, truncate_tool_result, ToolInfo,
};

/// Total budget a turn build waits for in-flight discovery before proceeding
/// with only the servers that are ready — npx cold boot can be slow, and a turn
/// must never block forever on a connector.
const DISCOVERY_TURN_BUDGET: Duration = Duration::from_secs(15);

/// One server's live connection plus the tools it advertised at connect. Held
/// behind an `Arc` so a mid-call clone keeps the child alive even if the slot is
/// invalidated concurrently.
struct Established {
    client: McpClient,
    tools: Vec<ToolInfo>,
}

/// Per-server connection slot, keyed by instance id. Its inner `Mutex` serializes
/// connect/redial AND caches the established connection, so a second caller
/// (warm vs. turn) that arrives mid-connect simply AWAITS the in-flight connect
/// on the lock — that is how "await in-flight discovery" is realized without a
/// separate task handle.
struct ServerSlot {
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
    let tools = tools.into_iter().map(tool_info_from_rmcp).collect();
    Ok(Established { client, tools })
}

/// Project rmcp's `Tool` onto our trimmed [`ToolInfo`] (name, description, schema).
fn tool_info_from_rmcp(tool: rmcp::model::Tool) -> ToolInfo {
    ToolInfo {
        name: tool.name.into_owned(),
        description: tool
            .description
            .map(|description| bound_tool_description(description.into_owned())),
        input_schema: serde_json::Value::Object((*tool.input_schema).clone()),
    }
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
struct Inner {
    /// server id → its connection slot. The `tokio::Mutex` guards the map itself;
    /// each slot has its own inner mutex for its connect/redial, so per-server
    /// connects never serialize behind one another.
    slots: Mutex<HashMap<String, Arc<ServerSlot>>>,
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
    /// Warm-on-open discovery: for every enabled server, kick off a BACKGROUND
    /// connect + `list_tools` and return immediately. Both chat doors call this on
    /// mount so a turn build usually finds discovery already done (or in-flight,
    /// which it awaits). Nothing here blocks; a failed warm is logged and left for
    /// the turn / next-use retry. Runs only on chat-surface open, never at app
    /// launch (deferred-startup invariant).
    pub(crate) fn warm(&self, app_handle: &tauri::AppHandle) {
        for cfg in enabled_servers(app_handle) {
            let manager = self.clone();
            tauri::async_runtime::spawn(async move {
                let slot = manager.inner.slot_for(&cfg).await;
                if let Err(error) = slot.ensure_ready(&cfg).await {
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

        let mut tools: Vec<ai_engine::AgentTool> = Vec::new();
        let mut notes: Vec<String> = Vec::new();
        for (cfg, ready) in ready {
            let established = match ready {
                Ok(Ok(established)) => established,
                Ok(Err(error)) => {
                    // Connect/list failed within the budget: skip this server this
                    // turn (remedy is the enabled toggle; next turn retries).
                    tauri_plugin_log::log::warn!(
                        "Ask AI MCP connector \"{}\" unavailable this turn: {error}",
                        cfg.label
                    );
                    continue;
                }
                Err(_elapsed) => {
                    // Still connecting when the budget expired: proceed without it;
                    // the in-flight connect keeps running for the next turn.
                    tauri_plugin_log::log::info!(
                        "Ask AI MCP connector \"{}\" not ready within {}s; proceeding without it",
                        cfg.label,
                        DISCOVERY_TURN_BUDGET.as_secs()
                    );
                    continue;
                }
            };

            let (offered, note) = offered_tools(&established.tools, cfg.enabled_tools.as_deref());
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
                    parameters_schema: normalize_schema(tool.input_schema),
                });
            }
        }
        (tools, notes)
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

        // First attempt on the (possibly cached) connection.
        match call_once(&slot, &cfg, tool, &params).await {
            Ok(result) => Ok(result),
            Err(_first_error) => {
                // Drop the handle and redial ONCE (failure policy).
                slot.invalidate().await;
                call_once(&slot, &cfg, tool, &params)
                    .await
                    .map_err(|second_error| {
                        format!(
                            "MCP server \"{}\" failed this tool call twice: {second_error}",
                            cfg.label
                        )
                    })
            }
        }
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
        self.inner
            .slots
            .lock()
            .await
            .retain(|id, _slot| keep.contains(id));
    }
}

/// One connect-then-call attempt. A connect failure surfaces as `Err`; a call
/// failure invalidates the slot (so the outer retry redials) and surfaces as
/// `Err`. On success the result is serialized and truncated to the result cap.
async fn call_once(
    slot: &ServerSlot,
    cfg: &McpServerConfig,
    tool: &str,
    params: &serde_json::Value,
) -> Result<String, String> {
    let established = slot.ensure_ready(cfg).await?;

    let arguments = match params {
        serde_json::Value::Object(map) => Some(map.clone()),
        // A null / non-object payload is treated as no arguments.
        _ => None,
    };
    let mut request = rmcp::model::CallToolRequestParams::new(tool.to_string());
    if let Some(arguments) = arguments {
        request = request.with_arguments(arguments);
    }

    match established.client.call_tool(request).await {
        Ok(result) => {
            let json = serde_json::to_string(&result)
                .map_err(|error| format!("failed to serialize MCP tool result: {error}"))?;
            Ok(truncate_tool_result(json))
        }
        Err(error) => {
            // Drop the handle so the outer retry redials on a fresh connection.
            slot.invalidate().await;
            Err(error.to_string())
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

    /// A stdio "server" that spawns but never completes the MCP initialize
    /// handshake (`sleep` ignores stdin and never writes stdout). rmcp's
    /// `serve_client` has no internal initialize timeout, so connecting this would
    /// hang until the child exits without the `ensure_ready` timeout wrapper.
    fn hung_stdio_cfg() -> McpServerConfig {
        McpServerConfig {
            id: "test-hung-mcp-server".to_string(),
            label: "Hung".to_string(),
            enabled: true,
            transport: McpTransport::Stdio,
            command: Some("sleep".to_string()),
            args: vec!["60".to_string()],
            env: Vec::new(),
            url: None,
            secret_env_name: None,
            enabled_tools: None,
        }
    }

    /// `warm` and `list_server_tools` call `ensure_ready` with no timeout of their
    /// own, and `ensure_ready` holds the per-slot lock across the connect. A server
    /// that hangs during initialize must therefore be bounded here, or it poisons
    /// the slot forever (leaked child/task) and taxes every later turn.
    #[tokio::test]
    async fn ensure_ready_is_bounded_on_a_hung_server() {
        let cfg = hung_stdio_cfg();
        let slot = ServerSlot {
            fingerprint: config_fingerprint(&cfg),
            state: Mutex::new(SlotState::default()),
        };
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
}
