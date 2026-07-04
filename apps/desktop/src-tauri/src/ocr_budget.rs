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
/// Minimum gap between two visual-novelty admissions in the same scope. The
/// firm cost bound: even a continuous stream of new frames adds at most one
/// novelty OCR read per this interval on top of the representative sampling.
const NOVELTY_MIN_INTERVAL_SECONDS: i64 = 2;
/// How many recently-seen fingerprints each scope remembers when deciding
/// whether a frame's `equivalence_hint` is new.
const NOVELTY_RECENT_FINGERPRINT_CAPACITY: usize = 128;
/// Number of consecutive novel frames that flips a scope into the
/// continuous-novelty (video/animation) regime, suppressing novelty admission
/// back to plain time-sampling until the run breaks.
const NOVELTY_SUSTAINED_RUN_SUPPRESS: u32 = 10;
/// Maximum gap between two novel frames for them to count as part of the same
/// continuous-novelty burst. Tied to the rate-floor interval: only a stream of
/// novel frames arriving at least this often (video/animation) builds toward
/// the suppressor; a longer pause in novel-content arrival un-latches it and
/// starts a fresh potential burst, so temporally-sparse distinct screens
/// (normal browsing) never trip the video guard.
const NOVELTY_CONTINUOUS_GAP_SECONDS: i64 = NOVELTY_MIN_INTERVAL_SECONDS;
const OCR_ACTIVE_RECORDING_COOLDOWN_MIN: Duration = Duration::from_millis(1000);
const OCR_ACTIVE_RECORDING_COOLDOWN_MAX: Duration = Duration::from_millis(10000);
const OCR_CATCH_UP_COOLDOWN_MIN: Duration = Duration::from_millis(250);
const OCR_CATCH_UP_COOLDOWN_MAX: Duration = Duration::from_millis(2000);

static OCR_BUDGET_STATES: OnceLock<Mutex<HashMap<PathBuf, OcrBudgetState>>> = OnceLock::new();

macro_rules! ocr_budget_trace {
    ($($arg:tt)*) => {{
        #[cfg(feature = "ocr-budget-trace")]
        {
            crate::native_capture::debug_log::log(format!($($arg)*));
        }
    }};
}

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
    /// FIFO ring of recently-seen fingerprint hashes (`equivalence_hint`),
    /// bounded by `NOVELTY_RECENT_FINGERPRINT_CAPACITY`. A frame is novel when
    /// its hash is absent here.
    recent_fingerprints: VecDeque<u64>,
    /// Timestamp of the last frame that produced a real OCR read of current
    /// content (any Admitted outcome); gates the per-scope novelty rate floor so
    /// that any current-content read makes an immediate novelty read redundant.
    last_novelty_admitted_at: Option<OffsetDateTime>,
    /// Capture time of the last frame that counted as novel-for-bookkeeping;
    /// used to decide whether a new novel frame is temporally continuous with
    /// the previous one (and so extends the burst) or starts a fresh run.
    last_novel_frame_at: Option<OffsetDateTime>,
    /// Length of the current run of temporally-continuous novel frames; drives
    /// the continuous-novelty (video) suppression guard. Only novel frames that
    /// arrive close in time to the previous novel frame extend the run.
    consecutive_novel_run: u32,
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

fn fingerprint_hash(hint: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    hint.hash(&mut hasher);
    hasher.finish()
}

fn frame_fingerprint(frame: &::app_infra::NewFrame) -> Option<u64> {
    frame
        .equivalence
        .ready_parts()
        .map(|(hint, _, _)| fingerprint_hash(hint))
}

fn stored_frame_fingerprint(frame: &::app_infra::Frame) -> Option<u64> {
    frame
        .equivalence
        .ready_parts()
        .map(|(hint, _, _)| fingerprint_hash(hint))
}

fn remember_fingerprint(ring: &mut VecDeque<u64>, fingerprint: u64) {
    while ring.len() >= NOVELTY_RECENT_FINGERPRINT_CAPACITY {
        ring.pop_front();
    }
    ring.push_back(fingerprint);
}

/// First-occurrence detection: a frame is novel when its fingerprint is absent
/// from the scope's recent ring. A missing fingerprint (equivalence not ready)
/// is treated as non-novel so existing time-sampling behavior stands.
fn fingerprint_is_novel(recent_fingerprints: &VecDeque<u64>, fingerprint: Option<u64>) -> bool {
    match fingerprint {
        Some(fingerprint) => !recent_fingerprints.contains(&fingerprint),
        None => false,
    }
}

