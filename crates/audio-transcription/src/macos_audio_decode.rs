#[cfg(any(
    test,
    all(
        target_os = "macos",
        any(feature = "local-whisper", feature = "parakeet-onnx")
    )
))]
use crate::{TranscriptionError, TranscriptionResult};

#[cfg(all(
    target_os = "macos",
    any(feature = "local-whisper", feature = "parakeet-onnx")
))]
use std::path::Path;

#[cfg(all(
    target_os = "macos",
    any(feature = "local-whisper", feature = "parakeet-onnx")
))]
use tempfile::NamedTempFile;

#[cfg(any(
    test,
    all(
        target_os = "macos",
        any(feature = "local-whisper", feature = "parakeet-onnx")
    )
))]
use std::time::{Duration, Instant};

/// Upper bound on waveform buckets: the scrubber is a few hundred pixels wide,
/// so anything past this is wasted work no caller can render.
pub const MAX_WAVEFORM_BUCKETS: u32 = 2_000;

/// Streaming `max(|sample|)`-per-bucket reducer for waveform scrubbers.
///
/// Buckets by sample index over the file's declared frame count, so it only
/// ever holds `bucket_count` floats regardless of audio length.
#[allow(dead_code)]
#[derive(Debug)]
pub(crate) struct PeakReducer {
    peaks: Vec<f32>,
    total_samples: u64,
    seen: u64,
}

#[allow(dead_code)]
impl PeakReducer {
    pub(crate) fn new(total_samples: u64, bucket_count: u32) -> Self {
        let buckets = bucket_count.clamp(1, MAX_WAVEFORM_BUCKETS) as usize;
        Self {
            peaks: vec![0.0; buckets],
            // ponytail: trailing buckets stay at 0 if the file reads short of
            // its declared frame count; not worth a second pass to trim.
            total_samples: total_samples.max(1),
            seen: 0,
        }
    }

    pub(crate) fn push(&mut self, samples: &[f32]) {
        let buckets = self.peaks.len() as u64;
        for sample in samples {
            let index = self
                .seen
                .saturating_mul(buckets)
                .checked_div(self.total_samples)
                .unwrap_or(0)
                .min(buckets - 1) as usize;
            let magnitude = sample.abs();
            if magnitude.is_finite() && magnitude > self.peaks[index] {
                self.peaks[index] = magnitude;
            }
            self.seen += 1;
        }
    }

    /// Peak-normalized 0.0..=1.0 buckets, or empty if no samples arrived.
    pub(crate) fn finish(self) -> Vec<f32> {
        if self.seen == 0 {
            return Vec::new();
        }
        let loudest = self.peaks.iter().copied().fold(0.0_f32, f32::max);
        if loudest <= 0.0 {
            return vec![0.0; self.peaks.len()];
        }
        self.peaks
            .into_iter()
            .map(|peak| (peak / loudest).clamp(0.0, 1.0))
            .collect()
    }
}

/// Amplitude peaks for a waveform scrubber: one `max(|sample|)` per bucket,
/// normalized to 0.0..=1.0. Any failure (missing file, unsupported/undecodable
/// audio, silence-free zero-length input) yields an empty `Vec` so callers can
/// degrade to a plain scrub bar.
///
/// ponytail: recomputed per request, no cache — a 5-minute segment decodes in
/// well under a second. Add a cache only if it ever measurably drags.
/// ponytail: primary AVAudioFile path only; the AVAssetReader temp-WAV
/// fallback used by transcription is skipped because transcoding a whole
/// segment just to draw a waveform is not worth it. Ceiling: exotic files that
/// only decode via the fallback render as a plain scrub bar.
pub fn audio_waveform_peaks(path: &std::path::Path, bucket_count: u32) -> Vec<f32> {
    #[cfg(all(
        target_os = "macos",
        any(feature = "local-whisper", feature = "parakeet-onnx")
    ))]
    let peaks = {
        let buckets = bucket_count.clamp(1, MAX_WAVEFORM_BUCKETS);
        let mut reducer: Option<PeakReducer> = None;
        match avaudiofile_decode_mono_streaming(path, None, |chunk, total_frames| {
            reducer
                .get_or_insert_with(|| PeakReducer::new(total_frames, buckets))
                .push(chunk);
        }) {
            Ok(_) => reducer.map(PeakReducer::finish).unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    };

    #[cfg(not(all(
        target_os = "macos",
        any(feature = "local-whisper", feature = "parakeet-onnx")
    )))]
    let peaks = {
        let _ = (path, bucket_count);
        Vec::new()
    };

    peaks
}

