//! Shared media-processing seam for Windows decode/extraction work.
//!
//! Per ADR 0024 (`docs/adr/0024-media-foundation-is-the-only-windows-media-backend.md`)
//! Media Foundation is the single Windows media backend — no FFmpeg, no
//! Symphonia. This crate owns the Windows decode side so that processing crates
//! (transcription, speaker analysis, system-audio speech activity, …) depend on
//! one decode seam rather than reaching into a capture crate. No capture crate
//! grows a decoder, and no processing crate depends on a capture crate.
//!
//! The first seam exposed here is [`decode_to_mono_f32`], which decodes a media
//! file's audio to **native-rate mono `f32`** using the MF Source Reader (the
//! same COM/threading idioms `capture-writers`' `WindowsAacM4aSinkWriter` and the
//! capture backends already use). The seam deliberately does **not** resample —
//! consumers keep their existing in-crate resamplers and resample the
//! native-rate output themselves.
//!
//! macOS audio decoders are intentionally **out of scope**: the existing
//! AVFoundation paths in `audio-transcription`, `speaker-analysis`, and
//! `capture-writers` are left untouched. On non-Windows targets this seam
//! returns [`MediaDecodeError::UnsupportedPlatform`] so callers can `cfg`-gate
//! cleanly without a build break.

use std::fmt;
use std::path::Path;

/// Native-rate mono PCM decoded from a media file.
///
/// `samples` is single-channel `f32` in `[-1.0, 1.0]` at `sample_rate_hz`; it is
/// **not** resampled — the decode seam returns the source's native rate and
/// leaves resampling to the consumer.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedAudio {
    /// Mono `f32` samples (downmixed across the source channels).
    pub samples: Vec<f32>,
    /// The source's native sample rate in Hz (never `0`).
    pub sample_rate_hz: u32,
}

/// Errors returned by the media-decode seam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaDecodeError {
    /// The path could not be expressed for the platform decode API (e.g. not
    /// valid UTF-8 / UTF-16-encodable).
    InvalidPath(String),
    /// The decode backend failed. `message` carries the backend context and the
    /// underlying error.
    Decode(String),
    /// This platform has no media-decode backend wired up. macOS keeps its
    /// existing AVFoundation decoders; only Windows is implemented here.
    UnsupportedPlatform(String),
}

impl fmt::Display for MediaDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath(message) => write!(f, "invalid media path: {message}"),
            Self::Decode(message) => write!(f, "media decode failed: {message}"),
            Self::UnsupportedPlatform(message) => {
                write!(f, "media decode unsupported on this platform: {message}")
            }
        }
    }
}

impl std::error::Error for MediaDecodeError {}

/// Result alias for the media-decode seam.
pub type Result<T> = std::result::Result<T, MediaDecodeError>;

/// Decode the audio track of `path` to **native-rate mono `f32`**.
///
/// On Windows this uses the MF Source Reader to decode the first audio stream to
/// uncompressed float PCM, then averages across channels into a single mono
/// channel. The returned [`DecodedAudio::sample_rate_hz`] is the source's native
/// rate; callers that need a fixed rate (e.g. 16 kHz for VAD/Whisper) resample
/// the result themselves.
///
/// On non-Windows targets this returns [`MediaDecodeError::UnsupportedPlatform`]
/// — the macOS decoders live in their existing crates and are out of scope for
/// this seam.
pub fn decode_to_mono_f32(path: impl AsRef<Path>) -> Result<DecodedAudio> {
    decode_to_mono_f32_inner(path.as_ref())
}

#[cfg(target_os = "windows")]
fn decode_to_mono_f32_inner(path: &Path) -> Result<DecodedAudio> {
    windows_mf::decode_to_mono_f32(path)
}

#[cfg(not(target_os = "windows"))]
fn decode_to_mono_f32_inner(path: &Path) -> Result<DecodedAudio> {
    Err(MediaDecodeError::UnsupportedPlatform(format!(
        "decode_to_mono_f32 is only implemented on Windows (Media Foundation); cannot decode {}",
        path.display()
    )))
}

/// Downmix interleaved `f32` frames to mono by averaging across `channels`.
///
/// Shared between the MF reader and unit tests; lives at module scope so it can
/// be exercised without a Media Foundation backend. `samples` is interleaved
/// (`frame0_ch0, frame0_ch1, …`); a trailing partial frame is ignored.
fn downmix_interleaved_f32_to_mono(samples: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return samples.to_vec();
    }
    let frame_count = samples.len() / channels;
    let mut mono = Vec::with_capacity(frame_count);
    for frame in 0..frame_count {
        let base = frame * channels;
        let mut sum = 0.0f32;
        for channel in 0..channels {
            sum += samples[base + channel];
        }
        mono.push((sum / channels as f32).clamp(-1.0, 1.0));
    }
    mono
}

