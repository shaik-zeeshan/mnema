//! Insights' one backend door. Every read and write is a synchronous call the
//! view runs on the background executor; `CoreBackend` answers from AppCore
//! (user_context, usage_charts, app_infra and the AI status all live there),
//! and `fixture::FixtureBackend` from the parity map for goldens and tests.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use app_core::recording_settings::{current_recording_settings, RecordingSettingsState};
use app_core::user_context::commands::{self as uc, AuthoredContextRow};
use app_core::{AppCore, AppInfraState};
use capture_types::{
    Activity, AuthoredContext, Conclusion, SubjectView, UsageChartsResponse,
    UserContextDigestResult,
};
use serde::de::DeserializeOwned;
use serde_json::json;

use super::model::{AiStatus, ContextData, Histories, Status, SubjectsData};

pub type Res<T> = Result<T, String>;

/// A frame on a receipt's strip: `(id, captured at ms)`.
pub type StripFrame = (i64, i64);

/// Every read answers "nothing" and every write fails unless overridden (`NoBackend`).
#[allow(unused_variables)]
pub trait Backend: Send + Sync + 'static {
    fn status(&self) -> Status {
        Status::default()
    }
    fn usage(&self, start: i64, end: i64) -> Res<UsageChartsResponse> {
        Err("no app core".into())
    }
    fn presence(&self, start: i64, end: i64) -> Option<(bool, bool)> {
        None
    }
    fn capture_hours(&self, start: i64, end: i64, offset_min: i32) -> Vec<i64> {
        Vec::new()
    }
    fn activities(&self, start: i64, end: i64) -> Res<Vec<Activity>> {
        Err("no app core".into())
    }
    /// `list_user_context_activities { limit, offset }` (newest first).
    fn recent_activities(&self, limit: i64, offset: i64) -> Vec<Activity> {
        Vec::new()
    }
    fn activity_meta(&self, start: i64, end: i64) -> Vec<capture_types::ActivityMeta> {
        Vec::new()
    }
    fn failed_windows(&self, start: i64, end: i64) -> Vec<(i64, i64)> {
        Vec::new()
    }
    fn conclusions(&self, range: Option<(i64, i64)>) -> Res<Vec<Conclusion>> {
        Err("no app core".into())
    }
    fn digest(
        &self,
        kind: &str,
        start: i64,
        end: i64,
        force: bool,
    ) -> Res<UserContextDigestResult> {
        Err("no app core".into())
    }
    fn subject(&self, subject: &str) -> Res<SubjectView> {
        Err("no app core".into())
    }
    fn frames(&self, start: i64, end: i64) -> Res<Vec<StripFrame>> {
        Err("no app core".into())
    }
    fn audio(&self, start: i64, end: i64) -> Vec<(i64, i64)> {
        Vec::new()
    }
    /// `list_audio_segments` over the span, for a receipt's spoken turns.
    fn audio_segments(&self, start: i64, end: i64) -> Res<Vec<super::journal::audio::SegRow>> {
        Err("no app core".into())
    }
    fn frame_preview(&self, frame_id: i64) -> Option<PathBuf> {
        None
    }
    fn authored(&self) -> Res<Vec<AuthoredContextRow>> {
        Err("no app core".into())
    }
    fn dismissed(&self) -> Res<Vec<capture_types::DismissedView>> {
        Err("no app core".into())
    }
    fn dismiss(&self, id: i64) -> Res<()> {
        Err("no app core".into())
    }
    fn set_pinned(&self, id: i64, pinned: bool) -> Res<()> {
        Err("no app core".into())
    }
    fn add_authored(&self, text: &str, topic: Option<&str>) -> Res<AuthoredContext> {
        Err("no app core".into())
    }
    fn update_authored(&self, id: i64, text: &str, topic: Option<&str>) -> Res<()> {
        Err("no app core".into())
    }
    fn delete_authored(&self, id: i64) -> Res<()> {
        Err("no app core".into())
    }
    fn restore_dismissed(&self, subject: &str, statement: &str) -> Res<()> {
        Err("no app core".into())
    }
    /// `resolve_app_icons`: (requested bundle id or name, icon PNG); a miss is absent.
    fn app_icons(&self, apps: &[String]) -> Vec<(String, PathBuf)> {
        Vec::new()
    }
    /// `user_context_correct_activity_category` (`None` = Uncategorized).
    fn correct_category(
        &self,
        id: i64,
        category: Option<capture_types::ActivityCategory>,
    ) -> Res<()> {
        Err("no app core".into())
    }
    /// `user_context_correct_activity_focus` (`None` = no focus read).
    fn correct_focus(&self, id: i64, focus: Option<capture_types::FocusLevel>) -> Res<()> {
        Err("no app core".into())
    }
}