/// Per-scope rate floor: at most one novelty admission per
/// `NOVELTY_MIN_INTERVAL_SECONDS`. A scope with no prior novelty admission is
/// clear; an unparseable capture time denies (cannot honor the floor).
fn novelty_rate_floor_satisfied(
    captured_at: Option<OffsetDateTime>,
    last_novelty_admitted_at: Option<OffsetDateTime>,
) -> bool {
    match (captured_at, last_novelty_admitted_at) {
        (_, None) => true,
        (Some(captured_at), Some(last)) => {
            (captured_at - last).whole_seconds() >= NOVELTY_MIN_INTERVAL_SECONDS
        }
        (None, Some(_)) => false,
    }
}

/// Video/animation guard: once a scope has produced a sustained run of novel
/// frames it is treated as continuous-novelty and novelty admission is
/// suppressed until a repeated frame resets the run.
fn in_continuous_novelty_burst(consecutive_novel_run: u32) -> bool {
    consecutive_novel_run >= NOVELTY_SUSTAINED_RUN_SUPPRESS
}

/// Two novel frames belong to the same continuous-novelty burst only when the
/// later one arrives within `NOVELTY_CONTINUOUS_GAP_SECONDS` of the previous
/// novel frame. A missing previous timestamp or an unparseable current capture
/// time is treated as a gap, so the run restarts rather than extending blindly.
///
/// The gap is compared with sub-second precision (not truncated whole seconds),
/// so a 2.9s gap does not slip under a 2s threshold. A negative gap (a current
/// capture time earlier than the previous novel frame, e.g. a clock adjustment
/// or out-of-order ingestion) is treated as a gap rather than continuity, so an
/// out-of-order frame restarts the run instead of extending the burst blindly.
fn novel_frame_is_temporally_continuous(
    previous_novel_at: Option<OffsetDateTime>,
    captured_at: Option<OffsetDateTime>,
) -> bool {
    match (previous_novel_at, captured_at) {
        (Some(previous), Some(current)) => {
            let gap = current - previous;
            gap >= time::Duration::ZERO
                && gap <= time::Duration::seconds(NOVELTY_CONTINUOUS_GAP_SECONDS)
        }
        _ => false,
    }
}

