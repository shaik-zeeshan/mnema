//! The turn's tool executor: every model tool call routes through here.

use super::*;

#[allow(clippy::too_many_arguments)]
pub fn build_tool_executor(
    core: Arc<AppCore>,
    conversation_id: String,
    turn_token: u64,
    scope: Option<AskAiScope>,
    utc_offset_minutes: Option<i32>,
    search_metadata: Arc<Mutex<HashMap<String, BrokerSearchResult>>>,
    tool_activities: Arc<Mutex<Vec<serde_json::Value>>>,
    sources: Arc<Mutex<Vec<serde_json::Value>>>,
) -> ai_engine::ToolExecutor {
    let core = core.clone();
    let conversation_id = conversation_id.clone();
    let search_metadata = Arc::clone(&search_metadata);
    let tool_activities = Arc::clone(&tool_activities);
    let sources = Arc::clone(&sources);
    Arc::new(move |tool: String, params: serde_json::Value| {
        let core = core.clone();
        let conversation_id = conversation_id.clone();
        let search_metadata = Arc::clone(&search_metadata);
        let tool_activities = Arc::clone(&tool_activities);
        let sources = Arc::clone(&sources);
        Box::pin(async move {
            // Presentation signal: validate/decode the nominated sources,
            // never dispatched to the broker. Its resolved source set is
            // stashed for persistence and emitted as a `Sources` view update.
            if tool == "reference_captures" {
                let (ack, nominated) =
                    handle_reference_captures(&core, &search_metadata, params).await?;
                if let Ok(mut buffer) = sources.lock() {
                    *buffer = nominated.clone();
                }
                // Emit the resolved source set as a `Sources` view update.
                emit_live_update(
                    &core,
                    &conversation_id,
                    turn_token,
                    TurnUpdate::Sources {
                        sources: serde_json::Value::Array(nominated),
                    },
                );
                return serde_json::to_string(&ack)
                    .map_err(|error| format!("failed to serialize reference ack: {error}"));
            }

            // CH3: clamp captured-history windows into the chat's scope.
            let params = scope::scope_tool_params(scope.as_ref(), &tool, params)?;
            // Data tool: record the activity, run it through the broker seam,
            // retain any search results, return the JSON result as a string.
            if let Ok(mut buffer) = tool_activities.lock() {
                buffer.push(serde_json::json!({ "tool": tool, "params": params }));
            }
            match classify_ask_ai_tool(&tool) {
                // App-control tools (Workstream A) act on the recording
                // lifecycle instead of the broker. Kept after the activity
                // push so the control action still shows in the working line.
                ToolRoute::AppControl => {
                    return app_control::execute_app_control_tool(&core, &tool).await;
                }
                // `fetch_url` (Workstream B): re-fetch a page the user visited.
                // Desktop-side (not brokered); rechecks its opt-in setting.
                ToolRoute::FetchUrl => {
                    return web_fetch::execute_web_fetch(&core, params).await;
                }
                // MCP connector tools (Workstream C): a `mcp__<server-id>__<tool>`
                // name routes to that server's tool via the manager. This is the
                // SINGLE MCP dispatch choke-point — a per-call approval hook would
                // insert HERE; trust-per-server is the consent (enabling the server
                // in Settings), so there is no per-call prompt in v1 (ADR 0048).
                ToolRoute::Mcp {
                    server_id,
                    tool: mcp_tool,
                } => {
                    let manager = (*core.state::<mcp::McpManager>()).clone();
                    return manager.call_tool(&core, server_id, mcp_tool, params).await;
                }
                // Everything else falls through to the broker seam below.
                ToolRoute::Broker => {}
            }
            let request = broker_request_from_tool(&tool, params)?;
            let response = execute_ask_ai_broker_request(core, request).await?;
            if let Ok(mut map) = search_metadata.lock() {
                retain_source_metadata(&response, &mut map);
            }
            let mut value = broker_response_to_tool_value(response)?;
            if let Some(offset) = utc_offset_minutes {
                annotate_local_times(&mut value, offset);
            }
            serde_json::to_string(&value)
                .map_err(|error| format!("failed to serialize Ask AI tool result: {error}"))
        })
    })
}
