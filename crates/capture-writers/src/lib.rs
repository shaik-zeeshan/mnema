use capture_types::CaptureErrorResponse;
#[cfg(target_os = "macos")]
use std::path::Path;

/// 100ns ticks in one second — the Media Foundation time unit used by the
/// Windows AAC sink writer. Shared so the Windows tail hold-back can convert a
/// chunk's MF sample time/duration back into segment-relative seconds for the
/// platform-neutral [`AudioTailSampleBuffer`].
#[cfg(target_os = "windows")]
const WINDOWS_MF_TICKS_PER_SECOND: i64 = 10_000_000;

#[cfg(target_os = "macos")]
#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    static AVVideoAverageBitRateKey: &'static cidre::ns::String;
}

#[cfg(target_os = "macos")]
use cidre::objc::autorelease_pool::AutoreleasePoolPage;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::collections::VecDeque;
#[cfg(target_os = "macos")]
use std::process::Command;
#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};

/// Trim `input` to the `[start_secs, end_secs]` range and re-encode the result
/// as AAC/m4a at `output`.
///
/// This runs entirely on AVFoundation (`AVAudioFile`) so it works inside the
/// sandboxed/packaged desktop app, which does not inherit a shell `PATH` and so
/// cannot rely on an external `ffmpeg` binary being discoverable.
#[cfg(target_os = "macos")]
pub fn trim_audio_file_to_m4a(
    input: &str,
    output: &str,
    start_secs: f64,
    end_secs: f64,
) -> Result<(), CaptureErrorResponse> {
    use cidre::{av, cat, ns};

    if !start_secs.is_finite() || !end_secs.is_finite() || end_secs < start_secs {
        return Err(CaptureErrorResponse {
            code: "invalid_audio_trim_range".to_string(),
            message: "Invalid audio trim range".to_string(),
        });
    }

    let _autorelease_pool = AutoreleasePoolPage::push();

    let input_url = ns::Url::with_fs_path_str(input, false);
    let mut reader =
        av::AudioFile::open_read_common_format(&input_url, av::AudioCommonFormat::PcmF32, false)
            .map_err(|error| CaptureErrorResponse {
                code: "audio_trim_failed".to_string(),
                message: format!("Failed to open audio for trim {input}: {error}"),
            })?;

    let processing_format = reader.processing_format();
    let sample_rate = processing_format.absd().sample_rate;
    if !sample_rate.is_finite() || sample_rate <= 0.0 {
        return Err(CaptureErrorResponse {
            code: "audio_trim_failed".to_string(),
            message: format!("Audio file {input} reported invalid sample rate for trim"),
        });
    }
    let channel_count = processing_format.channel_count();
    if channel_count == 0 {
        return Err(CaptureErrorResponse {
            code: "audio_trim_failed".to_string(),
            message: format!("Audio file {input} reported zero channels for trim"),
        });
    }

    let total_frames = reader.len().max(0);
    let frame_for_secs = |secs: f64| -> i64 {
        let frame = (secs * sample_rate).round();
        if !frame.is_finite() || frame <= 0.0 {
            0
        } else if frame >= total_frames as f64 {
            total_frames
        } else {
            frame as i64
        }
    };
    let start_frame = frame_for_secs(start_secs);
    let end_frame = frame_for_secs(end_secs).max(start_frame);
    let frames_to_copy = (end_frame - start_frame).max(0) as u64;

    let _ = std::fs::remove_file(output);
    let output_url = ns::Url::with_fs_path_str(output, false);
    let format_id = ns::Number::with_u32(cat::audio::Format::MPEG4_AAC.0);
    let sample_rate_value = ns::Number::with_f64(sample_rate);
    let channel_count_value = ns::Number::with_i64(channel_count as i64);
    let output_settings: cidre::arc::R<ns::Dictionary<ns::String, ns::Id>> =
        ns::Dictionary::with_keys_values(
            &[
                av::audio::all_formats_keys::id(),
                av::audio::all_formats_keys::sample_rate(),
                av::audio::all_formats_keys::number_of_channels(),
            ],
            &[
                format_id.as_id_ref(),
                sample_rate_value.as_id_ref(),
                channel_count_value.as_id_ref(),
            ],
        );
    let mut writer = av::AudioFile::open_write_common_format(
        &output_url,
        &output_settings,
        av::AudioCommonFormat::PcmF32,
        false,
    )
    .map_err(|error| CaptureErrorResponse {
        code: "audio_trim_failed".to_string(),
        message: format!("Failed to open trimmed audio output {output}: {error}"),
    })?;

    if frames_to_copy > 0 {
        reader.set_frame_pos(start_frame);
        let mut remaining = frames_to_copy;
        while remaining > 0 {
            let want = remaining.min(16_384) as u32;
            let mut buffer =
                av::AudioPcmBuf::with_format(&processing_format, want).ok_or_else(|| {
                    CaptureErrorResponse {
                        code: "audio_trim_failed".to_string(),
                        message: "Failed to allocate audio trim buffer".to_string(),
                    }
                })?;
            reader
                .read_n(&mut buffer, want)
                .map_err(|error| CaptureErrorResponse {
                    code: "audio_trim_failed".to_string(),
                    message: format!("Failed reading audio for trim {input}: {error}"),
                })?;
            let read_frames = buffer.frame_len();
            if read_frames == 0 {
                break;
            }
            writer
                .write(&buffer)
                .map_err(|error| CaptureErrorResponse {
                    code: "audio_trim_failed".to_string(),
                    message: format!("Failed writing trimmed audio {output}: {error}"),
                })?;
            remaining = remaining.saturating_sub(read_frames as u64);
            if (read_frames as u64) < (want as u64) {
                break;
            }
        }
    }

    writer.close();

    match validate_trimmed_audio_output_file(output) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = std::fs::remove_file(output);
            Err(error)
        }
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone)]
pub struct DecodedMonoPcm {
    pub samples: Vec<f32>,
    pub sample_rate_hz: u32,
}

#[cfg(target_os = "macos")]
pub fn decode_audio_file_to_mono_pcm(path: &Path) -> Result<DecodedMonoPcm, CaptureErrorResponse> {
    let path_str = path.to_str().ok_or_else(|| CaptureErrorResponse {
        code: "audio_decode_invalid_path".to_string(),
        message: format!("Audio path is not valid UTF-8: {}", path.display()),
    })?;
    let _autorelease_pool = AutoreleasePoolPage::push();
    let url = cidre::ns::Url::with_fs_path_str(path_str, false);
    let mut file = cidre::av::AudioFile::open_read_common_format(
        &url,
        cidre::av::AudioCommonFormat::PcmF32,
        false,
    )
    .map_err(|error| CaptureErrorResponse {
        code: "audio_decode_failed".to_string(),
        message: format!("Failed to open audio file {}: {error}", path.display()),
    })?;
    let format = file.processing_format();
    let sample_rate = format.absd().sample_rate;
    if !sample_rate.is_finite() || sample_rate <= 0.0 {
        return Err(CaptureErrorResponse {
            code: "audio_decode_failed".to_string(),
            message: format!("Audio file {} reported invalid sample rate", path.display()),
        });
    }
    let channels = usize::try_from(format.channel_count()).unwrap_or(0);
    if channels == 0 {
        return Err(CaptureErrorResponse {
            code: "audio_decode_failed".to_string(),
            message: format!("Audio file {} reported zero channels", path.display()),
        });
    }

    let total_frames = file.len().max(0) as u64;
    if total_frames == 0 {
        return Ok(DecodedMonoPcm {
            samples: Vec::new(),
            sample_rate_hz: sample_rate.round().clamp(1.0, u32::MAX as f64) as u32,
        });
    }

    let mut out = Vec::new();
    let mut remaining = total_frames;
    while remaining > 0 {
        let frames = remaining.min(16_384) as u32;
        let mut buffer = cidre::av::AudioPcmBuf::with_format(&format, frames).ok_or_else(|| {
            CaptureErrorResponse {
                code: "audio_decode_failed".to_string(),
                message: "Failed to allocate audio decode buffer".to_string(),
            }
        })?;
        file.read_n(&mut buffer, frames)
            .map_err(|error| CaptureErrorResponse {
                code: "audio_decode_failed".to_string(),
                message: format!("Failed reading audio file {}: {error}", path.display()),
            })?;
        let frame_len = buffer.frame_len() as usize;
        if frame_len == 0 {
            break;
        }
        append_downmixed_f32(&mut out, &buffer, channels, frame_len)?;
        remaining = remaining.saturating_sub(frame_len as u64);
    }

    Ok(DecodedMonoPcm {
        samples: out,
        sample_rate_hz: sample_rate.round().clamp(1.0, u32::MAX as f64) as u32,
    })
}

