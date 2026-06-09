mod ai_provider_key_store;
mod audio_segments;
pub mod brokered_access;
mod capture_index_key_store;
mod capture_retention;
mod captured_frame_equivalence;
mod captured_frame_pipeline;
mod db;
pub mod error;
mod frame_batch_artifact_cleanup;
mod frame_batch_runtime;
mod frame_batch_store;
mod hidden_segment_workspace;
pub mod jobs;
mod ocr_budget;
pub mod processing;
mod search;
pub mod status;
pub mod user_context;

use std::time::{SystemTime, UNIX_EPOCH};
use std::{collections::BTreeSet, path::Path, sync::Arc};

use sqlx::SqlitePool;

pub use ai_provider_key_store::{
    delete_ai_provider_key, has_ai_provider_key, load_ai_provider_key, store_ai_provider_key,
};
pub use audio_segments::{
    AudioSegment, AudioSegmentSourceKind, AudioSegmentStore, NewAudioSegment,
};
pub use capture_retention::{
    delete_capture_artifact_path_if_safe, CaptureRetentionStore, CaptureSegment, CaptureSourceKind,
    NewCaptureSegment, NewCaptureSession, RetentionCleanupContext, RetentionCleanupMode,
    RetentionCleanupSummary, RetentionPolicy, ScreenCaptureSegmentWindow,
};
pub use captured_frame_equivalence::{
    CapturedFrameEquivalenceResolver, CapturedFrameEquivalenceScope,
};
pub use captured_frame_pipeline::{
    CapturedFramePipeline, CapturedFramePipelineResult, CapturedFrameReprocessingOutcome,
    CapturedFrameReprocessingResult, ClosedFrameBatchSummary,
};
pub use error::{AppInfraError, Result};
pub use frame_batch_runtime::FrameBatchRuntime;
pub use frame_batch_store::{
    FrameBatch, FrameBatchFinalizePayload, FrameBatchFinalizeResult, FrameBatchStatus,
    FrameBatchStore, FrameBatchWindow, SegmentWorkspaceBatchReference,
    FRAME_BATCH_DURATION_MINUTES, FRAME_BATCH_FINALIZE_JOB_KIND,
};
pub use hidden_segment_workspace::{
    HiddenSegmentWorkspacePaths, HiddenSegmentWorkspaceRepairContext,
    HiddenSegmentWorkspaceRepairResult, SegmentWorkspaceCleanupDebugInfo,
    SegmentWorkspaceCleanupDisposition,
};
pub use jobs::{
    default_worker_thread_count, BackgroundJob, BackgroundJobStatus, CpuJobHandle, CpuJobResult,
    CpuJobSuccess, DebugCpuJobRequest, JobCounts, JobDescriptor, JobRuntime, JobStore,
};
pub use ocr::{
    AppleVisionProvider, FrozenOcrPayload, OcrBoundingBox, OcrObservation, OcrOutput, OcrProvider,
    OcrProviderKind, OcrRecognitionMode, OcrRequest, OcrStructuredPayload, PaddleOcrProvider,
    TesseractProvider,
};
pub use ocr_budget::{
    OcrAdmissionDecision, OcrAdmissionOutcome, OcrAdmissionReason, OcrAdmissionSignals,
};
pub use processing::{
    AudioTranscriptionJobPayload, AudioTranscriptionProcessorBackend, FocusedFrameWindow, Frame,
    FrameEquivalence, FrameEquivalenceStatus, FrameProcessingJob, FrameSummary, NewFrame,
    OcrProcessorBackend, PersonProfile, ProcessingJob, ProcessingJobCompletion, ProcessingJobDraft,
    ProcessingJobReclamationSummary, ProcessingJobRunOutcome, ProcessingJobStatus,
    ProcessingModelCleanupLock, ProcessingResult, ProcessingResultDraft, ProcessingRuntime,
    ProcessingStore, ProcessingSubject, ProcessorBackend, ProcessorRegistry,
    SegmentWorkspaceOcrReference, SpeakerAnalysisJobPayload, SpeakerAnalysisProcessorBackend,
    SpeakerClusterView, SpeakerTurnView, SystemAudioSpeechActivityJobPayload,
    SystemAudioSpeechActivityProcessorBackend, SystemAudioSpeechActivityResult,
    AUDIO_SEGMENT_SUBJECT_TYPE, AUDIO_TRANSCRIPTION_PROCESSOR, FRAME_SUBJECT_TYPE,
    HELPER_TIMEOUT_SECONDS_OPTION, OCR_PROCESSOR, SPEAKER_ANALYSIS_PAYLOAD_OPTION_KEY,
    SPEAKER_ANALYSIS_PROCESSOR, SYSTEM_AUDIO_SPEECH_ACTIVITY_PROCESSOR,
};
pub use search::{
    AudioSearchResult, FrameSearchResult, SearchAppRefinement, SearchAppRefinementKind,
    SearchCaptureRefinements, SearchCaptureRequest, SearchCaptureResponse, SearchDateRangeOrigin,
    SearchDateRangeRefinement, SearchParseError, SearchStore, SearchableApp,
};
pub use status::AppInfraStatus;
pub use user_context::{
    evidence_fingerprint, CaptureWindow, CaptureWindowItem, NewActivity, NewActivityEvidence,
    NewConclusion, NewConclusionEvidence, NewDerivationRun, UserContextCascadeSummary,
    UserContextStore,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioSegmentTranscriptionAdmission {
    pub enabled: bool,
    pub provider_available: bool,
    pub payload_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemAudioSpeechActivityAdmission {
    pub enabled: bool,
    pub detector_available: bool,
    pub payload_json: Option<String>,
}

impl SystemAudioSpeechActivityAdmission {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            detector_available: false,
            payload_json: None,
        }
    }

    pub fn unavailable() -> Self {
        Self {
            enabled: true,
            detector_available: false,
            payload_json: None,
        }
    }

    pub fn available(payload_json: impl Into<String>) -> Self {
        Self {
            enabled: true,
            detector_available: true,
            payload_json: Some(payload_json.into()),
        }
    }

    fn should_enqueue_for(&self, segment: &NewAudioSegment) -> bool {
        self.enabled
            && self.detector_available
            && self.payload_json.is_some()
            && segment.source_kind == AudioSegmentSourceKind::SystemAudio
    }
}

