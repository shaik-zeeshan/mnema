#[cfg(any(test, all(target_os = "macos", any(feature = "sherpa-onnx", feature = "speakrs"))))]
use crate::{SpeakerAnalysisError, SpeakerAnalysisResult};

#[cfg(all(target_os = "macos", any(feature = "sherpa-onnx", feature = "speakrs")))]
use std::path::Path;
#[cfg(any(test, all(target_os = "macos", any(feature = "sherpa-onnx", feature = "speakrs"))))]
use std::time::{Duration, Instant};

#[cfg(all(target_os = "macos", any(feature = "sherpa-onnx", feature = "speakrs")))]
use tempfile::NamedTempFile;

#[cfg(any(test, all(target_os = "macos", any(feature = "sherpa-onnx", feature = "speakrs"))))]
#[derive(Debug)]
pub(crate) struct DecodedAudio {
    pub(crate) samples: Vec<f32>,
    pub(crate) sample_rate_hz: u32,
}

#[cfg(all(target_os = "macos", any(feature = "sherpa-onnx", feature = "speakrs")))]
const AVASSETREADER_WRITER_READY_TIMEOUT: Duration = Duration::from_secs(30);

#[cfg(all(target_os = "macos", any(feature = "sherpa-onnx", feature = "speakrs")))]
const AVASSETREADER_WRITER_READY_POLL_INTERVAL: Duration = Duration::from_millis(1);

#[cfg(any(test, all(target_os = "macos", any(feature = "sherpa-onnx", feature = "speakrs"))))]
fn wait_for_writer_input_ready(
    mut is_ready: impl FnMut() -> bool,
    timeout: Duration,
    poll_interval: Duration,
) -> bool {
    let started_at = Instant::now();
    loop {
        if is_ready() {
            return true;
        }
        let elapsed = started_at.elapsed();
        if elapsed >= timeout {
            return false;
        }
        std::thread::sleep(poll_interval.min(timeout - elapsed));
    }
}

#[cfg(all(target_os = "macos", any(feature = "sherpa-onnx", feature = "speakrs")))]
pub(crate) fn decode_audio_to_mono_with_avassetreader_fallback(
    path: &Path,
) -> SpeakerAnalysisResult<DecodedAudio> {
    decode_with_fallback(
        || avaudiofile_decode_audio_to_mono(path),
        || avassetreader_decode_audio_to_mono(path),
        "AVAssetReader WAV fallback",
    )
}

#[cfg(any(test, all(target_os = "macos", any(feature = "sherpa-onnx", feature = "speakrs"))))]
fn decode_with_fallback<T, FPrimary, FFallback>(
    primary: FPrimary,
    fallback: FFallback,
    fallback_label: &str,
) -> SpeakerAnalysisResult<T>
where
    FPrimary: FnOnce() -> SpeakerAnalysisResult<T>,
    FFallback: FnOnce() -> SpeakerAnalysisResult<T>,
{
    match primary() {
        Ok(value) => Ok(value),
        Err(primary_error) => fallback().map_err(|fallback_error| {
            SpeakerAnalysisError::Analysis(format!(
                "{primary_error}; {fallback_label} also failed: {fallback_error}"
            ))
        }),
    }
}

