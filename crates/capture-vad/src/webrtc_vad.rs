use crate::{EffectiveMicrophoneVadAdapter, MicrophonePcmVadFrame, MicrophoneVadError, VadAdapter};
use ::webrtc_vad::{SampleRate, Vad, VadMode};
use std::fmt;

pub(super) struct WebrtcVadAdapter {
    backend: WebrtcVadBackend,
}

// The adapter is owned by NativeCaptureRuntime and only accessed behind the
// runtime mutex. Moving that owner between threads is safe; concurrent access
// still requires the mutex and mutable methods.
unsafe impl Send for WebrtcVadAdapter {}

impl fmt::Debug for WebrtcVadAdapter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebrtcVadAdapter")
            .field(
                "available",
                &matches!(self.backend, WebrtcVadBackend::Available(_)),
            )
            .finish()
    }
}

impl WebrtcVadAdapter {
    pub(super) fn new() -> Self {
        Self {
            backend: WebrtcVadBackend::Available(Vad::new_with_rate_and_mode(
                SampleRate::Rate16kHz,
                VadMode::Aggressive,
            )),
        }
    }

    #[cfg(test)]
    pub(super) fn unavailable(reason: &'static str) -> Self {
        Self {
            backend: WebrtcVadBackend::Unavailable(reason),
        }
    }
}

impl VadAdapter for WebrtcVadAdapter {
    fn process_pcm_frame(
        &mut self,
        frame: &MicrophonePcmVadFrame<'_>,
    ) -> Result<Option<bool>, MicrophoneVadError> {
        let sample_rate = sample_rate_for_hz(frame.sample_rate_hz)?;
        validate_webrtc_frame_length(frame.sample_rate_hz, frame.samples.len())?;

        let vad = match self.backend {
            WebrtcVadBackend::Available(ref mut vad) => vad,
            WebrtcVadBackend::Unavailable(reason) => {
                return Err(MicrophoneVadError::AdapterUnavailable {
                    adapter: EffectiveMicrophoneVadAdapter::Webrtc,
                    reason: reason.to_string(),
                });
            }
        };

        vad.set_sample_rate(sample_rate);
        vad.is_voice_segment(frame.samples)
            .map(Some)
            .map_err(|()| MicrophoneVadError::WebrtcProcessFailed)
    }
}

enum WebrtcVadBackend {
    Available(Vad),
    #[allow(dead_code)]
    Unavailable(&'static str),
}

fn sample_rate_for_hz(sample_rate_hz: u32) -> Result<SampleRate, MicrophoneVadError> {
    match sample_rate_hz {
        8_000 => Ok(SampleRate::Rate8kHz),
        16_000 => Ok(SampleRate::Rate16kHz),
        32_000 => Ok(SampleRate::Rate32kHz),
        48_000 => Ok(SampleRate::Rate48kHz),
        _ => Err(MicrophoneVadError::InvalidSampleRate(sample_rate_hz)),
    }
}

fn validate_webrtc_frame_length(
    sample_rate_hz: u32,
    sample_count: usize,
) -> Result<(), MicrophoneVadError> {
    let valid = [10, 20, 30]
        .into_iter()
        .any(|duration_ms| sample_count == (sample_rate_hz as usize * duration_ms / 1_000));

    if valid {
        Ok(())
    } else {
        Err(MicrophoneVadError::InvalidFrameLength {
            sample_rate_hz,
            sample_count,
        })
    }
}
