//! Pure view logic for the CLI Access request window, ported from
//! `lib/access/request-view.ts`. The request is app-core's
//! `PendingCliAccessRequestDto` (camelCase JSON), the same map the Svelte stub serves.

use serde_json::Value;

/// Narrow → wide. Anything below the request's minimum is disabled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
    LastDay,
    Last7Days,
    AllRetained,
}

impl Scope {
    pub const ALL: [Scope; 3] = [Scope::LastDay, Scope::Last7Days, Scope::AllRetained];

    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "lastDay" => Some(Self::LastDay),
            "last7Days" => Some(Self::Last7Days),
            "allRetained" => Some(Self::AllRetained),
            _ => None,
        }
    }
    pub fn wire(self) -> &'static str {
        match self {
            Self::LastDay => "lastDay",
            Self::Last7Days => "last7Days",
            Self::AllRetained => "allRetained",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::LastDay => "Last day",
            Self::Last7Days => "Last 7 days",
            Self::AllRetained => "All retained",
        }
    }
    /// Noun phrase for prose ("your last 24 hours").
    pub fn prose(self) -> &'static str {
        match self {
            Self::LastDay => "your last 24 hours",
            Self::Last7Days => "your last 7 days",
            Self::AllRetained => "your entire retained history",
        }
    }
    pub fn desc(self) -> &'static str {
        match self {
            Self::LastDay => "Only text captured in the last 24 hours.",
            Self::Last7Days => "Text captured in the last 7 days.",
            Self::AllRetained => "Your entire retained capture history.",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub id: String,
    pub tool: String,
    pub source: String,
    pub command: String,
    pub minimum: Scope,
    pub preferred: Scope,
    /// What the tool's standing permission already carries.
    pub current: Option<Scope>,
    pub created_at: String,
}

impl Request {
    pub fn from_json(v: &Value) -> Option<Self> {
        let s = |k: &str| v[k].as_str().map(str::to_string);
        Some(Self {
            id: s("requestId")?,
            tool: v["client"]["label"].as_str()?.to_string(),
            source: v["client"]["source"].as_str().unwrap_or("").to_string(),
            command: s("command").unwrap_or_default(),
            minimum: Scope::from_wire(v["minimumScope"].as_str()?)?,
            preferred: Scope::from_wire(v["preferredScope"].as_str()?)?,
            current: v["currentScope"].as_str().and_then(Scope::from_wire),
            created_at: s("createdAt").unwrap_or_default(),
        })
    }

    /// A widen: the tool holds a permission and asks for more than it carries.
    pub fn is_widen(&self) -> bool {
        self.current.is_some_and(|c| c < self.preferred)
    }