#[cfg(all(target_os = "macos", any(feature = "sherpa-onnx", feature = "speakrs")))]
fn avaudiofile_decode_audio_to_mono(path: &Path) -> SpeakerAnalysisResult<DecodedAudio> {
    use cidre::{av, ns, objc};

    let path_str = path.to_str().ok_or_else(|| {
        SpeakerAnalysisError::InvalidRequest(format!(
            "audio path is not valid UTF-8 for AVFoundation: {}",
            path.display()
        ))
    })?;

    let _pool = objc::autorelease_pool::AutoreleasePoolPage::push();
    let url = ns::Url::with_fs_path_str(path_str, false);
    let mut file =
        av::AudioFile::open_read_common_format(&url, av::AudioCommonFormat::PcmF32, false)
            .map_err(|error| {
                SpeakerAnalysisError::Analysis(format!(
                    "AVFoundation failed to open audio file {}: {error}",
                    path.display()
                ))
            })?;
    let format = file.processing_format();
    let sample_rate = format.absd().sample_rate;
    if !sample_rate.is_finite() || sample_rate <= 0.0 {
        return Err(SpeakerAnalysisError::Analysis(format!(
            "AVFoundation reported invalid sample rate {sample_rate} for {}",
            path.display()
        )));
    }
    let sample_rate_hz = sample_rate.round().clamp(1.0, u32::MAX as f64) as u32;
    let channels = usize::try_from(format.channel_count()).unwrap_or(0);
    if channels == 0 {
        return Err(SpeakerAnalysisError::Analysis(format!(
            "AVFoundation reported zero channels for {}",
            path.display()
        )));
    }

    let mut out = Vec::new();
    let total_frames = file.len().max(0) as u64;
    let chunk_frames = 16_384_u32;
    let mut remaining = total_frames;
    while remaining > 0 {
        let frames = remaining.min(chunk_frames as u64) as u32;
        let mut buffer = av::AudioPcmBuf::with_format(&format, frames).ok_or_else(|| {
            SpeakerAnalysisError::Analysis("failed to allocate AVAudioPCMBuffer".to_string())
        })?;
        file.read_n(&mut buffer, frames).map_err(|error| {
            SpeakerAnalysisError::Analysis(format!(
                "AVFoundation failed reading audio file {}: {error}",
                path.display()
            ))
        })?;
        let frame_len = buffer.frame_len() as usize;
        if frame_len == 0 {
            break;
        }
        append_downmixed_f32(&mut out, &buffer, channels, frame_len)?;
        remaining = remaining.saturating_sub(frame_len as u64);
    }

    Ok(DecodedAudio {
        samples: out,
        sample_rate_hz,
    })
}

#[cfg(all(target_os = "macos", any(feature = "sherpa-onnx", feature = "speakrs")))]
fn avassetreader_decode_audio_to_mono(path: &Path) -> SpeakerAnalysisResult<DecodedAudio> {
    let temp_wav = NamedTempFile::new().map_err(|error| {
        SpeakerAnalysisError::Analysis(format!(
            "failed to allocate temporary WAV for {}: {error}",
            path.display()
        ))
    })?;
    transcode_audio_to_wav_with_asset_reader(path, temp_wav.path())?;
    avaudiofile_decode_audio_to_mono(temp_wav.path())
}

