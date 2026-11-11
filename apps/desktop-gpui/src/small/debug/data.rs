//! The developer window's reads. Each poll fetches one [`Section`] off the main
//! thread (`DebugBackend::fetch`); the answer is the commands' JSON, so the
//! parity fixture (command → JSON, what the Svelte stub serves) and app-core
//! feed the same view. The window's few buttons go through `DebugBackend::act`.

use std::sync::Arc;
use std::time::Duration;

use app_core::recording_settings::{current_recording_settings, RecordingSettingsState};
use app_core::{AppCore, AppInfraState};
use serde::Serialize;
use serde_json::Value;

/// One poll's slice of the window's data, with its cadence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    Health,
    Pipeline,
    Housekeeping,
    Capture,
    Ai,
    Logs,
}

impl Section {
    pub fn every(self) -> Duration {
        Duration::from_millis(match self {
            Section::Health | Section::Pipeline | Section::Logs => 2_000,
            Section::Housekeeping => 30_000,
            Section::Capture => 1_000,
            Section::Ai => 5_000,
        })
    }
}

/// Lines requested per log poll (the viewer's whole window, said in the UI).
pub const LOG_TAIL_LINES: usize = 500;

#[derive(Clone, Debug, Default)]
pub struct DebugData {
    /// `get_debug_health` (the 12 rows).
    pub health: Vec<Value>,
    pub health_error: Option<String>,
    pub fetched_at_ms: Option<i64>,
    /// `get_processing_pipeline_status` + `get_semantic_index_status`.
    pub lanes: Vec<Value>,
    pub semantic: Option<Value>,
    pub pipeline_error: Option<String>,
    /// `get_retention_cleanup_status`, `list_frame_batches`, `list_app_jobs`.
    pub retention: Option<Value>,
    pub batches: Vec<Value>,
    pub app_jobs: Vec<Value>,
    /// `get_recording_settings`.
    pub settings: Option<Value>,
    /// `get_capture_health` + `get_storage_usage`.
    pub capture: Option<Value>,
    pub storage: Option<Value>,
    /// `get_ai_runtime_status`, `get_ask_ai_last_turn_usage`,
    /// `list_user_context_derivation_runs`, CLI history / grants, User Context status.
    pub ai_status: Option<Value>,
    pub usage: Option<Value>,
    pub runs: Vec<Value>,
    /// `list_user_context_derivation_runs` failed ("Couldn't load recent runs").
    pub runs_error: Option<String>,
    pub cli_history: Vec<Value>,
    /// `list_cli_access_history` failed ("Couldn't read CLI requests").
    pub cli_error: Option<String>,
    pub grants: Vec<Value>,
    pub uc_status: Option<Value>,
    /// `tail_app_log`.
    pub log_tail: Option<Value>,
    pub log_error: Option<String>,
    pub version: String,
}

impl DebugData {
    /// Take `section`'s fields from `from` (a fetch answer).
    pub fn merge(&mut self, section: Section, from: DebugData) {
        if !from.version.is_empty() {
            self.version = from.version;
        }
        match section {
            Section::Health => {
                // A failed poll keeps the last good rows (and their time).
                if from.health_error.is_none() {
                    self.health = from.health;
                    self.fetched_at_ms = from.fetched_at_ms;
                }
                self.health_error = from.health_error;
                // The verdict's "recording 2h 14m" reads the capture session in both modes.
                if from.capture.is_some() {
                    self.capture = from.capture;
                }
            }
            Section::Pipeline => {
                self.lanes = from.lanes;
                self.semantic = from.semantic;
                self.pipeline_error = from.pipeline_error;
            }
            Section::Housekeeping => {
                self.retention = from.retention;
                self.batches = from.batches;
                self.app_jobs = from.app_jobs;
                self.settings = from.settings.or(self.settings.take());
            }
            Section::Capture => {
                self.capture = from.capture;
                self.storage = from.storage.or(self.storage.take());
                self.settings = from.settings.or(self.settings.take());
            }
            Section::Ai => {
                self.ai_status = from.ai_status.or(self.ai_status.take());
                self.usage = from.usage;
                // A failed read keeps the last list behind its error line.
                if from.runs_error.is_none() {
                    self.runs = from.runs;
                }
                self.runs_error = from.runs_error;
                if from.cli_error.is_none() {
                    self.cli_history = from.cli_history;
                }
                self.cli_error = from.cli_error;
                self.grants = from.grants;
                self.uc_status = from.uc_status;
            }
            Section::Logs => {
                if from.log_error.is_none() {
                    self.log_tail = from.log_tail;
                }
                self.log_error = from.log_error;
            }
        }
    }
}

