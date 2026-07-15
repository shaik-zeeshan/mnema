//! Per-feature health rollup for the debug dock.
//!
//! The dock polls **only** this command, so it composes the cheap reads the
//! other debug commands already do (one pipeline `GROUP BY`, the semantic-index
//! status, and three in-memory state reads) into one ok/warn/error dot per
//! feature plus a plain-language reason for the tooltip.
//!
//! Severity is deliberately conservative (PLAN "Further Notes"): a transient
//! **liveness** condition is `Warn`, never `Error` — low-disk suspension and
//! display-unavailable keep the session alive and recover on their own
//! ([ADR 0021]), and Deepgram connectivity/auth requeues never increment a
//! failure count at all ([ADR 0048]), so they surface as queued work, not as
//! failures. Unknown (`None`) is not an error either: it means "no snapshot",
//! which reads as idle.
//!
//! [ADR 0021]: ../../../../docs/adr/0021-recover-from-display-unavailable-as-transient-liveness.md
//! [ADR 0048]: ../../../../docs/adr/0048-cloud-transcription-errors-are-transient-liveness-not-job-failures.md

use serde::Serialize;

use crate::app_infra::AppInfraState;
use crate::debug_status::{
    get_semantic_index_status, SemanticIndexStatusDto, SemanticWorkerHealthState,
};
use crate::native_capture::metadata::capture_privacy_debug_info;
use crate::native_capture::{
    read_recording_settings, CaptureMetadataState, NativeCaptureState, RecordingSettingsState,
};

/// The dock's health dot. Ordered so `max` picks the worst.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DebugSeverity {
    Ok,
    Warn,
    Error,
}

/// One debug section. Mirrors the PLAN's section list (Logs has no health).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DebugFeature {
    Capture,
    Privacy,
    Ocr,
    Transcription,
    Diarization,
    Embeddings,
    AiRuntime,
    UserContext,
    JobsAndStorage,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureHealthDto {
    pub feature: DebugFeature,
    pub severity: DebugSeverity,
    /// Short plain-language sentence for the dot's tooltip.
    pub reason: String,
}

/// Everything [`roll_up`] needs, already gathered. Flat booleans rather than the
/// source structs so the rules are unit-testable without a DB or Tauri state.
pub struct DebugHealthInputs {
    /// Only processors with at least one job appear — a missing lane is idle.
    pub pipeline: Vec<::app_infra::ProcessorPipelineStatus>,
    pub semantic: SemanticIndexStatusDto,
    pub capture_running: bool,
    pub capture_inactivity_paused: bool,
    pub capture_user_paused: bool,
    /// Low disk suspended the session — transient liveness, recovers by itself.
    pub capture_low_disk_suspended: bool,
    pub privacy_excluded_app_count: usize,
    pub privacy_filter_applied: bool,
    pub ai_enabled: bool,
    pub ai_has_default_model: bool,
    pub user_context_enabled: bool,
}

fn health(feature: DebugFeature, severity: DebugSeverity, reason: impl Into<String>) -> FeatureHealthDto {
    FeatureHealthDto {
        feature,
        severity,
        reason: reason.into(),
    }
}

/// First line of an error string, capped — the reason is a tooltip, not a log.
fn error_summary(error: &str) -> String {
    let line = error.lines().next().unwrap_or("").trim();
    if line.chars().count() > 120 {
        format!("{}…", line.chars().take(120).collect::<String>())
    } else {
        line.to_string()
    }
}

/// The shared rule for one processor lane (OCR / Transcription / Diarization).
///
/// An absent lane is idle, not broken: the pipeline query is a pure
/// `GROUP BY processor`, so a fresh install has no row at all. Failed jobs are
/// `Warn` — the data is recoverable with a reprocess, and a lane never escalates
/// to `Error`, because the conditions that would look like an outage (Deepgram
/// offline / rejected key) requeue without failing per ADR 0048.
fn lane_health(
    feature: DebugFeature,
    label: &str,
    pipeline: &[::app_infra::ProcessorPipelineStatus],
    processor: &str,
) -> FeatureHealthDto {
    let Some(lane) = pipeline.iter().find(|lane| lane.processor == processor) else {
        return health(feature, DebugSeverity::Ok, format!("{label} has no jobs queued."));
    };

    if lane.failed > 0 {
        let detail = lane
            .last_error
            .as_deref()
            .map(|error| format!(" Last error: {}", error_summary(error)))
            .unwrap_or_default();
        return health(
            feature,
            DebugSeverity::Warn,
            format!(
                "{} {} job{} failed and need reprocessing.{detail}",
                lane.failed,
                label.to_lowercase(),
                if lane.failed == 1 { "" } else { "s" }
            ),
        );
    }

    if lane.queued > 0 || lane.running > 0 {
        return health(
            feature,
            DebugSeverity::Ok,
            format!(
                "{label} is working through {} queued, {} running.",
                lane.queued, lane.running
            ),
        );
    }

    health(
        feature,
        DebugSeverity::Ok,
        format!("{label} is up to date ({} done).", lane.completed),
    )
}