#[cfg(all(target_os = "macos", any(feature = "sherpa-onnx", feature = "speakrs")))]
fn transcode_audio_to_wav_with_asset_reader(
    source_path: &Path,
    wav_path: &Path,
) -> SpeakerAnalysisResult<()> {
    use cidre::{av, cat, cm, ns, objc};

    let source_path_str = source_path.to_str().ok_or_else(|| {
        SpeakerAnalysisError::InvalidRequest(format!(
            "audio path is not valid UTF-8 for AVAssetReader: {}",
            source_path.display()
        ))
    })?;
    let wav_path_str = wav_path.to_str().ok_or_else(|| {
        SpeakerAnalysisError::InvalidRequest(format!(
            "temporary WAV path is not valid UTF-8 for AVAssetReader: {}",
            wav_path.display()
        ))
    })?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| {
            SpeakerAnalysisError::Analysis(format!(
                "failed to start AVAssetReader fallback runtime for {}: {error}",
                source_path.display()
            ))
        })?;

    runtime.block_on(async {
        let _pool = objc::autorelease_pool::AutoreleasePoolPage::push();
        let source_url = ns::Url::with_fs_path_str(source_path_str, false);
        let asset = av::UrlAsset::with_url(&source_url, None).ok_or_else(|| {
            SpeakerAnalysisError::Analysis(format!(
                "AVAssetReader could not create source asset for {}",
                source_path.display()
            ))
        })?;
        let tracks = asset
            .load_tracks_with_media_type(av::MediaType::audio())
            .await
            .map_err(|error| {
                SpeakerAnalysisError::Analysis(format!(
                    "AVAssetReader failed loading audio tracks for {}: {error}",
                    source_path.display()
                ))
            })?;
        let track = tracks.get(0).map_err(|_| {
            SpeakerAnalysisError::Analysis(format!(
                "AVAssetReader found no audio tracks in {}",
                source_path.display()
            ))
        })?;

        let mut output = av::AssetReaderTrackOutput::with_track(&track, None).map_err(|error| {
            SpeakerAnalysisError::Analysis(format!(
                "AVAssetReader failed to create track output for {}: {error}",
                source_path.display()
            ))
        })?;
        output.set_always_copies_sample_data(false);

        let mut reader = av::AssetReader::with_asset(&asset).map_err(|error| {
            SpeakerAnalysisError::Analysis(format!(
                "AVAssetReader failed to initialize for {}: {error}",
                source_path.display()
            ))
        })?;
        reader.add_output(&output).map_err(|error| {
            SpeakerAnalysisError::Analysis(format!(
                "AVAssetReader failed to attach track output for {}: {error}",
                source_path.display()
            ))
        })?;

        let wav_url = ns::Url::with_fs_path_str(wav_path_str, false);
        let mut writer =
            av::AssetWriter::with_url_and_file_type(&wav_url, av::FileType::wav()).map_err(
                |error| {
                    SpeakerAnalysisError::Analysis(format!(
                        "AVAssetWriter failed to create temporary WAV for {}: {error}",
                        source_path.display()
                    ))
                },
            )?;

        if !reader.start_reading().map_err(|error| {
            SpeakerAnalysisError::Analysis(format!(
                "AVAssetReader failed to start reading {}: {error}",
                source_path.display()
            ))
        })? {
            return Err(SpeakerAnalysisError::Analysis(format!(
                "AVAssetReader could not start reading {} (status: {:?}, error: {})",
                source_path.display(),
                reader.status(),
                reader
                    .error()
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            )));
        }

        let first_buf = output
            .next_sample_buf()
            .map_err(|error| {
                SpeakerAnalysisError::Analysis(format!(
                    "AVAssetReader failed to read the first sample from {}: {error}",
                    source_path.display()
                ))
            })?
            .ok_or_else(|| {
                SpeakerAnalysisError::Analysis(format!(
                    "AVAssetReader decoded no audio samples from {}",
                    source_path.display()
                ))
            })?;
        let format_desc = first_buf.format_desc().ok_or_else(|| {
            SpeakerAnalysisError::Analysis(format!(
                "AVAssetReader returned the first sample without a format description for {}",
                source_path.display()
            ))
        })?;
        let source_asbd = format_desc.stream_basic_desc().ok_or_else(|| {
            SpeakerAnalysisError::Analysis(format!(
                "AVAssetReader returned the first sample without an audio stream description for {}",
                source_path.display()
            ))
        })?;
        let source_hint = cm::AudioFormatDesc::with_asbd(source_asbd).map_err(|error| {
            SpeakerAnalysisError::Analysis(format!(
                "AVAssetReader failed to derive an audio format hint for {}: {error}",
                source_path.display()
            ))
        })?;

        let output_settings =
            ns::Dictionary::with_keys_values(&[av::audio::all_formats_keys::id()], &[cat::AudioFormat::LINEAR_PCM.as_ref()]);
        let mut input = av::AssetWriterInput::with_media_type_output_settings_source_format_hint(
            av::MediaType::audio(),
            Some(output_settings.as_ref()),
            Some(&source_hint),
        )
        .map_err(|error| {
            SpeakerAnalysisError::Analysis(format!(
                "AVAssetWriter failed to create a WAV writer input for {}: {error}",
                source_path.display()
            ))
        })?;
        writer.add_input(&input).map_err(|error| {
            SpeakerAnalysisError::Analysis(format!(
                "AVAssetWriter failed to attach a WAV writer input for {}: {error}",
                source_path.display()
            ))
        })?;

        if !writer.start_writing() {
            return Err(SpeakerAnalysisError::Analysis(format!(
                "AVAssetWriter failed to start writing temporary WAV for {} (status: {:?}, error: {})",
                source_path.display(),
                writer.status(),
                writer
                    .error()
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            )));
        }
        writer.start_session_at_src_time(cm::Time::zero());

        let mut current_buf = Some(first_buf);
        loop {
            if !wait_for_writer_input_ready(
                || input.is_ready_for_more_media_data(),
                AVASSETREADER_WRITER_READY_TIMEOUT,
                AVASSETREADER_WRITER_READY_POLL_INTERVAL,
            ) {
                reader.cancel_reading();
                writer.cancel_writing();
                return Err(SpeakerAnalysisError::Analysis(format!(
                    "AVAssetWriter timed out waiting for input readiness while transcoding {} to WAV",
                    source_path.display()
                )));
            }

            let Some(buf) = current_buf.take() else {
                match output.next_sample_buf().map_err(|error| {
                    SpeakerAnalysisError::Analysis(format!(
                        "AVAssetReader failed while transcoding {} to WAV: {error}",
                        source_path.display()
                    ))
                })? {
                    Some(next) => {
                        current_buf = Some(next);
                        continue;
                    }
                    None => break,
                }
            };

            if !input.append_sample_buf(&buf).map_err(|error| {
                SpeakerAnalysisError::Analysis(format!(
                    "AVAssetWriter threw while appending a decoded sample from {}: {error}",
                    source_path.display()
                ))
            })? {
                return Err(SpeakerAnalysisError::Analysis(format!(
                    "AVAssetWriter failed while appending decoded audio from {} (status: {:?}, error: {})",
                    source_path.display(),
                    writer.status(),
                    writer
                        .error()
                        .map(|error| error.to_string())
                        .unwrap_or_else(|| "unknown".to_string())
                )));
            }
        }

        input.mark_as_finished();
        writer.finish_writing();
        reader.cancel_reading();

        if writer.status() != av::asset::writer::Status::Completed || !wav_path.is_file() {
            return Err(SpeakerAnalysisError::Analysis(format!(
                "AVAssetReader WAV fallback did not complete for {} (status: {:?}, error: {})",
                source_path.display(),
                writer.status(),
                writer
                    .error()
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            )));
        }

        Ok(())
    })
}