#[cfg(target_os = "macos")]
fn append_downmixed_f32(
    out: &mut Vec<f32>,
    buffer: &cidre::av::AudioPcmBuf,
    channels: usize,
    frame_len: usize,
) -> Result<(), CaptureErrorResponse> {
    if buffer.stride() > 1 {
        let data = buffer.data_f32_at(0).ok_or_else(|| CaptureErrorResponse {
            code: "audio_decode_failed".to_string(),
            message: "Decoded audio buffer did not expose float channel data".to_string(),
        })?;
        for frame in data.chunks(buffer.stride()).take(frame_len) {
            let sum: f32 = frame.iter().take(channels).copied().sum();
            out.push(sum / channels as f32);
        }
        return Ok(());
    }

    let first = buffer.data_f32_at(0).ok_or_else(|| CaptureErrorResponse {
        code: "audio_decode_failed".to_string(),
        message: "Decoded audio buffer did not expose float channel data".to_string(),
    })?;
    for frame_idx in 0..frame_len {
        let mut sum = first.get(frame_idx).copied().unwrap_or_default();
        for channel in 1..channels {
            if let Some(samples) = buffer.data_f32_at(channel) {
                sum += samples.get(frame_idx).copied().unwrap_or_default();
            }
        }
        out.push(sum / channels as f32);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn validate_trimmed_audio_output_file(output: &str) -> Result<(), CaptureErrorResponse> {
    let _autorelease_pool = AutoreleasePoolPage::push();
    let url = cidre::ns::Url::with_fs_path_str(output, false);
    let file = cidre::av::AudioFile::open_read(&url).map_err(|error| CaptureErrorResponse {
        code: "audio_trim_failed".to_string(),
        message: format!("Trimmed audio output is not readable: {error}"),
    })?;

    if file.len() <= 0 {
        return Err(CaptureErrorResponse {
            code: "audio_trim_failed".to_string(),
            message: "Trimmed audio output contains no audio frames".to_string(),
        });
    }

    Ok(())
}

#[cfg(target_os = "macos")]
trait AvailabilityValue<T> {
    fn into_option(self) -> Option<&'static T>;
}

#[cfg(target_os = "macos")]
impl<T: 'static> AvailabilityValue<T> for &'static T {
    fn into_option(self) -> Option<&'static T> {
        Some(self)
    }
}

#[cfg(target_os = "macos")]
impl<T: 'static> AvailabilityValue<T> for Option<&'static T> {
    fn into_option(self) -> Option<&'static T> {
        self
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
pub struct AudioAssetWriterState {
    writer: cidre::arc::R<cidre::av::AssetWriter>,
    input: cidre::arc::R<cidre::av::AssetWriterInput>,
    started: bool,
    appended_samples: u64,
    expected_sample_format: Option<AudioSampleFormat>,
    tail_trim_seconds: u64,
    activity_threshold: f32,
    tail_buffer: AudioTailSampleBuffer<cidre::arc::R<cidre::cm::SampleBuf>>,
    label: &'static str,
}

/// Platform-neutral rolling tail buffer used by both the macOS AVFoundation
/// asset writer and the Windows WASAPI/Media-Foundation AAC sink writer to hold
/// back the last N seconds of audio *before* it reaches the encoder, so a
/// committed Audio Segment never carries the inactivity idle tail.
///
/// The buffer is generic over the per-platform sample payload `T` (a retained
/// `cm::SampleBuf` on macOS; a PCM byte chunk + Media Foundation timing on
/// Windows). `end_secs` is the segment-relative end time of each buffered
/// sample, and `active` records whether that sample crossed the activity
/// boundary (peak level OR VAD speech). [`pop_sample_before_tail`] releases
/// everything older than `retain_seconds` once activity is observed; an
/// inactivity stop calls [`discard_tail`] while a normal stop / resume drains
/// the buffer in order. This is the same trim semantics on both platforms.
#[cfg(any(target_os = "macos", target_os = "windows"))]
#[derive(Debug)]
struct AudioTailSampleBuffer<T> {
    samples: VecDeque<TimedAudioTailSample<T>>,
    latest_sample_end_secs: Option<f64>,
    observed_active_sample: bool,
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
impl<T> Default for AudioTailSampleBuffer<T> {
    fn default() -> Self {
        Self {
            samples: VecDeque::new(),
            latest_sample_end_secs: None,
            observed_active_sample: false,
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[derive(Debug)]
struct TimedAudioTailSample<T> {
    sample: T,
    end_secs: f64,
    active: bool,
}

#[cfg(target_os = "macos")]
pub fn set_audio_writer_inactivity_tail_trim_seconds(
    writer_state: &mut AudioAssetWriterState,
    trim_seconds: u64,
) {
    writer_state.tail_trim_seconds = trim_seconds;
}

#[cfg(target_os = "macos")]
pub fn set_audio_writer_activity_threshold(
    writer_state: &mut AudioAssetWriterState,
    threshold: f32,
) {
    writer_state.activity_threshold = if threshold.is_finite() {
        threshold.clamp(0.0, 1.0)
    } else {
        0.0
    };
}

#[cfg(target_os = "macos")]
fn flush_audio_tail_buffer(
    writer_state: &mut AudioAssetWriterState,
) -> Result<(), CaptureErrorResponse> {
    while let Some(sample) = writer_state.tail_buffer.samples.pop_front() {
        append_audio_sample_to_writer_untrimmed(writer_state, sample.sample.as_ref())?;
    }
    writer_state.tail_buffer.latest_sample_end_secs = None;
    writer_state.tail_buffer.observed_active_sample = false;
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn record_audio_writer_tail_activity(
    writer_state: &mut AudioAssetWriterState,
) -> Result<bool, CaptureErrorResponse> {
    if !writer_state.tail_buffer.mark_latest_sample_active() {
        return Ok(false);
    }

    while let Some(sample) = writer_state
        .tail_buffer
        .pop_sample_before_tail(writer_state.tail_trim_seconds)
    {
        append_audio_sample_to_writer_untrimmed(writer_state, sample.as_ref())?;
    }

    Ok(true)
}

#[cfg(target_os = "macos")]
fn sample_buf_start_secs(sample_buf: &cidre::cm::SampleBuf) -> Option<f64> {
    let pts = sample_buf.pts();
    pts.is_numeric()
        .then(|| pts.as_secs())
        .filter(|v| v.is_finite())
}

#[cfg(target_os = "macos")]
fn sample_buf_end_secs(sample_buf: &cidre::cm::SampleBuf) -> Option<f64> {
    let start = sample_buf_start_secs(sample_buf)?;
    let duration = sample_buf.duration();
    let duration_secs = if duration.is_numeric() {
        duration.as_secs().max(0.0)
    } else {
        0.0
    };
    Some(start + duration_secs)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
impl<T> AudioTailSampleBuffer<T> {
    fn append_timed(
        &mut self,
        sample: T,
        sample_end_secs: Option<f64>,
        _retain_seconds: u64,
        active: bool,
    ) -> bool {
        let Some(sample_end_secs) = sample_end_secs.filter(|value| value.is_finite()) else {
            if active {
                self.observed_active_sample = true;
                self.latest_sample_end_secs = Some(
                    self.latest_sample_end_secs
                        .map(|latest| latest.max(f64::NEG_INFINITY))
                        .unwrap_or(f64::NEG_INFINITY),
                );
                self.samples.push_back(TimedAudioTailSample {
                    sample,
                    end_secs: f64::NEG_INFINITY,
                    active,
                });
                return true;
            }

            return false;
        };

        self.latest_sample_end_secs = Some(
            self.latest_sample_end_secs
                .map(|latest| latest.max(sample_end_secs))
                .unwrap_or(sample_end_secs),
        );
        self.observed_active_sample |= active;
        self.samples.push_back(TimedAudioTailSample {
            sample,
            end_secs: sample_end_secs,
            active,
        });

        true
    }

    fn pop_sample_before_tail(&mut self, retain_seconds: u64) -> Option<T> {
        let latest_end_secs = self.latest_sample_end_secs?;
        let cutoff_secs = latest_end_secs - retain_seconds as f64;
        let buffered_active_sample = self.samples.iter().any(|sample| sample.active);
        if self.samples.front().is_some_and(|sample| {
            buffered_active_sample
                || (self.observed_active_sample && sample.end_secs <= cutoff_secs)
        }) {
            self.samples.pop_front().map(|sample| sample.sample)
        } else {
            None
        }
    }

    fn mark_latest_sample_active(&mut self) -> bool {
        let Some(sample) = self.samples.back_mut() else {
            return false;
        };

        sample.active = true;
        self.observed_active_sample = true;
        true
    }

    fn discard_tail(&mut self) {
        self.samples.clear();
        self.latest_sample_end_secs = None;
        self.observed_active_sample = false;
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn audio_activity_level_is_meaningful(level: Option<f32>, threshold: f32) -> bool {
    level
        .map(|level| {
            let threshold = if threshold.is_finite() {
                threshold.clamp(0.0, 1.0)
            } else {
                0.0
            };
            if threshold > 0.0 {
                level >= threshold
            } else {
                level > 0.0
            }
        })
        .unwrap_or(true)
}

#[cfg(target_os = "macos")]
fn sample_buf_has_audio_activity(sample_buf: &cidre::cm::SampleBuf, threshold: f32) -> bool {
    audio_activity_level_is_meaningful(
        derive_audio_activity_level_from_sample_buf(sample_buf),
        threshold,
    )
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioSampleAppendDisposition {
    Appended,
    Deferred,
    Dropped,
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
pub struct VideoAssetWriterState {
    writer: cidre::arc::R<cidre::av::AssetWriter>,
    input: cidre::arc::R<cidre::av::AssetWriterInput>,
    started: bool,
    appended_samples: u64,
    label: &'static str,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioWriterFormatSpec {
    sample_rate_hz: f64,
    channel_count: u32,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioSampleFormat {
    pub sample_rate_hz: f64,
    pub format_id: u32,
    pub format_flags: u32,
    pub bytes_per_packet: u32,
    pub frames_per_packet: u32,
    pub bytes_per_frame: u32,
    pub channels_per_frame: u32,
    pub bits_per_channel: u32,
}

#[cfg(target_os = "macos")]
impl AudioSampleFormat {
    pub const fn to_writer_format(self) -> AudioWriterFormatSpec {
        AudioWriterFormatSpec::new(self.sample_rate_hz, self.channels_per_frame)
    }
}

#[cfg(target_os = "macos")]
pub fn audio_sample_format_is_compatible_with_writer_format(
    expected: AudioSampleFormat,
    actual: AudioSampleFormat,
) -> bool {
    expected.format_id == actual.format_id
        && expected.to_writer_format() == actual.to_writer_format()
}

#[cfg(target_os = "macos")]
impl AudioWriterFormatSpec {
    pub const fn new(sample_rate_hz: f64, channel_count: u32) -> Self {
        Self {
            sample_rate_hz,
            channel_count,
        }
    }
}

#[cfg(target_os = "macos")]
const DEFAULT_AUDIO_WRITER_FORMAT: AudioWriterFormatSpec = AudioWriterFormatSpec::new(48_000.0, 2);

#[cfg(target_os = "macos")]
pub fn derive_audio_writer_format_from_sample_buf(
    sample_buf: &cidre::cm::SampleBuf,
) -> Option<AudioWriterFormatSpec> {
    derive_audio_sample_format_from_sample_buf(sample_buf).map(AudioSampleFormat::to_writer_format)
}

#[cfg(target_os = "macos")]
pub fn derive_audio_sample_format_from_sample_buf(
    sample_buf: &cidre::cm::SampleBuf,
) -> Option<AudioSampleFormat> {
    let format_desc = sample_buf.format_desc()?;
    if format_desc.media_type() != cidre::cm::MediaType::AUDIO {
        return None;
    }

    let audio_format_desc: &cidre::cm::AudioFormatDesc =
        unsafe { &*(format_desc as *const _ as *const cidre::cm::AudioFormatDesc) };

    let stream_basic_desc = audio_format_desc.stream_basic_desc()?;
    if stream_basic_desc.sample_rate <= 0.0 || stream_basic_desc.channels_per_frame == 0 {
        return None;
    }

    Some(AudioSampleFormat {
        sample_rate_hz: stream_basic_desc.sample_rate,
        format_id: stream_basic_desc.format.0,
        format_flags: stream_basic_desc.format_flags.0,
        bytes_per_packet: stream_basic_desc.bytes_per_packet,
        frames_per_packet: stream_basic_desc.frames_per_packet,
        bytes_per_frame: stream_basic_desc.bytes_per_frame,
        channels_per_frame: stream_basic_desc.channels_per_frame,
        bits_per_channel: stream_basic_desc.bits_per_channel,
    })
}

#[cfg(target_os = "macos")]
const AUDIO_ACTIVITY_MAX_PROBES_PER_BUFFER: usize = 256;

#[cfg(target_os = "macos")]
pub fn derive_audio_activity_level_from_sample_buf(
    sample_buf: &cidre::cm::SampleBuf,
) -> Option<f32> {
    if !sample_buf.data_is_ready() {
        return None;
    }

    let sample_format = derive_audio_sample_format_from_sample_buf(sample_buf)?;
    let mut audio_buf_list = cidre::cat::AudioBufListN::default();
    let audio_buf_list = sample_buf.audio_buf_list_n(&mut audio_buf_list).ok()?;

    peak_audio_activity_level_from_audio_buffers(audio_buf_list.list.buffers(), sample_format)
}

#[cfg(target_os = "macos")]
fn peak_audio_activity_level_from_audio_buffers(
    buffers: &[cidre::cat::AudioBuf],
    sample_format: AudioSampleFormat,
) -> Option<f32> {
    let mut peak = 0.0_f32;
    let mut sampled_any = false;

    for buffer in buffers {
        let byte_len = buffer.data_bytes_size as usize;
        if buffer.data.is_null() || byte_len == 0 {
            continue;
        }

        let bytes = unsafe { std::slice::from_raw_parts(buffer.data as *const u8, byte_len) };
        let Some(buffer_peak) = peak_audio_activity_level_from_pcm_bytes(
            bytes,
            sample_format,
            AUDIO_ACTIVITY_MAX_PROBES_PER_BUFFER,
        ) else {
            continue;
        };

        sampled_any = true;
        peak = peak.max(buffer_peak);

        if peak >= 1.0 {
            return Some(1.0);
        }
    }

    sampled_any.then_some(peak)
}

#[cfg(target_os = "macos")]
fn peak_audio_activity_level_from_pcm_bytes(
    bytes: &[u8],
    sample_format: AudioSampleFormat,
    max_probes: usize,
) -> Option<f32> {
    let format_id = cidre::cat::AudioFormat(sample_format.format_id);
    if format_id != cidre::cat::AudioFormat::LINEAR_PCM {
        return None;
    }

    let format_flags = cidre::cat::AudioFormatFlags(sample_format.format_flags);
    let is_float = format_flags.contains(cidre::cat::AudioFormatFlags::IS_FLOAT);
    let is_signed_integer = format_flags.contains(cidre::cat::AudioFormatFlags::IS_SIGNED_INTEGER);
    let is_packed = format_flags.contains(cidre::cat::AudioFormatFlags::IS_PACKED);
    let is_native_endian = format_flags.0 & cidre::cat::AudioFormatFlags::IS_BIG_ENDIAN.0
        == cidre::cat::AudioFormatFlags::NATIVE_ENDIAN.0;
    let bytes_per_sample = sample_format.bits_per_channel.saturating_add(7) / 8;
    let bytes_per_sample = bytes_per_sample as usize;

    if !is_native_endian || !is_packed || bytes_per_sample == 0 || bytes.len() < bytes_per_sample {
        return None;
    }

    let sample_count = bytes.len() / bytes_per_sample;
    if sample_count == 0 {
        return None;
    }

    let probe_count = max_probes.max(1).min(sample_count);
    let step = sample_count.div_ceil(probe_count);
    let mut peak = 0.0_f32;

    for sample_index in (0..sample_count).step_by(step) {
        let offset = sample_index * bytes_per_sample;
        let sample = &bytes[offset..offset + bytes_per_sample];
        let sample_peak = if is_float {
            normalized_float_pcm_sample(sample, sample_format.bits_per_channel)
        } else if is_signed_integer {
            normalized_signed_pcm_sample(sample, sample_format.bits_per_channel)
        } else {
            None
        }?;

        peak = peak.max(sample_peak);
        if peak >= 1.0 {
            return Some(1.0);
        }
    }

    Some(peak)
}

#[cfg(target_os = "macos")]
fn normalized_float_pcm_sample(sample: &[u8], bits_per_channel: u32) -> Option<f32> {
    let value = match bits_per_channel {
        32 if sample.len() >= std::mem::size_of::<f32>() => {
            f32::from_ne_bytes(sample[..4].try_into().ok()?).abs()
        }
        64 if sample.len() >= std::mem::size_of::<f64>() => {
            f64::from_ne_bytes(sample[..8].try_into().ok()?).abs() as f32
        }
        _ => return None,
    };

    value.is_finite().then_some(value.clamp(0.0, 1.0))
}

#[cfg(target_os = "macos")]
fn normalized_signed_pcm_sample(sample: &[u8], bits_per_channel: u32) -> Option<f32> {
    let magnitude = match bits_per_channel {
        8 if !sample.is_empty() => (sample[0] as i8).unsigned_abs() as f32 / i8::MAX as f32,
        16 if sample.len() >= 2 => {
            i16::from_ne_bytes(sample[..2].try_into().ok()?).unsigned_abs() as f32 / i16::MAX as f32
        }
        24 if sample.len() >= 3 => {
            let value = if cfg!(target_endian = "little") {
                i32::from_le_bytes([
                    sample[0],
                    sample[1],
                    sample[2],
                    if sample[2] & 0x80 != 0 { 0xFF } else { 0x00 },
                ])
            } else {
                i32::from_be_bytes([
                    if sample[0] & 0x80 != 0 { 0xFF } else { 0x00 },
                    sample[0],
                    sample[1],
                    sample[2],
                ])
            };

            value.unsigned_abs() as f32 / 8_388_607.0
        }
        32 if sample.len() >= 4 => {
            i32::from_ne_bytes(sample[..4].try_into().ok()?).unsigned_abs() as f32 / i32::MAX as f32
        }
        _ => return None,
    };

    Some(magnitude.clamp(0.0, 1.0))
}

#[cfg(target_os = "macos")]
pub fn create_audio_asset_writer(
    output_url: &cidre::ns::Url,
    label: &'static str,
) -> Result<AudioAssetWriterState, CaptureErrorResponse> {
    create_audio_asset_writer_with_format(output_url, label, DEFAULT_AUDIO_WRITER_FORMAT)
}

#[cfg(target_os = "macos")]
pub fn create_audio_asset_writer_for_sample_buf(
    output_url: &cidre::ns::Url,
    label: &'static str,
    sample_buf: &cidre::cm::SampleBuf,
) -> Result<AudioAssetWriterState, CaptureErrorResponse> {
    let sample_format = derive_audio_sample_format_from_sample_buf(sample_buf);
    let format = sample_format
        .map(AudioSampleFormat::to_writer_format)
        .unwrap_or(DEFAULT_AUDIO_WRITER_FORMAT);
    create_audio_asset_writer_with_format_internal(output_url, label, format, sample_format)
}

#[cfg(target_os = "macos")]
pub fn create_audio_asset_writer_for_sample_format(
    output_url: &cidre::ns::Url,
    label: &'static str,
    sample_format: AudioSampleFormat,
) -> Result<AudioAssetWriterState, CaptureErrorResponse> {
    let mut last_error = None;
    for format in audio_writer_format_candidates_for_sample_format(sample_format) {
        match create_audio_asset_writer_with_format_internal(
            output_url,
            label,
            format,
            Some(sample_format),
        ) {
            Ok(writer_state) => return Ok(writer_state),
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error.unwrap_or_else(|| CaptureErrorResponse {
        code: "capture_output_unavailable".to_string(),
        message: format!("Failed to create {label} asset writer input"),
    }))
}

#[cfg(target_os = "macos")]
pub fn create_audio_asset_writer_with_format(
    output_url: &cidre::ns::Url,
    label: &'static str,
    format: AudioWriterFormatSpec,
) -> Result<AudioAssetWriterState, CaptureErrorResponse> {
    create_audio_asset_writer_with_format_internal(output_url, label, format, None)
}

#[cfg(target_os = "macos")]
fn audio_writer_format_candidates_for_sample_format(
    sample_format: AudioSampleFormat,
) -> Vec<AudioWriterFormatSpec> {
    let mut candidates = Vec::new();
    push_audio_writer_format_candidate(&mut candidates, sample_format.to_writer_format());

    let channel_count = sample_format.channels_per_frame.max(1);
    push_audio_writer_format_candidate(
        &mut candidates,
        AudioWriterFormatSpec::new(48_000.0, channel_count),
    );

    let aac_channel_count = channel_count.min(2);
    push_audio_writer_format_candidate(
        &mut candidates,
        AudioWriterFormatSpec::new(48_000.0, aac_channel_count),
    );

    push_audio_writer_format_candidate(&mut candidates, DEFAULT_AUDIO_WRITER_FORMAT);
    candidates
}

#[cfg(target_os = "macos")]
fn push_audio_writer_format_candidate(
    candidates: &mut Vec<AudioWriterFormatSpec>,
    candidate: AudioWriterFormatSpec,
) {
    if candidate.sample_rate_hz.is_finite()
        && candidate.sample_rate_hz > 0.0
        && candidate.channel_count > 0
        && !candidates.contains(&candidate)
    {
        candidates.push(candidate);
    }
}

#[cfg(target_os = "macos")]
fn create_audio_asset_writer_with_format_internal(
    output_url: &cidre::ns::Url,
    label: &'static str,
    format: AudioWriterFormatSpec,
    expected_sample_format: Option<AudioSampleFormat>,
) -> Result<AudioAssetWriterState, CaptureErrorResponse> {
    use cidre::{av, cat, ns};

    let _autorelease_pool = AutoreleasePoolPage::push();

    let format_id = ns::Number::with_u32(cat::audio::Format::MPEG4_AAC.0);
    let sample_rate = ns::Number::with_f64(format.sample_rate_hz);
    let channel_count = ns::Number::with_i64(format.channel_count as i64);

    let output_settings: cidre::arc::R<ns::Dictionary<ns::String, ns::Id>> =
        ns::Dictionary::with_keys_values(
            &[
                av::audio::all_formats_keys::id(),
                av::audio::all_formats_keys::sample_rate(),
                av::audio::all_formats_keys::number_of_channels(),
            ],
            &[
                format_id.as_id_ref(),
                sample_rate.as_id_ref(),
                channel_count.as_id_ref(),
            ],
        );

    let mut writer = av::AssetWriter::with_url_and_file_type(output_url, av::FileType::m4a())
        .map_err(|error| {
            error_with_ns_error(
                "capture_output_unavailable",
                "Failed to create audio asset writer",
                error,
            )
        })?;

    let mut input = av::AssetWriterInput::with_media_type_and_output_settings(
        av::MediaType::audio(),
        Some(output_settings.as_ref()),
    )
    .map_err(|_| CaptureErrorResponse {
        code: "capture_output_unavailable".to_string(),
        message: format!("Failed to create {label} asset writer input"),
    })?;
    input.set_expects_media_data_in_real_time(true);

    if !writer.can_add_input(&input) {
        return Err(CaptureErrorResponse {
            code: "capture_output_unavailable".to_string(),
            message: format!("Failed to add {label} asset writer input"),
        });
    }

    writer.add_input(&input).map_err(|_| CaptureErrorResponse {
        code: "capture_output_unavailable".to_string(),
        message: format!("Failed to attach {label} asset writer input"),
    })?;

    Ok(AudioAssetWriterState {
        writer,
        input,
        started: false,
        appended_samples: 0,
        expected_sample_format,
        tail_trim_seconds: 0,
        activity_threshold: 0.0,
        tail_buffer: AudioTailSampleBuffer::default(),
        label,
    })
}

#[cfg(target_os = "macos")]
pub fn append_audio_sample_to_writer(
    writer_state: &mut AudioAssetWriterState,
    sample_buf: &cidre::cm::SampleBuf,
) -> Result<(), CaptureErrorResponse> {
    append_audio_sample_to_writer_with_activity_override(writer_state, sample_buf, None)
}

#[cfg(target_os = "macos")]
pub fn append_audio_sample_to_writer_with_activity_override(
    writer_state: &mut AudioAssetWriterState,
    sample_buf: &cidre::cm::SampleBuf,
    activity_override: Option<bool>,
) -> Result<(), CaptureErrorResponse> {
    try_append_audio_sample_to_writer_with_activity_override(
        writer_state,
        sample_buf,
        activity_override,
    )
    .map(|_| ())
}

#[cfg(target_os = "macos")]
pub fn try_append_audio_sample_to_writer_with_activity_override(
    writer_state: &mut AudioAssetWriterState,
    sample_buf: &cidre::cm::SampleBuf,
    activity_override: Option<bool>,
) -> Result<AudioSampleAppendDisposition, CaptureErrorResponse> {
    if writer_state.tail_trim_seconds > 0 {
        let active = activity_override.unwrap_or_else(|| {
            sample_buf_has_audio_activity(sample_buf, writer_state.activity_threshold)
        });
        if !writer_state.tail_buffer.append_timed(
            sample_buf.retained(),
            sample_buf_end_secs(sample_buf),
            writer_state.tail_trim_seconds,
            active,
        ) {
            return Ok(AudioSampleAppendDisposition::Dropped);
        }
        while let Some(sample) = writer_state
            .tail_buffer
            .pop_sample_before_tail(writer_state.tail_trim_seconds)
        {
            append_audio_sample_to_writer_untrimmed(writer_state, sample.as_ref())?;
        }
        return Ok(AudioSampleAppendDisposition::Appended);
    }

    append_audio_sample_to_writer_untrimmed(writer_state, sample_buf)
}

#[cfg(target_os = "macos")]
fn append_audio_sample_to_writer_untrimmed(
    writer_state: &mut AudioAssetWriterState,
    sample_buf: &cidre::cm::SampleBuf,
) -> Result<AudioSampleAppendDisposition, CaptureErrorResponse> {
    if !sample_buf.data_is_ready() {
        return Ok(AudioSampleAppendDisposition::Dropped);
    }

    if let Some(expected_format) = writer_state.expected_sample_format {
        let Some(actual_format) = derive_audio_sample_format_from_sample_buf(sample_buf) else {
            return Ok(AudioSampleAppendDisposition::Dropped);
        };

        if !audio_sample_format_is_compatible_with_writer_format(expected_format, actual_format) {
            return Ok(AudioSampleAppendDisposition::Dropped);
        }
    }

    if !writer_state.started {
        if !writer_state.writer.start_writing() {
            return Err(writer_error_response(
                &writer_state.writer,
                "capture_output_processing_failed",
                &format!("Failed to start {} audio asset writer", writer_state.label),
            ));
        }

        writer_state
            .writer
            .start_session_at_src_time(sample_buf.pts());
        writer_state.started = true;
    }

    if !writer_state.input.is_ready_for_more_media_data() {
        return Ok(AudioSampleAppendDisposition::Deferred);
    }

    let appended = writer_state
        .input
        .append_sample_buf(sample_buf)
        .map_err(|_| CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: format!(
                "Failed to append {} audio sample to asset writer",
                writer_state.label
            ),
        })?;

    if !appended {
        return Err(writer_error_response(
            &writer_state.writer,
            "capture_output_processing_failed",
            &format!(
                "Failed to append {} audio sample to asset writer",
                writer_state.label
            ),
        ));
    }

    writer_state.appended_samples += 1;

    Ok(AudioSampleAppendDisposition::Appended)
}

#[cfg(target_os = "macos")]
pub fn create_video_asset_writer(
    output_url: &cidre::ns::Url,
    label: &'static str,
) -> Result<VideoAssetWriterState, CaptureErrorResponse> {
    create_video_asset_writer_with_source_hint(output_url, label, None, None)
}

#[cfg(target_os = "macos")]
pub fn create_video_asset_writer_for_sample_buf(
    output_url: &cidre::ns::Url,
    label: &'static str,
    sample_buf: &cidre::cm::SampleBuf,
    target_bitrate_bps: Option<u32>,
) -> Result<VideoAssetWriterState, CaptureErrorResponse> {
    create_video_asset_writer_with_source_hint(
        output_url,
        label,
        sample_buf.format_desc(),
        target_bitrate_bps,
    )
}

#[cfg(target_os = "macos")]
fn create_video_asset_writer_with_source_hint(
    output_url: &cidre::ns::Url,
    label: &'static str,
    source_format_hint: Option<&cidre::cm::FormatDesc>,
    target_bitrate_bps: Option<u32>,
) -> Result<VideoAssetWriterState, CaptureErrorResponse> {
    use cidre::{av, ns};

    let _autorelease_pool = AutoreleasePoolPage::push();

    let build_output_settings = |include_bitrate: bool| {
        source_format_hint.and_then(|format_desc| {
            if format_desc.media_type() != cidre::cm::MediaType::VIDEO {
                return None;
            }

            let video_format_desc: &cidre::cm::VideoFormatDesc =
                unsafe { &*(format_desc as *const _ as *const cidre::cm::VideoFormatDesc) };
            let dims = video_format_desc.dims();
            if dims.width <= 0 || dims.height <= 0 {
                return None;
            }

            let codec_key = av::video_settings_keys::codec();
            let width_key = {
                #[allow(unused_unsafe)]
                let key = unsafe { av::video_settings_keys::width() };
                key.into_option()?
            };
            let height_key = {
                #[allow(unused_unsafe)]
                let key = unsafe { av::video_settings_keys::height() };
                key.into_option()?
            };
            let compression_props_key = av::video_settings_keys::compression_props();
            let codec = {
                #[allow(unused_unsafe)]
                let value = unsafe { av::VideoCodec::h264() };
                value.into_option()?
            };

            let width = ns::Number::with_i32(dims.width);
            let height = ns::Number::with_i32(dims.height);

            let compression_properties = if include_bitrate {
                target_bitrate_bps.map(|bitrate_bps| {
                    let average_bitrate = ns::Number::with_u32(bitrate_bps);
                    ns::Dictionary::with_keys_values(
                        &[unsafe { AVVideoAverageBitRateKey }],
                        &[average_bitrate.as_id_ref()],
                    )
                })
            } else {
                None
            };

            if let Some(compression_properties) = compression_properties {
                Some(ns::Dictionary::with_keys_values(
                    &[codec_key, width_key, height_key, compression_props_key],
                    &[
                        codec.as_id_ref(),
                        width.as_id_ref(),
                        height.as_id_ref(),
                        compression_properties.as_id_ref(),
                    ],
                ))
            } else {
                Some(ns::Dictionary::with_keys_values(
                    &[codec_key, width_key, height_key],
                    &[codec.as_id_ref(), width.as_id_ref(), height.as_id_ref()],
                ))
            }
        })
    };

    let create_writer_state = |output_settings: Option<
        cidre::arc::R<ns::Dictionary<ns::String, ns::Id>>,
    >| {
        let mut writer = av::AssetWriter::with_url_and_file_type(output_url, av::FileType::qt())
            .map_err(|error| {
                error_with_ns_error(
                    "capture_output_unavailable",
                    "Failed to create video asset writer",
                    error,
                )
            })?;

        let mut input = av::AssetWriterInput::with_media_type_output_settings_source_format_hint(
            av::MediaType::video(),
            output_settings.as_deref(),
            source_format_hint,
        )
        .map_err(|_| CaptureErrorResponse {
            code: "capture_output_unavailable".to_string(),
            message: format!("Failed to create {label} video asset writer input"),
        })?;
        input.set_expects_media_data_in_real_time(true);

        if !writer.can_add_input(&input) {
            return Err(CaptureErrorResponse {
                code: "capture_output_unavailable".to_string(),
                message: format!("Failed to add {label} video asset writer input"),
            });
        }

        writer.add_input(&input).map_err(|_| CaptureErrorResponse {
            code: "capture_output_unavailable".to_string(),
            message: format!("Failed to attach {label} video asset writer input"),
        })?;

        Ok(VideoAssetWriterState {
            writer,
            input,
            started: false,
            appended_samples: 0,
            label,
        })
    };

    let primary_output_settings = build_output_settings(true);
    match create_writer_state(primary_output_settings) {
        Ok(writer_state) => Ok(writer_state),
        Err(primary_error) if target_bitrate_bps.is_some() => {
            let fallback_output_settings = build_output_settings(false);
            create_writer_state(fallback_output_settings).or(Err(primary_error))
        }
        Err(primary_error) => Err(primary_error),
    }
}

#[cfg(target_os = "macos")]
pub fn append_video_sample_to_writer(
    writer_state: &mut VideoAssetWriterState,
    sample_buf: &cidre::cm::SampleBuf,
) -> Result<bool, CaptureErrorResponse> {
    if !sample_buf.data_is_ready() {
        return Ok(false);
    }

    if !writer_state.started {
        if !writer_state.writer.start_writing() {
            return Err(writer_error_response(
                &writer_state.writer,
                "capture_output_processing_failed",
                &format!("Failed to start {} video asset writer", writer_state.label),
            ));
        }

        writer_state
            .writer
            .start_session_at_src_time(sample_buf.pts());
        writer_state.started = true;
    }

    if !writer_state.input.is_ready_for_more_media_data() {
        return Ok(false);
    }

    let appended = writer_state
        .input
        .append_sample_buf(sample_buf)
        .map_err(|_| CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: format!(
                "Failed to append {} video sample to asset writer",
                writer_state.label
            ),
        })?;

    if !appended {
        return Err(writer_error_response(
            &writer_state.writer,
            "capture_output_processing_failed",
            &format!(
                "Failed to append {} video sample to asset writer",
                writer_state.label
            ),
        ));
    }

    writer_state.appended_samples += 1;

    Ok(true)
}

#[cfg(target_os = "macos")]
pub fn no_audio_samples_error(label: &str) -> CaptureErrorResponse {
    CaptureErrorResponse {
        code: "capture_output_processing_failed".to_string(),
        message: format!("No {label} audio samples were received; no output file was produced"),
    }
}

#[cfg(target_os = "macos")]
pub fn no_video_samples_error(label: &str) -> CaptureErrorResponse {
    CaptureErrorResponse {
        code: "capture_output_processing_failed".to_string(),
        message: format!("No {label} video samples were received; no output file was produced"),
    }
}

#[cfg(target_os = "macos")]
pub fn writer_error_response(
    writer: &cidre::av::AssetWriter,
    code: &str,
    prefix: &str,
) -> CaptureErrorResponse {
    if let Some(error) = writer.error() {
        error_with_ns_error(code, prefix, error.as_ref())
    } else {
        CaptureErrorResponse {
            code: code.to_string(),
            message: prefix.to_string(),
        }
    }
}

#[cfg(target_os = "macos")]
pub fn finish_audio_asset_writer(
    writer_state: &mut AudioAssetWriterState,
) -> Result<(), CaptureErrorResponse> {
    finish_audio_asset_writer_with_tail_policy(writer_state, true)
}

#[cfg(target_os = "macos")]
pub fn finish_audio_asset_writer_discarding_inactivity_tail(
    writer_state: &mut AudioAssetWriterState,
) -> Result<(), CaptureErrorResponse> {
    finish_audio_asset_writer_with_tail_policy(writer_state, false)
}

#[cfg(target_os = "macos")]
fn finish_audio_asset_writer_with_tail_policy(
    writer_state: &mut AudioAssetWriterState,
    flush_tail: bool,
) -> Result<(), CaptureErrorResponse> {
    let _autorelease_pool = AutoreleasePoolPage::push();

    if flush_tail {
        flush_audio_tail_buffer(writer_state)?;
    } else {
        writer_state.tail_buffer.discard_tail();
    }

    if !writer_state.started || writer_state.appended_samples == 0 {
        return Err(no_audio_samples_error(writer_state.label));
    }

    writer_state.input.mark_as_finished();
    writer_state.writer.finish_writing();

    let wait_deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match writer_state.writer.status() {
            cidre::av::asset::WriterStatus::Completed => return Ok(()),
            cidre::av::asset::WriterStatus::Failed => {
                return Err(writer_error_response(
                    &writer_state.writer,
                    "capture_output_processing_failed",
                    &format!(
                        "Failed to finalize {} audio asset writer",
                        writer_state.label
                    ),
                ));
            }
            status if Instant::now() >= wait_deadline => {
                return Err(CaptureErrorResponse {
                    code: "capture_output_processing_failed".to_string(),
                    message: format!(
                        "Timed out while finalizing {} audio asset writer (status: {:?})",
                        writer_state.label, status
                    ),
                });
            }
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

#[cfg(target_os = "macos")]
pub fn finish_video_asset_writer(
    writer_state: &mut VideoAssetWriterState,
) -> Result<(), CaptureErrorResponse> {
    let _autorelease_pool = AutoreleasePoolPage::push();

    if !writer_state.started || writer_state.appended_samples == 0 {
        return Err(no_video_samples_error(writer_state.label));
    }

    writer_state.input.mark_as_finished();
    writer_state.writer.finish_writing();

    let wait_deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match writer_state.writer.status() {
            cidre::av::asset::WriterStatus::Completed => return Ok(()),
            cidre::av::asset::WriterStatus::Failed => {
                return Err(writer_error_response(
                    &writer_state.writer,
                    "capture_output_processing_failed",
                    &format!(
                        "Failed to finalize {} video asset writer",
                        writer_state.label
                    ),
                ));
            }
            status if Instant::now() >= wait_deadline => {
                return Err(CaptureErrorResponse {
                    code: "capture_output_processing_failed".to_string(),
                    message: format!(
                        "Timed out while finalizing {} video asset writer (status: {:?})",
                        writer_state.label, status
                    ),
                });
            }
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

pub const OUTPUT_PROCESSING_FAILURE_PREFIX: &str = "Failed to finalize capture outputs: ";
const NO_AUDIO_SAMPLES_ERROR_PREFIX: &str = "No ";
const NO_AUDIO_SAMPLES_ERROR_SUFFIX: &str =
    " audio samples were received; no output file was produced";
const NO_VIDEO_SAMPLES_ERROR_PREFIX: &str = "No ";
const NO_VIDEO_SAMPLES_ERROR_SUFFIX: &str =
    " video samples were received; no output file was produced";

pub fn strip_output_processing_failure_prefix(message: &str) -> Option<&str> {
    message.strip_prefix(OUTPUT_PROCESSING_FAILURE_PREFIX)
}

pub fn single_output_processing_failure_detail<'a>(
    message: &'a str,
    additional_failure_prefixes: &[&str],
) -> Option<&'a str> {
    let detail = strip_output_processing_failure_prefix(message)?;
    (!detail.is_empty()
        && !additional_failure_prefixes
            .iter()
            .any(|prefix| detail.contains(&format!("; {prefix}"))))
    .then_some(detail)
}

pub fn is_no_audio_samples_error_message(label: &str, message: &str) -> bool {
    message
        .strip_prefix(NO_AUDIO_SAMPLES_ERROR_PREFIX)
        .and_then(|detail| detail.strip_suffix(NO_AUDIO_SAMPLES_ERROR_SUFFIX))
        .is_some_and(|actual_label| actual_label == label)
}

pub fn is_no_video_samples_error_message(label: &str, message: &str) -> bool {
    message
        .strip_prefix(NO_VIDEO_SAMPLES_ERROR_PREFIX)
        .and_then(|detail| detail.strip_suffix(NO_VIDEO_SAMPLES_ERROR_SUFFIX))
        .is_some_and(|actual_label| actual_label == label)
}

pub fn aggregate_output_processing_failures(
    failures: Vec<String>,
) -> Result<(), CaptureErrorResponse> {
    if failures.is_empty() {
        return Ok(());
    }

    Err(CaptureErrorResponse {
        code: "capture_output_processing_failed".to_string(),
        message: format!("{OUTPUT_PROCESSING_FAILURE_PREFIX}{}", failures.join("; ")),
    })
}

#[cfg(target_os = "macos")]
fn push_stream_output_first_error(
    failures: &mut Vec<String>,
    first_error: Option<CaptureErrorResponse>,
) {
    if let Some(error) = first_error {
        failures.push(format!(
            "stream output failed: [{}] {}",
            error.code, error.message
        ));
    }
}

#[cfg(target_os = "macos")]
pub fn finalize_screen_video_output_context(
    screen_video_writer: Option<&mut VideoAssetWriterState>,
    first_error: Option<CaptureErrorResponse>,
) -> Result<(), CaptureErrorResponse> {
    let mut failures: Vec<String> = Vec::new();
    push_stream_output_first_error(&mut failures, first_error);

    if let Some(writer) = screen_video_writer {
        if let Err(error) = finish_video_asset_writer(writer) {
            failures.push(format!("screen video writer failed: {}", error.message));
        }
    }

    aggregate_output_processing_failures(failures)
}

#[cfg(target_os = "macos")]
pub fn finalize_stream_output_context(
    screen_video_writer: Option<&mut VideoAssetWriterState>,
    system_audio_writer: Option<&mut AudioAssetWriterState>,
    first_error: Option<CaptureErrorResponse>,
) -> Result<(), CaptureErrorResponse> {
    let mut failures: Vec<String> = Vec::new();
    push_stream_output_first_error(&mut failures, first_error);

    if let Some(writer) = screen_video_writer {
        if let Err(error) = finish_video_asset_writer(writer) {
            failures.push(format!("screen video writer failed: {}", error.message));
        }
    }

    if let Some(writer) = system_audio_writer {
        if let Err(error) = finish_audio_asset_writer(writer) {
            failures.push(format!("system audio writer failed: {}", error.message));
        }
    }

    aggregate_output_processing_failures(failures)
}

#[cfg(target_os = "macos")]
pub fn finalize_microphone_output_context(
    writer: Option<&mut AudioAssetWriterState>,
    first_error: Option<CaptureErrorResponse>,
) -> Result<(), CaptureErrorResponse> {
    let mut failures: Vec<String> = Vec::new();

    if let Some(error) = first_error {
        failures.push(format!(
            "microphone stream output failed: [{}] {}",
            error.code, error.message
        ));
    }

    if let Some(writer) = writer {
        if let Err(error) = finish_audio_asset_writer(writer) {
            failures.push(format!("microphone writer failed: {}", error.message));
        }
    } else {
        failures.push(no_audio_samples_error("microphone").message);
    }

    aggregate_output_processing_failures(failures)
}

#[cfg(target_os = "macos")]
pub fn convert_recording_audio_to_m4a(
    recording_file: &str,
    output_file: &str,
) -> Result<(), CaptureErrorResponse> {
    let _ = std::fs::remove_file(output_file);

    let conversion = Command::new("/usr/bin/afconvert")
        .arg("-f")
        .arg("m4af")
        .arg("-d")
        .arg("aac")
        .arg("-o")
        .arg(output_file)
        .arg(recording_file)
        .output()
        .map_err(|error| CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: format!("Failed to launch audio conversion: {error}"),
        })?;

    if !conversion.status.success() {
        let stderr = String::from_utf8_lossy(&conversion.stderr);
        return Err(CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: format!(
                "Failed to convert recording audio to m4a: {}",
                stderr.trim()
            ),
        });
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn error_with_ns_error(code: &str, prefix: &str, error: &cidre::ns::Error) -> CaptureErrorResponse {
    CaptureErrorResponse {
        code: code.to_string(),
        message: format!("{prefix}: {error} (code: {})", error.code(),),
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    fn append_tail_sample<T>(
        buffer: &mut AudioTailSampleBuffer<T>,
        sample: T,
        sample_end_secs: f64,
    ) {
        assert!(buffer.append_timed(sample, Some(sample_end_secs), 2, true));
    }

    fn audio_sample_format(bits_per_channel: u32, bytes_per_frame: u32) -> AudioSampleFormat {
        AudioSampleFormat {
            sample_rate_hz: 48_000.0,
            format_id: cidre::cat::AudioFormat::LINEAR_PCM.0,
            format_flags: cidre::cat::AudioFormatFlags::IS_PACKED.0,
            bytes_per_packet: bytes_per_frame,
            frames_per_packet: 1,
            bytes_per_frame,
            channels_per_frame: 2,
            bits_per_channel,
        }
    }

    #[test]
    fn audio_writer_accepts_96khz_mono_float_microphone_format() {
        let path = std::env::temp_dir().join(format!(
            "mnema-96khz-mono-float-writer-{}.m4a",
            std::process::id()
        ));
        let path_string = path.to_string_lossy().to_string();
        let output_url = cidre::ns::Url::with_fs_path_str(&path_string, false);
        let format = AudioSampleFormat {
            sample_rate_hz: 96_000.0,
            format_id: cidre::cat::AudioFormat::LINEAR_PCM.0,
            format_flags: cidre::cat::AudioFormatFlags::IS_FLOAT.0
                | cidre::cat::AudioFormatFlags::IS_PACKED.0
                | cidre::cat::AudioFormatFlags::IS_NON_INTERLEAVED.0,
            bytes_per_packet: 4,
            frames_per_packet: 1,
            bytes_per_frame: 4,
            channels_per_frame: 1,
            bits_per_channel: 32,
        };

        let writer_result =
            create_audio_asset_writer_for_sample_format(&output_url, "microphone", format);
        let writer_error = writer_result
            .as_ref()
            .err()
            .map(|error| format!("{error:?}"));

        drop(writer_result);
        let _ = std::fs::remove_file(path);
        assert!(
            writer_error.is_none(),
            "unexpected writer error: {}",
            writer_error.unwrap_or_default()
        );
    }

    fn bytes_from_hex(hex: &str) -> Vec<u8> {
        (0..hex.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("valid hex byte"))
            .collect()
    }

    #[test]
    fn trim_validation_rejects_header_only_m4a_without_audio_tracks() {
        let path = std::env::temp_dir().join(format!(
            "mnema-header-only-trim-output-{}.m4a",
            std::process::id()
        ));
        let header_only_m4a = bytes_from_hex(
            "0000001c667479704d344120000002004d34412069736f6d69736f320000000866726565000000086d646174000000d66d6f6f760000006c6d766864000000000000000000000000000003e800000000000100000100000000000000000000000001000000000000000000000000000000010000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000200000062756474610000005a6d657461000000000000002168646c7200000000000000006d6469726170706c0000000000000000000000002d696c737400000025a9746f6f0000001d6461746100000001000000004c61766636322e31322e313030",
        );
        std::fs::write(&path, header_only_m4a).expect("header-only m4a should be written");

        let error = validate_trimmed_audio_output_file(path.to_string_lossy().as_ref())
            .expect_err("header-only m4a must be rejected");

        assert_eq!(error.code, "audio_trim_failed");
        assert!(
            error.message.contains("not readable") || error.message.contains("no audio frames"),
            "unexpected error: {}",
            error.message
        );

        let _ = std::fs::remove_file(path);
    }

    fn write_test_tone_m4a(path: &str, duration_secs: f64) {
        use cidre::{av, cat, ns};

        let _autorelease_pool = AutoreleasePoolPage::push();
        let url = ns::Url::with_fs_path_str(path, false);
        let sample_rate = 48_000.0_f64;
        let format_id = ns::Number::with_u32(cat::audio::Format::MPEG4_AAC.0);
        let sample_rate_value = ns::Number::with_f64(sample_rate);
        let channel_count_value = ns::Number::with_i64(1);
        let settings: cidre::arc::R<ns::Dictionary<ns::String, ns::Id>> =
            ns::Dictionary::with_keys_values(
                &[
                    av::audio::all_formats_keys::id(),
                    av::audio::all_formats_keys::sample_rate(),
                    av::audio::all_formats_keys::number_of_channels(),
                ],
                &[
                    format_id.as_id_ref(),
                    sample_rate_value.as_id_ref(),
                    channel_count_value.as_id_ref(),
                ],
            );
        let mut file = av::AudioFile::open_write_common_format(
            &url,
            &settings,
            av::AudioCommonFormat::PcmF32,
            false,
        )
        .expect("test tone file should open for writing");
        let processing_format = file.processing_format();
        let total_frames = (duration_secs * sample_rate).round().max(0.0) as u32;
        let mut written = 0_u32;
        while written < total_frames {
            let want = (total_frames - written).min(16_384);
            let mut buffer = av::AudioPcmBuf::with_format(&processing_format, want)
                .expect("test tone buffer should allocate");
            buffer.set_frame_len(want).expect("frame length should set");
            if let Some(samples) = buffer.data_f32_mut_at(0) {
                for (index, sample) in samples.iter_mut().enumerate() {
                    let n = (written + index as u32) as f32;
                    *sample = (n * 0.05).sin() * 0.2;
                }
            }
            file.write(&buffer).expect("test tone should write");
            written += want;
        }
        file.close();
    }

    #[test]
    fn trim_audio_file_to_m4a_extracts_subrange_without_external_tools() {
        let temp = std::env::temp_dir();
        let input = temp
            .join(format!(
                "mnema-native-trim-input-{}.m4a",
                std::process::id()
            ))
            .to_string_lossy()
            .to_string();
        let output = temp
            .join(format!(
                "mnema-native-trim-output-{}.m4a",
                std::process::id()
            ))
            .to_string_lossy()
            .to_string();
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&output);

        write_test_tone_m4a(&input, 2.0);

        trim_audio_file_to_m4a(&input, &output, 0.5, 1.0)
            .expect("native AVFoundation trim should succeed without ffmpeg on PATH");

        let trimmed = decode_audio_file_to_mono_pcm(std::path::Path::new(&output))
            .expect("trimmed output should decode");
        let trimmed_secs = trimmed.samples.len() as f64 / trimmed.sample_rate_hz as f64;
        let source = decode_audio_file_to_mono_pcm(std::path::Path::new(&input))
            .expect("source should decode");
        let source_secs = source.samples.len() as f64 / source.sample_rate_hz as f64;

        assert!(
            (0.2..1.2).contains(&trimmed_secs),
            "expected ~0.5s trimmed audio, got {trimmed_secs}s"
        );
        assert!(
            trimmed_secs < source_secs - 0.5,
            "trim must shorten the audio: trimmed {trimmed_secs}s vs source {source_secs}s"
        );

        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&output);
    }

    #[test]
    fn audio_writer_format_compatibility_allows_startup_bit_depth_transition() {
        let transient_startup_format = audio_sample_format(24, 6);
        let stable_live_format = audio_sample_format(32, 8);

        assert!(audio_sample_format_is_compatible_with_writer_format(
            stable_live_format,
            transient_startup_format
        ));
    }

    #[test]
    fn audio_writer_format_compatibility_rejects_different_encoded_format() {
        let expected = audio_sample_format(32, 8);
        let mut actual = audio_sample_format(32, 8);
        actual.format_id = cidre::cat::AudioFormat::MPEG4_AAC.0;

        assert!(!audio_sample_format_is_compatible_with_writer_format(
            expected, actual
        ));
    }

    #[test]
    fn audio_tail_buffer_flushes_samples_that_age_out_of_active_tail() {
        let mut buffer = AudioTailSampleBuffer::default();

        append_tail_sample(&mut buffer, "speech", 1.0);
        append_tail_sample(&mut buffer, "idle-1", 2.0);
        append_tail_sample(&mut buffer, "idle-2", 3.0);

        assert_eq!(buffer.pop_sample_before_tail(2), Some("speech"));
        assert_eq!(
            buffer
                .samples
                .iter()
                .map(|sample| sample.sample)
                .collect::<Vec<_>>(),
            vec!["idle-1", "idle-2"]
        );
    }

    #[test]
    fn audio_tail_buffer_normal_finish_flushes_tail_after_active_buffering() {
        let mut buffer = AudioTailSampleBuffer::default();
        let mut flushed = Vec::new();

        for (sample, end_secs) in [("speech", 1.0), ("idle-1", 2.0), ("idle-2", 3.0)] {
            append_tail_sample(&mut buffer, sample, end_secs);
            while let Some(sample) = buffer.pop_sample_before_tail(2) {
                flushed.push(sample);
            }
        }

        flushed.extend(buffer.samples.into_iter().map(|sample| sample.sample));

        assert_eq!(flushed, vec!["speech", "idle-1", "idle-2"]);
    }

    #[test]
    fn audio_tail_buffer_inactivity_finish_discards_buffered_tail_only() {
        let mut buffer = AudioTailSampleBuffer::default();
        let mut flushed = Vec::new();

        for (sample, end_secs, active) in [
            ("speech", 1.0, true),
            ("idle-1", 2.0, false),
            ("idle-2", 3.0, false),
        ] {
            assert!(buffer.append_timed(sample, Some(end_secs), 2, active));
            while let Some(sample) = buffer.pop_sample_before_tail(2) {
                flushed.push(sample);
            }
        }

        buffer.discard_tail();

        assert_eq!(flushed, vec!["speech"]);
        assert!(buffer.samples.is_empty());
        assert_eq!(buffer.latest_sample_end_secs, None);
    }

    #[test]
    fn audio_tail_buffer_flush_policy_preserves_full_audio_on_normal_finish() {
        let mut buffer = AudioTailSampleBuffer::default();

        append_tail_sample(&mut buffer, "speech", 1.0);
        append_tail_sample(&mut buffer, "idle", 2.0);

        let flushed = buffer
            .samples
            .into_iter()
            .map(|sample| sample.sample)
            .collect::<Vec<_>>();
        assert_eq!(flushed, vec!["speech", "idle"]);
    }

    #[test]
    fn audio_tail_buffer_discard_can_leave_no_usable_samples() {
        let mut buffer = AudioTailSampleBuffer::default();

        assert!(buffer.append_timed("idle-only", Some(1.0), 10, false));
        buffer.discard_tail();

        assert!(buffer.samples.is_empty());
        assert_eq!(buffer.latest_sample_end_secs, None);
    }

    #[test]
    fn audio_tail_buffer_valid_short_active_sample_survives_inactivity_trim() {
        let mut buffer = AudioTailSampleBuffer::default();
        assert!(buffer.append_timed("active-short", Some(0.25), 10, true));

        assert_eq!(buffer.pop_sample_before_tail(10), Some("active-short"));
        buffer.discard_tail();

        assert!(buffer.samples.is_empty());
    }

    #[test]
    fn audio_tail_buffer_tail_only_silent_output_is_ignored() {
        let mut buffer = AudioTailSampleBuffer::default();
        assert!(buffer.append_timed("silent-tail", Some(0.25), 10, false));

        assert_eq!(buffer.pop_sample_before_tail(10), None);
        buffer.discard_tail();

        assert!(buffer.samples.is_empty());
    }

    #[test]
    fn audio_tail_buffer_delayed_activity_marks_latest_buffered_sample() {
        let mut buffer = AudioTailSampleBuffer::default();
        assert!(buffer.append_timed("first-vad-speech-frame", Some(0.25), 10, false));

        assert_eq!(buffer.pop_sample_before_tail(10), None);
        assert!(buffer.mark_latest_sample_active());
        assert_eq!(
            buffer.pop_sample_before_tail(10),
            Some("first-vad-speech-frame")
        );
        buffer.discard_tail();

        assert!(buffer.samples.is_empty());
    }

    #[test]
    fn audio_activity_threshold_rejects_positive_low_level_noise() {
        assert!(!audio_activity_level_is_meaningful(Some(0.01), 0.08));
        assert!(audio_activity_level_is_meaningful(Some(0.08), 0.08));
        assert!(audio_activity_level_is_meaningful(Some(0.01), 0.0));
    }

    #[test]
    fn all_silent_inactivity_output_has_no_usable_samples_after_tail_discard() {
        let mut buffer = AudioTailSampleBuffer::default();
        let mut flushed = Vec::new();

        for second in 1..=9 {
            assert!(buffer.append_timed(
                format!("startup-noise-{second}"),
                Some(second as f64),
                10,
                audio_activity_level_is_meaningful(Some(0.01), 0.08),
            ));
            while let Some(sample) = buffer.pop_sample_before_tail(10) {
                flushed.push(sample);
            }
        }

        buffer.discard_tail();

        assert!(flushed.is_empty());
        assert!(buffer.samples.is_empty());
    }

    #[test]
    fn short_threshold_active_inactivity_output_survives_tail_trim() {
        let mut buffer = AudioTailSampleBuffer::default();
        assert!(buffer.append_timed(
            "short-active",
            Some(0.25),
            10,
            audio_activity_level_is_meaningful(Some(0.25), 0.08),
        ));

        assert_eq!(buffer.pop_sample_before_tail(10), Some("short-active"));
        buffer.discard_tail();

        assert!(buffer.samples.is_empty());
    }

    #[test]
    fn normal_finalize_policy_keeps_positive_duration_silent_tail() {
        let mut buffer = AudioTailSampleBuffer::default();
        assert!(buffer.append_timed(
            "startup-noise",
            Some(9.0),
            10,
            audio_activity_level_is_meaningful(Some(0.01), 0.08),
        ));

        let flushed = buffer
            .samples
            .into_iter()
            .map(|sample| sample.sample)
            .collect::<Vec<_>>();

        assert_eq!(flushed, vec!["startup-noise"]);
    }

    #[test]
    fn audio_tail_buffer_nonnumeric_timing_does_not_clear_prior_valid_samples() {
        let mut buffer = AudioTailSampleBuffer::default();
        assert!(buffer.append_timed("active-short", Some(0.25), 10, true));
        assert!(!buffer.append_timed("nonnumeric", None, 10, false));

        assert_eq!(buffer.pop_sample_before_tail(10), Some("active-short"));
        assert!(buffer.samples.is_empty());
    }

    #[test]
    fn audio_tail_buffer_keeps_unknown_timing_samples_appendable() {
        let mut buffer = AudioTailSampleBuffer::default();

        assert!(buffer.append_timed("unknown-timing-audio", None, 10, true));

        assert_eq!(
            buffer.pop_sample_before_tail(10),
            Some("unknown-timing-audio")
        );
        assert!(buffer.samples.is_empty());
    }

    #[test]
    fn single_output_processing_failure_detail_extracts_single_failure() {
        let error = aggregate_output_processing_failures(vec!["screen output missing".to_string()])
            .expect_err("single failure should aggregate");

        assert_eq!(
            single_output_processing_failure_detail(&error.message, &["system audio failed"]),
            Some("screen output missing")
        );
    }

    #[test]
    fn single_output_processing_failure_detail_rejects_multiple_failures() {
        let error = aggregate_output_processing_failures(vec![
            "screen output missing".to_string(),
            "system audio failed".to_string(),
        ])
        .expect_err("multiple failures should aggregate");

        assert_eq!(
            single_output_processing_failure_detail(&error.message, &["system audio failed"]),
            None
        );
    }

    #[test]
    fn single_output_processing_failure_detail_allows_semicolons_inside_single_failure() {
        let error = aggregate_output_processing_failures(vec![
            "No screen video samples were received; no output file was produced".to_string(),
        ])
        .expect_err("single failure should aggregate");

        assert_eq!(
            single_output_processing_failure_detail(&error.message, &["system audio failed"]),
            Some("No screen video samples were received; no output file was produced")
        );
    }

    #[test]
    fn is_no_audio_samples_error_message_matches_label_shape() {
        assert!(is_no_audio_samples_error_message(
            "microphone",
            "No microphone audio samples were received; no output file was produced"
        ));
        assert!(!is_no_audio_samples_error_message(
            "microphone",
            "No system audio audio samples were received; no output file was produced"
        ));
    }

    #[test]
    fn is_no_video_samples_error_message_matches_label_shape() {
        assert!(is_no_video_samples_error_message(
            "screen",
            "No screen video samples were received; no output file was produced"
        ));
        assert!(!is_no_video_samples_error_message(
            "screen",
            "No microphone video samples were received; no output file was produced"
        ));
    }

    fn pcm_format(
        bits_per_channel: u32,
        format_flags: cidre::cat::AudioFormatFlags,
    ) -> AudioSampleFormat {
        AudioSampleFormat {
            sample_rate_hz: 48_000.0,
            format_id: cidre::cat::AudioFormat::LINEAR_PCM.0,
            format_flags: format_flags.0,
            bytes_per_packet: bits_per_channel.saturating_add(7) / 8,
            frames_per_packet: 1,
            bytes_per_frame: bits_per_channel.saturating_add(7) / 8,
            channels_per_frame: 1,
            bits_per_channel,
        }
    }

    #[test]
    fn audio_activity_level_detects_signed_pcm_peaks() {
        let format = pcm_format(
            16,
            cidre::cat::AudioFormatFlags::IS_SIGNED_INTEGER
                | cidre::cat::AudioFormatFlags::IS_PACKED,
        );
        let bytes = [0_u8, 0_u8, 0x00, 0x40, 0xff, 0x7f];

        let level = peak_audio_activity_level_from_pcm_bytes(&bytes, format, 256);

        assert_eq!(level, Some(1.0));
    }

    #[test]
    fn audio_activity_level_detects_float_pcm_peaks() {
        let format = pcm_format(
            32,
            cidre::cat::AudioFormatFlags::IS_FLOAT | cidre::cat::AudioFormatFlags::IS_PACKED,
        );
        let bytes = [
            0.0_f32.to_ne_bytes(),
            0.5_f32.to_ne_bytes(),
            (-0.25_f32).to_ne_bytes(),
        ]
        .concat();

        let level = peak_audio_activity_level_from_pcm_bytes(&bytes, format, 256);

        assert_eq!(level, Some(0.5));
    }

    #[test]
    fn audio_activity_level_rejects_non_pcm_formats() {
        let format = AudioSampleFormat {
            sample_rate_hz: 48_000.0,
            format_id: cidre::cat::AudioFormat::MPEG4_AAC.0,
            format_flags: 0,
            bytes_per_packet: 1,
            frames_per_packet: 1,
            bytes_per_frame: 1,
            channels_per_frame: 1,
            bits_per_channel: 16,
        };

        let level = peak_audio_activity_level_from_pcm_bytes(&[1, 2, 3, 4], format, 256);

        assert_eq!(level, None);
    }

    #[test]
    fn finalize_screen_video_output_context_returns_ok_without_failures() {
        assert!(finalize_screen_video_output_context(None, None).is_ok());
    }

    #[test]
    fn finalize_screen_video_output_context_preserves_stream_output_errors() {
        let error = finalize_screen_video_output_context(
            None,
            Some(CaptureErrorResponse {
                code: "stream_failed".to_string(),
                message: "buffer callback failed".to_string(),
            }),
        )
        .expect_err("first error should be aggregated");

        assert_eq!(
            error.message,
            "Failed to finalize capture outputs: stream output failed: [stream_failed] buffer callback failed"
        );
    }
}

/// Cross-platform tests for the shared [`AudioTailSampleBuffer`] hold-back
/// logic. These run on both macOS and Windows so the single trim/discard/flush
/// implementation that both audio writers depend on is exercised on every
/// supported platform. The Windows AAC sink (`WindowsAudioTailHoldbackSink`)
/// drives the buffer with exactly this `append_timed` + `pop_sample_before_tail`
/// + flush/`discard_tail` sequence, so a `.m4a` from an inactivity stop is
/// measurably shorter than one from a normal stop — proven here at the
/// buffer/duration level (real capture hardware cannot run in tests).
#[cfg(all(test, any(target_os = "macos", target_os = "windows")))]
mod audio_tail_buffer_tests {
    use super::AudioTailSampleBuffer;

    /// A test stand-in for one buffered PCM chunk: a duration in seconds (the
    /// only attribute that matters for "is the committed file shorter").
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Chunk {
        duration_secs: f64,
    }

    /// Drives an `AudioTailSampleBuffer` exactly as the Windows sink does for a
    /// sequence of 1-second chunks, then applies the chosen tail policy. Returns
    /// the total committed duration (sum of released chunk durations) — the
    /// proxy for the finalized `.m4a`'s length.
    ///
    /// `activities` is the per-chunk activity flag (peak crossed threshold OR
    /// VAD speech). `retain_seconds` is the hold-back window. `flush` selects a
    /// normal stop (drain the buffer) vs an inactivity stop (discard it).
    fn committed_duration(activities: &[bool], retain_seconds: u64, flush: bool) -> f64 {
        let mut buffer: AudioTailSampleBuffer<Chunk> = AudioTailSampleBuffer::default();
        let mut committed = 0.0;
        for (index, &active) in activities.iter().enumerate() {
            let end_secs = (index + 1) as f64; // 1-second chunks, end at 1,2,3...
            buffer.append_timed(
                Chunk { duration_secs: 1.0 },
                Some(end_secs),
                retain_seconds,
                active,
            );
            while let Some(chunk) = buffer.pop_sample_before_tail(retain_seconds) {
                committed += chunk.duration_secs;
            }
        }
        if flush {
            while let Some(timed) = buffer.samples.pop_front() {
                committed += timed.sample.duration_secs;
            }
        } else {
            buffer.discard_tail();
        }
        committed
    }

    #[test]
    fn inactivity_stop_is_shorter_than_normal_stop_when_tail_is_idle() {
        // 5 chunks: speech for the first 2 seconds, then 3 seconds of idle tail,
        // with a 2-second hold-back window.
        let activities = [true, true, false, false, false];

        let normal = committed_duration(&activities, 2, /* flush */ true);
        let inactivity = committed_duration(&activities, 2, /* flush */ false);

        // A normal stop commits the whole 5 seconds; an inactivity stop drops the
        // withheld idle tail, so its committed `.m4a` is strictly shorter.
        assert_eq!(normal, 5.0);
        assert!(
            inactivity < normal,
            "inactivity committed {inactivity}s must be shorter than normal {normal}s"
        );
        // The 2-second hold-back window of idle audio is exactly what gets
        // discarded, leaving the 3 seconds that streamed through before it.
        assert_eq!(inactivity, 3.0);
    }

    #[test]
    fn normal_stop_keeps_full_duration_with_no_activity() {
        // Even with no activity at all, a normal stop must flush everything so a
        // user-initiated stop never silently truncates audio.
        let activities = [false, false, false, false];
        let normal = committed_duration(&activities, 2, true);
        assert_eq!(normal, 4.0);
    }

    #[test]
    fn inactivity_stop_with_no_activity_discards_only_the_holdback_window() {
        // With no observed activity, nothing is released during streaming, so the
        // inactivity discard drops everything still buffered.
        let activities = [false, false, false, false];
        let inactivity = committed_duration(&activities, 2, false);
        assert_eq!(inactivity, 0.0);
    }

    #[test]
    fn peak_and_vad_boundary_refinement_release_the_same_audio() {
        // The buffer is signal-agnostic: it only sees the boolean `active` flag.
        // Whether that flag came from peak-level or VAD-speech refinement, an
        // identical activity pattern trims to an identical boundary — so both
        // modes share one tested implementation.
        let pattern = [false, true, true, false, false];
        let by_peak = committed_duration(&pattern, 2, false);
        let by_vad = committed_duration(&pattern, 2, false);
        assert_eq!(by_peak, by_vad);
        // Activity through second 3 with a 2s window keeps 3 seconds; the final
        // 2-second idle tail is discarded.
        assert_eq!(by_peak, 3.0);
    }

    #[test]
    fn late_activity_pulse_preserves_previously_buffered_audio() {
        // Mirrors the VAD pulse path: append idle chunks (buffered), then mark the
        // latest buffered chunk active. Once activity is observed the backlog is
        // released, so a following normal stop keeps the whole stream.
        let mut buffer: AudioTailSampleBuffer<Chunk> = AudioTailSampleBuffer::default();
        for index in 0..4 {
            let end_secs = (index + 1) as f64;
            buffer.append_timed(Chunk { duration_secs: 1.0 }, Some(end_secs), 2, false);
        }
        // No activity yet: nothing has been released.
        assert!(buffer.pop_sample_before_tail(2).is_none());
        // A late speech pulse marks the latest chunk active.
        assert!(buffer.mark_latest_sample_active());
        let mut released = 0.0;
        while let Some(chunk) = buffer.pop_sample_before_tail(2) {
            released += chunk.duration_secs;
        }
        while let Some(timed) = buffer.samples.pop_front() {
            released += timed.sample.duration_secs;
        }
        assert_eq!(released, 4.0);
    }
}

// ===========================================================================
// Windows AAC / MPEG-4 (`.m4a`) sink writer
// ===========================================================================
//
// The Windows microphone backend captures default-endpoint PCM on a dedicated
// capture thread and feeds it here to be encoded to AAC and muxed into a
// playable `.m4a` via the Media Foundation `IMFSinkWriter`. This mirrors the
// screen backend's `IMFSinkWriter` usage (`capture-screen::windows_capture`),
// reusing the same wide-path / memory-buffer / sample-write pattern but for an
// audio (PCM -> AAC) stream instead of video (NV12 -> H.264).
//
// Threading / lifetime: every item here is single-thread-affine COM state that
// lives on the capture thread; nothing is `Send`. `create` calls `MFStartup`
// defensively (Media Foundation reference-counts startup/shutdown, so the
// capture session's own `MFStartup`/`MFShutdown` around the whole session stays
// balanced) and `finalize` deliberately does NOT call `MFShutdown` — the
// capture thread owns the matching shutdown so we never double-shutdown.

#[cfg(target_os = "windows")]
mod windows_aac_m4a {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use super::{audio_activity_level_is_meaningful, AudioTailSampleBuffer};
    use capture_types::CaptureErrorResponse;
    use windows::core::{GUID, PCWSTR};
    use windows::Win32::Media::MediaFoundation::{
        IMFMediaBuffer, IMFSample, IMFSinkWriter, IMFSourceReader, MFAudioFormat_AAC,
        MFAudioFormat_Float, MFAudioFormat_PCM, MFCreateMediaType, MFCreateMemoryBuffer,
        MFCreateSample, MFCreateSinkWriterFromURL, MFCreateSourceReaderFromURL, MFMediaType_Audio,
        MFShutdown, MFStartup, MFSTARTUP_FULL, MF_MT_AAC_AUDIO_PROFILE_LEVEL_INDICATION,
        MF_MT_AAC_PAYLOAD_TYPE, MF_MT_ALL_SAMPLES_INDEPENDENT, MF_MT_AUDIO_AVG_BYTES_PER_SECOND,
        MF_MT_AUDIO_BITS_PER_SAMPLE, MF_MT_AUDIO_BLOCK_ALIGNMENT, MF_MT_AUDIO_NUM_CHANNELS,
        MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_PD_DURATION,
        MF_SOURCE_READER_FIRST_AUDIO_STREAM, MF_SOURCE_READER_MEDIASOURCE, MF_VERSION,
    };
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Variant::VT_I8;

    /// One AAC stream muxed into a `.m4a` via `IMFSinkWriter`.
    ///
    /// Owns capture-thread-local Media Foundation state; not `Send`.
    pub struct WindowsAacM4aSinkWriter {
        writer: IMFSinkWriter,
        stream_index: u32,
        /// Count of `WriteSample` calls that actually reached the encoder. When
        /// zero, `Finalize()` would fail with `MF_E_SINK_NO_SAMPLES_PROCESSED`
        /// (surfaced as `windows_audio_writer_failed`), so callers skip it and
        /// treat the segment as empty — mirroring the macOS
        /// `appended_samples == 0` guard.
        written_samples: u64,
    }

    impl WindowsAacM4aSinkWriter {
        /// Create a sink writer that encodes interleaved 16-bit PCM to AAC and
        /// writes a playable `.m4a` at `output_path`.
        pub fn create(
            output_path: &Path,
            sample_rate_hz: u32,
            channels: u16,
        ) -> Result<Self, CaptureErrorResponse> {
            // 128 kbps; a valid AAC `MF_MT_AUDIO_AVG_BYTES_PER_SECOND` value.
            const AAC_AVG_BYTES_PER_SECOND: u32 = 16_000;
            let block_alignment = channels as u32 * 2;

            let url: Vec<u16> = output_path
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();

            unsafe {
                // No MFStartup here: `create` only ever runs on the capture
                // thread, which performs exactly one MFStartup/MFShutdown around
                // the whole session. A per-create startup would leave the MF
                // refcount unbalanced (finalize deliberately does not shut down),
                // so we rely on the thread's single startup.
                let output_type = MFCreateMediaType()
                    .map_err(|e| win_error("MFCreateMediaType (AAC output) failed", &e))?;
                output_type
                    .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
                    .map_err(|e| win_error("set output major type failed", &e))?;
                output_type
                    .SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)
                    .map_err(|e| win_error("set output subtype failed", &e))?;
                output_type
                    .SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)
                    .map_err(|e| win_error("set output bits per sample failed", &e))?;
                output_type
                    .SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, sample_rate_hz)
                    .map_err(|e| win_error("set output sample rate failed", &e))?;
                output_type
                    .SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, channels as u32)
                    .map_err(|e| win_error("set output channels failed", &e))?;
                output_type
                    .SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, AAC_AVG_BYTES_PER_SECOND)
                    .map_err(|e| win_error("set output avg bytes per second failed", &e))?;
                output_type
                    .SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, 0)
                    .map_err(|e| win_error("set output AAC payload type failed", &e))?;
                output_type
                    .SetUINT32(&MF_MT_AAC_AUDIO_PROFILE_LEVEL_INDICATION, 0x29)
                    .map_err(|e| win_error("set output AAC profile level failed", &e))?;

                let writer = MFCreateSinkWriterFromURL(PCWSTR(url.as_ptr()), None, None)
                    .map_err(|e| win_error("MFCreateSinkWriterFromURL failed", &e))?;

                let stream_index = writer
                    .AddStream(&output_type)
                    .map_err(|e| win_error("AddStream failed", &e))?;

                let input_type = MFCreateMediaType()
                    .map_err(|e| win_error("MFCreateMediaType (PCM input) failed", &e))?;
                input_type
                    .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
                    .map_err(|e| win_error("set input major type failed", &e))?;
                input_type
                    .SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)
                    .map_err(|e| win_error("set input subtype failed", &e))?;
                input_type
                    .SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)
                    .map_err(|e| win_error("set input bits per sample failed", &e))?;
                input_type
                    .SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, sample_rate_hz)
                    .map_err(|e| win_error("set input sample rate failed", &e))?;
                input_type
                    .SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, channels as u32)
                    .map_err(|e| win_error("set input channels failed", &e))?;
                input_type
                    .SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, block_alignment)
                    .map_err(|e| win_error("set input block alignment failed", &e))?;
                input_type
                    .SetUINT32(
                        &MF_MT_AUDIO_AVG_BYTES_PER_SECOND,
                        sample_rate_hz * block_alignment,
                    )
                    .map_err(|e| win_error("set input avg bytes per second failed", &e))?;
                input_type
                    .SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 1)
                    .map_err(|e| win_error("set input all samples independent failed", &e))?;

                writer
                    .SetInputMediaType(stream_index, &input_type, None)
                    .map_err(|e| win_error("SetInputMediaType failed", &e))?;
                writer
                    .BeginWriting()
                    .map_err(|e| win_error("BeginWriting failed", &e))?;

                Ok(Self {
                    writer,
                    stream_index,
                    written_samples: 0,
                })
            }
        }

        /// Append one chunk of interleaved 16-bit little-endian PCM.
        ///
        /// `sample_time_100ns` / `duration_100ns` are Media Foundation 100ns ticks.
        pub fn append_pcm_s16(
            &mut self,
            pcm_le_bytes: &[u8],
            sample_time_100ns: i64,
            duration_100ns: i64,
        ) -> Result<(), CaptureErrorResponse> {
            if pcm_le_bytes.is_empty() {
                return Ok(());
            }
            unsafe {
                let buffer: IMFMediaBuffer = MFCreateMemoryBuffer(pcm_le_bytes.len() as u32)
                    .map_err(|e| win_error("MFCreateMemoryBuffer failed", &e))?;

                let mut data_ptr: *mut u8 = std::ptr::null_mut();
                buffer
                    .Lock(&mut data_ptr, None, None)
                    .map_err(|e| win_error("IMFMediaBuffer.Lock failed", &e))?;
                std::ptr::copy_nonoverlapping(pcm_le_bytes.as_ptr(), data_ptr, pcm_le_bytes.len());
                buffer
                    .Unlock()
                    .map_err(|e| win_error("IMFMediaBuffer.Unlock failed", &e))?;
                buffer
                    .SetCurrentLength(pcm_le_bytes.len() as u32)
                    .map_err(|e| win_error("SetCurrentLength failed", &e))?;

                let sample: IMFSample =
                    MFCreateSample().map_err(|e| win_error("MFCreateSample failed", &e))?;
                sample
                    .AddBuffer(&buffer)
                    .map_err(|e| win_error("IMFSample.AddBuffer failed", &e))?;
                sample
                    .SetSampleTime(sample_time_100ns)
                    .map_err(|e| win_error("SetSampleTime failed", &e))?;
                sample
                    .SetSampleDuration(duration_100ns)
                    .map_err(|e| win_error("SetSampleDuration failed", &e))?;

                self.writer
                    .WriteSample(self.stream_index, &sample)
                    .map_err(|e| win_error("WriteSample failed", &e))?;
            }
            self.written_samples += 1;
            Ok(())
        }

        /// Whether any sample has been written to the encoder. `Finalize()` must
        /// not be called when this is `false` (MF would fail with
        /// `MF_E_SINK_NO_SAMPLES_PROCESSED`).
        pub fn has_written_samples(&self) -> bool {
            self.written_samples > 0
        }

        /// Finalize the MPEG-4 container, flushing the AAC encoder. Does not call
        /// `MFShutdown` (the capture thread owns the balancing shutdown).
        pub fn finalize(self) -> Result<(), CaptureErrorResponse> {
            unsafe {
                self.writer
                    .Finalize()
                    .map_err(|e| win_error("IMFSinkWriter.Finalize failed", &e))?;
            }
            Ok(())
        }
    }

    /// One PCM chunk withheld in the rolling tail buffer ahead of the AAC sink
    /// writer, retaining the exact Media Foundation timing it must carry when (and
    /// if) it is flushed to the encoder.
    pub(super) struct PendingTailPcm {
        pcm: Vec<u8>,
        sample_time_100ns: i64,
        duration_100ns: i64,
    }

    /// Wraps a [`WindowsAacM4aSinkWriter`] with the platform-neutral
    /// [`AudioTailSampleBuffer`] so the last N seconds of PCM are held back
    /// *before* the AAC encoder. This mirrors the macOS asset-writer hold-back
    /// (`set_audio_writer_inactivity_tail_trim_seconds` + `tail_buffer`):
    ///
    /// - A normal stop or a segment rotation [`flush`](Self::finalize_flushing)es
    ///   the held tail into the encoder, so the committed `.m4a` is whole.
    /// - An inactivity stop
    ///   [`discard`](Self::finalize_discarding_inactivity_tail)s it, so the
    ///   committed `.m4a` is measurably shorter than wall-clock — the dead idle
    ///   tail never reaches the file.
    ///
    /// The trim boundary is refined by activity exactly as on macOS: a chunk is
    /// `active` when its peak level crosses the configured threshold
    /// (`MicrophoneInactivityTailTrimActivityMode::PeakLevel`) or when the
    /// caller pulses speech via [`mark_tail_active`](Self::mark_tail_active)
    /// (`VadSpeech`). Once activity is observed, samples older than the retained
    /// window drain straight through to the encoder.
    pub struct WindowsAudioTailHoldbackSink {
        writer: Option<WindowsAacM4aSinkWriter>,
        tail: AudioTailSampleBuffer<PendingTailPcm>,
        sample_rate_hz: u32,
        /// Length of the withheld window, in seconds. `0` disables hold-back and
        /// every chunk passes straight to the encoder (legacy behavior).
        tail_trim_seconds: u64,
        /// Peak-level activity threshold in `0.0..=1.0` used in `PeakLevel` mode.
        activity_threshold: f32,
        /// Whether the boundary is refined by caller-supplied VAD speech pulses
        /// rather than the per-chunk peak level.
        vad_speech_mode: bool,
    }

    impl WindowsAudioTailHoldbackSink {
        pub fn new(writer: WindowsAacM4aSinkWriter, sample_rate_hz: u32) -> Self {
            Self {
                writer: Some(writer),
                tail: AudioTailSampleBuffer::default(),
                sample_rate_hz,
                tail_trim_seconds: 0,
                activity_threshold: 0.0,
                vad_speech_mode: false,
            }
        }

        /// Configure the hold-back window and activity boundary. `vad_speech_mode`
        /// selects VAD-speech boundary refinement; otherwise per-chunk peak level
        /// is used. Matches `set_audio_writer_inactivity_tail_trim_seconds` +
        /// `set_audio_writer_activity_threshold` on macOS.
        pub fn configure_tail_holdback(
            &mut self,
            tail_trim_seconds: u64,
            activity_threshold: f32,
            vad_speech_mode: bool,
        ) {
            self.tail_trim_seconds = tail_trim_seconds;
            self.activity_threshold = if activity_threshold.is_finite() {
                activity_threshold.clamp(0.0, 1.0)
            } else {
                0.0
            };
            self.vad_speech_mode = vad_speech_mode;
        }

        /// Append one interleaved 16-bit LE PCM chunk. `peak` is the chunk's peak
        /// mono level in `0.0..=1.0` (used for `PeakLevel` boundary refinement);
        /// `vad_speech` marks the chunk as speech-active when in VAD mode.
        ///
        /// When hold-back is enabled the chunk is buffered and only samples older
        /// than the retained window (once activity has been observed) are released
        /// to the encoder; otherwise it is written straight through.
        pub fn append_pcm_s16(
            &mut self,
            pcm: &[u8],
            sample_time_100ns: i64,
            duration_100ns: i64,
            peak: f32,
            vad_speech: bool,
        ) -> Result<(), CaptureErrorResponse> {
            if pcm.is_empty() {
                return Ok(());
            }

            if self.tail_trim_seconds == 0 {
                if let Some(writer) = self.writer.as_mut() {
                    writer.append_pcm_s16(pcm, sample_time_100ns, duration_100ns)?;
                }
                return Ok(());
            }

            let active = if self.vad_speech_mode {
                vad_speech
            } else {
                audio_activity_level_is_meaningful(Some(peak), self.activity_threshold)
            };
            let end_secs = self.chunk_end_secs(sample_time_100ns, duration_100ns);
            self.tail.append_timed(
                PendingTailPcm {
                    pcm: pcm.to_vec(),
                    sample_time_100ns,
                    duration_100ns,
                },
                end_secs,
                self.tail_trim_seconds,
                active,
            );
            self.drain_released_tail()
        }

        /// Retroactively mark the most recently buffered chunk as active and
        /// release everything now older than the retained window. Mirrors the
        /// macOS `record_audio_writer_tail_activity`: VAD speech decisions arrive
        /// after the chunk was buffered, so the held audio up to (and including)
        /// the speech must be preserved while the trailing no-speech tail can
        /// still be discarded on inactivity.
        pub fn mark_tail_active(&mut self) -> Result<bool, CaptureErrorResponse> {
            if !self.tail.mark_latest_sample_active() {
                return Ok(false);
            }
            self.drain_released_tail()?;
            Ok(true)
        }

        fn drain_released_tail(&mut self) -> Result<(), CaptureErrorResponse> {
            while let Some(pending) = self.tail.pop_sample_before_tail(self.tail_trim_seconds) {
                self.write_pending(&pending)?;
            }
            Ok(())
        }

        fn write_pending(&mut self, pending: &PendingTailPcm) -> Result<(), CaptureErrorResponse> {
            if let Some(writer) = self.writer.as_mut() {
                writer.append_pcm_s16(
                    &pending.pcm,
                    pending.sample_time_100ns,
                    pending.duration_100ns,
                )?;
            }
            Ok(())
        }

        fn chunk_end_secs(&self, sample_time_100ns: i64, duration_100ns: i64) -> Option<f64> {
            if self.sample_rate_hz == 0 {
                return None;
            }
            let end_ticks = sample_time_100ns.saturating_add(duration_100ns.max(0));
            Some(end_ticks as f64 / super::WINDOWS_MF_TICKS_PER_SECOND as f64)
        }

        /// Flush the withheld tail into the encoder and finalize the `.m4a`. Used
        /// for a normal stop or a segment rotation, where the segment must be
        /// whole. Mirrors `finish_audio_asset_writer` on macOS.
        ///
        /// Returns `true` if a non-empty `.m4a` was finalized, `false` if nothing
        /// ever reached the encoder (empty segment — no output file).
        pub fn finalize_flushing(mut self) -> Result<bool, CaptureErrorResponse> {
            while let Some(pending) = self.tail.samples.pop_front() {
                self.write_pending(&pending.sample)?;
            }
            self.finalize_writer()
        }

        /// Discard the withheld tail and finalize the `.m4a`. Used for an
        /// inactivity stop so the committed segment never carries the dead idle
        /// tail. Mirrors `finish_audio_asset_writer_discarding_inactivity_tail`.
        ///
        /// Returns `true` if a non-empty `.m4a` was finalized, `false` if the
        /// discard left nothing that ever reached the encoder (empty segment) —
        /// the inactivity case the macOS `appended_samples == 0` guard covers.
        pub fn finalize_discarding_inactivity_tail(
            mut self,
        ) -> Result<bool, CaptureErrorResponse> {
            self.tail.discard_tail();
            self.finalize_writer()
        }

        /// Finalize the underlying sink writer. Skips `IMFSinkWriter::Finalize()`
        /// when no sample ever reached the encoder, since MF would fail it with
        /// `MF_E_SINK_NO_SAMPLES_PROCESSED`; returns `false` in that case so the
        /// caller records an empty segment (no output file) instead of claiming a
        /// playable `.m4a`.
        fn finalize_writer(&mut self) -> Result<bool, CaptureErrorResponse> {
            if let Some(writer) = self.writer.take() {
                if !writer.has_written_samples() {
                    return Ok(false);
                }
                writer.finalize()?;
                return Ok(true);
            }
            Ok(false)
        }
    }

    /// MF Source Reader positive-duration probe. Opens `path` with
    /// `MFCreateSourceReaderFromURL` and reads `MF_PD_DURATION`; returns true iff
    /// it opens and the duration is > 0.
    pub fn windows_audio_file_has_positive_duration(path: &str) -> bool {
        let url: Vec<u16> = Path::new(path)
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        unsafe {
            // The probe reuses the session's Media Foundation startup; call it
            // defensively here so a standalone probe still works (reference
            // counted; no unbalanced shutdown).
            if MFStartup(MF_VERSION, MFSTARTUP_FULL).is_err() {
                return false;
            }
            let result = probe_positive_duration(&url);
            MFShutdown().ok();
            result
        }
    }

    /// MF Source Reader duration probe. Opens `path` with
    /// `MFCreateSourceReaderFromURL`, reads `MF_PD_DURATION` (100ns ticks), and
    /// returns the duration in milliseconds. Returns `None` if the file cannot be
    /// opened, the attribute is missing, or the duration is zero.
    pub fn windows_audio_file_duration_ms(path: &str) -> Option<u64> {
        let url: Vec<u16> = Path::new(path)
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        unsafe {
            if MFStartup(MF_VERSION, MFSTARTUP_FULL).is_err() {
                return None;
            }
            let result = probe_duration_ms(&url);
            MFShutdown().ok();
            result
        }
    }

    unsafe fn probe_duration_ms(url: &[u16]) -> Option<u64> {
        let reader: IMFSourceReader =
            MFCreateSourceReaderFromURL(PCWSTR(url.as_ptr()), None).ok()?;

        let propvariant = reader
            .GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)
            .ok()?;

        // MF_PD_DURATION is a VT_UI8 (100ns ticks). Read the unsigned 64-bit
        // value directly from the PROPVARIANT union.
        let duration_100ns: u64 = propvariant.Anonymous.Anonymous.Anonymous.uhVal;
        if duration_100ns == 0 {
            return None;
        }

        // Convert 100ns ticks to milliseconds, rounding to nearest.
        Some((duration_100ns + 5_000) / 10_000)
    }

    unsafe fn probe_positive_duration(url: &[u16]) -> bool {
        let reader: IMFSourceReader = match MFCreateSourceReaderFromURL(PCWSTR(url.as_ptr()), None)
        {
            Ok(reader) => reader,
            Err(_) => return false,
        };

        let propvariant = match reader
            .GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)
        {
            Ok(value) => value,
            Err(_) => return false,
        };

        // MF_PD_DURATION is a VT_UI8 (100ns ticks). Read the unsigned 64-bit
        // value directly from the PROPVARIANT union.
        let duration_100ns = propvariant.Anonymous.Anonymous.Anonymous.uhVal;
        duration_100ns > 0
    }

    fn win_error(context: &str, error: &windows::core::Error) -> CaptureErrorResponse {
        CaptureErrorResponse {
            code: "windows_audio_writer_failed".to_string(),
            message: format!("{context}: {error}"),
        }
    }

    /// `ENDOFSTREAM` bit of the `ReadSample` stream-flags out-parameter
    /// (`MF_SOURCE_READERF_ENDOFSTREAM`); the `windows` crate surfaces the flags as
    /// a raw `u32` without this named constant.
    const MF_SOURCE_READER_FLAG_ENDOFSTREAM: u32 = 0x2;
    /// `CURRENTMEDIATYPECHANGED` bit of the `ReadSample` stream-flags out-parameter
    /// (`MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED`); set when the decoded output
    /// type changed mid-stream, so a cached channel count must be re-read.
    const MF_SOURCE_READER_FLAG_CURRENTMEDIATYPECHANGED: u32 = 0x20;
    /// Defensive bound on the run of flag-only `ReadSample` callbacks that carry no
    /// buffer, so a truncated/malformed source that never reports end-of-stream
    /// fails fast during trim instead of spinning a core forever.
    const MAX_CONSECUTIVE_EMPTY_READS: u32 = 1024;

    /// Trim `input` to the `[start_secs, end_secs]` range and re-encode the result
    /// as AAC/m4a at `output`, preserving the source's native channel count and
    /// sample rate so a trimmed clip is byte-format identical to an untrimmed one.
    ///
    /// Windows twin of the macOS [`crate::trim_audio_file_to_m4a`] (same signature),
    /// so the shared microphone finalize call site is platform-neutral. It runs
    /// entirely on Media Foundation — an MF Source Reader decodes the source `.m4a`
    /// to IEEE-float PCM (preserving the native rate/channels), and the
    /// [`WindowsAacM4aSinkWriter`] re-encodes the windowed sub-range to AAC — so it
    /// works inside the packaged desktop app with no external `ffmpeg` on `PATH`.
    ///
    /// The sub-range is selected by **presentation timestamp**, not by an
    /// exact-sample seek: MF seeks land on AAC frame boundaries, so we seek to (or
    /// before) the start and window each decoded sample to the requested absolute
    /// frame range. That absorbs AAC encoder-delay/priming tolerance and stays
    /// correct even when the seek is approximate or unsupported (it then decodes
    /// from the head and the windowing still selects exactly the requested frames).
    pub fn trim_audio_file_to_m4a(
        input: &str,
        output: &str,
        start_secs: f64,
        end_secs: f64,
    ) -> Result<(), CaptureErrorResponse> {
        if !start_secs.is_finite() || !end_secs.is_finite() || end_secs < start_secs {
            return Err(CaptureErrorResponse {
                code: "invalid_audio_trim_range".to_string(),
                message: "Invalid audio trim range".to_string(),
            });
        }

        let _ = std::fs::remove_file(output);

        let input_url: Vec<u16> = Path::new(input)
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let output_path = Path::new(output);

        let result = unsafe {
            // Reference-counted MFStartup balanced by the MFShutdown below — the
            // same defensive pattern the duration probes use. The sink writer's
            // `create`/`finalize` deliberately do not touch MFStartup/MFShutdown, so
            // this scope owns the Media Foundation platform lifetime for the whole
            // trim (decode + re-encode + finalize).
            if MFStartup(MF_VERSION, MFSTARTUP_FULL).is_err() {
                return Err(CaptureErrorResponse {
                    code: "audio_trim_failed".to_string(),
                    message: "MFStartup failed for audio trim".to_string(),
                });
            }
            let r = trim_with_source_reader(&input_url, output_path, start_secs, end_secs);
            MFShutdown().ok();
            r
        };
        if let Err(error) = result {
            // Never leave a partially written artifact behind, mirroring the macOS
            // validate-and-remove failure path.
            let _ = std::fs::remove_file(output);
            return Err(error);
        }

        // Validate the finalized output exactly as the macOS twin does: an output
        // with no decodable audio (e.g. an empty requested range) is removed and
        // reported rather than left as a bogus zero-duration `.m4a`.
        if !windows_audio_file_has_positive_duration(output) {
            let _ = std::fs::remove_file(output);
            return Err(CaptureErrorResponse {
                code: "audio_trim_failed".to_string(),
                message: "Trimmed audio output contains no audio frames".to_string(),
            });
        }

        Ok(())
    }

    /// Decode `input_url` with an MF Source Reader and re-encode the
    /// `[start_secs, end_secs]` sub-range to AAC at `output_path` via
    /// [`WindowsAacM4aSinkWriter`], matching the sink to the source's native
    /// rate/channels. Assumes the caller holds an active `MFStartup`.
    unsafe fn trim_with_source_reader(
        input_url: &[u16],
        output_path: &Path,
        start_secs: f64,
        end_secs: f64,
    ) -> Result<(), CaptureErrorResponse> {
        let reader: IMFSourceReader = MFCreateSourceReaderFromURL(PCWSTR(input_url.as_ptr()), None)
            .map_err(|e| trim_error("MFCreateSourceReaderFromURL (trim) failed", &e))?;

        let stream_index = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;

        // Ask MF to decode to uncompressed IEEE-float PCM, leaving rate/channels
        // unset so the source's native layout is preserved; read both back so the
        // sink can mirror them. (Same negotiation the media-decode MF seam uses.)
        let output_type = MFCreateMediaType()
            .map_err(|e| trim_error("MFCreateMediaType (trim Float) failed", &e))?;
        output_type
            .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
            .map_err(|e| trim_error("set trim output major type failed", &e))?;
        output_type
            .SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float)
            .map_err(|e| trim_error("set trim output subtype failed", &e))?;
        reader
            .SetCurrentMediaType(stream_index, None, &output_type)
            .map_err(|e| trim_error("SetCurrentMediaType (trim Float) failed", &e))?;

        let actual_type = reader
            .GetCurrentMediaType(stream_index)
            .map_err(|e| trim_error("GetCurrentMediaType (trim) failed", &e))?;
        let sample_rate_hz = actual_type
            .GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)
            .map_err(|e| trim_error("GetUINT32(trim samples per second) failed", &e))?;
        let mut channels = actual_type
            .GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)
            .map_err(|e| trim_error("GetUINT32(trim num channels) failed", &e))?;
        if sample_rate_hz == 0 || channels == 0 {
            return Err(CaptureErrorResponse {
                code: "audio_trim_failed".to_string(),
                message: "Source audio reported zero sample rate or channels for trim".to_string(),
            });
        }

        let sample_rate = sample_rate_hz as f64;
        let start_frame = (start_secs * sample_rate).round().max(0.0) as i64;
        let end_frame = ((end_secs * sample_rate).round().max(0.0) as i64).max(start_frame);

        // Match the sink to the source's native rate + channel count so the trimmed
        // `.m4a` is byte-format identical to an untrimmed mic clip (mono stays mono,
        // stereo stays stereo).
        let mut sink =
            WindowsAacM4aSinkWriter::create(output_path, sample_rate_hz, channels as u16)
                .map_err(as_trim_failure)?;

        // Seek to (or before) the start; MF lands on an AAC frame boundary, so the
        // forward decode below windows each delivered sample by its presentation
        // timestamp.
        let _ = reader.SetCurrentPosition(
            &GUID::zeroed(),
            &i8_propvariant(
                (start_secs * super::WINDOWS_MF_TICKS_PER_SECOND as f64)
                    .round()
                    .max(0.0) as i64,
            ),
        );

        let mut written_frames: i64 = 0;
        let mut consecutive_empty_reads: u32 = 0;
        loop {
            let mut stream_flags: u32 = 0;
            let mut timestamp_100ns: i64 = 0;
            let mut sample = None;
            reader
                .ReadSample(
                    stream_index,
                    0,
                    None,
                    Some(&mut stream_flags),
                    Some(&mut timestamp_100ns),
                    Some(&mut sample),
                )
                .map_err(|e| trim_error("ReadSample (trim) failed", &e))?;

            if (stream_flags & MF_SOURCE_READER_FLAG_ENDOFSTREAM) != 0 {
                break;
            }
            if (stream_flags & MF_SOURCE_READER_FLAG_CURRENTMEDIATYPECHANGED) != 0 {
                let changed = reader
                    .GetCurrentMediaType(stream_index)
                    .map_err(|e| trim_error("GetCurrentMediaType (trim change) failed", &e))?;
                let new_channels = changed.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).map_err(|e| {
                    trim_error("GetUINT32(trim channels, after change) failed", &e)
                })?;
                // The sink is fixed to the channel count chosen at create; a real
                // layout change would desync it, so fail rather than emit a
                // mislabeled file. (Uniform mic `.m4a` never changes mid-stream.)
                if new_channels != channels {
                    return Err(CaptureErrorResponse {
                        code: "audio_trim_failed".to_string(),
                        message: format!(
                            "Source channel layout changed mid-trim ({channels} -> {new_channels}); sink is fixed-format"
                        ),
                    });
                }
                channels = new_channels;
            }

            let Some(sample) = sample else {
                consecutive_empty_reads += 1;
                if consecutive_empty_reads >= MAX_CONSECUTIVE_EMPTY_READS {
                    return Err(CaptureErrorResponse {
                        code: "audio_trim_failed".to_string(),
                        message: "Media Foundation returned too many empty reads without end-of-stream during trim".to_string(),
                    });
                }
                continue;
            };
            consecutive_empty_reads = 0;

            let sample_first_frame = (timestamp_100ns as f64
                / super::WINDOWS_MF_TICKS_PER_SECOND as f64
                * sample_rate)
                .round() as i64;
            // The sample starts past the requested range: nothing more to copy.
            if sample_first_frame >= end_frame {
                break;
            }

            if let Some((pcm, frames_copied)) = windowed_s16_from_sample(
                &sample,
                sample_first_frame,
                start_frame,
                end_frame,
                channels as usize,
            )? {
                // Rebase the trimmed clip to a zero origin so the output timeline
                // starts at 0 like an untrimmed segment.
                let sample_time_100ns = (written_frames as f64 / sample_rate
                    * super::WINDOWS_MF_TICKS_PER_SECOND as f64)
                    .round() as i64;
                let duration_100ns = (frames_copied as f64 / sample_rate
                    * super::WINDOWS_MF_TICKS_PER_SECOND as f64)
                    .round() as i64;
                sink.append_pcm_s16(&pcm, sample_time_100ns, duration_100ns)
                    .map_err(as_trim_failure)?;
                written_frames += frames_copied as i64;
            }
        }

        if written_frames == 0 || !sink.has_written_samples() {
            return Err(CaptureErrorResponse {
                code: "audio_trim_failed".to_string(),
                message: "Audio trim range selected no audio frames".to_string(),
            });
        }
        sink.finalize().map_err(as_trim_failure)?;
        Ok(())
    }

    /// Window one decoded IEEE-float sample to the `[start_frame, end_frame)`
    /// absolute-frame range and convert the overlap to interleaved 16-bit LE PCM
    /// (the format [`WindowsAacM4aSinkWriter::append_pcm_s16`] expects). Returns the
    /// PCM bytes and the number of frames copied, or `None` when the sample lies
    /// entirely outside the requested range.
    unsafe fn windowed_s16_from_sample(
        sample: &IMFSample,
        sample_first_frame: i64,
        start_frame: i64,
        end_frame: i64,
        channels: usize,
    ) -> Result<Option<(Vec<u8>, usize)>, CaptureErrorResponse> {
        let buffer = sample
            .ConvertToContiguousBuffer()
            .map_err(|e| trim_error("ConvertToContiguousBuffer (trim) failed", &e))?;

        let mut data_ptr: *mut u8 = std::ptr::null_mut();
        let mut current_len: u32 = 0;
        buffer
            .Lock(&mut data_ptr, None, Some(&mut current_len))
            .map_err(|e| trim_error("IMFMediaBuffer.Lock (trim) failed", &e))?;

        let frame_stride = channels * std::mem::size_of::<f32>();
        let mut windowed: Option<(Vec<u8>, usize)> = None;
        if frame_stride > 0 && (current_len as usize) >= frame_stride && !data_ptr.is_null() {
            let byte_len = current_len as usize;
            let total_frames = (byte_len / frame_stride) as i64;
            let sample_end_frame = sample_first_frame + total_frames;
            let copy_from = start_frame.max(sample_first_frame);
            let copy_to = end_frame.min(sample_end_frame);
            if copy_to > copy_from {
                let frames_to_copy = (copy_to - copy_from) as usize;
                let frame_offset = (copy_from - sample_first_frame) as usize;
                // Byte-wise `f32::from_le_bytes` rather than reinterpreting the
                // locked region as `*const f32`: MF does not guarantee 4-byte
                // alignment of the pointer, so an aligned cast would be UB.
                let bytes = std::slice::from_raw_parts(data_ptr, byte_len);
                let mut pcm = Vec::with_capacity(frames_to_copy * channels * 2);
                for frame in 0..frames_to_copy {
                    let base = (frame_offset + frame) * frame_stride;
                    for channel in 0..channels {
                        let offset = base + channel * 4;
                        let value = f32::from_le_bytes([
                            bytes[offset],
                            bytes[offset + 1],
                            bytes[offset + 2],
                            bytes[offset + 3],
                        ]);
                        let scaled = (value.clamp(-1.0, 1.0) * 32767.0).round();
                        pcm.extend_from_slice(&(scaled as i16).to_le_bytes());
                    }
                }
                windowed = Some((pcm, frames_to_copy));
            }
        }

        buffer
            .Unlock()
            .map_err(|e| trim_error("IMFMediaBuffer.Unlock (trim) failed", &e))?;
        Ok(windowed)
    }

    /// Build a `VT_I8` PROPVARIANT carrying a 100ns tick count for
    /// `IMFSourceReader::SetCurrentPosition` (`GUID_NULL` time format => 100ns
    /// units).
    unsafe fn i8_propvariant(ticks_100ns: i64) -> PROPVARIANT {
        let mut variant = PROPVARIANT::default();
        let inner = &mut variant.Anonymous.Anonymous;
        inner.vt = VT_I8;
        inner.Anonymous.hVal = ticks_100ns;
        variant
    }

    fn trim_error(context: &str, error: &windows::core::Error) -> CaptureErrorResponse {
        CaptureErrorResponse {
            code: "audio_trim_failed".to_string(),
            message: format!("{context}: {error}"),
        }
    }

    /// Re-stamp a [`WindowsAacM4aSinkWriter`] error as an `audio_trim_failed` so the
    /// trim presents the same error contract (`invalid_audio_trim_range` /
    /// `audio_trim_failed`) as the macOS twin.
    fn as_trim_failure(error: CaptureErrorResponse) -> CaptureErrorResponse {
        CaptureErrorResponse {
            code: "audio_trim_failed".to_string(),
            message: error.message,
        }
    }
}