fn capture_health(inputs: &DebugHealthInputs) -> FeatureHealthDto {
    // Liveness, not failure: the session is alive and resumes when disk frees up
    // (same family as display-unavailable, ADR 0021). Warn, never error.
    if inputs.capture_low_disk_suspended {
        return health(
            DebugFeature::Capture,
            DebugSeverity::Warn,
            "Recording is suspended because the disk is low. It resumes when space frees up.",
        );
    }
    if inputs.capture_user_paused {
        return health(DebugFeature::Capture, DebugSeverity::Ok, "Paused by you.");
    }
    if inputs.capture_inactivity_paused {
        return health(
            DebugFeature::Capture,
            DebugSeverity::Ok,
            "Paused because you have been inactive.",
        );
    }
    if inputs.capture_running {
        return health(DebugFeature::Capture, DebugSeverity::Ok, "Recording.");
    }
    health(DebugFeature::Capture, DebugSeverity::Ok, "Not recording.")
}

fn privacy_health(inputs: &DebugHealthInputs) -> FeatureHealthDto {
    if inputs.privacy_excluded_app_count > 0 && !inputs.privacy_filter_applied {
        return health(
            DebugFeature::Privacy,
            DebugSeverity::Warn,
            format!(
                "{} app{} should be excluded, but the capture filter is not applied right now.",
                inputs.privacy_excluded_app_count,
                if inputs.privacy_excluded_app_count == 1 { "" } else { "s" }
            ),
        );
    }
    if inputs.privacy_excluded_app_count > 0 {
        return health(
            DebugFeature::Privacy,
            DebugSeverity::Ok,
            format!(
                "{} app{} excluded from capture.",
                inputs.privacy_excluded_app_count,
                if inputs.privacy_excluded_app_count == 1 { "" } else { "s" }
            ),
        );
    }
    health(
        DebugFeature::Privacy,
        DebugSeverity::Ok,
        "No apps are excluded from capture.",
    )
}

/// Embeddings is the one lane that can hard-`Error`: a model that will not load
/// means semantic search is silently degraded to keyword-only, and nothing
/// retries it back into health on its own.
fn embeddings_health(semantic: &SemanticIndexStatusDto) -> FeatureHealthDto {
    if let Some(error) = semantic.last_load_error.as_deref() {
        return health(
            DebugFeature::Embeddings,
            DebugSeverity::Error,
            format!(
                "The embedding model failed to load, so search is keyword-only. {}",
                error_summary(error)
            ),
        );
    }
    if semantic.consecutive_load_failures.unwrap_or(0) > 0 {
        return health(
            DebugFeature::Embeddings,
            DebugSeverity::Error,
            "The embedding model failed to load, so search is keyword-only.",
        );
    }
    if semantic.quarantined_count.unwrap_or(0) > 0 {
        return health(
            DebugFeature::Embeddings,
            DebugSeverity::Warn,
            format!(
                "{} item{} could not be embedded and are quarantined since app start.",
                semantic.quarantined_count.unwrap_or(0),
                if semantic.quarantined_count == Some(1) { "" } else { "s" }
            ),
        );
    }
    if semantic.backlog_count > 0 {
        return health(
            DebugFeature::Embeddings,
            DebugSeverity::Ok,
            format!(
                "Indexing: {} item{} still to embed ({} indexed).",
                semantic.backlog_count,
                if semantic.backlog_count == 1 { "" } else { "s" },
                semantic.vector_count
            ),
        );
    }
    if semantic.vector_count == 0 {
        return health(
            DebugFeature::Embeddings,
            DebugSeverity::Ok,
            "Nothing is indexed for semantic search yet.",
        );
    }
    health(
        DebugFeature::Embeddings,
        DebugSeverity::Ok,
        format!("{} items indexed, nothing waiting.", semantic.vector_count),
    )
}