/// Maintain a scope's novelty memory for a frame that was just processed,
/// reasoning from the FINAL admission outcome/reason rather than only the
/// decide-time signal.
///
/// `decide_time_fingerprint_novel` is the (possibly stale) novelty signal
/// computed at decide time; the pipeline may have OVERRIDDEN the decision to
/// `SkippedEquivalentFrame` afterward while keeping that stale signal, so a
/// confirmed duplicate must not be treated as novel here (finding #5).
///
/// Bookkeeping rules:
/// - "novel-for-bookkeeping" = decide-time novel AND the final reason is not
///   `SkippedEquivalentFrame` (a confirmed duplicate is never novel).
/// - Remember a fingerprint only when its content is actually OCR-covered:
///   either the frame was Admitted (a real read happens) or it deduped to an
///   eligible equivalent (`SkippedEquivalentFrame`, covered by reuse). A frame
///   that was merely skipped for rate-floor / queue pressure / low value is
///   left novel so a later frame of the same dwelled-on screen can still be
///   admitted once the floor clears (finding #1).
/// - The continuous-novelty run reflects TEMPORAL CONTINUITY: a
///   novel-for-bookkeeping frame extends the run only when it is temporally
///   adjacent to the previous novel frame, otherwise the run restarts at 1; a
///   frame that is not novel-for-bookkeeping resets the run to 0. Only a
///   genuinely continuous (high-frequency) novel stream trips the suppressor,
///   and a pause in novel-content arrival un-latches it (finding #3).
/// - The novelty rate-floor anchor advances whenever ANY frame is Admitted,
///   because any real OCR read of current content makes an immediate novelty
///   read redundant (finding #7); it is never stamped with a `None` capture
///   time (finding #6).
///
/// Tension between #1 and burst recovery: because a rate-floor-skipped novel
/// frame is intentionally NOT remembered, a later identical frame stays novel.
/// During an active burst that repeated frame is still novel-for-bookkeeping,
/// so it can re-extend the run rather than reset it; in the worst case a screen
/// dwelled on during a burst is only re-read at the representative cadence
/// (~`REPRESENTATIVE_SECONDS`) instead of via novelty. This matches the ADR's
/// "video falls back to time-sampling" intent. The temporal-gap reset above is
/// what un-latches the burst once novel content stops arriving continuously.
fn record_novelty_memory(
    scope: &mut AdmissionScopeState,
    decide_time_fingerprint_novel: bool,
    fingerprint: Option<u64>,
    outcome: ::app_infra::OcrAdmissionOutcome,
    reason: ::app_infra::OcrAdmissionReason,
    captured_at: Option<OffsetDateTime>,
) {
    let admitted = outcome == ::app_infra::OcrAdmissionOutcome::Admitted;
    let deduped_to_equivalent = reason == ::app_infra::OcrAdmissionReason::SkippedEquivalentFrame;
    // A confirmed duplicate (equivalence reuse) is never novel, regardless of
    // the stale decide-time signal.
    let novel_for_bookkeeping = decide_time_fingerprint_novel && !deduped_to_equivalent;
    // Content is OCR-covered when it was actually read (admitted) or reused via
    // an eligible equivalent; only then do we remember its fingerprint.
    let content_is_covered = admitted || deduped_to_equivalent;

    if novel_for_bookkeeping {
        if novel_frame_is_temporally_continuous(scope.last_novel_frame_at, captured_at) {
            scope.consecutive_novel_run = scope.consecutive_novel_run.saturating_add(1);
        } else {
            scope.consecutive_novel_run = 1;
        }
        scope.last_novel_frame_at = captured_at;
    } else {
        scope.consecutive_novel_run = 0;
    }

    if content_is_covered {
        if let Some(fingerprint) = fingerprint {
            remember_fingerprint(&mut scope.recent_fingerprints, fingerprint);
        }
    }

    // Any real OCR read of current content advances the novelty rate-floor
    // anchor; never stamp a `None` capture time (which would permanently
    // disable the floor).
    if admitted {
        if let Some(captured_at) = captured_at {
            scope.last_novelty_admitted_at = Some(captured_at);
        }
    }
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

    let fingerprint = frame_fingerprint(frame);
    let empty_ring = VecDeque::new();
    let (fingerprint_novel_in_scope, novelty_admission_available) =
        with_state(infra.base_dir(), |state| {
            let scope = state.admission_scopes.get(&key);
            let recent_fingerprints = scope
                .map(|scope| &scope.recent_fingerprints)
                .unwrap_or(&empty_ring);
            let fingerprint_novel_in_scope = fingerprint_is_novel(recent_fingerprints, fingerprint);
            let rate_floor_satisfied = novelty_rate_floor_satisfied(
                captured_at,
                scope.and_then(|scope| scope.last_novelty_admitted_at),
            );
            let in_burst = scope
                .map(|scope| in_continuous_novelty_burst(scope.consecutive_novel_run))
                .unwrap_or(false);
            (
                fingerprint_novel_in_scope,
                rate_floor_satisfied && !in_burst,
            )
        });

    let signals = ::app_infra::OcrAdmissionSignals {
        first_candidate_in_scope: first_candidate,
        context_changed,
        low_queue_pressure,
        representative_due,
        high_queue_pressure,
        fingerprint_novel_in_scope,
        novelty_admission_available,
    };

    Ok(ocr_admission_decision_for_signals(
        &signals,
        queue_pressure,
        recording_active,
    ))
}