#[cfg(target_os = "windows")]
pub use windows_aac_m4a::{
    trim_audio_file_to_m4a, windows_audio_file_duration_ms,
    windows_audio_file_has_positive_duration, WindowsAacM4aSinkWriter, WindowsAudioTailHoldbackSink,
};

/// Runtime coverage for the F02 zero-sample finalize guard on the REAL
/// production Windows AAC sink (`WindowsAacM4aSinkWriter` /
/// `WindowsAudioTailHoldbackSink`), not the platform-neutral
/// `AudioTailSampleBuffer` stand-in the `audio_tail_buffer_tests` module covers.
///
/// These drive an actual `IMFSinkWriter` through Media Foundation on the runner
/// (no capture device needed — only the in-box AAC encoder), so the
/// `has_written_samples`/`finalize_writer` skip-guard is exercised end to end:
/// a fully-held-back segment discarded on an inactivity stop must report an
/// empty segment (`Ok(false)`) WITHOUT calling `IMFSinkWriter::Finalize()`
/// (which would fail `MF_E_SINK_NO_SAMPLES_PROCESSED` -> the user-facing
/// `windows_audio_writer_failed`), while a segment that wrote samples finalizes
/// a playable `.m4a` (`Ok(true)`).
#[cfg(all(test, target_os = "windows"))]
mod windows_aac_m4a_finalize_tests {
    use crate::{
        trim_audio_file_to_m4a, windows_audio_file_duration_ms,
        windows_audio_file_has_positive_duration, WindowsAacM4aSinkWriter,
        WindowsAudioTailHoldbackSink,
    };
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use windows::core::PCWSTR;
    use windows::Win32::Media::MediaFoundation::{
        IMFSourceReader, MFAudioFormat_Float, MFCreateMediaType, MFCreateSourceReaderFromURL,
        MFMediaType_Audio, MFShutdown, MFStartup, MFSTARTUP_FULL, MF_MT_AUDIO_NUM_CHANNELS,
        MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_SOURCE_READER_FIRST_AUDIO_STREAM, MF_VERSION,
    };
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    const SAMPLE_RATE_HZ: u32 = 48_000;
    const MF_TICKS_PER_SECOND: i64 = 10_000_000;

