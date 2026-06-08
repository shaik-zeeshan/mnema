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

/// A single decoded video frame as CPU-readable pixels.
///
/// Pixels are tightly packed (no row padding) top-down RGBA8 (`R, G, B, A` per
/// pixel, row-major), `width * height * 4` bytes. RGBA is the format the
/// preview/`image`-crate layer wants; the MF backend negotiates a CPU-readable
/// 32-bit format (`RGB32`) and converts to straight RGBA here, so consumers do
/// not see the platform's pixel layout. `presented_offset_ms` is the timestamp
/// of the frame MF actually returned after seek + decode-forward (see
/// [`extract_video_frame_rgba`]); it can differ from the requested offset by
/// less than one inter-frame interval when the source has no sample at the
/// exact target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFrame {
    /// Tightly packed top-down RGBA8 pixels, `width * height * 4` bytes.
    pub pixels: Vec<u8>,
    /// Frame width in pixels (never `0`).
    pub width: u32,
    /// Frame height in pixels (never `0`).
    pub height: u32,
    /// Presentation timestamp of the returned frame in milliseconds.
    pub presented_offset_ms: u64,
}

impl VideoFrame {
    /// Encode this frame as a JPEG at `quality` (1–100) using the `image` crate.
    ///
    /// This is the JPEG-only Windows-v1 preview rendition (ADR 0024 / issue
    /// #81): lossy WebP is out of scope. The convenience lives here so both the
    /// exact-preview fallback and the future scrub-preview slice (#83) share one
    /// encoder; consumers that need to resize first can read [`VideoFrame::pixels`]
    /// and own their own encode instead.
    pub fn encode_jpeg(&self, quality: u8) -> Result<Vec<u8>> {
        let buffer: image::RgbaImage =
            image::ImageBuffer::from_raw(self.width, self.height, self.pixels.clone()).ok_or_else(
                || {
                    MediaDecodeError::Decode(format!(
                        "video frame pixel buffer ({} bytes) does not match {}x{} RGBA",
                        self.pixels.len(),
                        self.width,
                        self.height
                    ))
                },
            )?;
        // JPEG has no alpha channel; drop it to RGB before encoding.
        let rgb = image::DynamicImage::ImageRgba8(buffer).to_rgb8();
        let mut out = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
        encoder
            .encode_image(&image::DynamicImage::ImageRgb8(rgb))
            .map_err(|error| {
                MediaDecodeError::Decode(format!("failed to JPEG-encode video frame: {error}"))
            })?;
        Ok(out)
    }
}

/// Timing/openability inspection of a finalized video's first video stream.
///
/// This is the MF equivalent of the macOS `AVAssetReader` finalized-video
/// validation (issue #81): it proves the container opens, carries a decodable
/// video stream, and reports positive timing — a stronger guarantee than the
/// byte-level `moov` openability probe alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoInfo {
    /// Total presentation duration in milliseconds (never `0`).
    pub duration_ms: u64,
    /// First video stream frame width in pixels (never `0`).
    pub width: u32,
    /// First video stream frame height in pixels (never `0`).
    pub height: u32,
}

/// Extract the video frame at (or, when MF lands on an earlier keyframe,
/// decode-forward to) `target_offset_ms` as CPU-readable RGBA pixels.
///
/// MF seeking lands on **keyframes**, so this seeks to (or before) the target
/// with `SetCurrentPosition`, then decodes forward sample-by-sample, keeping the
/// last frame whose presentation timestamp is `<= target`, and stops at the
/// first frame past the target — the same seek + reconcile the macOS
/// `AVAssetImageGenerator` path and the binary frame-index sidecar perform. When
/// the source has no sample at or before the target (e.g. target `0` on a stream
/// whose first sample is slightly later) the first decoded frame is returned.
///
/// On non-Windows targets this returns [`MediaDecodeError::UnsupportedPlatform`]
/// — macOS keeps `AVAssetImageGenerator` and is out of scope for this seam.
pub fn extract_video_frame_rgba(
    path: impl AsRef<Path>,
    target_offset_ms: u64,
) -> Result<VideoFrame> {
    extract_video_frame_rgba_inner(path.as_ref(), target_offset_ms)
}

