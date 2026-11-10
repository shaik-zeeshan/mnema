//! The processor drill-in (Pipeline › <processor> › #job): what Svelte's
//! `detail/{jobs,specs}.ts` and `state/detail.svelte.ts` hold. Pure helpers
//! over the jobs' camelCase JSON, the drill-in's state, and the window's
//! navigation and job actions. Views: `detail_view.rs`, `inspector.rs`.

use std::sync::LazyLock;

use gpui_kit::prelude::*;
use gpui_kit::{Context, Window};
use regex::Regex;
use serde_json::Value;

use super::data::JobCall;
use super::model::Tab;
use super::DebugWindow;
use crate::settings::model::Json;

/// `DETAIL_SPECS`: one drill-in per processor row, in the rows' order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spec {
    pub key: &'static str,
    pub name: &'static str,
    /// What the average time is measured against.
    pub per: Option<&'static str>,
    /// `processing_jobs.processor`; `None` = no job lane (the meaning index).
    pub processor: Option<&'static str>,
    /// The `subject_type` its jobs carry; guards Reprocess.
    pub subject_type: Option<&'static str>,
    /// The settings slice it runs on.
    pub cfg: &'static str,
}

pub const SPECS: [Spec; 4] = [
    Spec {
        key: "reading",
        name: "Reading",
        per: Some("per frame"),
        processor: Some("ocr"),
        subject_type: Some("frame"),
        cfg: "ocr",
    },
    Spec {
        key: "transcription",
        name: "Transcription",
        per: Some("per 5-minute segment"),
        processor: Some("audio_transcription"),
        subject_type: Some("audio_segment"),
        cfg: "transcription",
    },
    Spec {
        key: "speakers",
        name: "Speakers",
        per: Some("per 5-minute segment"),
        processor: Some("speaker_analysis"),
        subject_type: Some("audio_segment"),
        cfg: "speakerAnalysis",
    },
    Spec {
        key: "meaningIndex",
        name: "Meaning index",
        per: None,
        processor: None,
        subject_type: None,
        cfg: "semanticSearch",
    },
];

/// Rows the table shows before "N more".
pub const JOBS_SHOWN: usize = 25;
/// Queued and retrying share the wire status: those two filters fetch a wider
/// page and split it here (as Svelte; past 100 waiting jobs the split can under-fill).
const SPLIT_FETCH: i64 = 100;

/// What a row says: the wire status, or the derived `retrying` / `preparing`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobState {
    Queued,
    Running,
    Completed,
    Failed,
    Retrying,
    Preparing,
}

impl JobState {
    pub fn word(self) -> &'static str {
        match self {
            JobState::Queued => "queued",
            JobState::Running => "running",
            JobState::Completed => "completed",
            JobState::Failed => "failed",
            JobState::Retrying => "retrying",
            JobState::Preparing => "preparing",
        }
    }
}

/// The jobs table's filter, in the Segmented's order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Queued,
    Running,
    Retrying,
    Failed,
}

impl Filter {
    pub const ALL: [Filter; 5] = [
        Filter::All,
        Filter::Queued,
        Filter::Running,
        Filter::Retrying,
        Filter::Failed,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Filter::All => "All",
            Filter::Queued => "Queued",
            Filter::Running => "Running",
            Filter::Retrying => "Retrying",
            Filter::Failed => "Failed",
        }
    }
    fn split(self) -> bool {
        matches!(self, Filter::Queued | Filter::Retrying)
    }
    /// The list request's wire status.
    fn status(self) -> Option<&'static str> {
        match self {
            Filter::All => None,
            Filter::Queued | Filter::Retrying => Some("queued"),
            Filter::Running => Some("running"),
            Filter::Failed => Some("failed"),
        }
    }
}

/// `jobState`: a queued job past its backoff deadline is `retrying`, a parked
/// one (its model locked) `preparing`.
pub fn job_state(job: &Value, now_ms: i64) -> JobState {
    match job.str_at(&["status"]) {
        Some("running") => return JobState::Running,
        Some("completed") => return JobState::Completed,
        Some("failed") => return JobState::Failed,
        _ => {}
    }
    let next = job
        .str_at(&["nextAttemptAt"])
        .and_then(super::tabs::parse_job_ts);
    if next.is_some_and(|n| n > now_ms) {
        JobState::Retrying
    } else if job.bool_at(&["modelLocked"]) {
        JobState::Preparing
    } else {
        JobState::Queued
    }
}

