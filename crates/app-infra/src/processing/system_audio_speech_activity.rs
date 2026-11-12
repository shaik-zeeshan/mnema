use async_trait::async_trait;
use capture_types::AudioSpeechDetector;
use serde::{Deserialize, Serialize};

use crate::{AppInfraError, AudioSegment, Result};

use super::{
    ProcessingJob, ProcessingResultDraft, ProcessingStore, AUDIO_SEGMENT_SUBJECT_TYPE,
    SYSTEM_AUDIO_SPEECH_ACTIVITY_PROCESSOR,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SystemAudioSpeechActivityJobPayload {
    pub detector: AudioSpeechDetector,
    pub transcription_payload: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_analysis_payload: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpeechRangeMs {
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SystemAudioSpeechActivityResult {
    pub speech_detected: bool,
    pub speech_ranges_ms: Vec<SpeechRangeMs>,
    pub range_strategy: String,
    pub padding_ms: u64,
    pub timing_reliable: bool,
    pub configured_detector: AudioSpeechDetector,
    pub effective_detector: AudioSpeechDetector,
    pub fallback_reason: Option<String>,
    pub detector_version: Option<String>,
    pub processed_duration_ms: u64,
}

impl SystemAudioSpeechActivityJobPayload {
    pub fn from_job(job: &ProcessingJob) -> Result<Self> {
        let Some(payload_json) = job.payload_json.as_deref() else {
            return Err(AppInfraError::AudioTranscriptionEngine(
                "system-audio speech activity job is missing frozen payload".to_string(),
            ));
        };
        Ok(serde_json::from_str(payload_json)?)
    }
}

#[derive(Clone, Default)]
pub struct SystemAudioSpeechActivityProcessorBackend;

impl SystemAudioSpeechActivityProcessorBackend {
    async fn load_audio_segment(
        &self,
        store: &ProcessingStore,
        job: &ProcessingJob,
    ) -> Result<AudioSegment> {
        if job.subject_type != AUDIO_SEGMENT_SUBJECT_TYPE {
            return Err(AppInfraError::UnsupportedProcessingSubject {
                processor: job.processor.clone(),
                subject_type: job.subject_type.clone(),
            });
        }

        store
            .get_audio_segment(job.subject_id)
            .await?
            .ok_or(AppInfraError::AudioSegmentNotFound(job.subject_id))
    }
}

#[async_trait]
impl super::ProcessorBackend for SystemAudioSpeechActivityProcessorBackend {
    fn processor(&self) -> &'static str {
        SYSTEM_AUDIO_SPEECH_ACTIVITY_PROCESSOR
    }

    async fn process(
        &self,
        store: &ProcessingStore,
        job: &ProcessingJob,
    ) -> Result<ProcessingResultDraft> {
        let segment = self.load_audio_segment(store, job).await?;
        let payload = SystemAudioSpeechActivityJobPayload::from_job(job)?;
        if payload.detector == AudioSpeechDetector::Off {
            return Err(AppInfraError::AudioTranscriptionEngine(
                "speech detector is off".to_string(),
            ));
        }

        let metadata = std::fs::metadata(&segment.file_path).map_err(|error| {
            AppInfraError::AudioTranscriptionEngine(format!(
                "failed to read system-audio segment {}: {error}",
                segment.file_path
            ))
        })?;
        if metadata.len() == 0 {
            return Err(AppInfraError::AudioTranscriptionEngine(format!(
                "system-audio segment {} is empty",
                segment.file_path
            )));
        }

        // Off the runtime: decoding up to 5 min of audio plus Silero is slow
        // synchronous work, and a quit's abort only lands at an `.await`.
        let file_path = segment.file_path.clone();
        let detector = payload.detector;
        let detection = tokio::task::spawn_blocking(move || {
            with_utility_qos(|| detect_system_audio_speech(&file_path, detector))
        })
        .await
        .map_err(|error| {
            AppInfraError::AudioTranscriptionEngine(format!(
                "system-audio speech activity worker failed to join: {error}"
            ))
        })??;
        let speech_detected = detection.speech_detected;
        let result = SystemAudioSpeechActivityResult {
            speech_detected,
            speech_ranges_ms: detection
                .speech_ranges_ms
                .into_iter()
                .map(|range| SpeechRangeMs {
                    start_ms: range.start_ms.saturating_sub(1000),
                    end_ms: range.end_ms.saturating_add(1000),
                })
                .collect(),
            range_strategy: "first_last_with_padding".to_string(),
            padding_ms: 1000,
            timing_reliable: detection.timing_reliable,
            configured_detector: payload.detector,
            effective_detector: detection.effective_detector,
            fallback_reason: None,
            detector_version: detection.detector_version,
            processed_duration_ms: detection.processed_duration_ms,
        };
        let structured_payload_json = serde_json::to_string(&result)?;

        Ok(ProcessingResultDraft::new()
            .with_result_text(if speech_detected {
                "speech detected"
            } else {
                "no speech detected"
            })
            .with_processor_version("system_audio_speech_activity:metadata-v1")
            .with_structured_payload_json(structured_payload_json))
    }
}

/// Runs `work` at utility QoS on this thread, then puts the thread's previous
/// class back: blocking-pool threads are reused by latency-sensitive work.
#[cfg(target_os = "macos")]
fn with_utility_qos<T>(work: impl FnOnce() -> T) -> T {
    struct Restore(Option<(libc::qos_class_t, libc::c_int)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            if let Some((class, relative_priority)) = self.0 {
                // SAFETY: changes only the calling thread's QoS.
                unsafe { libc::pthread_set_qos_class_self_np(class, relative_priority) };
            }
        }
    }

    let mut class = libc::qos_class_t::QOS_CLASS_UNSPECIFIED;
    let mut relative_priority = 0;
    // SAFETY: per-thread QoS FFI; the out-pointers are valid stack locals.
    let read = unsafe {
        libc::pthread_get_qos_class_np(libc::pthread_self(), &mut class, &mut relative_priority)
    };
    let _restore = Restore((read == 0).then_some((class, relative_priority)));
    // SAFETY: changes only the calling thread's QoS.
    unsafe { libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0) };
    work()
}

