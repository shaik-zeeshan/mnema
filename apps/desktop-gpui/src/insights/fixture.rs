//! Parity fixtures -> Insights. `apps/desktop/parity/fixtures/insights/<set>.json`
//! is what the Svelte stub serves (command -> JSON, one answer per command
//! whatever the arguments); `FixtureBackend` answers from the same map through
//! the real types, so both sides draw one dossier. It records every write, for
//! the Undo timing tests.

use std::path::PathBuf;
use std::sync::Mutex;

use app_core::user_context::commands::AuthoredContextRow;
use capture_types::{
    Activity, ActivityMeta, AuthoredContext, Conclusion, DismissedView, SubjectView,
    UsageChartsResponse, UserContextDigestResult,
};
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::data::{Backend, Res, StripFrame};
use super::model::{AiStatus, Status, Tab};
use crate::shell::fixture::{get, load};

/// A parity state: `(name, fixture set, tab, opens)`.
pub struct State {
    pub name: &'static str,
    pub set: &'static str,
    pub tab: Tab,
    pub open: Open,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Open {
    None,
    /// The subject page for the dossier's first subject.
    Subject,
    /// The Journal's newest card with its receipt open.
    Receipt,
    /// Overview's period control switched to this unit, its calendar clicked open.
    Period(crate::ui::calendar::Unit),
}

/// The goldens and parity states, in `surfaces.ts` order.
pub const STATES: [State; 13] = [
    State {
        name: "overview",
        set: "populated",
        tab: Tab::Overview,
        open: Open::None,
    },
    State {
        name: "overview-empty",
        set: "empty",
        tab: Tab::Overview,
        open: Open::None,
    },
    State {
        name: "journal",
        set: "populated",
        tab: Tab::Journal,
        open: Open::None,
    },
    State {
        name: "journal-empty",
        set: "empty",
        tab: Tab::Journal,
        open: Open::None,
    },
    State {
        name: "subjects",
        set: "populated",
        tab: Tab::Subjects,
        open: Open::None,
    },
    State {
        name: "subjects-empty",
        set: "empty",
        tab: Tab::Subjects,
        open: Open::None,
    },
    State {
        name: "context",
        set: "populated",
        tab: Tab::Context,
        open: Open::None,
    },
    State {
        name: "context-empty",
        set: "empty",
        tab: Tab::Context,
        open: Open::None,
    },
    State {
        name: "subject",
        set: "populated",
        tab: Tab::Subjects,
        open: Open::Subject,
    },
    State {
        name: "receipt",
        set: "populated",
        tab: Tab::Journal,
        open: Open::Receipt,
    },
    State {
        name: "period-day-open",
        set: "populated",
        tab: Tab::Overview,
        open: Open::Period(crate::ui::calendar::Unit::Day),
    },
    State {
        name: "period-week-open",
        set: "populated",
        tab: Tab::Overview,
        open: Open::Period(crate::ui::calendar::Unit::Week),
    },
    State {
        name: "period-month-open",
        set: "populated",
        tab: Tab::Overview,
        open: Open::Period(crate::ui::calendar::Unit::Month),
    },
];

pub struct FixtureBackend {
    map: Value,
    /// Writes in order: `dismiss:<id>`, `pin:<id>:<bool>`, `add:<text>`, …
    pub calls: Mutex<Vec<String>>,
}

impl FixtureBackend {
    pub fn load(set: &str) -> anyhow::Result<Self> {
        Ok(Self {
            map: load("insights", set)?,
            calls: Mutex::new(Vec::new()),
        })
    }

    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn val<T: DeserializeOwned>(&self, cmd: &str) -> Option<T> {
        get(&self.map, &[cmd]).ok().flatten()
    }

    fn list<T: DeserializeOwned>(&self, cmd: &str) -> Vec<T> {
        self.val(cmd).unwrap_or_default()
    }

