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

/// Format a `time::Date` as a bare `YYYY-MM-DD` ISO calendar date.
fn format_iso_date(date: time::Date) -> String {
    format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        u8::from(date.month()),
        date.day(),
    )
}

/// Parse one RFC3339 UTC bound from `params` into an `OffsetDateTime` instant.
/// Returns `None` when the param is absent or unparseable, so a bad bound
/// silently drops out of the label exactly as the broker handler silently
/// ignores it.
fn parse_bound(params: &Value, key: &str) -> Option<time::OffsetDateTime> {
    let raw = read_string_param(params, key)?;
    time::OffsetDateTime::parse(&raw, &time::format_description::well_known::Rfc3339).ok()
}

/// Shift a UTC instant into the user's local wall clock by `offset` minutes and
/// take its calendar date. Mirrors `utc_rfc3339_to_local_display` /
/// `build_temporal_grounding` in `ask_ai.rs`.
fn local_date(instant: time::OffsetDateTime, offset: i32) -> time::Date {
    (instant + time::Duration::minutes(i64::from(offset))).date()
}

/// Render a local calendar date relative to the local "today" anchor: `today_d`
/// → `"Today"`, the day before → `"Yesterday"`, otherwise the bare ISO date.
fn rel_or_date(date: time::Date, today_d: time::Date) -> String {
    if date == today_d {
        "Today".to_string()
    } else if today_d.previous_day() == Some(date) {
        "Yesterday".to_string()
    } else {
        format_iso_date(date)
    }
}

/// Build the human range suffix for a `recall_context` call's `from`/`to`
/// window, in the user's LOCAL time with human-readable relative words.
///
/// Given the parsed bounds, `now_ms`, and `utc_offset_minutes`:
/// - Each bound is shifted by the local offset before its calendar date is
///   taken. The half-open `to` (exclusive next-midnight) is collapsed to the
///   INCLUSIVE last local day via `(to_instant - 1ms)`, so a single-day "today"
///   window renders as one day.
/// - Dates relativize against the local "today" anchor derived from `now_ms`.
/// - Both bounds, single local day → `relOrDate(from)` (e.g. `"Today"`);
///   multi-day → `"{relOrDate(from)} \u{2013} {relOrDate(last)}"`; only `from` →
///   `"since {relOrDate(from)}"`; only `to` → `"until {relOrDate(last)}"`;
///   neither → `None`.
///
/// When `utc_offset_minutes` is `None` the offset is unknown, so we render
/// WITHOUT relativization — falling back to bare UTC ISO dates (no wrong
/// "Today"): both bounds → `from \u{2013} to`; only `from` → `from \u{2013} now`;
/// only `to` → `until to`.
fn format_recall_range(params: &Value, now_ms: i64, utc_offset_minutes: Option<i32>) -> Option<String> {
    let from = parse_bound(params, "from");
    let to = parse_bound(params, "to");

    let Some(offset) = utc_offset_minutes else {
        // Offset unknown → keep the legacy bare-UTC-date behavior so we never
        // show a misleading "Today".
        let from = from.map(|dt| format_iso_date(dt.date()));
        let to = to.map(|dt| format_iso_date(dt.date()));
        return match (from, to) {
            (Some(from), Some(to)) => Some(format!("{from} \u{2013} {to}")),
            (Some(from), None) => Some(format!("{from} \u{2013} now")),
            (None, Some(to)) => Some(format!("until {to}")),
            (None, None) => None,
        };
    };

    // Local "today" anchor from `now_ms`.
    let now_utc = time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(now_ms) * 1_000_000)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    let today_d = local_date(now_utc, offset);

    let from_d = from.map(|dt| local_date(dt, offset));
    // Collapse the half-open end: the inclusive last local day is the local date
    // of the instant one millisecond before the exclusive `to`.
    let last_d = to.map(|dt| local_date(dt - time::Duration::milliseconds(1), offset));

    match (from_d, last_d) {
        (Some(from_d), Some(last_d)) if from_d == last_d => Some(rel_or_date(from_d, today_d)),
        (Some(from_d), Some(last_d)) => Some(format!(
            "{} \u{2013} {}",
            rel_or_date(from_d, today_d),
            rel_or_date(last_d, today_d),
        )),
        (Some(from_d), None) => Some(format!("since {}", rel_or_date(from_d, today_d))),
        (None, Some(last_d)) => Some(format!("until {}", rel_or_date(last_d, today_d))),
        (None, None) => None,
    }
}