pub fn matches_filter(state: JobState, filter: Filter) -> bool {
    match filter {
        Filter::All => true,
        Filter::Queued => matches!(state, JobState::Queued | JobState::Preparing),
        Filter::Running => state == JobState::Running,
        Filter::Retrying => state == JobState::Retrying,
        Filter::Failed => state == JobState::Failed,
    }
}

/// `filterCounts`: a lane's `queued` includes its `retrying`.
pub fn filter_count(lane: Option<&Value>, f: Filter) -> Option<i64> {
    let lane = lane?;
    let n = |k: &str| lane.num_at(&[k]).unwrap_or(0.) as i64;
    Some(match f {
        Filter::All => n("queued") + n("running") + n("completed") + n("failed"),
        Filter::Queued => n("queued") - n("retrying"),
        Filter::Running => n("running"),
        Filter::Retrying => n("retrying"),
        Filter::Failed => n("failed"),
    })
}

fn format_delta(ms: i64) -> String {
    let s = (ms as f64 / 1000.).round() as i64;
    if s < 60 {
        format!("{s}s")
    } else {
        format!("{}m {}s", s / 60, s % 60)
    }
}

/// "in 4m 12s" / "due now", `None` when nothing is scheduled.
pub fn next_attempt_label(next: Option<&str>, now_ms: i64) -> Option<String> {
    let next = next.and_then(super::tabs::parse_job_ts)?;
    let d = next - now_ms;
    Some(if d <= 0 {
        "due now".into()
    } else {
        format!("in {}", format_delta(d))
    })
}

fn payload_str(job: &Value, keys: &[&str]) -> Option<String> {
    let payload: Value = serde_json::from_str(job.str_at(&["payloadJson"])?).ok()?;
    keys.iter()
        .find_map(|k| payload[*k].as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// The `provider` the job was enqueued for, out of its payload.
pub fn job_provider(job: &Value) -> Option<String> {
    payload_str(job, &["provider"])
}

/// The payload's frozen model id.
pub fn job_model(job: &Value) -> Option<String> {
    payload_str(job, &["modelId", "model_id"])
}

/// "Segment #2291" / "Frame #918244".
pub fn subject_label(job: &Value) -> String {
    let id = job.num_at(&["subjectId"]).unwrap_or(0.) as i64;
    let noun = match job.str_at(&["subjectType"]).unwrap_or("") {
        "audio_segment" => "Segment".to_string(),
        "frame" => "Frame".to_string(),
        other => other.replace('_', " "),
    };
    format!("{noun} #{id}")
}

/// "Show in log"'s filter (a bare "job N" also hits app-infra's "cpu job N").
pub fn log_needle(id: i64) -> String {
    format!("processing job {id}")
}

/// "deepgram" → "Deepgram".
pub fn provider_name(provider: &str) -> String {
    let words = provider.replace('_', " ");
    let mut c = words.chars();
    c.next()
        .map_or(String::new(), |f| f.to_uppercase().chain(c).collect())
}

// String sniffing over the raw error, as Svelte does (no typed error kind on the wire).
static TRANSIENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)timed? ?out|timeout|unreachable|could not connect|connection|network|dns|offline|temporar|(http|status)\s*(429|5\d\d)\b").unwrap()
});
static CUT_OFF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)moov|decode|truncat|unexpected (eof|end)|end of file|invalid data").unwrap()
});
static NO_MODEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)model.*(missing|not found|not installed|absent|not downloaded)|no such model")
        .unwrap()
});

pub fn is_transient(error: Option<&str>) -> bool {
    error.is_some_and(|e| TRANSIENT.is_match(e))
}