/// Histories for a set of subjects (`loadSubjectHistories`); a failed read just
/// leaves that subject out.
pub fn histories(b: &dyn Backend, subjects: impl IntoIterator<Item = String>) -> Histories {
    let mut out = Histories::new();
    for s in subjects {
        if out.contains_key(&s) {
            continue;
        }
        if let Ok(v) = b.subject(&s) {
            let map: HashMap<_, _> = v
                .trajectories
                .into_iter()
                .map(|t| (t.conclusion_id, t.history))
                .collect();
            out.insert(s, map);
        }
    }
    out
}

pub fn subjects(b: &dyn Backend) -> SubjectsData {
    match b.conclusions(None) {
        Ok(conclusions) => {
            let names: Vec<String> = conclusions.iter().map(|c| c.subject.clone()).collect();
            SubjectsData {
                histories: histories(b, names),
                conclusions,
                error: None,
            }
        }
        Err(e) => SubjectsData {
            error: Some(e),
            ..Default::default()
        },
    }
}

pub fn context(b: &dyn Backend) -> ContextData {
    let authored = b.authored();
    ContextData {
        error: authored.as_ref().err().cloned(),
        authored: authored.ok(),
        inferred: b.conclusions(None).ok().map(|l| {
            l.into_iter()
                .filter(|c| c.status == capture_types::ConclusionStatus::Visible)
                .collect()
        }),
        dismissed: b.dismissed().unwrap_or_default(),
    }
}

/// No `AppCore` (a window without one).
pub struct NoBackend;

impl Backend for NoBackend {}

/// The real app.
pub struct CoreBackend(pub Arc<AppCore>);

impl CoreBackend {
    fn infra(&self) -> Res<Arc<AppInfraState>> {
        self.0
            .try_state::<AppInfraState>()
            .ok_or_else(|| "app infrastructure isn't ready".into())
    }
    fn run<T: Send>(&self, f: impl std::future::Future<Output = Res<T>> + Send) -> Res<T> {
        self.0.block_on(f)
    }
}

/// Request structs are Deserialize-only (private construction); build them from JSON.
fn req<T: DeserializeOwned>(v: serde_json::Value) -> Res<T> {
    serde_json::from_value(v).map_err(|e| e.to_string())
}

