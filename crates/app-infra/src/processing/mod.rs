mod apple_vision;
mod backend;
mod engine;
mod frame;
mod job;
mod ocr;
mod result;
mod runtime;
mod store;

pub use backend::{ProcessorBackend, ProcessorRegistry};
pub use engine::{AppleVisionOcrEngine, OcrEngine, OcrOutput, OcrProvider, OcrRequest};
pub use frame::{Frame, FrameSummary, NewFrame};
pub use job::{
    ProcessingJob, ProcessingJobDraft, ProcessingJobStatus, ProcessingSubject, FRAME_SUBJECT_TYPE,
    OCR_PROCESSOR,
};
pub use ocr::OcrProcessorBackend;
pub use result::{ProcessingResult, ProcessingResultDraft};
pub use runtime::{ProcessingJobRunOutcome, ProcessingRuntime};
pub use store::{FrameProcessingJob, ProcessingJobCompletion, ProcessingStore};