#[cfg(not(target_os = "macos"))]
fn with_utility_qos<T>(work: impl FnOnce() -> T) -> T {
    work()
}

#[cfg(target_os = "macos")]
fn detect_system_audio_speech(
    file_path: &str,
    detector: AudioSpeechDetector,
) -> Result<capture_vad::AudioSpeechDetectionOutcome> {
    let decoded = capture_writers::decode_audio_file_to_mono_pcm(std::path::Path::new(file_path))
        .map_err(|error| AppInfraError::AudioTranscriptionEngine(error.message))?;
    let mut runtime = capture_vad::AudioSpeechDetectorRuntime::new(detector)
        .map_err(|error| AppInfraError::AudioTranscriptionEngine(error.to_string()))?;
    runtime
        .detect_f32_mono(&decoded.samples, decoded.sample_rate_hz)
        .map_err(|error| AppInfraError::AudioTranscriptionEngine(error.to_string()))
}

#[cfg(not(target_os = "macos"))]
fn detect_system_audio_speech(
    _file_path: &str,
    detector: AudioSpeechDetector,
) -> Result<capture_vad::AudioSpeechDetectionOutcome> {
    Err(AppInfraError::AudioTranscriptionEngine(format!(
        "system-audio speech detection with {} is only available on macOS",
        capture_vad::configured_adapter_as_str(detector)
    )))
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use libc::qos_class_t::{QOS_CLASS_UNSPECIFIED, QOS_CLASS_USER_INITIATED, QOS_CLASS_UTILITY};

    fn current_qos() -> u32 {
        let mut class = QOS_CLASS_UNSPECIFIED;
        let mut relative_priority = 0;
        unsafe {
            libc::pthread_get_qos_class_np(libc::pthread_self(), &mut class, &mut relative_priority)
        };
        class as u32
    }

    #[test]
    fn utility_qos_lasts_only_for_the_work() {
        std::thread::spawn(|| {
            unsafe { libc::pthread_set_qos_class_self_np(QOS_CLASS_USER_INITIATED, 0) };
            assert_eq!(
                super::with_utility_qos(current_qos),
                QOS_CLASS_UTILITY as u32
            );
            assert_eq!(current_qos(), QOS_CLASS_USER_INITIATED as u32);
        })
        .join()
        .expect("qos thread");
    }
}