#[cfg(any(
    test,
    all(
        target_os = "macos",
        any(feature = "local-whisper", feature = "parakeet-onnx")
    )
))]
#[derive(Debug)]
pub(crate) struct DecodedAudio {
    pub(crate) samples: Vec<f32>,
    pub(crate) sample_rate_hz: u32,
}

#[cfg(all(
    target_os = "macos",
    any(feature = "local-whisper", feature = "parakeet-onnx")
))]
const AVASSETREADER_WRITER_READY_TIMEOUT: Duration = Duration::from_secs(30);

#[cfg(all(
    target_os = "macos",
    any(feature = "local-whisper", feature = "parakeet-onnx")
))]
const AVASSETREADER_WRITER_READY_POLL_INTERVAL: Duration = Duration::from_millis(1);

#[cfg(any(
    test,
    all(
        target_os = "macos",
        any(feature = "local-whisper", feature = "parakeet-onnx")
    )
))]
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

#[cfg(all(
    target_os = "macos",
    any(feature = "local-whisper", feature = "parakeet-onnx")
))]
pub(crate) fn decode_audio_to_mono_with_avassetreader_fallback(
    path: &Path,
    sample_rate_override: Option<f64>,
) -> TranscriptionResult<DecodedAudio> {
    decode_with_fallback(
        || avaudiofile_decode_audio_to_mono(path, sample_rate_override),
        || avassetreader_decode_audio_to_mono(path, sample_rate_override),
        "AVAssetReader WAV fallback",
    )
}

#[cfg(any(
    test,
    all(
        target_os = "macos",
        any(feature = "local-whisper", feature = "parakeet-onnx")
    )
))]
fn decode_with_fallback<T, FPrimary, FFallback>(
    primary: FPrimary,
    fallback: FFallback,
    fallback_label: &str,
) -> TranscriptionResult<T>
where
    FPrimary: FnOnce() -> TranscriptionResult<T>,
    FFallback: FnOnce() -> TranscriptionResult<T>,
{
    match primary() {
        Ok(value) => Ok(value),
        Err(primary_error) => fallback().map_err(|fallback_error| {
            TranscriptionError::Transcription(format!(
                "{primary_error}; {fallback_label} also failed: {fallback_error}"
            ))
        }),
    }
}

#[cfg(all(
    target_os = "macos",
    any(feature = "local-whisper", feature = "parakeet-onnx")
))]
fn avaudiofile_decode_audio_to_mono(
    path: &Path,
    sample_rate_override: Option<f64>,
) -> TranscriptionResult<DecodedAudio> {
    let mut samples = Vec::new();
    let sample_rate_hz =
        avaudiofile_decode_mono_streaming(path, sample_rate_override, |chunk, _total_frames| {
            samples.extend_from_slice(chunk);
        })?;
    Ok(DecodedAudio {
        samples,
        sample_rate_hz,
    })
}

