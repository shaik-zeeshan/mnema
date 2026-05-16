use std::{
    collections::{hash_map::DefaultHasher, HashMap, VecDeque},
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use serde::Serialize;
use time::{
    format_description, format_description::well_known::Rfc3339, OffsetDateTime, PrimitiveDateTime,
};

const HIGH_PRESSURE_THRESHOLD: i64 = 3;
const REPRESENTATIVE_SECONDS: i64 = 15;
const DEBUG_RING_CAPACITY: usize = 256;
const OCR_ACTIVE_RECORDING_COOLDOWN_MIN: Duration = Duration::from_millis(1000);
const OCR_ACTIVE_RECORDING_COOLDOWN_MAX: Duration = Duration::from_millis(10000);
const OCR_CATCH_UP_COOLDOWN_MIN: Duration = Duration::from_millis(250);
const OCR_CATCH_UP_COOLDOWN_MAX: Duration = Duration::from_millis(2000);

static OCR_BUDGET_STATES: OnceLock<Mutex<HashMap<PathBuf, OcrBudgetState>>> = OnceLock::new();

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrBudgetDebugDto {
    pub summary: OcrBudgetExecutionSummaryDto,
    pub admission_events: Vec<OcrAdmissionDebugEventDto>,
    pub execution_events: Vec<OcrExecutionDebugEventDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrBudgetExecutionSummaryDto {
    pub queued_or_running_count: i64,
    pub execution_state: String,
    pub cooldown_remaining_ms: i64,
    pub last_run_duration_ms: Option<i64>,
    pub last_run_status: Option<String>,
    pub last_pacing_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrAdmissionDebugEventDto {
    pub occurred_at: String,
    pub session_id: String,
    pub workspace_scope: String,
    pub frame_id: i64,
    pub outcome: ::app_infra::OcrAdmissionOutcome,
    pub reason: ::app_infra::OcrAdmissionReason,
    pub queue_pressure_count: i64,
    pub recording_active: bool,
    pub job_id: Option<i64>,
    pub related_frame_id: Option<i64>,
    pub signals: ::app_infra::OcrAdmissionSignals,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrExecutionDebugEventDto {
    pub occurred_at: String,
    pub job_id: i64,
    pub frame_id: Option<i64>,
    pub provider: String,
    pub model_id: Option<String>,
    pub recognition_mode: Option<String>,
    pub status: String,
    pub run_duration_ms: i64,
    pub queue_wait_ms: Option<i64>,
    pub result_text_length: Option<i64>,
    pub observation_count: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcrProcessingPass {
    DidWork,
    Idle,
    CoolingDown(Duration),
}

#[derive(Debug, Default)]
struct OcrBudgetState {
    admission_scopes: HashMap<AdmissionScopeKey, AdmissionScopeState>,
    admission_events: VecDeque<OcrAdmissionDebugEventDto>,
    execution_events: VecDeque<OcrExecutionDebugEventDto>,
    execution: OcrExecutionBudgetState,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AdmissionScopeKey {
    session_id: String,
    workspace_prefix: Option<String>,
}

#[derive(Debug, Default)]
struct AdmissionScopeState {
    seen_candidate: bool,
    last_admitted_at: Option<OffsetDateTime>,
}

#[derive(Debug, Default)]
struct OcrExecutionBudgetState {
    next_due_at: Option<Instant>,
    last_run_at: Option<Instant>,
    last_run_ms: Option<u64>,
    last_status: Option<String>,
    last_recording_active: bool,
}

pub fn reset_for_base_dir(base_dir: &Path) {
    let states = OCR_BUDGET_STATES.get_or_init(|| Mutex::new(HashMap::new()));
    states
        .lock()
        .expect("OCR budget states poisoned")
        .remove(base_dir);
}

pub fn clear_sessions_for_base_dir(base_dir: &Path, session_ids: &[String]) {
    if session_ids.is_empty() {
        return;
    }
    let Some(states) = OCR_BUDGET_STATES.get() else {
        return;
    };
    let mut states = states.lock().expect("OCR budget states poisoned");
    let Some(state) = states.get_mut(base_dir) else {
        return;
    };
    state
        .admission_scopes
        .retain(|key, _| !session_ids.iter().any(|id| id == &key.session_id));
}

fn with_state<R>(base_dir: &Path, f: impl FnOnce(&mut OcrBudgetState) -> R) -> R {
    let states = OCR_BUDGET_STATES.get_or_init(|| Mutex::new(HashMap::new()));
    let mut states = states.lock().expect("OCR budget states poisoned");
    f(states.entry(base_dir.to_path_buf()).or_default())
}

fn workspace_prefix(frame_path: &str) -> Option<String> {
    ::app_infra::HiddenSegmentWorkspacePaths::from_frame_artifact_path(Path::new(frame_path))
        .map(|paths| paths.workspace_dir)
}

fn scope_key(frame: &::app_infra::NewFrame) -> AdmissionScopeKey {
    AdmissionScopeKey {
        session_id: frame.session_id.clone(),
        workspace_prefix: workspace_prefix(&frame.file_path),
    }
}

fn workspace_scope_label(workspace_prefix: Option<&str>) -> String {
    let Some(prefix) = workspace_prefix else {
        return "session".to_string();
    };
    let basename = Path::new(prefix)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("workspace");
    let mut hasher = DefaultHasher::new();
    prefix.hash(&mut hasher);
    format!("{basename}-{:08x}", (hasher.finish() & 0xffff_ffff) as u32)
}

fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn parse_rfc3339(value: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(value, &Rfc3339).ok()
}

fn push_capped<T>(ring: &mut VecDeque<T>, value: T) {
    if ring.len() >= DEBUG_RING_CAPACITY {
        ring.pop_front();
    }
    ring.push_back(value);
}

pub async fn decide_admission(
    infra: &::app_infra::AppInfra,
    frame: &::app_infra::NewFrame,
    recording_active: bool,
) -> ::app_infra::Result<::app_infra::OcrAdmissionDecision> {
    let key = scope_key(frame);
    let queue_pressure = infra
        .count_queued_or_running_processing_jobs_for_processor(::app_infra::OCR_PROCESSOR)
        .await?;
    let high_queue_pressure = queue_pressure >= HIGH_PRESSURE_THRESHOLD;
    let low_queue_pressure = !high_queue_pressure;
    let first_candidate = with_state(infra.base_dir(), |state| {
        !state
            .admission_scopes
            .get(&key)
            .map(|scope| scope.seen_candidate)
            .unwrap_or(false)
    });
    let context_changed = infra
        .latest_frame_context_differs(frame, key.workspace_prefix.as_deref())
        .await?;
    let captured_at = parse_rfc3339(&frame.captured_at);
    let recent_admitted = with_state(infra.base_dir(), |state| {
        let Some(captured_at) = captured_at else {
            return false;
        };
        state
            .admission_scopes
            .get(&key)
            .and_then(|scope| scope.last_admitted_at)
            .map(|last| (captured_at - last).whole_seconds() <= REPRESENTATIVE_SECONDS)
            .unwrap_or(false)
    });
    let representative_due = !recent_admitted;

    let signals = ::app_infra::OcrAdmissionSignals {
        first_candidate_in_scope: first_candidate,
        context_changed,
        low_queue_pressure,
        representative_due,
        high_queue_pressure,
    };

    let mut decision = if first_candidate {
        ::app_infra::OcrAdmissionDecision::admit(
            ::app_infra::OcrAdmissionReason::AdmittedInitial,
            queue_pressure,
            recording_active,
        )
    } else if context_changed {
        ::app_infra::OcrAdmissionDecision::admit(
            ::app_infra::OcrAdmissionReason::AdmittedContextChange,
            queue_pressure,
            recording_active,
        )
    } else if low_queue_pressure {
        ::app_infra::OcrAdmissionDecision::admit(
            ::app_infra::OcrAdmissionReason::AdmittedLowPressure,
            queue_pressure,
            recording_active,
        )
    } else if representative_due {
        ::app_infra::OcrAdmissionDecision::admit(
            ::app_infra::OcrAdmissionReason::AdmittedRepresentative,
            queue_pressure,
            recording_active,
        )
    } else if recording_active && high_queue_pressure {
        ::app_infra::OcrAdmissionDecision::skip(
            ::app_infra::OcrAdmissionReason::SkippedLowOcrValue,
            queue_pressure,
            recording_active,
        )
    } else {
        ::app_infra::OcrAdmissionDecision::admit(
            ::app_infra::OcrAdmissionReason::AdmittedLowPressure,
            queue_pressure,
            recording_active,
        )
    };
    decision.signals = signals;
    Ok(decision)
}

pub fn record_admission_result(
    infra: &::app_infra::AppInfra,
    result: &::app_infra::CapturedFramePipelineResult,
) {
    let Some(decision) = result.ocr_admission_decision.clone() else {
        return;
    };
    let workspace_prefix = workspace_prefix(&result.frame.file_path);
    let key = AdmissionScopeKey {
        session_id: result.frame.session_id.clone(),
        workspace_prefix: workspace_prefix.clone(),
    };
    let event = OcrAdmissionDebugEventDto {
        occurred_at: now_rfc3339(),
        session_id: result.frame.session_id.clone(),
        workspace_scope: workspace_scope_label(workspace_prefix.as_deref()),
        frame_id: result.frame.id,
        outcome: decision.outcome,
        reason: decision.reason,
        queue_pressure_count: decision.queue_pressure_count,
        recording_active: decision.recording_active,
        job_id: result.job.as_ref().map(|job| job.id),
        related_frame_id: decision.related_frame_id,
        signals: decision.signals.clone(),
    };
    with_state(infra.base_dir(), |state| {
        push_capped(&mut state.admission_events, event.clone());
        if decision.reason != ::app_infra::OcrAdmissionReason::SkippedOcrDisabled {
            let scope = state.admission_scopes.entry(key).or_default();
            scope.seen_candidate = true;
            if decision.outcome == ::app_infra::OcrAdmissionOutcome::Admitted {
                scope.last_admitted_at = parse_rfc3339(&result.frame.captured_at);
            }
        }
    });
    crate::native_capture::debug_log::log_info(format!(
        "OCR admission budget event frame_id={} outcome={} reason={} queue_pressure={} recording_active={}",
        result.frame.id,
        decision.outcome.as_str(),
        decision.reason.as_str(),
        decision.queue_pressure_count,
        decision.recording_active
    ));
}

fn scaled_clamped_duration(
    last_run_ms: u64,
    multiplier: f64,
    min: Duration,
    max: Duration,
) -> Duration {
    let scaled = ((last_run_ms as f64) * multiplier).round() as u64;
    Duration::from_millis(scaled).clamp(min, max)
}

fn cooldown_duration(last_run_ms: u64, recording_active: bool) -> Duration {
    if recording_active {
        scaled_clamped_duration(
            last_run_ms,
            2.5,
            OCR_ACTIVE_RECORDING_COOLDOWN_MIN,
            OCR_ACTIVE_RECORDING_COOLDOWN_MAX,
        )
    } else {
        scaled_clamped_duration(
            last_run_ms,
            0.5,
            OCR_CATCH_UP_COOLDOWN_MIN,
            OCR_CATCH_UP_COOLDOWN_MAX,
        )
    }
}

fn parse_job_timestamp(value: &str) -> Option<OffsetDateTime> {
    if let Ok(parsed) = OffsetDateTime::parse(value, &Rfc3339) {
        return Some(parsed);
    }

    let sqlite_format =
        format_description::parse("[year]-[month]-[day] [hour]:[minute]:[second]").ok()?;
    PrimitiveDateTime::parse(value, &sqlite_format)
        .ok()
        .map(|parsed| parsed.assume_utc())
}

fn timestamp_delta_ms(start: Option<&str>, end: Option<&str>) -> Option<i64> {
    let start = parse_job_timestamp(start?)?;
    let end = parse_job_timestamp(end?)?;
    Some((end - start).whole_milliseconds().max(0) as i64)
}

fn processing_job_queue_wait_ms(job: &::app_infra::ProcessingJob) -> Option<i64> {
    timestamp_delta_ms(Some(&job.queued_at), job.started_at.as_deref())
}

fn observation_count(structured_payload_json: Option<&str>) -> Option<i64> {
    let payload = structured_payload_json?;
    let parsed: serde_json::Value = serde_json::from_str(payload).ok()?;
    parsed
        .get("observations")
        .and_then(|value| value.as_array())
        .map(|items| items.len().min(i64::MAX as usize) as i64)
}

fn execution_event_for_outcome(
    outcome: &::app_infra::ProcessingJobRunOutcome,
    run_duration_ms: i64,
) -> OcrExecutionDebugEventDto {
    let (job, status, result) = match outcome {
        ::app_infra::ProcessingJobRunOutcome::Completed(completion) => {
            (&completion.job, "completed", Some(&completion.result))
        }
        ::app_infra::ProcessingJobRunOutcome::Failed(job) => (job, "failed", None),
    };
    let parsed_payload =
        ::app_infra::FrozenOcrPayload::from_payload_json(job.payload_json.as_deref());
    if let Err(error) = &parsed_payload {
        crate::native_capture::debug_log::log_error(format!(
            "failed to parse OCR payload for budget debug job_id={}: {error}",
            job.id
        ));
    }
    let (provider, model_id, recognition_mode) = parsed_payload
        .ok()
        .map(|payload| {
            let recognition_mode = payload
                .options
                .get("recognitionMode")
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned);
            (payload.provider, payload.model_id, recognition_mode)
        })
        .unwrap_or_else(|| ("unknown".to_string(), None, None));

    OcrExecutionDebugEventDto {
        occurred_at: now_rfc3339(),
        job_id: job.id,
        frame_id: if job.subject_type == ::app_infra::FRAME_SUBJECT_TYPE {
            Some(job.subject_id)
        } else {
            None
        },
        provider,
        model_id,
        recognition_mode,
        status: status.to_string(),
        run_duration_ms,
        queue_wait_ms: processing_job_queue_wait_ms(job),
        result_text_length: result
            .and_then(|result| result.result_text.as_ref())
            .map(|text| text.chars().count().min(i64::MAX as usize) as i64),
        observation_count: result
            .and_then(|result| observation_count(result.structured_payload_json.as_deref())),
        last_error: job.last_error.clone(),
    }
}

pub async fn process_pending_ocr_job_once(
    infra: &::app_infra::AppInfra,
    recording_active: bool,
) -> ::app_infra::Result<OcrProcessingPass> {
    let now = Instant::now();
    let cooldown_remaining = with_state(infra.base_dir(), |state| {
        if state.execution.last_recording_active != recording_active {
            if let (Some(last_run_at), Some(last_run_ms)) =
                (state.execution.last_run_at, state.execution.last_run_ms)
            {
                state.execution.next_due_at =
                    Some(last_run_at + cooldown_duration(last_run_ms, recording_active));
            }
            state.execution.last_recording_active = recording_active;
        }

        state
            .execution
            .next_due_at
            .and_then(|due| due.checked_duration_since(now))
            .filter(|duration| !duration.is_zero())
    });
    if let Some(duration) = cooldown_remaining {
        return Ok(OcrProcessingPass::CoolingDown(duration));
    }

    let started_at = Instant::now();
    let outcome = infra
        .process_next_processing_job_for_processor(::app_infra::OCR_PROCESSOR)
        .await?;
    let Some(outcome) = outcome else {
        return Ok(OcrProcessingPass::Idle);
    };
    let run_duration_ms = started_at.elapsed().as_millis().min(i64::MAX as u128) as i64;
    let event = execution_event_for_outcome(&outcome, run_duration_ms);
    let cooldown = cooldown_duration(run_duration_ms.max(0) as u64, recording_active);
    with_state(infra.base_dir(), |state| {
        let completed_at = Instant::now();
        state.execution.last_run_ms = Some(run_duration_ms.max(0) as u64);
        state.execution.last_run_at = Some(completed_at);
        state.execution.last_status = Some(event.status.clone());
        state.execution.last_recording_active = recording_active;
        state.execution.next_due_at = Some(completed_at + cooldown);
        push_capped(&mut state.execution_events, event.clone());
    });
    crate::native_capture::debug_log::log_info(format!(
        "OCR execution budget paced job_id={} status={} run_duration_ms={} cooldown_ms={} recording_active={}",
        event.job_id,
        event.status,
        run_duration_ms,
        cooldown.as_millis(),
        recording_active
    ));
    Ok(OcrProcessingPass::DidWork)
}

pub async fn debug_for_infra(
    infra: &::app_infra::AppInfra,
) -> ::app_infra::Result<OcrBudgetDebugDto> {
    let queued_or_running_count = infra
        .count_queued_or_running_processing_jobs_for_processor(::app_infra::OCR_PROCESSOR)
        .await?;
    let now = Instant::now();
    Ok(with_state(infra.base_dir(), |state| {
        let cooldown_remaining = state
            .execution
            .next_due_at
            .and_then(|due| due.checked_duration_since(now))
            .unwrap_or_default();
        OcrBudgetDebugDto {
            summary: OcrBudgetExecutionSummaryDto {
                queued_or_running_count,
                execution_state: if cooldown_remaining.is_zero() {
                    "idle".to_string()
                } else {
                    "cooling_down".to_string()
                },
                cooldown_remaining_ms: cooldown_remaining.as_millis().min(i64::MAX as u128) as i64,
                last_run_duration_ms: state.execution.last_run_ms.map(|value| value as i64),
                last_run_status: state.execution.last_status.clone(),
                last_pacing_mode: state.execution.last_run_at.map(|_| {
                    if state.execution.last_recording_active {
                        "recording_active".to_string()
                    } else {
                        "catch_up".to_string()
                    }
                }),
            },
            admission_events: state.admission_events.iter().cloned().rev().collect(),
            execution_events: state.execution_events.iter().cloned().rev().collect(),
        }
    }))
}

#[tauri::command]
pub async fn get_ocr_budget_debug(
    state: tauri::State<'_, crate::app_infra::AppInfraState>,
) -> Result<OcrBudgetDebugDto, String> {
    let infra = std::sync::Arc::clone(&*state);
    debug_for_infra(&infra)
        .await
        .map_err(|error| format!("failed to get OCR budget debug state: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn representative_window_detects_recent_admission() {
        let base_dir = PathBuf::from("/tmp/ocr-budget-test-representative");
        reset_for_base_dir(&base_dir);
        let key = AdmissionScopeKey {
            session_id: "session-a".to_string(),
            workspace_prefix: None,
        };
        with_state(&base_dir, |state| {
            state.admission_scopes.insert(
                key.clone(),
                AdmissionScopeState {
                    seen_candidate: true,
                    last_admitted_at: parse_rfc3339("2026-04-12T10:00:00Z"),
                },
            );
        });

        let recent = with_state(&base_dir, |state| {
            let captured_at = parse_rfc3339("2026-04-12T10:00:14Z").unwrap();
            state
                .admission_scopes
                .get(&key)
                .and_then(|scope| scope.last_admitted_at)
                .map(|last| (captured_at - last).whole_seconds() <= REPRESENTATIVE_SECONDS)
                .unwrap_or(false)
        });
        let stale = with_state(&base_dir, |state| {
            let captured_at = parse_rfc3339("2026-04-12T10:00:16Z").unwrap();
            state
                .admission_scopes
                .get(&key)
                .and_then(|scope| scope.last_admitted_at)
                .map(|last| (captured_at - last).whole_seconds() <= REPRESENTATIVE_SECONDS)
                .unwrap_or(false)
        });

        assert!(recent);
        assert!(!stale);
    }

    #[test]
    fn session_stop_clears_admission_memory() {
        let base_dir = PathBuf::from("/tmp/ocr-budget-test-clear");
        reset_for_base_dir(&base_dir);
        with_state(&base_dir, |state| {
            state.admission_scopes.insert(
                AdmissionScopeKey {
                    session_id: "stopped".to_string(),
                    workspace_prefix: None,
                },
                AdmissionScopeState {
                    seen_candidate: true,
                    last_admitted_at: parse_rfc3339("2026-04-12T10:00:00Z"),
                },
            );
            state.admission_scopes.insert(
                AdmissionScopeKey {
                    session_id: "active".to_string(),
                    workspace_prefix: None,
                },
                AdmissionScopeState {
                    seen_candidate: true,
                    last_admitted_at: parse_rfc3339("2026-04-12T10:00:00Z"),
                },
            );
        });

        clear_sessions_for_base_dir(&base_dir, &["stopped".to_string()]);

        with_state(&base_dir, |state| {
            assert!(!state
                .admission_scopes
                .keys()
                .any(|key| key.session_id == "stopped"));
            assert!(state
                .admission_scopes
                .keys()
                .any(|key| key.session_id == "active"));
        });
    }

    #[test]
    fn timestamp_delta_ms_accepts_sqlite_current_timestamp_format() {
        assert_eq!(
            timestamp_delta_ms(Some("2026-04-12 10:00:00"), Some("2026-04-12 10:00:02")),
            Some(2000)
        );
        assert_eq!(
            timestamp_delta_ms(Some("2026-04-12T10:00:00Z"), Some("2026-04-12T10:00:02Z")),
            Some(2000)
        );
    }
}