fn ocr_admission_decision_for_signals(
    signals: &::app_infra::OcrAdmissionSignals,
    queue_pressure: i64,
    recording_active: bool,
) -> ::app_infra::OcrAdmissionDecision {
    let mut decision = if signals.first_candidate_in_scope {
        ::app_infra::OcrAdmissionDecision::admit(
            ::app_infra::OcrAdmissionReason::AdmittedInitial,
            queue_pressure,
            recording_active,
        )
    } else if signals.context_changed {
        ::app_infra::OcrAdmissionDecision::admit(
            ::app_infra::OcrAdmissionReason::AdmittedContextChange,
            queue_pressure,
            recording_active,
        )
    } else if recording_active {
        if signals.low_queue_pressure && signals.representative_due {
            ::app_infra::OcrAdmissionDecision::admit(
                ::app_infra::OcrAdmissionReason::AdmittedRepresentative,
                queue_pressure,
                recording_active,
            )
        } else if signals.low_queue_pressure
            && signals.fingerprint_novel_in_scope
            && signals.novelty_admission_available
        {
            ::app_infra::OcrAdmissionDecision::admit(
                ::app_infra::OcrAdmissionReason::AdmittedVisualNovelty,
                queue_pressure,
                recording_active,
            )
        } else {
            ::app_infra::OcrAdmissionDecision::skip(
                ::app_infra::OcrAdmissionReason::SkippedLowOcrValue,
                queue_pressure,
                recording_active,
            )
        }
    } else if signals.low_queue_pressure {
        ::app_infra::OcrAdmissionDecision::admit(
            ::app_infra::OcrAdmissionReason::AdmittedLowPressure,
            queue_pressure,
            recording_active,
        )
    } else if signals.representative_due {
        ::app_infra::OcrAdmissionDecision::admit(
            ::app_infra::OcrAdmissionReason::AdmittedRepresentative,
            queue_pressure,
            recording_active,
        )
    } else {
        ::app_infra::OcrAdmissionDecision::skip(
            ::app_infra::OcrAdmissionReason::SkippedLowOcrValue,
            queue_pressure,
            recording_active,
        )
    };
    decision.signals = signals.clone();
    decision
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
    // The decide-time novelty signal; may be stale if the pipeline overrode the
    // decision to SkippedEquivalentFrame afterward. `record_novelty_memory`
    // reconciles it against the final outcome/reason.
    let decide_time_fingerprint_novel = decision.signals.fingerprint_novel_in_scope;
    let fingerprint = stored_frame_fingerprint(&result.frame);
    with_state(infra.base_dir(), |state| {
        push_capped(&mut state.admission_events, event.clone());
        if decision.reason != ::app_infra::OcrAdmissionReason::SkippedOcrDisabled {
            let scope = state.admission_scopes.entry(key).or_default();
            scope.seen_candidate = true;
            if decision.outcome == ::app_infra::OcrAdmissionOutcome::Admitted {
                scope.last_admitted_at = parse_rfc3339(&result.frame.captured_at);
            }
            // Novelty memory updates for every frame so first-occurrence and the
            // continuous-novelty run are tracked even when the frame is skipped.
            record_novelty_memory(
                scope,
                decide_time_fingerprint_novel,
                fingerprint,
                decision.outcome,
                decision.reason,
                parse_rfc3339(&result.frame.captured_at),
            );
        }
    });
    ocr_budget_trace!(
        "OCR admission budget event frame_id={} outcome={} reason={} queue_pressure={} recording_active={}",
        result.frame.id,
        decision.outcome.as_str(),
        decision.reason.as_str(),
        decision.queue_pressure_count,
        decision.recording_active
    );
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
        ::app_infra::ProcessingJobRunOutcome::RequeuedForLiveness(job) => (job, "requeued", None),
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
    ocr_budget_trace!(
        "OCR execution budget paced job_id={} status={} run_duration_ms={} cooldown_ms={} recording_active={}",
        event.job_id,
        event.status,
        run_duration_ms,
        cooldown.as_millis(),
        recording_active
    );
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

    fn admission_signals(
        first_candidate_in_scope: bool,
        context_changed: bool,
        low_queue_pressure: bool,
        representative_due: bool,
        high_queue_pressure: bool,
    ) -> ::app_infra::OcrAdmissionSignals {
        ::app_infra::OcrAdmissionSignals {
            first_candidate_in_scope,
            context_changed,
            low_queue_pressure,
            representative_due,
            high_queue_pressure,
            fingerprint_novel_in_scope: false,
            novelty_admission_available: false,
        }
    }

    #[test]
    fn active_recording_skips_recent_low_pressure_frames() {
        let signals = admission_signals(false, false, true, false, false);

        let decision = ocr_admission_decision_for_signals(&signals, 0, true);

        assert_eq!(decision.outcome, ::app_infra::OcrAdmissionOutcome::Skipped);
        assert_eq!(
            decision.reason,
            ::app_infra::OcrAdmissionReason::SkippedLowOcrValue
        );
    }

    #[test]
    fn active_recording_admits_due_representative_frames() {
        let signals = admission_signals(false, false, true, true, false);

        let decision = ocr_admission_decision_for_signals(&signals, 0, true);

        assert_eq!(decision.outcome, ::app_infra::OcrAdmissionOutcome::Admitted);
        assert_eq!(
            decision.reason,
            ::app_infra::OcrAdmissionReason::AdmittedRepresentative
        );
    }

    #[test]
    fn active_recording_respects_high_queue_pressure_for_representatives() {
        let signals = admission_signals(false, false, false, true, true);

        let decision = ocr_admission_decision_for_signals(&signals, HIGH_PRESSURE_THRESHOLD, true);

        assert_eq!(decision.outcome, ::app_infra::OcrAdmissionOutcome::Skipped);
        assert_eq!(
            decision.reason,
            ::app_infra::OcrAdmissionReason::SkippedLowOcrValue
        );
    }

    #[test]
    fn active_recording_admits_novel_frame_when_novelty_available() {
        let signals = ::app_infra::OcrAdmissionSignals {
            fingerprint_novel_in_scope: true,
            novelty_admission_available: true,
            ..admission_signals(false, false, true, false, false)
        };

        let decision = ocr_admission_decision_for_signals(&signals, 0, true);

        assert_eq!(decision.outcome, ::app_infra::OcrAdmissionOutcome::Admitted);
        assert_eq!(
            decision.reason,
            ::app_infra::OcrAdmissionReason::AdmittedVisualNovelty
        );
    }

    #[test]
    fn active_recording_skips_novel_frame_when_novelty_unavailable() {
        // Rate floor not satisfied or in a continuous-novelty burst.
        let signals = ::app_infra::OcrAdmissionSignals {
            fingerprint_novel_in_scope: true,
            novelty_admission_available: false,
            ..admission_signals(false, false, true, false, false)
        };

        let decision = ocr_admission_decision_for_signals(&signals, 0, true);

        assert_eq!(decision.outcome, ::app_infra::OcrAdmissionOutcome::Skipped);
        assert_eq!(
            decision.reason,
            ::app_infra::OcrAdmissionReason::SkippedLowOcrValue
        );
    }

    #[test]
    fn active_recording_skips_novel_frame_under_high_queue_pressure() {
        let signals = ::app_infra::OcrAdmissionSignals {
            fingerprint_novel_in_scope: true,
            novelty_admission_available: true,
            ..admission_signals(false, false, false, false, true)
        };

        let decision = ocr_admission_decision_for_signals(&signals, HIGH_PRESSURE_THRESHOLD, true);

        assert_eq!(decision.outcome, ::app_infra::OcrAdmissionOutcome::Skipped);
        assert_eq!(
            decision.reason,
            ::app_infra::OcrAdmissionReason::SkippedLowOcrValue
        );
    }

    #[test]
    fn active_recording_prefers_representative_over_novelty_when_due() {
        // A due representative outranks the novelty path even if both qualify.
        let signals = ::app_infra::OcrAdmissionSignals {
            fingerprint_novel_in_scope: true,
            novelty_admission_available: true,
            ..admission_signals(false, false, true, true, false)
        };

        let decision = ocr_admission_decision_for_signals(&signals, 0, true);

        assert_eq!(decision.outcome, ::app_infra::OcrAdmissionOutcome::Admitted);
        assert_eq!(
            decision.reason,
            ::app_infra::OcrAdmissionReason::AdmittedRepresentative
        );
    }

    #[test]
    fn active_recording_skips_non_novel_frame_even_when_novelty_available() {
        let signals = ::app_infra::OcrAdmissionSignals {
            fingerprint_novel_in_scope: false,
            novelty_admission_available: true,
            ..admission_signals(false, false, true, false, false)
        };

        let decision = ocr_admission_decision_for_signals(&signals, 0, true);

        assert_eq!(decision.outcome, ::app_infra::OcrAdmissionOutcome::Skipped);
        assert_eq!(
            decision.reason,
            ::app_infra::OcrAdmissionReason::SkippedLowOcrValue
        );
    }

    #[test]
    fn catch_up_still_admits_low_pressure_frames_when_not_recording() {
        let signals = admission_signals(false, false, true, false, false);

        let decision = ocr_admission_decision_for_signals(&signals, 0, false);

        assert_eq!(decision.outcome, ::app_infra::OcrAdmissionOutcome::Admitted);
        assert_eq!(
            decision.reason,
            ::app_infra::OcrAdmissionReason::AdmittedLowPressure
        );
    }

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
                    ..Default::default()
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
                    ..Default::default()
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
                    ..Default::default()
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
    fn fingerprint_is_novel_detects_first_occurrence_and_repeat() {
        let mut ring = VecDeque::new();
        assert!(fingerprint_is_novel(&ring, Some(7)));
        remember_fingerprint(&mut ring, 7);
        assert!(!fingerprint_is_novel(&ring, Some(7)));
        assert!(fingerprint_is_novel(&ring, Some(8)));
    }

    #[test]
    fn fingerprint_without_equivalence_is_not_novel() {
        let ring = VecDeque::new();
        assert!(!fingerprint_is_novel(&ring, None));
    }

    #[test]
    fn fingerprint_ring_evicts_oldest_and_re_novels_it() {
        let mut ring = VecDeque::new();
        for value in 0..(NOVELTY_RECENT_FINGERPRINT_CAPACITY as u64) {
            remember_fingerprint(&mut ring, value);
        }
        assert_eq!(ring.len(), NOVELTY_RECENT_FINGERPRINT_CAPACITY);
        // The oldest fingerprint (0) is still remembered until one more pushes it out.
        assert!(!fingerprint_is_novel(&ring, Some(0)));
        remember_fingerprint(&mut ring, u64::MAX);
        assert_eq!(ring.len(), NOVELTY_RECENT_FINGERPRINT_CAPACITY);
        // After eviction the oldest fingerprint reads as novel again.
        assert!(fingerprint_is_novel(&ring, Some(0)));
        assert!(!fingerprint_is_novel(&ring, Some(1)));
    }

    #[test]
    fn novelty_rate_floor_respects_minimum_interval() {
        let last = parse_rfc3339("2026-04-12T10:00:00Z");
        // No prior novelty admission: floor is clear regardless of capture time.
        assert!(novelty_rate_floor_satisfied(
            parse_rfc3339("2026-04-12T10:00:00Z"),
            None
        ));
        // Below the interval: denied.
        assert!(!novelty_rate_floor_satisfied(
            parse_rfc3339("2026-04-12T10:00:01Z"),
            last
        ));
        // Exactly at the interval: satisfied.
        assert!(novelty_rate_floor_satisfied(
            parse_rfc3339("2026-04-12T10:00:02Z"),
            last
        ));
        // Unparseable capture time with a prior admission: denied.
        assert!(!novelty_rate_floor_satisfied(None, last));
    }

    #[test]
    fn continuous_novelty_burst_trips_at_threshold() {
        assert!(!in_continuous_novelty_burst(
            NOVELTY_SUSTAINED_RUN_SUPPRESS - 1
        ));
        assert!(in_continuous_novelty_burst(NOVELTY_SUSTAINED_RUN_SUPPRESS));
        assert!(in_continuous_novelty_burst(
            NOVELTY_SUSTAINED_RUN_SUPPRESS + 5
        ));
    }

    /// Capture time `base_seconds + offset` (UTC) for building temporally
    /// adjacent or spaced novel frames in novelty-memory tests.
    fn captured_at_seconds(offset: i64) -> Option<OffsetDateTime> {
        Some(OffsetDateTime::from_unix_timestamp(1_700_000_000 + offset).unwrap())
    }

    fn admitted_novelty() -> (
        ::app_infra::OcrAdmissionOutcome,
        ::app_infra::OcrAdmissionReason,
    ) {
        (
            ::app_infra::OcrAdmissionOutcome::Admitted,
            ::app_infra::OcrAdmissionReason::AdmittedVisualNovelty,
        )
    }

    fn skipped_low_value() -> (
        ::app_infra::OcrAdmissionOutcome,
        ::app_infra::OcrAdmissionReason,
    ) {
        (
            ::app_infra::OcrAdmissionOutcome::Skipped,
            ::app_infra::OcrAdmissionReason::SkippedLowOcrValue,
        )
    }

    fn skipped_equivalent() -> (
        ::app_infra::OcrAdmissionOutcome,
        ::app_infra::OcrAdmissionReason,
    ) {
        (
            ::app_infra::OcrAdmissionOutcome::Skipped,
            ::app_infra::OcrAdmissionReason::SkippedEquivalentFrame,
        )
    }

    #[test]
    fn temporally_continuous_novel_frames_grow_run_and_trip_burst() {
        let mut scope = AdmissionScopeState::default();
        let (outcome, reason) = admitted_novelty();
        // Distinct novel frames arriving one second apart (within the 2s gap).
        for step in 0..(NOVELTY_SUSTAINED_RUN_SUPPRESS as i64) {
            record_novelty_memory(
                &mut scope,
                true,
                Some(step as u64),
                outcome,
                reason,
                captured_at_seconds(step),
            );
        }
        assert_eq!(scope.consecutive_novel_run, NOVELTY_SUSTAINED_RUN_SUPPRESS);
        assert!(in_continuous_novelty_burst(scope.consecutive_novel_run));
    }

    #[test]
    fn non_novel_frame_resets_run() {
        let mut scope = AdmissionScopeState {
            consecutive_novel_run: NOVELTY_SUSTAINED_RUN_SUPPRESS,
            last_novel_frame_at: captured_at_seconds(5),
            ..Default::default()
        };
        let (outcome, reason) = skipped_low_value();
        // A non-novel frame (decide-time not novel) resets the run to 0.
        record_novelty_memory(
            &mut scope,
            false,
            Some(99),
            outcome,
            reason,
            captured_at_seconds(6),
        );
        assert_eq!(scope.consecutive_novel_run, 0);
        assert!(!in_continuous_novelty_burst(scope.consecutive_novel_run));
    }

    #[test]
    fn rate_floor_skipped_novel_frame_is_not_remembered_and_stays_novel() {
        // Finding #1: a NOVEL frame skipped for low OCR value (rate floor /
        // queue pressure) must NOT be remembered, so a later identical frame is
        // still novel and can be admitted once the floor clears.
        let mut scope = AdmissionScopeState::default();
        let (outcome, reason) = skipped_low_value();
        record_novelty_memory(
            &mut scope,
            true,
            Some(42),
            outcome,
            reason,
            captured_at_seconds(0),
        );
        // Fingerprint was not remembered.
        assert!(fingerprint_is_novel(&scope.recent_fingerprints, Some(42)));
        // A skipped frame is not novel-for-coverage but the decide-time novel
        // signal still moves the run; here a single skipped frame leaves run at
        // 1 (no prior novel frame to be continuous with).
        assert_eq!(scope.consecutive_novel_run, 1);

        // Later, the same screen is admitted via novelty: now it is covered.
        let (outcome, reason) = admitted_novelty();
        record_novelty_memory(
            &mut scope,
            true,
            Some(42),
            outcome,
            reason,
            captured_at_seconds(3),
        );
        assert!(!fingerprint_is_novel(&scope.recent_fingerprints, Some(42)));
    }

    #[test]
    fn skipped_equivalent_resets_run_and_is_remembered() {
        // Finding #5: a SkippedEquivalentFrame is a confirmed duplicate. Even
        // though the stale decide-time signal says novel, it must reset the run
        // (not count as novel) yet its fingerprint IS covered, so remember it.
        let mut scope = AdmissionScopeState {
            consecutive_novel_run: 4,
            last_novel_frame_at: captured_at_seconds(0),
            ..Default::default()
        };
        let (outcome, reason) = skipped_equivalent();
        record_novelty_memory(
            &mut scope,
            true, // stale decide-time novelty
            Some(7),
            outcome,
            reason,
            captured_at_seconds(1),
        );
        // Confirmed duplicate resets the run.
        assert_eq!(scope.consecutive_novel_run, 0);
        // But its content is covered by reuse, so it is remembered.
        assert!(!fingerprint_is_novel(&scope.recent_fingerprints, Some(7)));
    }

    #[test]
    fn temporal_gap_un_latches_burst_and_restarts_run() {
        // Finding #3: a continuous run of novel frames trips the burst, but a
        // large gap before the next novel frame restarts the run (un-latch).
        let mut scope = AdmissionScopeState::default();
        let (outcome, reason) = admitted_novelty();
        for step in 0..(NOVELTY_SUSTAINED_RUN_SUPPRESS as i64) {
            record_novelty_memory(
                &mut scope,
                true,
                Some(step as u64),
                outcome,
                reason,
                captured_at_seconds(step),
            );
        }
        assert!(in_continuous_novelty_burst(scope.consecutive_novel_run));

        // A novel frame far in the future (well past the continuous gap)
        // restarts the run at 1 rather than extending the burst.
        let far = (NOVELTY_SUSTAINED_RUN_SUPPRESS as i64) + 100;
        record_novelty_memory(
            &mut scope,
            true,
            Some(9999),
            outcome,
            reason,
            captured_at_seconds(far),
        );
        assert_eq!(scope.consecutive_novel_run, 1);
        assert!(!in_continuous_novelty_burst(scope.consecutive_novel_run));
    }

    #[test]
    fn sub_second_gap_beyond_threshold_is_not_continuous() {
        // A gap of 2.9s exceeds the 2s continuous-gap threshold, so the frames
        // are NOT part of the same continuous-novelty burst. The earlier
        // `.whole_seconds()` comparison truncated 2.9 -> 2 and wrongly counted
        // it as continuous; the gap must be compared with sub-second precision.
        let previous = captured_at_seconds(0);
        let current = previous.map(|t| t + time::Duration::milliseconds(2_900));
        assert!(!novel_frame_is_temporally_continuous(previous, current));
    }

    #[test]
    fn gap_at_exact_threshold_is_continuous() {
        // Exactly NOVELTY_CONTINUOUS_GAP_SECONDS apart is still continuous
        // (frames arriving "at least this often" build the burst).
        let previous = captured_at_seconds(0);
        let current = previous.map(|t| t + time::Duration::seconds(NOVELTY_CONTINUOUS_GAP_SECONDS));
        assert!(novel_frame_is_temporally_continuous(previous, current));
    }

    #[test]
    fn regressing_timestamp_is_not_continuous() {
        // A current capture time earlier than the previous novel frame (clock
        // adjustment / out-of-order ingestion) yields a negative gap. That is
        // not a high-frequency novel burst, so it must NOT count as continuous
        // and should restart the run rather than extend it.
        let previous = captured_at_seconds(10);
        let current = captured_at_seconds(9);
        assert!(!novel_frame_is_temporally_continuous(previous, current));
    }

    #[test]
    fn any_admitted_reason_advances_novelty_rate_floor_anchor() {
        // Finding #7: any Admitted frame (representative, initial, context,
        // low-pressure, novelty) advances last_novelty_admitted_at.
        let representative = (
            ::app_infra::OcrAdmissionOutcome::Admitted,
            ::app_infra::OcrAdmissionReason::AdmittedRepresentative,
        );
        let mut scope = AdmissionScopeState::default();
        record_novelty_memory(
            &mut scope,
            true,
            Some(1),
            representative.0,
            representative.1,
            captured_at_seconds(5),
        );
        assert_eq!(scope.last_novelty_admitted_at, captured_at_seconds(5));

        // A novelty admission also advances it.
        let (outcome, reason) = admitted_novelty();
        record_novelty_memory(
            &mut scope,
            true,
            Some(2),
            outcome,
            reason,
            captured_at_seconds(9),
        );
        assert_eq!(scope.last_novelty_admitted_at, captured_at_seconds(9));
    }

    #[test]
    fn skipped_frame_does_not_advance_novelty_rate_floor_anchor() {
        let mut scope = AdmissionScopeState {
            last_novelty_admitted_at: captured_at_seconds(5),
            ..Default::default()
        };
        let (outcome, reason) = skipped_low_value();
        record_novelty_memory(
            &mut scope,
            true,
            Some(1),
            outcome,
            reason,
            captured_at_seconds(9),
        );
        // A skipped frame is not a real read of current content; anchor stays.
        assert_eq!(scope.last_novelty_admitted_at, captured_at_seconds(5));
    }

    #[test]
    fn none_capture_time_never_overwrites_novelty_rate_floor_anchor() {
        // Finding #6: a None capture time must never be stamped into
        // last_novelty_admitted_at (which would permanently disable the floor).
        let mut scope = AdmissionScopeState {
            last_novelty_admitted_at: captured_at_seconds(5),
            ..Default::default()
        };
        let (outcome, reason) = admitted_novelty();
        record_novelty_memory(&mut scope, true, Some(1), outcome, reason, None);
        // Prior parseable anchor is preserved, not clobbered by None.
        assert_eq!(scope.last_novelty_admitted_at, captured_at_seconds(5));
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
