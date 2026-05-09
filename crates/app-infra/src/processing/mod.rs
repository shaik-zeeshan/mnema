mod audio_transcription;
mod backend;
mod frame;
mod job;
mod ocr;
mod result;
mod runtime;
mod speaker_analysis;
mod store;

pub use audio_transcription::{AudioTranscriptionJobPayload, AudioTranscriptionProcessorBackend};
pub use backend::{ProcessorBackend, ProcessorRegistry};
pub use frame::{Frame, FrameEquivalence, FrameEquivalenceStatus, FrameSummary, NewFrame};
pub use job::{
    ProcessingJob, ProcessingJobDraft, ProcessingJobStatus, ProcessingSubject,
    AUDIO_SEGMENT_SUBJECT_TYPE, AUDIO_TRANSCRIPTION_PROCESSOR, FRAME_SUBJECT_TYPE, OCR_PROCESSOR,
    SPEAKER_ANALYSIS_PROCESSOR,
};
pub use ocr::OcrProcessorBackend;
pub use result::{ProcessingResult, ProcessingResultDraft};
pub use runtime::{ProcessingJobRunOutcome, ProcessingRuntime};
pub use speaker_analysis::{SpeakerAnalysisJobPayload, SpeakerAnalysisProcessorBackend};
pub use store::{
    FocusedFrameWindow, FrameProcessingJob, PersonProfile, ProcessingJobCompletion,
    ProcessingModelCleanupLock, ProcessingStore, SegmentWorkspaceOcrReference, SpeakerClusterView,
    SpeakerTurnView, SPEAKER_ANALYSIS_PAYLOAD_OPTION_KEY,
};