#[cfg(all(target_os = "macos", any(feature = "sherpa-onnx", feature = "speakrs")))]
fn append_downmixed_f32(
    out: &mut Vec<f32>,
    buffer: &cidre::av::AudioPcmBuf,
    channels: usize,
    frame_len: usize,
) -> SpeakerAnalysisResult<()> {
    if buffer.stride() > 1 {
        let data = buffer.data_f32_at(0).ok_or_else(|| {
            SpeakerAnalysisError::Analysis("AVFoundation returned no f32 audio data".to_string())
        })?;
        for frame in data.chunks(buffer.stride()).take(frame_len) {
            let sum: f32 = frame.iter().take(channels).copied().sum();
            out.push(sum / channels as f32);
        }
        return Ok(());
    }

    let first = buffer.data_f32_at(0).ok_or_else(|| {
        SpeakerAnalysisError::Analysis("AVFoundation returned no f32 audio data".to_string())
    })?;
    for frame_index in 0..frame_len {
        let mut sum = first.get(frame_index).copied().unwrap_or_default();
        for channel in 1..channels {
            if let Some(samples) = buffer.data_f32_at(channel) {
                sum += samples.get(frame_index).copied().unwrap_or_default();
            }
        }
        out.push(sum / channels as f32);
    }
    Ok(())
}

#[cfg(any(test, all(target_os = "macos", any(feature = "sherpa-onnx", feature = "speakrs"))))]
pub(crate) fn resample_linear(
    samples: &[f32],
    source_rate_hz: u32,
    target_rate_hz: u32,
) -> Vec<f32> {
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

    fn sample_decoded_audio() -> DecodedAudio {
        DecodedAudio {
            samples: vec![0.1, -0.1],
            sample_rate_hz: 44_100,
        }
    }

    #[test]
    fn decode_fallback_uses_primary_success() {
        let decoded = decode_with_fallback(
            || Ok(sample_decoded_audio()),
            || Err(SpeakerAnalysisError::Analysis("fallback".to_string())),
            "fallback",
        )
        .expect("primary result should win");

        assert_eq!(decoded.sample_rate_hz, 44_100);
    }

    #[test]
    fn decode_fallback_uses_secondary_when_primary_fails() {
        let decoded = decode_with_fallback(
            || Err(SpeakerAnalysisError::Analysis("primary failed".to_string())),
            || Ok(sample_decoded_audio()),
            "fallback",
        )
        .expect("fallback result should be returned");

        assert_eq!(decoded.samples.len(), 2);
    }

    #[test]
    fn decode_fallback_reports_both_errors() {
        let error = decode_with_fallback::<DecodedAudio, _, _>(
            || Err(SpeakerAnalysisError::Analysis("primary failed".to_string())),
            || {
                Err(SpeakerAnalysisError::Analysis(
                    "fallback failed".to_string(),
                ))
            },
            "AVAssetReader WAV fallback",
        )
        .expect_err("both paths should fail");

        assert!(error.to_string().contains("primary failed"));
        assert!(error.to_string().contains("fallback failed"));
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
    fn writer_input_ready_wait_times_out() {
        assert!(!wait_for_writer_input_ready(
            || false,
            Duration::ZERO,
            Duration::ZERO,
        ));
    }

    #[test]
    fn writer_input_ready_wait_returns_when_ready() {
        let mut attempts = 0;

        assert!(wait_for_writer_input_ready(
            || {
                attempts += 1;
                attempts == 3
            },
            Duration::from_secs(1),
            Duration::ZERO,
        ));
        assert_eq!(attempts, 3);
    }
}