fn iso(ms: i64) -> String {
    time::OffsetDateTime::from_unix_timestamp_nanos(ms as i128 * 1_000_000)
        .ok()
        .and_then(|t| {
            t.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_default()
}

fn parse_ms(s: &str) -> Option<i64> {
    time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)
        .ok()
        .map(|t| (t.unix_timestamp_nanos() / 1_000_000) as i64)
}

impl Backend for CoreBackend {
    fn status(&self) -> Status {
        let core = &self.0;
        let infra = core.try_state::<AppInfraState>();
        let ai = core
            .block_on(app_core::ai_runtime::get_ai_runtime_status(core))
            .ok()
            .and_then(|s| serde_json::to_value(s).ok())
            .and_then(|v| serde_json::from_value::<AiStatus>(v).ok());
        let ctx = infra
            .as_ref()
            .and_then(|i| core.block_on(uc::get_user_context_status(core, i)).ok());
        let authored = infra
            .as_ref()
            .and_then(|i| core.block_on(uc::list_user_context_authored(i)).ok())
            .map(|v| v.len());
        Status {
            loaded: true,
            ai,
            ctx,
            settings: core
                .try_state::<RecordingSettingsState>()
                .map(|s| current_recording_settings(&s)),
            authored,
        }
    }
    fn usage(&self, start: i64, end: i64) -> Res<UsageChartsResponse> {
        let i = self.infra()?;
        self.run(app_core::usage_charts::get_usage_charts(
            &i,
            Some(start),
            Some(end),
        ))
    }
    fn presence(&self, start: i64, end: i64) -> Option<(bool, bool)> {
        let i = self.infra().ok()?;
        self.run(app_core::usage_charts::capture_presence(&i, start, end))
            .ok()
            .map(|p| (p.screen, p.audio))
    }
    fn capture_hours(&self, start: i64, end: i64, offset_min: i32) -> Vec<i64> {
        let Ok(i) = self.infra() else {
            return Vec::new();
        };
        self.run(app_core::usage_charts::get_capture_hours(
            &i, start, end, offset_min,
        ))
        .unwrap_or_default()
    }
    fn activities(&self, start: i64, end: i64) -> Res<Vec<Activity>> {
        let i = self.infra()?;
        self.run(uc::list_user_context_activities(
            &i,
            None,
            None,
            Some(start),
            Some(end),
        ))
    }
    fn recent_activities(&self, limit: i64, offset: i64) -> Vec<Activity> {
        let Ok(i) = self.infra() else {
            return Vec::new();
        };
        self.run(uc::list_user_context_activities(
            &i,
            Some(limit),
            Some(offset),
            None,
            None,
        ))
        .unwrap_or_default()
    }
    fn activity_meta(&self, start: i64, end: i64) -> Vec<capture_types::ActivityMeta> {
        let Ok(i) = self.infra() else {
            return Vec::new();
        };
        self.run(uc::list_user_context_activity_meta(&i, start, end))
            .unwrap_or_default()
    }
    fn failed_windows(&self, start: i64, end: i64) -> Vec<(i64, i64)> {
        let Ok(i) = self.infra() else {
            return Vec::new();
        };
        self.run(uc::list_failed_derivation_windows(&i, start, end))
            .unwrap_or_default()
    }
    fn conclusions(&self, range: Option<(i64, i64)>) -> Res<Vec<Conclusion>> {
        let i = self.infra()?;
        self.run(uc::list_user_context_conclusions(
            &i,
            Some(true),
            range.map(|r| r.0),
            range.map(|r| r.1),
        ))
    }
    fn digest(
        &self,
        kind: &str,
        start: i64,
        end: i64,
        force: bool,
    ) -> Res<UserContextDigestResult> {
        let (core, i) = (&self.0, self.infra()?);
        let kind = kind.to_string();
        if force {
            self.run(uc::regenerate_user_context_digest(
                core, &i, kind, start, end,
            ))
        } else {
            self.run(uc::get_user_context_digest(core, &i, kind, start, end))
        }
    }
    fn subject(&self, subject: &str) -> Res<SubjectView> {
        let i = self.infra()?;
        self.run(uc::get_user_context_subject(&i, subject.to_string()))
    }
    fn frames(&self, start: i64, end: i64) -> Res<Vec<StripFrame>> {
        let i = self.infra()?;
        let request = req(
            json!({ "capturedAtStart": iso(start), "capturedAtEnd": iso(end), "excludeLockScreen": true }),
        )?;
        let list = self.run(app_core::app_infra::list_frame_summaries_in_range(
            request, &i,
        ))?;
        Ok(list
            .into_iter()
            .filter_map(|f| Some((f.id, parse_ms(&f.captured_at)?)))
            .collect())
    }
    fn audio(&self, start: i64, end: i64) -> Vec<(i64, i64)> {
        let Ok(i) = self.infra() else {
            return Vec::new();
        };
        let Ok(request) = req(json!({ "capturedAtStart": iso(start), "capturedAtEnd": iso(end) }))
        else {
            return Vec::new();
        };
        let list = self
            .run(app_core::app_infra::list_audio_segments(request, &i))
            .unwrap_or_default();
        list.iter()
            .filter_map(|a| {
                let v = serde_json::to_value(a).ok()?;
                Some((
                    parse_ms(v["startedAt"].as_str()?)?,
                    parse_ms(v["endedAt"].as_str()?)?,
                ))
            })
            .collect()
    }
    fn audio_segments(&self, start: i64, end: i64) -> Res<Vec<super::journal::audio::SegRow>> {
        let i = self.infra()?;
        let request = req(json!({ "capturedAtStart": iso(start), "capturedAtEnd": iso(end) }))?;
        let list = self.run(app_core::app_infra::list_audio_segments(request, &i))?;
        Ok(list
            .iter()
            .filter_map(|a| {
                let v = serde_json::to_value(a).ok()?;
                Some(super::journal::audio::SegRow {
                    id: v["id"].as_i64()?,
                    start: parse_ms(v["startedAt"].as_str()?)?,
                    end: parse_ms(v["endedAt"].as_str()?)?,
                    mic: v["sourceKind"].as_str() == Some("microphone"),
                })
            })
            .collect())
    }
    fn frame_preview(&self, frame_id: i64) -> Option<PathBuf> {
        let request = req(json!({ "frameId": frame_id, "videoScope": null })).ok()?;
        let dto = self
            .0
            .block_on(app_core::app_infra::frame_preview::get_frame_preview(
                &self.0, request,
            ))
            .ok()??;
        Some(PathBuf::from(dto.file_path))
    }
    fn authored(&self) -> Res<Vec<AuthoredContextRow>> {
        let i = self.infra()?;
        self.run(uc::list_user_context_authored(&i))
    }
    fn dismissed(&self) -> Res<Vec<capture_types::DismissedView>> {
        let i = self.infra()?;
        self.run(uc::user_context_list_dismissed(&i))
    }
    fn dismiss(&self, id: i64) -> Res<()> {
        let (core, i) = (&self.0, self.infra()?);
        self.run(uc::user_context_dismiss_conclusion(core, &i, id))
            .map(drop)
    }
    fn set_pinned(&self, id: i64, pinned: bool) -> Res<()> {
        let (core, i) = (&self.0, self.infra()?);
        self.run(uc::user_context_set_pinned(core, &i, id, pinned))
            .map(drop)
    }
    fn add_authored(&self, text: &str, topic: Option<&str>) -> Res<AuthoredContext> {
        let (core, i) = (&self.0, self.infra()?);
        self.run(uc::user_context_add_authored(
            core,
            &i,
            text.into(),
            topic.map(Into::into),
        ))
    }
    fn update_authored(&self, id: i64, text: &str, topic: Option<&str>) -> Res<()> {
        let (core, i) = (&self.0, self.infra()?);
        self.run(uc::user_context_update_authored(
            core,
            &i,
            id,
            text.into(),
            topic.map(Into::into),
        ))
        .map(drop)
    }
    fn delete_authored(&self, id: i64) -> Res<()> {
        let (core, i) = (&self.0, self.infra()?);
        self.run(uc::user_context_delete_authored(core, &i, id))
            .map(drop)
    }
    fn restore_dismissed(&self, subject: &str, statement: &str) -> Res<()> {
        let (core, i) = (&self.0, self.infra()?);
        self.run(uc::user_context_restore_dismissed(
            core,
            &i,
            subject.into(),
            statement.into(),
        ))
        .map(drop)
    }
    fn correct_category(
        &self,
        id: i64,
        category: Option<capture_types::ActivityCategory>,
    ) -> Res<()> {
        let (core, i) = (&self.0, self.infra()?);
        self.run(uc::user_context_correct_activity_category(
            core, &i, id, category,
        ))
        .map(drop)
    }
    fn app_icons(&self, apps: &[String]) -> Vec<(String, PathBuf)> {
        use crate::timeline::data::Source as _;
        crate::timeline::data::CoreSource(self.0.clone()).app_icons(apps)
    }
    fn correct_focus(&self, id: i64, focus: Option<capture_types::FocusLevel>) -> Res<()> {
        let (core, i) = (&self.0, self.infra()?);
        self.run(uc::user_context_correct_activity_focus(core, &i, id, focus))
            .map(drop)
    }
}