fn ai_runtime_health(inputs: &DebugHealthInputs) -> FeatureHealthDto {
    if !inputs.ai_enabled {
        return health(
            DebugFeature::AiRuntime,
            DebugSeverity::Ok,
            "AI features are turned off.",
        );
    }
    if !inputs.ai_has_default_model {
        return health(
            DebugFeature::AiRuntime,
            DebugSeverity::Warn,
            "AI features are on, but no default model is selected.",
        );
    }
    health(DebugFeature::AiRuntime, DebugSeverity::Ok, "Ready.")
}

fn user_context_health(inputs: &DebugHealthInputs) -> FeatureHealthDto {
    if !inputs.user_context_enabled {
        return health(
            DebugFeature::UserContext,
            DebugSeverity::Ok,
            "User Context is turned off.",
        );
    }
    if !inputs.ai_enabled || !inputs.ai_has_default_model {
        return health(
            DebugFeature::UserContext,
            DebugSeverity::Warn,
            "User Context is on, but it has no usable AI model to think with.",
        );
    }
    health(DebugFeature::UserContext, DebugSeverity::Ok, "Deriving.")
}

fn jobs_and_storage_health(pipeline: &[::app_infra::ProcessorPipelineStatus]) -> FeatureHealthDto {
    let failed: i64 = pipeline.iter().map(|lane| lane.failed).sum();
    let queued: i64 = pipeline.iter().map(|lane| lane.queued).sum();
    let running: i64 = pipeline.iter().map(|lane| lane.running).sum();

    if failed > 0 {
        return health(
            DebugFeature::JobsAndStorage,
            DebugSeverity::Warn,
            format!(
                "{failed} job{} failed across the queue.",
                if failed == 1 { "" } else { "s" }
            ),
        );
    }
    health(
        DebugFeature::JobsAndStorage,
        DebugSeverity::Ok,
        format!("{queued} queued, {running} running, nothing failed."),
    )
}

/// The whole rollup as a pure function, so the severity rules are testable
/// without a database. Order is the dock's order.
pub fn roll_up(inputs: &DebugHealthInputs) -> Vec<FeatureHealthDto> {
    vec![
        capture_health(inputs),
        privacy_health(inputs),
        lane_health(
            DebugFeature::Ocr,
            "OCR",
            &inputs.pipeline,
            ::app_infra::OCR_PROCESSOR,
        ),
        lane_health(
            DebugFeature::Transcription,
            "Transcription",
            &inputs.pipeline,
            ::app_infra::AUDIO_TRANSCRIPTION_PROCESSOR,
        ),
        lane_health(
            DebugFeature::Diarization,
            "Diarization",
            &inputs.pipeline,
            ::app_infra::SPEAKER_ANALYSIS_PROCESSOR,
        ),
        embeddings_health(&inputs.semantic),
        ai_runtime_health(inputs),
        user_context_health(inputs),
        jobs_and_storage_health(&inputs.pipeline),
    ]
}