    /// The quiet tag after an option's title.
    pub fn tag(&self, scope: Scope) -> &'static str {
        if Some(scope) == self.current {
            "current"
        } else if scope == self.preferred && self.preferred != self.minimum {
            "asked for"
        } else if scope == self.minimum && (self.current.is_some() || self.minimum > Scope::LastDay)
        {
            "needed"
        } else {
            ""
        }
    }

    pub fn disabled(&self, scope: Scope) -> bool {
        scope < self.minimum
    }

    /// The muted line under the scope list ("" = none).
    pub fn hint(&self, selected: Scope) -> String {
        if let (true, Some(c)) = (self.is_widen(), self.current) {
            return format!("Deny keeps {} \u{2014} this command fails.", c.label());
        }
        if selected < self.preferred {
            return format!(
                "{} asked for {}. Start narrow \u{2014} it asks again if it needs more.",
                self.tool,
                self.preferred.prose()
            );
        }
        String::new()
    }

    pub fn allow_label(&self, selected: Scope) -> String {
        if selected == Scope::AllRetained {
            "Allow full-history access".into()
        } else if self.is_widen() {
            format!("Allow {}", selected.label().to_lowercase())
        } else {
            "Allow access".into()
        }
    }

    /// Allowing SETS the scope: a selection under what the tool holds takes access away.
    pub fn narrows(&self, selected: Scope) -> Option<Scope> {
        self.current.filter(|c| *c > selected)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiptKind {
    Granted,
    Blocked,
}

/// What the click did, snapshotted so the receipt survives the teardown.
#[derive(Clone, Debug, PartialEq)]
pub struct Receipt {
    pub kind: ReceiptKind,
    pub tool: String,
    pub scope: Scope,
    pub current: Option<Scope>,
    pub widen: bool,
}

/// Identity provenance → (chip, sentence, warn).
pub fn identity_source(source: &str) -> (&'static str, &'static str, bool) {
    match source {
        "explicit" => ("Declared", "Identity declared by the tool", false),
        "env" => ("From env", "Identity from an environment variable", false),
        "inferred" => ("Inferred", "Identity inferred from the process", true),
        _ => ("Unnamed", "No identity provided", true),
    }
}

/// "mnema search" (bold) + the rest of the command line.
pub fn split_command(command: &str) -> (String, String) {
    let t = command.trim();
    match t.find(char::is_whitespace) {
        None => (format!("mnema {t}").trim().to_string(), String::new()),
        Some(i) => (format!("mnema {}", &t[..i]), t[i + 1..].to_string()),
    }
}

/// How long the request has waited (an honesty cue, not a countdown).
pub fn request_age(created_at: &str, now_ms: i64) -> Option<String> {
    let at =
        time::OffsetDateTime::parse(created_at, &time::format_description::well_known::Rfc3339)
            .ok()?;
    let created_ms = (at.unix_timestamp_nanos() / 1_000_000) as i64;
    let secs = ((now_ms - created_ms).max(0) as f64 / 1000.).round() as i64;
    Some(if secs < 5 {
        "just now".into()
    } else if secs < 60 {
        format!("Requested {secs}s ago")
    } else {
        format!("Requested {} min ago", (secs as f64 / 60.).round() as i64)
    })
}

/// `fitAccessWindowHeight`: a new view fits exactly; within one view the window only grows.
pub fn fit_height(current: f32, natural: f32, same_view: bool, max: f32) -> Option<f32> {
    let target = if same_view {
        current.max(natural)
    } else {
        natural
    }
    .min(max)
    .round();
    ((target - current).abs() >= 2.).then_some(target)
}

/// "no pending CLI Access request" means the rendered request can't be answered any more.
pub fn action_error(message: &str) -> String {
    if message.contains("no pending CLI Access request") {
        "This request is no longer valid \u{2014} the requesting tool stopped waiting. You can close this window.".into()
    } else {
        message.to_string()
    }
}

/// The rendered request is gone: only Close is left.
pub fn is_gone(message: &str) -> bool {
    message.contains("blocked in Settings") || message.contains("no pending CLI Access request")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn req(min: &str, pref: &str, current: Option<&str>) -> Request {
        Request::from_json(&json!({
            "requestId": "r1", "client": { "label": "Codex", "source": "inferred" },
            "command": "timeline --from 2026-10-04", "minimumScope": min,
            "preferredScope": pref, "currentScope": current, "createdAt": "2026-11-04T09:01:30Z"
        }))
        .unwrap()
    }

    #[test]
    fn widen_tags_hint_and_label_match_the_web() {
        let r = req("last7Days", "last7Days", Some("lastDay"));
        assert!(r.is_widen());
        assert_eq!(r.tag(Scope::LastDay), "current");
        assert_eq!(r.tag(Scope::Last7Days), "needed");
        assert!(r.disabled(Scope::LastDay));
        assert_eq!(
            r.hint(Scope::Last7Days),
            "Deny keeps Last day \u{2014} this command fails."
        );
        assert_eq!(r.allow_label(Scope::Last7Days), "Allow last 7 days");
        assert_eq!(
            r.allow_label(Scope::AllRetained),
            "Allow full-history access"
        );
    }

    #[test]
    fn a_fresh_request_starts_narrow_and_names_the_ask() {
        let r = req("lastDay", "last7Days", None);
        assert!(!r.is_widen());
        assert_eq!(r.tag(Scope::Last7Days), "asked for");
        assert_eq!(r.tag(Scope::LastDay), "");
        assert!(r
            .hint(Scope::LastDay)
            .starts_with("Codex asked for your last 7 days."));
        assert_eq!(r.hint(Scope::Last7Days), "");
        assert_eq!(r.allow_label(Scope::LastDay), "Allow access");
        assert_eq!(r.narrows(Scope::LastDay), None);
        let held = req("lastDay", "lastDay", Some("allRetained"));
        assert_eq!(held.narrows(Scope::LastDay), Some(Scope::AllRetained));
    }

    #[test]
    fn command_age_and_fit() {
        assert_eq!(
            split_command("search --query x"),
            ("mnema search".into(), "--query x".into())
        );
        assert_eq!(
            split_command("status"),
            ("mnema status".into(), String::new())
        );
        let at = 1_793_782_920_000; // 2026-11-04T09:02:00Z
        assert_eq!(
            request_age("2026-11-04T09:01:30Z", at).as_deref(),
            Some("Requested 30s ago")
        );
        assert_eq!(
            request_age("2026-11-04T09:01:58Z", at).as_deref(),
            Some("just now")
        );
        assert_eq!(request_age("bad", at), None);
        assert_eq!(fit_height(600., 300., false, 900.), Some(300.));
        assert_eq!(fit_height(600., 300., true, 900.), None);
        assert_eq!(fit_height(600., 1000., true, 900.), Some(900.));
    }
}
