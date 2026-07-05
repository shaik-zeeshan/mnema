//! MCP tool connectors for the Ask AI chat agent (Workstream C, ADR 0048).
//!
//! User-configured MCP servers (stdio or streamable-HTTP) whose tools join the
//! chat agent under trust-per-server gating. This module owns:
//!   - the persistent [`McpManager`] in Tauri managed state — lazy connect,
//!     warm-on-open discovery, per-app-session connection cache — in `manager`;
//!   - transport construction + per-transport secret delivery — in `transport`;
//!   - the pure, unit-tested routing/curation helpers below.
//!
//! `crates/ai-runtime` stays MCP-ignorant (ADR 0033): MCP tools are injected
//! into the agent loop as ordinary Tauri-layer tool callbacks, exactly like the
//! broker tools — the engine never learns what the tools are.

pub(crate) mod manager;
pub(crate) mod node_check;
mod transport;

pub(crate) use manager::McpManager;

/// Model-facing tool-name prefix (Claude Code convention): a chat tool named
/// `mcp__<server-id>__<tool>` routes to server `<server-id>`'s `<tool>`.
pub(crate) const MCP_TOOL_PREFIX: &str = "mcp__";

/// Default tool budget when a server is NOT curated (`enabled_tools = None`):
/// offer the first N tools in server order. A curated `Some(list)` has no cap.
const MCP_DEFAULT_TOOL_CAP: usize = 32;

/// Cap on a single MCP tool result handed back to the model (~24k chars), so one
/// rogue tool cannot flood a turn. A visible marker is appended when it bites.
const MCP_TOOL_RESULT_CHAR_CAP: usize = 24_000;
/// Cap on a server-supplied tool DESCRIPTION before it enters the model prompt.
/// Descriptions are far shorter than results in practice; this only bites a
/// pathological/hostile server trying to context-stuff via the tool declaration.
const MCP_TOOL_DESCRIPTION_CHAR_CAP: usize = 4_000;
/// Cap on a server-supplied tool input SCHEMA (serialized) before it enters the
/// model prompt as the tool's parameter schema. Like the description cap, this
/// stops a malicious/compromised server from shipping a multi-megabyte schema
/// (padding, or injection text in property `description` fields the model reads)
/// that stuffs the model context on every turn. Generous, since a legitimate
/// tool schema is a few KB at most; it only bites a pathological one.
const MCP_TOOL_SCHEMA_CHAR_CAP: usize = 16_000;

/// One tool discovered from an MCP server — our trimmed view of rmcp's `Tool`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ToolInfo {
    pub name: String,
    pub description: Option<String>,
    /// JSON Schema object for the tool's params, passed through to the model.
    pub input_schema: serde_json::Value,
}

/// Build the model-facing tool name `mcp__<server-id>__<tool>`.
pub(crate) fn model_tool_name(server_id: &str, tool: &str) -> String {
    format!("{MCP_TOOL_PREFIX}{server_id}__{tool}")
}

/// Parse a model-facing `mcp__<server-id>__<tool>` name into `(server_id, tool)`.
///
/// Returns `None` for any name that is not a well-formed MCP tool name:
///   - no `mcp__` prefix → a non-MCP tool the executor routes elsewhere;
///   - a bare `mcp__`, or `mcp__<id>` with no second `__`, or an empty id/tool →
///     malformed.
///
/// Server ids are slug-safe `[a-z0-9-]` (enforced at add time, ADR 0048), so
/// splitting on the FIRST `__` after the prefix is unambiguous — the id can never
/// itself contain `__`.
pub(crate) fn parse_mcp_tool_name(name: &str) -> Option<(&str, &str)> {
    let rest = name.strip_prefix(MCP_TOOL_PREFIX)?;
    let (server_id, tool) = rest.split_once("__")?;
    if server_id.is_empty() || tool.is_empty() {
        return None;
    }
    Some((server_id, tool))
}

