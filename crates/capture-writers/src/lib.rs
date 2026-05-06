use capture_types::CaptureErrorResponse;

#[cfg(target_os = "macos")]
#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    static AVVideoAverageBitRateKey: &'static cidre::ns::String;
}

#[cfg(target_os = "macos")]
use cidre::objc::autorelease_pool::AutoreleasePoolPage;
#[cfg(target_os = "macos")]
use std::collections::VecDeque;
#[cfg(target_os = "macos")]
use std::process::Command;
#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};

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

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct AudioTailSampleBuffer<T> {
    samples: VecDeque<TimedAudioTailSample<T>>,
    latest_sample_end_secs: Option<f64>,
    observed_active_sample: bool,
}

#[cfg(target_os = "macos")]
impl<T> Default for AudioTailSampleBuffer<T> {
    fn default() -> Self {
        Self {
            samples: VecDeque::new(),
            latest_sample_end_secs: None,
            observed_active_sample: false,
        }
    }
}

#[cfg(target_os = "macos")]
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

#[cfg(target_os = "macos")]
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

#[cfg(target_os = "macos")]
fn audio_activity_level_is_meaningful(level: Option<f32>, threshold: f32) -> bool {
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
#[derive(Debug)]
pub struct VideoAssetWriterState {
    writer: cidre::arc::R<cidre::av::AssetWriter>,
    input: cidre::arc::R<cidre::av::AssetWriterInput>,
    started: bool,
    appended_samples: u64,
    label: &'static str,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy)]
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
    create_audio_asset_writer_with_format_internal(
        output_url,
        label,
        sample_format.to_writer_format(),
        Some(sample_format),
    )
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
            return Ok(());
        }
        while let Some(sample) = writer_state
            .tail_buffer
            .pop_sample_before_tail(writer_state.tail_trim_seconds)
        {
            append_audio_sample_to_writer_untrimmed(writer_state, sample.as_ref())?;
        }
        return Ok(());
    }

    append_audio_sample_to_writer_untrimmed(writer_state, sample_buf)
}

#[cfg(target_os = "macos")]
fn append_audio_sample_to_writer_untrimmed(
    writer_state: &mut AudioAssetWriterState,
    sample_buf: &cidre::cm::SampleBuf,
) -> Result<(), CaptureErrorResponse> {
    if !sample_buf.data_is_ready() {
        return Ok(());
    }

    if let Some(expected_format) = writer_state.expected_sample_format {
        let Some(actual_format) = derive_audio_sample_format_from_sample_buf(sample_buf) else {
            return Ok(());
        };

        if actual_format != expected_format {
            return Ok(());
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
        return Ok(());
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

    Ok(())
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