/// The window's buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Record,
    Stop,
    TestAi,
    Derive,
}

/// The processor drill-in's commands (`list_processing_jobs_by_processor`,
/// `retry_processing_job_now`, the `reprocess_*` per subject,
/// `transcription_test_deepgram`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobCall {
    /// `status`: a wire status (`queued`/`running`/`failed`), `None` = every status.
    List {
        processor: String,
        status: Option<String>,
        limit: i64,
    },
    /// Skip a waiting job's backoff; answers the job.
    RetryNow(i64),
    /// Requeue one subject through its processor's reprocess command.
    Reprocess {
        processor: String,
        subject_id: i64,
    },
    TestDeepgram,
}

pub trait DebugBackend: Send + Sync + 'static {
    /// One section's fields (the rest default). Sync: runs on the background executor.
    fn fetch(&self, section: Section, now_ms: i64) -> DebugData;
    /// Run a button's command; `Ok(Some(line))` is worth showing, `Err` is the
    /// backend's own words.
    fn act(&self, action: Action) -> Result<Option<String>, String> {
        let _ = action;
        Err("not available without the app core".into())
    }
    /// One drill-in command's JSON answer (a jobs page, a job, or nothing).
    fn jobs(&self, call: &JobCall) -> Result<Value, String> {
        let _ = call;
        Err("not available without the app core".into())
    }
}

fn json(v: impl Serialize) -> Option<Value> {
    serde_json::to_value(v).ok()
}

fn list(v: impl Serialize) -> Vec<Value> {
    json(v)
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
}

/// `get_capture_health` (the capture runtime is managed at startup).
fn capture(core: &AppCore) -> Option<Value> {
    core.try_state::<app_core::native_capture::lifecycle::NativeCaptureState>()
        .and_then(|s| {
            json(app_core::native_capture::capture_health::capture_health_snapshot(&s, core))
        })
}

/// The version About shows: the updater's app info (read from the bundle).
fn app_version(core: &AppCore) -> String {
    core.try_state::<app_core::updates::Updater>()
        .map_or_else(String::new, |u| u.config.app.version.clone())
}

pub struct CoreDebug(pub Arc<AppCore>);