#[cfg(target_os = "windows")]
mod windows_mf {
    //! MF Source Reader audio decode.
    //!
    //! Mirrors the COM/Media-Foundation lifecycle of `capture-writers`'
    //! `WindowsAacM4aSinkWriter`: `MFStartup` is called defensively (reference
    //! counted) and balanced by `MFShutdown` on the same path, all COM state is
    //! single-thread-affine and never leaves this function, and errors map
    //! through a small `win_error` helper. We configure the reader's output type
    //! to uncompressed IEEE-float PCM, read every audio sample, and average the
    //! interleaved channels down to mono.

    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows::core::PCWSTR;
    use windows::Win32::Media::MediaFoundation::{
        IMFSourceReader, MFCreateMediaType, MFCreateSourceReaderFromURL, MFMediaType_Audio,
        MFShutdown, MFStartup, MFAudioFormat_Float, MFSTARTUP_FULL, MF_MT_AUDIO_NUM_CHANNELS,
        MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE,
        MF_SOURCE_READER_FIRST_AUDIO_STREAM, MF_VERSION,
    };

    use crate::{downmix_interleaved_f32_to_mono, DecodedAudio, MediaDecodeError, Result};

    pub(crate) fn decode_to_mono_f32(path: &Path) -> Result<DecodedAudio> {
        let url: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        // A path with no characters would yield a single NUL terminator.
        if url.len() <= 1 {
            return Err(MediaDecodeError::InvalidPath(format!(
                "empty media path: {}",
                path.display()
            )));
        }

        unsafe {
            // Defensive, reference-counted MFStartup balanced by the MFShutdown
            // below — the same pattern as the capture writers' standalone probe.
            MFStartup(MF_VERSION, MFSTARTUP_FULL)
                .map_err(|e| win_error("MFStartup failed", &e))?;
            let result = decode_with_reader(&url, path);
            MFShutdown().ok();
            result
        }
    }

    unsafe fn decode_with_reader(url: &[u16], path: &Path) -> Result<DecodedAudio> {
        let reader: IMFSourceReader =
            MFCreateSourceReaderFromURL(PCWSTR(url.as_ptr()), None)
                .map_err(|e| win_error("MFCreateSourceReaderFromURL failed", &e))?;

        let stream_index = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;

        // Ask MF to decode the compressed audio to uncompressed IEEE-float PCM.
        // We leave sample rate / channel count unset so MF preserves the source's
        // native rate and channel layout; we read both back afterwards.
        let output_type = MFCreateMediaType()
            .map_err(|e| win_error("MFCreateMediaType (Float output) failed", &e))?;
        output_type
            .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
            .map_err(|e| win_error("set output major type failed", &e))?;
        output_type
            .SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float)
            .map_err(|e| win_error("set output subtype failed", &e))?;
        reader
            .SetCurrentMediaType(stream_index, None, &output_type)
            .map_err(|e| win_error("SetCurrentMediaType (Float) failed", &e))?;