#[tauri::command]
pub async fn get_debug_health(
    infra: tauri::State<'_, AppInfraState>,
    worker_health: tauri::State<'_, SemanticWorkerHealthState>,
    capture_state: tauri::State<'_, NativeCaptureState>,
    metadata_state: tauri::State<'_, CaptureMetadataState>,
    settings_state: tauri::State<'_, RecordingSettingsState>,
) -> Result<Vec<FeatureHealthDto>, String> {
    // Gather the in-memory reads first and in their own scope: these are `std`
    // mutexes, whose guards must not be held across the awaits below.
    let inputs_from_state = {
        let session = capture_state
            .lock()
            .expect("native capture state poisoned")
            .session();
        let privacy = capture_privacy_debug_info(metadata_state.inner());
        let settings = read_recording_settings(settings_state.inner());
        (session, privacy, settings)
    };
    let (session, privacy, settings) = inputs_from_state;

    let pipeline = infra
        .processing_pipeline_status()
        .await
        .map_err(|error| format!("failed to read processing pipeline status: {error}"))?;
    let semantic = get_semantic_index_status(infra.clone(), worker_health).await?;

    Ok(roll_up(&DebugHealthInputs {
        pipeline,
        semantic,
        capture_running: session.is_running,
        capture_inactivity_paused: session.is_inactivity_paused,
        capture_user_paused: session.is_user_paused,
        capture_low_disk_suspended: session.is_low_disk_suspended,
        privacy_excluded_app_count: privacy.currently_excluded_bundle_ids.len(),
        privacy_filter_applied: privacy.privacy_filter_applied,
        ai_enabled: settings.ai_runtime.enabled,
        ai_has_default_model: settings
            .ai_runtime
            .default_model
            .as_ref()
            .is_some_and(|model| !model.model.trim().is_empty()),
        user_context_enabled: settings.user_context.enabled,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn semantic() -> SemanticIndexStatusDto {
        SemanticIndexStatusDto {
            vector_count: 10,
            backlog_count: 0,
            live_dimension: Some(768),
            // The slice-3 worker snapshot is not wired yet: unknown, not broken.
            model_loaded: None,
            consecutive_load_failures: None,
            quarantined_count: None,
            last_load_error: None,
        }
    }

    fn lane(processor: &str) -> ::app_infra::ProcessorPipelineStatus {
        ::app_infra::ProcessorPipelineStatus {
            processor: processor.to_string(),
            queued: 0,
            running: 0,
            completed: 5,
            failed: 0,
            failed_last_24h: 0,
            average_completed_seconds_last_24h: Some(1.0),
            last_error: None,
        }
    }

    /// A healthy, fully-configured, recording app.
    fn healthy() -> DebugHealthInputs {
        DebugHealthInputs {
            pipeline: vec![
                lane(::app_infra::OCR_PROCESSOR),
                lane(::app_infra::AUDIO_TRANSCRIPTION_PROCESSOR),
                lane(::app_infra::SPEAKER_ANALYSIS_PROCESSOR),
            ],
            semantic: semantic(),
            capture_running: true,
            capture_inactivity_paused: false,
            capture_user_paused: false,
            capture_low_disk_suspended: false,
            privacy_excluded_app_count: 0,
            privacy_filter_applied: false,
            ai_enabled: true,
            ai_has_default_model: true,
            user_context_enabled: true,
        }
    }

    fn severity_of(report: &[FeatureHealthDto], feature: DebugFeature) -> DebugSeverity {
        report
            .iter()
            .find(|entry| entry.feature == feature)
            .unwrap_or_else(|| panic!("{feature:?} should be in the rollup"))
            .severity
    }

    #[test]
    fn a_healthy_app_is_ok_everywhere() {
        let report = roll_up(&healthy());
        assert_eq!(report.len(), 9);
        assert!(report.iter().all(|entry| entry.severity == DebugSeverity::Ok));
    }

    #[test]
    fn failed_jobs_warn_but_never_error() {
        let mut inputs = healthy();
        inputs.pipeline[1].failed = 3;
        inputs.pipeline[1].failed_last_24h = 3;
        inputs.pipeline[1].last_error = Some("deepgram rejected the audio\nstack trace".to_string());

        let report = roll_up(&inputs);
        assert_eq!(
            severity_of(&report, DebugFeature::Transcription),
            DebugSeverity::Warn
        );
        // The queue rollup sees the same failures.
        assert_eq!(
            severity_of(&report, DebugFeature::JobsAndStorage),
            DebugSeverity::Warn
        );
        // Only the first line of the error reaches the tooltip.
        let transcription = report
            .iter()
            .find(|entry| entry.feature == DebugFeature::Transcription)
            .expect("transcription entry");
        assert!(transcription.reason.contains("3 transcription jobs failed"));
        assert!(transcription.reason.contains("deepgram rejected the audio"));
        assert!(!transcription.reason.contains("stack trace"));
    }

    #[test]
    fn an_absent_processor_lane_is_idle_not_broken() {
        // A fresh install: the GROUP BY returns no rows at all.
        let mut inputs = healthy();
        inputs.pipeline = Vec::new();

        let report = roll_up(&inputs);
        for feature in [
            DebugFeature::Ocr,
            DebugFeature::Transcription,
            DebugFeature::Diarization,
            DebugFeature::JobsAndStorage,
        ] {
            assert_eq!(severity_of(&report, feature), DebugSeverity::Ok);
        }
    }

    #[test]
    fn a_queued_backlog_is_ok() {
        // Deepgram offline requeues without failing (ADR 0048): work waits, it
        // does not break.
        let mut inputs = healthy();
        inputs.pipeline[1].queued = 400;

        let report = roll_up(&inputs);
        assert_eq!(
            severity_of(&report, DebugFeature::Transcription),
            DebugSeverity::Ok
        );
    }

    #[test]
    fn an_embedding_model_load_failure_is_an_error() {
        let mut inputs = healthy();
        inputs.semantic.last_load_error = Some("onnx session init failed".to_string());
        inputs.semantic.consecutive_load_failures = Some(4);
        inputs.semantic.model_loaded = Some(false);

        let report = roll_up(&inputs);
        assert_eq!(
            severity_of(&report, DebugFeature::Embeddings),
            DebugSeverity::Error
        );
    }

    #[test]
    fn repeated_load_failures_error_even_without_an_error_string() {
        let mut inputs = healthy();
        inputs.semantic.consecutive_load_failures = Some(2);

        assert_eq!(
            severity_of(&roll_up(&inputs), DebugFeature::Embeddings),
            DebugSeverity::Error
        );
    }

    #[test]
    fn unknown_worker_health_is_never_an_error() {
        // All four snapshot fields `None` (slice 3 not wired / no sweep yet).
        let mut inputs = healthy();
        inputs.semantic.backlog_count = 120;
        inputs.semantic.vector_count = 0;
        inputs.semantic.live_dimension = None;

        assert_eq!(
            severity_of(&roll_up(&inputs), DebugFeature::Embeddings),
            DebugSeverity::Ok
        );
    }

    #[test]
    fn quarantined_items_warn() {
        let mut inputs = healthy();
        inputs.semantic.quarantined_count = Some(3);

        assert_eq!(
            severity_of(&roll_up(&inputs), DebugFeature::Embeddings),
            DebugSeverity::Warn
        );
    }

    #[test]
    fn a_transient_liveness_suspension_warns_never_errors() {
        // Low disk suspended capture: the session is alive and self-recovers, so
        // it is a warning, not a failure (same family as ADR 0021's
        // display-unavailable).
        let mut inputs = healthy();
        inputs.capture_low_disk_suspended = true;

        let report = roll_up(&inputs);
        assert_eq!(severity_of(&report, DebugFeature::Capture), DebugSeverity::Warn);
    }

    #[test]
    fn paused_capture_is_ok() {
        let mut inputs = healthy();
        inputs.capture_running = false;
        inputs.capture_user_paused = true;
        assert_eq!(
            severity_of(&roll_up(&inputs), DebugFeature::Capture),
            DebugSeverity::Ok
        );

        let mut inputs = healthy();
        inputs.capture_inactivity_paused = true;
        assert_eq!(
            severity_of(&roll_up(&inputs), DebugFeature::Capture),
            DebugSeverity::Ok
        );
    }

    #[test]
    fn configured_exclusions_that_are_not_applied_warn() {
        let mut inputs = healthy();
        inputs.privacy_excluded_app_count = 2;
        inputs.privacy_filter_applied = false;
        assert_eq!(
            severity_of(&roll_up(&inputs), DebugFeature::Privacy),
            DebugSeverity::Warn
        );

        inputs.privacy_filter_applied = true;
        assert_eq!(
            severity_of(&roll_up(&inputs), DebugFeature::Privacy),
            DebugSeverity::Ok
        );
    }

    #[test]
    fn ai_on_without_a_model_warns_and_drags_user_context_with_it() {
        let mut inputs = healthy();
        inputs.ai_has_default_model = false;

        let report = roll_up(&inputs);
        assert_eq!(severity_of(&report, DebugFeature::AiRuntime), DebugSeverity::Warn);
        assert_eq!(
            severity_of(&report, DebugFeature::UserContext),
            DebugSeverity::Warn
        );
    }

    #[test]
    fn features_turned_off_are_ok_not_broken() {
        let mut inputs = healthy();
        inputs.ai_enabled = false;
        inputs.ai_has_default_model = false;
        inputs.user_context_enabled = false;

        let report = roll_up(&inputs);
        assert_eq!(severity_of(&report, DebugFeature::AiRuntime), DebugSeverity::Ok);
        assert_eq!(severity_of(&report, DebugFeature::UserContext), DebugSeverity::Ok);
    }

    #[test]
    fn severity_orders_worst_last_so_a_dock_can_max_over_it() {
        assert!(DebugSeverity::Error > DebugSeverity::Warn);
        assert!(DebugSeverity::Warn > DebugSeverity::Ok);
    }

    #[test]
    fn the_wire_shape_is_camel_case() {
        let json = serde_json::to_value(health(
            DebugFeature::JobsAndStorage,
            DebugSeverity::Warn,
            "x",
        ))
        .expect("health should serialize");
        assert_eq!(json["feature"], "jobsAndStorage");
        assert_eq!(json["severity"], "warn");
        assert_eq!(json["reason"], "x");
    }
}
