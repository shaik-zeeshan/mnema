//! Chat scope (CH3): the time range + "About you" switch a Chat turn carries.
//!
//! The scope governs *captured history* only: the brokered data tools' `from`/
//! `to` are clamped into it before they reach the broker, and `recall_context`
//! (Mnema's notes about the user) is refused when About you is off. App control,
//! `fetch_url` and MCP connectors are not limited by it (IMPLEMENTATION.md §5 c);
//! `show_text` only takes opaque ids a scoped result handed out.

use capture_types::AskAiScope;
use serde_json::Value;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// Clamp one tool call's window into `scope`: a missing bound is filled with the
/// scope's (`search` / `recall_context` take optional bounds), a bound past the
/// scope is narrowed (`timeline` / `activities` require theirs). `Ok(None)` for
/// a tool the scope doesn't govern; `Err` (shown to the model) when the call
/// asks for a window entirely outside the scope, or for `recall_context` with
/// About you off.
pub(super) fn clamp_tool_range(
    scope: &AskAiScope,
    tool: &str,
    from: Option<i64>,
    to: Option<i64>,
) -> Result<Option<(i64, i64)>, String> {
    match tool {
        "recall_context" if !scope.about_you => {
            return Err(
                "recall_context is off for this chat: the user turned off \"About you\"."
                    .to_string(),
            )
        }
        "search" | "recall_context" | "timeline" | "activities" => {}
        _ => return Ok(None),
    }
    let from = from.map_or(scope.from_ms, |from| from.max(scope.from_ms));
    let to = to.map_or(scope.to_ms, |to| to.min(scope.to_ms));
    if from > to {
        return Err(format!(
            "That window is outside this chat's scope ({} – {} UTC); only captures inside it can be read.",
            rfc3339(scope.from_ms),
            rfc3339(scope.to_ms),
        ));
    }
    Ok(Some((from, to)))
}

/// Apply [`clamp_tool_range`] to a tool call's camelCase params (RFC3339 `from`/
/// `to`). An unparseable bound counts as omitted, as the broker would ignore it.
pub(super) fn scope_tool_params(
    scope: Option<&AskAiScope>,
    tool: &str,
    mut params: Value,
) -> Result<Value, String> {
    let Some(scope) = scope else {
        return Ok(params);
    };
    let bound = |key: &str| {
        params
            .get(key)
            .and_then(Value::as_str)
            .and_then(|raw| OffsetDateTime::parse(raw, &Rfc3339).ok())
            .map(|dt| (dt.unix_timestamp_nanos() / 1_000_000) as i64)
    };
    let (from, to) = (bound("from"), bound("to"));
    if let (Some((from, to)), Some(object)) = (
        clamp_tool_range(scope, tool, from, to)?,
        params.as_object_mut(),
    ) {
        object.insert("from".to_string(), Value::String(rfc3339(from)));
        object.insert("to".to_string(), Value::String(rfc3339(to)));
    }
    Ok(params)
}

/// The one prompt line naming the scope (local wall clock when known).
pub(super) fn scope_prompt_line(scope: &AskAiScope, utc_offset_minutes: Option<i32>) -> String {
    let local = |ms: i64| {
        let offset = i64::from(utc_offset_minutes.unwrap_or(0));
        super::format_ymd_hm(at(ms) + time::Duration::minutes(offset))
    };
    let zone = if utc_offset_minutes.is_some() {
        "local"
    } else {
        "UTC"
    };
    let about = if scope.about_you {
        ""
    } else {
        " About you is off: you have no `recall_context` and no notes about the user."
    };
    format!(
        "Scope: the user limited this chat to captures from {} to {} ({zone}); every \
`search`/`timeline`/`activities`/`recall_context` window is clamped to it, so say so when the \
question needs other dates.{about}\n\n",
        local(scope.from_ms),
        local(scope.to_ms),
    )
}

fn at(ms: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

fn rfc3339(ms: i64) -> String {
    at(ms).format(&Rfc3339).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SCOPE: AskAiScope = AskAiScope {
        from_ms: 1_000,
        to_ms: 2_000,
        about_you: true,
    };

    #[test]
    fn clamp_fills_narrows_and_rejects() {
        // Omitted bounds are filled (search / recall_context).
        assert_eq!(
            clamp_tool_range(&SCOPE, "search", None, None),
            Ok(Some((1_000, 2_000)))
        );
        // Wider bounds are narrowed; inner ones kept (timeline / activities).
        assert_eq!(
            clamp_tool_range(&SCOPE, "timeline", Some(0), Some(9_000)),
            Ok(Some((1_000, 2_000)))
        );
        assert_eq!(
            clamp_tool_range(&SCOPE, "activities", Some(1_200), Some(1_500)),
            Ok(Some((1_200, 1_500)))
        );
        // A window entirely outside the scope is refused, not inverted.
        assert!(clamp_tool_range(&SCOPE, "activities", Some(3_000), Some(4_000)).is_err());
        // Tools the scope doesn't govern pass through.
        assert_eq!(clamp_tool_range(&SCOPE, "fetch_url", None, None), Ok(None));
        // About you off refuses recall_context.
        let off = AskAiScope {
            about_you: false,
            ..SCOPE
        };
        assert!(clamp_tool_range(&off, "recall_context", None, None).is_err());
    }

    #[test]
    fn scope_tool_params_rewrites_rfc3339_bounds() {
        let scope = AskAiScope {
            from_ms: 1_791_158_400_000, // 2026-10-05T00:00:00Z
            to_ms: 1_791_763_199_999,   // 2026-10-11T23:59:59.999Z
            about_you: true,
        };
        let params = json!({ "from": "2026-01-01T00:00:00Z", "to": "2026-10-06T00:00:00Z" });
        let out = scope_tool_params(Some(&scope), "timeline", params.clone()).unwrap();
        assert_eq!(out["from"], json!("2026-10-05T00:00:00Z"));
        assert_eq!(out["to"], json!("2026-10-06T00:00:00Z"));
        let out = scope_tool_params(Some(&scope), "search", json!({ "query": "q" })).unwrap();
        assert_eq!(out["to"], json!("2026-10-11T23:59:59.999Z"));
        // No scope = today's behaviour exactly.
        assert_eq!(
            scope_tool_params(None, "timeline", params.clone()).unwrap(),
            params
        );
    }
}