    fn record(&self, call: String) -> Res<()> {
        self.calls.lock().unwrap().push(call);
        Ok(())
    }
}

impl Backend for FixtureBackend {
    fn status(&self) -> Status {
        let authored: Vec<AuthoredContextRow> = self.list("list_user_context_authored");
        Status {
            loaded: true,
            ai: self.val::<AiStatus>("get_ai_runtime_status"),
            ctx: self.val("get_user_context_status"),
            settings: self.val("get_recording_settings"),
            authored: Some(authored.len()),
        }
    }
    fn usage(&self, _: i64, _: i64) -> Res<UsageChartsResponse> {
        self.val("get_usage_charts")
            .ok_or_else(|| "no usage in the fixture".into())
    }
    fn presence(&self, _: i64, _: i64) -> Option<(bool, bool)> {
        let v: Value = self.val("capture_presence")?;
        Some((v["screen"].as_bool()?, v["audio"].as_bool()?))
    }
    fn capture_hours(&self, _: i64, _: i64, _: i32) -> Vec<i64> {
        self.list("get_capture_hours")
    }
    fn activities(&self, _: i64, _: i64) -> Res<Vec<Activity>> {
        Ok(self.list("list_user_context_activities"))
    }
    fn recent_activities(&self, _: i64, offset: i64) -> Vec<Activity> {
        if offset > 0 {
            return Vec::new();
        }
        self.list("list_user_context_activities")
    }
    fn activity_meta(&self, _: i64, _: i64) -> Vec<ActivityMeta> {
        self.list("list_user_context_activity_meta")
    }
    fn failed_windows(&self, _: i64, _: i64) -> Vec<(i64, i64)> {
        self.list("list_failed_derivation_windows")
    }
    fn conclusions(&self, _: Option<(i64, i64)>) -> Res<Vec<Conclusion>> {
        Ok(self.list("list_user_context_conclusions"))
    }
    fn digest(&self, _: &str, _: i64, _: i64, _: bool) -> Res<UserContextDigestResult> {
        Ok(self
            .val("get_user_context_digest")
            .unwrap_or(UserContextDigestResult {
                digest: None,
                reason: None,
            }))
    }
    fn subject(&self, _: &str) -> Res<SubjectView> {
        self.val("get_user_context_subject")
            .ok_or_else(|| "no subject in the fixture".into())
    }
    fn frames(&self, _: i64, _: i64) -> Res<Vec<StripFrame>> {
        let list: Vec<Value> = self.list("list_frame_summaries_in_range");
        Ok(list
            .iter()
            .filter_map(|f| {
                let at = time::OffsetDateTime::parse(
                    f["capturedAt"].as_str()?,
                    &time::format_description::well_known::Rfc3339,
                )
                .ok()?;
                Some((
                    f["id"].as_i64()?,
                    (at.unix_timestamp_nanos() / 1_000_000) as i64,
                ))
            })
            .collect())
    }
    fn audio(&self, _: i64, _: i64) -> Vec<(i64, i64)> {
        Vec::new()
    }
    fn frame_preview(&self, _: i64) -> Option<PathBuf> {
        let v: Value = self.val("get_frame_preview")?;
        Some(PathBuf::from(v["filePath"].as_str()?))
    }
    fn authored(&self) -> Res<Vec<AuthoredContextRow>> {
        Ok(self.list("list_user_context_authored"))
    }
    fn dismissed(&self) -> Res<Vec<DismissedView>> {
        Ok(self.list("user_context_list_dismissed"))
    }
    fn dismiss(&self, id: i64) -> Res<()> {
        self.record(format!("dismiss:{id}"))
    }
    fn set_pinned(&self, id: i64, pinned: bool) -> Res<()> {
        self.record(format!("pin:{id}:{pinned}"))
    }
    fn add_authored(&self, text: &str, topic: Option<&str>) -> Res<AuthoredContext> {
        self.record(format!("add:{text}"))?;
        Ok(AuthoredContext {
            id: 900,
            text: text.into(),
            topic: topic.map(Into::into),
            created_at_ms: 0,
            updated_at_ms: 0,
        })
    }
    fn update_authored(&self, id: i64, text: &str, _: Option<&str>) -> Res<()> {
        self.record(format!("update:{id}:{text}"))
    }
    fn delete_authored(&self, id: i64) -> Res<()> {
        self.record(format!("delete:{id}"))
    }
    fn restore_dismissed(&self, subject: &str, _: &str) -> Res<()> {
        self.record(format!("restore:{subject}"))
    }
    fn correct_category(&self, id: i64, c: capture_types::ActivityCategory) -> Res<()> {
        self.record(format!("category:{id}:{c:?}"))
    }
}