/// Inspect a finalized video's first video stream timing/openability through MF.
///
/// Returns [`VideoInfo`] iff the container opens, exposes a video stream, and
/// reports positive duration and frame dimensions. On non-Windows targets this
/// returns [`MediaDecodeError::UnsupportedPlatform`].
pub fn inspect_video(path: impl AsRef<Path>) -> Result<VideoInfo> {
    inspect_video_inner(path.as_ref())
}

#[cfg(target_os = "windows")]
fn extract_video_frame_rgba_inner(path: &Path, target_offset_ms: u64) -> Result<VideoFrame> {
    windows_mf_video::extract_video_frame_rgba(path, target_offset_ms)
}

#[cfg(not(target_os = "windows"))]
fn extract_video_frame_rgba_inner(path: &Path, _target_offset_ms: u64) -> Result<VideoFrame> {
    Err(MediaDecodeError::UnsupportedPlatform(format!(
        "extract_video_frame_rgba is only implemented on Windows (Media Foundation); cannot extract from {}",
        path.display()
    )))
}

#[cfg(target_os = "windows")]
fn inspect_video_inner(path: &Path) -> Result<VideoInfo> {
    windows_mf_video::inspect_video(path)
}

#[cfg(not(target_os = "windows"))]
fn inspect_video_inner(path: &Path) -> Result<VideoInfo> {
    Err(MediaDecodeError::UnsupportedPlatform(format!(
        "inspect_video is only implemented on Windows (Media Foundation); cannot inspect {}",
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

#[cfg(target_os = "windows")]
mod windows_mf_video {
    //! MF Source Reader **video** frame extraction and finalized-video timing
    //! inspection.
    //!
    //! Mirrors the COM/Media-Foundation lifecycle of the audio seam above and of
    //! `capture-writers`' sink writers: `MFStartup` is reference-counted and
    //! balanced by `MFShutdown` on the same path, every COM object is
    //! single-thread-affine and never leaves the call, and errors map through the
    //! same small `win_error` helper.
    //!
    //! Extraction negotiates the first video stream to uncompressed 32-bit
    //! `RGB32` (top-down, B,G,R,X byte order in memory), seeks to (or before) the
    //! target via `SetCurrentPosition`, decodes forward to the target timestamp —
    //! MF seeking lands on keyframes, so we must walk forward — and converts the
    //! kept frame to straight RGBA so the caller never sees the BGRX layout.

    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows::core::{GUID, PCWSTR};
    use windows::Win32::Media::MediaFoundation::{
        IMFSourceReader, MFCreateAttributes, MFCreateSourceReaderFromURL, MFShutdown, MFStartup,
        MFVideoFormat_RGB32, MFMediaType_Video, MFSTARTUP_FULL, MF_MT_DEFAULT_STRIDE,
        MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_PD_DURATION,
        MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, MF_SOURCE_READER_FIRST_VIDEO_STREAM,
        MF_SOURCE_READER_MEDIASOURCE, MF_VERSION,
    };
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Variant::VT_I8;

    use crate::{MediaDecodeError, Result, VideoFrame, VideoInfo};

    /// `ENDOFSTREAM` bit of the `ReadSample` stream-flags out-parameter (same
    /// constant the audio seam defines locally; the `windows` crate surfaces the
    /// flags as a raw `u32`).
    const MF_SOURCE_READER_FLAG_ENDOFSTREAM: u32 = 0x2;

    /// Media Foundation measures time in 100ns ticks.
    const HUNDRED_NS_PER_MS: i64 = 10_000;

    pub(crate) fn extract_video_frame_rgba(
        path: &Path,
        target_offset_ms: u64,
    ) -> Result<VideoFrame> {
        let url = encode_url(path)?;
        unsafe {
            MFStartup(MF_VERSION, MFSTARTUP_FULL)
                .map_err(|e| win_error("MFStartup failed", &e))?;
            let result = extract_with_reader(&url, path, target_offset_ms);
            MFShutdown().ok();
            result
        }
    }

    pub(crate) fn inspect_video(path: &Path) -> Result<VideoInfo> {
        let url = encode_url(path)?;
        unsafe {
            MFStartup(MF_VERSION, MFSTARTUP_FULL)
                .map_err(|e| win_error("MFStartup failed", &e))?;
            let result = inspect_with_reader(&url, path);
            MFShutdown().ok();
            result
        }
    }

    fn encode_url(path: &Path) -> Result<Vec<u16>> {
        let url: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        if url.len() <= 1 {
            return Err(MediaDecodeError::InvalidPath(format!(
                "empty media path: {}",
                path.display()
            )));
        }
        Ok(url)
    }

    /// Open `url` as an MF Source Reader with advanced video processing enabled.
    ///
    /// `MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING` lets the reader insert
    /// the Video Processor MFT so it can satisfy an `RGB32` output request from a
    /// decoder that only natively produces `NV12` (which is what the Microsoft
    /// H.264 decoder behind our captured `.mp4`s emits). Without it,
    /// `SetCurrentMediaType(RGB32)` fails with `MF_E_INVALIDMEDIATYPE`
    /// (`0xC00D36B4`) on every real H.264 segment, since the reader will only
    /// accept the decoder's native (non-RGB) output subtypes.
    unsafe fn create_video_source_reader(url: &[u16]) -> Result<IMFSourceReader> {
        let mut attributes = None;
        MFCreateAttributes(&mut attributes, 1)
            .map_err(|e| win_error("MFCreateAttributes (source reader) failed", &e))?;
        let attributes =
            attributes.ok_or_else(|| MediaDecodeError::Decode(
                "MFCreateAttributes returned a null attribute store".to_string(),
            ))?;
        attributes
            .SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)
            .map_err(|e| win_error("enable advanced video processing failed", &e))?;
        MFCreateSourceReaderFromURL(PCWSTR(url.as_ptr()), &attributes)
            .map_err(|e| win_error("MFCreateSourceReaderFromURL failed", &e))
    }

    /// Configure the first video stream of `reader` to decode to uncompressed
    /// `RGB32` and return the negotiated `(width, height)`.
    unsafe fn configure_rgb32_output(
        reader: &IMFSourceReader,
        path: &Path,
    ) -> Result<(u32, u32)> {
        let stream_index = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

        let output_type = windows::Win32::Media::MediaFoundation::MFCreateMediaType()
            .map_err(|e| win_error("MFCreateMediaType (RGB32 output) failed", &e))?;
        output_type
            .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
            .map_err(|e| win_error("set output major type (video) failed", &e))?;
        output_type
            .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)
            .map_err(|e| win_error("set output subtype (RGB32) failed", &e))?;
        reader
            .SetCurrentMediaType(stream_index, None, &output_type)
            .map_err(|e| win_error("SetCurrentMediaType (RGB32) failed", &e))?;

        let actual_type = reader
            .GetCurrentMediaType(stream_index)
            .map_err(|e| win_error("GetCurrentMediaType (video) failed", &e))?;
        let frame_size = actual_type
            .GetUINT64(&MF_MT_FRAME_SIZE)
            .map_err(|e| win_error("GetUINT64(frame size) failed", &e))?;
        // MF_MT_FRAME_SIZE packs width in the high 32 bits, height in the low.
        let width = (frame_size >> 32) as u32;
        let height = (frame_size & 0xFFFF_FFFF) as u32;
        if width == 0 || height == 0 {
            return Err(MediaDecodeError::Decode(format!(
                "Media Foundation reported a zero video frame size for {}",
                path.display()
            )));
        }
        Ok((width, height))
    }

    unsafe fn extract_with_reader(
        url: &[u16],
        path: &Path,
        target_offset_ms: u64,
    ) -> Result<VideoFrame> {
        let reader = create_video_source_reader(url)?;
        let stream_index = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        let (width, height) = configure_rgb32_output(&reader, path)?;

        let target_ticks = (target_offset_ms as i64).saturating_mul(HUNDRED_NS_PER_MS);

        // MF seeks land on keyframes; seek to (or before) the target, then decode
        // forward. A seek to the keyframe at/<= target keeps the forward walk
        // short. Seeking can fail for non-seekable sources; fall back to decoding
        // from the current position.
        if target_ticks > 0 {
            let position = i8_propvariant(target_ticks);
            let time_format = GUID::zeroed();
            let _ = reader.SetCurrentPosition(&time_format, &position);
        }

        // Decode forward, keeping the latest frame whose presentation time is
        // <= target, and stop once we pass the target. If no frame is at/<=
        // target (e.g. the seek landed after it, or target precedes the first
        // sample), keep the first frame we see so the caller always gets a frame.
        let mut kept: Option<(i64, Vec<u8>)> = None;
        loop {
            let mut stream_flags: u32 = 0;
            let mut timestamp: i64 = 0;
            let mut sample = None;
            reader
                .ReadSample(
                    stream_index,
                    0,
                    None,
                    Some(&mut stream_flags),
                    Some(&mut timestamp),
                    Some(&mut sample),
                )
                .map_err(|e| win_error("ReadSample (video) failed", &e))?;

            if (stream_flags & MF_SOURCE_READER_FLAG_ENDOFSTREAM) != 0 {
                break;
            }
            let Some(sample) = sample else {
                // Flag-only callback (e.g. a stream tick); keep reading.
                continue;
            };

            let rgba = sample_to_rgba(&sample, width, height)?;

            let past_target = timestamp > target_ticks;
            if kept.is_none() || !past_target {
                kept = Some((timestamp, rgba));
            }
            if past_target {
                // We already hold the best <= target frame (or the first frame);
                // no need to decode further.
                break;
            }
        }

        let (timestamp, pixels) = kept.ok_or_else(|| {
            MediaDecodeError::Decode(format!(
                "Media Foundation returned no decodable video frame for {}",
                path.display()
            ))
        })?;

        let presented_offset_ms = (timestamp.max(0) / HUNDRED_NS_PER_MS) as u64;
        Ok(VideoFrame {
            pixels,
            width,
            height,
            presented_offset_ms,
        })
    }

    unsafe fn inspect_with_reader(url: &[u16], path: &Path) -> Result<VideoInfo> {
        let reader = create_video_source_reader(url)?;
        let (width, height) = configure_rgb32_output(&reader, path)?;

        let duration = reader
            .GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)
            .map_err(|e| win_error("GetPresentationAttribute(duration) failed", &e))?;
        // MF_PD_DURATION is VT_UI8 (100ns ticks).
        let duration_100ns = duration.Anonymous.Anonymous.Anonymous.uhVal;
        if duration_100ns == 0 {
            return Err(MediaDecodeError::Decode(format!(
                "Media Foundation reported zero video duration for {}",
                path.display()
            )));
        }
        let duration_ms = duration_100ns / HUNDRED_NS_PER_MS as u64;

        Ok(VideoInfo {
            duration_ms,
            width,
            height,
        })
    }

    /// Build a `VT_I8` PROPVARIANT carrying a 100ns tick count for
    /// `SetCurrentPosition` (`GUID_NULL` time format => 100ns units).
    unsafe fn i8_propvariant(ticks_100ns: i64) -> PROPVARIANT {
        let mut variant = PROPVARIANT::default();
        let inner = &mut variant.Anonymous.Anonymous;
        inner.vt = VT_I8;
        inner.Anonymous.hVal = ticks_100ns;
        variant
    }

    /// Copy one decoded `RGB32` video sample into a tightly packed top-down RGBA
    /// buffer. MF `RGB32` is 32-bit `BGRX` per pixel in memory; we swap to RGBA
    /// and force opaque alpha. A negative `MF_MT_DEFAULT_STRIDE` means the source
    /// is bottom-up, so we flip rows to deliver a top-down buffer.
    unsafe fn sample_to_rgba(
        sample: &windows::Win32::Media::MediaFoundation::IMFSample,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>> {
        let buffer = sample
            .ConvertToContiguousBuffer()
            .map_err(|e| win_error("ConvertToContiguousBuffer (video) failed", &e))?;

        let mut data_ptr: *mut u8 = std::ptr::null_mut();
        let mut current_len: u32 = 0;
        buffer
            .Lock(&mut data_ptr, None, Some(&mut current_len))
            .map_err(|e| win_error("IMFMediaBuffer.Lock (video) failed", &e))?;

        let result = copy_rgb32_to_rgba(data_ptr, current_len as usize, width, height);

        let unlock = buffer.Unlock();
        unlock.map_err(|e| win_error("IMFMediaBuffer.Unlock (video) failed", &e))?;
        result
    }

    /// Convert a locked contiguous `RGB32`/`BGRX` region to a tightly packed
    /// top-down RGBA `Vec<u8>`. Assumes the contiguous buffer is top-down with a
    /// `width * 4` stride (the default the source reader produces for `RGB32`).
    unsafe fn copy_rgb32_to_rgba(
        data_ptr: *const u8,
        data_len: usize,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>> {
        let width = width as usize;
        let height = height as usize;
        let row_bytes = width * 4;
        let expected = row_bytes * height;
        if data_ptr.is_null() || data_len < expected {
            return Err(MediaDecodeError::Decode(format!(
                "video frame buffer too small: have {data_len} bytes, need {expected} for {width}x{height} RGB32"
            )));
        }
        let src = std::slice::from_raw_parts(data_ptr, expected);
        let mut rgba = vec![0_u8; expected];
        for (dst_px, src_px) in rgba.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
            // BGRX -> RGBA, opaque alpha.
            dst_px[0] = src_px[2];
            dst_px[1] = src_px[1];
            dst_px[2] = src_px[0];
            dst_px[3] = 0xFF;
        }
        Ok(rgba)
    }

    // Touch `MF_MT_DEFAULT_STRIDE` import so it is available for future stride
    // handling; the default `RGB32` source-reader output is top-down packed, so
    // the current path does not need a per-row stride walk.
    #[allow(dead_code)]
    const _DEFAULT_STRIDE_GUID: GUID = MF_MT_DEFAULT_STRIDE;

    fn win_error(context: &str, error: &windows::core::Error) -> MediaDecodeError {
        MediaDecodeError::Decode(format!("{context}: {error}"))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn copy_rgb32_swaps_bgrx_to_opaque_rgba() {
            // One pixel, BGRX in memory = (B=10, G=20, R=30, X=255).
            let bgrx: [u8; 4] = [10, 20, 30, 255];
            let rgba = unsafe { copy_rgb32_to_rgba(bgrx.as_ptr(), bgrx.len(), 1, 1) }
                .expect("1x1 BGRX converts to RGBA");
            assert_eq!(rgba, vec![30, 20, 10, 0xFF]);
        }

        #[test]
        fn copy_rgb32_rejects_short_buffer() {
            let bytes: [u8; 4] = [0, 0, 0, 0];
            // Claim a 2x2 frame (needs 16 bytes) but only 4 are available.
            let error =
                unsafe { copy_rgb32_to_rgba(bytes.as_ptr(), bytes.len(), 2, 2) }.unwrap_err();
            assert!(matches!(error, MediaDecodeError::Decode(_)));
        }
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

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn video_extraction_and_inspection_are_unsupported_off_windows() {
        let extract = extract_video_frame_rgba("/tmp/whatever.mp4", 0)
            .expect_err("non-Windows frame extraction must report unsupported");
        assert!(matches!(extract, MediaDecodeError::UnsupportedPlatform(_)));
        let inspect = inspect_video("/tmp/whatever.mp4")
            .expect_err("non-Windows inspection must report unsupported");
        assert!(matches!(inspect, MediaDecodeError::UnsupportedPlatform(_)));
    }

    // VideoFrame::encode_jpeg is platform-neutral (pure `image`-crate work over a
    // caller-supplied pixel buffer), so it is exercised on every target. It
    // proves the RGBA -> JPEG boundary the exact-preview fallback and scrub
    // preview (#83) both rely on.
    #[test]
    fn video_frame_encodes_to_jpeg_bytes() {
        // 2x2 solid red RGBA.
        let pixels = vec![255, 0, 0, 255].repeat(4);
        let frame = VideoFrame {
            pixels,
            width: 2,
            height: 2,
            presented_offset_ms: 17,
        };
        let jpeg = frame.encode_jpeg(80).expect("encode 2x2 RGBA to JPEG");
        // JPEG SOI marker.
        assert!(jpeg.len() > 2, "jpeg should be non-trivial");
        assert_eq!(&jpeg[0..2], &[0xFF, 0xD8], "expected JPEG SOI marker");
        // Round-trips back to the same dimensions through the `image` decoder.
        let decoded = image::load_from_memory(&jpeg).expect("decode produced JPEG");
        assert_eq!(decoded.width(), 2);
        assert_eq!(decoded.height(), 2);
    }

    #[test]
    fn video_frame_encode_rejects_mismatched_buffer() {
        let frame = VideoFrame {
            pixels: vec![0; 3], // far too small for 2x2 RGBA (needs 16 bytes)
            width: 2,
            height: 2,
            presented_offset_ms: 0,
        };
        let error = frame
            .encode_jpeg(80)
            .expect_err("undersized buffer must fail to encode");
        assert!(matches!(error, MediaDecodeError::Decode(_)));
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