impl AudioSegmentTranscriptionAdmission {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            provider_available: false,
            payload_json: None,
        }
    }

    pub fn unavailable() -> Self {
        Self {
            enabled: true,
            provider_available: false,
            payload_json: None,
        }
    }

    pub fn available(payload_json: impl Into<String>) -> Self {
        Self {
            enabled: true,
            provider_available: true,
            payload_json: Some(payload_json.into()),
        }
    }

    fn should_enqueue_for(&self, segment: &NewAudioSegment) -> bool {
        self.enabled
            && self.provider_available
            && self.payload_json.is_some()
            && segment.source_kind == AudioSegmentSourceKind::Microphone
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioSegmentTranscriptionAdmissionOutcome {
    pub segment: AudioSegment,
    pub job: Option<ProcessingJob>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioSegmentSpeakerAnalysisAdmission {
    pub enabled: bool,
    pub provider_available: bool,
    pub payload_json: Option<String>,
}

impl AudioSegmentSpeakerAnalysisAdmission {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            provider_available: false,
            payload_json: None,
        }
    }

    pub fn unavailable() -> Self {
        Self {
            enabled: true,
            provider_available: false,
            payload_json: None,
        }
    }

    pub fn available(payload_json: impl Into<String>) -> Self {
        Self {
            enabled: true,
            provider_available: true,
            payload_json: Some(payload_json.into()),
        }
    }

    fn should_enqueue_for(&self, segment: &NewAudioSegment) -> bool {
        self.enabled
            && self.provider_available
            && self.payload_json.is_some()
            && segment.source_kind == AudioSegmentSourceKind::Microphone
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioSegmentProcessingAdmissionOutcome {
    pub segment: AudioSegment,
    pub transcription_job: Option<ProcessingJob>,
    pub speaker_analysis_job: Option<ProcessingJob>,
    pub system_audio_speech_activity_job: Option<ProcessingJob>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AudioSegmentTranscriptionReprocessingOutcome {
    Created,
    Ignored,
    Requeued,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioSegmentTranscriptionReprocessingResult {
    pub outcome: AudioSegmentTranscriptionReprocessingOutcome,
    pub job: ProcessingJob,
}

pub type AudioSegmentSpeakerAnalysisReprocessingOutcome =
    AudioSegmentTranscriptionReprocessingOutcome;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioSegmentSpeakerAnalysisReprocessingResult {
    pub outcome: AudioSegmentSpeakerAnalysisReprocessingOutcome,
    pub job: ProcessingJob,
}

pub type SystemAudioSpeechActivityReprocessingOutcome =
    AudioSegmentTranscriptionReprocessingOutcome;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemAudioSpeechActivityReprocessingResult {
    pub outcome: SystemAudioSpeechActivityReprocessingOutcome,
    pub job: ProcessingJob,
}

#[derive(Clone)]
pub struct AppInfra {
    database: db::Database,
    jobs: JobStore,
    audio_segments: AudioSegmentStore,
    frame_batches: FrameBatchStore,
    capture_retention: CaptureRetentionStore,
    processing: ProcessingStore,
    search: SearchStore,
    user_context: UserContextStore,
    captured_frame_equivalence: CapturedFrameEquivalenceResolver,
    captured_frame_pipeline: CapturedFramePipeline,
    runtime: JobRuntime,
    frame_batch_runtime: FrameBatchRuntime,
    processing_runtime: ProcessingRuntime,
}

impl AppInfra {
    pub async fn initialize<P: AsRef<Path>>(base_dir: P) -> Result<Self> {
        Self::initialize_with_processing_registry(base_dir, default_processing_registry()).await
    }

    pub async fn initialize_with_processing_registry<P: AsRef<Path>>(
        base_dir: P,
        processing_registry: ProcessorRegistry,
    ) -> Result<Self> {
        let infra =
            Self::initialize_fast_with_processing_registry(base_dir, processing_registry).await?;
        infra.run_startup_maintenance().await?;
        Ok(infra)
    }

    /// Read-only initialization for brokered-access consumers (the `mnema` CLI and
    /// the in-app Ask AI agent).
    ///
    /// Opens the database and builds every store but, unlike [`Self::initialize`],
    /// does **not** run [`Self::run_startup_maintenance`]. Brokered access only
    /// issues read queries (search / timeline / show-text / opaque-reference
    /// authorization) and never spawns processing workers, so it must not run the
    /// startup maintenance scans — in particular orphaned-job reconciliation, which
    /// is only safe while nothing is executing those jobs (see ADR 0020). Running
    /// it against a database whose live owner (the desktop app) has workers
    /// actively processing would requeue legitimately-`running` jobs and cause
    /// duplicate processing.
    pub async fn initialize_read_only<P: AsRef<Path>>(base_dir: P) -> Result<Self> {
        Self::initialize_fast_with_processing_registry(base_dir, default_processing_registry()).await
    }

    /// Fast initialization path: opens the database and constructs every store
    /// and runtime, but does **not** run the startup maintenance scans
    /// (frame-equivalence / search-projection backfills, orphaned-job
    /// reconciliation, frame-batch reconciliation).
    ///
    /// This exists so the desktop app can open its window without blocking on
    /// the maintenance passes, which scan the whole index and dominate startup
    /// time. Callers that take this path **must** run [`Self::run_startup_maintenance`]
    /// before spawning processing workers (see ADR 0020): orphaned-job
    /// reconciliation is only safe while nothing is executing those jobs.
    pub async fn initialize_fast_with_processing_registry<P: AsRef<Path>>(
        base_dir: P,
        processing_registry: ProcessorRegistry,
    ) -> Result<Self> {
        let database = db::Database::initialize(base_dir.as_ref()).await?;
        let jobs = JobStore::new(database.pool().clone());
        let audio_segments = AudioSegmentStore::new(database.pool().clone());
        let frame_batches = FrameBatchStore::new(database.pool().clone());
        let capture_retention = CaptureRetentionStore::new(database.pool().clone());
        let processing = ProcessingStore::new(database.pool().clone());
        let search = SearchStore::new(database.pool().clone());
        let user_context = UserContextStore::new(database.pool().clone());
        let captured_frame_equivalence = CapturedFrameEquivalenceResolver::new(processing.clone());
        let captured_frame_pipeline =
            CapturedFramePipeline::new(processing.clone(), frame_batches.clone());
        let runtime = JobRuntime::new(default_worker_thread_count())?;
        let frame_batch_runtime = FrameBatchRuntime::new(frame_batches.clone());
        let processing_runtime = ProcessingRuntime::new(processing.clone(), processing_registry);

        Ok(Self {
            database,
            jobs,
            audio_segments,
            frame_batches,
            capture_retention,
            processing,
            search,
            user_context,
            captured_frame_equivalence,
            captured_frame_pipeline,
            runtime,
            frame_batch_runtime,
            processing_runtime,
        })
    }

    /// Runs the one-time startup maintenance passes that repair/reconcile the
    /// index: clearing stale model-cleanup locks, backfilling frame-equivalence
    /// and search projections, reconciling orphaned `running` jobs (ADR 0020),
    /// and reconciling frame batches left without finalize jobs / active capture.
    ///
    /// These passes are scans over the whole index; in steady state they find
    /// nothing to do (the normal write paths keep everything current) but still
    /// cost real time, so the desktop app runs this off the window-open path.
    /// It must complete before processing workers are spawned.
    pub async fn run_startup_maintenance(&self) -> Result<()> {
        // Stale-only: this runs on the deferred-startup thread while model-deletion commands can be
        // live and holding a freshly acquired cleanup lock, so only clear locks old enough to be
        // orphaned by a prior crash (see `MODEL_CLEANUP_LOCK_STALE_AFTER_SECONDS`).
        self.processing
            .clear_stale_model_cleanup_locks(processing::MODEL_CLEANUP_LOCK_STALE_AFTER_SECONDS)
            .await?;
        self.processing.backfill_frame_equivalence().await?;
        self.search.backfill_missing_projections().await?;
        self.jobs.reconcile_orphaned_running_jobs().await?;
        let reclamation = self.processing.reconcile_orphaned_running_jobs().await?;
        if reclamation.requeued > 0 || reclamation.failed_on_ceiling > 0 {
            capture_runtime::debug_log!(
                "[app-infra] startup reclaimed orphaned processing jobs (requeued={}, failed_on_ceiling={})",
                reclamation.requeued,
                reclamation.failed_on_ceiling
            );
        }
        self.frame_batches
            .reconcile_closed_batches_without_finalize_jobs()
            .await?;
        self.frame_batches
            .reconcile_open_batches_without_active_capture()
            .await?;
        Ok(())
    }

    pub fn pool(&self) -> &SqlitePool {
        self.database.pool()
    }

    pub fn base_dir(&self) -> &Path {
        self.database.base_dir()
    }

    #[cfg(test)]
    pub(crate) fn jobs(&self) -> &JobStore {
        &self.jobs
    }

    #[cfg(test)]
    pub(crate) fn processing(&self) -> &ProcessingStore {
        &self.processing
    }

    #[cfg(test)]
    pub(crate) fn frame_batches(&self) -> &FrameBatchStore {
        &self.frame_batches
    }

    pub fn capture_retention(&self) -> &CaptureRetentionStore {
        &self.capture_retention
    }

    pub fn user_context(&self) -> &user_context::UserContextStore {
        &self.user_context
    }

    pub async fn frame_secret_redaction_count(&self, frame_id: i64) -> Result<u32> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS count \
             FROM secret_redactions \
             WHERE anchor_type = 'frame' \
               AND (frame_id = ?1 \
                    OR processing_result_id IN (\
                        SELECT processing_result_id \
                        FROM search_documents \
                        WHERE frame_id = ?1 \
                          AND text_source_kind = 'equivalent_reuse' \
                          AND processing_result_id IS NOT NULL\
                    ))",
        )
        .bind(frame_id)
        .fetch_one(self.pool())
        .await?;
        let count: i64 = sqlx::Row::get(&row, "count");
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }

    pub async fn audio_segment_secret_redaction_count(&self, audio_segment_id: i64) -> Result<u32> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS count \
             FROM secret_redactions \
             WHERE anchor_type = 'audio' AND audio_segment_id = ?1",
        )
        .bind(audio_segment_id)
        .fetch_one(self.pool())
        .await?;
        let count: i64 = sqlx::Row::get(&row, "count");
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }

    pub async fn capture_session_id_for_source_session(
        &self,
        source_session_id: &str,
    ) -> Result<Option<String>> {
        let row = sqlx::query(
            "SELECT capture_session_id FROM capture_sessions \
             WHERE screen_source_session_id = ?1 \
                OR microphone_source_session_id = ?1 \
                OR system_audio_source_session_id = ?1 \
             ORDER BY id DESC LIMIT 1",
        )
        .bind(source_session_id)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(|row| sqlx::Row::get(&row, "capture_session_id")))
    }

    #[cfg(test)]
    pub(crate) fn captured_frame_pipeline(&self) -> &CapturedFramePipeline {
        &self.captured_frame_pipeline
    }

    pub async fn enqueue_job(
        &self,
        descriptor: &JobDescriptor,
        payload_json: Option<&str>,
    ) -> Result<BackgroundJob> {
        self.jobs.enqueue(descriptor, payload_json).await
    }

    pub async fn list_jobs(&self) -> Result<Vec<BackgroundJob>> {
        self.jobs.list(None).await
    }

    pub async fn get_job(&self, job_id: i64) -> Result<Option<BackgroundJob>> {
        self.jobs.get(job_id).await
    }

    pub async fn insert_frame(&self, frame: &NewFrame) -> Result<Frame> {
        self.processing.insert_frame(frame).await
    }

    pub async fn debug_insert_frame_and_enqueue_processing_job(
        &self,
        frame: &NewFrame,
        processor: &str,
        payload_json: Option<&str>,
    ) -> Result<FrameProcessingJob> {
        self.captured_frame_pipeline
            .debug_insert_frame_and_enqueue_processor_job(frame, processor, payload_json)
            .await
    }

    pub async fn debug_insert_frame_and_enqueue_ocr_job(
        &self,
        frame: &NewFrame,
        payload_json: Option<&str>,
    ) -> Result<FrameProcessingJob> {
        self.debug_insert_frame_and_enqueue_processing_job(frame, OCR_PROCESSOR, payload_json)
            .await
    }

    pub async fn capture_frame(
        &self,
        frame: &NewFrame,
        payload_json: Option<&str>,
    ) -> Result<CapturedFramePipelineResult> {
        self.captured_frame_pipeline
            .capture_frame(frame, payload_json)
            .await
    }

    pub async fn capture_frame_with_ocr_admission(
        &self,
        frame: &NewFrame,
        payload_json: Option<&str>,
        decision: OcrAdmissionDecision,
    ) -> Result<CapturedFramePipelineResult> {
        self.captured_frame_pipeline
            .capture_frame_with_ocr_admission(frame, payload_json, decision)
            .await
    }

    pub async fn capture_frame_skipping_ocr_with_reason(
        &self,
        frame: &NewFrame,
        decision: OcrAdmissionDecision,
    ) -> Result<CapturedFramePipelineResult> {
        self.captured_frame_pipeline
            .capture_frame_skipping_ocr_with_reason(frame, decision)
            .await
    }

    pub async fn capture_frame_without_ocr(
        &self,
        frame: &NewFrame,
    ) -> Result<CapturedFramePipelineResult> {
        self.captured_frame_pipeline
            .capture_frame_without_ocr(frame)
            .await
    }

    pub async fn reprocess_captured_frame_ocr(
        &self,
        frame_id: i64,
        payload_json: Option<&str>,
    ) -> Result<CapturedFrameReprocessingResult> {
        self.captured_frame_pipeline
            .reprocess_captured_frame_ocr(frame_id, payload_json)
            .await
    }

    pub async fn list_frames(
        &self,
        session_id: Option<&str>,
        before_id: Option<i64>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<Frame>> {
        self.processing
            .list_frames(session_id, before_id, limit, offset)
            .await
    }

    pub async fn list_frame_summaries_in_range(
        &self,
        captured_at_start: &str,
        captured_at_end: &str,
    ) -> Result<Vec<FrameSummary>> {
        self.processing
            .list_frame_summaries_in_range(captured_at_start, captured_at_end)
            .await
    }

    pub async fn get_timeline_window_around_frame(
        &self,
        frame_id: i64,
        newer_limit: u32,
        older_limit: u32,
    ) -> Result<FocusedFrameWindow> {
        self.processing
            .get_timeline_window_around_frame(frame_id, newer_limit, older_limit)
            .await
    }

    pub async fn get_latest_frame_in_range(
        &self,
        captured_at_start: &str,
        captured_at_end: &str,
    ) -> Result<Option<Frame>> {
        self.processing
            .get_latest_frame_in_range(captured_at_start, captured_at_end)
            .await
    }

    pub async fn list_frame_batches(&self, session_id: Option<&str>) -> Result<Vec<FrameBatch>> {
        self.frame_batches.list_batches(session_id).await
    }

    pub async fn get_frame_batch(&self, batch_id: i64) -> Result<Option<FrameBatch>> {
        self.frame_batches.get(batch_id).await
    }

    pub async fn list_frames_for_batch(&self, batch_id: i64) -> Result<Vec<Frame>> {
        self.frame_batches.list_frames_for_batch(batch_id).await
    }

    pub async fn close_and_schedule_all_frame_batches_for_session(
        &self,
        session_id: &str,
    ) -> Result<Vec<FrameBatch>> {
        self.frame_batches
            .close_and_schedule_all_batches_for_session(session_id)
            .await
    }

    pub async fn get_frame(&self, frame_id: i64) -> Result<Option<Frame>> {
        self.processing.get_frame(frame_id).await
    }

    pub async fn get_nearest_earlier_equivalent_frame(
        &self,
        frame_id: i64,
    ) -> Result<Option<Frame>> {
        self.captured_frame_equivalence
            .get_frame_and_find_nearest_earlier_equivalent_frame_in_default_scope(frame_id)
            .await
    }

    pub async fn get_earliest_earlier_equivalent_frame(
        &self,
        frame_id: i64,
    ) -> Result<Option<Frame>> {
        self.captured_frame_equivalence
            .get_frame_and_find_earliest_earlier_equivalent_frame_in_default_scope(frame_id)
            .await
    }

    pub async fn list_frames_for_segment_workspace(
        &self,
        session_id: &str,
        workspace_prefix: &str,
    ) -> Result<Vec<Frame>> {
        self.processing
            .list_frames_for_segment_workspace(session_id, workspace_prefix)
            .await
    }

    pub async fn list_finalized_screen_segments_overlapping_window(
        &self,
        start_at: &str,
        end_at: &str,
    ) -> Result<Vec<ScreenCaptureSegmentWindow>> {
        self.capture_retention
            .list_finalized_screen_segments_overlapping_window(start_at, end_at)
            .await
    }

    pub async fn classify_hidden_segment_workspace(
        &self,
        workspace_dir: &Path,
    ) -> Result<Option<SegmentWorkspaceCleanupDebugInfo>> {
        self.frame_batches
            .classify_hidden_segment_workspace(workspace_dir)
            .await
    }

    pub async fn repair_hidden_segment_workspaces(
        &self,
        recordings_root: &Path,
    ) -> Result<HiddenSegmentWorkspaceRepairResult> {
        self.repair_hidden_segment_workspaces_with_context(
            recordings_root,
            &HiddenSegmentWorkspaceRepairContext::default(),
        )
        .await
    }

    pub async fn repair_hidden_segment_workspaces_with_context(
        &self,
        recordings_root: &Path,
        context: &HiddenSegmentWorkspaceRepairContext,
    ) -> Result<HiddenSegmentWorkspaceRepairResult> {
        self.frame_batches
            .repair_hidden_segment_workspaces_with_context(recordings_root, context)
            .await
    }

    pub async fn upsert_audio_segment(&self, segment: &NewAudioSegment) -> Result<AudioSegment> {
        self.audio_segments.upsert(segment).await
    }

    pub async fn upsert_audio_segment_and_maybe_enqueue_transcription(
        &self,
        segment: &NewAudioSegment,
        admission: &AudioSegmentTranscriptionAdmission,
    ) -> Result<AudioSegmentTranscriptionAdmissionOutcome> {
        let outcome = self
            .upsert_audio_segment_and_maybe_enqueue_processing(
                segment,
                admission,
                &AudioSegmentSpeakerAnalysisAdmission::disabled(),
                &SystemAudioSpeechActivityAdmission::disabled(),
            )
            .await?;
        return Ok(AudioSegmentTranscriptionAdmissionOutcome {
            segment: outcome.segment,
            job: outcome.transcription_job,
        });
    }

    pub async fn upsert_audio_segment_and_maybe_enqueue_processing(
        &self,
        segment: &NewAudioSegment,
        transcription_admission: &AudioSegmentTranscriptionAdmission,
        speaker_admission: &AudioSegmentSpeakerAnalysisAdmission,
        system_audio_speech_admission: &SystemAudioSpeechActivityAdmission,
    ) -> Result<AudioSegmentProcessingAdmissionOutcome> {
        let should_enqueue_transcription = transcription_admission.should_enqueue_for(segment);
        let should_enqueue_speaker = speaker_admission.should_enqueue_for(segment);
        let should_enqueue_system_audio_speech =
            system_audio_speech_admission.should_enqueue_for(segment);
        let mut transaction = self.pool().begin().await?;
        let mut segment = self
            .audio_segments
            .upsert_in_transaction(&mut transaction, segment)
            .await?;
        let transcription_job = if should_enqueue_transcription {
            let subject = ProcessingSubject::audio_segment(segment.id);
            let existing = self
                .processing
                .get_latest_processing_job_for_subject_and_processor_in_transaction(
                    &mut transaction,
                    &subject,
                    AUDIO_TRANSCRIPTION_PROCESSOR,
                )
                .await?;
            match existing {
                Some(job) => Some(job),
                None => Some(
                    self.processing
                        .enqueue_job_in_transaction(
                            &mut transaction,
                            &subject,
                            AUDIO_TRANSCRIPTION_PROCESSOR,
                            transcription_admission.payload_json.as_deref(),
                        )
                        .await?,
                ),
            }
        } else {
            None
        };
        let speaker_analysis_job = if should_enqueue_speaker {
            let subject = ProcessingSubject::audio_segment(segment.id);
            let existing = self
                .processing
                .get_latest_processing_job_for_subject_and_processor_in_transaction(
                    &mut transaction,
                    &subject,
                    SPEAKER_ANALYSIS_PROCESSOR,
                )
                .await?;
            match existing {
                Some(job) => Some(job),
                None => Some(
                    self.processing
                        .enqueue_job_in_transaction(
                            &mut transaction,
                            &subject,
                            SPEAKER_ANALYSIS_PROCESSOR,
                            speaker_admission.payload_json.as_deref(),
                        )
                        .await?,
                ),
            }
        } else {
            None
        };
        let system_audio_speech_activity_job = if should_enqueue_system_audio_speech {
            let subject = ProcessingSubject::audio_segment(segment.id);
            let existing = self
                .processing
                .get_latest_processing_job_for_subject_and_processor_in_transaction(
                    &mut transaction,
                    &subject,
                    SYSTEM_AUDIO_SPEECH_ACTIVITY_PROCESSOR,
                )
                .await?;
            match existing {
                Some(job) => Some(job),
                None => Some(
                    self.processing
                        .enqueue_job_in_transaction(
                            &mut transaction,
                            &subject,
                            SYSTEM_AUDIO_SPEECH_ACTIVITY_PROCESSOR,
                            system_audio_speech_admission.payload_json.as_deref(),
                        )
                        .await?,
                ),
            }
        } else {
            None
        };
        transaction.commit().await?;

        if segment.capture_segment_id.is_none() {
            if let Some(capture_segment_id) = self
                .ensure_capture_segment_for_audio_segment(&segment)
                .await?
            {
                segment.capture_segment_id = Some(capture_segment_id);
            }
        }

        Ok(AudioSegmentProcessingAdmissionOutcome {
            segment,
            transcription_job,
            speaker_analysis_job,
            system_audio_speech_activity_job,
        })
    }

    async fn ensure_capture_segment_for_audio_segment(
        &self,
        segment: &AudioSegment,
    ) -> Result<Option<i64>> {
        let source_column = match segment.source_kind {
            AudioSegmentSourceKind::Microphone => "microphone_source_session_id",
            AudioSegmentSourceKind::SystemAudio => "system_audio_source_session_id",
        };
        let query = format!(
            "SELECT capture_session_id FROM capture_sessions WHERE {source_column} = ?1 ORDER BY id DESC LIMIT 1"
        );
        let Some(row) = sqlx::query(&query)
            .bind(&segment.source_session_id)
            .fetch_optional(self.pool())
            .await?
        else {
            return Ok(None);
        };
        let capture_session_id: String = sqlx::Row::get(&row, "capture_session_id");
        let source_kind = match segment.source_kind {
            AudioSegmentSourceKind::Microphone => CaptureSourceKind::Microphone,
            AudioSegmentSourceKind::SystemAudio => CaptureSourceKind::SystemAudio,
        };
        let capture_segment = self
            .capture_retention
            .upsert_capture_segment(&NewCaptureSegment {
                capture_session_id,
                source_kind,
                source_session_id: segment.source_session_id.clone(),
                segment_index: segment.segment_index,
                media_file_path: Some(segment.file_path.clone()),
                workspace_dir_path: None,
                frame_dir_path: None,
                sidecar_file_path: None,
                started_at: segment.started_at.clone(),
                ended_at: segment.ended_at.clone(),
                status: "completed".to_string(),
            })
            .await?;
        sqlx::query("UPDATE audio_segments SET capture_segment_id = ?2, updated_at = CURRENT_TIMESTAMP WHERE id = ?1")
            .bind(segment.id)
            .bind(capture_segment.id)
            .execute(self.pool())
            .await?;
        Ok(Some(capture_segment.id))
    }

    pub async fn backfill_missing_audio_transcription_jobs(
        &self,
        admission: &AudioSegmentTranscriptionAdmission,
    ) -> Result<u64> {
        if !admission.enabled || !admission.provider_available || admission.payload_json.is_none() {
            return Ok(0);
        }

        let mut transaction = self.pool().begin().await?;
        let segments = self
            .audio_segments
            .list_microphone_without_audio_transcription_job_in_transaction(&mut transaction)
            .await?;
        let mut enqueued = 0_u64;
        for segment in segments {
            if !audio_segment_file_exists(&segment) {
                continue;
            }
            let subject = ProcessingSubject::audio_segment(segment.id);
            if self
                .processing
                .get_latest_processing_job_for_subject_and_processor_in_transaction(
                    &mut transaction,
                    &subject,
                    AUDIO_TRANSCRIPTION_PROCESSOR,
                )
                .await?
                .is_none()
            {
                self.processing
                    .enqueue_job_in_transaction(
                        &mut transaction,
                        &subject,
                        AUDIO_TRANSCRIPTION_PROCESSOR,
                        admission.payload_json.as_deref(),
                    )
                    .await?;
                enqueued = enqueued.saturating_add(1);
            }
        }
        transaction.commit().await?;
        Ok(enqueued)
    }

    pub async fn backfill_missing_speaker_analysis_jobs(
        &self,
        admission: &AudioSegmentSpeakerAnalysisAdmission,
    ) -> Result<u64> {
        if !admission.enabled || !admission.provider_available || admission.payload_json.is_none() {
            return Ok(0);
        }

        let mut transaction = self.pool().begin().await?;
        let segments = self
            .audio_segments
            .list_microphone_without_speaker_analysis_job_in_transaction(&mut transaction)
            .await?;
        let mut enqueued = 0_u64;
        for segment in segments {
            if !audio_segment_file_exists(&segment) {
                continue;
            }
            let subject = ProcessingSubject::audio_segment(segment.id);
            if self
                .processing
                .get_latest_processing_job_for_subject_and_processor_in_transaction(
                    &mut transaction,
                    &subject,
                    SPEAKER_ANALYSIS_PROCESSOR,
                )
                .await?
                .is_none()
            {
                self.processing
                    .enqueue_job_in_transaction(
                        &mut transaction,
                        &subject,
                        SPEAKER_ANALYSIS_PROCESSOR,
                        admission.payload_json.as_deref(),
                    )
                    .await?;
                enqueued = enqueued.saturating_add(1);
            }
        }
        transaction.commit().await?;
        Ok(enqueued)
    }

    pub async fn reprocess_audio_segment_transcription(
        &self,
        audio_segment_id: i64,
        admission: &AudioSegmentTranscriptionAdmission,
    ) -> Result<AudioSegmentTranscriptionReprocessingResult> {
        let segment = self
            .audio_segments
            .get(audio_segment_id)
            .await?
            .ok_or(AppInfraError::AudioSegmentNotFound(audio_segment_id))?;

        if segment.source_kind != AudioSegmentSourceKind::Microphone {
            return Err(AppInfraError::AudioTranscriptionEngine(
                "only microphone segments can be transcribed".to_string(),
            ));
        }

        let payload_json = if !admission.enabled {
            return Err(AppInfraError::AudioTranscriptionEngine(
                "audio transcription is disabled".to_string(),
            ));
        } else if !admission.provider_available {
            return Err(AppInfraError::AudioTranscriptionEngine(
                "selected audio transcription model is unavailable".to_string(),
            ));
        } else {
            admission.payload_json.as_deref().ok_or_else(|| {
                AppInfraError::AudioTranscriptionEngine(
                    "audio transcription job payload is unavailable".to_string(),
                )
            })?
        };

        let mut transaction = self.pool().begin().await?;
        let subject = ProcessingSubject::audio_segment(segment.id);
        let existing_job = self
            .processing
            .get_latest_processing_job_for_subject_and_processor_in_transaction(
                &mut transaction,
                &subject,
                AUDIO_TRANSCRIPTION_PROCESSOR,
            )
            .await?;

        let result = match existing_job {
            None => {
                let job = self
                    .processing
                    .enqueue_job_in_transaction(
                        &mut transaction,
                        &subject,
                        AUDIO_TRANSCRIPTION_PROCESSOR,
                        Some(payload_json),
                    )
                    .await?;
                AudioSegmentTranscriptionReprocessingResult {
                    outcome: AudioSegmentTranscriptionReprocessingOutcome::Created,
                    job,
                }
            }
            Some(job) if job.status == ProcessingJobStatus::Queued => {
                AudioSegmentTranscriptionReprocessingResult {
                    outcome: AudioSegmentTranscriptionReprocessingOutcome::Ignored,
                    job,
                }
            }
            Some(job) if job.status == ProcessingJobStatus::Running => {
                return Err(AppInfraError::ProcessingJobInvalidTransition {
                    job_id: job.id,
                    from: job.status.as_str().to_string(),
                    to: ProcessingJobStatus::Queued.as_str().to_string(),
                });
            }
            Some(job) => {
                let job = self
                    .processing
                    .requeue_processing_job_in_transaction(
                        &mut transaction,
                        job.id,
                        Some(payload_json),
                    )
                    .await?;
                AudioSegmentTranscriptionReprocessingResult {
                    outcome: AudioSegmentTranscriptionReprocessingOutcome::Requeued,
                    job,
                }
            }
        };

        transaction.commit().await?;

        Ok(result)
    }

    pub async fn reprocess_audio_segment_speaker_analysis(
        &self,
        audio_segment_id: i64,
        admission: &AudioSegmentSpeakerAnalysisAdmission,
    ) -> Result<AudioSegmentSpeakerAnalysisReprocessingResult> {
        let segment = self
            .audio_segments
            .get(audio_segment_id)
            .await?
            .ok_or(AppInfraError::AudioSegmentNotFound(audio_segment_id))?;

        if segment.source_kind != AudioSegmentSourceKind::Microphone {
            return Err(AppInfraError::SpeakerAnalysisEngine(
                "only microphone segments can be speaker-analyzed".to_string(),
            ));
        }

        let payload_json = if !admission.enabled {
            return Err(AppInfraError::SpeakerAnalysisEngine(
                "speaker analysis is disabled".to_string(),
            ));
        } else if !admission.provider_available {
            return Err(AppInfraError::SpeakerAnalysisEngine(
                "selected speaker analysis model is unavailable".to_string(),
            ));
        } else {
            admission.payload_json.as_deref().ok_or_else(|| {
                AppInfraError::SpeakerAnalysisEngine(
                    "speaker analysis job payload is unavailable".to_string(),
                )
            })?
        };

        let mut transaction = self.pool().begin().await?;
        let subject = ProcessingSubject::audio_segment(segment.id);
        let existing_job = self
            .processing
            .get_latest_processing_job_for_subject_and_processor_in_transaction(
                &mut transaction,
                &subject,
                SPEAKER_ANALYSIS_PROCESSOR,
            )
            .await?;

        let result = match existing_job {
            None => {
                let job = self
                    .processing
                    .enqueue_job_in_transaction(
                        &mut transaction,
                        &subject,
                        SPEAKER_ANALYSIS_PROCESSOR,
                        Some(payload_json),
                    )
                    .await?;
                AudioSegmentSpeakerAnalysisReprocessingResult {
                    outcome: AudioSegmentTranscriptionReprocessingOutcome::Created,
                    job,
                }
            }
            Some(job) if job.status == ProcessingJobStatus::Queued => {
                AudioSegmentSpeakerAnalysisReprocessingResult {
                    outcome: AudioSegmentTranscriptionReprocessingOutcome::Ignored,
                    job,
                }
            }
            Some(job) if job.status == ProcessingJobStatus::Running => {
                return Err(AppInfraError::ProcessingJobInvalidTransition {
                    job_id: job.id,
                    from: job.status.as_str().to_string(),
                    to: ProcessingJobStatus::Queued.as_str().to_string(),
                });
            }
            Some(job) => {
                let job = self
                    .processing
                    .requeue_processing_job_in_transaction(
                        &mut transaction,
                        job.id,
                        Some(payload_json),
                    )
                    .await?;
                AudioSegmentSpeakerAnalysisReprocessingResult {
                    outcome: AudioSegmentTranscriptionReprocessingOutcome::Requeued,
                    job,
                }
            }
        };

        transaction.commit().await?;
        Ok(result)
    }

    pub async fn reprocess_system_audio_speech_activity(
        &self,
        audio_segment_id: i64,
        admission: &SystemAudioSpeechActivityAdmission,
    ) -> Result<SystemAudioSpeechActivityReprocessingResult> {
        let segment = self
            .audio_segments
            .get(audio_segment_id)
            .await?
            .ok_or(AppInfraError::AudioSegmentNotFound(audio_segment_id))?;

        if segment.source_kind != AudioSegmentSourceKind::SystemAudio {
            return Err(AppInfraError::AudioTranscriptionEngine(
                "only system-audio segments can run speech-gated transcription".to_string(),
            ));
        }

        let payload_json = if !admission.enabled {
            return Err(AppInfraError::AudioTranscriptionEngine(
                "system-audio transcription is disabled".to_string(),
            ));
        } else if !admission.detector_available {
            return Err(AppInfraError::AudioTranscriptionEngine(
                "selected speech detector is unavailable".to_string(),
            ));
        } else {
            admission.payload_json.as_deref().ok_or_else(|| {
                AppInfraError::AudioTranscriptionEngine(
                    "system-audio speech activity payload is unavailable".to_string(),
                )
            })?
        };

        let mut transaction = self.pool().begin().await?;
        let subject = ProcessingSubject::audio_segment(segment.id);
        let existing_job = self
            .processing
            .get_latest_processing_job_for_subject_and_processor_in_transaction(
                &mut transaction,
                &subject,
                SYSTEM_AUDIO_SPEECH_ACTIVITY_PROCESSOR,
            )
            .await?;

        let result = match existing_job {
            None => {
                let job = self
                    .processing
                    .enqueue_job_in_transaction(
                        &mut transaction,
                        &subject,
                        SYSTEM_AUDIO_SPEECH_ACTIVITY_PROCESSOR,
                        Some(payload_json),
                    )
                    .await?;
                SystemAudioSpeechActivityReprocessingResult {
                    outcome: SystemAudioSpeechActivityReprocessingOutcome::Created,
                    job,
                }
            }
            Some(job) if job.status == ProcessingJobStatus::Queued => {
                SystemAudioSpeechActivityReprocessingResult {
                    outcome: SystemAudioSpeechActivityReprocessingOutcome::Ignored,
                    job,
                }
            }
            Some(job) if job.status == ProcessingJobStatus::Running => {
                return Err(AppInfraError::ProcessingJobInvalidTransition {
                    job_id: job.id,
                    from: job.status.as_str().to_string(),
                    to: ProcessingJobStatus::Queued.as_str().to_string(),
                });
            }
            Some(job) => {
                let job = self
                    .processing
                    .requeue_processing_job_in_transaction(
                        &mut transaction,
                        job.id,
                        Some(payload_json),
                    )
                    .await?;
                SystemAudioSpeechActivityReprocessingResult {
                    outcome: SystemAudioSpeechActivityReprocessingOutcome::Requeued,
                    job,
                }
            }
        };

        transaction.commit().await?;

        Ok(result)
    }

    pub async fn get_audio_segment(&self, audio_segment_id: i64) -> Result<Option<AudioSegment>> {
        self.audio_segments.get(audio_segment_id).await
    }

    pub async fn list_audio_segments_overlapping_range(
        &self,
        range_start: &str,
        range_end: &str,
        source_kind: Option<AudioSegmentSourceKind>,
        source_session_id: Option<&str>,
    ) -> Result<Vec<AudioSegment>> {
        self.audio_segments
            .list_overlapping_range(range_start, range_end, source_kind, source_session_id)
            .await
    }

    pub async fn enqueue_processing_job(
        &self,
        draft: &ProcessingJobDraft,
    ) -> Result<ProcessingJob> {
        self.processing.enqueue_job(draft).await
    }

    pub async fn get_processing_job(&self, job_id: i64) -> Result<Option<ProcessingJob>> {
        self.processing.get_job(job_id).await
    }

    pub async fn list_processing_jobs_for_subject(
        &self,
        subject: &ProcessingSubject,
    ) -> Result<Vec<ProcessingJob>> {
        self.processing.list_jobs_for_subject(subject).await
    }

    pub async fn list_running_ocr_model_keys(&self) -> Result<BTreeSet<String>> {
        let jobs = self
            .processing
            .list_running_jobs_for_processor(OCR_PROCESSOR)
            .await?;
        let mut keys = BTreeSet::new();

        for job in jobs {
            if let Some(key) = ocr_model_key_for_job(&job)? {
                keys.insert(key);
            }
        }

        Ok(keys)
    }

    pub async fn fail_queued_ocr_jobs_because_disabled(&self) -> Result<u64> {
        self.processing
            .mark_queued_jobs_failed_for_processor(OCR_PROCESSOR, "OCR is disabled")
            .await
    }

    /// **Processing Job Reclamation**: requeue any **Orphaned Processing Job** left `running` so it
    /// re-runs and still produces its result. Call at graceful shutdown *after* background workers
    /// have been aborted and awaited (nothing executing), mirroring the startup reclamation pass.
    pub async fn reconcile_orphaned_processing_jobs(
        &self,
    ) -> Result<ProcessingJobReclamationSummary> {
        self.processing.reconcile_orphaned_running_jobs().await
    }

    pub async fn acquire_ocr_model_cleanup_locks(
        &self,
        model_keys: &BTreeSet<String>,
    ) -> Result<ProcessingModelCleanupLock> {
        self.acquire_processing_model_cleanup_locks(OCR_PROCESSOR, model_keys)
            .await
    }

    pub async fn list_running_audio_transcription_model_keys(&self) -> Result<BTreeSet<String>> {
        let jobs = self
            .processing
            .list_running_jobs_for_processor(AUDIO_TRANSCRIPTION_PROCESSOR)
            .await?;
        let mut keys = BTreeSet::new();

        for job in jobs {
            if let Some(key) = audio_transcription_model_key_for_job(&job)? {
                keys.insert(key);
            }
        }

        Ok(keys)
    }

    pub async fn acquire_audio_transcription_model_cleanup_locks(
        &self,
        model_keys: &BTreeSet<String>,
    ) -> Result<ProcessingModelCleanupLock> {
        self.acquire_processing_model_cleanup_locks(AUDIO_TRANSCRIPTION_PROCESSOR, model_keys)
            .await
    }

    pub async fn list_running_speaker_analysis_model_keys(&self) -> Result<BTreeSet<String>> {
        let jobs = self
            .processing
            .list_running_jobs_for_processor(SPEAKER_ANALYSIS_PROCESSOR)
            .await?;
        let mut keys = BTreeSet::new();

        for job in jobs {
            if let Some(key) = speaker_analysis_model_key_for_job(&job)? {
                keys.insert(key);
            }
        }

        Ok(keys)
    }

    pub async fn list_active_speaker_analysis_model_keys(&self) -> Result<BTreeSet<String>> {
        let running_jobs = self
            .processing
            .list_running_jobs_for_processor(SPEAKER_ANALYSIS_PROCESSOR)
            .await?;
        let pending_jobs = self
            .processing
            .list_retargetable_jobs_for_processor(SPEAKER_ANALYSIS_PROCESSOR)
            .await?;
        let mut keys = BTreeSet::new();

        for job in running_jobs.into_iter().chain(
            pending_jobs
                .into_iter()
                .filter(|job| matches!(job.status, ProcessingJobStatus::Queued)),
        ) {
            if let Some(key) = speaker_analysis_model_key_for_job(&job)? {
                keys.insert(key);
            }
        }

        Ok(keys)
    }

    pub async fn acquire_speaker_analysis_model_cleanup_locks(
        &self,
        model_keys: &BTreeSet<String>,
    ) -> Result<ProcessingModelCleanupLock> {
        self.acquire_processing_model_cleanup_locks(SPEAKER_ANALYSIS_PROCESSOR, model_keys)
            .await
    }

    pub async fn retarget_ocr_jobs_referencing_model_keys(
        &self,
        model_keys: &BTreeSet<String>,
        provider: &str,
        model_id: Option<&str>,
    ) -> Result<u64> {
        let jobs = self
            .processing
            .list_retargetable_jobs_for_processor(OCR_PROCESSOR)
            .await?;
        let mut updated = 0_u64;

        for job in jobs {
            let Some(key) = ocr_model_key_for_job(&job)? else {
                continue;
            };
            if !model_keys.contains(&key) {
                continue;
            }
            let mut payload = FrozenOcrPayload::from_payload_json(job.payload_json.as_deref())
                .map_err(|error| AppInfraError::OcrEngine(error.to_string()))?;
            payload.provider = provider.to_string();
            payload.model_id = model_id.map(str::to_string);
            let payload_json = serde_json::to_string(&payload)?;
            if self
                .processing
                .update_retargetable_job_payload(job.id, &payload_json)
                .await?
                .is_some()
            {
                updated = updated.saturating_add(1);
            }
        }

        Ok(updated)
    }

    pub async fn retarget_audio_transcription_jobs_referencing_model_keys(
        &self,
        model_keys: &BTreeSet<String>,
        provider: &str,
        model_id: Option<&str>,
    ) -> Result<u64> {
        let jobs = self
            .processing
            .list_retargetable_jobs_for_processor(AUDIO_TRANSCRIPTION_PROCESSOR)
            .await?;
        let mut updated = 0_u64;

        for job in jobs {
            let Some(key) = audio_transcription_model_key_for_job(&job)? else {
                continue;
            };
            if !model_keys.contains(&key) {
                continue;
            }
            let Some(payload_json) = job.payload_json.as_deref() else {
                continue;
            };
            let mut payload: AudioTranscriptionJobPayload = serde_json::from_str(payload_json)?;
            payload.provider = provider.to_string();
            payload.model_id = model_id.map(str::to_string);
            let payload_json = serde_json::to_string(&payload)?;
            if self
                .processing
                .update_retargetable_job_payload(job.id, &payload_json)
                .await?
                .is_some()
            {
                updated = updated.saturating_add(1);
            }
        }

        Ok(updated)
    }

    pub async fn release_processing_model_cleanup_locks(
        &self,
        lock: &ProcessingModelCleanupLock,
    ) -> Result<u64> {
        self.processing.release_model_cleanup_locks(lock).await
    }

    pub async fn claim_queued_processing_job(&self, job_id: i64) -> Result<Option<ProcessingJob>> {
        self.processing.claim_queued_job(job_id).await
    }

    pub async fn mark_processing_job_running(&self, job_id: i64) -> Result<ProcessingJob> {
        self.processing.mark_job_running(job_id).await
    }

    pub async fn mark_processing_job_failed(
        &self,
        job_id: i64,
        error_text: Option<&str>,
    ) -> Result<ProcessingJob> {
        self.processing.mark_job_failed(job_id, error_text).await
    }

    pub async fn complete_processing_job(
        &self,
        job_id: i64,
        result: &ProcessingResultDraft,
    ) -> Result<ProcessingJobCompletion> {
        self.processing.complete_job(job_id, result).await
    }

    pub async fn search_capture(
        &self,
        request: SearchCaptureRequest,
    ) -> Result<SearchCaptureResponse> {
        self.search.search_capture(request).await
    }

    pub async fn list_searchable_apps(&self) -> Result<Vec<SearchableApp>> {
        self.search.list_searchable_apps().await
    }

    pub async fn get_processing_result_for_job(
        &self,
        job_id: i64,
    ) -> Result<Option<ProcessingResult>> {
        self.processing.get_result_for_job(job_id).await
    }

    pub async fn list_processing_results_for_subject(
        &self,
        subject: &ProcessingSubject,
    ) -> Result<Vec<ProcessingResult>> {
        self.processing.list_results_for_subject(subject).await
    }

    pub async fn list_speaker_turns_for_audio_segment(
        &self,
        audio_segment_id: i64,
    ) -> Result<Vec<SpeakerTurnView>> {
        self.processing
            .list_speaker_turns_for_audio_segment(audio_segment_id)
            .await
    }

    pub async fn list_person_profiles(&self) -> Result<Vec<PersonProfile>> {
        self.processing.list_person_profiles().await
    }

    pub async fn create_person_profile(
        &self,
        display_name: &str,
        notes: Option<&str>,
    ) -> Result<PersonProfile> {
        self.processing
            .create_person_profile(display_name, notes)
            .await
    }

    pub async fn delete_person_profile(&self, person_id: i64) -> Result<()> {
        self.processing.delete_person_profile(person_id).await
    }

    pub async fn list_speaker_clusters_for_session(
        &self,
        session_id: &str,
    ) -> Result<Vec<SpeakerClusterView>> {
        self.processing
            .list_speaker_clusters_for_session(session_id)
            .await
    }

    pub async fn name_speaker_cluster(
        &self,
        cluster_id: i64,
        label: &str,
    ) -> Result<SpeakerClusterView> {
        self.processing
            .name_speaker_cluster(cluster_id, label)
            .await
    }

    pub async fn link_speaker_cluster_to_person(
        &self,
        cluster_id: i64,
        person_id: i64,
        add_embedding: bool,
    ) -> Result<SpeakerClusterView> {
        self.processing
            .link_speaker_cluster_to_person(cluster_id, person_id, add_embedding)
            .await
    }

    pub async fn unlink_speaker_cluster_from_person(
        &self,
        cluster_id: i64,
    ) -> Result<SpeakerClusterView> {
        self.processing
            .unlink_speaker_cluster_from_person(cluster_id)
            .await
    }

    pub async fn confirm_speaker_recognition_suggestion(
        &self,
        cluster_id: i64,
        add_embedding: bool,
    ) -> Result<SpeakerClusterView> {
        self.processing
            .confirm_speaker_recognition_suggestion(cluster_id, add_embedding)
            .await
    }

    pub async fn reject_speaker_recognition_suggestion(
        &self,
        cluster_id: i64,
    ) -> Result<SpeakerClusterView> {
        self.processing
            .reject_speaker_recognition_suggestion(cluster_id)
            .await
    }

    pub async fn merge_speaker_clusters(
        &self,
        source_cluster_id: i64,
        target_cluster_id: i64,
    ) -> Result<SpeakerClusterView> {
        self.processing
            .merge_speaker_clusters(source_cluster_id, target_cluster_id)
            .await
    }

    pub async fn move_speaker_turn_to_cluster(
        &self,
        turn_id: i64,
        target_cluster_id: i64,
    ) -> Result<SpeakerTurnView> {
        self.processing
            .move_speaker_turn_to_cluster(turn_id, target_cluster_id)
            .await
    }

    pub async fn process_processing_job(&self, job_id: i64) -> Result<ProcessingJobRunOutcome> {
        self.processing_runtime.process_job(job_id).await
    }

    pub async fn process_next_processing_job(&self) -> Result<Option<ProcessingJobRunOutcome>> {
        self.processing_runtime.process_next_queued_job().await
    }

    pub async fn process_next_processing_job_for_processor(
        &self,
        processor: &str,
    ) -> Result<Option<ProcessingJobRunOutcome>> {
        self.processing_runtime
            .process_next_queued_job_for_processor(processor)
            .await
    }

    pub async fn process_next_processing_job_excluding_processor(
        &self,
        excluded_processor: &str,
    ) -> Result<Option<ProcessingJobRunOutcome>> {
        self.process_next_processing_job_excluding_processors(&[excluded_processor])
            .await
    }

    pub async fn process_next_processing_job_excluding_processors(
        &self,
        excluded_processors: &[&str],
    ) -> Result<Option<ProcessingJobRunOutcome>> {
        self.processing_runtime
            .process_next_queued_job_excluding_processors(excluded_processors)
            .await
    }

    pub async fn count_queued_or_running_processing_jobs_for_processor(
        &self,
        processor: &str,
    ) -> Result<i64> {
        self.processing
            .count_queued_or_running_jobs_for_processor(processor)
            .await
    }

    pub async fn latest_frame_context_differs(
        &self,
        frame: &NewFrame,
        workspace_prefix: Option<&str>,
    ) -> Result<bool> {
        self.processing
            .latest_frame_context_differs(frame, workspace_prefix)
            .await
    }

    pub async fn process_next_frame_batch_job(&self) -> Result<Option<FrameBatchFinalizeResult>> {
        self.frame_batch_runtime.process_next_queued_job().await
    }

    pub async fn submit_debug_cpu_job(&self, request: DebugCpuJobRequest) -> Result<BackgroundJob> {
        let request = request.normalized();
        let payload_json = serde_json::to_string(&request)?;
        let task_request = request.clone();
        let handle = self
            .spawn_cpu_job(
                JobDescriptor::new(jobs::DEBUG_CPU_JOB_KIND),
                Some(&payload_json),
                move || {
                    let result_text = task_request.simulated_result_text();
                    Ok(CpuJobSuccess::new(result_text.clone()).with_result_text(result_text))
                },
            )
            .await?;

        self.get_job(handle.job_id())
            .await?
            .ok_or(AppInfraError::JobNotFound(handle.job_id()))
    }

    pub async fn spawn_cpu_job<F, T>(
        &self,
        descriptor: JobDescriptor,
        payload_json: Option<&str>,
        task: F,
    ) -> Result<CpuJobHandle<T>>
    where
        F: FnOnce() -> CpuJobResult<T> + Send + 'static,
        T: Send + 'static,
    {
        let job = self.jobs.enqueue(&descriptor, payload_json).await?;
        self.runtime.spawn_cpu(self.jobs.clone(), job, task)
    }

    pub async fn status(&self) -> Result<AppInfraStatus> {
        Ok(AppInfraStatus {
            database_path: self.database.database_path().display().to_string(),
            migrations_ran: self.database.migrations_ran(),
            worker_thread_count: self.runtime.worker_thread_count(),
            job_counts: self.jobs.counts().await?,
        })
    }
}

impl AppInfra {
    async fn acquire_processing_model_cleanup_locks(
        &self,
        processor: &str,
        model_keys: &BTreeSet<String>,
    ) -> Result<ProcessingModelCleanupLock> {
        let lock_token = processing_model_cleanup_lock_token(processor);
        self.processing
            .acquire_model_cleanup_locks(processor, model_keys, &lock_token)
            .await
    }
}

fn default_processing_registry() -> ProcessorRegistry {
    ProcessorRegistry::new()
        .register(OcrProcessorBackend::from_provider_arcs([
            Arc::new(AppleVisionProvider::new()) as Arc<dyn ocr::OcrProvider>,
            Arc::new(TesseractProvider::with_models_dir(std::env::temp_dir())),
            Arc::new(PaddleOcrProvider::with_models_dir(std::env::temp_dir())),
        ]))
        .register(AudioTranscriptionProcessorBackend::from_provider_arcs([
            Arc::new(audio_transcription::providers::LocalWhisperProvider)
                as Arc<dyn audio_transcription::TranscriptionProvider>,
            Arc::new(audio_transcription::providers::AppleSpeechOnDeviceProvider),
            Arc::new(audio_transcription::providers::ParakeetProvider),
        ]))
        .register(SystemAudioSpeechActivityProcessorBackend)
}

fn model_key(provider: &str, model_id: &str) -> String {
    format!("{provider}/{model_id}")
}

fn processing_model_cleanup_lock_token(processor: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("model-cleanup:{processor}:{}:{nanos}", std::process::id())
}

fn audio_segment_file_exists(segment: &AudioSegment) -> bool {
    Path::new(&segment.file_path).is_file()
}

fn ocr_model_key_for_job(job: &ProcessingJob) -> Result<Option<String>> {
    let payload = FrozenOcrPayload::from_payload_json(job.payload_json.as_deref())
        .map_err(|error| AppInfraError::OcrEngine(error.to_string()))?;
    Ok(payload
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|model_id| !model_id.is_empty())
        .map(|model_id| model_key(&payload.provider, model_id)))
}

fn audio_transcription_model_key_for_job(job: &ProcessingJob) -> Result<Option<String>> {
    let Some(payload_json) = job.payload_json.as_deref() else {
        return Ok(None);
    };
    let payload: AudioTranscriptionJobPayload = serde_json::from_str(payload_json)?;
    Ok(payload
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|model_id| !model_id.is_empty())
        .map(|model_id| model_key(&payload.provider, model_id)))
}

