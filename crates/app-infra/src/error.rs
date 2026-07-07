use thiserror::Error;

pub type Result<T> = std::result::Result<T, AppInfraError>;

#[derive(Debug, Error)]
pub enum AppInfraError {
    #[error("failed to access app infrastructure files: {0}")]
    Io(#[from] std::io::Error),
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("database migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("failed to build the CPU worker pool: {0}")]
    WorkerPoolBuild(#[from] rayon::ThreadPoolBuildError),
    #[error("background jobs require an active Tokio runtime")]
    AsyncRuntimeUnavailable,
    #[error("cpu job worker stopped before returning a result")]
    WorkerTaskCancelled,
    #[error("background job {0} was not found")]
    JobNotFound(i64),
    #[error("invalid background job status: {0}")]
    InvalidJobStatus(String),
    #[error("background job {job_id} cannot transition from '{from}' to '{to}'")]
    BackgroundJobInvalidTransition {
        job_id: i64,
        from: String,
        to: String,
    },
    #[error("frame {0} was not found")]
    FrameNotFound(i64),
    #[error("audio segment {0} was not found")]
    AudioSegmentNotFound(i64),
    #[error("invalid frame equivalence status: {0}")]
    InvalidFrameEquivalenceStatus(String),
    #[error("frame batch {0} was not found")]
    FrameBatchNotFound(i64),
    #[error("processing job {0} was not found")]
    ProcessingJobNotFound(i64),
    #[error("processing result {0} was not found")]
    ProcessingResultNotFound(i64),
    #[error("invalid processing job status: {0}")]
    InvalidProcessingJobStatus(String),
    #[error("processing job {job_id} cannot transition from '{from}' to '{to}'")]
    ProcessingJobInvalidTransition {
        job_id: i64,
        from: String,
        to: String,
    },
    #[error("processor backend is not registered for '{0}'")]
    UnknownProcessor(String),
    #[error("processor '{processor}' does not support subject type '{subject_type}'")]
    UnsupportedProcessingSubject {
        processor: String,
        subject_type: String,
    },
    #[error("processing job {job_id} is not runnable from status '{status}'")]
    ProcessingJobNotRunnable { job_id: i64, status: String },
    #[error("ocr engine error: {0}")]
    OcrEngine(String),
    #[error("audio transcription error: {0}")]
    AudioTranscriptionEngine(String),
    /// A transient-liveness audio transcription failure (ADR 0048): the queue requeues the job
    /// with backoff without incrementing its failure count. Carries the provider's message.
    #[error("audio transcription provider is temporarily unavailable: {0}")]
    AudioTranscriptionTransientLiveness(String),
    #[error("speaker analysis error: {0}")]
    SpeakerAnalysisEngine(String),
    #[error("{0}")]
    SecretRedactionGate(String),
    #[error("invalid frame batch status: {0}")]
    InvalidFrameBatchStatus(String),
    #[error("frame batch {batch_id} cannot transition from '{from}' to '{to}'")]
    FrameBatchInvalidTransition {
        batch_id: i64,
        from: String,
        to: String,
    },
    #[error("invalid frame batch timestamp '{0}'")]
    InvalidFrameBatchTimestamp(String),
    #[error("invalid search request: {0}")]
    InvalidSearchRequest(String),
    #[error("{0}")]
    BrokeredAccess(String),
    #[error("capture index encryption error: {0}")]
    CaptureIndexEncryption(String),
    #[error("ai provider key store error: {0}")]
    AiProviderKeyStore(String),
    #[error("license token store error: {0}")]
    LicenseTokenStore(String),
    #[error("mcp server secret store error: {0}")]
    McpServerSecretStore(String),
    #[error("frame batch {batch_id} is not ready because OCR is still pending")]
    FrameBatchOcrPending { batch_id: i64 },
    #[error("frame batch {batch_id} has no frames to finalize")]
    EmptyFrameBatch { batch_id: i64 },
    #[error("failed to finalize frame batch: {0}")]
    FrameBatchFinalize(String),
}

impl AppInfraError {
    /// Whether this error is a transient-liveness condition that should requeue the job without
    /// spending a failure attempt (ADR 0048), rather than walking the bounded-retry failure path.
    pub fn is_transient_liveness(&self) -> bool {
        matches!(self, Self::AudioTranscriptionTransientLiveness(_))
    }
}