/// Decodes to mono f32 and hands each decoded chunk to `sink` along with the
/// file's total frame count, so callers that only need a reduction (waveform
/// peaks) never hold the whole file in memory.
#[cfg(all(
    target_os = "macos",
    any(feature = "local-whisper", feature = "parakeet-onnx")
))]
fn avaudiofile_decode_mono_streaming(
    path: &Path,
    sample_rate_override: Option<f64>,
    mut sink: impl FnMut(&[f32], u64),
) -> TranscriptionResult<u32> {
    use cidre::{av, ns, objc};

    let path_str = path.to_str().ok_or_else(|| {
        TranscriptionError::InvalidRequest(format!(
            "audio path is not valid UTF-8 for AVFoundation: {}",
            path.display()
        ))
    })?;

    let _pool = objc::autorelease_pool::AutoreleasePoolPage::push();
    let url = ns::Url::with_fs_path_str(path_str, false);
    let mut file =
        av::AudioFile::open_read_common_format(&url, av::AudioCommonFormat::PcmF32, false)
            .map_err(|error| {
                TranscriptionError::Transcription(format!(
                    "AVFoundation failed to open audio file {}: {error}",
                    path.display()
                ))
            })?;
    let format = file.processing_format();
    let sample_rate = sample_rate_override.unwrap_or(format.absd().sample_rate);
    if !sample_rate.is_finite() || sample_rate <= 0.0 {
        return Err(TranscriptionError::Transcription(format!(
            "AVFoundation reported invalid sample rate {sample_rate} for {}",
            path.display()
        )));
    }
    let sample_rate_hz = sample_rate.round().clamp(1.0, u32::MAX as f64) as u32;
    let channels = usize::try_from(format.channel_count()).unwrap_or(0);
    if channels == 0 {
        return Err(TranscriptionError::Transcription(format!(
            "AVFoundation reported zero channels for {}",
            path.display()
        )));
    }

    let mut chunk = Vec::new();
    let total_frames = file.len().max(0) as u64;
    let chunk_frames = 16_384_u32;
    let mut remaining = total_frames;
    while remaining > 0 {
        let frames = remaining.min(chunk_frames as u64) as u32;
        let mut buffer = av::AudioPcmBuf::with_format(&format, frames).ok_or_else(|| {
            TranscriptionError::Transcription("failed to allocate AVAudioPCMBuffer".to_string())
        })?;
        file.read_n(&mut buffer, frames).map_err(|error| {
            TranscriptionError::Transcription(format!(
                "AVFoundation failed reading audio file {}: {error}",
                path.display()
            ))
        })?;
        let frame_len = buffer.frame_len() as usize;
        if frame_len == 0 {
            break;
        }
        chunk.clear();
        append_downmixed_f32(&mut chunk, &buffer, channels, frame_len)?;
        sink(&chunk, total_frames);
        remaining = remaining.saturating_sub(frame_len as u64);
    }

    Ok(sample_rate_hz)
}

#[cfg(all(
    target_os = "macos",
    any(feature = "local-whisper", feature = "parakeet-onnx")
))]
fn avassetreader_decode_audio_to_mono(
    path: &Path,
    sample_rate_override: Option<f64>,
) -> TranscriptionResult<DecodedAudio> {
    let temp_wav = NamedTempFile::new().map_err(|error| {
        TranscriptionError::Transcription(format!(
            "failed to allocate temporary WAV for {}: {error}",
            path.display()
        ))
    })?;
    transcode_audio_to_wav_with_asset_reader(path, temp_wav.path())?;
    avaudiofile_decode_audio_to_mono(temp_wav.path(), sample_rate_override)
}