fn speaker_analysis_model_key_for_job(job: &ProcessingJob) -> Result<Option<String>> {
    let Some(payload_json) = job.payload_json.as_deref() else {
        return Ok(None);
    };
    let payload: SpeakerAnalysisJobPayload = serde_json::from_str(payload_json)?;
    Ok(payload
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|model_id| !model_id.is_empty())
        .map(|model_id| model_key(&payload.provider, model_id)))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::{mpsc, Arc, Mutex},
        time::{SystemTime, UNIX_EPOCH},
    };

    use async_trait::async_trait;

    use super::*;
    use crate::{db::Database, jobs::ORPHANED_RUNNING_JOB_ERROR};

    const TEST_PROCESSOR: &str = "mock-recovery";
    const RECLAIMED_ORPHANED_PROCESSING_JOB_MESSAGE: &str =
        "processing job was requeued by reclamation after the app shut down while it was running";

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("app-infra-{label}-{unique}"));

            fs::create_dir_all(&path).expect("test directory should be created");

            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn write_existing_audio_placeholder(path: &Path) -> String {
        fs::write(path, b"placeholder audio").expect("placeholder audio file should exist");
        path.to_string_lossy().to_string()
    }

    fn run_async_test(test: impl std::future::Future<Output = ()>) {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(test);
    }

    fn build_test_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
    }

    fn test_frame(session_id: &str, file_name: &str) -> NewFrame {
        NewFrame::new(
            session_id,
            format!("/tmp/{file_name}"),
            "2026-04-12T10:00:00Z",
        )
        .with_dimensions(1920, 1080)
    }

    fn test_frame_at(session_id: &str, file_name: &str, captured_at: &str) -> NewFrame {
        NewFrame::new(session_id, format!("/tmp/{file_name}"), captured_at)
            .with_dimensions(1920, 1080)
    }

    fn ready_equivalence(hint: &str, proof_fill: u8) -> FrameEquivalence {
        FrameEquivalence {
            hint: Some(hint.to_string()),
            proof: Some(vec![proof_fill; 1024]),
            version: Some(1),
            status: Some(FrameEquivalenceStatus::Ready),
            error: None,
        }
    }

    fn novelty_admit_decision() -> OcrAdmissionDecision {
        let mut decision =
            OcrAdmissionDecision::admit(OcrAdmissionReason::AdmittedVisualNovelty, 0, true);
        decision.signals = OcrAdmissionSignals {
            low_queue_pressure: true,
            fingerprint_novel_in_scope: true,
            novelty_admission_available: true,
            ..Default::default()
        };
        decision
    }

    async fn search_representative_frame_ids(infra: &AppInfra, query: &str) -> Vec<i64> {
        infra
            .search_capture(SearchCaptureRequest {
                query: query.to_string(),
                frame_limit: Some(10),
                frame_offset: None,
                audio_limit: Some(0),
                audio_offset: None,
                snapshot_document_id: None,
                refinements: None,
            })
            .await
            .expect("search should run")
            .frames
            .into_iter()
            .map(|frame| frame.representative_frame.id)
            .collect()
    }

    async fn search_document_kind(infra: &AppInfra, frame_id: i64) -> Option<String> {
        sqlx::query_scalar(
            "SELECT text_source_kind FROM search_documents WHERE frame_id = ?1 LIMIT 1",
        )
        .bind(frame_id)
        .fetch_optional(infra.pool())
        .await
        .expect("search document lookup should run")
    }

    #[test]
    fn frame_secret_redaction_count_includes_equivalent_reuse_source_result() {
        run_async_test(async {
            let dir = TestDir::new("frame-secret-redaction-count-reuse");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let equivalence = FrameEquivalence {
                hint: Some("same-secret-source".to_string()),
                proof: Some(vec![42; 1024]),
                version: Some(1),
                status: Some(FrameEquivalenceStatus::Ready),
                error: None,
            };

            let source = infra
                .capture_frame(
                    &test_frame("session-redaction-reuse", "source.png")
                        .with_equivalence(equivalence.clone()),
                    None,
                )
                .await
                .expect("source frame should persist");
            let source_job = source.job.expect("source OCR job should enqueue");
            infra
                .claim_queued_processing_job(source_job.id)
                .await
                .expect("source job should claim")
                .expect("source job should exist");
            infra
                .complete_processing_job(
                    source_job.id,
                    &ProcessingResultDraft::new().with_result_text("shared secret text"),
                )
                .await
                .expect("source job should complete");
            let source_result_id: i64 = sqlx::query_scalar(
                "SELECT id FROM processing_results WHERE job_id = ?1 ORDER BY id DESC LIMIT 1",
            )
            .bind(source_job.id)
            .fetch_one(infra.pool())
            .await
            .expect("source result should exist");
            sqlx::query(
                "INSERT INTO secret_redactions \
                    (anchor_type, frame_id, audio_segment_id, processing_result_id, category, redacted_start, redacted_end, detector_version) \
                 VALUES ('frame', ?1, NULL, ?2, 'api_key', 0, 6, 'test')",
            )
            .bind(source.frame.id)
            .bind(source_result_id)
            .execute(infra.pool())
            .await
            .expect("source redaction should insert");

            let target = infra
                .capture_frame(
                    &test_frame("session-redaction-reuse", "target.png")
                        .with_equivalence(equivalence),
                    None,
                )
                .await
                .expect("target frame should persist");

            assert!(target.job.is_none());
            assert_eq!(
                infra
                    .frame_secret_redaction_count(target.frame.id)
                    .await
                    .expect("redaction count should load"),
                1
            );
        });
    }

    #[test]
    fn visual_novelty_admitted_singleton_gets_ocr_job_and_becomes_searchable() {
        run_async_test(async {
            let dir = TestDir::new("visual-novelty-singleton-searchable");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            // A one-off readable screen with a unique fingerprint and no earlier
            // equivalent to borrow text from: the visual-novelty path admits it.
            let captured = infra
                .capture_frame_with_ocr_admission(
                    &test_frame("session-novelty", "scrolled-pr.png")
                        .with_equivalence(ready_equivalence("hint-scrolled-pr", 11)),
                    None,
                    novelty_admit_decision(),
                )
                .await
                .expect("novel frame should persist");

            // No equivalent existed, so the equivalence gate did not override the
            // novelty admission: an OCR job is enqueued for the singleton.
            let job = captured
                .job
                .expect("novel singleton should enqueue an OCR job");
            let decision = captured
                .ocr_admission_decision
                .expect("admission decision should be recorded");
            assert_eq!(decision.outcome, OcrAdmissionOutcome::Admitted);
            assert_eq!(decision.reason, OcrAdmissionReason::AdmittedVisualNovelty);

            infra
                .claim_queued_processing_job(job.id)
                .await
                .expect("novel job should claim")
                .expect("novel job should exist");
            infra
                .complete_processing_job(
                    job.id,
                    &ProcessingResultDraft::new()
                        .with_result_text("scrolled pull request diff line 4242"),
                )
                .await
                .expect("novel job should complete");

            // The previously-skippable singleton now carries a direct search row,
            // which is the exact criterion for CLI find-by-content.
            assert_eq!(
                search_document_kind(&infra, captured.frame.id)
                    .await
                    .as_deref(),
                Some("direct")
            );
            assert!(
                search_representative_frame_ids(&infra, "scrolled pull request")
                    .await
                    .contains(&captured.frame.id)
            );
        });
    }

    #[test]
    fn visual_novelty_admission_yields_to_equivalence_reuse_for_repeats() {
        run_async_test(async {
            let dir = TestDir::new("visual-novelty-repeat-reuse");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let equivalence = ready_equivalence("hint-repeated-dashboard", 22);

            // First occurrence of a repeated state is read directly.
            let first = infra
                .capture_frame_with_ocr_admission(
                    &test_frame_at("session-repeat", "dashboard-1.png", "2026-04-12T10:00:00Z")
                        .with_equivalence(equivalence.clone()),
                    None,
                    novelty_admit_decision(),
                )
                .await
                .expect("first frame should persist");
            let first_job = first.job.expect("first frame should enqueue an OCR job");
            infra
                .claim_queued_processing_job(first_job.id)
                .await
                .expect("first job should claim")
                .expect("first job should exist");
            infra
                .complete_processing_job(
                    first_job.id,
                    &ProcessingResultDraft::new().with_result_text("repeated dashboard overview"),
                )
                .await
                .expect("first job should complete");

            // An identical later frame, even if the budget admitted it for novelty,
            // is overridden by the equivalence gate and reuses the earlier text
            // rather than spending a second OCR read.
            let repeat = infra
                .capture_frame_with_ocr_admission(
                    &test_frame_at("session-repeat", "dashboard-2.png", "2026-04-12T10:00:01Z")
                        .with_equivalence(equivalence),
                    None,
                    novelty_admit_decision(),
                )
                .await
                .expect("repeat frame should persist");

            assert!(
                repeat.job.is_none(),
                "repeat must not spend a novelty OCR read"
            );
            let decision = repeat
                .ocr_admission_decision
                .expect("admission decision should be recorded");
            assert_eq!(decision.outcome, OcrAdmissionOutcome::Skipped);
            assert_eq!(decision.reason, OcrAdmissionReason::SkippedEquivalentFrame);
            assert_eq!(decision.related_frame_id, Some(first.frame.id));

            // The repeat is still searchable, via reused text rather than a direct read.
            assert_eq!(
                search_document_kind(&infra, first.frame.id)
                    .await
                    .as_deref(),
                Some("direct")
            );
            assert_eq!(
                search_document_kind(&infra, repeat.frame.id)
                    .await
                    .as_deref(),
                Some("equivalent_reuse")
            );
        });
    }

    #[test]
    fn novelty_denied_frame_is_dropped_without_an_ocr_job() {
        run_async_test(async {
            let dir = TestDir::new("visual-novelty-denied");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            // When the budget denies novelty (rate cap / continuous-novelty burst),
            // the decision arrives as a skip. A unique frame with no equivalent then
            // gets neither an OCR job nor a search row, keeping the cost bounded.
            let denied = infra
                .capture_frame_with_ocr_admission(
                    &test_frame("session-denied", "video-frame.png")
                        .with_equivalence(ready_equivalence("hint-video-frame", 33)),
                    None,
                    OcrAdmissionDecision::skip(OcrAdmissionReason::SkippedLowOcrValue, 0, true),
                )
                .await
                .expect("denied frame should persist");

            assert!(denied.job.is_none());
            assert_eq!(search_document_kind(&infra, denied.frame.id).await, None);
        });
    }

    fn write_test_png_rgba(
        dir: &TestDir,
        file_name: &str,
        width: u32,
        height: u32,
        pixels: &[u8],
    ) -> String {
        let path = dir.path().join(file_name);
        image::save_buffer(&path, pixels, width, height, image::ColorType::Rgba8)
            .expect("test png should be written");
        path.to_string_lossy().into_owned()
    }

    fn solid_rgba(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
        let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
        for _ in 0..(width as usize * height as usize) {
            pixels.extend_from_slice(&rgba);
        }
        pixels
    }

    fn set_pixel_rgba(pixels: &mut [u8], width: u32, x: u32, y: u32, rgba: [u8; 4]) {
        let offset = ((y * width + x) * 4) as usize;
        pixels[offset..offset + 4].copy_from_slice(&rgba);
    }

    fn test_embedding_bytes(embedding: &[f32]) -> Vec<u8> {
        let mut out = Vec::with_capacity(embedding.len() * 4);
        for value in embedding {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out
    }

    fn speaker_analysis_output_for_segment(
        session_id: &str,
        audio_segment_id: i64,
        provider_cluster_id: &str,
        embedding: &[f32],
        turn_text: Option<&str>,
    ) -> speaker_analysis::SpeakerAnalysisOutput {
        speaker_analysis::SpeakerAnalysisOutput {
            clusters: vec![speaker_analysis::SpeakerCluster {
                provider_cluster_id: provider_cluster_id.to_string(),
                stable_label: "Unknown Speaker 1".to_string(),
                embedding: test_embedding_bytes(embedding),
                embedding_model_id: "voice-model".to_string(),
                suggestion: None,
            }],
            turns: vec![speaker_analysis::SpeakerTurn {
                provider_cluster_id: provider_cluster_id.to_string(),
                start_ms: 0,
                end_ms: 1_000,
                transcript_text: turn_text.map(str::to_string),
                overlaps: false,
            }],
            metadata: speaker_analysis::SpeakerAnalysisMetadata {
                provider: "mock_speaker".to_string(),
                model_id: Some("voice-model".to_string()),
                session_id: session_id.to_string(),
                audio_segment_id,
                provenance: Default::default(),
            },
            provider_version: None,
        }
    }

    async fn complete_speaker_output(
        infra: &AppInfra,
        segment: &AudioSegment,
        output: speaker_analysis::SpeakerAnalysisOutput,
    ) -> ProcessingJob {
        let payload = serde_json::to_string(&SpeakerAnalysisJobPayload::new(
            "mock_speaker",
            Some("voice-model".to_string()),
        ))
        .expect("speaker payload should encode");
        let job = infra
            .enqueue_processing_job(
                &ProcessingJobDraft::for_audio_segment_speaker_analysis(segment.id)
                    .with_payload_json(payload),
            )
            .await
            .expect("speaker job should enqueue");
        infra
            .claim_queued_processing_job(job.id)
            .await
            .expect("speaker job should claim")
            .expect("claimed speaker job should exist");
        infra
            .complete_processing_job(
                job.id,
                &ProcessingResultDraft::new().with_structured_payload_json(
                    serde_json::to_string(&output).expect("speaker output should encode"),
                ),
            )
            .await
            .expect("speaker output should complete");
        job
    }

    #[test]
    fn completed_empty_speaker_analysis_persists_without_turns_or_clusters() {
        run_async_test(async {
            let dir = TestDir::new("speaker-analysis-empty-success");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "speaker-empty-session",
                    1,
                    "/tmp/speaker-empty.m4a",
                    "2026-04-12T10:00:00Z",
                    "2026-04-12T10:01:00Z",
                ))
                .await
                .expect("audio segment should insert");
            let mut output = speaker_analysis::SpeakerAnalysisOutput::new(
                speaker_analysis::SpeakerAnalysisMetadata {
                    provider: "mock_speaker".to_string(),
                    model_id: Some("voice-model".to_string()),
                    session_id: "speaker-empty-session".to_string(),
                    audio_segment_id: segment.id,
                    provenance: Default::default(),
                },
            );
            output
                .metadata
                .provenance
                .insert("skipReason".to_string(), serde_json::json!("silent"));

            let job = complete_speaker_output(&infra, &segment, output).await;
            let completed = infra
                .get_processing_job(job.id)
                .await
                .expect("job lookup should succeed")
                .expect("job should exist");
            assert_eq!(completed.status, ProcessingJobStatus::Completed);
            assert!(infra
                .get_processing_result_for_job(job.id)
                .await
                .expect("result lookup should succeed")
                .is_some());
            assert!(infra
                .list_speaker_turns_for_audio_segment(segment.id)
                .await
                .expect("turns should list")
                .is_empty());
            assert!(infra
                .list_speaker_clusters_for_session("speaker-empty-session")
                .await
                .expect("clusters should list")
                .is_empty());
        });
    }

    #[test]
    fn failed_speaker_analysis_records_error_and_clears_stale_result() {
        run_async_test(async {
            let dir = TestDir::new("speaker-analysis-failed-clears-result");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "speaker-fail-session",
                    1,
                    "/tmp/speaker-fail.m4a",
                    "2026-04-12T10:00:00Z",
                    "2026-04-12T10:01:00Z",
                ))
                .await
                .expect("audio segment should insert");
            let output = speaker_analysis_output_for_segment(
                "speaker-fail-session",
                segment.id,
                "speaker_00",
                &[1.0, 0.0],
                Some("hello"),
            );
            let job = complete_speaker_output(&infra, &segment, output).await;
            assert!(infra
                .get_processing_result_for_job(job.id)
                .await
                .expect("stale result lookup should succeed")
                .is_some());

            infra
                .mark_processing_job_running(job.id)
                .await
                .expect("speaker job should rerun");
            let failed = infra
                .mark_processing_job_failed(job.id, Some("subprocess failed during helper_exit"))
                .await
                .expect("speaker job should fail");

            assert_eq!(failed.status, ProcessingJobStatus::Failed);
            assert_eq!(
                failed.last_error.as_deref(),
                Some("subprocess failed during helper_exit")
            );
            assert!(infra
                .get_processing_result_for_job(job.id)
                .await
                .expect("failed result lookup should succeed")
                .is_none());
        });
    }

    #[derive(Clone, Default)]
    struct CapturingSpeakerAnalysisProvider {
        requests: Arc<Mutex<Vec<speaker_analysis::SpeakerAnalysisRequest>>>,
    }

    #[async_trait]
    impl speaker_analysis::SpeakerAnalysisProvider for CapturingSpeakerAnalysisProvider {
        fn provider(&self) -> &'static str {
            "mock_speaker"
        }

        async fn analyze(
            &self,
            request: speaker_analysis::SpeakerAnalysisRequest,
        ) -> speaker_analysis::SpeakerAnalysisResult<speaker_analysis::SpeakerAnalysisOutput>
        {
            self.requests
                .lock()
                .expect("captured speaker requests should lock")
                .push(request.clone());

            let suggestion = request.enrolled_people.first().map(|person| {
                speaker_analysis::SpeakerRecognitionSuggestion {
                    person_id: person.person_id,
                    display_name: person.display_name.clone(),
                    confidence: speaker_analysis::RecognitionConfidence::High,
                    score: 0.94,
                }
            });

            Ok(speaker_analysis::SpeakerAnalysisOutput {
                clusters: vec![speaker_analysis::SpeakerCluster {
                    provider_cluster_id: "speaker_00".to_string(),
                    stable_label: "Unknown Speaker 1".to_string(),
                    embedding: test_embedding_bytes(&[0.5, 0.5]),
                    embedding_model_id: request.model_id.clone().unwrap_or_default(),
                    suggestion,
                }],
                turns: vec![speaker_analysis::SpeakerTurn {
                    provider_cluster_id: "speaker_00".to_string(),
                    start_ms: 0,
                    end_ms: 1_000,
                    transcript_text: Some("hello".to_string()),
                    overlaps: false,
                }],
                metadata: speaker_analysis::SpeakerAnalysisMetadata::from_request(&request),
                provider_version: None,
            })
        }
    }

    fn test_frame_with_equivalent_image(
        dir: &TestDir,
        session_id: &str,
        file_name: &str,
        captured_at: &str,
        pixels: &[u8],
        width: u32,
        height: u32,
    ) -> NewFrame {
        let file_path = write_test_png_rgba(dir, file_name, width, height, pixels);
        let equivalence =
            match capture_screen::captured_frame_equivalence_from_image_path(Path::new(&file_path))
            {
                capture_screen::CapturedFrameEquivalenceOutcome::Ready(equivalence) => {
                    FrameEquivalence::ready(
                        equivalence.hint,
                        equivalence.proof,
                        equivalence.version,
                    )
                }
                capture_screen::CapturedFrameEquivalenceOutcome::Quarantined(error) => {
                    panic!("test image equivalence should compute: {error}");
                }
            };

        NewFrame::new(session_id, file_path, captured_at)
            .with_dimensions(width as i64, height as i64)
            .with_equivalence(equivalence)
    }

    fn test_segment_frame_with_equivalent_image(
        dir: &TestDir,
        session_id: &str,
        segment_index: u64,
        file_name: &str,
        captured_at: &str,
        pixels: &[u8],
        width: u32,
        height: u32,
    ) -> NewFrame {
        let frames_dir = dir.path().join(format!(
            "2026/04/12/.{session_id}-segment-{segment_index:04}/frames"
        ));
        fs::create_dir_all(&frames_dir).expect("segment frames dir should exist");
        let relative_name =
            format!("2026/04/12/.{session_id}-segment-{segment_index:04}/frames/{file_name}");
        test_frame_with_equivalent_image(
            dir,
            session_id,
            &relative_name,
            captured_at,
            pixels,
            width,
            height,
        )
    }

    #[derive(Debug)]
    struct SuccessfulProcessingBackend {
        processor: &'static str,
        result: ProcessingResultDraft,
    }

    impl SuccessfulProcessingBackend {
        fn new(processor: &'static str, result_text: &str) -> Self {
            Self {
                processor,
                result: ProcessingResultDraft::new().with_result_text(result_text),
            }
        }
    }

    #[async_trait]
    impl ProcessorBackend for SuccessfulProcessingBackend {
        fn processor(&self) -> &'static str {
            self.processor
        }

        async fn process(
            &self,
            _store: &ProcessingStore,
            _job: &ProcessingJob,
        ) -> Result<ProcessingResultDraft> {
            Ok(self.result.clone())
        }
    }

    fn test_processing_registry(result_text: &str) -> ProcessorRegistry {
        ProcessorRegistry::new().register(SuccessfulProcessingBackend::new(
            TEST_PROCESSOR,
            result_text,
        ))
    }

    #[test]
    fn database_reports_when_embedded_migrations_ran() {
        run_async_test(async {
            let dir = TestDir::new("migrations-ran");

            let first = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            assert!(first.migrations_ran());

            drop(first);

            let second = Database::initialize(dir.path())
                .await
                .expect("database should re-initialize");
            assert!(!second.migrations_ran());
        });
    }

    #[test]
    fn app_infra_reports_initialized_base_dir() {
        run_async_test(async {
            let dir = TestDir::new("base-dir");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            assert_eq!(infra.base_dir(), dir.path());
        });
    }

    #[test]
    fn cpu_jobs_persist_running_and_completed_transitions() {
        run_async_test(async {
            let dir = TestDir::new("cpu-job-success");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let (started_tx, started_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();

            let handle = infra
                .spawn_cpu_job(
                    JobDescriptor::new("ocr"),
                    Some("{\"documentId\":1}"),
                    move || {
                        started_tx
                            .send(())
                            .expect("job should notify when it starts");
                        release_rx
                            .recv()
                            .expect("job should wait until the test releases it");

                        Ok(CpuJobSuccess::new("finished".to_string())
                            .with_result_text("recognized text"))
                    },
                )
                .await
                .expect("cpu job should spawn");

            started_rx.recv().expect("job should reach the worker pool");

            let running = infra
                .jobs()
                .get(handle.job_id())
                .await
                .expect("running job should be readable")
                .expect("running job should exist");
            assert_eq!(running.status, BackgroundJobStatus::Running);
            assert_eq!(running.attempt_count, 1);
            assert!(running.started_at.is_some());
            assert!(running.finished_at.is_none());

            release_tx.send(()).expect("test should release the job");

            let outcome = handle.join().await.expect("job join should succeed");
            assert_eq!(
                outcome,
                Ok(CpuJobSuccess::new("finished".to_string()).with_result_text("recognized text"))
            );

            let completed = infra
                .jobs()
                .get(running.id)
                .await
                .expect("completed job should be readable")
                .expect("completed job should exist");
            assert_eq!(completed.status, BackgroundJobStatus::Completed);
            assert_eq!(completed.result_text.as_deref(), Some("recognized text"));
            assert!(completed.finished_at.is_some());
            assert_eq!(completed.last_error, None);
        });
    }

    #[test]
    fn enqueued_jobs_are_persisted_as_queued() {
        run_async_test(async {
            let dir = TestDir::new("queued-job");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let queued = infra
                .enqueue_job(&JobDescriptor::new("ocr"), Some("{\"documentId\":1}"))
                .await
                .expect("job should enqueue");

            assert_eq!(queued.status, BackgroundJobStatus::Queued);
            assert_eq!(queued.payload_json.as_deref(), Some("{\"documentId\":1}"));
            assert_eq!(queued.attempt_count, 0);
            assert!(queued.started_at.is_none());
            assert!(queued.finished_at.is_none());
        });
    }

    #[test]
    fn cpu_jobs_persist_failed_transitions() {
        run_async_test(async {
            let dir = TestDir::new("cpu-job-failure");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let handle: CpuJobHandle<String> = infra
                .spawn_cpu_job(JobDescriptor::new("transcription"), None, || {
                    Err("transcription failed".to_string())
                })
                .await
                .expect("cpu job should spawn");

            let job_id = handle.job_id();
            let outcome = handle.join().await.expect("job join should complete");
            assert_eq!(outcome, Err("transcription failed".to_string()));

            let failed = infra
                .jobs()
                .get(job_id)
                .await
                .expect("failed job should be readable")
                .expect("failed job should exist");
            assert_eq!(failed.status, BackgroundJobStatus::Failed);
            assert_eq!(failed.last_error.as_deref(), Some("transcription failed"));
            assert!(failed.started_at.is_some());
            assert!(failed.finished_at.is_some());
            assert_eq!(failed.result_text, None);
        });
    }

    #[test]
    fn cpu_job_panics_are_persisted_as_failed() {
        run_async_test(async {
            let dir = TestDir::new("cpu-job-panic");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let handle: CpuJobHandle<String> = infra
                .spawn_cpu_job(JobDescriptor::new("transcription"), None, || {
                    panic!("worker panic");
                })
                .await
                .expect("cpu job should spawn");

            let job_id = handle.job_id();
            let outcome = handle.join().await.expect("job join should complete");
            assert_eq!(outcome, Err("cpu job panicked: worker panic".to_string()));

            let failed = infra
                .jobs()
                .get(job_id)
                .await
                .expect("failed job should be readable")
                .expect("failed job should exist");
            assert_eq!(failed.status, BackgroundJobStatus::Failed);
            assert_eq!(
                failed.last_error.as_deref(),
                Some("cpu job panicked: worker panic")
            );
            assert!(failed.started_at.is_some());
            assert!(failed.finished_at.is_some());
            assert_eq!(failed.result_text, None);
        });
    }

    #[test]
    fn startup_reconciles_orphaned_running_jobs() {
        run_async_test(async {
            let dir = TestDir::new("orphaned-running-job");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let queued = infra
                .enqueue_job(&JobDescriptor::new("ocr"), Some("{\"documentId\":1}"))
                .await
                .expect("job should enqueue");

            let running = infra
                .jobs()
                .mark_running(queued.id)
                .await
                .expect("job should be marked running");
            assert_eq!(running.status, BackgroundJobStatus::Running);

            drop(infra);

            let recovered = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should re-initialize");

            let failed = recovered
                .jobs()
                .get(queued.id)
                .await
                .expect("recovered job should be readable")
                .expect("recovered job should exist");
            assert_eq!(failed.status, BackgroundJobStatus::Failed);
            assert_eq!(
                failed.last_error.as_deref(),
                Some(ORPHANED_RUNNING_JOB_ERROR)
            );
            assert!(failed.finished_at.is_some());
        });
    }

    #[test]
    fn queued_processing_jobs_persist_across_restart_and_stay_processable() {
        run_async_test(async {
            let dir = TestDir::new("processing-queued-restart");
            let initial = AppInfra::initialize_with_processing_registry(
                dir.path(),
                test_processing_registry("recovered text"),
            )
            .await
            .expect("app infra should initialize");

            let persisted = initial
                .debug_insert_frame_and_enqueue_processing_job(
                    &test_frame("session-processing-restart", "frame-processing-restart.png"),
                    TEST_PROCESSOR,
                    Some("{\"mode\":\"queued\"}"),
                )
                .await
                .expect("frame and job should persist");

            drop(initial);

            let recovered = AppInfra::initialize_with_processing_registry(
                dir.path(),
                test_processing_registry("recovered text"),
            )
            .await
            .expect("app infra should re-initialize");

            let queued = recovered
                .get_processing_job(persisted.job.id)
                .await
                .expect("queued job should be readable")
                .expect("queued job should exist");
            assert_eq!(queued.status, ProcessingJobStatus::Queued);
            assert_eq!(queued.attempt_count, 0);
            assert_eq!(
                queued.payload_json.as_deref(),
                Some("{\"mode\":\"queued\"}")
            );

            let outcome = recovered
                .process_next_processing_job()
                .await
                .expect("queued job should process after restart")
                .expect("queued job should exist");

            let ProcessingJobRunOutcome::Completed(completion) = outcome else {
                panic!("expected completed outcome");
            };

            assert_eq!(completion.job.id, persisted.job.id);
            assert_eq!(completion.job.status, ProcessingJobStatus::Completed);
            assert_eq!(completion.job.attempt_count, 1);
            assert_eq!(
                completion.result.result_text.as_deref(),
                Some("recovered text")
            );

            let stored_result = recovered
                .get_processing_result_for_job(persisted.job.id)
                .await
                .expect("processing result should be readable")
                .expect("processing result should exist");
            assert_eq!(stored_result, completion.result);
        });
    }

    #[test]
    fn startup_reclaims_orphaned_running_processing_jobs_by_requeue() {
        run_async_test(async {
            let dir = TestDir::new("processing-running-restart");
            let initial = AppInfra::initialize_with_processing_registry(
                dir.path(),
                test_processing_registry("unused result"),
            )
            .await
            .expect("app infra should initialize");

            let persisted = initial
                .debug_insert_frame_and_enqueue_processing_job(
                    &test_frame("session-processing-running", "frame-processing-running.png"),
                    TEST_PROCESSOR,
                    None,
                )
                .await
                .expect("frame and job should persist");

            let running = initial
                .claim_queued_processing_job(persisted.job.id)
                .await
                .expect("job claim should succeed")
                .expect("job should claim");
            assert_eq!(running.status, ProcessingJobStatus::Running);
            assert_eq!(running.attempt_count, 1);

            drop(initial);

            let recovered = AppInfra::initialize_with_processing_registry(
                dir.path(),
                test_processing_registry("unused result"),
            )
            .await
            .expect("app infra should re-initialize");

            // Startup reclamation requeues the orphaned job so it re-runs, rather than failing it.
            let reclaimed = recovered
                .get_processing_job(persisted.job.id)
                .await
                .expect("recovered job should be readable")
                .expect("recovered job should exist");
            assert_eq!(reclaimed.status, ProcessingJobStatus::Queued);
            assert_eq!(
                reclaimed.attempt_count, 1,
                "the abandoned run still counts toward the total attempt ceiling"
            );
            assert_eq!(
                reclaimed.failure_count, 0,
                "abandonment must not spend a failure attempt"
            );
            assert_eq!(
                reclaimed.last_error.as_deref(),
                Some(RECLAIMED_ORPHANED_PROCESSING_JOB_MESSAGE)
            );
            assert!(reclaimed.started_at.is_none());
            assert!(reclaimed.finished_at.is_none());
            assert!(recovered
                .get_processing_result_for_job(persisted.job.id)
                .await
                .expect("result lookup should succeed")
                .is_none());
        });
    }

    #[test]
    fn processing_results_stay_clean_when_retrying_after_restart_recovery() {
        run_async_test(async {
            let dir = TestDir::new("processing-retry-recovery");
            let initial = AppInfra::initialize_with_processing_registry(
                dir.path(),
                test_processing_registry("first pass"),
            )
            .await
            .expect("app infra should initialize");

            let persisted = initial
                .debug_insert_frame_and_enqueue_processing_job(
                    &test_frame("session-processing-retry", "frame-processing-retry.png"),
                    TEST_PROCESSOR,
                    None,
                )
                .await
                .expect("frame and job should persist");

            let first_outcome = initial
                .process_processing_job(persisted.job.id)
                .await
                .expect("initial processing should succeed");
            let ProcessingJobRunOutcome::Completed(first_completion) = first_outcome else {
                panic!("expected completed outcome");
            };
            assert_eq!(
                first_completion.result.result_text.as_deref(),
                Some("first pass")
            );

            let rerunning = initial
                .mark_processing_job_running(persisted.job.id)
                .await
                .expect("completed job should restart");
            assert_eq!(rerunning.status, ProcessingJobStatus::Running);
            assert_eq!(rerunning.attempt_count, 2);
            assert!(initial
                .get_processing_result_for_job(persisted.job.id)
                .await
                .expect("stale result lookup should succeed")
                .is_none());

            drop(initial);

            let recovered = AppInfra::initialize_with_processing_registry(
                dir.path(),
                test_processing_registry("second pass"),
            )
            .await
            .expect("app infra should re-initialize");

            // Startup reclamation requeues the orphaned re-run rather than failing it, and the
            // stale result from the previous run stays cleared.
            let reclaimed = recovered
                .get_processing_job(persisted.job.id)
                .await
                .expect("recovered job should be readable")
                .expect("recovered job should exist");
            assert_eq!(reclaimed.status, ProcessingJobStatus::Queued);
            assert_eq!(reclaimed.attempt_count, 2);
            assert_eq!(
                reclaimed.last_error.as_deref(),
                Some(RECLAIMED_ORPHANED_PROCESSING_JOB_MESSAGE)
            );
            assert!(recovered
                .get_processing_result_for_job(persisted.job.id)
                .await
                .expect("recovered result lookup should succeed")
                .is_none());

            // The reclaimed (requeued) job re-runs on the next drain and produces a fresh result.
            let retried_outcome = recovered
                .process_processing_job(persisted.job.id)
                .await
                .expect("retried processing should succeed");
            let ProcessingJobRunOutcome::Completed(retried_completion) = retried_outcome else {
                panic!("expected completed outcome");
            };
            assert_eq!(retried_completion.job.attempt_count, 3);

            assert_eq!(
                retried_completion.result.result_text.as_deref(),
                Some("second pass")
            );
            assert_ne!(retried_completion.result.id, first_completion.result.id);

            let stored_result = recovered
                .get_processing_result_for_job(persisted.job.id)
                .await
                .expect("final result lookup should succeed")
                .expect("final result should exist");
            assert_eq!(stored_result, retried_completion.result);

            let subject_results = recovered
                .list_processing_results_for_subject(&ProcessingSubject::frame(persisted.frame.id))
                .await
                .expect("subject results should list");
            assert_eq!(subject_results, vec![retried_completion.result]);
        });
    }

    #[test]
    fn spawn_setup_failures_mark_jobs_failed() {
        let dir = TestDir::new("spawn-setup-failure");

        let (jobs, job, runtime) = {
            let setup_runtime = build_test_runtime();
            setup_runtime.block_on(async {
                let database = Database::initialize(dir.path())
                    .await
                    .expect("database should initialize");
                let jobs = JobStore::new(database.pool().clone());
                let job = jobs
                    .enqueue(&JobDescriptor::new("ocr"), Some("{\"documentId\":1}"))
                    .await
                    .expect("job should enqueue");
                let runtime = JobRuntime::new(1).expect("job runtime should initialize");

                (jobs, job, runtime)
            })
        };

        let error = runtime
            .spawn_cpu(jobs.clone(), job.clone(), || {
                Ok(CpuJobSuccess::new("done".to_string()))
            })
            .err()
            .expect("spawning without a tokio runtime should fail");
        assert!(matches!(error, AppInfraError::AsyncRuntimeUnavailable));

        let verify_runtime = build_test_runtime();
        verify_runtime.block_on(async {
            let failed = jobs
                .get(job.id)
                .await
                .expect("failed job should be readable")
                .expect("failed job should exist");
            assert_eq!(failed.status, BackgroundJobStatus::Failed);
            assert_eq!(
                failed.last_error.as_deref(),
                Some("background jobs require an active Tokio runtime")
            );
            assert!(failed.started_at.is_none());
            assert!(failed.finished_at.is_some());
        });
    }

    #[test]
    fn frames_are_persisted_and_listable() {
        run_async_test(async {
            let dir = TestDir::new("processing-frames");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let first = infra
                .insert_frame(&test_frame("session-a", "frame-a.png"))
                .await
                .expect("frame should persist");
            let second = infra
                .insert_frame(&test_frame("session-b", "frame-b.png"))
                .await
                .expect("second frame should persist");

            let fetched = infra
                .get_frame(first.id)
                .await
                .expect("frame should be readable")
                .expect("frame should exist");
            assert_eq!(fetched, first);

            let session_a_frames = infra
                .list_frames(Some("session-a"), None, None, None)
                .await
                .expect("session frames should list");
            assert_eq!(session_a_frames, vec![first.clone()]);

            let all_frames = infra
                .list_frames(None, None, None, None)
                .await
                .expect("frames should list");
            assert_eq!(all_frames.len(), 2);
            assert_eq!(all_frames[0].id, second.id);
            assert_eq!(all_frames[1].id, first.id);
        });
    }

    #[test]
    fn frame_metadata_snapshots_are_deduped_and_linked() {
        run_async_test(async {
            let dir = TestDir::new("processing-frame-metadata");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let snapshot = capture_metadata::FrameMetadataSnapshot {
                app_bundle_id: Some("com.example.App".to_string()),
                app_name: Some("Example".to_string()),
                window_title: Some("Private Project".to_string()),
                window_id: None,
                browser_url: Some("https://example.com/private".to_string()),
                display_id: Some(1),
                metadata_redaction_reason: None,
                metadata_redaction_source_id: None,
            };

            let first = infra
                .insert_frame(
                    &test_frame("session-metadata", "frame-a.png")
                        .with_metadata_snapshot(snapshot.clone()),
                )
                .await
                .expect("frame should persist");
            let second = infra
                .insert_frame(
                    &test_frame("session-metadata", "frame-b.png")
                        .with_metadata_snapshot(snapshot.clone()),
                )
                .await
                .expect("second frame should persist");

            assert_eq!(first.metadata_snapshot.as_ref(), Some(&snapshot));
            assert_eq!(second.metadata_snapshot.as_ref(), Some(&snapshot));

            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM frame_metadata_snapshots")
                .fetch_one(infra.pool())
                .await
                .expect("snapshot count should load");
            assert_eq!(count, 1);
        });
    }

    #[test]
    fn audio_segments_upsert_is_idempotent_and_lists_overlapping_ranges() {
        run_async_test(async {
            let dir = TestDir::new("audio-segments");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let segment = NewAudioSegment::new(
                AudioSegmentSourceKind::Microphone,
                "mic-session",
                1,
                "/tmp/mic-1.m4a",
                "2026-04-12T10:00:00Z",
                "2026-04-12T10:01:00Z",
            );

            let inserted = infra
                .upsert_audio_segment(&segment)
                .await
                .expect("audio segment should insert");
            let updated = infra
                .upsert_audio_segment(&segment)
                .await
                .expect("duplicate audio segment should upsert");

            assert_eq!(inserted.id, updated.id);

            let fetched = infra
                .get_audio_segment(inserted.id)
                .await
                .expect("audio segment should be readable")
                .expect("audio segment should exist");
            assert_eq!(fetched, updated);

            let missing = infra
                .get_audio_segment(inserted.id + 10_000)
                .await
                .expect("missing audio segment lookup should succeed");
            assert!(missing.is_none());

            infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::SystemAudio,
                    "system-session",
                    1,
                    "/tmp/system-1.m4a",
                    "2026-04-12T10:01:00Z",
                    "2026-04-12T10:02:00Z",
                ))
                .await
                .expect("system audio segment should insert");

            let overlapping = infra
                .list_audio_segments_overlapping_range(
                    "2026-04-12T10:00:30Z",
                    "2026-04-12T10:01:30Z",
                    None,
                    None,
                )
                .await
                .expect("overlapping audio segments should list");
            assert_eq!(overlapping.len(), 2);
            assert_eq!(
                overlapping[0].source_kind,
                AudioSegmentSourceKind::Microphone
            );
            assert_eq!(
                overlapping[1].source_kind,
                AudioSegmentSourceKind::SystemAudio
            );

            let microphone_only = infra
                .list_audio_segments_overlapping_range(
                    "2026-04-12T10:00:30Z",
                    "2026-04-12T10:01:30Z",
                    Some(AudioSegmentSourceKind::Microphone),
                    Some("mic-session"),
                )
                .await
                .expect("filtered audio segments should list");
            assert_eq!(microphone_only, vec![updated]);

            let touching_boundary = infra
                .list_audio_segments_overlapping_range(
                    "2026-04-12T10:01:00Z",
                    "2026-04-12T10:01:00Z",
                    None,
                    None,
                )
                .await
                .expect("boundary-touching audio segments should list");
            assert_eq!(touching_boundary.len(), 2);
            assert_eq!(
                touching_boundary[0].source_kind,
                AudioSegmentSourceKind::Microphone
            );
            assert_eq!(
                touching_boundary[1].source_kind,
                AudioSegmentSourceKind::SystemAudio
            );

            let outside = infra
                .list_audio_segments_overlapping_range(
                    "2026-04-12T10:02:01Z",
                    "2026-04-12T10:03:00Z",
                    None,
                    None,
                )
                .await
                .expect("outside range should list");
            assert!(outside.is_empty());
        });
    }

    #[test]
    fn rejecting_speaker_suggestion_persists_negative_voice_example() {
        run_async_test(async {
            let dir = TestDir::new("speaker-rejection");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "speaker-rejection-session",
                    1,
                    "/tmp/speaker-rejection.m4a",
                    "2026-04-12T10:00:00Z",
                    "2026-04-12T10:01:00Z",
                ))
                .await
                .expect("audio segment should insert");
            let jack = infra
                .create_person_profile("Jack", None)
                .await
                .expect("person profile should insert");
            let job = infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_audio_segment_speaker_analysis(segment.id)
                        .with_payload_json(
                            serde_json::to_string(&SpeakerAnalysisJobPayload::new(
                                "sherpa_onnx",
                                Some("pyannote-3.0-nemo-titanet-small".to_string()),
                            ))
                            .expect("payload should encode"),
                        ),
                )
                .await
                .expect("speaker analysis job should enqueue");
            infra
                .claim_queued_processing_job(job.id)
                .await
                .expect("job should claim")
                .expect("claimed job should exist");

            let metadata = speaker_analysis::SpeakerAnalysisMetadata {
                provider: "sherpa_onnx".to_string(),
                model_id: Some("pyannote-3.0-nemo-titanet-small".to_string()),
                session_id: "speaker-rejection-session".to_string(),
                audio_segment_id: segment.id,
                provenance: Default::default(),
            };
            let output = speaker_analysis::SpeakerAnalysisOutput {
                clusters: vec![speaker_analysis::SpeakerCluster {
                    provider_cluster_id: "speaker_00".to_string(),
                    stable_label: "Unknown Speaker 1".to_string(),
                    embedding: test_embedding_bytes(&[1.0, 0.0]),
                    embedding_model_id: "pyannote-3.0-nemo-titanet-small".to_string(),
                    suggestion: Some(speaker_analysis::SpeakerRecognitionSuggestion {
                        person_id: jack.id,
                        display_name: "Jack".to_string(),
                        confidence: speaker_analysis::RecognitionConfidence::High,
                        score: 0.91,
                    }),
                }],
                turns: vec![speaker_analysis::SpeakerTurn {
                    provider_cluster_id: "speaker_00".to_string(),
                    start_ms: 0,
                    end_ms: 1_000,
                    transcript_text: Some("hello".to_string()),
                    overlaps: false,
                }],
                metadata,
                provider_version: None,
            };

            infra
                .complete_processing_job(
                    job.id,
                    &ProcessingResultDraft::new().with_structured_payload_json(
                        serde_json::to_string(&output).expect("output should encode"),
                    ),
                )
                .await
                .expect("speaker analysis should complete");
            let cluster = infra
                .list_speaker_clusters_for_session("speaker-rejection-session")
                .await
                .expect("clusters should list")
                .into_iter()
                .next()
                .expect("cluster should exist");
            assert_eq!(cluster.suggested_person_id, Some(jack.id));

            let rejected = infra
                .reject_speaker_recognition_suggestion(cluster.id)
                .await
                .expect("suggestion should reject");
            assert_eq!(rejected.suggested_person_id, None);
            let rejections = infra
                .processing
                .list_person_recognition_rejections_for_speaker_model(
                    "sherpa_onnx",
                    Some("pyannote-3.0-nemo-titanet-small"),
                )
                .await
                .expect("rejections should list");

            assert_eq!(rejections.len(), 1);
            assert_eq!(rejections[0].person_id, jack.id);
            assert_eq!(rejections[0].embedding, test_embedding_bytes(&[1.0, 0.0]));
        });
    }

    #[test]
    fn unlinking_confirmed_speaker_profile_persists_negative_voice_example() {
        run_async_test(async {
            let dir = TestDir::new("speaker-confirmed-unlink");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "speaker-confirmed-unlink-session",
                    1,
                    "/tmp/speaker-confirmed-unlink.m4a",
                    "2026-04-12T10:00:00Z",
                    "2026-04-12T10:01:00Z",
                ))
                .await
                .expect("audio segment should insert");
            let jack = infra
                .create_person_profile("Jack", None)
                .await
                .expect("person profile should insert");
            let job = infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_audio_segment_speaker_analysis(segment.id)
                        .with_payload_json(
                            serde_json::to_string(&SpeakerAnalysisJobPayload::new(
                                "sherpa_onnx",
                                Some("pyannote-3.0-nemo-titanet-small".to_string()),
                            ))
                            .expect("payload should encode"),
                        ),
                )
                .await
                .expect("speaker analysis job should enqueue");
            infra
                .claim_queued_processing_job(job.id)
                .await
                .expect("job should claim")
                .expect("claimed job should exist");

            let output = speaker_analysis::SpeakerAnalysisOutput {
                clusters: vec![speaker_analysis::SpeakerCluster {
                    provider_cluster_id: "speaker_00".to_string(),
                    stable_label: "Unknown Speaker 1".to_string(),
                    embedding: test_embedding_bytes(&[0.0, 1.0]),
                    embedding_model_id: "pyannote-3.0-nemo-titanet-small".to_string(),
                    suggestion: None,
                }],
                turns: vec![speaker_analysis::SpeakerTurn {
                    provider_cluster_id: "speaker_00".to_string(),
                    start_ms: 0,
                    end_ms: 1_000,
                    transcript_text: Some("hello".to_string()),
                    overlaps: false,
                }],
                metadata: speaker_analysis::SpeakerAnalysisMetadata {
                    provider: "sherpa_onnx".to_string(),
                    model_id: Some("pyannote-3.0-nemo-titanet-small".to_string()),
                    session_id: "speaker-confirmed-unlink-session".to_string(),
                    audio_segment_id: segment.id,
                    provenance: Default::default(),
                },
                provider_version: None,
            };

            infra
                .complete_processing_job(
                    job.id,
                    &ProcessingResultDraft::new().with_structured_payload_json(
                        serde_json::to_string(&output).expect("output should encode"),
                    ),
                )
                .await
                .expect("speaker analysis should complete");
            let cluster = infra
                .list_speaker_clusters_for_session("speaker-confirmed-unlink-session")
                .await
                .expect("clusters should list")
                .into_iter()
                .next()
                .expect("cluster should exist");
            let linked = infra
                .link_speaker_cluster_to_person(cluster.id, jack.id, false)
                .await
                .expect("cluster should link");
            assert_eq!(linked.person_id, Some(jack.id));

            let unlinked = infra
                .unlink_speaker_cluster_from_person(cluster.id)
                .await
                .expect("cluster should unlink");
            assert_eq!(unlinked.person_id, None);
            let rejections = infra
                .processing
                .list_person_recognition_rejections_for_speaker_model(
                    "sherpa_onnx",
                    Some("pyannote-3.0-nemo-titanet-small"),
                )
                .await
                .expect("rejections should list");

            assert_eq!(rejections.len(), 1);
            assert_eq!(rejections[0].person_id, jack.id);
            assert_eq!(rejections[0].embedding, test_embedding_bytes(&[0.0, 1.0]));
        });
    }

    #[test]
    fn saved_speaker_profile_embedding_is_available_to_later_sessions() {
        run_async_test(async {
            let dir = TestDir::new("speaker-cross-session-recognition");
            let provider = CapturingSpeakerAnalysisProvider::default();
            let captured_requests = provider.requests.clone();
            let infra = AppInfra::initialize_with_processing_registry(
                dir.path(),
                ProcessorRegistry::new().register(SpeakerAnalysisProcessorBackend::new(provider)),
            )
            .await
            .expect("app infra should initialize");

            let first_segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "speaker-profile-source-session",
                    1,
                    "/tmp/speaker-profile-source.m4a",
                    "2026-04-12T10:00:00Z",
                    "2026-04-12T10:01:00Z",
                ))
                .await
                .expect("source audio segment should insert");
            let first_job = infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_audio_segment_speaker_analysis(first_segment.id)
                        .with_payload_json(
                            serde_json::to_string(&SpeakerAnalysisJobPayload::new(
                                "mock_speaker",
                                Some("voice-model".to_string()),
                            ))
                            .expect("source payload should encode"),
                        ),
                )
                .await
                .expect("source speaker analysis job should enqueue");
            infra
                .claim_queued_processing_job(first_job.id)
                .await
                .expect("source job should claim")
                .expect("source job should exist");
            let first_output = speaker_analysis::SpeakerAnalysisOutput {
                clusters: vec![speaker_analysis::SpeakerCluster {
                    provider_cluster_id: "speaker_00".to_string(),
                    stable_label: "Unknown Speaker 1".to_string(),
                    embedding: test_embedding_bytes(&[1.0, 0.0]),
                    embedding_model_id: "voice-model".to_string(),
                    suggestion: None,
                }],
                turns: vec![speaker_analysis::SpeakerTurn {
                    provider_cluster_id: "speaker_00".to_string(),
                    start_ms: 0,
                    end_ms: 1_000,
                    transcript_text: Some("hello".to_string()),
                    overlaps: false,
                }],
                metadata: speaker_analysis::SpeakerAnalysisMetadata {
                    provider: "mock_speaker".to_string(),
                    model_id: Some("voice-model".to_string()),
                    session_id: "speaker-profile-source-session".to_string(),
                    audio_segment_id: first_segment.id,
                    provenance: Default::default(),
                },
                provider_version: None,
            };
            infra
                .complete_processing_job(
                    first_job.id,
                    &ProcessingResultDraft::new().with_structured_payload_json(
                        serde_json::to_string(&first_output).expect("source output should encode"),
                    ),
                )
                .await
                .expect("source speaker analysis should complete");
            let source_cluster = infra
                .list_speaker_clusters_for_session("speaker-profile-source-session")
                .await
                .expect("source clusters should list")
                .into_iter()
                .next()
                .expect("source cluster should exist");
            let jack = infra
                .create_person_profile("Jack", None)
                .await
                .expect("person profile should insert");
            infra
                .link_speaker_cluster_to_person(source_cluster.id, jack.id, true)
                .await
                .expect("source cluster should link and enroll embedding");

            let later_segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "speaker-profile-later-session",
                    1,
                    "/tmp/speaker-profile-later.m4a",
                    "2026-04-12T10:05:00Z",
                    "2026-04-12T10:06:00Z",
                ))
                .await
                .expect("later audio segment should insert");
            let mut later_payload =
                SpeakerAnalysisJobPayload::new("mock_speaker", Some("voice-model".to_string()));
            later_payload.recognize_people = true;
            let later_job = infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_audio_segment_speaker_analysis(later_segment.id)
                        .with_payload_json(
                            serde_json::to_string(&later_payload)
                                .expect("later payload should encode"),
                        ),
                )
                .await
                .expect("later speaker analysis job should enqueue");

            infra
                .process_processing_job(later_job.id)
                .await
                .expect("later speaker analysis should process");

            let requests = captured_requests
                .lock()
                .expect("captured speaker requests should lock");
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].session_id, "speaker-profile-later-session");
            assert_eq!(requests[0].enrolled_people.len(), 1);
            assert_eq!(requests[0].enrolled_people[0].person_id, jack.id);
            assert_eq!(
                requests[0].enrolled_people[0].embedding,
                test_embedding_bytes(&[1.0, 0.0])
            );
            drop(requests);

            let later_cluster = infra
                .list_speaker_clusters_for_session("speaker-profile-later-session")
                .await
                .expect("later clusters should list")
                .into_iter()
                .next()
                .expect("later cluster should exist");
            assert_eq!(later_cluster.suggested_person_id, Some(jack.id));
            assert_eq!(
                later_cluster.recognition_confidence.as_deref(),
                Some("high")
            );
        });
    }

    #[test]
    fn microphone_audio_segment_commit_admits_transcription_job_idempotently() {
        run_async_test(async {
            let dir = TestDir::new("audio-segment-transcription-admission");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let payload = serde_json::to_string(&AudioTranscriptionJobPayload::new(
                "local_whisper",
                Some("base".to_string()),
                "auto",
            ))
            .expect("payload should serialize");
            let admission = AudioSegmentTranscriptionAdmission::available(payload.clone());
            let segment = NewAudioSegment::new(
                AudioSegmentSourceKind::Microphone,
                "mic-session",
                1,
                "/tmp/mic-1.m4a",
                "2026-04-12T10:00:00Z",
                "2026-04-12T10:01:00Z",
            );

            let first = infra
                .upsert_audio_segment_and_maybe_enqueue_transcription(&segment, &admission)
                .await
                .expect("segment and transcription job should commit");
            let second = infra
                .upsert_audio_segment_and_maybe_enqueue_transcription(&segment, &admission)
                .await
                .expect("duplicate commit should be idempotent");

            assert_eq!(first.segment.id, second.segment.id);
            assert_eq!(
                first.job.as_ref().map(|job| job.id),
                second.job.map(|job| job.id)
            );
            let jobs = infra
                .list_processing_jobs_for_subject(&ProcessingSubject::audio_segment(
                    first.segment.id,
                ))
                .await
                .expect("jobs should list");
            assert_eq!(jobs.len(), 1);
            assert_eq!(jobs[0].subject_type, AUDIO_SEGMENT_SUBJECT_TYPE);
            assert_eq!(jobs[0].processor, AUDIO_TRANSCRIPTION_PROCESSOR);
            assert_eq!(jobs[0].payload_json.as_deref(), Some(payload.as_str()));
        });
    }

    #[test]
    fn speaker_segment_local_labels_do_not_force_stable_identity() {
        run_async_test(async {
            let dir = TestDir::new("speaker-segment-local-labels");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let first = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "speaker-session",
                    1,
                    "/tmp/speaker-local-1.m4a",
                    "2026-04-12T10:00:00Z",
                    "2026-04-12T10:01:00Z",
                ))
                .await
                .expect("first segment should insert");
            let second = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "speaker-session",
                    2,
                    "/tmp/speaker-local-2.m4a",
                    "2026-04-12T10:01:00Z",
                    "2026-04-12T10:02:00Z",
                ))
                .await
                .expect("second segment should insert");

            complete_speaker_output(
                &infra,
                &first,
                speaker_analysis_output_for_segment(
                    "speaker-session",
                    first.id,
                    "speaker_00",
                    &[1.0, 0.0],
                    Some("first"),
                ),
            )
            .await;
            complete_speaker_output(
                &infra,
                &second,
                speaker_analysis_output_for_segment(
                    "speaker-session",
                    second.id,
                    "speaker_00",
                    &[0.0, 1.0],
                    Some("second"),
                ),
            )
            .await;

            let clusters = infra
                .list_speaker_clusters_for_session("speaker-session")
                .await
                .expect("clusters should list");
            assert_eq!(clusters.len(), 2);
            assert_ne!(clusters[0].id, clusters[1].id);
            let second_turn = infra
                .list_speaker_turns_for_audio_segment(second.id)
                .await
                .expect("second turns should list")
                .pop()
                .expect("second turn should exist");
            assert_eq!(second_turn.cluster_id, clusters[1].id);
            assert!(second_turn.segment_cluster_id.is_some());
        });
    }

    #[test]
    fn moving_speaker_turn_rejects_cross_session_cluster() {
        run_async_test(async {
            let dir = TestDir::new("speaker-turn-cross-session-move");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let source_segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "speaker-move-source",
                    1,
                    "/tmp/speaker-move-source.m4a",
                    "2026-04-12T10:00:00Z",
                    "2026-04-12T10:01:00Z",
                ))
                .await
                .expect("source segment should insert");
            let target_segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "speaker-move-target",
                    1,
                    "/tmp/speaker-move-target.m4a",
                    "2026-04-12T10:00:00Z",
                    "2026-04-12T10:01:00Z",
                ))
                .await
                .expect("target segment should insert");

            complete_speaker_output(
                &infra,
                &source_segment,
                speaker_analysis_output_for_segment(
                    "speaker-move-source",
                    source_segment.id,
                    "speaker_00",
                    &[1.0, 0.0],
                    Some("source"),
                ),
            )
            .await;
            complete_speaker_output(
                &infra,
                &target_segment,
                speaker_analysis_output_for_segment(
                    "speaker-move-target",
                    target_segment.id,
                    "speaker_00",
                    &[0.0, 1.0],
                    Some("target"),
                ),
            )
            .await;

            let source_turn = infra
                .list_speaker_turns_for_audio_segment(source_segment.id)
                .await
                .expect("source turns should list")
                .pop()
                .expect("source turn should exist");
            let target_cluster = infra
                .list_speaker_clusters_for_session("speaker-move-target")
                .await
                .expect("target clusters should list")
                .pop()
                .expect("target cluster should exist");

            let result = infra
                .move_speaker_turn_to_cluster(source_turn.id, target_cluster.id)
                .await;
            assert!(result.is_err());
            let unchanged = infra
                .list_speaker_turns_for_audio_segment(source_segment.id)
                .await
                .expect("source turns should list after failed move")
                .pop()
                .expect("source turn should still exist");
            assert_eq!(unchanged.cluster_id, source_turn.cluster_id);
        });
    }

    #[test]
    fn merging_speaker_clusters_removes_source_cluster() {
        run_async_test(async {
            let dir = TestDir::new("speaker-cluster-merge-removes-source");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let first = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "speaker-cluster-merge-session",
                    1,
                    "/tmp/speaker-cluster-merge-1.m4a",
                    "2026-04-12T10:00:00Z",
                    "2026-04-12T10:01:00Z",
                ))
                .await
                .expect("first segment should insert");
            let second = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "speaker-cluster-merge-session",
                    2,
                    "/tmp/speaker-cluster-merge-2.m4a",
                    "2026-04-12T10:01:00Z",
                    "2026-04-12T10:02:00Z",
                ))
                .await
                .expect("second segment should insert");

            complete_speaker_output(
                &infra,
                &first,
                speaker_analysis_output_for_segment(
                    "speaker-cluster-merge-session",
                    first.id,
                    "speaker_00",
                    &[1.0, 0.0],
                    Some("first"),
                ),
            )
            .await;
            complete_speaker_output(
                &infra,
                &second,
                speaker_analysis_output_for_segment(
                    "speaker-cluster-merge-session",
                    second.id,
                    "speaker_00",
                    &[0.0, 1.0],
                    Some("second"),
                ),
            )
            .await;

            let clusters = infra
                .list_speaker_clusters_for_session("speaker-cluster-merge-session")
                .await
                .expect("clusters should list");
            assert_eq!(clusters.len(), 2);
            let source_cluster_id = clusters[1].id;
            let target_cluster_id = clusters[0].id;

            let merged = infra
                .merge_speaker_clusters(source_cluster_id, target_cluster_id)
                .await
                .expect("clusters should merge");
            assert_eq!(merged.id, target_cluster_id);

            let clusters = infra
                .list_speaker_clusters_for_session("speaker-cluster-merge-session")
                .await
                .expect("clusters should list after merge");
            assert_eq!(clusters.len(), 1);
            assert_eq!(clusters[0].id, target_cluster_id);
            let moved_turn = infra
                .list_speaker_turns_for_audio_segment(second.id)
                .await
                .expect("second turns should list after merge")
                .pop()
                .expect("second turn should still exist");
            assert_eq!(moved_turn.cluster_id, target_cluster_id);
        });
    }

    #[test]
    fn reprocessing_speaker_analysis_removes_obsolete_clusters() {
        run_async_test(async {
            let dir = TestDir::new("speaker-reprocess-obsolete-clusters");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "speaker-reprocess-session",
                    1,
                    "/tmp/speaker-reprocess.m4a",
                    "2026-04-12T10:00:00Z",
                    "2026-04-12T10:01:00Z",
                ))
                .await
                .expect("segment should insert");

            complete_speaker_output(
                &infra,
                &segment,
                speaker_analysis_output_for_segment(
                    "speaker-reprocess-session",
                    segment.id,
                    "speaker_00",
                    &[1.0, 0.0],
                    Some("old"),
                ),
            )
            .await;
            let old_clusters = infra
                .list_speaker_clusters_for_session("speaker-reprocess-session")
                .await
                .expect("old clusters should list");
            assert_eq!(old_clusters.len(), 1);

            complete_speaker_output(
                &infra,
                &segment,
                speaker_analysis_output_for_segment(
                    "speaker-reprocess-session",
                    segment.id,
                    "speaker_01",
                    &[0.0, 1.0],
                    Some("new"),
                ),
            )
            .await;

            let clusters = infra
                .list_speaker_clusters_for_session("speaker-reprocess-session")
                .await
                .expect("clusters should list after reprocess");
            assert_eq!(clusters.len(), 1);
            assert_eq!(
                clusters[0].provider_cluster_id,
                format!("{}:speaker_01", segment.id)
            );
            assert_ne!(clusters[0].id, old_clusters[0].id);
            let turns = infra
                .list_speaker_turns_for_audio_segment(segment.id)
                .await
                .expect("turns should list after reprocess");
            assert_eq!(turns.len(), 1);
            assert_eq!(turns[0].cluster_id, clusters[0].id);
        });
    }

    #[test]
    fn speaker_cluster_resolution_auto_merges_suggests_and_skips_ambiguous() {
        run_async_test(async {
            let dir = TestDir::new("speaker-cluster-resolution");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let mut segment_ids = Vec::new();
            for index in 0..4 {
                let segment = infra
                    .upsert_audio_segment(&NewAudioSegment::new(
                        AudioSegmentSourceKind::Microphone,
                        "speaker-resolution",
                        index + 1,
                        format!("/tmp/speaker-resolution-{index}.m4a"),
                        "2026-04-12T10:00:00Z",
                        "2026-04-12T10:01:00Z",
                    ))
                    .await
                    .expect("segment should insert");
                segment_ids.push(segment);
            }

            complete_speaker_output(
                &infra,
                &segment_ids[0],
                speaker_analysis_output_for_segment(
                    "speaker-resolution",
                    segment_ids[0].id,
                    "speaker_00",
                    &[1.0, 0.0],
                    None,
                ),
            )
            .await;
            complete_speaker_output(
                &infra,
                &segment_ids[1],
                speaker_analysis_output_for_segment(
                    "speaker-resolution",
                    segment_ids[1].id,
                    "speaker_00",
                    &[0.99, 0.01],
                    None,
                ),
            )
            .await;
            let clusters_after_auto = infra
                .list_speaker_clusters_for_session("speaker-resolution")
                .await
                .expect("clusters should list after auto merge");
            assert_eq!(clusters_after_auto.len(), 1);

            complete_speaker_output(
                &infra,
                &segment_ids[2],
                speaker_analysis_output_for_segment(
                    "speaker-resolution",
                    segment_ids[2].id,
                    "speaker_00",
                    &[0.70, 0.71],
                    None,
                ),
            )
            .await;
            let clusters_after_suggestion = infra
                .list_speaker_clusters_for_session("speaker-resolution")
                .await
                .expect("clusters should list after suggestion");
            assert_eq!(clusters_after_suggestion.len(), 2);
            assert_eq!(
                clusters_after_suggestion[1].suggested_merge_target_cluster_id,
                Some(clusters_after_suggestion[0].id)
            );
            assert!(clusters_after_suggestion[1]
                .suggested_merge_score
                .is_some_and(|score| score >= 0.68));

            complete_speaker_output(
                &infra,
                &segment_ids[3],
                speaker_analysis_output_for_segment(
                    "speaker-resolution",
                    segment_ids[3].id,
                    "speaker_00",
                    &[0.92, 0.39],
                    None,
                ),
            )
            .await;
            let clusters_after_ambiguous = infra
                .list_speaker_clusters_for_session("speaker-resolution")
                .await
                .expect("clusters should list after ambiguous candidate");
            assert_eq!(clusters_after_ambiguous.len(), 3);
        });
    }

    #[test]
    fn speaker_analysis_admission_backfills_without_transcription() {
        run_async_test(async {
            let dir = TestDir::new("speaker-analysis-backfill");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let segment = NewAudioSegment::new(
                AudioSegmentSourceKind::Microphone,
                "speaker-backfill",
                1,
                "/tmp/speaker-backfill.m4a",
                "2026-04-12T10:00:00Z",
                "2026-04-12T10:01:00Z",
            );
            let speaker_payload = serde_json::to_string(&SpeakerAnalysisJobPayload::new(
                "mock_speaker",
                Some("voice-model".to_string()),
            ))
            .expect("speaker payload should serialize");
            let outcome = infra
                .upsert_audio_segment_and_maybe_enqueue_processing(
                    &segment,
                    &AudioSegmentTranscriptionAdmission::disabled(),
                    &AudioSegmentSpeakerAnalysisAdmission::available(speaker_payload.clone()),
                    &SystemAudioSpeechActivityAdmission::disabled(),
                )
                .await
                .expect("segment should persist with speaker job");
            assert!(outcome.transcription_job.is_none());
            assert_eq!(
                outcome
                    .speaker_analysis_job
                    .as_ref()
                    .map(|job| job.processor.as_str()),
                Some(SPEAKER_ANALYSIS_PROCESSOR)
            );

            assert_eq!(
                infra
                    .backfill_missing_speaker_analysis_jobs(
                        &AudioSegmentSpeakerAnalysisAdmission::available(speaker_payload)
                    )
                    .await
                    .expect("speaker backfill should be idempotent"),
                0
            );
        });
    }

    #[test]
    fn speaker_analysis_backfill_skips_missing_audio_files() {
        run_async_test(async {
            let dir = TestDir::new("speaker-analysis-backfill-missing-file");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let missing_file = dir.path().join("missing-speaker.m4a");
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "speaker-missing-file",
                    1,
                    missing_file.to_string_lossy().to_string(),
                    "2026-04-12T10:00:00Z",
                    "2026-04-12T10:01:00Z",
                ))
                .await
                .expect("audio segment should persist");
            let payload = serde_json::to_string(&SpeakerAnalysisJobPayload::new(
                "mock_speaker",
                Some("voice-model".to_string()),
            ))
            .expect("speaker payload should serialize");

            assert_eq!(
                infra
                    .backfill_missing_speaker_analysis_jobs(
                        &AudioSegmentSpeakerAnalysisAdmission::available(payload)
                    )
                    .await
                    .expect("backfill should skip missing files"),
                0
            );

            let jobs = infra
                .list_processing_jobs_for_subject(&ProcessingSubject::audio_segment(segment.id))
                .await
                .expect("jobs should list");
            assert!(jobs.is_empty());
        });
    }

    #[test]
    fn transcript_text_attaches_to_speaker_turns_in_either_completion_order() {
        run_async_test(async {
            let dir = TestDir::new("speaker-transcript-alignment");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            async fn insert_segment(infra: &AppInfra, suffix: &str) -> AudioSegment {
                infra
                    .upsert_audio_segment(&NewAudioSegment::new(
                        AudioSegmentSourceKind::Microphone,
                        "speaker-transcript",
                        if suffix == "a" { 1 } else { 2 },
                        format!("/tmp/speaker-transcript-{suffix}.m4a"),
                        "2026-04-12T10:00:00Z",
                        "2026-04-12T10:01:00Z",
                    ))
                    .await
                    .expect("segment should insert")
            }

            async fn complete_transcription(infra: &AppInfra, segment: &AudioSegment) {
                let payload = serde_json::to_string(&AudioTranscriptionJobPayload::new(
                    "local_whisper",
                    Some("base".to_string()),
                    "auto",
                ))
                .expect("transcription payload should encode");
                let job = infra
                    .enqueue_processing_job(
                        &ProcessingJobDraft::for_audio_segment_transcription(segment.id)
                            .with_payload_json(payload),
                    )
                    .await
                    .expect("transcription job should enqueue");
                infra
                    .claim_queued_processing_job(job.id)
                    .await
                    .expect("transcription job should claim")
                    .expect("claimed transcription job should exist");
                let metadata = audio_transcription::TranscriptionMetadata {
                    provider: "local_whisper".to_string(),
                    model_id: Some("base".to_string()),
                    language: "auto".to_string(),
                    segments: vec![audio_transcription::TranscriptionSegment {
                        start_ms: 0,
                        end_ms: 1_000,
                        text: "hello world".to_string(),
                        confidence: None,
                    }],
                    words: vec![
                        audio_transcription::TranscriptionWord {
                            start_ms: 0,
                            end_ms: 400,
                            text: "hello".to_string(),
                            confidence: None,
                        },
                        audio_transcription::TranscriptionWord {
                            start_ms: 450,
                            end_ms: 900,
                            text: "world".to_string(),
                            confidence: None,
                        },
                    ],
                    provenance: Default::default(),
                };
                infra
                    .complete_processing_job(
                        job.id,
                        &ProcessingResultDraft::new().with_structured_payload_json(
                            serde_json::to_string(&metadata).unwrap(),
                        ),
                    )
                    .await
                    .expect("transcription should complete");
            }

            let speaker_first = insert_segment(&infra, "a").await;
            complete_speaker_output(
                &infra,
                &speaker_first,
                speaker_analysis_output_for_segment(
                    "speaker-transcript",
                    speaker_first.id,
                    "speaker_00",
                    &[1.0, 0.0],
                    None,
                ),
            )
            .await;
            complete_transcription(&infra, &speaker_first).await;
            let turn = infra
                .list_speaker_turns_for_audio_segment(speaker_first.id)
                .await
                .expect("turns should list")
                .pop()
                .expect("turn should exist");
            assert_eq!(turn.transcript_text.as_deref(), Some("hello world"));

            let transcription_first = insert_segment(&infra, "b").await;
            complete_transcription(&infra, &transcription_first).await;
            complete_speaker_output(
                &infra,
                &transcription_first,
                speaker_analysis_output_for_segment(
                    "speaker-transcript",
                    transcription_first.id,
                    "speaker_00",
                    &[0.0, 1.0],
                    None,
                ),
            )
            .await;
            let turn = infra
                .list_speaker_turns_for_audio_segment(transcription_first.id)
                .await
                .expect("turns should list")
                .pop()
                .expect("turn should exist");
            assert_eq!(turn.transcript_text.as_deref(), Some("hello world"));
        });
    }

    #[test]
    fn running_ocr_model_keys_include_only_running_jobs() {
        run_async_test(async {
            let dir = TestDir::new("ocr-model-keys");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let queued_frame = infra
                .insert_frame(&test_frame("ocr-model-keys", "queued.png"))
                .await
                .expect("queued frame should insert");
            let running_frame = infra
                .insert_frame(&test_frame("ocr-model-keys", "running.png"))
                .await
                .expect("running frame should insert");
            let completed_frame = infra
                .insert_frame(&test_frame("ocr-model-keys", "completed.png"))
                .await
                .expect("completed frame should insert");

            infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_frame_ocr(queued_frame.id).with_payload_json(
                        "{\"provider\":\"tesseract\",\"modelId\":\"tesseract-5.5.2\"}",
                    ),
                )
                .await
                .expect("queued ocr job should insert");
            let running = infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_frame_ocr(running_frame.id).with_payload_json(
                        "{\"provider\":\"paddleocr\",\"modelId\":\"paddleocr-en-v5\"}",
                    ),
                )
                .await
                .expect("running ocr job should insert");
            infra
                .claim_queued_processing_job(running.id)
                .await
                .expect("running ocr job should claim")
                .expect("running ocr job should exist");
            let completed = infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_frame_ocr(completed_frame.id).with_payload_json(
                        "{\"provider\":\"tesseract\",\"modelId\":\"completed-model\"}",
                    ),
                )
                .await
                .expect("completed ocr job should insert");
            infra
                .claim_queued_processing_job(completed.id)
                .await
                .expect("completed ocr job should claim")
                .expect("completed ocr job should exist");
            infra
                .complete_processing_job(completed.id, &ProcessingResultDraft::new())
                .await
                .expect("completed ocr job should complete");

            let keys = infra
                .list_running_ocr_model_keys()
                .await
                .expect("ocr model keys should list");

            assert!(!keys.contains("tesseract/tesseract-5.5.2"));
            assert!(keys.contains("paddleocr/paddleocr-en-v5"));
            assert!(!keys.contains("tesseract/completed-model"));
        });
    }

    #[test]
    fn retarget_ocr_jobs_updates_queued_and_failed_but_not_running_jobs() {
        run_async_test(async {
            let dir = TestDir::new("ocr-retarget-model-keys");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let queued_frame = infra
                .insert_frame(&test_frame("ocr-retarget-model-keys", "queued.png"))
                .await
                .expect("queued frame should insert");
            let failed_frame = infra
                .insert_frame(&test_frame("ocr-retarget-model-keys", "failed.png"))
                .await
                .expect("failed frame should insert");
            let running_frame = infra
                .insert_frame(&test_frame("ocr-retarget-model-keys", "running.png"))
                .await
                .expect("running frame should insert");
            let old_payload =
                "{\"provider\":\"tesseract\",\"modelId\":\"tesseract-5.5.2\",\"language\":\"eng\"}";

            let queued = infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_frame_ocr(queued_frame.id)
                        .with_payload_json(old_payload),
                )
                .await
                .expect("queued ocr job should insert");
            let failed = infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_frame_ocr(failed_frame.id)
                        .with_payload_json(old_payload),
                )
                .await
                .expect("failed ocr job should insert");
            infra
                .claim_queued_processing_job(failed.id)
                .await
                .expect("failed ocr job should claim")
                .expect("failed ocr job should exist");
            infra
                .mark_processing_job_failed(failed.id, Some("old failure"))
                .await
                .expect("failed ocr job should fail");
            let running = infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_frame_ocr(running_frame.id)
                        .with_payload_json(old_payload),
                )
                .await
                .expect("running ocr job should insert");
            infra
                .claim_queued_processing_job(running.id)
                .await
                .expect("running ocr job should claim")
                .expect("running ocr job should exist");

            let updated = infra
                .retarget_ocr_jobs_referencing_model_keys(
                    &BTreeSet::from(["tesseract/tesseract-5.5.2".to_string()]),
                    "apple_vision",
                    None,
                )
                .await
                .expect("ocr jobs should retarget");

            assert_eq!(updated, 2);
            for job_id in [queued.id, failed.id] {
                let job = infra
                    .get_processing_job(job_id)
                    .await
                    .expect("job should load")
                    .expect("job should exist");
                let payload = FrozenOcrPayload::from_payload_json(job.payload_json.as_deref())
                    .expect("payload should parse");
                assert_eq!(payload.provider, "apple_vision");
                assert_eq!(payload.model_id, None);
                assert_eq!(payload.language.as_deref(), Some("eng"));
            }
            let running = infra
                .get_processing_job(running.id)
                .await
                .expect("running job should load")
                .expect("running job should exist");
            let running_payload =
                FrozenOcrPayload::from_payload_json(running.payload_json.as_deref())
                    .expect("running payload should parse");
            assert_eq!(running_payload.provider, "tesseract");
            assert_eq!(running_payload.model_id.as_deref(), Some("tesseract-5.5.2"));
        });
    }

    #[test]
    fn ocr_model_cleanup_lock_blocks_direct_queued_job_claim_until_released() {
        run_async_test(async {
            let dir = TestDir::new("ocr-cleanup-lock-direct-claim");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let frame = infra
                .insert_frame(&test_frame("ocr-cleanup-lock-direct-claim", "queued.png"))
                .await
                .expect("frame should insert");
            let job = infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_frame_ocr(frame.id).with_payload_json(
                        "{\"provider\":\"tesseract\",\"modelId\":\"tesseract-5.5.2\"}",
                    ),
                )
                .await
                .expect("ocr job should insert");
            let lock = infra
                .acquire_ocr_model_cleanup_locks(&BTreeSet::from([
                    "tesseract/tesseract-5.5.2".to_string()
                ]))
                .await
                .expect("cleanup lock should acquire");

            let blocked = infra
                .claim_queued_processing_job(job.id)
                .await
                .expect("claim should not fail");
            assert!(blocked.is_none());
            let queued = infra
                .get_processing_job(job.id)
                .await
                .expect("job should load")
                .expect("job should exist");
            assert_eq!(queued.status, ProcessingJobStatus::Queued);

            infra
                .release_processing_model_cleanup_locks(&lock)
                .await
                .expect("cleanup lock should release");
            let claimed = infra
                .claim_queued_processing_job(job.id)
                .await
                .expect("claim should succeed after release")
                .expect("job should claim");
            assert_eq!(claimed.status, ProcessingJobStatus::Running);
        });
    }

    #[test]
    fn ocr_model_cleanup_lock_makes_next_claim_skip_locked_model_jobs() {
        run_async_test(async {
            let dir = TestDir::new("ocr-cleanup-lock-next-claim");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let locked_frame = infra
                .insert_frame(&test_frame("ocr-cleanup-lock-next-claim", "locked.png"))
                .await
                .expect("locked frame should insert");
            let unlocked_frame = infra
                .insert_frame(&test_frame("ocr-cleanup-lock-next-claim", "unlocked.png"))
                .await
                .expect("unlocked frame should insert");
            let locked_job = infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_frame_ocr(locked_frame.id).with_payload_json(
                        "{\"provider\":\"tesseract\",\"modelId\":\"tesseract-5.5.2\"}",
                    ),
                )
                .await
                .expect("locked ocr job should insert");
            let unlocked_job = infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_frame_ocr(unlocked_frame.id).with_payload_json(
                        "{\"provider\":\"paddleocr\",\"modelId\":\"paddleocr-en-v5\"}",
                    ),
                )
                .await
                .expect("unlocked ocr job should insert");
            let lock = infra
                .acquire_ocr_model_cleanup_locks(&BTreeSet::from([
                    "tesseract/tesseract-5.5.2".to_string()
                ]))
                .await
                .expect("cleanup lock should acquire");

            let claimed = infra
                .process_next_processing_job_for_processor(OCR_PROCESSOR)
                .await
                .expect("next job should process")
                .expect("unlocked job should be claimable");
            let claimed_job_id = match claimed {
                ProcessingJobRunOutcome::Completed(completion) => completion.job.id,
                ProcessingJobRunOutcome::Failed(job) => job.id,
            };
            assert_eq!(claimed_job_id, unlocked_job.id);
            let locked = infra
                .get_processing_job(locked_job.id)
                .await
                .expect("locked job should load")
                .expect("locked job should exist");
            assert_eq!(locked.status, ProcessingJobStatus::Queued);

            infra
                .release_processing_model_cleanup_locks(&lock)
                .await
                .expect("cleanup lock should release");
        });
    }

    #[test]
    fn startup_stale_model_cleanup_clear_preserves_fresh_locks() {
        run_async_test(async {
            let dir = TestDir::new("startup-stale-cleanup-clear");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            // A lock left orphaned by a prior crashed session: backdated well past the threshold.
            let stale_lock = infra
                .acquire_ocr_model_cleanup_locks(&BTreeSet::from([
                    "tesseract/tesseract-5.5.2".to_string()
                ]))
                .await
                .expect("stale cleanup lock should acquire");
            sqlx::query(
                "UPDATE processing_model_cleanup_locks \
                 SET created_at = datetime('now', '-1 day') \
                 WHERE model_key = ?1",
            )
            .bind("tesseract/tesseract-5.5.2")
            .execute(infra.pool())
            .await
            .expect("stale lock created_at should backdate");

            // A lock a live model-deletion command just acquired (created_at = now).
            let fresh_lock = infra
                .acquire_ocr_model_cleanup_locks(&BTreeSet::from([
                    "paddleocr/paddleocr-en-v5".to_string()
                ]))
                .await
                .expect("fresh cleanup lock should acquire");

            let cleared = infra
                .processing()
                .clear_stale_model_cleanup_locks(processing::MODEL_CLEANUP_LOCK_STALE_AFTER_SECONDS)
                .await
                .expect("stale-only clear should run");
            assert_eq!(cleared, 1, "only the stale lock should be cleared");

            let stale_remaining: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM processing_model_cleanup_locks WHERE model_key = ?1",
            )
            .bind("tesseract/tesseract-5.5.2")
            .fetch_one(infra.pool())
            .await
            .expect("stale lock count should query");
            assert_eq!(stale_remaining, 0, "stale orphaned lock should be gone");

            let fresh_remaining: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM processing_model_cleanup_locks WHERE model_key = ?1",
            )
            .bind("paddleocr/paddleocr-en-v5")
            .fetch_one(infra.pool())
            .await
            .expect("fresh lock count should query");
            assert_eq!(
                fresh_remaining, 1,
                "freshly acquired live lock should survive"
            );

            infra
                .release_processing_model_cleanup_locks(&fresh_lock)
                .await
                .expect("fresh cleanup lock should release");
            // The stale lock was already cleared; releasing it is a no-op but must not error.
            infra
                .release_processing_model_cleanup_locks(&stale_lock)
                .await
                .expect("releasing an already-cleared lock should be a no-op");
        });
    }

    #[test]
    fn malformed_ocr_payload_is_isolated_to_that_job_during_next_claim() {
        run_async_test(async {
            let dir = TestDir::new("ocr-malformed-payload-next-claim");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let malformed_frame = infra
                .insert_frame(&test_frame(
                    "ocr-malformed-payload-next-claim",
                    "malformed.png",
                ))
                .await
                .expect("malformed frame should insert");
            let valid_frame = infra
                .insert_frame(&test_frame("ocr-malformed-payload-next-claim", "valid.png"))
                .await
                .expect("valid frame should insert");
            let malformed_job = infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_frame_ocr(malformed_frame.id)
                        .with_payload_json("{not valid json"),
                )
                .await
                .expect("malformed ocr job should insert");
            let valid_job = infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_frame_ocr(valid_frame.id).with_payload_json(
                        "{\"provider\":\"tesseract\",\"modelId\":\"tesseract-5.5.2\"}",
                    ),
                )
                .await
                .expect("valid ocr job should insert");

            let first = infra
                .process_next_processing_job_for_processor(OCR_PROCESSOR)
                .await
                .expect("malformed payload should be claimed and failed by the processor")
                .expect("malformed job should run");
            let first_job_id = match first {
                ProcessingJobRunOutcome::Completed(completion) => completion.job.id,
                ProcessingJobRunOutcome::Failed(job) => job.id,
            };
            assert_eq!(first_job_id, malformed_job.id);

            // The malformed job's payload failure repeats each attempt, but its retry is deferred
            // by a backoff window, so later queued OCR work drains in the gap instead of being
            // starved. Drain OCR jobs until the valid job runs.
            let mut valid_job_ran = false;
            for _ in 0..16 {
                let Some(outcome) = infra
                    .process_next_processing_job_for_processor(OCR_PROCESSOR)
                    .await
                    .expect("later queued jobs should keep draining")
                else {
                    break;
                };
                let job_id = match outcome {
                    ProcessingJobRunOutcome::Completed(completion) => completion.job.id,
                    ProcessingJobRunOutcome::Failed(job) => job.id,
                };
                if job_id == valid_job.id {
                    valid_job_ran = true;
                    break;
                }
            }
            assert!(
                valid_job_ran,
                "the valid OCR job must still drain even while the malformed job is bounded-retried"
            );

            // The malformed job is deferred (still queued, awaiting its backoff) rather than
            // re-claimed every cycle — exactly what lets the valid job drain promptly instead
            // of waiting out the malformed job's attempt cap.
            let malformed_after = infra
                .get_processing_job(malformed_job.id)
                .await
                .expect("malformed job should be readable")
                .expect("malformed job should exist");
            assert_eq!(malformed_after.status, ProcessingJobStatus::Queued);
            assert_eq!(malformed_after.attempt_count, 1);
        });
    }

    #[test]
    fn running_audio_transcription_model_keys_include_only_running_jobs() {
        run_async_test(async {
            let dir = TestDir::new("audio-transcription-model-keys");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let queued_segment = NewAudioSegment::new(
                AudioSegmentSourceKind::Microphone,
                "audio-model-keys",
                1,
                "/tmp/audio-model-keys-queued.m4a",
                "2026-04-12T10:00:00Z",
                "2026-04-12T10:01:00Z",
            );
            let running_segment = NewAudioSegment::new(
                AudioSegmentSourceKind::Microphone,
                "audio-model-keys",
                2,
                "/tmp/audio-model-keys-running.m4a",
                "2026-04-12T10:01:00Z",
                "2026-04-12T10:02:00Z",
            );
            let completed_segment = NewAudioSegment::new(
                AudioSegmentSourceKind::Microphone,
                "audio-model-keys",
                3,
                "/tmp/audio-model-keys-completed.m4a",
                "2026-04-12T10:02:00Z",
                "2026-04-12T10:03:00Z",
            );
            let queued_segment = infra
                .upsert_audio_segment(&queued_segment)
                .await
                .expect("queued segment should insert");
            let running_segment = infra
                .upsert_audio_segment(&running_segment)
                .await
                .expect("running segment should insert");
            let completed_segment = infra
                .upsert_audio_segment(&completed_segment)
                .await
                .expect("completed segment should insert");

            let queued_payload = serde_json::to_string(&AudioTranscriptionJobPayload::new(
                "local_whisper",
                Some("base".to_string()),
                "auto",
            ))
            .expect("queued payload should serialize");
            let running_payload = serde_json::to_string(&AudioTranscriptionJobPayload::new(
                "parakeet",
                Some("parakeet-tdt-0.6b-v3-onnx".to_string()),
                "auto",
            ))
            .expect("running payload should serialize");
            let completed_payload = serde_json::to_string(&AudioTranscriptionJobPayload::new(
                "local_whisper",
                Some("completed-model".to_string()),
                "auto",
            ))
            .expect("completed payload should serialize");

            infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_audio_segment_transcription(queued_segment.id)
                        .with_payload_json(queued_payload),
                )
                .await
                .expect("queued transcription job should insert");
            let running = infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_audio_segment_transcription(running_segment.id)
                        .with_payload_json(running_payload),
                )
                .await
                .expect("running transcription job should insert");
            infra
                .claim_queued_processing_job(running.id)
                .await
                .expect("running transcription job should claim")
                .expect("running transcription job should exist");
            let completed = infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_audio_segment_transcription(completed_segment.id)
                        .with_payload_json(completed_payload),
                )
                .await
                .expect("completed transcription job should insert");
            infra
                .claim_queued_processing_job(completed.id)
                .await
                .expect("completed transcription job should claim")
                .expect("completed transcription job should exist");
            infra
                .complete_processing_job(completed.id, &ProcessingResultDraft::new())
                .await
                .expect("completed transcription job should complete");

            let keys = infra
                .list_running_audio_transcription_model_keys()
                .await
                .expect("transcription model keys should list");

            assert!(!keys.contains("local_whisper/base"));
            assert!(keys.contains("parakeet/parakeet-tdt-0.6b-v3-onnx"));
            assert!(!keys.contains("local_whisper/completed-model"));
        });
    }

    #[test]
    fn audio_transcription_model_cleanup_lock_blocks_direct_queued_job_claim_until_released() {
        run_async_test(async {
            let dir = TestDir::new("audio-cleanup-lock-direct-claim");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "audio-cleanup-lock-direct-claim",
                    1,
                    "/tmp/audio-cleanup-lock-direct-claim.m4a",
                    "2026-04-12T10:00:00Z",
                    "2026-04-12T10:01:00Z",
                ))
                .await
                .expect("segment should insert");
            let payload = serde_json::to_string(&AudioTranscriptionJobPayload::new(
                "local_whisper",
                Some("base".to_string()),
                "auto",
            ))
            .expect("payload should serialize");
            let job = infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_audio_segment_transcription(segment.id)
                        .with_payload_json(payload),
                )
                .await
                .expect("transcription job should insert");
            let lock = infra
                .acquire_audio_transcription_model_cleanup_locks(&BTreeSet::from([
                    "local_whisper/base".to_string(),
                ]))
                .await
                .expect("cleanup lock should acquire");

            let blocked = infra
                .claim_queued_processing_job(job.id)
                .await
                .expect("claim should not fail");
            assert!(blocked.is_none());
            let queued = infra
                .get_processing_job(job.id)
                .await
                .expect("job should load")
                .expect("job should exist");
            assert_eq!(queued.status, ProcessingJobStatus::Queued);

            infra
                .release_processing_model_cleanup_locks(&lock)
                .await
                .expect("cleanup lock should release");
            let claimed = infra
                .claim_queued_processing_job(job.id)
                .await
                .expect("claim should succeed after release")
                .expect("job should claim");
            assert_eq!(claimed.status, ProcessingJobStatus::Running);
        });
    }

    #[test]
    fn speaker_analysis_model_cleanup_lock_blocks_direct_queued_job_claim_until_released() {
        run_async_test(async {
            let dir = TestDir::new("speaker-cleanup-lock-direct-claim");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "speaker-cleanup-lock-direct-claim",
                    1,
                    "/tmp/speaker-cleanup-lock-direct-claim.m4a",
                    "2026-04-12T10:00:00Z",
                    "2026-04-12T10:01:00Z",
                ))
                .await
                .expect("segment should insert");
            let payload = serde_json::to_string(&SpeakerAnalysisJobPayload::new(
                "mock_speaker",
                Some("voice-model".to_string()),
            ))
            .expect("payload should serialize");
            let job = infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_audio_segment_speaker_analysis(segment.id)
                        .with_payload_json(payload),
                )
                .await
                .expect("speaker analysis job should insert");
            let lock = infra
                .acquire_speaker_analysis_model_cleanup_locks(&BTreeSet::from([
                    "mock_speaker/voice-model".to_string(),
                ]))
                .await
                .expect("cleanup lock should acquire");

            let blocked = infra
                .claim_queued_processing_job(job.id)
                .await
                .expect("claim should not fail");
            assert!(blocked.is_none());
            let queued = infra
                .get_processing_job(job.id)
                .await
                .expect("job should load")
                .expect("job should exist");
            assert_eq!(queued.status, ProcessingJobStatus::Queued);

            infra
                .release_processing_model_cleanup_locks(&lock)
                .await
                .expect("cleanup lock should release");
            let claimed = infra
                .claim_queued_processing_job(job.id)
                .await
                .expect("claim should succeed after release")
                .expect("job should claim");
            assert_eq!(claimed.status, ProcessingJobStatus::Running);
        });
    }

    #[test]
    fn speaker_analysis_model_cleanup_lock_makes_next_claim_skip_locked_model_jobs() {
        run_async_test(async {
            let dir = TestDir::new("speaker-cleanup-lock-next-claim");
            let infra = AppInfra::initialize_with_processing_registry(
                dir.path(),
                ProcessorRegistry::new().register(SuccessfulProcessingBackend::new(
                    SPEAKER_ANALYSIS_PROCESSOR,
                    "speaker analysis done",
                )),
            )
            .await
            .expect("app infra should initialize");
            let locked_segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "speaker-cleanup-lock-next-claim",
                    1,
                    "/tmp/speaker-cleanup-lock-next-claim-locked.m4a",
                    "2026-04-12T10:00:00Z",
                    "2026-04-12T10:01:00Z",
                ))
                .await
                .expect("locked segment should insert");
            let unlocked_segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "speaker-cleanup-lock-next-claim",
                    2,
                    "/tmp/speaker-cleanup-lock-next-claim-unlocked.m4a",
                    "2026-04-12T10:01:00Z",
                    "2026-04-12T10:02:00Z",
                ))
                .await
                .expect("unlocked segment should insert");
            let locked_payload = serde_json::to_string(&SpeakerAnalysisJobPayload::new(
                "mock_speaker",
                Some("voice-model".to_string()),
            ))
            .expect("locked payload should serialize");
            let unlocked_payload = serde_json::to_string(&SpeakerAnalysisJobPayload::new(
                "mock_speaker",
                Some("other-voice-model".to_string()),
            ))
            .expect("unlocked payload should serialize");
            let locked_job = infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_audio_segment_speaker_analysis(locked_segment.id)
                        .with_payload_json(locked_payload),
                )
                .await
                .expect("locked speaker analysis job should insert");
            let unlocked_job = infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_audio_segment_speaker_analysis(unlocked_segment.id)
                        .with_payload_json(unlocked_payload),
                )
                .await
                .expect("unlocked speaker analysis job should insert");
            let lock = infra
                .acquire_speaker_analysis_model_cleanup_locks(&BTreeSet::from([
                    "mock_speaker/voice-model".to_string(),
                ]))
                .await
                .expect("cleanup lock should acquire");

            let claimed = infra
                .process_next_processing_job_for_processor(SPEAKER_ANALYSIS_PROCESSOR)
                .await
                .expect("next job should process")
                .expect("unlocked job should be claimable");
            let claimed_job_id = match claimed {
                ProcessingJobRunOutcome::Completed(completion) => completion.job.id,
                ProcessingJobRunOutcome::Failed(job) => job.id,
            };
            assert_eq!(claimed_job_id, unlocked_job.id);
            let locked = infra
                .get_processing_job(locked_job.id)
                .await
                .expect("locked job should load")
                .expect("locked job should exist");
            assert_eq!(locked.status, ProcessingJobStatus::Queued);

            infra
                .release_processing_model_cleanup_locks(&lock)
                .await
                .expect("cleanup lock should release");
        });
    }

    #[test]
    fn active_speaker_analysis_model_keys_include_queued_and_running_jobs() {
        run_async_test(async {
            let dir = TestDir::new("speaker-active-model-keys");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let queued_segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "speaker-active-model-keys",
                    1,
                    "/tmp/speaker-active-model-keys-queued.m4a",
                    "2026-04-12T10:00:00Z",
                    "2026-04-12T10:01:00Z",
                ))
                .await
                .expect("queued segment should insert");
            let running_segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "speaker-active-model-keys",
                    2,
                    "/tmp/speaker-active-model-keys-running.m4a",
                    "2026-04-12T10:01:00Z",
                    "2026-04-12T10:02:00Z",
                ))
                .await
                .expect("running segment should insert");
            let completed_segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "speaker-active-model-keys",
                    3,
                    "/tmp/speaker-active-model-keys-completed.m4a",
                    "2026-04-12T10:02:00Z",
                    "2026-04-12T10:03:00Z",
                ))
                .await
                .expect("completed segment should insert");

            let queued_payload = serde_json::to_string(&SpeakerAnalysisJobPayload::new(
                "mock_speaker",
                Some("queued-model".to_string()),
            ))
            .expect("queued payload should serialize");
            let running_payload = serde_json::to_string(&SpeakerAnalysisJobPayload::new(
                "mock_speaker",
                Some("running-model".to_string()),
            ))
            .expect("running payload should serialize");
            let completed_payload = serde_json::to_string(&SpeakerAnalysisJobPayload::new(
                "mock_speaker",
                Some("completed-model".to_string()),
            ))
            .expect("completed payload should serialize");

            infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_audio_segment_speaker_analysis(queued_segment.id)
                        .with_payload_json(queued_payload),
                )
                .await
                .expect("queued speaker analysis job should insert");
            let running = infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_audio_segment_speaker_analysis(running_segment.id)
                        .with_payload_json(running_payload),
                )
                .await
                .expect("running speaker analysis job should insert");
            infra
                .claim_queued_processing_job(running.id)
                .await
                .expect("running speaker analysis job should claim")
                .expect("running speaker analysis job should exist");
            let completed = infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_audio_segment_speaker_analysis(completed_segment.id)
                        .with_payload_json(completed_payload),
                )
                .await
                .expect("completed speaker analysis job should insert");
            infra
                .claim_queued_processing_job(completed.id)
                .await
                .expect("completed speaker analysis job should claim")
                .expect("completed speaker analysis job should exist");
            infra
                .complete_processing_job(completed.id, &ProcessingResultDraft::new())
                .await
                .expect("completed speaker analysis job should complete");

            let keys = infra
                .list_active_speaker_analysis_model_keys()
                .await
                .expect("speaker analysis model keys should list");

            assert!(keys.contains("mock_speaker/queued-model"));
            assert!(keys.contains("mock_speaker/running-model"));
            assert!(!keys.contains("mock_speaker/completed-model"));
        });
    }

    #[test]
    fn startup_clears_stale_processing_model_cleanup_locks() {
        run_async_test(async {
            let dir = TestDir::new("stale-processing-model-cleanup-locks");
            let initial = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let lock = initial
                .acquire_ocr_model_cleanup_locks(&BTreeSet::from([
                    "tesseract/tesseract-5.5.2".to_string()
                ]))
                .await
                .expect("cleanup lock should acquire");
            assert_eq!(lock.acquired_model_keys.len(), 1);
            // Simulate a lock orphaned by a prior crashed session: by the time the
            // app restarts and deferred startup maintenance runs, an orphaned lock
            // is well past the stale threshold. Startup maintenance now clears only
            // stale locks (it runs while live model-deletion commands may hold a
            // fresh lock), so the orphaned lock must be backdated to be cleared.
            sqlx::query(
                "UPDATE processing_model_cleanup_locks \
                 SET created_at = datetime('now', '-1 day') \
                 WHERE model_key = ?1",
            )
            .bind("tesseract/tesseract-5.5.2")
            .execute(initial.pool())
            .await
            .expect("orphaned lock created_at should backdate");
            drop(initial);

            let recovered = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should reinitialize");
            let lock = recovered
                .acquire_ocr_model_cleanup_locks(&BTreeSet::from([
                    "tesseract/tesseract-5.5.2".to_string()
                ]))
                .await
                .expect("cleanup lock should acquire after restart");

            assert_eq!(lock.acquired_model_keys.len(), 1);
        });
    }

    #[test]
    fn retarget_audio_transcription_jobs_updates_queued_and_failed_but_not_running_jobs() {
        run_async_test(async {
            let dir = TestDir::new("audio-retarget-model-keys");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let queued_segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "audio-retarget-model-keys",
                    1,
                    "/tmp/audio-retarget-queued.m4a",
                    "2026-04-12T10:00:00Z",
                    "2026-04-12T10:01:00Z",
                ))
                .await
                .expect("queued segment should insert");
            let failed_segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "audio-retarget-model-keys",
                    2,
                    "/tmp/audio-retarget-failed.m4a",
                    "2026-04-12T10:01:00Z",
                    "2026-04-12T10:02:00Z",
                ))
                .await
                .expect("failed segment should insert");
            let running_segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "audio-retarget-model-keys",
                    3,
                    "/tmp/audio-retarget-running.m4a",
                    "2026-04-12T10:02:00Z",
                    "2026-04-12T10:03:00Z",
                ))
                .await
                .expect("running segment should insert");
            let old_payload = serde_json::to_string(&AudioTranscriptionJobPayload::new(
                "local_whisper",
                Some("base".to_string()),
                "auto",
            ))
            .expect("old payload should serialize");

            let queued = infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_audio_segment_transcription(queued_segment.id)
                        .with_payload_json(old_payload.clone()),
                )
                .await
                .expect("queued transcription job should insert");
            let failed = infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_audio_segment_transcription(failed_segment.id)
                        .with_payload_json(old_payload.clone()),
                )
                .await
                .expect("failed transcription job should insert");
            infra
                .claim_queued_processing_job(failed.id)
                .await
                .expect("failed transcription job should claim")
                .expect("failed transcription job should exist");
            infra
                .mark_processing_job_failed(failed.id, Some("old failure"))
                .await
                .expect("failed transcription job should fail");
            let running = infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_audio_segment_transcription(running_segment.id)
                        .with_payload_json(old_payload),
                )
                .await
                .expect("running transcription job should insert");
            infra
                .claim_queued_processing_job(running.id)
                .await
                .expect("running transcription job should claim")
                .expect("running transcription job should exist");

            let updated = infra
                .retarget_audio_transcription_jobs_referencing_model_keys(
                    &BTreeSet::from(["local_whisper/base".to_string()]),
                    "apple_speech_on_device",
                    None,
                )
                .await
                .expect("transcription jobs should retarget");

            assert_eq!(updated, 2);
            for job_id in [queued.id, failed.id] {
                let job = infra
                    .get_processing_job(job_id)
                    .await
                    .expect("job should load")
                    .expect("job should exist");
                let payload: AudioTranscriptionJobPayload =
                    serde_json::from_str(job.payload_json.as_deref().expect("payload"))
                        .expect("payload should parse");
                assert_eq!(payload.provider, "apple_speech_on_device");
                assert_eq!(payload.model_id, None);
                assert_eq!(payload.language, "auto");
            }
            let running = infra
                .get_processing_job(running.id)
                .await
                .expect("running job should load")
                .expect("running job should exist");
            let running_payload: AudioTranscriptionJobPayload =
                serde_json::from_str(running.payload_json.as_deref().expect("payload"))
                    .expect("running payload should parse");
            assert_eq!(running_payload.provider, "local_whisper");
            assert_eq!(running_payload.model_id.as_deref(), Some("base"));
        });
    }

    #[test]
    fn audio_segment_transcription_admission_skips_system_audio_and_missing_models_then_backfills()
    {
        run_async_test(async {
            let dir = TestDir::new("audio-segment-transcription-backfill");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let unavailable = AudioSegmentTranscriptionAdmission::unavailable();
            let microphone_file = write_existing_audio_placeholder(&dir.path().join("mic-1.m4a"));
            let microphone = NewAudioSegment::new(
                AudioSegmentSourceKind::Microphone,
                "mic-session",
                1,
                microphone_file,
                "2026-04-12T10:00:00Z",
                "2026-04-12T10:01:00Z",
            );
            let system_audio = NewAudioSegment::new(
                AudioSegmentSourceKind::SystemAudio,
                "system-session",
                1,
                "/tmp/system-1.m4a",
                "2026-04-12T10:00:00Z",
                "2026-04-12T10:01:00Z",
            );

            let persisted_mic = infra
                .upsert_audio_segment_and_maybe_enqueue_transcription(&microphone, &unavailable)
                .await
                .expect("missing model should not block segment persistence");
            let persisted_system = infra
                .upsert_audio_segment_and_maybe_enqueue_transcription(
                    &system_audio,
                    &AudioSegmentTranscriptionAdmission::available("{}"),
                )
                .await
                .expect("system audio should persist without v1 transcription job");

            assert!(persisted_mic.job.is_none());
            assert!(persisted_system.job.is_none());

            let payload = serde_json::to_string(&AudioTranscriptionJobPayload::new(
                "local_whisper",
                Some("base".to_string()),
                "auto",
            ))
            .expect("payload should serialize");
            let available = AudioSegmentTranscriptionAdmission::available(payload.clone());
            assert_eq!(
                infra
                    .backfill_missing_audio_transcription_jobs(&available)
                    .await
                    .expect("backfill should enqueue"),
                1
            );
            assert_eq!(
                infra
                    .backfill_missing_audio_transcription_jobs(&available)
                    .await
                    .expect("backfill should be idempotent"),
                0
            );

            let mic_jobs = infra
                .list_processing_jobs_for_subject(&ProcessingSubject::audio_segment(
                    persisted_mic.segment.id,
                ))
                .await
                .expect("mic jobs should list");
            assert_eq!(mic_jobs.len(), 1);
            assert_eq!(mic_jobs[0].payload_json.as_deref(), Some(payload.as_str()));
            let system_jobs = infra
                .list_processing_jobs_for_subject(&ProcessingSubject::audio_segment(
                    persisted_system.segment.id,
                ))
                .await
                .expect("system jobs should list");
            assert!(system_jobs.is_empty());
        });
    }

    #[test]
    fn audio_transcription_backfill_skips_missing_audio_files() {
        run_async_test(async {
            let dir = TestDir::new("audio-transcription-backfill-missing-file");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let missing_file = dir.path().join("missing-mic.m4a");
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "mic-missing-file",
                    1,
                    missing_file.to_string_lossy().to_string(),
                    "2026-04-12T10:00:00Z",
                    "2026-04-12T10:01:00Z",
                ))
                .await
                .expect("audio segment should persist");
            let payload = serde_json::to_string(&AudioTranscriptionJobPayload::new(
                "local_whisper",
                Some("base".to_string()),
                "auto",
            ))
            .expect("payload should serialize");

            assert_eq!(
                infra
                    .backfill_missing_audio_transcription_jobs(
                        &AudioSegmentTranscriptionAdmission::available(payload)
                    )
                    .await
                    .expect("backfill should skip missing files"),
                0
            );

            let jobs = infra
                .list_processing_jobs_for_subject(&ProcessingSubject::audio_segment(segment.id))
                .await
                .expect("jobs should list");
            assert!(jobs.is_empty());
        });
    }

    #[test]
    fn reprocess_audio_segment_transcription_requeues_terminal_microphone_job_with_current_payload()
    {
        run_async_test(async {
            let dir = TestDir::new("audio-segment-transcription-reprocess");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let original_payload = serde_json::to_string(&AudioTranscriptionJobPayload::new(
                "local_whisper",
                Some("base".to_string()),
                "auto",
            ))
            .expect("payload should serialize");
            let current_payload = serde_json::to_string(&AudioTranscriptionJobPayload::new(
                "local_whisper",
                Some("small".to_string()),
                "en",
            ))
            .expect("payload should serialize");
            let segment = NewAudioSegment::new(
                AudioSegmentSourceKind::Microphone,
                "mic-session",
                1,
                "/tmp/mic-1.m4a",
                "2026-04-12T10:00:00Z",
                "2026-04-12T10:01:00Z",
            );
            let committed = infra
                .upsert_audio_segment_and_maybe_enqueue_transcription(
                    &segment,
                    &AudioSegmentTranscriptionAdmission::available(original_payload),
                )
                .await
                .expect("segment and transcription job should commit");
            let job = committed.job.expect("transcription job should be enqueued");
            infra
                .claim_queued_processing_job(job.id)
                .await
                .expect("queued job should claim")
                .expect("queued job should exist");
            infra
                .complete_processing_job(
                    job.id,
                    &ProcessingResultDraft::new().with_result_text("old transcript"),
                )
                .await
                .expect("job should complete");
            assert!(infra
                .get_processing_result_for_job(job.id)
                .await
                .expect("result lookup should succeed")
                .is_some());

            let reprocessed = infra
                .reprocess_audio_segment_transcription(
                    committed.segment.id,
                    &AudioSegmentTranscriptionAdmission::available(current_payload.clone()),
                )
                .await
                .expect("terminal transcription should requeue");

            assert_eq!(
                reprocessed.outcome,
                AudioSegmentTranscriptionReprocessingOutcome::Requeued
            );
            assert_eq!(reprocessed.job.id, job.id);
            assert_eq!(reprocessed.job.status, ProcessingJobStatus::Queued);
            assert_eq!(
                reprocessed.job.payload_json.as_deref(),
                Some(current_payload.as_str())
            );
            assert!(infra
                .get_processing_result_for_job(job.id)
                .await
                .expect("result lookup should succeed")
                .is_none());
        });
    }

    #[test]
    fn rerun_system_audio_speech_activity_requeues_completed_transcription_with_current_payload() {
        run_async_test(async {
            let dir = TestDir::new("system-audio-speech-rerun-transcription");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let original_transcription_payload =
                serde_json::to_string(&AudioTranscriptionJobPayload::new(
                    "local_whisper",
                    Some("base".to_string()),
                    "auto",
                ))
                .expect("original transcription payload should serialize");
            let current_transcription_payload =
                serde_json::to_string(&AudioTranscriptionJobPayload::new(
                    "local_whisper",
                    Some("small".to_string()),
                    "en",
                ))
                .expect("current transcription payload should serialize");
            let original_speech_payload =
                serde_json::to_string(&SystemAudioSpeechActivityJobPayload {
                    detector: capture_types::AudioSpeechDetector::Webrtc,
                    transcription_payload: original_transcription_payload.clone(),
                    speaker_analysis_payload: None,
                })
                .expect("original speech payload should serialize");
            let current_speech_payload =
                serde_json::to_string(&SystemAudioSpeechActivityJobPayload {
                    detector: capture_types::AudioSpeechDetector::Webrtc,
                    transcription_payload: current_transcription_payload.clone(),
                    speaker_analysis_payload: None,
                })
                .expect("current speech payload should serialize");
            let segment = NewAudioSegment::new(
                AudioSegmentSourceKind::SystemAudio,
                "system-audio-session",
                1,
                "/tmp/system-audio-1.m4a",
                "2026-04-12T10:00:00Z",
                "2026-04-12T10:01:00Z",
            );
            let committed = infra
                .upsert_audio_segment_and_maybe_enqueue_processing(
                    &segment,
                    &AudioSegmentTranscriptionAdmission::disabled(),
                    &AudioSegmentSpeakerAnalysisAdmission::disabled(),
                    &SystemAudioSpeechActivityAdmission::available(original_speech_payload),
                )
                .await
                .expect("system-audio segment and speech job should commit");
            let speech_job = committed
                .system_audio_speech_activity_job
                .expect("speech job should enqueue");
            infra
                .claim_queued_processing_job(speech_job.id)
                .await
                .expect("speech job should claim")
                .expect("speech job should exist");
            infra
                .complete_processing_job(
                    speech_job.id,
                    &ProcessingResultDraft::new()
                        .with_structured_payload_json("{\"speechDetected\":true}"),
                )
                .await
                .expect("speech job should complete");
            let transcription_job = infra
                .list_processing_jobs_for_subject(&ProcessingSubject::audio_segment(
                    committed.segment.id,
                ))
                .await
                .expect("jobs should list")
                .into_iter()
                .find(|job| job.processor == AUDIO_TRANSCRIPTION_PROCESSOR)
                .expect("transcription job should enqueue");
            infra
                .claim_queued_processing_job(transcription_job.id)
                .await
                .expect("transcription job should claim")
                .expect("transcription job should exist");
            infra
                .complete_processing_job(
                    transcription_job.id,
                    &ProcessingResultDraft::new().with_result_text("old transcript"),
                )
                .await
                .expect("transcription job should complete");
            assert!(infra
                .get_processing_result_for_job(transcription_job.id)
                .await
                .expect("result lookup should succeed")
                .is_some());

            let reprocessed = infra
                .reprocess_system_audio_speech_activity(
                    committed.segment.id,
                    &SystemAudioSpeechActivityAdmission::available(current_speech_payload),
                )
                .await
                .expect("terminal speech job should requeue");
            assert_eq!(
                reprocessed.outcome,
                SystemAudioSpeechActivityReprocessingOutcome::Requeued
            );
            infra
                .claim_queued_processing_job(reprocessed.job.id)
                .await
                .expect("requeued speech job should claim")
                .expect("requeued speech job should exist");
            infra
                .complete_processing_job(
                    reprocessed.job.id,
                    &ProcessingResultDraft::new()
                        .with_structured_payload_json("{\"speechDetected\":true}"),
                )
                .await
                .expect("requeued speech job should complete");

            let jobs = infra
                .list_processing_jobs_for_subject(&ProcessingSubject::audio_segment(
                    committed.segment.id,
                ))
                .await
                .expect("jobs should list");
            let refreshed_transcription_job = jobs
                .iter()
                .find(|job| job.processor == AUDIO_TRANSCRIPTION_PROCESSOR)
                .expect("transcription job should exist");
            assert_eq!(refreshed_transcription_job.id, transcription_job.id);
            assert_eq!(
                refreshed_transcription_job.status,
                ProcessingJobStatus::Queued
            );
            assert_eq!(
                refreshed_transcription_job.payload_json.as_deref(),
                Some(current_transcription_payload.as_str())
            );
            assert!(infra
                .get_processing_result_for_job(transcription_job.id)
                .await
                .expect("result lookup should succeed")
                .is_none());
        });
    }

    #[test]
    fn rerun_system_audio_speech_activity_refreshes_queued_transcription_payload() {
        run_async_test(async {
            let dir = TestDir::new("system-audio-speech-rerun-queued-transcription");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let original_transcription_payload =
                serde_json::to_string(&AudioTranscriptionJobPayload::new(
                    "local_whisper",
                    Some("base".to_string()),
                    "auto",
                ))
                .expect("original transcription payload should serialize");
            let current_transcription_payload =
                serde_json::to_string(&AudioTranscriptionJobPayload::new(
                    "local_whisper",
                    Some("small".to_string()),
                    "en",
                ))
                .expect("current transcription payload should serialize");
            let original_speech_payload =
                serde_json::to_string(&SystemAudioSpeechActivityJobPayload {
                    detector: capture_types::AudioSpeechDetector::Webrtc,
                    transcription_payload: original_transcription_payload,
                    speaker_analysis_payload: None,
                })
                .expect("original speech payload should serialize");
            let current_speech_payload =
                serde_json::to_string(&SystemAudioSpeechActivityJobPayload {
                    detector: capture_types::AudioSpeechDetector::Webrtc,
                    transcription_payload: current_transcription_payload.clone(),
                    speaker_analysis_payload: None,
                })
                .expect("current speech payload should serialize");
            let segment = NewAudioSegment::new(
                AudioSegmentSourceKind::SystemAudio,
                "system-audio-queued-rerun-session",
                1,
                "/tmp/system-audio-queued-rerun-1.m4a",
                "2026-04-12T10:00:00Z",
                "2026-04-12T10:01:00Z",
            );
            let committed = infra
                .upsert_audio_segment_and_maybe_enqueue_processing(
                    &segment,
                    &AudioSegmentTranscriptionAdmission::disabled(),
                    &AudioSegmentSpeakerAnalysisAdmission::disabled(),
                    &SystemAudioSpeechActivityAdmission::available(original_speech_payload),
                )
                .await
                .expect("system-audio segment and speech job should commit");
            let speech_job = committed
                .system_audio_speech_activity_job
                .expect("speech job should enqueue");
            infra
                .claim_queued_processing_job(speech_job.id)
                .await
                .expect("speech job should claim")
                .expect("speech job should exist");
            infra
                .complete_processing_job(
                    speech_job.id,
                    &ProcessingResultDraft::new()
                        .with_structured_payload_json("{\"speechDetected\":true}"),
                )
                .await
                .expect("speech job should complete");
            let transcription_job = infra
                .list_processing_jobs_for_subject(&ProcessingSubject::audio_segment(
                    committed.segment.id,
                ))
                .await
                .expect("jobs should list")
                .into_iter()
                .find(|job| job.processor == AUDIO_TRANSCRIPTION_PROCESSOR)
                .expect("transcription job should enqueue");
            assert_eq!(transcription_job.status, ProcessingJobStatus::Queued);

            let reprocessed = infra
                .reprocess_system_audio_speech_activity(
                    committed.segment.id,
                    &SystemAudioSpeechActivityAdmission::available(current_speech_payload),
                )
                .await
                .expect("terminal speech job should requeue");
            infra
                .claim_queued_processing_job(reprocessed.job.id)
                .await
                .expect("requeued speech job should claim")
                .expect("requeued speech job should exist");
            infra
                .complete_processing_job(
                    reprocessed.job.id,
                    &ProcessingResultDraft::new()
                        .with_structured_payload_json("{\"speechDetected\":true}"),
                )
                .await
                .expect("requeued speech job should complete");

            let jobs = infra
                .list_processing_jobs_for_subject(&ProcessingSubject::audio_segment(
                    committed.segment.id,
                ))
                .await
                .expect("jobs should list");
            let refreshed_transcription_job = jobs
                .iter()
                .find(|job| job.processor == AUDIO_TRANSCRIPTION_PROCESSOR)
                .expect("transcription job should exist");
            assert_eq!(refreshed_transcription_job.id, transcription_job.id);
            assert_eq!(
                refreshed_transcription_job.status,
                ProcessingJobStatus::Queued
            );
            assert_eq!(
                refreshed_transcription_job.payload_json.as_deref(),
                Some(current_transcription_payload.as_str())
            );
        });
    }

    #[test]
    fn no_speech_system_audio_rerun_clears_queued_downstream_jobs() {
        run_async_test(async {
            let dir = TestDir::new("system-audio-no-speech-clears-queued-downstream");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let transcription_payload = serde_json::to_string(&AudioTranscriptionJobPayload::new(
                "local_whisper",
                Some("base".to_string()),
                "auto",
            ))
            .expect("transcription payload should serialize");
            let speech_payload = serde_json::to_string(&SystemAudioSpeechActivityJobPayload {
                detector: capture_types::AudioSpeechDetector::Webrtc,
                transcription_payload,
                speaker_analysis_payload: None,
            })
            .expect("speech payload should serialize");
            let segment = NewAudioSegment::new(
                AudioSegmentSourceKind::SystemAudio,
                "system-audio-no-speech-clear-session",
                1,
                "/tmp/system-audio-no-speech-clear-1.m4a",
                "2026-04-12T10:00:00Z",
                "2026-04-12T10:01:00Z",
            );
            let committed = infra
                .upsert_audio_segment_and_maybe_enqueue_processing(
                    &segment,
                    &AudioSegmentTranscriptionAdmission::disabled(),
                    &AudioSegmentSpeakerAnalysisAdmission::disabled(),
                    &SystemAudioSpeechActivityAdmission::available(speech_payload.clone()),
                )
                .await
                .expect("system-audio segment and speech job should commit");
            let speech_job = committed
                .system_audio_speech_activity_job
                .expect("speech job should enqueue");
            infra
                .claim_queued_processing_job(speech_job.id)
                .await
                .expect("speech job should claim")
                .expect("speech job should exist");
            infra
                .complete_processing_job(
                    speech_job.id,
                    &ProcessingResultDraft::new()
                        .with_structured_payload_json("{\"speechDetected\":true}"),
                )
                .await
                .expect("speech job should complete");
            let queued_speaker_job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_audio_segment_speaker_analysis(
                    committed.segment.id,
                ))
                .await
                .expect("speaker job should enqueue");
            assert_eq!(queued_speaker_job.status, ProcessingJobStatus::Queued);

            let reprocessed = infra
                .reprocess_system_audio_speech_activity(
                    committed.segment.id,
                    &SystemAudioSpeechActivityAdmission::available(speech_payload),
                )
                .await
                .expect("terminal speech job should requeue");
            infra
                .claim_queued_processing_job(reprocessed.job.id)
                .await
                .expect("requeued speech job should claim")
                .expect("requeued speech job should exist");
            infra
                .complete_processing_job(
                    reprocessed.job.id,
                    &ProcessingResultDraft::new()
                        .with_structured_payload_json("{\"speechDetected\":false}"),
                )
                .await
                .expect("no-speech rerun should complete");

            let jobs = infra
                .list_processing_jobs_for_subject(&ProcessingSubject::audio_segment(
                    committed.segment.id,
                ))
                .await
                .expect("jobs should list");
            assert!(jobs
                .iter()
                .all(|job| job.processor != AUDIO_TRANSCRIPTION_PROCESSOR));
            assert!(jobs
                .iter()
                .all(|job| job.processor != SPEAKER_ANALYSIS_PROCESSOR));
        });
    }

    #[test]
    fn no_speech_system_audio_rerun_preserves_running_transcription_job() {
        run_async_test(async {
            let dir = TestDir::new("system-audio-no-speech-preserves-running-transcription");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let transcription_payload = serde_json::to_string(&AudioTranscriptionJobPayload::new(
                "local_whisper",
                Some("base".to_string()),
                "auto",
            ))
            .expect("transcription payload should serialize");
            let speech_payload = serde_json::to_string(&SystemAudioSpeechActivityJobPayload {
                detector: capture_types::AudioSpeechDetector::Webrtc,
                transcription_payload: transcription_payload.clone(),
                speaker_analysis_payload: None,
            })
            .expect("speech payload should serialize");
            let segment = NewAudioSegment::new(
                AudioSegmentSourceKind::SystemAudio,
                "system-audio-no-speech-session",
                1,
                "/tmp/system-audio-no-speech-1.m4a",
                "2026-04-12T10:00:00Z",
                "2026-04-12T10:01:00Z",
            );
            let committed = infra
                .upsert_audio_segment_and_maybe_enqueue_processing(
                    &segment,
                    &AudioSegmentTranscriptionAdmission::disabled(),
                    &AudioSegmentSpeakerAnalysisAdmission::disabled(),
                    &SystemAudioSpeechActivityAdmission::available(speech_payload.clone()),
                )
                .await
                .expect("system-audio segment and speech job should commit");
            let speech_job = committed
                .system_audio_speech_activity_job
                .expect("speech job should enqueue");
            infra
                .claim_queued_processing_job(speech_job.id)
                .await
                .expect("speech job should claim")
                .expect("speech job should exist");
            infra
                .complete_processing_job(
                    speech_job.id,
                    &ProcessingResultDraft::new()
                        .with_structured_payload_json("{\"speechDetected\":true}"),
                )
                .await
                .expect("speech job should complete");

            let transcription_job = infra
                .list_processing_jobs_for_subject(&ProcessingSubject::audio_segment(
                    committed.segment.id,
                ))
                .await
                .expect("jobs should list")
                .into_iter()
                .find(|job| job.processor == AUDIO_TRANSCRIPTION_PROCESSOR)
                .expect("transcription job should enqueue");
            infra
                .claim_queued_processing_job(transcription_job.id)
                .await
                .expect("transcription job should claim")
                .expect("transcription job should exist");

            let reprocessed = infra
                .reprocess_system_audio_speech_activity(
                    committed.segment.id,
                    &SystemAudioSpeechActivityAdmission::available(speech_payload),
                )
                .await
                .expect("terminal speech job should requeue");
            infra
                .claim_queued_processing_job(reprocessed.job.id)
                .await
                .expect("requeued speech job should claim")
                .expect("requeued speech job should exist");
            infra
                .complete_processing_job(
                    reprocessed.job.id,
                    &ProcessingResultDraft::new()
                        .with_structured_payload_json("{\"speechDetected\":false}"),
                )
                .await
                .expect("no-speech rerun should complete");

            let jobs = infra
                .list_processing_jobs_for_subject(&ProcessingSubject::audio_segment(
                    committed.segment.id,
                ))
                .await
                .expect("jobs should list");
            let preserved_transcription_job = jobs
                .iter()
                .find(|job| job.id == transcription_job.id)
                .expect("running transcription job should be preserved");
            assert_eq!(
                preserved_transcription_job.status,
                ProcessingJobStatus::Running
            );
            assert_eq!(
                preserved_transcription_job.payload_json.as_deref(),
                Some(transcription_payload.as_str())
            );
        });
    }

    #[test]
    fn completed_transcription_requeues_failed_speaker_analysis_with_current_payload() {
        run_async_test(async {
            let dir = TestDir::new("speaker-analysis-requeue-after-transcription");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "speaker-requeue-session",
                    1,
                    "/tmp/speaker-requeue.m4a",
                    "2026-04-12T10:00:00Z",
                    "2026-04-12T10:01:00Z",
                ))
                .await
                .expect("audio segment should insert");

            let old_speaker_payload = serde_json::to_string(&SpeakerAnalysisJobPayload::new(
                "sherpa_onnx",
                Some("old-speaker-model".to_string()),
            ))
            .expect("old speaker payload should serialize");
            let failed_speaker_job = infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_audio_segment_speaker_analysis(segment.id)
                        .with_payload_json(old_speaker_payload),
                )
                .await
                .expect("speaker job should enqueue");
            infra
                .claim_queued_processing_job(failed_speaker_job.id)
                .await
                .expect("speaker job should claim")
                .expect("speaker job should exist");
            infra
                .mark_processing_job_failed(failed_speaker_job.id, Some("model missing"))
                .await
                .expect("speaker job should fail");

            let mut speaker_payload = SpeakerAnalysisJobPayload::new(
                "sherpa_onnx",
                Some("pyannote-3.0-nemo-titanet-small".to_string()),
            );
            speaker_payload.recognize_people = true;
            let speaker_payload_value =
                serde_json::to_value(&speaker_payload).expect("speaker payload should encode");
            let mut transcription_payload = AudioTranscriptionJobPayload::new(
                "local_whisper",
                Some("base".to_string()),
                "auto",
            );
            transcription_payload.options.insert(
                SPEAKER_ANALYSIS_PAYLOAD_OPTION_KEY.to_string(),
                speaker_payload_value,
            );
            let transcription_payload_json = serde_json::to_string(&transcription_payload)
                .expect("transcription payload should serialize");
            let transcription_job = infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_audio_segment_transcription(segment.id)
                        .with_payload_json(transcription_payload_json),
                )
                .await
                .expect("transcription job should enqueue");
            infra
                .claim_queued_processing_job(transcription_job.id)
                .await
                .expect("transcription job should claim")
                .expect("transcription job should exist");
            infra
                .complete_processing_job(
                    transcription_job.id,
                    &ProcessingResultDraft::new().with_result_text("new transcript"),
                )
                .await
                .expect("transcription completion should requeue speaker analysis");

            let jobs = infra
                .list_processing_jobs_for_subject(&ProcessingSubject::audio_segment(segment.id))
                .await
                .expect("jobs should list");
            let speaker_jobs = jobs
                .iter()
                .filter(|job| job.processor == SPEAKER_ANALYSIS_PROCESSOR)
                .collect::<Vec<_>>();
            assert_eq!(speaker_jobs.len(), 1);
            assert_eq!(speaker_jobs[0].id, failed_speaker_job.id);
            assert_eq!(speaker_jobs[0].status, ProcessingJobStatus::Queued);
            let actual_payload: SpeakerAnalysisJobPayload = serde_json::from_str(
                speaker_jobs[0]
                    .payload_json
                    .as_deref()
                    .expect("speaker payload should be present"),
            )
            .expect("speaker payload should decode");
            assert_eq!(actual_payload, speaker_payload);
            assert!(infra
                .get_processing_result_for_job(failed_speaker_job.id)
                .await
                .expect("speaker result lookup should succeed")
                .is_none());
        });
    }

    #[test]
    fn reclaimed_transcription_completion_rechains_speaker_analysis() {
        run_async_test(async {
            let dir = TestDir::new("reclaimed-transcription-rechains-speaker");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "reclaim-rechain-session",
                    1,
                    "/tmp/reclaim-rechain.m4a",
                    "2026-04-12T10:00:00Z",
                    "2026-04-12T10:01:00Z",
                ))
                .await
                .expect("audio segment should insert");

            let mut speaker_payload = SpeakerAnalysisJobPayload::new(
                "sherpa_onnx",
                Some("pyannote-3.0-nemo-titanet-small".to_string()),
            );
            speaker_payload.recognize_people = true;
            let speaker_payload_value =
                serde_json::to_value(&speaker_payload).expect("speaker payload should encode");
            let mut transcription_payload = AudioTranscriptionJobPayload::new(
                "local_whisper",
                Some("base".to_string()),
                "auto",
            );
            transcription_payload.options.insert(
                SPEAKER_ANALYSIS_PAYLOAD_OPTION_KEY.to_string(),
                speaker_payload_value,
            );
            let transcription_payload_json = serde_json::to_string(&transcription_payload)
                .expect("transcription payload should serialize");
            let transcription_job = infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_audio_segment_transcription(segment.id)
                        .with_payload_json(transcription_payload_json),
                )
                .await
                .expect("transcription job should enqueue");

            // Claim then abandon the transcription (quit/crash), leaving it running, and reclaim it.
            infra
                .claim_queued_processing_job(transcription_job.id)
                .await
                .expect("transcription job should claim")
                .expect("transcription job should exist");
            let summary = infra
                .reconcile_orphaned_processing_jobs()
                .await
                .expect("reclamation should succeed");
            assert_eq!(summary.requeued, 1);
            assert_eq!(summary.failed_on_ceiling, 0);
            let reclaimed = infra
                .get_processing_job(transcription_job.id)
                .await
                .expect("job should be readable")
                .expect("job should exist");
            assert_eq!(reclaimed.status, ProcessingJobStatus::Queued);

            // Re-run the reclaimed transcription to completion.
            infra
                .claim_queued_processing_job(transcription_job.id)
                .await
                .expect("reclaimed transcription should re-claim")
                .expect("reclaimed transcription should exist");
            infra
                .complete_processing_job(
                    transcription_job.id,
                    &ProcessingResultDraft::new().with_result_text("recovered transcript"),
                )
                .await
                .expect("reclaimed transcription completion should chain speaker analysis");

            // The reclaimed transcription's completion still chains speaker analysis, so a recovered
            // transcript also recovers its speaker labels.
            let jobs = infra
                .list_processing_jobs_for_subject(&ProcessingSubject::audio_segment(segment.id))
                .await
                .expect("jobs should list");
            let speaker_jobs = jobs
                .iter()
                .filter(|job| job.processor == SPEAKER_ANALYSIS_PROCESSOR)
                .collect::<Vec<_>>();
            assert_eq!(
                speaker_jobs.len(),
                1,
                "a reclaimed transcription should still enqueue its speaker analysis"
            );
            assert_eq!(speaker_jobs[0].status, ProcessingJobStatus::Queued);
            let actual_payload: SpeakerAnalysisJobPayload = serde_json::from_str(
                speaker_jobs[0]
                    .payload_json
                    .as_deref()
                    .expect("speaker payload should be present"),
            )
            .expect("speaker payload should decode");
            assert_eq!(actual_payload, speaker_payload);
        });
    }

    #[test]
    fn completed_transcription_does_not_requeue_completed_speaker_analysis() {
        run_async_test(async {
            let dir = TestDir::new("speaker-analysis-completed-before-transcription");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    AudioSegmentSourceKind::Microphone,
                    "speaker-completed-session",
                    1,
                    "/tmp/speaker-completed.m4a",
                    "2026-04-12T10:00:00Z",
                    "2026-04-12T10:01:00Z",
                ))
                .await
                .expect("audio segment should insert");

            let speaker_output = speaker_analysis_output_for_segment(
                "speaker-completed-session",
                segment.id,
                "speaker-1",
                &[0.1, 0.2],
                None,
            );
            let completed_speaker_job =
                complete_speaker_output(&infra, &segment, speaker_output).await;
            let completed_speaker_result = infra
                .get_processing_result_for_job(completed_speaker_job.id)
                .await
                .expect("speaker result lookup should succeed")
                .expect("speaker result should exist");

            let mut speaker_payload = SpeakerAnalysisJobPayload::new(
                "sherpa_onnx",
                Some("pyannote-3.0-nemo-titanet-small".to_string()),
            );
            speaker_payload.recognize_people = true;
            let speaker_payload_value =
                serde_json::to_value(&speaker_payload).expect("speaker payload should encode");
            let mut transcription_payload = AudioTranscriptionJobPayload::new(
                "local_whisper",
                Some("base".to_string()),
                "auto",
            );
            transcription_payload.options.insert(
                SPEAKER_ANALYSIS_PAYLOAD_OPTION_KEY.to_string(),
                speaker_payload_value,
            );
            let transcription_payload_json = serde_json::to_string(&transcription_payload)
                .expect("transcription payload should serialize");
            let transcription_job = infra
                .enqueue_processing_job(
                    &ProcessingJobDraft::for_audio_segment_transcription(segment.id)
                        .with_payload_json(transcription_payload_json),
                )
                .await
                .expect("transcription job should enqueue");
            infra
                .claim_queued_processing_job(transcription_job.id)
                .await
                .expect("transcription job should claim")
                .expect("transcription job should exist");
            infra
                .complete_processing_job(
                    transcription_job.id,
                    &ProcessingResultDraft::new().with_result_text("new transcript"),
                )
                .await
                .expect("transcription completion should not requeue completed speaker analysis");

            let jobs = infra
                .list_processing_jobs_for_subject(&ProcessingSubject::audio_segment(segment.id))
                .await
                .expect("jobs should list");
            let speaker_jobs = jobs
                .iter()
                .filter(|job| job.processor == SPEAKER_ANALYSIS_PROCESSOR)
                .collect::<Vec<_>>();
            assert_eq!(speaker_jobs.len(), 1);
            assert_eq!(speaker_jobs[0].id, completed_speaker_job.id);
            assert_eq!(speaker_jobs[0].status, ProcessingJobStatus::Completed);

            let speaker_result = infra
                .get_processing_result_for_job(completed_speaker_job.id)
                .await
                .expect("speaker result lookup should succeed")
                .expect("speaker result should remain");
            assert_eq!(speaker_result.id, completed_speaker_result.id);
            assert_eq!(
                speaker_result.structured_payload_json,
                completed_speaker_result.structured_payload_json
            );
        });
    }

    #[test]
    fn reprocess_audio_segment_transcription_rejects_system_audio_segments() {
        run_async_test(async {
            let dir = TestDir::new("audio-segment-transcription-system-reprocess");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let system_audio = NewAudioSegment::new(
                AudioSegmentSourceKind::SystemAudio,
                "system-session",
                1,
                "/tmp/system-1.m4a",
                "2026-04-12T10:00:00Z",
                "2026-04-12T10:01:00Z",
            );
            let persisted = infra
                .upsert_audio_segment(&system_audio)
                .await
                .expect("system audio segment should persist");

            let error = infra
                .reprocess_audio_segment_transcription(
                    persisted.id,
                    &AudioSegmentTranscriptionAdmission::available("{}"),
                )
                .await
                .expect_err("system audio reprocess should be rejected");

            assert!(error
                .to_string()
                .contains("only microphone segments can be transcribed"));
        });
    }

    #[test]
    fn transcription_worker_path_completes_audio_jobs_without_starving_ocr() {
        run_async_test(async {
            let dir = TestDir::new("audio-transcription-worker-e2e");
            let infra = AppInfra::initialize_with_processing_registry(
                dir.path(),
                ProcessorRegistry::new()
                    .register(SuccessfulProcessingBackend::new(
                        AUDIO_TRANSCRIPTION_PROCESSOR,
                        "transcribed speech",
                    ))
                    .register(SuccessfulProcessingBackend::new(
                        OCR_PROCESSOR,
                        "recognized text",
                    )),
            )
            .await
            .expect("app infra should initialize");
            let payload = serde_json::to_string(&AudioTranscriptionJobPayload::new(
                "local_whisper",
                Some("base".to_string()),
                "auto",
            ))
            .expect("payload should serialize");
            let committed = infra
                .upsert_audio_segment_and_maybe_enqueue_transcription(
                    &NewAudioSegment::new(
                        AudioSegmentSourceKind::Microphone,
                        "mic-session-worker",
                        1,
                        "/tmp/mic-worker.m4a",
                        "2026-04-12T10:00:00Z",
                        "2026-04-12T10:01:00Z",
                    ),
                    &AudioSegmentTranscriptionAdmission::available(payload),
                )
                .await
                .expect("microphone commit should enqueue transcription");
            let transcription_job = committed.job.expect("transcription job should exist");
            let ocr_job = infra
                .debug_insert_frame_and_enqueue_processing_job(
                    &test_frame("session-worker-ocr", "frame-worker-ocr.png"),
                    OCR_PROCESSOR,
                    None,
                )
                .await
                .expect("ocr job should enqueue")
                .job;

            let transcription_outcome = infra
                .process_next_processing_job_for_processor(AUDIO_TRANSCRIPTION_PROCESSOR)
                .await
                .expect("transcription worker should run")
                .expect("transcription job should be queued");
            let ProcessingJobRunOutcome::Completed(transcription_completion) =
                transcription_outcome
            else {
                panic!("expected completed transcription job");
            };
            assert_eq!(transcription_completion.job.id, transcription_job.id);
            assert_eq!(
                transcription_completion.result.result_text.as_deref(),
                Some("transcribed speech")
            );

            let ocr_still_queued = infra
                .get_processing_job(ocr_job.id)
                .await
                .expect("ocr job should be readable")
                .expect("ocr job should exist");
            assert_eq!(ocr_still_queued.status, ProcessingJobStatus::Queued);

            let ocr_outcome = infra
                .process_next_processing_job_excluding_processor(AUDIO_TRANSCRIPTION_PROCESSOR)
                .await
                .expect("non-transcription worker should run")
                .expect("ocr job should be queued");
            let ProcessingJobRunOutcome::Completed(ocr_completion) = ocr_outcome else {
                panic!("expected completed ocr job");
            };
            assert_eq!(ocr_completion.job.id, ocr_job.id);
            assert_eq!(
                ocr_completion.result.result_text.as_deref(),
                Some("recognized text")
            );
        });
    }

    #[test]
    fn frames_can_be_listed_with_limit_and_offset() {
        run_async_test(async {
            let dir = TestDir::new("processing-frames-pagination");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let first = infra
                .insert_frame(&test_frame("session-a", "frame-1.png"))
                .await
                .expect("first frame should persist");
            let second = infra
                .insert_frame(&test_frame("session-a", "frame-2.png"))
                .await
                .expect("second frame should persist");
            let third = infra
                .insert_frame(&test_frame("session-b", "frame-3.png"))
                .await
                .expect("third frame should persist");

            let limited = infra
                .list_frames(None, None, Some(2), None)
                .await
                .expect("limited frames should list");
            assert_eq!(
                limited.iter().map(|frame| frame.id).collect::<Vec<_>>(),
                vec![third.id, second.id]
            );

            let paged = infra
                .list_frames(None, None, Some(1), Some(1))
                .await
                .expect("paged frames should list");
            assert_eq!(
                paged.iter().map(|frame| frame.id).collect::<Vec<_>>(),
                vec![second.id]
            );

            let session_paged = infra
                .list_frames(Some("session-a"), None, Some(1), Some(1))
                .await
                .expect("session paged frames should list");
            assert_eq!(
                session_paged
                    .iter()
                    .map(|frame| frame.id)
                    .collect::<Vec<_>>(),
                vec![first.id]
            );

            let zero_limit = infra
                .list_frames(None, None, Some(0), None)
                .await
                .expect("zero-limit frames should list");
            assert!(zero_limit.is_empty());
        });
    }

    #[test]
    fn frames_can_be_listed_with_stable_before_id_cursor() {
        run_async_test(async {
            let dir = TestDir::new("processing-frames-before-id-pagination");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let first = infra
                .insert_frame(&test_frame("session-a", "frame-1.png"))
                .await
                .expect("first frame should persist");
            let second = infra
                .insert_frame(&test_frame("session-a", "frame-2.png"))
                .await
                .expect("second frame should persist");
            let third = infra
                .insert_frame(&test_frame("session-a", "frame-3.png"))
                .await
                .expect("third frame should persist");

            let first_page = infra
                .list_frames(Some("session-a"), None, Some(2), None)
                .await
                .expect("first page should list");
            assert_eq!(
                first_page.iter().map(|frame| frame.id).collect::<Vec<_>>(),
                vec![third.id, second.id]
            );

            let inserted_after_first_page = infra
                .insert_frame(&test_frame("session-a", "frame-4.png"))
                .await
                .expect("newest frame should persist");

            let second_page = infra
                .list_frames(Some("session-a"), Some(second.id), Some(2), None)
                .await
                .expect("cursor page should list");
            assert_eq!(
                second_page.iter().map(|frame| frame.id).collect::<Vec<_>>(),
                vec![first.id]
            );

            let offset_page = infra
                .list_frames(Some("session-a"), None, Some(2), Some(2))
                .await
                .expect("offset page should list");
            assert_eq!(
                offset_page.iter().map(|frame| frame.id).collect::<Vec<_>>(),
                vec![second.id, first.id]
            );

            assert!(offset_page.iter().any(|frame| frame.id == second.id));
            assert!(first_page
                .iter()
                .all(|frame| frame.id != inserted_after_first_page.id));
        });
    }

    #[test]
    fn frame_summaries_in_range_are_filtered_and_sorted_newest_first() {
        run_async_test(async {
            let dir = TestDir::new("processing-frame-summaries-range");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            infra
                .insert_frame(&test_frame_at(
                    "session-a",
                    "frame-before.png",
                    "2026-04-11T23:59:59Z",
                ))
                .await
                .expect("earlier frame should persist");
            let start = infra
                .insert_frame(&test_frame_at(
                    "session-a",
                    "frame-start.png",
                    "2026-04-12T00:00:00Z",
                ))
                .await
                .expect("start frame should persist");
            let middle = infra
                .insert_frame(&test_frame_at(
                    "session-b",
                    "frame-middle.png",
                    "2026-04-12T12:30:00Z",
                ))
                .await
                .expect("middle frame should persist");
            let end_first = infra
                .insert_frame(&test_frame_at(
                    "session-c",
                    "frame-end-first.png",
                    "2026-04-12T23:59:59Z",
                ))
                .await
                .expect("end frame should persist");
            let end_second = infra
                .insert_frame(&test_frame_at(
                    "session-d",
                    "frame-end-second.png",
                    "2026-04-12T23:59:59Z",
                ))
                .await
                .expect("second end frame should persist");
            infra
                .insert_frame(&test_frame_at(
                    "session-a",
                    "frame-after.png",
                    "2026-04-13T00:00:00Z",
                ))
                .await
                .expect("later frame should persist");

            let summaries = infra
                .list_frame_summaries_in_range("2026-04-12T00:00:00Z", "2026-04-12T23:59:59Z")
                .await
                .expect("frame summaries should list");

            assert_eq!(
                summaries.iter().map(|frame| frame.id).collect::<Vec<_>>(),
                vec![end_second.id, end_first.id, middle.id, start.id]
            );
            assert_eq!(summaries[0].captured_at, "2026-04-12T23:59:59Z");
            assert_eq!(summaries[1].captured_at, "2026-04-12T23:59:59Z");
        });
    }

    #[test]
    fn latest_frame_in_range_returns_newest_match_or_none() {
        run_async_test(async {
            let dir = TestDir::new("processing-latest-frame-range");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let latest_snapshot = capture_metadata::FrameMetadataSnapshot {
                app_bundle_id: Some("com.example.Latest".to_string()),
                app_name: Some("Latest".to_string()),
                window_title: Some("Latest Window".to_string()),
                window_id: None,
                browser_url: None,
                display_id: None,
                metadata_redaction_reason: None,
                metadata_redaction_source_id: None,
            };

            let earliest = infra
                .insert_frame(&test_frame_at(
                    "session-a",
                    "frame-early.png",
                    "2026-04-12T08:00:00Z",
                ))
                .await
                .expect("early frame should persist");
            let tied_first = infra
                .insert_frame(&test_frame_at(
                    "session-b",
                    "frame-tied-first.png",
                    "2026-04-12T09:30:00Z",
                ))
                .await
                .expect("first tied frame should persist");
            let tied_second = infra
                .insert_frame(
                    &test_frame_at("session-c", "frame-tied-second.png", "2026-04-12T09:30:00Z")
                        .with_metadata_snapshot(latest_snapshot.clone()),
                )
                .await
                .expect("second tied frame should persist");

            let latest = infra
                .get_latest_frame_in_range("2026-04-12T08:30:00Z", "2026-04-12T09:30:00Z")
                .await
                .expect("latest frame should resolve")
                .expect("latest frame should exist");

            assert_eq!(latest.id, tied_second.id);
            assert_eq!(latest.captured_at, tied_first.captured_at);
            assert_eq!(latest.metadata_snapshot.as_ref(), Some(&latest_snapshot));

            let missing = infra
                .get_latest_frame_in_range("2026-04-12T07:00:00Z", "2026-04-12T07:59:59Z")
                .await
                .expect("empty latest frame lookup should succeed");

            assert!(missing.is_none());
            assert!(latest.id > earliest.id);
        });
    }

    #[test]
    fn timeline_window_around_frame_is_newest_first_and_reports_older_history() {
        run_async_test(async {
            let dir = TestDir::new("processing-timeline-window");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let second_snapshot = capture_metadata::FrameMetadataSnapshot {
                app_bundle_id: Some("com.example.Second".to_string()),
                app_name: Some("Second".to_string()),
                window_title: Some("Second Window".to_string()),
                window_id: None,
                browser_url: None,
                display_id: None,
                metadata_redaction_reason: None,
                metadata_redaction_source_id: None,
            };
            let third_snapshot = capture_metadata::FrameMetadataSnapshot {
                app_bundle_id: Some("com.example.Third".to_string()),
                app_name: Some("Third".to_string()),
                window_title: Some("Third Window".to_string()),
                window_id: None,
                browser_url: None,
                display_id: None,
                metadata_redaction_reason: None,
                metadata_redaction_source_id: None,
            };
            let fourth_snapshot = capture_metadata::FrameMetadataSnapshot {
                app_bundle_id: Some("com.example.Fourth".to_string()),
                app_name: Some("Fourth".to_string()),
                window_title: Some("Fourth Window".to_string()),
                window_id: None,
                browser_url: None,
                display_id: None,
                metadata_redaction_reason: None,
                metadata_redaction_source_id: None,
            };

            let first = infra
                .insert_frame(&test_frame("session-a", "frame-1.png"))
                .await
                .expect("first frame should persist");
            let second = infra
                .insert_frame(
                    &test_frame("session-a", "frame-2.png")
                        .with_metadata_snapshot(second_snapshot.clone()),
                )
                .await
                .expect("second frame should persist");
            let third = infra
                .insert_frame(
                    &test_frame("session-a", "frame-3.png")
                        .with_metadata_snapshot(third_snapshot.clone()),
                )
                .await
                .expect("third frame should persist");
            let fourth = infra
                .insert_frame(
                    &test_frame("session-a", "frame-4.png")
                        .with_metadata_snapshot(fourth_snapshot.clone()),
                )
                .await
                .expect("fourth frame should persist");
            let fifth = infra
                .insert_frame(&test_frame("session-a", "frame-5.png"))
                .await
                .expect("fifth frame should persist");

            let window = infra
                .get_timeline_window_around_frame(third.id, 1, 1)
                .await
                .expect("timeline window should resolve");

            assert_eq!(
                window
                    .frames
                    .iter()
                    .map(|frame| frame.id)
                    .collect::<Vec<_>>(),
                vec![fourth.id, third.id, second.id]
            );
            assert_eq!(
                window.frames[0].metadata_snapshot.as_ref(),
                Some(&fourth_snapshot)
            );
            assert_eq!(
                window.frames[1].metadata_snapshot.as_ref(),
                Some(&third_snapshot)
            );
            assert_eq!(
                window.frames[2].metadata_snapshot.as_ref(),
                Some(&second_snapshot)
            );
            assert_eq!(window.target_index, 1);
            assert!(window.has_newer);
            assert!(window.has_older);

            let oldest_window = infra
                .get_timeline_window_around_frame(first.id, 2, 2)
                .await
                .expect("oldest timeline window should resolve");

            assert_eq!(
                oldest_window
                    .frames
                    .iter()
                    .map(|frame| frame.id)
                    .collect::<Vec<_>>(),
                vec![third.id, second.id, first.id]
            );
            assert_eq!(oldest_window.target_index, 2);
            assert!(oldest_window.has_newer);
            assert!(!oldest_window.has_older);

            let newest_window = infra
                .get_timeline_window_around_frame(fifth.id, 2, 1)
                .await
                .expect("newest timeline window should resolve");

            assert_eq!(
                newest_window
                    .frames
                    .iter()
                    .map(|frame| frame.id)
                    .collect::<Vec<_>>(),
                vec![fifth.id, fourth.id]
            );
            assert_eq!(newest_window.target_index, 0);
            assert!(!newest_window.has_newer);
            assert!(newest_window.has_older);
        });
    }

    #[test]
    fn list_frames_for_segment_workspace_escapes_like_wildcards() {
        run_async_test(async {
            let dir = TestDir::new("segment-workspace-like-escape");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let matching = infra
                .insert_frame(
                    &NewFrame::new(
                        "session-a",
                        "/tmp/workspaces/segment_%/frame-1.png",
                        "2026-04-12T10:00:00Z",
                    )
                    .with_dimensions(1920, 1080),
                )
                .await
                .expect("matching frame should persist");

            infra
                .insert_frame(
                    &NewFrame::new(
                        "session-a",
                        "/tmp/workspaces/segment-xx/frame-2.png",
                        "2026-04-12T10:00:01Z",
                    )
                    .with_dimensions(1920, 1080),
                )
                .await
                .expect("wildcard frame should persist");

            infra
                .insert_frame(
                    &NewFrame::new(
                        "session-a",
                        "/tmp/workspaces/segment_%extra/frame-3.png",
                        "2026-04-12T10:00:02Z",
                    )
                    .with_dimensions(1920, 1080),
                )
                .await
                .expect("prefix frame should persist");

            let frames = infra
                .list_frames_for_segment_workspace("session-a", "/tmp/workspaces/segment_%/")
                .await
                .expect("segment workspace frames should list");

            assert_eq!(frames, vec![matching]);
        });
    }

    #[test]
    fn captured_frame_pipeline_persists_frame_batch_and_ocr_job() {
        run_async_test(async {
            let dir = TestDir::new("captured-frame-pipeline");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = infra
                .capture_frame(
                    &test_frame("session-pipeline", "frame-pipeline.png"),
                    Some("{\"language\":\"eng\"}"),
                )
                .await
                .expect("captured frame pipeline should persist frame and job");

            let job = persisted.job.expect("ocr job should be queued");
            assert_eq!(job.subject_type, FRAME_SUBJECT_TYPE);
            assert_eq!(job.subject_id, persisted.frame.id);
            assert_eq!(job.processor, OCR_PROCESSOR);
            assert_eq!(job.status, ProcessingJobStatus::Queued);
            assert_eq!(job.payload_json.as_deref(), Some("{\"language\":\"eng\"}"));
            assert_eq!(persisted.active_batch.session_id, "session-pipeline");
            assert_eq!(persisted.active_batch.frame_count, 1);
            assert!(persisted.closed_batches.is_empty());

            let batch_frames = infra
                .list_frames_for_batch(persisted.active_batch.id)
                .await
                .expect("batch frames should list");
            assert_eq!(batch_frames, vec![persisted.frame]);
        });
    }

    #[test]
    fn stopping_session_closes_and_schedules_last_open_frame_batch() {
        run_async_test(async {
            let dir = TestDir::new("captured-frame-stop-close");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = infra
                .capture_frame(
                    &test_frame("session-stop-close", "frame-final.png"),
                    Some("{\"language\":\"eng\"}"),
                )
                .await
                .expect("captured frame pipeline should persist frame and job");
            assert_eq!(persisted.active_batch.status, FrameBatchStatus::Open);
            assert!(persisted.active_batch.finalize_job_id.is_none());

            let closed = infra
                .close_and_schedule_all_frame_batches_for_session("session-stop-close")
                .await
                .expect("stopped session should close final batch");
            assert_eq!(closed.len(), 1);

            let batch = infra
                .get_frame_batch(persisted.active_batch.id)
                .await
                .expect("batch should be readable")
                .expect("batch should exist");
            assert_eq!(batch.status, FrameBatchStatus::Closed);
            assert!(
                batch.finalize_job_id.is_some(),
                "closed final batch should schedule cleanup finalization"
            );
        });
    }

    #[test]
    fn debug_insert_frame_and_enqueue_ocr_job_persists_linked_subject() {
        run_async_test(async {
            let dir = TestDir::new("processing-enqueue");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = infra
                .debug_insert_frame_and_enqueue_ocr_job(
                    &test_frame("session-ocr", "frame-ocr.png"),
                    Some("{\"language\":\"eng\"}"),
                )
                .await
                .expect("frame and job should persist");

            assert_eq!(persisted.job.subject_type, FRAME_SUBJECT_TYPE);
            assert_eq!(persisted.job.subject_id, persisted.frame.id);
            assert_eq!(persisted.job.processor, OCR_PROCESSOR);
            assert_eq!(persisted.job.status, ProcessingJobStatus::Queued);
            assert_eq!(
                persisted.job.payload_json.as_deref(),
                Some("{\"language\":\"eng\"}")
            );

            let jobs = infra
                .list_processing_jobs_for_subject(&ProcessingSubject::frame(persisted.frame.id))
                .await
                .expect("subject jobs should list");
            assert_eq!(jobs, vec![persisted.job.clone()]);
        });
    }

    #[test]
    fn duplicate_fingerprinted_frames_skip_redundant_ocr_jobs() {
        run_async_test(async {
            let dir = TestDir::new("processing-ocr-dedupe");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let width = 32;
            let height = 32;
            let repeated_pixels = solid_rgba(width, height, [64, 64, 64, 255]);
            let mut changed_pixels = repeated_pixels.clone();
            for y in 8..20 {
                for x in 8..20 {
                    set_pixel_rgba(&mut changed_pixels, width, x, y, [240, 240, 240, 255]);
                }
            }

            let first = infra
                .capture_frame(
                    &test_frame_with_equivalent_image(
                        &dir,
                        "session-dedupe",
                        "frame-1.png",
                        "2026-04-12T10:00:00Z",
                        &repeated_pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("first frame should persist");
            let duplicate = infra
                .capture_frame(
                    &test_frame_with_equivalent_image(
                        &dir,
                        "session-dedupe",
                        "frame-2.png",
                        "2026-04-12T10:00:01Z",
                        &repeated_pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("duplicate frame should persist");
            let changed = infra
                .capture_frame(
                    &test_frame_with_equivalent_image(
                        &dir,
                        "session-dedupe",
                        "frame-3.png",
                        "2026-04-12T10:00:02Z",
                        &changed_pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("changed frame should persist");

            assert!(first.job.is_some());
            assert!(duplicate.job.is_none());
            assert!(changed.job.is_some());

            let frames = infra
                .list_frames(Some("session-dedupe"), None, None, None)
                .await
                .expect("frames should list");
            assert_eq!(frames.len(), 3);
            assert_eq!(frames[0].equivalence.hint, changed.frame.equivalence.hint);
            assert_eq!(frames[1].equivalence.hint, duplicate.frame.equivalence.hint);
            assert_eq!(frames[2].equivalence.hint, first.frame.equivalence.hint);

            let duplicate_jobs = infra
                .list_processing_jobs_for_subject(&ProcessingSubject::frame(duplicate.frame.id))
                .await
                .expect("duplicate jobs should list");
            assert!(duplicate_jobs.is_empty());
        });
    }

    #[test]
    fn non_consecutive_duplicate_fingerprinted_frames_skip_redundant_ocr_jobs() {
        run_async_test(async {
            let dir = TestDir::new("processing-ocr-dedupe-non-consecutive");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let width = 32;
            let height = 32;
            let repeated_pixels = solid_rgba(width, height, [64, 64, 64, 255]);
            let mut changed_pixels = repeated_pixels.clone();
            for y in 8..20 {
                for x in 8..20 {
                    set_pixel_rgba(&mut changed_pixels, width, x, y, [240, 240, 240, 255]);
                }
            }

            let first = infra
                .capture_frame(
                    &test_frame_with_equivalent_image(
                        &dir,
                        "session-dedupe-repeat",
                        "frame-1.png",
                        "2026-04-12T10:00:00Z",
                        &repeated_pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("first frame should persist");
            let changed = infra
                .capture_frame(
                    &test_frame_with_equivalent_image(
                        &dir,
                        "session-dedupe-repeat",
                        "frame-2.png",
                        "2026-04-12T10:00:01Z",
                        &changed_pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("changed frame should persist");
            let repeated = infra
                .capture_frame(
                    &test_frame_with_equivalent_image(
                        &dir,
                        "session-dedupe-repeat",
                        "frame-3.png",
                        "2026-04-12T10:00:02Z",
                        &repeated_pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("repeated frame should persist");

            assert!(first.job.is_some());
            assert!(changed.job.is_some());
            assert!(repeated.job.is_none());

            let frames = infra
                .list_frames(Some("session-dedupe-repeat"), None, None, None)
                .await
                .expect("frames should list");
            assert_eq!(frames.len(), 3);
            assert_eq!(frames[0].equivalence.hint, repeated.frame.equivalence.hint);
            assert_eq!(frames[1].equivalence.hint, changed.frame.equivalence.hint);
            assert_eq!(frames[2].equivalence.hint, first.frame.equivalence.hint);

            let repeated_jobs = infra
                .list_processing_jobs_for_subject(&ProcessingSubject::frame(repeated.frame.id))
                .await
                .expect("repeated jobs should list");
            assert!(repeated_jobs.is_empty());

            let resolved = infra
                .get_nearest_earlier_equivalent_frame(repeated.frame.id)
                .await
                .expect("nearest earlier equivalent frame should resolve");
            assert_eq!(resolved, Some(first.frame));
        });
    }

    #[test]
    fn equivalent_frames_in_different_segment_workspaces_do_not_skip_ocr() {
        run_async_test(async {
            let dir = TestDir::new("processing-ocr-dedupe-segment-scoped");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let width = 32;
            let height = 32;
            let pixels = solid_rgba(width, height, [88, 88, 88, 255]);

            let first = infra
                .capture_frame(
                    &test_segment_frame_with_equivalent_image(
                        &dir,
                        "session-segment-scope",
                        1,
                        "frame-1.png",
                        "2026-04-12T10:00:00Z",
                        &pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("first frame should persist");
            let second = infra
                .capture_frame(
                    &test_segment_frame_with_equivalent_image(
                        &dir,
                        "session-segment-scope",
                        2,
                        "frame-2.png",
                        "2026-04-12T10:00:01Z",
                        &pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("second frame should persist");

            assert!(first.job.is_some());
            assert!(
                second.job.is_some(),
                "equivalent frames in different segment workspaces must not skip OCR"
            );
        });
    }

    #[test]
    fn earlier_equivalent_frame_lookup_does_not_cross_segment_workspaces() {
        run_async_test(async {
            let dir = TestDir::new("processing-ocr-ui-fallback-segment-scoped");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let width = 32;
            let height = 32;
            let pixels = solid_rgba(width, height, [104, 104, 104, 255]);

            let first = infra
                .capture_frame(
                    &test_segment_frame_with_equivalent_image(
                        &dir,
                        "session-segment-ui-scope",
                        1,
                        "frame-1.png",
                        "2026-04-12T10:00:00Z",
                        &pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("first frame should persist");
            let second = infra
                .capture_frame(
                    &test_segment_frame_with_equivalent_image(
                        &dir,
                        "session-segment-ui-scope",
                        2,
                        "frame-2.png",
                        "2026-04-12T10:00:01Z",
                        &pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("second frame should persist");

            assert!(first.job.is_some());
            assert!(second.job.is_some());

            let resolved = infra
                .get_nearest_earlier_equivalent_frame(second.frame.id)
                .await
                .expect("cross-segment equivalent frame lookup should succeed");
            assert_eq!(resolved, None);
        });
    }

    #[test]
    fn same_fingerprint_but_different_frame_bytes_skip_ocr() {
        run_async_test(async {
            let dir = TestDir::new("processing-ocr-dedupe-byte-confirmation");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let width = 32;
            let height = 32;
            let pixels = solid_rgba(width, height, [96, 96, 96, 255]);

            let first = infra
                .capture_frame(
                    &test_frame_with_equivalent_image(
                        &dir,
                        "session-dedupe-confirmed",
                        "frame-1.png",
                        "2026-04-12T10:00:00Z",
                        &pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("first frame should persist");
            let second = infra
                .capture_frame(
                    &test_frame_with_equivalent_image(
                        &dir,
                        "session-dedupe-confirmed",
                        "frame-2.png",
                        "2026-04-12T10:00:01Z",
                        &pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("second frame should persist");

            assert!(first.job.is_some());
            assert!(
                second.job.is_none(),
                "same-fingerprint frames should skip OCR even when bytes differ"
            );
        });
    }

    #[test]
    fn same_fingerprint_but_different_frame_bytes_stay_skipped_after_time_passes() {
        run_async_test(async {
            let dir = TestDir::new("processing-ocr-dedupe-same-fingerprint-late");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let width = 32;
            let height = 32;
            let pixels = solid_rgba(width, height, [112, 112, 112, 255]);

            let first = infra
                .capture_frame(
                    &test_frame_with_equivalent_image(
                        &dir,
                        "session-dedupe-same-fingerprint-late",
                        "frame-1.png",
                        "2026-04-12T10:00:00Z",
                        &pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("first frame should persist");
            let second = infra
                .capture_frame(
                    &test_frame_with_equivalent_image(
                        &dir,
                        "session-dedupe-same-fingerprint-late",
                        "frame-2.png",
                        "2026-04-12T10:00:06Z",
                        &pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("second frame should persist");

            assert!(first.job.is_some());
            assert!(
                second.job.is_none(),
                "same-fingerprint frames should stay skipped until the fingerprint changes"
            );
        });
    }

    #[test]
    fn startup_reconciles_closed_batches_missing_finalize_jobs() {
        run_async_test(async {
            let dir = TestDir::new("frame-batch-startup-reconcile");
            let initial = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let batch = initial
                .frame_batches()
                .upsert_open_batch_for_frame("session-batch-reconcile", "2026-04-12T10:01:00Z")
                .await
                .expect("batch should persist");
            let persisted = initial
                .capture_frame(
                    &NewFrame::new(
                        "session-batch-reconcile",
                        "/tmp/session-batch-reconcile-segment-0001/frames/frame-1.png",
                        "2026-04-12T10:01:00Z",
                    ),
                    None,
                )
                .await
                .expect("frame and OCR state should persist");
            assert_eq!(persisted.active_batch.id, batch.id);

            let closed = initial
                .frame_batches()
                .close_completed_batches_for_session("session-batch-reconcile", None)
                .await
                .expect("batch should close without scheduling");
            assert_eq!(closed.len(), 1);
            assert!(closed[0].finalize_job_id.is_none());

            drop(initial);

            let recovered = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should re-initialize");

            let reconciled = recovered
                .get_frame_batch(batch.id)
                .await
                .expect("batch should be readable")
                .expect("batch should exist");
            assert_eq!(reconciled.status, FrameBatchStatus::Closed);
            let finalize_job_id = reconciled
                .finalize_job_id
                .expect("startup should schedule finalize job");

            let finalize_job = recovered
                .get_job(finalize_job_id)
                .await
                .expect("finalize job should be readable")
                .expect("finalize job should exist");
            assert_eq!(finalize_job.kind, FRAME_BATCH_FINALIZE_JOB_KIND);
            assert_eq!(finalize_job.status, BackgroundJobStatus::Queued);
        });
    }

    #[test]
    fn startup_reconciles_open_batches_from_stopped_sessions() {
        run_async_test(async {
            let dir = TestDir::new("frame-batch-startup-open-reconcile");
            let initial = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let batch = initial
                .frame_batches()
                .upsert_open_batch_for_frame("session-open-reconcile", "2026-04-12T10:01:00Z")
                .await
                .expect("batch should persist");
            let frame = initial
                .processing()
                .insert_frame(&test_frame("session-open-reconcile", "frame-open.png"))
                .await
                .expect("frame should persist");
            initial
                .frame_batches()
                .attach_frame_to_batch(frame.id, batch.id, &frame.captured_at)
                .await
                .expect("frame should attach");

            drop(initial);

            let recovered = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should recover open batches");
            let reconciled = recovered
                .get_frame_batch(batch.id)
                .await
                .expect("batch should be readable")
                .expect("batch should exist");
            assert_eq!(reconciled.status, FrameBatchStatus::Closed);
            assert!(
                reconciled.finalize_job_id.is_some(),
                "startup should schedule finalization for orphaned open batch"
            );
        });
    }

    #[test]
    fn startup_retries_failed_finalize_jobs_and_repairs_processing_batches() {
        run_async_test(async {
            let dir = TestDir::new("frame-batch-startup-finalize-retry");
            let initial = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = initial
                .capture_frame(
                    &NewFrame::new(
                        "session-finalize-retry",
                        "/tmp/session-finalize-retry-segment-0001/frames/frame-1.png",
                        "2026-04-12T10:01:00Z",
                    ),
                    None,
                )
                .await
                .expect("frame should persist");

            let closed = initial
                .close_and_schedule_all_frame_batches_for_session("session-finalize-retry")
                .await
                .expect("batch should close and schedule");
            assert_eq!(closed.len(), 1);

            let scheduled_batch = initial
                .get_frame_batch(closed[0].id)
                .await
                .expect("scheduled batch should be readable")
                .expect("scheduled batch should exist");
            let first_job_id = scheduled_batch
                .finalize_job_id
                .expect("closed batch should have finalize job");
            initial
                .jobs()
                .mark_failed(first_job_id, Some("expected finalize failure"))
                .await
                .expect("finalize job should fail");

            initial
                .frame_batches()
                .mark_batch_processing(persisted.active_batch.id)
                .await
                .expect("batch should enter processing to simulate interrupted finalization");

            drop(initial);

            let recovered = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should re-initialize");

            let repaired = recovered
                .get_frame_batch(persisted.active_batch.id)
                .await
                .expect("batch should be readable")
                .expect("batch should exist");
            assert_eq!(repaired.status, FrameBatchStatus::Closed);
            let retried_job_id = repaired
                .finalize_job_id
                .expect("startup should schedule replacement finalize job");
            assert_ne!(retried_job_id, first_job_id);

            let retried_job = recovered
                .get_job(retried_job_id)
                .await
                .expect("replacement finalize job should be readable")
                .expect("replacement finalize job should exist");
            assert_eq!(retried_job.kind, FRAME_BATCH_FINALIZE_JOB_KIND);
            assert_eq!(retried_job.status, BackgroundJobStatus::Queued);

            let original_job = recovered
                .get_job(first_job_id)
                .await
                .expect("original finalize job should be readable")
                .expect("original finalize job should exist");
            assert_eq!(original_job.status, BackgroundJobStatus::Failed);

            let finalize_job_count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM background_jobs WHERE kind = ?1")
                    .bind(FRAME_BATCH_FINALIZE_JOB_KIND)
                    .fetch_one(recovered.pool())
                    .await
                    .expect("finalize jobs should count");
            assert_eq!(finalize_job_count, 2);
        });
    }

    #[test]
    fn batch_insert_rolls_back_frame_and_job_when_attachment_fails() {
        run_async_test(async {
            let dir = TestDir::new("frame-batch-atomic");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let mut transaction = infra
                .pool()
                .begin()
                .await
                .expect("transaction should start");
            let batch = infra
                .frame_batches()
                .upsert_open_batch_for_frame_in_transaction(
                    &mut transaction,
                    "session-batch-atomic",
                    "2026-04-12T10:01:00Z",
                )
                .await
                .expect("batch should persist in transaction");
            let persisted = infra
                .captured_frame_pipeline()
                .capture_frame_in_transaction(
                    &mut transaction,
                    &test_frame_with_equivalent_image(
                        &dir,
                        "session-batch-atomic",
                        "frame-atomic.png",
                        "2026-04-12T10:01:00Z",
                        &solid_rgba(32, 32, [72, 72, 72, 255]),
                        32,
                        32,
                    ),
                    None,
                )
                .await
                .expect("frame and OCR state should persist in transaction");

            let error = infra
                .frame_batches()
                .attach_frame_to_batch_in_transaction(
                    &mut transaction,
                    persisted.frame.id,
                    i64::MAX,
                    &persisted.frame.captured_at,
                )
                .await
                .expect_err("invalid batch attachment should fail");
            assert!(matches!(error, AppInfraError::Sqlx(_)));

            transaction
                .rollback()
                .await
                .expect("failed transaction should roll back");

            let frames = infra
                .list_frames(Some("session-batch-atomic"), None, None, None)
                .await
                .expect("frames should list");
            assert!(frames.is_empty());

            let batches = infra
                .list_frame_batches(Some("session-batch-atomic"))
                .await
                .expect("batches should list");
            assert!(batches.is_empty());

            let processing_job_count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM processing_jobs WHERE subject_id = ?1")
                    .bind(persisted.frame.id)
                    .fetch_one(infra.pool())
                    .await
                    .expect("processing job count should query");
            assert_eq!(processing_job_count, 0);

            assert!(infra
                .get_frame_batch(batch.id)
                .await
                .expect("batch lookup should succeed")
                .is_none());
        });
    }

    #[test]
    fn batched_frame_insert_assigns_ten_minute_windows_and_schedules_closed_batch() {
        run_async_test(async {
            let dir = TestDir::new("frame-batches");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let first = infra
                .capture_frame(
                    &NewFrame::new(
                        "session-batches",
                        "/tmp/session-batches-segment-0001/frames/frame-1.png",
                        "2026-04-12T10:01:00Z",
                    ),
                    None,
                )
                .await
                .expect("first frame should persist");
            let second = infra
                .capture_frame(
                    &NewFrame::new(
                        "session-batches",
                        "/tmp/session-batches-segment-0002/frames/frame-2.png",
                        "2026-04-12T10:11:00Z",
                    ),
                    None,
                )
                .await
                .expect("second frame should persist");
            assert_eq!(second.closed_batches.len(), 1);
            assert_eq!(second.closed_batches[0].id, first.active_batch.id);

            let first_batches = infra
                .list_frame_batches(Some("session-batches"))
                .await
                .expect("frame batches should list");
            assert_eq!(first_batches.len(), 2);
            assert_eq!(first_batches[0].status, FrameBatchStatus::Open);
            assert_eq!(first_batches[0].frame_count, 1);
            assert_eq!(first_batches[1].status, FrameBatchStatus::Closed);
            assert_eq!(first_batches[1].frame_count, 1);
            assert!(first_batches[1].finalize_job_id.is_some());

            let first_batch_frames = infra
                .list_frames_for_batch(first_batches[1].id)
                .await
                .expect("batch frames should list");
            assert_eq!(first_batch_frames.len(), 1);
            assert_eq!(first_batch_frames[0].id, first.frame.id);

            let second_batch_frames = infra
                .list_frames_for_batch(first_batches[0].id)
                .await
                .expect("second batch frames should list");
            assert_eq!(second_batch_frames.len(), 1);
            assert_eq!(second_batch_frames[0].id, second.frame.id);
        });
    }

    #[test]
    fn batch_insert_rolls_back_frame_and_batch_when_finalize_scheduling_fails() {
        run_async_test(async {
            let dir = TestDir::new("frame-batch-finalize-atomic");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let first = infra
                .capture_frame(
                    &NewFrame::new(
                        "session-finalize-atomic",
                        "/tmp/session-finalize-atomic-segment-0001/frames/frame-1.png",
                        "2026-04-12T10:01:00Z",
                    ),
                    None,
                )
                .await
                .expect("first frame should persist");

            sqlx::query(
                "CREATE TRIGGER fail_frame_batch_finalize_job \
                 BEFORE INSERT ON background_jobs \
                 WHEN NEW.kind = 'frame_batch_combine' \
                 BEGIN \
                     SELECT RAISE(FAIL, 'forced finalize scheduling failure'); \
                 END",
            )
            .execute(infra.pool())
            .await
            .expect("failure trigger should install");

            let error = infra
                .capture_frame(
                    &NewFrame::new(
                        "session-finalize-atomic",
                        "/tmp/session-finalize-atomic-segment-0002/frames/frame-2.png",
                        "2026-04-12T10:11:00Z",
                    ),
                    None,
                )
                .await
                .expect_err("finalize scheduling failure should abort batch insert");
            assert!(matches!(error, AppInfraError::Sqlx(_)));

            sqlx::query("DROP TRIGGER fail_frame_batch_finalize_job")
                .execute(infra.pool())
                .await
                .expect("failure trigger should drop");

            let batches = infra
                .list_frame_batches(Some("session-finalize-atomic"))
                .await
                .expect("batches should list");
            assert_eq!(batches.len(), 1);
            assert_eq!(batches[0].status, FrameBatchStatus::Open);
            assert_eq!(batches[0].frame_count, 1);
            assert!(batches[0].finalize_job_id.is_none());

            let frames = infra
                .list_frames(Some("session-finalize-atomic"), None, None, None)
                .await
                .expect("frames should list");
            assert_eq!(frames.len(), 1);
            assert_eq!(frames[0].id, first.frame.id);

            let finalize_job_count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM background_jobs WHERE kind = ?1")
                    .bind(FRAME_BATCH_FINALIZE_JOB_KIND)
                    .fetch_one(infra.pool())
                    .await
                    .expect("finalize jobs should count");
            assert_eq!(finalize_job_count, 0);
        });
    }

    #[test]
    fn frames_without_fingerprint_still_enqueue_ocr() {
        run_async_test(async {
            let dir = TestDir::new("processing-ocr-no-fingerprint");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let first = infra
                .capture_frame(&test_frame("session-no-fingerprint", "frame-1.png"), None)
                .await
                .expect("first frame should persist");
            let second = infra
                .capture_frame(&test_frame("session-no-fingerprint", "frame-2.png"), None)
                .await
                .expect("second frame should persist");

            assert!(first.job.is_some());
            assert!(second.job.is_some());
        });
    }

    #[test]
    fn reprocess_captured_frame_ocr_creates_job_when_none_exists() {
        run_async_test(async {
            let dir = TestDir::new("captured-frame-reprocessing-create");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let width = 32;
            let height = 32;
            let pixels = solid_rgba(width, height, [80, 80, 80, 255]);

            infra
                .capture_frame(
                    &test_frame_with_equivalent_image(
                        &dir,
                        "session-reprocess-create",
                        "frame-1.png",
                        "2026-04-12T10:00:00Z",
                        &pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("first frame should persist");
            let duplicate = infra
                .capture_frame(
                    &test_frame_with_equivalent_image(
                        &dir,
                        "session-reprocess-create",
                        "frame-2.png",
                        "2026-04-12T10:00:01Z",
                        &pixels,
                        width,
                        height,
                    ),
                    None,
                )
                .await
                .expect("duplicate frame should persist");
            assert!(duplicate.job.is_none());

            let reprocessed = infra
                .reprocess_captured_frame_ocr(duplicate.frame.id, Some("{\"language\":\"eng\"}"))
                .await
                .expect("reprocessing should create an OCR job");

            assert_eq!(
                reprocessed.outcome,
                CapturedFrameReprocessingOutcome::Created
            );
            assert_eq!(reprocessed.job.subject_id, duplicate.frame.id);
            assert_eq!(reprocessed.job.processor, OCR_PROCESSOR);
            assert_eq!(reprocessed.job.status, ProcessingJobStatus::Queued);
            assert_eq!(
                reprocessed.job.payload_json.as_deref(),
                Some("{\"language\":\"eng\"}")
            );

            let subject_jobs = infra
                .list_processing_jobs_for_subject(&ProcessingSubject::frame(duplicate.frame.id))
                .await
                .expect("subject jobs should list");
            assert_eq!(subject_jobs, vec![reprocessed.job.clone()]);
        });
    }

    #[test]
    fn reprocess_captured_frame_ocr_ignores_queued_job_and_keeps_payload() {
        run_async_test(async {
            let dir = TestDir::new("captured-frame-reprocessing-ignore");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = infra
                .capture_frame(
                    &test_frame("session-reprocess-ignore", "frame-queued.png"),
                    Some("{\"language\":\"eng\"}"),
                )
                .await
                .expect("captured frame pipeline should persist frame and job");
            let queued_job = persisted.job.expect("ocr job should be queued");

            let reprocessed = infra
                .reprocess_captured_frame_ocr(queued_job.subject_id, Some("{\"language\":\"fra\"}"))
                .await
                .expect("queued reprocessing should be ignored");

            assert_eq!(
                reprocessed.outcome,
                CapturedFrameReprocessingOutcome::Ignored
            );
            assert_eq!(reprocessed.job.id, queued_job.id);
            assert_eq!(
                reprocessed.job.payload_json.as_deref(),
                Some("{\"language\":\"eng\"}")
            );
        });
    }

    #[test]
    fn reprocess_captured_frame_ocr_requeues_terminal_job_and_clears_results() {
        run_async_test(async {
            let dir = TestDir::new("captured-frame-reprocessing-requeue");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = infra
                .capture_frame(
                    &test_frame("session-reprocess-requeue", "frame-terminal.png"),
                    Some("{\"language\":\"eng\"}"),
                )
                .await
                .expect("captured frame pipeline should persist frame and job");
            let queued_job = persisted.job.expect("ocr job should be queued");

            infra
                .claim_queued_processing_job(queued_job.id)
                .await
                .expect("job should start")
                .expect("job should claim successfully");
            infra
                .complete_processing_job(
                    queued_job.id,
                    &ProcessingResultDraft::new().with_result_text("first pass"),
                )
                .await
                .expect("job should complete");
            sqlx::query(
                "UPDATE processing_jobs SET queued_at = '2000-01-01 00:00:00' WHERE id = ?1",
            )
            .bind(queued_job.id)
            .execute(infra.pool())
            .await
            .expect("queued timestamp should be adjustable");
            assert!(infra
                .get_processing_result_for_job(queued_job.id)
                .await
                .expect("result lookup should succeed")
                .is_some());

            let reprocessed = infra
                .reprocess_captured_frame_ocr(persisted.frame.id, Some("{\"language\":\"fra\"}"))
                .await
                .expect("terminal job should requeue");

            assert_eq!(
                reprocessed.outcome,
                CapturedFrameReprocessingOutcome::Requeued
            );
            assert_eq!(reprocessed.job.id, queued_job.id);
            assert_eq!(reprocessed.job.status, ProcessingJobStatus::Queued);
            assert_eq!(reprocessed.job.attempt_count, 1);
            assert_ne!(reprocessed.job.queued_at, "2000-01-01 00:00:00");
            assert_eq!(
                reprocessed.job.payload_json.as_deref(),
                Some("{\"language\":\"fra\"}")
            );
            assert_eq!(reprocessed.job.last_error, None);
            assert!(reprocessed.job.started_at.is_none());
            assert!(reprocessed.job.finished_at.is_none());
            assert!(infra
                .get_processing_result_for_job(queued_job.id)
                .await
                .expect("requeued result lookup should succeed")
                .is_none());

            let subject_results = infra
                .list_processing_results_for_subject(&ProcessingSubject::frame(persisted.frame.id))
                .await
                .expect("subject results should list");
            assert!(subject_results.is_empty());
        });
    }

    #[test]
    fn reprocess_captured_frame_ocr_rejects_running_job() {
        run_async_test(async {
            let dir = TestDir::new("captured-frame-reprocessing-running");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = infra
                .capture_frame(
                    &test_frame("session-reprocess-running", "frame-running.png"),
                    None,
                )
                .await
                .expect("captured frame pipeline should persist frame and job");
            let queued_job = persisted.job.expect("ocr job should be queued");

            infra
                .claim_queued_processing_job(queued_job.id)
                .await
                .expect("job should start")
                .expect("job should claim successfully");

            let error = infra
                .reprocess_captured_frame_ocr(persisted.frame.id, None)
                .await
                .expect_err("running jobs should reject reprocessing");

            assert!(matches!(
                error,
                AppInfraError::ProcessingJobInvalidTransition { job_id, ref from, ref to }
                    if job_id == queued_job.id && from == "running" && to == "queued"
            ));
        });
    }

    #[test]
    fn reprocess_captured_frame_ocr_requires_existing_frame() {
        run_async_test(async {
            let dir = TestDir::new("captured-frame-reprocessing-missing-frame");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let error = infra
                .reprocess_captured_frame_ocr(404, None)
                .await
                .expect_err("missing frames should fail");

            assert!(matches!(error, AppInfraError::FrameNotFound(404)));
        });
    }

    #[test]
    fn processing_results_persist_separately_from_frames() {
        run_async_test(async {
            let dir = TestDir::new("processing-results");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = infra
                .debug_insert_frame_and_enqueue_ocr_job(
                    &test_frame("session-results", "frame-results.png"),
                    None,
                )
                .await
                .expect("frame and job should persist");

            let running = infra
                .claim_queued_processing_job(persisted.job.id)
                .await
                .expect("job should transition to running")
                .expect("job should claim successfully");
            assert_eq!(running.status, ProcessingJobStatus::Running);
            assert_eq!(running.attempt_count, 1);

            // A real OCR result carries a parseable OcrStructuredPayload; the secret
            // redaction gate rejects an unparseable one before persistence, so use a
            // valid (benign, secret-free) payload here.
            let structured_payload_json = serde_json::to_string(&OcrStructuredPayload::new(
                ocr::APPLE_VISION_PROVIDER_ID,
                None,
                vec![OcrObservation::new(
                    "recognized text",
                    0.95,
                    OcrBoundingBox::new(0.1, 0.2, 0.3, 0.4),
                )],
            ))
            .expect("structured payload should serialize");

            let completion = infra
                .complete_processing_job(
                    persisted.job.id,
                    &ProcessingResultDraft::new()
                        .with_result_text("recognized text")
                        .with_structured_payload_json(&structured_payload_json)
                        .with_processor_version("ocr-v1"),
                )
                .await
                .expect("job completion should persist result");

            assert_eq!(completion.job.status, ProcessingJobStatus::Completed);
            assert_eq!(completion.result.job_id, persisted.job.id);
            assert_eq!(completion.result.subject_id, persisted.frame.id);
            assert_eq!(completion.result.processor, OCR_PROCESSOR);
            assert_eq!(
                completion.result.result_text.as_deref(),
                Some("recognized text")
            );

            let stored_result = infra
                .get_processing_result_for_job(persisted.job.id)
                .await
                .expect("job result should be readable")
                .expect("job result should exist");
            assert_eq!(stored_result, completion.result);

            let subject_results = infra
                .list_processing_results_for_subject(&ProcessingSubject::frame(persisted.frame.id))
                .await
                .expect("subject results should list");
            assert_eq!(subject_results, vec![completion.result.clone()]);

            let frame = infra
                .get_frame(persisted.frame.id)
                .await
                .expect("frame should still be readable")
                .expect("frame should exist");
            assert_eq!(frame.file_path, persisted.frame.file_path);
            assert_eq!(frame.width, Some(1920));
            assert_eq!(frame.height, Some(1080));
        });
    }

    #[test]
    fn processing_results_redact_secrets_before_search_projection() {
        run_async_test(async {
            let dir = TestDir::new("processing-redactions");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = infra
                .debug_insert_frame_and_enqueue_ocr_job(
                    &test_frame("session-redactions", "frame-redactions.png"),
                    None,
                )
                .await
                .expect("frame and job should persist");

            infra
                .claim_queued_processing_job(persisted.job.id)
                .await
                .expect("job should transition to running")
                .expect("job should claim successfully");

            let secret = "sk-abcdefghijklmnopqrstuvwxyz123456";
            infra
                .complete_processing_job(
                    persisted.job.id,
                    &ProcessingResultDraft::new()
                        .with_result_text(format!("OPENAI_API_KEY={secret} nearby context")),
                )
                .await
                .expect("job completion should persist redacted result");

            let stored_result = infra
                .get_processing_result_for_job(persisted.job.id)
                .await
                .expect("job result should be readable")
                .expect("job result should exist");
            let stored_text = stored_result
                .result_text
                .as_deref()
                .expect("redacted result text should be stored");
            assert!(stored_text.contains("[REDACTED_SECRET: API_KEY]"));
            assert!(!stored_text.contains(secret));

            let secret_results = infra
                .search_capture(SearchCaptureRequest {
                    query: secret.to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                })
                .await
                .expect("secret search should run");
            assert!(secret_results.frames.is_empty());

            let context_results = infra
                .search_capture(SearchCaptureRequest {
                    query: "nearby context".to_string(),
                    frame_limit: Some(5),
                    frame_offset: None,
                    audio_limit: Some(0),
                    audio_offset: None,
                    snapshot_document_id: None,
                    refinements: None,
                })
                .await
                .expect("context search should run");
            assert_eq!(context_results.frames.len(), 1);
            assert!(context_results.frames[0]
                .snippet
                .contains("[REDACTED_SECRET: API_KEY]"));
        });
    }

    #[test]
    fn processing_job_lifecycle_clears_stale_results_on_retry() {
        run_async_test(async {
            let dir = TestDir::new("processing-lifecycle");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = infra
                .debug_insert_frame_and_enqueue_ocr_job(
                    &test_frame("session-retry", "frame-retry.png"),
                    Some("{\"language\":\"eng\"}"),
                )
                .await
                .expect("frame and job should persist");

            infra
                .claim_queued_processing_job(persisted.job.id)
                .await
                .expect("job should start")
                .expect("job should claim successfully");
            infra
                .complete_processing_job(
                    persisted.job.id,
                    &ProcessingResultDraft::new().with_result_text("first pass"),
                )
                .await
                .expect("job should complete");
            assert!(infra
                .get_processing_result_for_job(persisted.job.id)
                .await
                .expect("result lookup should succeed")
                .is_some());

            let retried = infra
                .mark_processing_job_running(persisted.job.id)
                .await
                .expect("job should restart");
            assert_eq!(retried.status, ProcessingJobStatus::Running);
            assert_eq!(retried.attempt_count, 2);
            assert!(infra
                .get_processing_result_for_job(persisted.job.id)
                .await
                .expect("retry result lookup should succeed")
                .is_none());

            let failed = infra
                .mark_processing_job_failed(persisted.job.id, Some("ocr retry failed"))
                .await
                .expect("job should fail");
            assert_eq!(failed.status, ProcessingJobStatus::Failed);
            assert_eq!(failed.last_error.as_deref(), Some("ocr retry failed"));
            assert!(infra
                .get_processing_result_for_job(persisted.job.id)
                .await
                .expect("failed result lookup should succeed")
                .is_none());
        });
    }

    #[test]
    fn processing_job_completion_requires_running_state() {
        run_async_test(async {
            let dir = TestDir::new("processing-complete-transition");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = infra
                .debug_insert_frame_and_enqueue_ocr_job(
                    &test_frame("session-complete", "frame-complete.png"),
                    None,
                )
                .await
                .expect("frame and job should persist");

            let error = infra
                .complete_processing_job(
                    persisted.job.id,
                    &ProcessingResultDraft::new().with_result_text("recognized text"),
                )
                .await
                .expect_err("queued jobs should not complete directly");

            assert!(matches!(
                error,
                AppInfraError::ProcessingJobInvalidTransition { job_id, ref from, ref to }
                    if job_id == persisted.job.id && from == "queued" && to == "completed"
            ));
        });
    }

    #[test]
    fn processing_job_failure_requires_running_state() {
        run_async_test(async {
            let dir = TestDir::new("processing-fail-transition");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = infra
                .debug_insert_frame_and_enqueue_ocr_job(
                    &test_frame("session-fail", "frame-fail.png"),
                    None,
                )
                .await
                .expect("frame and job should persist");

            let error = infra
                .mark_processing_job_failed(persisted.job.id, Some("not running"))
                .await
                .expect_err("queued jobs should not fail directly");

            assert!(matches!(
                error,
                AppInfraError::ProcessingJobInvalidTransition { job_id, ref from, ref to }
                    if job_id == persisted.job.id && from == "queued" && to == "failed"
            ));
        });
    }

    #[test]
    fn processing_job_retry_requires_terminal_state() {
        run_async_test(async {
            let dir = TestDir::new("processing-retry-transition");
            let infra = AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = infra
                .debug_insert_frame_and_enqueue_ocr_job(
                    &test_frame("session-running", "frame-running.png"),
                    None,
                )
                .await
                .expect("frame and job should persist");

            let claimed = infra
                .claim_queued_processing_job(persisted.job.id)
                .await
                .expect("job claim should succeed")
                .expect("job should claim");
            assert_eq!(claimed.status, ProcessingJobStatus::Running);

            let error = infra
                .mark_processing_job_running(persisted.job.id)
                .await
                .expect_err("running jobs should not restart");

            assert!(matches!(
                error,
                AppInfraError::ProcessingJobInvalidTransition { job_id, ref from, ref to }
                    if job_id == persisted.job.id && from == "running" && to == "running"
            ));
        });
    }
}