impl DebugBackend for CoreDebug {
    fn fetch(&self, section: Section, now_ms: i64) -> DebugData {
        let core = &self.0;
        let infra = core.try_state::<AppInfraState>();
        let settings = || {
            core.try_state::<RecordingSettingsState>()
                .and_then(|s| json(current_recording_settings(&s)))
        };
        let mut d = DebugData {
            // The bundle's version, as Settings > About shows it (not this crate's).
            version: app_version(core),
            ..Default::default()
        };
        match section {
            Section::Health => {
                d.capture = capture(core);
                let states = (
                    infra,
                    core.try_state::<app_core::debug_status::SemanticWorkerHealthState>(),
                    core.try_state::<RecordingSettingsState>(),
                );
                let (Some(infra), Some(worker), Some(settings)) = states else {
                    d.health_error = Some("the app is still starting".into());
                    return d;
                };
                match core.block_on(app_core::debug_health::get_debug_health(
                    core, &infra, &worker, &settings,
                )) {
                    Ok(rows) => {
                        d.health = list(rows);
                        d.fetched_at_ms = Some(now_ms);
                    }
                    Err(e) => {
                        log::warn!("debug: the health rollup failed: {e}");
                        d.health_error = Some(e);
                    }
                }
            }
            Section::Pipeline => {
                let Some(infra) = infra else {
                    d.pipeline_error = Some("app infrastructure isn't ready".into());
                    return d;
                };
                match core.block_on(app_core::debug_pipeline::get_processing_pipeline_status(
                    &infra,
                )) {
                    Ok(l) => d.lanes = list(l),
                    Err(e) => d.pipeline_error = Some(e),
                }
                if let Some(h) =
                    core.try_state::<app_core::debug_status::SemanticWorkerHealthState>()
                {
                    d.semantic = core
                        .block_on(app_core::debug_status::get_semantic_index_status(
                            &infra, &h,
                        ))
                        .ok()
                        .and_then(json);
                }
            }
            Section::Housekeeping => {
                d.settings = settings();
                let Some(infra) = infra else { return d };
                if let Some(s) = core.try_state::<RecordingSettingsState>() {
                    d.retention = core
                        .block_on(app_core::app_infra::get_retention_cleanup_status(
                            &infra, &s,
                        ))
                        .ok()
                        .and_then(json);
                }
                d.batches = core
                    .block_on(app_core::debug_status::list_frame_batches(&infra, Some(50)))
                    .map(list)
                    .unwrap_or_default();
                d.app_jobs = core
                    .block_on(app_core::app_infra::list_app_jobs(&infra))
                    .map(list)
                    .unwrap_or_default();
            }
            Section::Capture => {
                d.settings = settings();
                d.storage = json(app_core::managed_storage_layout::get_storage_usage(
                    core, None,
                ));
                d.capture = capture(core);
            }
            Section::Ai => {
                d.ai_status = core
                    .block_on(app_core::ai_runtime::get_ai_runtime_status(core))
                    .ok()
                    .and_then(json);
                d.usage = app_core::ask_ai::get_ask_ai_last_turn_usage().and_then(json);
                match core.block_on(app_core::cli_access::list_cli_access_history(core)) {
                    Ok(h) => {
                        d.cli_history = json(h)
                            .and_then(|v| v["events"].as_array().cloned())
                            .unwrap_or_default()
                    }
                    Err(e) => d.cli_error = Some(e.to_string()),
                }
                d.grants = core
                    .block_on(app_core::cli_access::list_cli_access_grants(core))
                    .ok()
                    .and_then(json)
                    .and_then(|v| v["grants"].as_array().cloned())
                    .unwrap_or_default();
                if let Some(infra) = infra {
                    match core.block_on(app_core::debug_status::list_user_context_derivation_runs(
                        &infra,
                        Some(5),
                    )) {
                        Ok(runs) => d.runs = list(runs),
                        Err(e) => d.runs_error = Some(e.to_string()),
                    }
                    d.uc_status = core
                        .block_on(app_core::user_context::commands::get_user_context_status(
                            core, &infra,
                        ))
                        .ok()
                        .and_then(json);
                }
            }
            Section::Logs => match core.block_on(app_core::debug_status::tail_app_log(
                core,
                Some(LOG_TAIL_LINES),
            )) {
                Ok(t) => d.log_tail = json(t),
                Err(e) => d.log_error = Some(e),
            },
        }
        d
    }

    fn act(&self, action: Action) -> Result<Option<String>, String> {
        let result = self.run(action);
        if let Err(e) = &result {
            log::warn!("debug: {action:?} failed: {e}");
        }
        result
    }

    fn jobs(&self, call: &JobCall) -> Result<Value, String> {
        let core = &self.0;
        let infra = core
            .try_state::<AppInfraState>()
            .ok_or("app infrastructure isn't ready")?;
        let result = match call {
            JobCall::List {
                processor,
                status,
                limit,
            } => {
                let request = app_core::debug_pipeline::ListProcessingJobsRequest {
                    processor: processor.clone(),
                    status: status
                        .as_ref()
                        .map(|s| serde_json::from_value(Value::String(s.clone())))
                        .transpose()
                        .map_err(|e| e.to_string())?,
                    subject_id: None,
                    limit: Some(*limit),
                    offset: Some(0),
                };
                core.block_on(app_core::debug_pipeline::list_processing_jobs_by_processor(
                    request, &infra,
                ))
                .map(|page| json(page).unwrap_or_default())
            }
            JobCall::RetryNow(id) => core
                .block_on(app_core::app_infra::retry_processing_job_now(*id, &infra))
                .map(|job| json(job).unwrap_or_default()),
            JobCall::Reprocess {
                processor,
                subject_id,
            } => self.reprocess(&infra, processor, *subject_id),
            JobCall::TestDeepgram => {
                crate::settings::live::deepgram_test(core).map(|_| Value::Null)
            }
        };
        // Ids and the backend's words only: a job's own error never reaches the log here.
        if let (Err(e), false) = (&result, matches!(call, JobCall::List { .. })) {
            log::warn!("debug: a job action failed: {e}");
        }
        result
    }
}

