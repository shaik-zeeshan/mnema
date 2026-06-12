//! Server-side tool-activity labelling + app-icon resolution (issue #110,
//! Slice 3).
//!
//! The frontend used to format brokered tool calls into human labels and then
//! fetch app icons itself. That formatting now lives here so a
//! [`ToolActivityEntry`] arrives at the frontend fully labelled AND, when the
//! call was app-scoped, already carrying a resolved `app_icon_path`. This is a
//! faithful port of `formatToolActivity` / `readString` from
//! `apps/desktop/src/lib/insights/Chat.svelte` — the label strings (including
//! the curly double-quotes around a search query) match byte-for-byte.
//!
//! Icon resolution reuses the existing
//! [`crate::native_capture::resolve_app_icons`] command (which already handles
//! both bundle ids and human display names, and is macOS-only). The returned
//! path is a raw filesystem path; the frontend applies `convertFileSrc()`
//! itself, so nothing is URL-encoded here.

use capture_types::ToolActivityEntry;
use serde_json::Value;

use crate::native_capture::{resolve_app_icons, ResolveAppIconsRequest};

/// Read a string parameter, trimmed, returning `None` when absent, non-string,
/// or empty after trimming. Mirrors the frontend's `readString`.
fn read_string_param(params: &Value, key: &str) -> Option<String> {
    match params.get(key) {
        Some(Value::String(value)) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        _ => None,
    }
}

/// Format a brokered tool call into a render-ready [`ToolActivityEntry`].
///
/// Pure, no IO — `app_icon_path` is always `None` here; the async enrich step
/// ([`resolve_app_icon_path`] / [`build_tool_activity_entry`]) fills it in.
/// Faithful port of the frontend's `formatToolActivity`.
pub(crate) fn format_tool_activity(tool: &str, params: &Value) -> ToolActivityEntry {
    match tool {
        "search" => {
            let label = match read_string_param(params, "query") {
                // Curly double-quotes (U+201C / U+201D) to match the frontend's
                // `“${queryText}”`.
                Some(query) => format!("Searching \u{201c}{query}\u{201d}"),
                None => "Searching your captures".to_string(),
            };
            ToolActivityEntry {
                kind: "search".to_string(),
                label,
                // The app scope renders as an icon + name chip, not label text.
                app: read_string_param(params, "app"),
                app_icon_path: None,
            }
        }
        "timeline" => ToolActivityEntry {
            kind: "timeline".to_string(),
            label: "Scanning timeline".to_string(),
            app: read_string_param(params, "app"),
            app_icon_path: None,
        },
        "show_text" => ToolActivityEntry {
            kind: "show_text".to_string(),
            label: "Reading a capture".to_string(),
            app: None,
            app_icon_path: None,
        },
        other => ToolActivityEntry {
            kind: "other".to_string(),
            label: if other.is_empty() {
                "Working".to_string()
            } else {
                format!("Running {other}")
            },
            app: None,
            app_icon_path: None,
        },
    }
}

/// Resolve an icon path for an app scope (bundle id OR display name) by reusing
/// the existing [`resolve_app_icons`] command. Returns the first resolution's
/// `icon_path`, or `None` on any error or empty result. The path is raw (not
/// URL-encoded); the frontend applies `convertFileSrc()`.
pub(crate) async fn resolve_app_icon_path(
    app_handle: &tauri::AppHandle,
    app: &str,
) -> Option<String> {
    let request = ResolveAppIconsRequest {
        bundle_ids: vec![app.to_string()],
    };
    resolve_app_icons(app_handle.clone(), request)
        .await
        .ok()
        .and_then(|resolutions| resolutions.into_iter().next())
        .and_then(|resolution| resolution.icon_path)
}

/// Format a tool call into a [`ToolActivityEntry`] and, when it is app-scoped,
/// enrich it with a resolved `app_icon_path`. Convenience used by Slice 4.
pub(crate) async fn build_tool_activity_entry(
    app_handle: &tauri::AppHandle,
    tool: &str,
    params: &Value,
) -> ToolActivityEntry {
    let mut entry = format_tool_activity(tool, params);
    if let Some(app) = entry.app.clone() {
        entry.app_icon_path = resolve_app_icon_path(app_handle, &app).await;
    }
    entry
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn search_with_query_formats_curly_quoted_label() {
        let entry = format_tool_activity("search", &json!({ "query": "rust async" }));
        assert_eq!(entry.kind, "search");
        assert_eq!(entry.label, "Searching \u{201c}rust async\u{201d}");
        assert!(entry.label.contains("\u{201c}rust async\u{201d}"));
        assert_eq!(entry.app, None);
        assert_eq!(entry.app_icon_path, None);
    }

    #[test]
    fn search_without_query_uses_fallback_label() {
        let entry = format_tool_activity("search", &json!({}));
        assert_eq!(entry.kind, "search");
        assert_eq!(entry.label, "Searching your captures");
    }

    #[test]
    fn search_blank_query_uses_fallback_label() {
        // Whitespace-only query trims to empty → fallback.
        let entry = format_tool_activity("search", &json!({ "query": "   " }));
        assert_eq!(entry.label, "Searching your captures");
    }

    #[test]
    fn search_with_app_param_sets_app_but_not_icon() {
        let entry = format_tool_activity(
            "search",
            &json!({ "query": "notes", "app": "com.example.app" }),
        );
        assert_eq!(entry.app, Some("com.example.app".to_string()));
        // Pure stage never resolves an icon.
        assert_eq!(entry.app_icon_path, None);
    }

    #[test]
    fn timeline_label_and_kind() {
        let entry = format_tool_activity("timeline", &json!({ "app": "Zen Browser" }));
        assert_eq!(entry.kind, "timeline");
        assert_eq!(entry.label, "Scanning timeline");
        assert_eq!(entry.app, Some("Zen Browser".to_string()));
    }

    #[test]
    fn show_text_label_kind_and_no_app() {
        let entry = format_tool_activity("show_text", &json!({ "app": "ignored" }));
        assert_eq!(entry.kind, "show_text");
        assert_eq!(entry.label, "Reading a capture");
        assert_eq!(entry.app, None);
    }

    #[test]
    fn unknown_tool_runs_named() {
        let entry = format_tool_activity("foo", &json!({}));
        assert_eq!(entry.kind, "other");
        assert_eq!(entry.label, "Running foo");
        assert_eq!(entry.app, None);
    }

    #[test]
    fn empty_tool_falls_back_to_working() {
        let entry = format_tool_activity("", &json!({}));
        assert_eq!(entry.kind, "other");
        assert_eq!(entry.label, "Working");
    }

    #[test]
    fn entry_serializes_camel_case_and_skips_absent_icon_path() {
        let entry = format_tool_activity(
            "search",
            &json!({ "query": "x", "app": "com.example.app" }),
        );
        let value = serde_json::to_value(&entry).unwrap();
        let obj = value.as_object().unwrap();
        assert!(obj.contains_key("kind"));
        assert!(obj.contains_key("label"));
        assert!(obj.contains_key("app"));
        // `app_icon_path` is None here → skip_serializing_if omits it.
        assert!(!obj.contains_key("appIconPath"));
        assert_eq!(obj["app"], json!("com.example.app"));
    }

    #[test]
    fn entry_omits_app_key_when_absent() {
        let entry = format_tool_activity("show_text", &json!({}));
        let value = serde_json::to_value(&entry).unwrap();
        let obj = value.as_object().unwrap();
        assert!(!obj.contains_key("app"));
        assert!(!obj.contains_key("appIconPath"));
    }
}
