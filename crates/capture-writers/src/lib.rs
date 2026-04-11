use capture_types::CaptureErrorResponse;

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
    label: &'static str,
}

#[cfg(target_os = "macos")]
pub fn create_audio_asset_writer(
    output_url: &cidre::ns::Url,
    label: &'static str,
) -> Result<AudioAssetWriterState, CaptureErrorResponse> {
    use cidre::{av, cat, ns};

    let format_id = ns::Number::with_u32(cat::audio::Format::MPEG4_AAC.0);
    let sample_rate = ns::Number::with_i64(48_000);
    let channel_count = ns::Number::with_i64(2);

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
pub fn no_audio_samples_error(label: &str) -> CaptureErrorResponse {
    CaptureErrorResponse {
        code: "capture_output_processing_failed".to_string(),
        message: format!("No {label} audio samples were received; no output file was produced"),
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
    microphone_writer: Option<&mut AudioAssetWriterState>,
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

    if let Some(writer) = microphone_writer {
        if let Err(error) = finish_audio_asset_writer(writer) {
            failures.push(format!("microphone writer failed: {}", error.message));
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
    writer: &mut AudioAssetWriterState,
    first_error: Option<CaptureErrorResponse>,
) -> Result<(), CaptureErrorResponse> {
    let mut failures: Vec<String> = Vec::new();

    if let Some(error) = first_error {
        failures.push(format!(
            "microphone stream output failed: [{}] {}",
            error.code, error.message
        ));
    }

    if let Err(error) = finish_audio_asset_writer(writer) {
        failures.push(format!("microphone writer failed: {}", error.message));
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
