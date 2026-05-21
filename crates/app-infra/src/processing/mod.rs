mod audio_transcription;
mod backend;
mod frame;
mod job;
mod ocr;
mod result;
mod runtime;
mod secret_redaction_pipeline;
mod speaker_analysis;
mod store;
mod system_audio_speech_activity;

pub use audio_transcription::{AudioTranscriptionJobPayload, AudioTranscriptionProcessorBackend};
pub use backend::{ProcessorBackend, ProcessorRegistry};
pub use frame::{Frame, FrameEquivalence, FrameEquivalenceStatus, FrameSummary, NewFrame};
pub use job::{
    ProcessingJob, ProcessingJobDraft, ProcessingJobStatus, ProcessingSubject,
    AUDIO_SEGMENT_SUBJECT_TYPE, AUDIO_TRANSCRIPTION_PROCESSOR, FRAME_SUBJECT_TYPE, OCR_PROCESSOR,
    SPEAKER_ANALYSIS_PROCESSOR, SYSTEM_AUDIO_SPEECH_ACTIVITY_PROCESSOR,
};
pub use ocr::OcrProcessorBackend;
pub use result::{ProcessingResult, ProcessingResultDraft};
pub use runtime::{ProcessingJobRunOutcome, ProcessingRuntime};
pub use speaker_analysis::{
    SpeakerAnalysisJobPayload, SpeakerAnalysisProcessorBackend, HELPER_TIMEOUT_SECONDS_OPTION,
};
pub(crate) use store::map_frame_for_search;
pub use store::{
    FocusedFrameWindow, FrameProcessingJob, PersonProfile, ProcessingJobCompletion,
    ProcessingModelCleanupLock, ProcessingStore, SegmentWorkspaceOcrReference, SpeakerClusterView,
    SpeakerTurnView, SPEAKER_ANALYSIS_PAYLOAD_OPTION_KEY,
};
pub use system_audio_speech_activity::{
    SystemAudioSpeechActivityJobPayload, SystemAudioSpeechActivityProcessorBackend,
    SystemAudioSpeechActivityResult,
};
