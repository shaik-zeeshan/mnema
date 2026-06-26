//! Windows audio decode for speaker analysis.
//!
//! Mirrors the macOS AVFoundation decode entry (`macos_audio_decode` +
//! `providers::shared::decode_audio_to_mono_16khz`) but routes through the shared
//! `media-decode` Media Foundation seam (ADR 0024): it decodes a captured media
//! file to native-rate mono `f32` and resamples to the fixed analysis rate of
//! 16 kHz, feeding the same mono-16-kHz contract the platform-agnostic pipeline
//! consumes.
//!
//! This module is gated only on `target_os = "windows"` — NOT on the `speakrs`
//! feature — so plain `cargo check`/`cargo test` on Windows build and exercise
//! the decode path before the native speakrs CPU backend is wired into dispatch
//! (#135), which cannot be enabled on Windows yet.

use std::path::Path;

use crate::{SpeakerAnalysisError, SpeakerAnalysisResult};

/// Fixed analysis sample rate. Mirrors `providers::shared::SAMPLE_RATE_HZ`, which
/// is `speakrs`-feature-gated; kept local so this module compiles with no
/// features.
const SAMPLE_RATE_HZ: u32 = 16_000;

/// Decode an audio file to mono 16 kHz `f32` via the Media Foundation seam.
///
/// A decode failure is a **Speaker Analysis Job** failure (per CONTEXT.md), not a
/// provider-availability problem, so it maps to
/// [`SpeakerAnalysisError::Runtime`] — never `ProviderUnavailable`. The
/// native-rate mono output is resampled to 16 kHz with the same linear resampler
/// the macOS path uses.
#[allow(dead_code)] // Wired into dispatch by #135; #134 ships the substrate + tests.
pub(crate) fn decode_audio_to_mono_16khz(path: &Path) -> SpeakerAnalysisResult<Vec<f32>> {
    let decoded = media_decode::decode_to_mono_f32(path).map_err(|error| {
        SpeakerAnalysisError::Runtime {
            stage: "windows_audio_decode".to_string(),
            message: format!(
                "Media Foundation decode failed for {}: {error}",
                path.display()
            ),
        }
    })?;
    Ok(resample_linear(
        &decoded.samples,
        decoded.sample_rate_hz,
        SAMPLE_RATE_HZ,
    ))
}

