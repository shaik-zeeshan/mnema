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

        let detection = detect_system_audio_speech(&segment.file_path, payload.detector)?;
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

// On Windows the captured system-audio segment is an AAC `.m4a`; decode it to
// native-rate mono `f32` through the `media-decode` seam (MF Source Reader) and
// run the same VAD runtime over the mono PCM as the macOS path. The seam returns
// the native rate; `detect_f32_mono` resamples internally as needed.
#[cfg(target_os = "windows")]
fn detect_system_audio_speech(
    file_path: &str,
    detector: AudioSpeechDetector,
) -> Result<capture_vad::AudioSpeechDetectionOutcome> {
    let decoded = media_decode::decode_to_mono_f32(file_path)
        .map_err(|error| AppInfraError::AudioTranscriptionEngine(error.to_string()))?;
    let mut runtime = capture_vad::AudioSpeechDetectorRuntime::new(detector)
        .map_err(|error| AppInfraError::AudioTranscriptionEngine(error.to_string()))?;
    runtime
        .detect_f32_mono(&decoded.samples, decoded.sample_rate_hz)
        .map_err(|error| AppInfraError::AudioTranscriptionEngine(error.to_string()))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn detect_system_audio_speech(
    _file_path: &str,
    detector: AudioSpeechDetector,
) -> Result<capture_vad::AudioSpeechDetectionOutcome> {
    Err(AppInfraError::AudioTranscriptionEngine(format!(
        "system-audio speech detection with {} is only available on macOS and Windows",
        capture_vad::configured_adapter_as_str(detector)
    )))
}