/// Whether a MODEL-FACING tool name satisfies the provider tool-name contract
/// `^[a-zA-Z0-9_-]{1,64}$` (Anthropic and OpenAI both enforce it). Checked over
/// the FULL `mcp__<id>__<tool>` name at discovery: one violating name (a dot, a
/// space, or sheer length) makes the provider reject the ENTIRE request — every
/// tool, the whole turn — so an invalid tool is DROPPED there, never truncated
/// (a rewritten name would no longer route back through [`parse_mcp_tool_name`]
/// and could collide with a sibling tool).
pub(crate) fn is_valid_model_tool_name(name: &str) -> bool {
    // Byte length equals char length here: the charset check admits ASCII only.
    (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// Curate a server's discovered tools into the set offered to the model.
///
/// - `enabled_tools = None` → the FIRST [`MCP_DEFAULT_TOOL_CAP`] tools in server
///   order. When the server exposes more, the trim is non-silent: a `Some(note)`
///   is returned (the caller logs it + adds one preamble line).
/// - `enabled_tools = Some(list)` → EXACTLY the named tools that still exist, in
///   server order (drift: a selected name no longer present is silently dropped),
///   **no cap**, never a note.
///
/// Pure so the curation + drift rules are unit-testable. The note is label-free
/// (count-based) — the caller prepends the server label for the preamble.
pub(crate) fn offered_tools(
    all: &[ToolInfo],
    enabled_tools: Option<&[String]>,
) -> (Vec<ToolInfo>, Option<String>) {
    match enabled_tools {
        Some(selected) => {
            let wanted: std::collections::HashSet<&str> =
                selected.iter().map(String::as_str).collect();
            let chosen = all
                .iter()
                .filter(|tool| wanted.contains(tool.name.as_str()))
                .cloned()
                .collect();
            (chosen, None)
        }
        None => {
            if all.len() > MCP_DEFAULT_TOOL_CAP {
                let note = format!(
                    "exposes {} tools; only the first {} are available (curate it in Settings to \
pick which)",
                    all.len(),
                    MCP_DEFAULT_TOOL_CAP
                );
                (all[..MCP_DEFAULT_TOOL_CAP].to_vec(), Some(note))
            } else {
                (all.to_vec(), None)
            }
        }
    }
}

/// Truncate a serialized MCP tool result to [`MCP_TOOL_RESULT_CHAR_CAP`] chars,
/// appending a visible marker when it bites, so one tool cannot flood the turn.
pub(crate) fn truncate_tool_result(result: String) -> String {
    if result.chars().count() <= MCP_TOOL_RESULT_CHAR_CAP {
        return result;
    }
    let mut out: String = result.chars().take(MCP_TOOL_RESULT_CHAR_CAP).collect();
    out.push_str("\n\n[… MCP tool result truncated by Mnema to keep the turn bounded …]");
    out
}

/// Cap a server-supplied tool DESCRIPTION before it enters the model prompt. The
/// description is untrusted third-party text (an MCP server the user connected,
/// whose payloads are only semi-trusted); like the result cap, this stops a
/// malicious or compromised server from shipping a multi-megabyte description
/// that stuffs the model context on every turn.
pub(crate) fn bound_tool_description(description: String) -> String {
    if description.chars().count() <= MCP_TOOL_DESCRIPTION_CHAR_CAP {
        return description;
    }
    let mut out: String = description
        .chars()
        .take(MCP_TOOL_DESCRIPTION_CHAR_CAP)
        .collect();
    out.push_str(" […truncated by Mnema]");
    out
}

/// Bound a server-supplied tool input schema before it reaches the model as the
/// tool's parameter schema. A JSON object cannot be safely char-truncated (it
/// would no longer parse), so an over-cap schema is DROPPED for a permissive
/// empty-object schema — the same fallback a non-object schema gets — ensuring
/// no unbounded server-controlled text rides the schema channel into the model.
pub(crate) fn bound_tool_schema(schema: serde_json::Value) -> serde_json::Value {
    let within_cap = serde_json::to_string(&schema)
        .map(|serialized| serialized.len() <= MCP_TOOL_SCHEMA_CHAR_CAP)
        .unwrap_or(false);
    if within_cap {
        schema
    } else {
        serde_json::json!({ "type": "object", "additionalProperties": true, "properties": {} })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str) -> ToolInfo {
        ToolInfo {
            name: name.to_string(),
            description: Some(format!("does {name}")),
            input_schema: serde_json::json!({ "type": "object" }),
        }
    }

    fn tools(names: &[&str]) -> Vec<ToolInfo> {
        names.iter().map(|name| tool(name)).collect()
    }

    #[test]
    fn bound_tool_description_caps_a_giant_server_description() {
        // A malicious/compromised MCP server ships a multi-megabyte tool
        // description; it must be bounded before it enters the model prompt so one
        // rogue server cannot stuff the context on every turn.
        let bounded = bound_tool_description("z".repeat(1_000_000));
        assert!(
            bounded.chars().count() <= MCP_TOOL_DESCRIPTION_CHAR_CAP + 32,
            "description must be bounded, got {} chars",
            bounded.chars().count()
        );
        // A short description is passed through untouched.
        assert_eq!(bound_tool_description("hi".to_string()), "hi");
    }

    #[test]
    fn model_tool_name_round_trips_through_the_parser() {
        let name = model_tool_name("github-2", "create_issue");
        assert_eq!(name, "mcp__github-2__create_issue");
        assert_eq!(
            parse_mcp_tool_name(&name),
            Some(("github-2", "create_issue"))
        );
    }

    #[test]
    fn parse_splits_on_the_first_double_underscore() {
        // A tool name that itself contains `__` stays whole; the id never does.
        assert_eq!(
            parse_mcp_tool_name("mcp__srv__list__things"),
            Some(("srv", "list__things"))
        );
    }

    #[test]
    fn parse_rejects_non_mcp_names_as_passthrough() {
        // No prefix → the executor routes it elsewhere (broker / app-control).
        assert_eq!(parse_mcp_tool_name("search"), None);
        assert_eq!(parse_mcp_tool_name("stop_capture"), None);
    }

    #[test]
    fn parse_rejects_malformed_mcp_names() {
        assert_eq!(parse_mcp_tool_name("mcp__"), None); // bare prefix
        assert_eq!(parse_mcp_tool_name("mcp__github"), None); // no second `__`
        assert_eq!(parse_mcp_tool_name("mcp____tool"), None); // empty id
        assert_eq!(parse_mcp_tool_name("mcp__srv__"), None); // empty tool
    }

    // The provider tool-name contract (`^[a-zA-Z0-9_-]{1,64}$`) is enforced over
    // the FULL model-facing name at discovery; one violating name would make the
    // provider reject the whole request, dropping every tool for the turn.

    #[test]
    fn a_simple_model_tool_name_is_valid() {
        assert!(is_valid_model_tool_name(&model_tool_name(
            "github",
            "create_issue"
        )));
    }

    #[test]
    fn a_tool_name_with_a_dot_is_invalid() {
        assert!(!is_valid_model_tool_name(&model_tool_name(
            "srv",
            "list.files"
        )));
    }

    #[test]
    fn a_model_tool_name_over_64_chars_is_invalid() {
        // `mcp__srv__` is 10 chars, so a 55-char tool name lands on 65.
        let over = model_tool_name("srv", &"t".repeat(55));
        assert_eq!(over.len(), 65);
        assert!(!is_valid_model_tool_name(&over));
    }

    #[test]
    fn a_model_tool_name_at_exactly_64_chars_is_valid() {
        let at_cap = model_tool_name("srv", &"t".repeat(54));
        assert_eq!(at_cap.len(), 64);
        assert!(is_valid_model_tool_name(&at_cap));
    }

    #[test]
    fn offered_none_under_cap_returns_all_and_no_note() {
        let all = tools(&["a", "b", "c"]);
        let (offered, note) = offered_tools(&all, None);
        assert_eq!(offered, all);
        assert!(note.is_none());
    }

    #[test]
    fn offered_none_at_cap_returns_all_and_no_note() {
        let names: Vec<String> = (0..MCP_DEFAULT_TOOL_CAP).map(|i| format!("t{i}")).collect();
        let all: Vec<ToolInfo> =
            names.iter().map(|name| tool(name)).collect();
        let (offered, note) = offered_tools(&all, None);
        assert_eq!(offered.len(), MCP_DEFAULT_TOOL_CAP);
        assert!(note.is_none(), "exactly the cap must not note a trim");
    }

    #[test]
    fn offered_none_over_cap_takes_first_32_and_notes() {
        let names: Vec<String> = (0..40).map(|i| format!("t{i}")).collect();
        let all: Vec<ToolInfo> = names.iter().map(|name| tool(name)).collect();
        let (offered, note) = offered_tools(&all, None);
        assert_eq!(offered.len(), MCP_DEFAULT_TOOL_CAP);
        // First 32 in server order.
        assert_eq!(offered.first().unwrap().name, "t0");
        assert_eq!(offered.last().unwrap().name, "t31");
        let note = note.expect("over-cap trim must be non-silent");
        assert!(note.contains("40"), "note mentions the real count: {note}");
        assert!(note.contains("32"), "note mentions the cap: {note}");
    }

    #[test]
    fn offered_some_returns_exactly_the_selected_subset_in_server_order() {
        let all = tools(&["a", "b", "c", "d"]);
        let selected = vec!["c".to_string(), "a".to_string()];
        let (offered, note) = offered_tools(&all, Some(&selected));
        // Preserves SERVER order (a before c), drops the unselected.
        let names: Vec<&str> = offered.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["a", "c"]);
        assert!(note.is_none(), "a curated server never emits a trim note");
    }

    #[test]
    fn offered_some_over_cap_has_no_limit() {
        // A curated list larger than the default cap is offered in full.
        let names: Vec<String> = (0..40).map(|i| format!("t{i}")).collect();
        let all: Vec<ToolInfo> = names.iter().map(|name| tool(name)).collect();
        let (offered, note) = offered_tools(&all, Some(&names));
        assert_eq!(offered.len(), 40);
        assert!(note.is_none());
    }

    #[test]
    fn offered_some_drops_a_selected_tool_that_vanished() {
        // Drift: a curated tool no longer present on the server is dropped, not
        // surfaced as a phantom tool.
        let all = tools(&["a", "b"]);
        let selected = vec!["a".to_string(), "gone".to_string()];
        let (offered, _note) = offered_tools(&all, Some(&selected));
        let names: Vec<&str> = offered.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["a"]);
    }

    #[test]
    fn truncate_leaves_short_results_untouched() {
        let short = "hello".to_string();
        assert_eq!(truncate_tool_result(short.clone()), short);
    }

    #[test]
    fn truncate_caps_long_results_and_marks_them() {
        let long = "x".repeat(MCP_TOOL_RESULT_CHAR_CAP + 500);
        let out = truncate_tool_result(long);
        assert!(out.chars().count() > MCP_TOOL_RESULT_CHAR_CAP);
        assert!(out.contains("truncated by Mnema"));
        // The kept prefix is exactly the cap; the rest is the marker.
        assert!(out.starts_with(&"x".repeat(MCP_TOOL_RESULT_CHAR_CAP)));
    }
}