/// A plain title for the few error shapes we recognise.
pub fn known_error_title(error: Option<&str>, provider: Option<&str>) -> Option<String> {
    let e = error?;
    if CUT_OFF.is_match(e) {
        Some("The recording was cut off".into())
    } else if NO_MODEL.is_match(e) {
        Some("The model isn\u{2019}t installed".into())
    } else if TRANSIENT.is_match(e) {
        Some(format!(
            "{} can\u{2019}t be reached",
            provider.map_or("The provider".into(), provider_name)
        ))
    } else {
        None
    }
}

pub fn error_title(error: Option<&str>, provider: Option<&str>) -> String {
    known_error_title(error, provider).unwrap_or_else(|| "This job failed".into())
}

fn first_line(s: &str) -> String {
    s.split('\n').next().unwrap_or("").trim().to_string()
}

/// `jobMessage`: the line under a row; `None` = no line.
pub fn job_message(
    job: &Value,
    state: JobState,
    now_ms: i64,
    lane_retrying: i64,
) -> Option<String> {
    let provider = job_provider(job);
    let err = job.str_at(&["lastError"]);
    match state {
        JobState::Retrying => {
            let why = known_error_title(err, provider.as_deref()).or_else(|| err.map(first_line));
            let next = format!(
                "Next try {}",
                next_attempt_label(job.str_at(&["nextAttemptAt"]), now_ms)
                    .unwrap_or_else(|| "soon".into())
            );
            Some(why.map_or(next.clone(), |w| format!("{next} \u{b7} {w}")))
        }
        JobState::Failed => Some(err.map_or("Failed".into(), |e| {
            known_error_title(Some(e), provider.as_deref()).unwrap_or_else(|| first_line(e))
        })),
        JobState::Preparing => Some("Waiting for its model".into()),
        JobState::Queued => Some(if lane_retrying > 0 {
            format!(
                "Waiting behind {lane_retrying} {}",
                if lane_retrying == 1 {
                    "retry"
                } else {
                    "retries"
                }
            )
        } else {
            "Waiting its turn".into()
        }),
        _ => None,
    }
}

/// "N more [filter] · same error": what this filter holds beyond the rows drawn.
pub fn tail(
    filter: Filter,
    split_count: Option<i64>,
    total: i64,
    rows: &[&Value],
) -> Option<String> {
    let more = if filter.split() {
        split_count.unwrap_or(0)
    } else {
        total
    } - rows.len() as i64;
    if more <= 0 {
        return None;
    }
    let word = if filter == Filter::All {
        String::new()
    } else {
        format!(" {}", filter.label().to_lowercase())
    };
    // "same error" judges the drawn rows only, as Svelte does.
    let first = rows.first().map(|r| r.str_at(&["lastError"]));
    let same = filter != Filter::All
        && first.is_some_and(|e| e.is_some())
        && rows.iter().all(|r| Some(r.str_at(&["lastError"])) == first);
    Some(format!(
        "{} more{word}{}",
        super::model::count(more),
        if same { " \u{b7} same error" } else { "" }
    ))
}

/// The drill-in's data (`createDetailStore`).
#[derive(Debug, Default)]
pub struct Detail {
    pub spec: Option<Spec>,
    /// The processor row it came from, for focus on the way back.
    pub row: usize,
    pub filter: Filter,
    pub jobs: Vec<Value>,
    pub total: i64,
    pub error: Option<String>,
    /// The inspected job, kept even if it leaves the polled page.
    pub inspected: Option<Value>,
    pub acting: bool,
    /// The last action's outcome, in place: `(ok, line)`.
    pub message: Option<(bool, String)>,
    /// The unreachable notice's Test: `None` running, `Some(Ok(()))` reachable.
    pub test: Option<Result<(), String>>,
    pub testing: bool,
    pub raw_open: bool,
    pub copied: bool,
    /// The jobs poll: last read and whether one is out.
    pub polled_at: Option<std::time::Instant>,
    pub in_flight: bool,
    /// Bumps on drill-in / refilter, so a stale page never lands.
    generation: u64,
}

impl Detail {
    /// Out of the drill-in: nothing kept, and a late page from it is dropped.
    pub fn leave(from: &Detail) -> Detail {
        Detail {
            generation: from.generation + 1,
            ..Default::default()
        }
    }

