mod silero_vad;
mod webrtc_vad;

use self::webrtc_vad::WebrtcVadAdapter;
pub use capture_types::AudioSpeechDetector;
use capture_types::MicrophoneVadAdapter;
use silero_vad::{SileroVadAdapter, SileroVadLoadError};
use std::collections::HashSet;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectiveMicrophoneVadAdapter {
    Silero,
    Webrtc,
    PeakLevel,
    Off,
}

impl EffectiveMicrophoneVadAdapter {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Silero => "silero",
            Self::Webrtc => "webrtc",
            Self::PeakLevel => "peak_level",
            Self::Off => "off",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MicrophoneSpeechDecision {
    pub idle_ms: Option<u64>,
    pub latest_normalized_level: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MicrophoneVadFrameOutcome {
    pub decision: MicrophoneSpeechDecision,
    pub vad_speech_detected: bool,
}

#[derive(Debug)]
pub struct MicrophoneVadRuntime {
    configured_adapter: MicrophoneVadAdapter,
    effective_adapter: EffectiveMicrophoneVadAdapter,
    fallback_reason: Option<String>,
    silero_adapter: Option<SileroVadAdapter>,
    webrtc_adapter: WebrtcVadAdapter,
    latest_vad_speech: Option<MicrophoneSpeechDecision>,
    last_vad_speech_unix_ms: Option<u64>,
    notified_failures: HashSet<MicrophoneVadAdapter>,
}

impl Default for MicrophoneVadRuntime {
    fn default() -> Self {
        Self::new(MicrophoneVadAdapter::Silero)
    }
}

impl MicrophoneVadRuntime {
    pub fn new(configured_adapter: MicrophoneVadAdapter) -> Self {
        let resolution = resolve_effective_adapter(configured_adapter);
        Self {
            configured_adapter,
            effective_adapter: resolution.effective_adapter,
            fallback_reason: resolution.fallback_reason,
            silero_adapter: resolution.silero_adapter,
            webrtc_adapter: WebrtcVadAdapter::new(),
            latest_vad_speech: None,
            last_vad_speech_unix_ms: None,
            notified_failures: HashSet::new(),
        }
    }

    pub fn configured_adapter(&self) -> MicrophoneVadAdapter {
        self.configured_adapter
    }

    pub fn effective_adapter(&self) -> EffectiveMicrophoneVadAdapter {
        self.effective_adapter
    }

    pub fn fallback_reason(&self) -> Option<&str> {
        self.fallback_reason.as_deref()
    }

    pub fn uses_vad_adapter(&self) -> bool {
        matches!(
            self.effective_adapter,
            EffectiveMicrophoneVadAdapter::Silero | EffectiveMicrophoneVadAdapter::Webrtc
        )
    }

    pub fn take_new_fallback_notification(&mut self) -> Option<MicrophoneVadFallbackNotice> {
        let failed_adapter = match (self.configured_adapter, self.effective_adapter) {
            (MicrophoneVadAdapter::Silero, EffectiveMicrophoneVadAdapter::Webrtc) => {
                MicrophoneVadAdapter::Silero
            }
            (MicrophoneVadAdapter::Silero, EffectiveMicrophoneVadAdapter::PeakLevel) => {
                MicrophoneVadAdapter::Silero
            }
            (MicrophoneVadAdapter::Webrtc, EffectiveMicrophoneVadAdapter::PeakLevel) => {
                MicrophoneVadAdapter::Webrtc
            }
            _ => return None,
        };

        if !self.notified_failures.insert(failed_adapter) {
            return None;
        }

        Some(MicrophoneVadFallbackNotice {
            configured_adapter: self.configured_adapter,
            effective_adapter: self.effective_adapter,
            reason: self
                .fallback_reason
                .clone()
                .unwrap_or_else(|| "selected VAD adapter is unavailable".to_string()),
        })
    }

    pub fn process_pcm_frame(
        &mut self,
        frame: MicrophonePcmVadFrame<'_>,
    ) -> Result<MicrophoneVadFrameOutcome, MicrophoneVadError> {
        let active_adapter = self.effective_adapter;
        let speech_detected = self
            .active_vad_adapter()
            .and_then(|adapter| adapter.process_pcm_frame(&frame));
        let speech_detected = match speech_detected {
            Ok(speech_detected) => speech_detected,
            Err(error) => {
                self.fall_back_to_peak_level_after_processing_error(active_adapter, &error);
                return Err(error);
            }
        };

        let Some(speech_detected) = speech_detected else {
            return Ok(MicrophoneVadFrameOutcome {
                decision: self.latest_vad_speech.unwrap_or(MicrophoneSpeechDecision {
                    idle_ms: None,
                    latest_normalized_level: None,
                }),
                vad_speech_detected: false,
            });
        };

        if speech_detected {
            self.last_vad_speech_unix_ms = Some(frame.captured_at_unix_ms);
        }

        let decision = MicrophoneSpeechDecision {
            idle_ms: self
                .last_vad_speech_unix_ms
                .map(|last_speech_ms| frame.captured_at_unix_ms.saturating_sub(last_speech_ms)),
            // Policy-facing microphone decisions are speech-first. Expose a
            // threshold-qualified activity marker only when the VAD adapter
            // reports speech; raw peak levels remain available on debug
            // sample fields and must not override a non-speech VAD result.
            latest_normalized_level: speech_detected.then_some(1.0),
        };
        self.latest_vad_speech = Some(decision);

        Ok(MicrophoneVadFrameOutcome {
            decision,
            vad_speech_detected: speech_detected,
        })
    }

    fn active_vad_adapter(&mut self) -> Result<&mut dyn VadAdapter, MicrophoneVadError> {
        match self.effective_adapter {
            EffectiveMicrophoneVadAdapter::Webrtc => Ok(&mut self.webrtc_adapter),
            EffectiveMicrophoneVadAdapter::Silero => match self.silero_adapter.as_mut() {
                Some(adapter) => Ok(adapter),
                None => Err(MicrophoneVadError::AdapterUnavailable {
                    adapter: EffectiveMicrophoneVadAdapter::Silero,
                    reason: self
                        .fallback_reason
                        .clone()
                        .unwrap_or_else(|| "Silero VAD adapter was not initialized".to_string()),
                }),
            },
            EffectiveMicrophoneVadAdapter::PeakLevel => {
                Err(MicrophoneVadError::AdapterUnavailable {
                    adapter: EffectiveMicrophoneVadAdapter::PeakLevel,
                    reason: self.fallback_reason.clone().unwrap_or_else(|| {
                        "selected VAD adapter is unavailable; using peak-level microphone activity"
                            .to_string()
                    }),
                })
            }
            EffectiveMicrophoneVadAdapter::Off => Err(MicrophoneVadError::AdapterUnavailable {
                adapter: EffectiveMicrophoneVadAdapter::Off,
                reason: "microphone VAD is disabled".to_string(),
            }),
        }
    }

    fn fall_back_to_peak_level_after_processing_error(
        &mut self,
        failed_adapter: EffectiveMicrophoneVadAdapter,
        error: &MicrophoneVadError,
    ) {
        if !matches!(
            failed_adapter,
            EffectiveMicrophoneVadAdapter::Silero | EffectiveMicrophoneVadAdapter::Webrtc
        ) {
            return;
        }

        self.effective_adapter = EffectiveMicrophoneVadAdapter::PeakLevel;
        self.fallback_reason = Some(format!(
            "{} VAD processing failed: {error}; using peak-level microphone activity",
            failed_adapter.as_str()
        ));
        self.latest_vad_speech = None;
        self.last_vad_speech_unix_ms = None;
    }

    pub fn decide_from_peak_level(
        &mut self,
        peak_level: Option<f32>,
        peak_idle_ms: Option<u64>,
        peak_threshold: f32,
    ) -> MicrophoneSpeechDecision {
        let decision = self.peek_decision_from_peak_level(peak_level, peak_idle_ms, peak_threshold);

        if matches!(
            self.effective_adapter,
            EffectiveMicrophoneVadAdapter::Webrtc | EffectiveMicrophoneVadAdapter::Silero
        ) && decision.latest_normalized_level.is_some()
        {
            self.latest_vad_speech = Some(MicrophoneSpeechDecision {
                idle_ms: decision.idle_ms,
                latest_normalized_level: None,
            });
        }

        decision
    }

    pub fn peek_decision_from_peak_level(
        &self,
        peak_level: Option<f32>,
        peak_idle_ms: Option<u64>,
        _peak_threshold: f32,
    ) -> MicrophoneSpeechDecision {
        match self.effective_adapter {
            EffectiveMicrophoneVadAdapter::Off | EffectiveMicrophoneVadAdapter::PeakLevel => {
                MicrophoneSpeechDecision {
                    idle_ms: peak_idle_ms,
                    latest_normalized_level: peak_level,
                }
            }
            EffectiveMicrophoneVadAdapter::Webrtc | EffectiveMicrophoneVadAdapter::Silero => {
                self.latest_vad_speech.unwrap_or(MicrophoneSpeechDecision {
                    idle_ms: None,
                    latest_normalized_level: None,
                })
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MicrophonePcmVadFrame<'a> {
    pub samples: &'a [i16],
    pub sample_rate_hz: u32,
    pub captured_at_unix_ms: u64,
    pub normalized_peak_level: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MicrophoneVadError {
    AdapterUnavailable {
        adapter: EffectiveMicrophoneVadAdapter,
        reason: String,
    },
    InvalidSampleRate(u32),
    InvalidFrameLength {
        sample_rate_hz: u32,
        sample_count: usize,
    },
    WebrtcProcessFailed,
}

impl fmt::Display for MicrophoneVadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AdapterUnavailable { adapter, reason } => {
                write!(
                    formatter,
                    "{} VAD adapter unavailable: {reason}",
                    adapter.as_str()
                )
            }
            Self::InvalidSampleRate(sample_rate_hz) => {
                write!(formatter, "unsupported VAD sample rate: {sample_rate_hz}")
            }
            Self::InvalidFrameLength {
                sample_rate_hz,
                sample_count,
            } => write!(
                formatter,
                "invalid VAD frame length: {sample_count} samples at {sample_rate_hz} Hz"
            ),
            Self::WebrtcProcessFailed => write!(formatter, "WebRTC VAD failed to process frame"),
        }
    }
}

impl std::error::Error for MicrophoneVadError {}

trait VadAdapter: fmt::Debug + Send {
    fn process_pcm_frame(
        &mut self,
        frame: &MicrophonePcmVadFrame<'_>,
    ) -> Result<Option<bool>, MicrophoneVadError>;
}

impl VadAdapter for SileroVadAdapter {
    fn process_pcm_frame(
        &mut self,
        frame: &MicrophonePcmVadFrame<'_>,
    ) -> Result<Option<bool>, MicrophoneVadError> {
        SileroVadAdapter::process_pcm_frame(self, frame.samples, frame.sample_rate_hz).map_err(
            |error| MicrophoneVadError::AdapterUnavailable {
                adapter: EffectiveMicrophoneVadAdapter::Silero,
                reason: error.to_string(),
            },
        )
    }
}

#[derive(Debug, Clone)]
pub struct MicrophoneVadFallbackNotice {
    pub configured_adapter: MicrophoneVadAdapter,
    pub effective_adapter: EffectiveMicrophoneVadAdapter,
    pub reason: String,
}

pub fn configured_adapter_as_str(adapter: MicrophoneVadAdapter) -> &'static str {
    match adapter {
        MicrophoneVadAdapter::Silero => "silero",
        MicrophoneVadAdapter::Webrtc => "webrtc",
        MicrophoneVadAdapter::Off => "off",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeechRangeMs {
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioSpeechDetectionOutcome {
    pub speech_detected: bool,
    pub speech_ranges_ms: Vec<SpeechRangeMs>,
    pub effective_detector: AudioSpeechDetector,
    pub detector_version: Option<String>,
    pub processed_duration_ms: u64,
    pub timing_reliable: bool,
}

#[derive(Debug)]
pub struct AudioSpeechDetectorRuntime {
    configured_detector: AudioSpeechDetector,
    silero_adapter: Option<SileroVadAdapter>,
    webrtc_adapter: WebrtcVadAdapter,
}

impl AudioSpeechDetectorRuntime {
    pub fn new(configured_detector: AudioSpeechDetector) -> Result<Self, MicrophoneVadError> {
        let silero_adapter = match configured_detector {
            AudioSpeechDetector::Silero => {
                Some(SileroVadAdapter::load_default().map_err(|error| {
                    MicrophoneVadError::AdapterUnavailable {
                        adapter: EffectiveMicrophoneVadAdapter::Silero,
                        reason: error.to_string(),
                    }
                })?)
            }
            AudioSpeechDetector::Webrtc | AudioSpeechDetector::Off => None,
        };

        if configured_detector == AudioSpeechDetector::Off {
            return Err(MicrophoneVadError::AdapterUnavailable {
                adapter: EffectiveMicrophoneVadAdapter::Off,
                reason: "speech detector is disabled".to_string(),
            });
        }

        Ok(Self {
            configured_detector,
            silero_adapter,
            webrtc_adapter: WebrtcVadAdapter::new(),
        })
    }

    pub fn detect_f32_mono(
        &mut self,
        samples: &[f32],
        sample_rate_hz: u32,
    ) -> Result<AudioSpeechDetectionOutcome, MicrophoneVadError> {
        if samples.is_empty() {
            return Ok(AudioSpeechDetectionOutcome {
                speech_detected: false,
                speech_ranges_ms: Vec::new(),
                effective_detector: self.configured_detector,
                detector_version: Some(detector_version(self.configured_detector).to_string()),
                processed_duration_ms: 0,
                timing_reliable: true,
            });
        }

        let target_rate_hz = 16_000;
        let resampled = resample_f32_linear(samples, sample_rate_hz, target_rate_hz);
        let pcm = f32_to_i16_pcm(&resampled);
        let frame_samples = (target_rate_hz as usize * 30) / 1_000;
        let mut speech_ranges = Vec::new();
        let mut active_start_ms: Option<u64> = None;
        let mut processed_samples = 0usize;

        for chunk in pcm.chunks_exact(frame_samples) {
            let frame_start_ms = (processed_samples as u64 * 1_000) / target_rate_hz as u64;
            let frame_end_ms =
                ((processed_samples + frame_samples) as u64 * 1_000) / target_rate_hz as u64;
            let speech = match self.configured_detector {
                AudioSpeechDetector::Silero => {
                    let adapter = self.silero_adapter.as_mut().ok_or_else(|| {
                        MicrophoneVadError::AdapterUnavailable {
                            adapter: EffectiveMicrophoneVadAdapter::Silero,
                            reason: "Silero VAD adapter was not initialized".to_string(),
                        }
                    })?;
                    adapter
                        .process_pcm_frame(chunk, target_rate_hz)
                        .map_err(|error| MicrophoneVadError::AdapterUnavailable {
                            adapter: EffectiveMicrophoneVadAdapter::Silero,
                            reason: error.to_string(),
                        })?
                        .unwrap_or(false)
                }
                AudioSpeechDetector::Webrtc => self
                    .webrtc_adapter
                    .process_pcm_frame(&MicrophonePcmVadFrame {
                        samples: chunk,
                        sample_rate_hz: target_rate_hz,
                        captured_at_unix_ms: frame_start_ms,
                        normalized_peak_level: 0.0,
                    })?
                    .unwrap_or(false),
                AudioSpeechDetector::Off => false,
            };

            if speech {
                active_start_ms.get_or_insert(frame_start_ms);
            } else if let Some(start_ms) = active_start_ms.take() {
                speech_ranges.push(SpeechRangeMs {
                    start_ms,
                    end_ms: frame_end_ms,
                });
            }
            processed_samples += frame_samples;
        }

        if let Some(start_ms) = active_start_ms {
            speech_ranges.push(SpeechRangeMs {
                start_ms,
                end_ms: (processed_samples as u64 * 1_000) / target_rate_hz as u64,
            });
        }

        Ok(AudioSpeechDetectionOutcome {
            speech_detected: !speech_ranges.is_empty(),
            speech_ranges_ms: speech_ranges,
            effective_detector: self.configured_detector,
            detector_version: Some(detector_version(self.configured_detector).to_string()),
            processed_duration_ms: (samples.len() as u64 * 1_000) / sample_rate_hz.max(1) as u64,
            timing_reliable: true,
        })
    }
}

pub fn detector_version(detector: AudioSpeechDetector) -> &'static str {
    match detector {
        AudioSpeechDetector::Silero => "silero",
        AudioSpeechDetector::Webrtc => "webrtc-vad",
        AudioSpeechDetector::Off => "off",
    }
}

fn f32_to_i16_pcm(samples: &[f32]) -> Vec<i16> {
    samples
        .iter()
        .map(|sample| {
            let clamped = sample.clamp(-1.0, 1.0);
            (clamped * i16::MAX as f32).round() as i16
        })
        .collect()
}

fn resample_f32_linear(samples: &[f32], source_rate_hz: u32, target_rate_hz: u32) -> Vec<f32> {
    if samples.is_empty() || source_rate_hz == 0 || target_rate_hz == 0 {
        return Vec::new();
    }
    if source_rate_hz == target_rate_hz {
        return samples.to_vec();
    }
    let output_len =
        ((samples.len() as u64 * target_rate_hz as u64) / source_rate_hz as u64).max(1) as usize;
    let ratio = source_rate_hz as f64 / target_rate_hz as f64;
    let mut output = Vec::with_capacity(output_len);
    for index in 0..output_len {
        let source_pos = index as f64 * ratio;
        let left = source_pos.floor() as usize;
        let right = (left + 1).min(samples.len() - 1);
        let frac = (source_pos - left as f64) as f32;
        output.push(samples[left] + (samples[right] - samples[left]) * frac);
    }
    output
}

#[derive(Debug)]
struct MicrophoneVadResolution {
    effective_adapter: EffectiveMicrophoneVadAdapter,
    fallback_reason: Option<String>,
    silero_adapter: Option<SileroVadAdapter>,
}

fn resolve_effective_adapter(configured_adapter: MicrophoneVadAdapter) -> MicrophoneVadResolution {
    resolve_effective_adapter_with_probes(
        configured_adapter,
        SileroVadAdapter::load_default,
        webrtc_adapter_available,
    )
}

fn resolve_effective_adapter_with_probes(
    configured_adapter: MicrophoneVadAdapter,
    load_silero: impl FnOnce() -> Result<SileroVadAdapter, SileroVadLoadError>,
    webrtc_available: impl FnOnce() -> bool,
) -> MicrophoneVadResolution {
    match configured_adapter {
        MicrophoneVadAdapter::Off => MicrophoneVadResolution {
            effective_adapter: EffectiveMicrophoneVadAdapter::Off,
            fallback_reason: None,
            silero_adapter: None,
        },
        MicrophoneVadAdapter::Webrtc => {
            if webrtc_available() {
                MicrophoneVadResolution {
                    effective_adapter: EffectiveMicrophoneVadAdapter::Webrtc,
                    fallback_reason: None,
                    silero_adapter: None,
                }
            } else {
                MicrophoneVadResolution {
                    effective_adapter: EffectiveMicrophoneVadAdapter::PeakLevel,
                    fallback_reason: Some(
                        "WebRTC VAD runtime is unavailable; using peak-level microphone activity"
                            .to_string(),
                    ),
                    silero_adapter: None,
                }
            }
        }
        MicrophoneVadAdapter::Silero => match load_silero() {
            Ok(silero_adapter) => MicrophoneVadResolution {
                effective_adapter: EffectiveMicrophoneVadAdapter::Silero,
                fallback_reason: None,
                silero_adapter: Some(silero_adapter),
            },
            Err(error) => MicrophoneVadResolution {
                effective_adapter: EffectiveMicrophoneVadAdapter::PeakLevel,
                fallback_reason: Some(error.fallback_reason().to_string()),
                silero_adapter: None,
            },
        },
    }
}

fn webrtc_adapter_available() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn silero_adapter_does_not_emit_fallback_notice_when_available() {
        let mut runtime = MicrophoneVadRuntime::new(MicrophoneVadAdapter::Silero);

        assert_eq!(
            runtime.effective_adapter(),
            EffectiveMicrophoneVadAdapter::Silero
        );
        assert_eq!(runtime.fallback_reason(), None);
        assert!(runtime.take_new_fallback_notification().is_none());
    }

    #[test]
    fn silero_to_webrtc_fallback_notifies_once() {
        let mut runtime = MicrophoneVadRuntime {
            configured_adapter: MicrophoneVadAdapter::Silero,
            effective_adapter: EffectiveMicrophoneVadAdapter::Webrtc,
            fallback_reason: Some("Silero VAD unavailable in test; using WebRTC VAD".to_string()),
            silero_adapter: None,
            webrtc_adapter: WebrtcVadAdapter::new(),
            latest_vad_speech: None,
            last_vad_speech_unix_ms: None,
            notified_failures: HashSet::new(),
        };

        let notice = runtime
            .take_new_fallback_notification()
            .expect("fallback should notify once");

        assert_eq!(notice.configured_adapter, MicrophoneVadAdapter::Silero);
        assert_eq!(
            notice.effective_adapter,
            EffectiveMicrophoneVadAdapter::Webrtc
        );
        assert!(notice.reason.contains("using WebRTC VAD"));
        assert!(runtime.take_new_fallback_notification().is_none());
    }

    #[test]
    fn webrtc_adapter_does_not_emit_fallback_notice_when_available() {
        let mut runtime = MicrophoneVadRuntime::new(MicrophoneVadAdapter::Webrtc);

        assert_eq!(
            runtime.effective_adapter(),
            EffectiveMicrophoneVadAdapter::Webrtc
        );
        assert_eq!(runtime.fallback_reason(), None);
        assert!(runtime.take_new_fallback_notification().is_none());
    }

    #[test]
    fn webrtc_to_peak_level_fallback_notifies_once() {
        let mut runtime = MicrophoneVadRuntime {
            configured_adapter: MicrophoneVadAdapter::Webrtc,
            effective_adapter: EffectiveMicrophoneVadAdapter::PeakLevel,
            fallback_reason: Some(
                "WebRTC VAD runtime is unavailable; using peak-level microphone activity"
                    .to_string(),
            ),
            silero_adapter: None,
            webrtc_adapter: WebrtcVadAdapter::unavailable("test unavailable"),
            latest_vad_speech: None,
            last_vad_speech_unix_ms: None,
            notified_failures: HashSet::new(),
        };

        let notice = runtime
            .take_new_fallback_notification()
            .expect("fallback should notify once");

        assert_eq!(notice.configured_adapter, MicrophoneVadAdapter::Webrtc);
        assert_eq!(
            notice.effective_adapter,
            EffectiveMicrophoneVadAdapter::PeakLevel
        );
        assert!(notice
            .reason
            .contains("using peak-level microphone activity"));
        assert!(runtime.take_new_fallback_notification().is_none());
    }

    #[test]
    fn effective_webrtc_does_not_use_peak_level_before_vad_frames_arrive() {
        let mut runtime = MicrophoneVadRuntime::new(MicrophoneVadAdapter::Webrtc);

        let decision = runtime.decide_from_peak_level(Some(1.0), Some(0), 0.1);

        assert_eq!(decision.idle_ms, None);
        assert_eq!(decision.latest_normalized_level, None);
    }

    #[test]
    fn off_uses_peak_level_without_fallback_notice() {
        let mut runtime = MicrophoneVadRuntime::new(MicrophoneVadAdapter::Off);
        let decision = runtime.decide_from_peak_level(Some(0.5), Some(20), 0.1);

        assert_eq!(decision.latest_normalized_level, Some(0.5));
        assert!(runtime.take_new_fallback_notification().is_none());
    }

    #[test]
    fn silero_does_not_fall_back_to_webrtc_when_unavailable() {
        let resolution = resolve_effective_adapter_with_probes(
            MicrophoneVadAdapter::Silero,
            || {
                Err(SileroVadLoadError::MissingModel {
                    candidates: vec![PathBuf::from("/missing/silero_vad.onnx")],
                })
            },
            || false,
        );

        assert_eq!(
            resolution.effective_adapter,
            EffectiveMicrophoneVadAdapter::PeakLevel
        );
        let reason = resolution.fallback_reason.expect("fallback reason");
        assert!(reason.contains("Silero VAD model was not found"));
        assert!(!reason.contains("WebRTC"));
    }

    #[test]
    fn selected_webrtc_falls_back_to_peak_level_when_unavailable() {
        let resolution = resolve_effective_adapter_with_probes(
            MicrophoneVadAdapter::Webrtc,
            || unreachable!(),
            || false,
        );

        assert_eq!(
            resolution.effective_adapter,
            EffectiveMicrophoneVadAdapter::PeakLevel
        );
        assert!(resolution
            .fallback_reason
            .expect("fallback reason")
            .contains("using peak-level microphone activity"));
    }

    #[test]
    fn webrtc_adapter_reports_non_speech_for_silence() {
        let mut runtime = MicrophoneVadRuntime::new(MicrophoneVadAdapter::Webrtc);
        let frame = MicrophonePcmVadFrame {
            samples: &[0; 160],
            sample_rate_hz: 16_000,
            captured_at_unix_ms: 1_000,
            normalized_peak_level: 0.0,
        };

        let outcome = runtime
            .process_pcm_frame(frame)
            .expect("silence is a valid WebRTC frame");

        assert_eq!(outcome.decision.idle_ms, None);
        assert_eq!(outcome.decision.latest_normalized_level, None);
        assert!(!outcome.vad_speech_detected);
    }

    #[test]
    fn effective_webrtc_non_speech_is_not_overridden_by_peak_level() {
        let mut runtime = MicrophoneVadRuntime::new(MicrophoneVadAdapter::Webrtc);
        runtime
            .process_pcm_frame(MicrophonePcmVadFrame {
                samples: &[0; 160],
                sample_rate_hz: 16_000,
                captured_at_unix_ms: 1_000,
                normalized_peak_level: 0.0,
            })
            .expect("silence is a valid WebRTC frame");

        let decision = runtime.decide_from_peak_level(Some(1.0), Some(0), 0.1);

        assert_eq!(decision.idle_ms, None);
        assert_eq!(decision.latest_normalized_level, None);
    }

    #[test]
    fn webrtc_adapter_reports_speech_for_voiced_like_frame() {
        let mut runtime = MicrophoneVadRuntime::new(MicrophoneVadAdapter::Webrtc);
        let samples = voiced_like_frame_16khz_30ms();
        let frame = MicrophonePcmVadFrame {
            samples: &samples,
            sample_rate_hz: 16_000,
            captured_at_unix_ms: 1_000,
            normalized_peak_level: 0.7,
        };

        let outcome = runtime
            .process_pcm_frame(frame)
            .expect("voiced-like samples are a valid WebRTC frame");

        assert_eq!(outcome.decision.idle_ms, Some(0));
        assert_eq!(outcome.decision.latest_normalized_level, Some(1.0));
        assert!(outcome.vad_speech_detected);
    }

    #[test]
    fn webrtc_processing_error_degrades_to_peak_level_decisions() {
        let mut runtime = MicrophoneVadRuntime::new(MicrophoneVadAdapter::Webrtc);
        let frame = MicrophonePcmVadFrame {
            samples: &[0; 160],
            sample_rate_hz: 44_100,
            captured_at_unix_ms: 1_000,
            normalized_peak_level: 0.0,
        };

        assert_eq!(
            runtime.process_pcm_frame(frame),
            Err(MicrophoneVadError::InvalidSampleRate(44_100))
        );
        assert_eq!(
            runtime.effective_adapter(),
            EffectiveMicrophoneVadAdapter::PeakLevel
        );
        assert!(runtime
            .fallback_reason()
            .expect("fallback reason")
            .contains("using peak-level microphone activity"));

        let decision = runtime.decide_from_peak_level(Some(0.8), Some(0), 0.1);

        assert_eq!(decision.idle_ms, Some(0));
        assert_eq!(decision.latest_normalized_level, Some(0.8));
    }

    #[test]
    fn webrtc_adapter_rejects_invalid_sample_rate() {
        let mut runtime = MicrophoneVadRuntime::new(MicrophoneVadAdapter::Webrtc);
        let frame = MicrophonePcmVadFrame {
            samples: &[0; 160],
            sample_rate_hz: 44_100,
            captured_at_unix_ms: 1_000,
            normalized_peak_level: 0.0,
        };

        assert_eq!(
            runtime.process_pcm_frame(frame),
            Err(MicrophoneVadError::InvalidSampleRate(44_100))
        );
    }

    #[test]
    fn webrtc_adapter_rejects_invalid_frame_length() {
        let mut runtime = MicrophoneVadRuntime::new(MicrophoneVadAdapter::Webrtc);
        let frame = MicrophonePcmVadFrame {
            samples: &[0; 159],
            sample_rate_hz: 16_000,
            captured_at_unix_ms: 1_000,
            normalized_peak_level: 0.0,
        };

        assert_eq!(
            runtime.process_pcm_frame(frame),
            Err(MicrophoneVadError::InvalidFrameLength {
                sample_rate_hz: 16_000,
                sample_count: 159
            })
        );
    }

    #[test]
    fn webrtc_adapter_reports_unavailable_backend() {
        let mut adapter = WebrtcVadAdapter::unavailable("test WebRTC load failure");
        let frame = MicrophonePcmVadFrame {
            samples: &[0; 160],
            sample_rate_hz: 16_000,
            captured_at_unix_ms: 1_000,
            normalized_peak_level: 0.0,
        };

        assert_eq!(
            adapter.process_pcm_frame(&frame),
            Err(MicrophoneVadError::AdapterUnavailable {
                adapter: EffectiveMicrophoneVadAdapter::Webrtc,
                reason: "test WebRTC load failure".to_string()
            })
        );
    }

    fn voiced_like_frame_16khz_30ms() -> Vec<i16> {
        (0..480)
            .map(|sample_index| {
                let phase = sample_index % 160;
                let envelope = if phase < 80 { phase } else { 160 - phase };
                let carrier = if sample_index % 32 < 16 { 1 } else { -1 };
                (carrier * envelope as i32 * 220) as i16
            })
            .collect()
    }
}