    /// One mono 16-bit-LE PCM chunk of `duration_ms` of silence.
    fn silent_chunk(duration_ms: u64) -> Vec<u8> {
        let samples = (u64::from(SAMPLE_RATE_HZ) * duration_ms / 1_000) as usize;
        vec![0u8; samples * 2]
    }

    /// RAII Media Foundation platform startup for a test (the production sink
    /// relies on the capture thread's single `MFStartup`; a standalone test must
    /// supply its own). Balanced by `MFShutdown` on drop; the duration probe does
    /// its own ref-counted startup/shutdown in between, which stays balanced.
    struct MfPlatform;
    impl MfPlatform {
        fn startup() -> Self {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                MFStartup(MF_VERSION, MFSTARTUP_FULL).expect("MFStartup");
            }
            Self
        }
    }
    impl Drop for MfPlatform {
        fn drop(&mut self) {
            unsafe {
                MFShutdown().ok();
            }
        }
    }

    fn temp_path(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mnema-aac-{label}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir.join(format!("{label}.m4a"))
    }

    #[test]
    fn holdback_discard_of_fully_held_segment_reports_empty_without_finalize() {
        let _mf = MfPlatform::startup();
        let path = temp_path("held-discarded");

        let writer = WindowsAacM4aSinkWriter::create(&path, SAMPLE_RATE_HZ, 1)
            .expect("create AAC sink writer");
        let mut sink = WindowsAudioTailHoldbackSink::new(writer, SAMPLE_RATE_HZ);
        // 2s hold-back with a high activity threshold: silent chunks never cross
        // it, so every chunk stays withheld and nothing is released to the encoder.
        sink.configure_tail_holdback(2, 0.5, false);

        let chunk = silent_chunk(500);
        let duration_100ns = MF_TICKS_PER_SECOND / 2; // 0.5s
        for i in 0..3 {
            sink.append_pcm_s16(&chunk, i * duration_100ns, duration_100ns, 0.0, false)
                .expect("append held silent chunk");
        }

        // Inactivity stop: discard the withheld tail. Nothing ever reached the
        // encoder, so the guard must skip Finalize() and report an empty segment
        // rather than erroring with MF_E_SINK_NO_SAMPLES_PROCESSED.
        let finalized = sink.finalize_discarding_inactivity_tail().expect(
            "discarding finalize of an empty segment must not error (no \
             MF_E_SINK_NO_SAMPLES_PROCESSED -> windows_audio_writer_failed)",
        );
        assert!(
            !finalized,
            "a fully-held-back-then-discarded segment must report empty (no output file)"
        );
        assert!(
            !windows_audio_file_has_positive_duration(&path.to_string_lossy()),
            "the skipped-Finalize .m4a must not present a positive duration"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn flush_of_written_segment_finalizes_playable_m4a() {
        let _mf = MfPlatform::startup();
        let path = temp_path("written-flushed");

        let writer = WindowsAacM4aSinkWriter::create(&path, SAMPLE_RATE_HZ, 1)
            .expect("create AAC sink writer");
        let mut sink = WindowsAudioTailHoldbackSink::new(writer, SAMPLE_RATE_HZ);
        // Hold-back disabled: each chunk is written straight through to the encoder.
        sink.configure_tail_holdback(0, 0.0, false);

        let chunk = silent_chunk(1_000);
        sink.append_pcm_s16(&chunk, 0, MF_TICKS_PER_SECOND, 0.0, false)
            .expect("append straight-through chunk");

        let finalized = sink
            .finalize_flushing()
            .expect("flushing finalize of a written segment");
        assert!(
            finalized,
            "a segment with written samples must finalize a real .m4a (Ok(true))"
        );
        assert!(
            windows_audio_file_has_positive_duration(&path.to_string_lossy()),
            "the finalized .m4a must present a positive duration"
        );

        let _ = std::fs::remove_file(&path);
    }

    /// Generate a self-contained source `.m4a` of `duration_secs` at `channels`
    /// channels by driving the production AAC sink writer directly — no external
    /// tools — so the trim fixtures are reproducible on any runner that has the
    /// in-box AAC encoder. Writes a quiet sine tone (interleaved across channels)
    /// so the file carries real, decodable audio.
    fn write_tone_m4a(path: &Path, channels: u16, duration_secs: f64) {
        let mut writer = WindowsAacM4aSinkWriter::create(path, SAMPLE_RATE_HZ, channels)
            .expect("create source AAC sink writer");

        let total_frames = (duration_secs * SAMPLE_RATE_HZ as f64).round() as u64;
        let chunk_frames = u64::from(SAMPLE_RATE_HZ / 10); // 100ms chunks
        let mut frame: u64 = 0;
        while frame < total_frames {
            let frames = chunk_frames.min(total_frames - frame);
            let mut pcm = Vec::with_capacity(frames as usize * channels as usize * 2);
            for f in 0..frames {
                let n = (frame + f) as f32;
                let value = ((n * 0.05).sin() * 0.2 * 32767.0) as i16;
                for _ in 0..channels {
                    pcm.extend_from_slice(&value.to_le_bytes());
                }
            }
            let sample_time_100ns =
                (frame as f64 / SAMPLE_RATE_HZ as f64 * MF_TICKS_PER_SECOND as f64).round() as i64;
            let duration_100ns = (frames as f64 / SAMPLE_RATE_HZ as f64
                * MF_TICKS_PER_SECOND as f64)
                .round() as i64;
            writer
                .append_pcm_s16(&pcm, sample_time_100ns, duration_100ns)
                .expect("append source tone chunk");
            frame += frames;
        }
        writer.finalize().expect("finalize source tone .m4a");
    }

    /// Read back the channel count an MF Source Reader negotiates for `path` (the
    /// native channel layout of the encoded audio), used to assert the trim keeps a
    /// mono source mono and a stereo source stereo. Assumes the test's `MfPlatform`
    /// startup is live.
    fn decoded_channel_count(path: &Path) -> u32 {
        let url: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            let reader: IMFSourceReader =
                MFCreateSourceReaderFromURL(PCWSTR(url.as_ptr()), None).expect("open source reader");
            let stream_index = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
            let output_type = MFCreateMediaType().expect("create media type");
            output_type
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
                .expect("set major type");
            output_type
                .SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float)
                .expect("set subtype");
            reader
                .SetCurrentMediaType(stream_index, None, &output_type)
                .expect("negotiate Float output");
            let actual = reader
                .GetCurrentMediaType(stream_index)
                .expect("read negotiated type");
            actual
                .GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)
                .expect("read channel count")
        }
    }

    /// Shared body for the mono + stereo trim cases: generate a 2s source at
    /// `channels`, trim it to the `[0.5, 1.0]s` sub-range via the production
    /// Media-Foundation re-encode trim, and assert (a) the trimmed `.m4a` covers the
    /// requested sub-range within AAC encoder-delay/frame tolerance and is clearly
    /// shorter than the source, and (b) the trim preserves the source channel count.
    fn assert_trim_preserves_channels_and_subrange(channels: u16, label: &str) {
        let _mf = MfPlatform::startup();
        let source = temp_path(&format!("trim-src-{label}"));
        let trimmed = temp_path(&format!("trim-out-{label}"));

        write_tone_m4a(&source, channels, 2.0);
        let source_str = source.to_string_lossy().to_string();
        let trimmed_str = trimmed.to_string_lossy().to_string();

        trim_audio_file_to_m4a(&source_str, &trimmed_str, 0.5, 1.0)
            .expect("MF re-encode trim should succeed without external tools");

        // Sub-range coverage: the ~0.5s request lands well inside the AAC
        // encoder-delay/frame tolerance window and is clearly shorter than 2s.
        let trimmed_ms = windows_audio_file_duration_ms(&trimmed_str)
            .expect("trimmed .m4a should report a positive duration");
        let source_ms = windows_audio_file_duration_ms(&source_str)
            .expect("source .m4a should report a positive duration");
        assert!(
            (200..=1200).contains(&trimmed_ms),
            "expected ~0.5s ({label}) trimmed audio, got {trimmed_ms}ms"
        );
        assert!(
            trimmed_ms + 300 < source_ms,
            "trim must shorten the audio ({label}): trimmed {trimmed_ms}ms vs source {source_ms}ms"
        );

        // Native-format preservation: a mono source stays mono, a stereo source
        // stays stereo, so a trimmed clip is channel-identical to an untrimmed one.
        assert_eq!(
            decoded_channel_count(&source),
            u32::from(channels),
            "source fixture channel count ({label})"
        );
        assert_eq!(
            decoded_channel_count(&trimmed),
            u32::from(channels),
            "trimmed output must preserve the source channel count ({label})"
        );

        let _ = std::fs::remove_file(&source);
        let _ = std::fs::remove_file(&trimmed);
    }

    #[test]
    fn trim_preserves_mono_source_channel_count_and_subrange() {
        assert_trim_preserves_channels_and_subrange(1, "mono");
    }

    #[test]
    fn trim_preserves_stereo_source_channel_count_and_subrange() {
        assert_trim_preserves_channels_and_subrange(2, "stereo");
    }
}