/// Format a brokered tool call into a render-ready [`ToolActivityEntry`].
///
/// Pure, no IO — `app_icon_path` is always `None` here; the async enrich step
/// ([`resolve_app_icon_path`] / [`build_tool_activity_entry`]) fills it in.
/// Faithful port of the frontend's `formatToolActivity`.
///
/// `now_ms` and `utc_offset_minutes` are threaded in (not read from a clock
/// inside) so this stays deterministic + unit-testable; they only feed the
/// `recall_context` range suffix's LOCAL/relative date rendering.
pub(crate) fn format_tool_activity(
    tool: &str,
    params: &Value,
    now_ms: i64,
    utc_offset_minutes: Option<i32>,
) -> ToolActivityEntry {
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
        "recall_context" => {
            let mut label = match read_string_param(params, "query") {
                // Curly double-quotes (U+201C / U+201D) to match `search`.
                Some(query) => format!("Recalling \u{201c}{query}\u{201d}"),
                None => "Recalling what I know about you".to_string(),
            };
            // When the call carries a `from`/`to` activity window, surface it as a
            // ` · <range>` suffix (middot U+00B7). Omitted when neither bound is
            // present or parseable, so the legacy label is byte-identical.
            if let Some(range) = format_recall_range(params, now_ms, utc_offset_minutes) {
                label.push_str(" \u{00b7} ");
                label.push_str(&range);
            }
            ToolActivityEntry {
                kind: "recall_context".to_string(),
                label,
                app: None,
                app_icon_path: None,
            }
        }
        // App-control tools (Workstream A) — fixed labels, no app scope.
        "capture_status" => ToolActivityEntry {
            kind: "app_control".to_string(),
            label: "Checking capture status".to_string(),
            app: None,
            app_icon_path: None,
        },
        "start_capture" => ToolActivityEntry {
            kind: "app_control".to_string(),
            label: "Starting capture".to_string(),
            app: None,
            app_icon_path: None,
        },
        "stop_capture" => ToolActivityEntry {
            kind: "app_control".to_string(),
            label: "Stopping capture".to_string(),
            app: None,
            app_icon_path: None,
        },
        "pause_capture" => ToolActivityEntry {
            kind: "app_control".to_string(),
            label: "Pausing capture".to_string(),
            app: None,
            app_icon_path: None,
        },
        "resume_capture" => ToolActivityEntry {
            kind: "app_control".to_string(),
            label: "Resuming capture".to_string(),
            app: None,
            app_icon_path: None,
        },
        // fetch_url (Workstream B) — fixed label, no app scope.
        "fetch_url" => ToolActivityEntry {
            kind: "fetch_url".to_string(),
            label: "Fetching a page you visited".to_string(),
            app: None,
            app_icon_path: None,
        },
        other => ToolActivityEntry {
            kind: "other".to_string(),
            label: fallback_tool_label(other),
            app: None,
            app_icon_path: None,
        },
    }
}