    /// This filter's rows (queued / retrying split here).
    pub fn visible(&self, now_ms: i64) -> Vec<&Value> {
        self.jobs
            .iter()
            .filter(|j| matches_filter(job_state(j, now_ms), self.filter))
            .collect()
    }
}

impl DebugWindow {
    /// A processor row's click / Enter: drill in and start its jobs poll.
    pub fn open_processor(&mut self, row: usize, cx: &mut Context<Self>) {
        self.detail = Detail {
            spec: SPECS.get(row).copied(),
            row,
            ..Detail::leave(&self.detail)
        };
        self.tab = Tab::Pipeline;
        self.fetch_jobs(cx);
        cx.notify();
    }

    pub fn open_job(&mut self, job: Value, cx: &mut Context<Self>) {
        self.detail.inspected = Some(job);
        self.detail.message = None;
        self.detail.raw_open = false;
        self.detail.copied = false;
        cx.notify();
    }

    /// One level up (job → processor → Pipeline), focus back on the row it came from.
    pub fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.detail.inspected.take().is_some() {
            self.detail.message = None;
        } else {
            let row = self.detail.row;
            self.detail = Detail::leave(&self.detail);
            if let Some(h) = self.proc_focus.get(row) {
                window.focus(h, cx);
            }
        }
        cx.notify();
    }

    pub fn drilled(&self) -> bool {
        self.detail.spec.is_some()
    }

    pub fn set_filter(&mut self, f: Filter, cx: &mut Context<Self>) {
        if f == self.detail.filter {
            return;
        }
        self.detail.filter = f;
        self.detail.jobs.clear();
        self.detail.total = 0;
        self.detail.generation += 1;
        self.fetch_jobs(cx);
        cx.notify();
    }

    /// `fetchJobs`: one page of this processor's jobs behind the filter.
    pub fn fetch_jobs(&mut self, cx: &mut Context<Self>) {
        let Some(processor) = self.detail.spec.and_then(|s| s.processor) else {
            return;
        };
        let filter = self.detail.filter;
        let call = JobCall::List {
            processor: processor.into(),
            status: filter.status().map(str::to_string),
            limit: if filter.split() {
                SPLIT_FETCH
            } else {
                JOBS_SHOWN as i64
            },
        };
        let generation = self.detail.generation;
        self.detail.polled_at = Some(cx.background_executor().now());
        self.detail.in_flight = true;
        let backend = self.backend.clone();
        let job = cx.background_spawn(async move { backend.jobs(&call) });
        cx.spawn(async move |this, cx| {
            let got = job.await;
            let _ = this.update(cx, |w, cx| {
                // Drilled elsewhere or refiltered while in flight: that request owns the view.
                if w.detail.generation != generation {
                    return;
                }
                let d = &mut w.detail;
                d.in_flight = false;
                match got {
                    Ok(page) => {
                        d.jobs = page["jobs"].as_array().cloned().unwrap_or_default();
                        d.total = page["total"].as_i64().unwrap_or(0);
                        d.error = None;
                        let id = d.inspected.as_ref().map(|j| j["id"].clone());
                        if let Some(fresh) = id.and_then(|id| d.jobs.iter().find(|j| j["id"] == id))
                        {
                            d.inspected = Some(fresh.clone());
                        }
                    }
                    Err(e) => d.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Retry now / Reprocess / Test: run off the main thread, say what came
    /// back in place, then re-read the page.
    pub fn job_act(&mut self, call: JobCall, cx: &mut Context<Self>) {
        let test = call == JobCall::TestDeepgram;
        if (test && self.detail.testing) || (!test && self.detail.acting) {
            return;
        }
        if test {
            self.detail.testing = true;
            self.detail.test = None;
        } else {
            self.detail.acting = true;
            self.detail.message = None;
        }
        let backend = self.backend.clone();
        let job = cx.background_spawn({
            let call = call.clone();
            async move { backend.jobs(&call) }
        });
        cx.spawn(async move |this, cx| {
            let got = job.await;
            let _ = this.update(cx, |w, cx| {
                let d = &mut w.detail;
                match (&call, got) {
                    (JobCall::TestDeepgram, got) => {
                        d.testing = false;
                        d.test = Some(got.map(|_| ()));
                        return cx.notify();
                    }
                    (JobCall::RetryNow(id), Ok(job)) => {
                        if let Some(i) = d.inspected.as_mut().filter(|i| i["id"] == *id) {
                            if let (Some(i), Some(j)) = (i.as_object_mut(), job.as_object()) {
                                i.extend(j.clone());
                                i.insert("nextAttemptAt".into(), Value::Null);
                            }
                        }
                        d.message = Some((true, "Will try again now".into()));
                    }
                    (_, Ok(_)) => d.message = Some((true, "Queued again".into())),
                    (_, Err(e)) => d.message = Some((false, e)),
                }
                d.acting = false;
                w.fetch_jobs(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The Pipeline tab's 2 s jobs poll while drilled into a lane.
    pub(super) fn jobs_due(&self) -> bool {
        self.tab == Tab::Pipeline && self.detail.spec.is_some_and(|s| s.processor.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: i64 = 1_793_782_920_000; // 2026-11-04 09:02:00 UTC

    #[test]
    fn a_queued_job_serving_a_backoff_is_retrying_and_a_parked_one_preparing() {
        let j = |next: Option<&str>, locked: bool| json!({ "status": "queued", "nextAttemptAt": next, "modelLocked": locked });
        assert_eq!(
            job_state(&j(Some("2026-11-04 09:02:42"), false), NOW),
            JobState::Retrying
        );
        assert_eq!(
            job_state(&j(Some("2026-11-04 09:02:42"), true), NOW),
            JobState::Retrying
        );
        assert_eq!(
            job_state(&j(Some("2026-11-04 09:01:00"), true), NOW),
            JobState::Preparing
        );
        assert_eq!(job_state(&j(None, false), NOW), JobState::Queued);
        assert_eq!(
            job_state(&json!({ "status": "failed" }), NOW),
            JobState::Failed
        );
        assert!(matches_filter(JobState::Preparing, Filter::Queued));
        assert!(!matches_filter(JobState::Retrying, Filter::Queued));
    }

    #[test]
    fn row_messages_say_why_a_job_sits_where_it_is() {
        let retry = json!({ "status": "queued", "nextAttemptAt": "2026-11-04 09:06:12",
            "lastError": "deepgram: connection refused", "payloadJson": "{\"provider\":\"deepgram\"}" });
        assert_eq!(
            job_message(&retry, JobState::Retrying, NOW, 12).as_deref(),
            Some("Next try in 4m 12s \u{b7} Deepgram can\u{2019}t be reached")
        );
        let failed = json!({ "status": "failed", "lastError": "worker panicked\nat line 2" });
        assert_eq!(
            job_message(&failed, JobState::Failed, NOW, 0).as_deref(),
            Some("worker panicked")
        );
        let q = json!({ "status": "queued" });
        assert_eq!(
            job_message(&q, JobState::Queued, NOW, 1).as_deref(),
            Some("Waiting behind 1 retry")
        );
        assert_eq!(job_message(&q, JobState::Completed, NOW, 0), None);
    }

    #[test]
    fn the_tail_counts_what_the_filter_holds_beyond_the_drawn_rows() {
        let e = json!({ "lastError": "x" });
        let rows = vec![&e, &e];
        assert_eq!(
            tail(Filter::All, None, 4226, &rows).as_deref(),
            Some("4,224 more")
        );
        assert_eq!(
            tail(Filter::Retrying, Some(12), 99, &rows).as_deref(),
            Some("10 more retrying \u{b7} same error")
        );
        assert_eq!(tail(Filter::Failed, None, 2, &rows), None);
    }

    #[test]
    fn filter_counts_take_retrying_out_of_queued() {
        let lane =
            json!({ "queued": 14, "retrying": 12, "running": 0, "completed": 4210, "failed": 1 });
        assert_eq!(filter_count(Some(&lane), Filter::All), Some(4225));
        assert_eq!(filter_count(Some(&lane), Filter::Queued), Some(2));
        assert_eq!(filter_count(None, Filter::Queued), None);
    }
}