#[cfg(all(
    target_os = "macos",
    any(feature = "local-whisper", feature = "parakeet-onnx")
))]
fn transcode_audio_to_wav_with_asset_reader(
    source_path: &Path,
    wav_path: &Path,
) -> TranscriptionResult<()> {
    use cidre::{av, cat, cm, ns, objc};

    let source_path_str = source_path.to_str().ok_or_else(|| {
        TranscriptionError::InvalidRequest(format!(
            "audio path is not valid UTF-8 for AVAssetReader: {}",
            source_path.display()
        ))
    })?;
    let wav_path_str = wav_path.to_str().ok_or_else(|| {
        TranscriptionError::InvalidRequest(format!(
            "temporary WAV path is not valid UTF-8 for AVAssetReader: {}",
            wav_path.display()
        ))
    })?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| {
            TranscriptionError::Transcription(format!(
                "failed to start AVAssetReader fallback runtime for {}: {error}",
                source_path.display()
            ))
        })?;

    runtime.block_on(async {
        let _pool = objc::autorelease_pool::AutoreleasePoolPage::push();
        let source_url = ns::Url::with_fs_path_str(source_path_str, false);
        let asset = av::UrlAsset::with_url(&source_url, None).ok_or_else(|| {
            TranscriptionError::Transcription(format!(
                "AVAssetReader could not create source asset for {}",
                source_path.display()
            ))
        })?;
        let tracks = asset
            .load_tracks_with_media_type(av::MediaType::audio())
            .await
            .map_err(|error| {
                TranscriptionError::Transcription(format!(
                    "AVAssetReader failed loading audio tracks for {}: {error}",
                    source_path.display()
                ))
            })?;
        let track = tracks.get(0).map_err(|_| {
            TranscriptionError::Transcription(format!(
                "AVAssetReader found no audio tracks in {}",
                source_path.display()
            ))
        })?;

        let mut output = av::AssetReaderTrackOutput::with_track(&track, None).map_err(|error| {
            TranscriptionError::Transcription(format!(
                "AVAssetReader failed to create track output for {}: {error}",
                source_path.display()
            ))
        })?;
        output.set_always_copies_sample_data(false);

        let mut reader = av::AssetReader::with_asset(&asset).map_err(|error| {
            TranscriptionError::Transcription(format!(
                "AVAssetReader failed to initialize for {}: {error}",
                source_path.display()
            ))
        })?;
        reader.add_output(&output).map_err(|error| {
            TranscriptionError::Transcription(format!(
                "AVAssetReader failed to attach track output for {}: {error}",
                source_path.display()
            ))
        })?;

        let wav_url = ns::Url::with_fs_path_str(wav_path_str, false);
        let mut writer =
            av::AssetWriter::with_url_and_file_type(&wav_url, av::FileType::wav()).map_err(
                |error| {
                    TranscriptionError::Transcription(format!(
                        "AVAssetWriter failed to create temporary WAV for {}: {error}",
                        source_path.display()
                    ))
                },
            )?;

        if !reader.start_reading().map_err(|error| {
            TranscriptionError::Transcription(format!(
                "AVAssetReader failed to start reading {}: {error}",
                source_path.display()
            ))
        })? {
            return Err(TranscriptionError::Transcription(format!(
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
                TranscriptionError::Transcription(format!(
                    "AVAssetReader failed to read the first sample from {}: {error}",
                    source_path.display()
                ))
            })?
            .ok_or_else(|| {
                TranscriptionError::Transcription(format!(
                    "AVAssetReader decoded no audio samples from {}",
                    source_path.display()
                ))
            })?;
        let format_desc = first_buf.format_desc().ok_or_else(|| {
            TranscriptionError::Transcription(format!(
                "AVAssetReader returned the first sample without a format description for {}",
                source_path.display()
            ))
        })?;
        let source_asbd = format_desc.stream_basic_desc().ok_or_else(|| {
            TranscriptionError::Transcription(format!(
                "AVAssetReader returned the first sample without an audio stream description for {}",
                source_path.display()
            ))
        })?;
        let source_hint = cm::AudioFormatDesc::with_asbd(source_asbd).map_err(|error| {
            TranscriptionError::Transcription(format!(
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
            TranscriptionError::Transcription(format!(
                "AVAssetWriter failed to create a WAV writer input for {}: {error}",
                source_path.display()
            ))
        })?;
        writer.add_input(&input).map_err(|error| {
            TranscriptionError::Transcription(format!(
                "AVAssetWriter failed to attach a WAV writer input for {}: {error}",
                source_path.display()
            ))
        })?;

        if !writer.start_writing() {
            return Err(TranscriptionError::Transcription(format!(
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
                return Err(TranscriptionError::Transcription(format!(
                    "AVAssetWriter timed out waiting for input readiness while transcoding {} to WAV",
                    source_path.display()
                )));
            }

            let Some(buf) = current_buf.take() else {
                match output.next_sample_buf().map_err(|error| {
                    TranscriptionError::Transcription(format!(
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
                TranscriptionError::Transcription(format!(
                    "AVAssetWriter threw while appending a decoded sample from {}: {error}",
                    source_path.display()
                ))
            })? {
                return Err(TranscriptionError::Transcription(format!(
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
            return Err(TranscriptionError::Transcription(format!(
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

#[cfg(all(
    target_os = "macos",
    any(feature = "local-whisper", feature = "parakeet-onnx")
))]
fn append_downmixed_f32(
    out: &mut Vec<f32>,
    buffer: &cidre::av::AudioPcmBuf,
    channels: usize,
    frame_len: usize,
) -> TranscriptionResult<()> {
    if buffer.stride() > 1 {
        let data = buffer.data_f32_at(0).ok_or_else(|| {
            TranscriptionError::Transcription("AVFoundation returned no f32 audio data".to_string())
        })?;
        for frame in data.chunks(buffer.stride()).take(frame_len) {
            let sum: f32 = frame.iter().take(channels).copied().sum();
            out.push(sum / channels as f32);
        }
        return Ok(());
    }

    let first = buffer.data_f32_at(0).ok_or_else(|| {
        TranscriptionError::Transcription("AVFoundation returned no f32 audio data".to_string())
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

#[cfg(any(test, feature = "local-whisper", feature = "parakeet-onnx"))]
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
    fn peaks_bucket_by_sample_index_and_normalize() {
        // 8 samples into 4 buckets: pairs peak at 0.25, 0.5, 0.75, 1.0.
        let mut reducer = PeakReducer::new(8, 4);
        reducer.push(&[0.1, 0.25, -0.5, 0.4]);
        reducer.push(&[0.75, -0.3, 0.2, -1.0]);
        assert_eq!(reducer.finish(), vec![0.25, 0.5, 0.75, 1.0]);
    }

    #[test]
    fn peaks_report_silence_in_silent_buckets() {
        let mut reducer = PeakReducer::new(4, 2);
        reducer.push(&[0.0, 0.0, 0.5, -0.25]);
        assert_eq!(reducer.finish(), vec![0.0, 1.0]);
    }

    #[test]
    fn silent_and_non_finite_samples_do_not_produce_empty_or_nan_peaks() {
        // All-silent is NOT the same as "decode failed": callers distinguish an
        // empty Vec (draw a plain bar) from all-zero buckets (this really is silence).
        let mut silent = PeakReducer::new(4, 2);
        silent.push(&[0.0, 0.0, 0.0, 0.0]);
        assert_eq!(silent.finish(), vec![0.0, 0.0]);

        // NaN/Inf must neither leak into the output nor become the normalizing
        // maximum (which would flatten every real sample to ~0).
        let mut poisoned = PeakReducer::new(4, 2);
        poisoned.push(&[f32::NAN, f32::INFINITY, 0.5, 0.5]);
        let peaks = poisoned.finish();
        assert!(
            peaks.iter().all(|peak| peak.is_finite()),
            "non-finite peak leaked: {peaks:?}"
        );
        assert_eq!(peaks, vec![0.0, 1.0]);
    }

    #[test]
    fn peaks_are_empty_without_samples() {
        assert!(PeakReducer::new(0, 16).finish().is_empty());
        assert!(PeakReducer::new(1_000, 16).finish().is_empty());
    }

    #[test]
    fn peaks_clamp_bucket_count() {
        let mut reducer = PeakReducer::new(2, 0);
        reducer.push(&[0.5, 1.0]);
        assert_eq!(reducer.finish().len(), 1);

        let mut huge = PeakReducer::new(2, u32::MAX);
        huge.push(&[0.5, 1.0]);
        assert_eq!(huge.finish().len(), MAX_WAVEFORM_BUCKETS as usize);
    }

    #[test]
    fn decode_fallback_uses_primary_success() {
        let decoded = decode_with_fallback(
            || Ok(sample_decoded_audio()),
            || Err(TranscriptionError::Transcription("fallback".to_string())),
            "fallback",
        )
        .expect("primary result should win");

        assert_eq!(decoded.sample_rate_hz, 44_100);
        assert_eq!(decoded.samples.len(), 2);
    }

    #[test]
    fn decode_fallback_uses_secondary_when_primary_fails() {
        let decoded = decode_with_fallback(
            || {
                Err(TranscriptionError::Transcription(
                    "primary failed".to_string(),
                ))
            },
            || Ok(sample_decoded_audio()),
            "fallback",
        )
        .expect("fallback result should be returned");

        assert_eq!(decoded.sample_rate_hz, 44_100);
    }

    #[test]
    fn decode_fallback_reports_both_errors() {
        let error = decode_with_fallback::<DecodedAudio, _, _>(
            || {
                Err(TranscriptionError::Transcription(
                    "primary failed".to_string(),
                ))
            },
            || {
                Err(TranscriptionError::Transcription(
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

    /// Tests that decode an actual file. Gated exactly like the decoder itself:
    /// with `default = []` the production path compiles to a stub, so these would
    /// otherwise assert on `Vec::new()` and prove nothing.
    /// Run with `cargo test -p audio-transcription --features parakeet-onnx`.
    #[cfg(all(
        target_os = "macos",
        any(feature = "local-whisper", feature = "parakeet-onnx")
    ))]
    mod real_file_decode {
        use crate::macos_audio_decode::{audio_waveform_peaks, avaudiofile_decode_audio_to_mono};
        use std::io::Write;
        use tempfile::NamedTempFile;

        const SAMPLE_RATE_HZ: u32 = 16_000;
        /// Two full 16_384-frame chunks plus a partial third, so the streaming
        /// loop runs three times and any per-chunk buffer reuse bug (a missing
        /// `chunk.clear()`) shows up as duplicated audio.
        const FRAME_COUNT: usize = 40_960;

        /// Hand-rolled interleaved stereo 16-bit PCM WAV so the expected mono
        /// samples are known exactly, without pulling in an encoder.
        fn write_stereo_wav(frames: &[(i16, i16)]) -> NamedTempFile {
            let mut file = tempfile::Builder::new()
                .suffix(".wav")
                .tempfile()
                .expect("temp wav");
            let data_len = (frames.len() * 2 * 2) as u32;
            let mut bytes = Vec::with_capacity(44 + data_len as usize);
            bytes.extend_from_slice(b"RIFF");
            bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
            bytes.extend_from_slice(b"WAVEfmt ");
            bytes.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
            bytes.extend_from_slice(&1u16.to_le_bytes()); // format: PCM
            bytes.extend_from_slice(&2u16.to_le_bytes()); // channels: stereo
            bytes.extend_from_slice(&SAMPLE_RATE_HZ.to_le_bytes());
            bytes.extend_from_slice(&(SAMPLE_RATE_HZ * 2 * 2).to_le_bytes()); // byte rate
            bytes.extend_from_slice(&4u16.to_le_bytes()); // block align
            bytes.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
            bytes.extend_from_slice(b"data");
            bytes.extend_from_slice(&data_len.to_le_bytes());
            for (left, right) in frames {
                bytes.extend_from_slice(&left.to_le_bytes());
                bytes.extend_from_slice(&right.to_le_bytes());
            }
            file.write_all(&bytes).expect("write wav");
            file.flush().expect("flush wav");
            file
        }

        /// Distinct per-frame, per-channel ramp: every frame is identifiable, so
        /// a repeated or dropped chunk lands on the wrong value.
        fn ramp_frame(index: usize) -> (i16, i16) {
            let left = (index as i32 % 3_001 - 1_500) as i16;
            let right = ((index as i32 * 2) % 3_001 - 1_500) as i16;
            (left, right)
        }

        fn expected_mono(left: i16, right: i16) -> f32 {
            (left as f32 / 32_768.0 + right as f32 / 32_768.0) / 2.0
        }

        #[test]
        fn streaming_decode_yields_one_mono_sample_per_frame_across_chunks() {
            let frames: Vec<(i16, i16)> = (0..FRAME_COUNT).map(ramp_frame).collect();
            let wav = write_stereo_wav(&frames);

            let decoded = avaudiofile_decode_audio_to_mono(wav.path(), None)
                .expect("hand-written stereo WAV should decode");

            assert_eq!(decoded.sample_rate_hz, SAMPLE_RATE_HZ);
            // Duplication (missing chunk.clear()) inflates this; truncation shrinks it.
            assert_eq!(decoded.samples.len(), FRAME_COUNT);

            for index in [0usize, 16_383, 16_384, 32_768, FRAME_COUNT - 1] {
                let (left, right) = frames[index];
                let want = expected_mono(left, right);
                assert!(
                    (decoded.samples[index] - want).abs() < 1e-3,
                    "frame {index}: got {}, want {want}",
                    decoded.samples[index]
                );
            }
        }

        #[test]
        fn waveform_peaks_track_amplitude_across_chunks() {
            const BUCKETS: u32 = 8;
            let bucket_frames = FRAME_COUNT / BUCKETS as usize;
            // Burst sits well inside bucket 5, away from either bucket edge.
            let burst = (bucket_frames * 5 + 500)..(bucket_frames * 6 - 500);
            let quiet_floor = 3_000_i16;
            let frames: Vec<(i16, i16)> = (0..FRAME_COUNT)
                .map(|index| {
                    if burst.contains(&index) {
                        (i16::MAX, i16::MAX)
                    } else {
                        (quiet_floor, quiet_floor)
                    }
                })
                .collect();
            let wav = write_stereo_wav(&frames);

            let peaks = audio_waveform_peaks(wav.path(), BUCKETS);

            assert_eq!(peaks.len(), BUCKETS as usize, "{peaks:?}");
            assert!(peaks[5] > 0.99, "burst should peak in bucket 5: {peaks:?}");
            // Every other bucket carries the quiet floor: non-zero proves the
            // stream ran to the last frame, low proves the burst didn't smear.
            let want_quiet = quiet_floor as f32 / 32_767.0;
            for bucket in [0usize, 1, 2, 3, 4, 6, 7] {
                assert!(
                    (peaks[bucket] - want_quiet).abs() < 0.01,
                    "bucket {bucket} should sit at the quiet floor {want_quiet}: {peaks:?}"
                );
            }
        }

        #[test]
        fn waveform_peaks_span_the_whole_file_for_stereo_input() {
            let loud_from = FRAME_COUNT / 4 * 3;
            let frames: Vec<(i16, i16)> = (0..FRAME_COUNT)
                .map(|index| {
                    if index >= loud_from {
                        (i16::MAX, i16::MAX)
                    } else {
                        (0, 0)
                    }
                })
                .collect();
            let wav = write_stereo_wav(&frames);

            // If the reducer were sized by interleaved sample count instead of
            // frames, a stereo file's energy would land in the middle bucket.
            assert_eq!(
                audio_waveform_peaks(wav.path(), 4),
                vec![0.0, 0.0, 0.0, 1.0]
            );
        }

        #[test]
        fn peaks_for_a_missing_file_are_empty() {
            assert!(audio_waveform_peaks(std::path::Path::new("/nope/missing.m4a"), 64).is_empty());
        }

        #[test]
        fn peaks_for_an_undecodable_file_are_empty() {
            let mut file = tempfile::Builder::new()
                .suffix(".m4a")
                .tempfile()
                .expect("temp m4a");
            file.write_all(b"this is not audio, just bytes")
                .expect("write garbage");
            file.flush().expect("flush garbage");

            assert!(audio_waveform_peaks(file.path(), 64).is_empty());
        }
    }
}