/// Human "Running …" label for a tool with no bespoke case. MCP connector tools
/// arrive model-namespaced as `mcp__<server>__<tool>`; strip that wire prefix and
/// de-snake the tool so the activity line reads "Running pull request read", not
/// "Running mcp__connector__pull_request_read".
fn fallback_tool_label(tool: &str) -> String {
    if tool.is_empty() {
        return "Working".to_string();
    }
    match super::mcp::parse_mcp_tool_name(tool) {
        // `_`/`-` → spaces, whitespace collapsed (a tool's own `__` survives the
        // parser as one segment, so it de-snakes cleanly here).
        Some((_server, name)) => {
            let words = name.replace(['_', '-'], " ");
            format!("Running {}", words.split_whitespace().collect::<Vec<_>>().join(" "))
        }
        None => format!("Running {tool}"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Unix-ms for a local instant given the offset (minutes). We pass the local
    /// wall-clock fields and back out the UTC ms, so tests read naturally.
    fn local_now_ms(
        year: i32,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        offset_minutes: i32,
    ) -> i64 {
        let date = time::Date::from_calendar_date(
            year,
            time::Month::try_from(month).unwrap(),
            day,
        )
        .unwrap();
        let local = date.with_hms(hour, minute, 0).unwrap().assume_utc();
        // `local` is the wall-clock instant tagged UTC; the true UTC instant is
        // that minus the offset.
        let utc = local - time::Duration::minutes(i64::from(offset_minutes));
        (utc.unix_timestamp_nanos() / 1_000_000) as i64
    }

    // Convenience: most non-recall tests don't care about the clock args.
    const NOW: i64 = 0;
    const NO_OFFSET: Option<i32> = None;

    #[test]
    fn search_with_query_formats_curly_quoted_label() {
        let entry = format_tool_activity("search", &json!({ "query": "rust async" }), NOW, NO_OFFSET);
        assert_eq!(entry.kind, "search");
        assert_eq!(entry.label, "Searching \u{201c}rust async\u{201d}");
        assert!(entry.label.contains("\u{201c}rust async\u{201d}"));
        assert_eq!(entry.app, None);
        assert_eq!(entry.app_icon_path, None);
    }

    #[test]
    fn search_without_query_uses_fallback_label() {
        let entry = format_tool_activity("search", &json!({}), NOW, NO_OFFSET);
        assert_eq!(entry.kind, "search");
        assert_eq!(entry.label, "Searching your captures");
    }

    #[test]
    fn search_blank_query_uses_fallback_label() {
        // Whitespace-only query trims to empty → fallback.
        let entry = format_tool_activity("search", &json!({ "query": "   " }), NOW, NO_OFFSET);
        assert_eq!(entry.label, "Searching your captures");
    }

    #[test]
    fn search_with_app_param_sets_app_but_not_icon() {
        let entry = format_tool_activity(
            "search",
            &json!({ "query": "notes", "app": "com.example.app" }),
            NOW,
            NO_OFFSET,
        );
        assert_eq!(entry.app, Some("com.example.app".to_string()));
        // Pure stage never resolves an icon.
        assert_eq!(entry.app_icon_path, None);
    }

    #[test]
    fn timeline_label_and_kind() {
        let entry = format_tool_activity("timeline", &json!({ "app": "Zen Browser" }), NOW, NO_OFFSET);
        assert_eq!(entry.kind, "timeline");
        assert_eq!(entry.label, "Scanning timeline");
        assert_eq!(entry.app, Some("Zen Browser".to_string()));
    }

    #[test]
    fn show_text_label_kind_and_no_app() {
        let entry = format_tool_activity("show_text", &json!({ "app": "ignored" }), NOW, NO_OFFSET);
        assert_eq!(entry.kind, "show_text");
        assert_eq!(entry.label, "Reading a capture");
        assert_eq!(entry.app, None);
    }

    #[test]
    fn recall_context_with_query_formats_curly_quoted_label() {
        let entry = format_tool_activity(
            "recall_context",
            &json!({ "query": "what do I work on" }),
            NOW,
            NO_OFFSET,
        );
        assert_eq!(entry.kind, "recall_context");
        assert_eq!(entry.label, "Recalling \u{201c}what do I work on\u{201d}");
        assert_eq!(entry.app, None);
        assert_eq!(entry.app_icon_path, None);
    }

    #[test]
    fn recall_context_without_query_uses_fallback_label() {
        let entry = format_tool_activity("recall_context", &json!({}), NOW, NO_OFFSET);
        assert_eq!(entry.kind, "recall_context");
        assert_eq!(entry.label, "Recalling what I know about you");
    }

    #[test]
    fn recall_context_today_single_local_day_ist() {
        // The exact bug from the report: an IST (+5:30 = 330) user asking about
        // "today" yields a half-open UTC window for the single local day
        // 2026-06-13. It must collapse to "Today", not "2026-06-12 – 2026-06-13".
        let offset = 330;
        let now = local_now_ms(2026, 6, 13, 10, 0, offset);
        let entry = format_tool_activity(
            "recall_context",
            &json!({
                "query": "what did I work on today",
                "from": "2026-06-12T18:30:00Z",
                "to": "2026-06-13T18:30:00Z",
            }),
            now,
            Some(offset),
        );
        assert_eq!(
            entry.label,
            "Recalling \u{201c}what did I work on today\u{201d} \u{00b7} Today"
        );
    }

    #[test]
    fn recall_context_yesterday_single_local_day_ist() {
        let offset = 330;
        let now = local_now_ms(2026, 6, 13, 10, 0, offset);
        // The single local day 2026-06-12 (yesterday) in IST.
        let entry = format_tool_activity(
            "recall_context",
            &json!({
                "query": "yesterday",
                "from": "2026-06-11T18:30:00Z",
                "to": "2026-06-12T18:30:00Z",
            }),
            now,
            Some(offset),
        );
        assert_eq!(
            entry.label,
            "Recalling \u{201c}yesterday\u{201d} \u{00b7} Yesterday"
        );
    }

    #[test]
    fn recall_context_multi_day_range_relative_and_iso_endpoints() {
        // UTC user (offset 0). `from` = yesterday (2026-06-12), inclusive last
        // day = an older date → "Yesterday – 2026-06-10" is impossible ordering,
        // so use from=older ISO, last=Yesterday.
        let offset = 0;
        let now = local_now_ms(2026, 6, 13, 10, 0, offset);
        let entry = format_tool_activity(
            "recall_context",
            &json!({
                "query": "this stretch",
                "from": "2026-06-10T00:00:00Z",
                "to": "2026-06-13T00:00:00Z",
            }),
            now,
            Some(offset),
        );
        // from = 2026-06-10 (ISO), inclusive last = 2026-06-12 (Yesterday).
        assert_eq!(
            entry.label,
            "Recalling \u{201c}this stretch\u{201d} \u{00b7} 2026-06-10 \u{2013} Yesterday"
        );
    }

    #[test]
    fn recall_context_offset_none_falls_back_to_iso_utc() {
        // Offset unknown → no relativization, bare UTC dates, no "Today".
        let now = local_now_ms(2026, 6, 13, 10, 0, 0);
        let entry = format_tool_activity(
            "recall_context",
            &json!({
                "query": "what did I work on today",
                "from": "2026-06-13T00:00:00Z",
                "to": "2026-06-14T00:00:00Z",
            }),
            now,
            None,
        );
        assert_eq!(
            entry.label,
            "Recalling \u{201c}what did I work on today\u{201d} \u{00b7} 2026-06-13 \u{2013} 2026-06-14"
        );
    }

    #[test]
    fn recall_context_older_single_day_renders_local_iso() {
        // A single local day ~10 days ago → bare ISO local date, no relative word.
        let offset = 330;
        let now = local_now_ms(2026, 6, 13, 10, 0, offset);
        let entry = format_tool_activity(
            "recall_context",
            &json!({
                "query": "a while ago",
                "from": "2026-06-02T18:30:00Z",
                "to": "2026-06-03T18:30:00Z",
            }),
            now,
            Some(offset),
        );
        // Single local day 2026-06-03 in IST.
        assert_eq!(
            entry.label,
            "Recalling \u{201c}a while ago\u{201d} \u{00b7} 2026-06-03"
        );
    }

    #[test]
    fn recall_context_with_only_from_bound_uses_since_tail() {
        let offset = 0;
        let now = local_now_ms(2026, 6, 13, 10, 0, offset);
        let entry = format_tool_activity(
            "recall_context",
            &json!({ "query": "recent work", "from": "2026-06-10T00:00:00Z" }),
            now,
            Some(offset),
        );
        assert_eq!(
            entry.label,
            "Recalling \u{201c}recent work\u{201d} \u{00b7} since 2026-06-10"
        );
    }

    #[test]
    fn recall_context_only_from_offset_none_uses_now_tail() {
        // Offset unknown legacy fallback keeps the `from \u{2013} now` shape.
        let entry = format_tool_activity(
            "recall_context",
            &json!({ "query": "recent work", "from": "2026-06-13T00:00:00Z" }),
            NOW,
            None,
        );
        assert_eq!(
            entry.label,
            "Recalling \u{201c}recent work\u{201d} \u{00b7} 2026-06-13 \u{2013} now"
        );
    }

    #[test]
    fn recall_context_without_time_window_keeps_legacy_label() {
        // No bounds → label is byte-identical to the pre-feature label.
        let entry = format_tool_activity(
            "recall_context",
            &json!({ "query": "what do I work on" }),
            NOW,
            NO_OFFSET,
        );
        assert_eq!(entry.label, "Recalling \u{201c}what do I work on\u{201d}");
    }

    #[test]
    fn recall_context_with_unparseable_bound_drops_it_from_label() {
        // A bad `to` is ignored; only the valid `from` shapes the suffix
        // (offset-aware → `since`).
        let offset = 0;
        let now = local_now_ms(2026, 6, 13, 10, 0, offset);
        let entry = format_tool_activity(
            "recall_context",
            &json!({ "query": "q", "from": "2026-06-10T00:00:00Z", "to": "garbage" }),
            now,
            Some(offset),
        );
        assert_eq!(
            entry.label,
            "Recalling \u{201c}q\u{201d} \u{00b7} since 2026-06-10"
        );
    }

    #[test]
    fn app_control_tools_have_fixed_labels_and_no_app_scope() {
        let stop = format_tool_activity("stop_capture", &json!({}), NOW, NO_OFFSET);
        assert_eq!(stop.kind, "app_control");
        assert_eq!(stop.label, "Stopping capture");
        assert_eq!(stop.app, None);

        let status = format_tool_activity("capture_status", &json!({}), NOW, NO_OFFSET);
        assert_eq!(status.kind, "app_control");
        assert_eq!(status.label, "Checking capture status");

        assert_eq!(
            format_tool_activity("start_capture", &json!({}), NOW, NO_OFFSET).label,
            "Starting capture"
        );
        assert_eq!(
            format_tool_activity("pause_capture", &json!({}), NOW, NO_OFFSET).label,
            "Pausing capture"
        );
        assert_eq!(
            format_tool_activity("resume_capture", &json!({}), NOW, NO_OFFSET).label,
            "Resuming capture"
        );
    }

    #[test]
    fn fetch_url_has_fixed_label_and_no_app_scope() {
        let entry = format_tool_activity("fetch_url", &json!({ "opaqueId": "op-1" }), NOW, NO_OFFSET);
        assert_eq!(entry.kind, "fetch_url");
        assert_eq!(entry.label, "Fetching a page you visited");
        assert_eq!(entry.app, None);
        assert_eq!(entry.app_icon_path, None);
    }

    #[test]
    fn unknown_tool_runs_named() {
        let entry = format_tool_activity("foo", &json!({}), NOW, NO_OFFSET);
        assert_eq!(entry.kind, "other");
        assert_eq!(entry.label, "Running foo");
        assert_eq!(entry.app, None);
    }

    #[test]
    fn mcp_tool_name_is_humanized_not_shown_raw() {
        let entry =
            format_tool_activity("mcp__connector__pull_request_read", &json!({}), NOW, NO_OFFSET);
        assert_eq!(entry.kind, "other");
        assert_eq!(entry.label, "Running pull request read");

        // A tool name that itself contains `__` de-snakes without doubling spaces.
        assert_eq!(
            format_tool_activity("mcp__srv__list__things", &json!({}), NOW, NO_OFFSET).label,
            "Running list things"
        );
        // Malformed mcp-ish names (bare prefix) fall through to the raw name.
        assert_eq!(
            format_tool_activity("mcp__github", &json!({}), NOW, NO_OFFSET).label,
            "Running mcp__github"
        );
    }

    #[test]
    fn empty_tool_falls_back_to_working() {
        let entry = format_tool_activity("", &json!({}), NOW, NO_OFFSET);
        assert_eq!(entry.kind, "other");
        assert_eq!(entry.label, "Working");
    }

    #[test]
    fn entry_serializes_camel_case_and_skips_absent_icon_path() {
        let entry = format_tool_activity(
            "search",
            &json!({ "query": "x", "app": "com.example.app" }),
            NOW,
            NO_OFFSET,
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
        let entry = format_tool_activity("show_text", &json!({}), NOW, NO_OFFSET);
        let value = serde_json::to_value(&entry).unwrap();
        let obj = value.as_object().unwrap();
        assert!(!obj.contains_key("app"));
        assert!(!obj.contains_key("appIconPath"));
    }
}