        // Read back the negotiated output type to learn the native sample rate
        // and channel count MF kept.
        let actual_type = reader
            .GetCurrentMediaType(stream_index)
            .map_err(|e| win_error("GetCurrentMediaType failed", &e))?;
        let sample_rate_hz = actual_type
            .GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)
            .map_err(|e| win_error("GetUINT32(samples per second) failed", &e))?;
        let channels = actual_type
            .GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)
            .map_err(|e| win_error("GetUINT32(num channels) failed", &e))?;
        if sample_rate_hz == 0 {
            return Err(MediaDecodeError::Decode(format!(
                "Media Foundation reported a zero sample rate for {}",
                path.display()
            )));
        }
        if channels == 0 {
            return Err(MediaDecodeError::Decode(format!(
                "Media Foundation reported zero channels for {}",
                path.display()
            )));
        }
        let channels = channels as usize;

        let mut interleaved: Vec<f32> = Vec::new();
        loop {
            let mut stream_flags: u32 = 0;
            let mut sample = None;
            reader
                .ReadSample(
                    stream_index,
                    0,
                    None,
                    Some(&mut stream_flags),
                    None,
                    Some(&mut sample),
                )
                .map_err(|e| win_error("ReadSample failed", &e))?;

            // End of stream: no more samples to read.
            if (stream_flags & MF_SOURCE_READER_FLAG_ENDOFSTREAM) != 0 {
                break;
            }

            let Some(sample) = sample else {
                // A flag-only callback (e.g. a stream tick) with no buffer; keep
                // reading until end-of-stream.
                continue;
            };

            append_sample_f32(&sample, &mut interleaved)?;
        }

        let samples = downmix_interleaved_f32_to_mono(&interleaved, channels);
        Ok(DecodedAudio {
            samples,
            sample_rate_hz,
        })
    }

    /// `ENDOFSTREAM` bit of the `ReadSample` stream-flags out-parameter; the
    /// `windows` crate surfaces the flags as a raw `u32` without this named
    /// constant, so define it locally (`MF_SOURCE_READERF_ENDOFSTREAM`).
    const MF_SOURCE_READER_FLAG_ENDOFSTREAM: u32 = 0x2;

    /// Copy one decoded float sample's contiguous buffer into `out` as `f32`.
    unsafe fn append_sample_f32(
        sample: &windows::Win32::Media::MediaFoundation::IMFSample,
        out: &mut Vec<f32>,
    ) -> Result<()> {
        let buffer = sample
            .ConvertToContiguousBuffer()
            .map_err(|e| win_error("ConvertToContiguousBuffer failed", &e))?;

        let mut data_ptr: *mut u8 = std::ptr::null_mut();
        let mut current_len: u32 = 0;
        buffer
            .Lock(&mut data_ptr, None, Some(&mut current_len))
            .map_err(|e| win_error("IMFMediaBuffer.Lock failed", &e))?;

        // Read exactly the whole-f32 prefix of the locked region; a trailing
        // partial float (which MF never emits for float PCM) is ignored.
        let float_count = (current_len as usize) / std::mem::size_of::<f32>();
        if float_count > 0 && !data_ptr.is_null() {
            let floats = std::slice::from_raw_parts(data_ptr as *const f32, float_count);
            out.extend_from_slice(floats);
        }

        // Always unlock, even if we copied nothing.
        let unlock = buffer.Unlock();
        unlock.map_err(|e| win_error("IMFMediaBuffer.Unlock failed", &e))?;
        Ok(())
    }

    fn win_error(context: &str, error: &windows::core::Error) -> MediaDecodeError {
        MediaDecodeError::Decode(format!("{context}: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmix_mono_passthrough() {
        let samples = vec![0.1, -0.2, 0.3];
        assert_eq!(downmix_interleaved_f32_to_mono(&samples, 1), samples);
    }

    #[test]
    fn downmix_stereo_averages_channels() {
        // frame0: (0.2, 0.4) -> 0.3 ; frame1: (-0.5, 0.1) -> -0.2
        let samples = vec![0.2, 0.4, -0.5, 0.1];
        let mono = downmix_interleaved_f32_to_mono(&samples, 2);
        assert_eq!(mono.len(), 2);
        assert!((mono[0] - 0.3).abs() < 1e-6);
        assert!((mono[1] - (-0.2)).abs() < 1e-6);
    }

    #[test]
    fn downmix_ignores_trailing_partial_frame() {
        // Three samples at 2 channels => one full frame, one dangling sample.
        let samples = vec![0.2, 0.4, 0.9];
        let mono = downmix_interleaved_f32_to_mono(&samples, 2);
        assert_eq!(mono.len(), 1);
        assert!((mono[0] - 0.3).abs() < 1e-6);
    }

    #[test]
    fn downmix_clamps_out_of_range_average() {
        // Average exceeds 1.0; result clamps to the f32 PCM range.
        let samples = vec![2.0, 2.0];
        let mono = downmix_interleaved_f32_to_mono(&samples, 2);
        assert_eq!(mono, vec![1.0]);
    }

    #[test]
    fn error_display_is_descriptive() {
        let error = MediaDecodeError::Decode("ReadSample failed".to_string());
        assert!(error.to_string().contains("ReadSample failed"));
        let unsupported = MediaDecodeError::UnsupportedPlatform("nope".to_string());
        assert!(unsupported.to_string().contains("unsupported"));
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn decode_is_unsupported_off_windows() {
        let error = decode_to_mono_f32("/tmp/whatever.m4a")
            .expect_err("non-Windows decode must report unsupported");
        assert!(matches!(error, MediaDecodeError::UnsupportedPlatform(_)));
    }

    // End-to-end MF Source Reader decode test. It synthesizes a self-contained
    // mono 16-bit PCM WAV (MF reads WAV natively, no codec/fixture needed) and
    // decodes it back through the real seam, proving the source-reader plumbing
    // (open, negotiate Float output, read every sample, downmix). This stands in
    // for an `.m4a`/`.mp4` fixture, which would still exercise the same code path
    // but requires the AAC decoder; on-device decode of a captured `.m4a` is the
    // operator-deferred gap.
    #[cfg(target_os = "windows")]
    #[test]
    fn decode_wav_round_trips_through_mf_source_reader() {
        let sample_rate_hz = 16_000u32;
        // Two frames of full-scale +/- to make downmix/scaling observable.
        let pcm_i16: [i16; 2] = [i16::MAX, i16::MIN];
        let wav = build_mono_pcm16_wav(sample_rate_hz, &pcm_i16);

        let path = std::env::temp_dir().join(format!(
            "media-decode-test-{}.wav",
            std::process::id()
        ));
        std::fs::write(&path, &wav).expect("write temp wav");

        let decoded = decode_to_mono_f32(&path);
        let _ = std::fs::remove_file(&path);
        let decoded = decoded.expect("MF Source Reader should decode a mono PCM WAV");

        assert_eq!(decoded.sample_rate_hz, sample_rate_hz);
        assert_eq!(decoded.samples.len(), pcm_i16.len());
        // Full-scale +/- map close to +/-1.0 after float conversion.
        assert!(decoded.samples[0] > 0.9, "expected ~+1.0, got {}", decoded.samples[0]);
        assert!(decoded.samples[1] < -0.9, "expected ~-1.0, got {}", decoded.samples[1]);
    }

    /// Build a minimal canonical 44-byte-header mono 16-bit PCM WAV.
    #[cfg(target_os = "windows")]
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
}