impl CoreDebug {
    /// `DETAIL_SPECS[..].reprocess`: the per-subject reprocess command of each lane.
    fn reprocess(
        &self,
        infra: &AppInfraState,
        processor: &str,
        subject_id: i64,
    ) -> Result<Value, String> {
        use app_core::app_infra as ai;
        let core = &self.0;
        let segment = || ai::ReprocessAudioSegmentTranscriptionRequest {
            audio_segment_id: subject_id,
        };
        match processor {
            "ocr" => {
                let settings = core
                    .try_state::<RecordingSettingsState>()
                    .ok_or("settings aren't ready")?;
                let request = ai::ReprocessCapturedFrameOcrRequest {
                    frame_id: subject_id,
                    payload_json: None,
                };
                core.block_on(ai::reprocess_captured_frame_ocr(request, infra, &settings))
                    .map(|_| Value::Null)
            }
            "audio_transcription" => core
                .block_on(ai::reprocess_audio_segment_transcription(
                    segment(),
                    infra,
                    core,
                ))
                .map(|_| Value::Null),
            "speaker_analysis" => core
                .block_on(ai::reprocess_audio_segment_speaker_analysis(
                    segment(),
                    infra,
                    core,
                ))
                .map(|_| Value::Null),
            other => Err(format!("{other} has no reprocess command")),
        }
    }

    fn run(&self, action: Action) -> Result<Option<String>, String> {
        use app_core::native_capture::control;
        let core = &self.0;
        match action {
            // The same door as the tray's Start / Stop: saved sources, session
            // event, tray refresh.
            Action::Record => control::start_native_capture_from_core("debug-window", core)
                .map(|_| None)
                .map_err(|e| e.message),
            Action::Stop => control::stop_native_capture_from_core(core)
                .map(|_| None)
                .map_err(|e| e.message),
            Action::TestAi => crate::settings::live::test_connection(core),
            Action::Derive => crate::settings::live::derive_now(core),
        }
    }
}

/// Serves a parity fixture map (command → JSON) and records the drill-in's calls.
/// `{ "__reject": "<reason>" }` under a command makes it fail, as in the Svelte stub.
pub struct FixtureDebug(pub Value, pub std::sync::Mutex<Vec<JobCall>>);

impl FixtureDebug {
    pub fn new(map: Value) -> Self {
        Self(map, Default::default())
    }

    pub fn calls(&self) -> Vec<JobCall> {
        self.1.lock().map(|c| c.clone()).unwrap_or_default()
    }
}

impl DebugBackend for FixtureDebug {
    fn fetch(&self, section: Section, now_ms: i64) -> DebugData {
        let m = &self.0;
        let v = |k: &str| {
            m.get(k)
                .cloned()
                .filter(|v| !v.is_null() && v != &Value::Object(Default::default()))
        };
        let arr = |k: &str| {
            m.get(k)
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default()
        };
        let mut d = DebugData {
            version: m["plugin:app|version"].as_str().unwrap_or("").into(),
            ..Default::default()
        };
        match section {
            Section::Health => {
                d.health = arr("get_debug_health");
                d.fetched_at_ms = Some(now_ms);
                d.capture = v("get_capture_health");
            }
            Section::Pipeline => {
                d.lanes = arr("get_processing_pipeline_status");
                d.semantic = v("get_semantic_index_status");
            }
            Section::Housekeeping => {
                d.settings = v("get_recording_settings");
                d.retention = v("get_retention_cleanup_status");
                d.batches = arr("list_frame_batches");
                d.app_jobs = arr("list_app_jobs");
            }
            Section::Capture => {
                d.settings = v("get_recording_settings");
                d.capture = v("get_capture_health");
                d.storage = v("get_storage_usage");
            }
            Section::Ai => {
                d.ai_status = v("get_ai_runtime_status");
                d.usage = v("get_ask_ai_last_turn_usage");
                d.runs = arr("list_user_context_derivation_runs");
                let reject = |key: &str| m[key]["__reject"].as_str().map(String::from);
                d.runs_error = reject("list_user_context_derivation_runs");
                d.cli_error = reject("list_cli_access_history");
                d.cli_history = m["list_cli_access_history"]["events"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                d.grants = m["list_cli_access_grants"]["grants"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                d.uc_status = v("get_user_context_status");
            }
            Section::Logs => d.log_tail = v("tail_app_log"),
        }
        d
    }

    fn jobs(&self, call: &JobCall) -> Result<Value, String> {
        if let Ok(mut c) = self.1.lock() {
            c.push(call.clone());
        }
        let key = match call {
            JobCall::List { .. } => "list_processing_jobs_by_processor",
            JobCall::RetryNow(_) => "retry_processing_job_now",
            JobCall::Reprocess { .. } => "reprocess",
            JobCall::TestDeepgram => "transcription_test_deepgram",
        };
        let v = self.0.get(key).cloned().unwrap_or(Value::Null);
        match v["__reject"].as_str() {
            Some(reason) => Err(reason.to_string()),
            None => Ok(v),
        }
    }
}
