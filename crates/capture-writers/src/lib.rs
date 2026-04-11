use capture_types::CaptureErrorResponse;

#[cfg(target_os = "macos")]
#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    static AVVideoAverageBitRateKey: &'static cidre::ns::String;
}

#[cfg(target_os = "macos")]
use std::process::Command;
#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
#[derive(Debug)]
pub struct AudioAssetWriterState {
    writer: cidre::arc::R<cidre::av::AssetWriter>,
    input: cidre::arc::R<cidre::av::AssetWriterInput>,
    started: bool,
    appended_samples: u64,
    expected_sample_format: Option<AudioSampleFormat>,
    label: &'static str,
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
        label,
    })
}

#[cfg(target_os = "macos")]
pub fn append_audio_sample_to_writer(
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
                    &[
                        av::video_settings_keys::codec(),
                        av::video_settings_keys::width(),
                        av::video_settings_keys::height(),
                        av::video_settings_keys::compression_props(),
                    ],
                    &[
                        av::VideoCodec::h264().as_id_ref(),
                        width.as_id_ref(),
                        height.as_id_ref(),
                        compression_properties.as_id_ref(),
                    ],
                ))
            } else {
                Some(ns::Dictionary::with_keys_values(
                    &[
                        av::video_settings_keys::codec(),
                        av::video_settings_keys::width(),
                        av::video_settings_keys::height(),
                    ],
                    &[
                        av::VideoCodec::h264().as_id_ref(),
                        width.as_id_ref(),
                        height.as_id_ref(),
                    ],
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
) -> Result<(), CaptureErrorResponse> {
    if !sample_buf.data_is_ready() {
        return Ok(());
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
        return Ok(());
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

    Ok(())
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

#[cfg(target_os = "macos")]
pub fn aggregate_output_processing_failures(
    failures: Vec<String>,
) -> Result<(), CaptureErrorResponse> {
    if failures.is_empty() {
        return Ok(());
    }

    Err(CaptureErrorResponse {
        code: "capture_output_processing_failed".to_string(),
        message: format!(
            "Failed to finalize capture outputs: {}",
            failures.join("; ")
        ),
    })
}

#[cfg(target_os = "macos")]
pub fn finalize_stream_output_context(
    screen_video_writer: Option<&mut VideoAssetWriterState>,
    system_audio_writer: Option<&mut AudioAssetWriterState>,
    first_error: Option<CaptureErrorResponse>,
) -> Result<(), CaptureErrorResponse> {
    let mut failures: Vec<String> = Vec::new();

    if let Some(error) = first_error {
        failures.push(format!(
            "stream output failed: [{}] {}",
            error.code, error.message
        ));
    }

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
