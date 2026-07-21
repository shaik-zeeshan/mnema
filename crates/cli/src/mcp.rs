//! Local MCP server over stdio: the same four brokered data commands the CLI
//! exposes, as MCP tools for chat clients (Claude Desktop, Cursor, ...).
//! Consent, redaction, and grant enforcement all stay in the app's broker.

use app_infra::brokered_access::{
    BrokerClientIdentity, BrokerSearchRequest, BrokerTimelineRequest, BrokeredCaptureRequest,
};
use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::stdio,
    ErrorData, ServerHandler, ServiceExt,
};
use serde::Deserialize;

use crate::{broker_error, execute_data_request, CliError};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SearchParams {
    /// Full-text query over captured screen text and audio transcripts.
    query: String,
    /// RFC3339 lower time bound, e.g. 2026-07-21T09:00:00Z.
    from: Option<String>,
    /// RFC3339 upper time bound.
    to: Option<String>,
    /// Maximum number of results.
    limit: Option<u32>,
    /// Filter by application name, e.g. "Linear".
    app: Option<String>,
    /// Filter by window title.
    window_title: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TimelineParams {
    /// RFC3339 lower time bound, e.g. 2026-07-21T09:00:00Z.
    from: String,
    /// RFC3339 upper time bound.
    to: String,
    /// Maximum number of intervals.
    limit: Option<u32>,
    /// Filter by application name, e.g. "Linear".
    app: Option<String>,
    /// Filter by window title.
    window_title: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ResultIdParams {
    /// Opaque result id returned by a previous search.
    opaque_result_id: String,
}

#[derive(Clone)]
struct MnemaMcp {
    identity: BrokerClientIdentity,
}

#[tool_router]
impl MnemaMcp {
    #[tool(
        description = "Search the user's captured screen text and audio transcripts. Returns snippets with opaque result ids; use show_text for the full text behind a result and open to reveal it in the Mnema app."
    )]
    async fn search(
        &self,
        Parameters(p): Parameters<SearchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run(
            "search",
            BrokeredCaptureRequest::Search(BrokerSearchRequest {
                query: p.query,
                from: p.from,
                to: p.to,
                limit: p.limit,
                app: p.app,
                window_title: p.window_title,
            }),
        )
        .await
    }

    #[tool(
        description = "List the user's capture activity intervals between two RFC3339 timestamps."
    )]
    async fn timeline(
        &self,
        Parameters(p): Parameters<TimelineParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run(
            "timeline",
            BrokeredCaptureRequest::Timeline(BrokerTimelineRequest {
                from: p.from,
                to: p.to,
                limit: p.limit,
                app: p.app,
                window_title: p.window_title,
            }),
        )
        .await
    }

    #[tool(description = "Fetch the full captured text behind a search result id.")]
    async fn show_text(
        &self,
        Parameters(p): Parameters<ResultIdParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run(
            "show-text",
            BrokeredCaptureRequest::ShowText {
                opaque_id: p.opaque_result_id,
            },
        )
        .await
    }

    #[tool(description = "Open a result in the Mnema app so the user can view it.")]
    async fn open(
        &self,
        Parameters(p): Parameters<ResultIdParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run(
            "open",
            BrokeredCaptureRequest::OpenInMnema {
                opaque_id: p.opaque_result_id,
            },
        )
        .await
    }

    async fn run(
        &self,
        command: &str,
        request: BrokeredCaptureRequest,
    ) -> Result<CallToolResult, ErrorData> {
        // No TTY under an MCP client, but the approval prompt is the Mnema
        // app's own consent dialog — the user is present, so let it fire.
        match execute_data_request(command, &self.identity, request, true).await {
            Ok(value) => {
                let text = serde_json::to_string(&value)
                    .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
                Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
            }
            // Broker/auth failures are tool-level errors so the model can
            // relay them to the user, not protocol failures.
            Err(error) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "{} ({})",
                error.message, error.code
            ))])),
        }
    }
}

#[tool_handler]
impl ServerHandler for MnemaMcp {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.instructions = Some(
            "Brokered read access to the user's Mnema screen and audio capture history. \
             The first call may pause while the user approves access in the Mnema app."
                .to_string(),
        );
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = Implementation::new("mnema", env!("CARGO_PKG_VERSION"));
        info
    }
}

pub(crate) async fn serve(identity: BrokerClientIdentity) -> Result<(), CliError> {
    let service = MnemaMcp { identity }
        .serve(stdio())
        .await
        .map_err(broker_error)?;
    service.waiting().await.map_err(broker_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_router_exposes_exactly_the_four_data_tools() {
        let mut names: Vec<String> = MnemaMcp::tool_router()
            .list_all()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect();
        names.sort();
        assert_eq!(names, ["open", "search", "show_text", "timeline"]);
    }
}