/// Linear-interpolation resampler from `source_rate_hz` to `target_rate_hz`,
/// clamping output to the `[-1.0, 1.0]` PCM range. Mirrors
/// `macos_audio_decode::resample_linear` but is duplicated here so this module
/// does not depend on the macOS-gated decode module. Passes empty input and an
/// already-matching rate through unchanged.
#[allow(dead_code)] // Used by `decode_audio_to_mono_16khz` (wired in by #135) + tests.
fn resample_linear(samples: &[f32], source_rate_hz: u32, target_rate_hz: u32) -> Vec<f32> {
    if samples.is_empty() || source_rate_hz == 0 || source_rate_hz == target_rate_hz {
        return samples.to_vec();
    }

    let ratio = source_rate_hz as f64 / target_rate_hz as f64;
    let out_len = ((samples.len() as f64) / ratio).ceil().max(1.0) as usize;
    let mut out = Vec::with_capacity(out_len);
    for out_index in 0..out_len {
        let source_pos = out_index as f64 * ratio;
        let left = source_pos.floor() as usize;
        let right = (left + 1).min(samples.len().saturating_sub(1));
        let frac = (source_pos - left as f64) as f32;
        let sample = samples[left] * (1.0 - frac) + samples[right] * frac;
        out.push(sample.clamp(-1.0, 1.0));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal canonical 44-byte-header mono 16-bit PCM WAV. Media
    /// Foundation reads WAV natively, so this needs no AAC fixture/codec.
    /// Replicates the helper in `media-decode`'s own tests.
    fn build_mono_pcm16_wav(sample_rate_hz: u32, samples: &[i16]) -> Vec<u8> {
        let channels: u16 = 1;
        let bits_per_sample: u16 = 16;
        let block_align: u16 = channels * bits_per_sample / 8;
        let byte_rate: u32 = sample_rate_hz * block_align as u32;
        let data_len: u32 = (samples.len() * 2) as u32;

        let mut wav = Vec::with_capacity(44 + data_len as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVE");
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
        wav.extend_from_slice(&1u16.to_le_bytes()); // WAVE_FORMAT_PCM
        wav.extend_from_slice(&channels.to_le_bytes());
        wav.extend_from_slice(&sample_rate_hz.to_le_bytes());
        wav.extend_from_slice(&byte_rate.to_le_bytes());
        wav.extend_from_slice(&block_align.to_le_bytes());
        wav.extend_from_slice(&bits_per_sample.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        for sample in samples {
            wav.extend_from_slice(&sample.to_le_bytes());
        }
        wav
    }

    #[test]
    fn resamples_to_target_rate() {
        let samples = vec![0.0, 1.0, 0.0, -1.0];
        let out = resample_linear(&samples, 4, 2);
        assert_eq!(out.len(), 2);
        assert!((out[0] - 0.0).abs() < 0.0001);
        assert!((out[1] - 0.0).abs() < 0.0001);
    }

    #[test]
    fn resample_passes_through_equal_rate_and_empty_input() {
        assert_eq!(
            resample_linear(&[0.1, -0.2], 16_000, 16_000),
            vec![0.1, -0.2]
        );
        assert!(resample_linear(&[], 8_000, 16_000).is_empty());
        // A zero source rate is degenerate; pass through rather than divide by 0.
        assert_eq!(resample_linear(&[0.3], 0, 16_000), vec![0.3]);
    }

    // End-to-end Media Foundation decode + resample. Synthesizes a self-contained
    // 8 kHz mono 16-bit PCM WAV (MF reads WAV natively, no AAC fixture needed) and
    // decodes it back through the real seam, proving open -> negotiate Float
    // output -> read every sample -> downmix -> resample to 16 kHz. On-device
    // decode of a captured `.m4a` is the operator-deferred gap (same as the
    // media-decode and local_whisper seam tests).
    #[test]
    fn decodes_wav_to_mono_16khz() {
        // 8 kHz source so the resample to 16 kHz is observable (~doubles count).
        let source_rate_hz = 8_000u32;
        let frames = 8_000usize; // exactly 1 second of audio.
        // A gentle ramp comfortably within [-1, 1] after i16 -> f32 scaling.
        let pcm_i16: Vec<i16> = (0..frames).map(|i| ((i % 100) as i16) * 100).collect();
        let wav = build_mono_pcm16_wav(source_rate_hz, &pcm_i16);

        let path = std::env::temp_dir().join(format!(
            "speaker-analysis-winaudio-{}.wav",
            std::process::id()
        ));
        std::fs::write(&path, &wav).expect("write temp wav");

        let result = decode_audio_to_mono_16khz(&path);
        let _ = std::fs::remove_file(&path);
        let samples = result.expect("MF seam should decode + resample a mono PCM WAV");

        // 1 s of 8 kHz audio resampled to 16 kHz is ~16_000 samples; allow a small
        // tolerance for resampler/decoder boundary effects.
        let expected = 16_000usize;
        let tolerance = 128usize;
        assert!(
            samples.len().abs_diff(expected) <= tolerance,
            "expected ~{expected} samples (+/-{tolerance}) for 1 s @ 16 kHz, got {}",
            samples.len()
        );
        assert!(
            samples
                .iter()
                .all(|sample| sample.is_finite() && (-1.0..=1.0).contains(sample)),
            "every resampled sample must be finite and in [-1, 1]"
        );
    }

    #[test]
    fn rejects_garbage_file() {
        let path = std::env::temp_dir().join(format!(
            "speaker-analysis-winaudio-garbage-{}.bin",
            std::process::id()
        ));
        std::fs::write(&path, b"this is not audio, just garbage bytes")
            .expect("write garbage fixture");

        let result = decode_audio_to_mono_16khz(&path);
        let _ = std::fs::remove_file(&path);

        // A malformed file is a decode failure (Err), not a panic, and maps to the
        // Runtime job-failure variant — never ProviderUnavailable.
        let error = result.expect_err("garbage bytes must be rejected, not decoded");
        assert!(matches!(
            error,
            SpeakerAnalysisError::Runtime { ref stage, .. } if stage == "windows_audio_decode"
        ));
    }
}
