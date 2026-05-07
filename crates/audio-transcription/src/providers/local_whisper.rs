use async_trait::async_trait;
use serde_json::Value;
#[cfg(feature = "local-whisper")]
use std::collections::HashMap;
use std::path::{Path, PathBuf};
#[cfg(feature = "local-whisper")]
use std::sync::{Arc, Mutex, OnceLock};

use crate::{
    model_install_dir, TranscriptionError, TranscriptionOutput, TranscriptionProvider,
    TranscriptionRequest, TranscriptionResult, LOCAL_WHISPER_PROVIDER_ID,
};
#[cfg(feature = "local-whisper")]
use crate::{TranscriptionMetadata, TranscriptionSegment};

const MODEL_PATH_OPTION: &str = "modelPath";
#[cfg(all(feature = "local-whisper", target_os = "macos"))]
const SAMPLE_RATE_OPTION: &str = "sampleRate";
#[cfg(feature = "local-whisper")]
const WHISPER_SAMPLE_RATE_HZ: u32 = 16_000;

#[derive(Debug, Default, Clone, Copy)]
pub struct LocalWhisperProvider;

#[derive(Debug, Clone)]
pub struct ConfiguredLocalWhisperProvider {
    models_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalWhisperModelSelection {
    pub model_id: String,
    pub model_path: PathBuf,
}

impl LocalWhisperProvider {
    pub fn with_models_dir(models_dir: impl Into<PathBuf>) -> ConfiguredLocalWhisperProvider {
        ConfiguredLocalWhisperProvider {
            models_dir: models_dir.into(),
        }
    }

    pub fn model_file_name(model_id: &str) -> String {
        format!("ggml-{model_id}.bin")
    }

    pub fn model_path_in_store(
        models_dir: impl AsRef<Path>,
        model_id: &str,
    ) -> TranscriptionResult<PathBuf> {
        model_install_dir(models_dir, LOCAL_WHISPER_PROVIDER_ID, model_id)
            .map(|dir| dir.join(Self::model_file_name(model_id)))
            .map_err(|error| TranscriptionError::InvalidRequest(error.to_string()))
    }

    pub fn model_path_option_key() -> &'static str {
        MODEL_PATH_OPTION
    }
}

impl ConfiguredLocalWhisperProvider {
    pub fn models_dir(&self) -> &Path {
        &self.models_dir
    }
}

#[async_trait]
impl TranscriptionProvider for LocalWhisperProvider {
    fn provider(&self) -> &'static str {
        LOCAL_WHISPER_PROVIDER_ID
    }

    async fn transcribe(
        &self,
        request: TranscriptionRequest,
    ) -> TranscriptionResult<TranscriptionOutput> {
        transcribe_with_model_resolver(request, |_| None).await
    }
}

#[async_trait]
impl TranscriptionProvider for ConfiguredLocalWhisperProvider {
    fn provider(&self) -> &'static str {
        LOCAL_WHISPER_PROVIDER_ID
    }

    async fn transcribe(
        &self,
        request: TranscriptionRequest,
    ) -> TranscriptionResult<TranscriptionOutput> {
        let models_dir = self.models_dir.clone();
        transcribe_with_model_resolver(request, move |model_id| {
            LocalWhisperProvider::model_path_in_store(&models_dir, model_id).ok()
        })
        .await
    }
}

async fn transcribe_with_model_resolver<F>(
    request: TranscriptionRequest,
    model_path_for_id: F,
) -> TranscriptionResult<TranscriptionOutput>
where
    F: FnOnce(&str) -> Option<PathBuf> + Send + 'static,
{
    let selection = resolve_model_selection(&request, model_path_for_id)?;
    if !request.audio_path.is_file() {
        return Err(TranscriptionError::InvalidRequest(format!(
            "audio file does not exist: {}",
            request.audio_path.display()
        )));
    }
    if !selection.model_path.is_file() {
        return Err(TranscriptionError::ProviderUnavailable(format!(
            "local Whisper model is not installed at {}",
            selection.model_path.display()
        )));
    }

    #[cfg(feature = "local-whisper")]
    {
        tokio::task::spawn_blocking(move || run_whisper_blocking(request, selection))
            .await
            .map_err(|error| {
                TranscriptionError::Transcription(format!(
                    "local Whisper worker failed to join: {error}"
                ))
            })?
    }

    #[cfg(not(feature = "local-whisper"))]
    {
        let _ = (request, selection);
        Err(TranscriptionError::ProviderUnavailable(
            "local Whisper runtime is not enabled in this build".to_string(),
        ))
    }
}

fn resolve_model_selection<F>(
    request: &TranscriptionRequest,
    model_path_for_id: F,
) -> TranscriptionResult<LocalWhisperModelSelection>
where
    F: FnOnce(&str) -> Option<PathBuf>,
{
    if request.provider != LOCAL_WHISPER_PROVIDER_ID {
        return Err(TranscriptionError::InvalidRequest(format!(
            "local Whisper provider received request for {}",
            request.provider
        )));
    }

    let model_id = request.model_id.clone().ok_or_else(|| {
        TranscriptionError::InvalidRequest("local Whisper requires a model id".to_string())
    })?;

    let model_path = request
        .options
        .get(MODEL_PATH_OPTION)
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .or_else(|| model_path_for_id(&model_id))
        .ok_or_else(|| {
            TranscriptionError::ProviderUnavailable(
                "local Whisper needs either a configured models directory or a modelPath request option"
                    .to_string(),
            )
        })?;

    Ok(LocalWhisperModelSelection {
        model_id,
        model_path,
    })
}

#[cfg(feature = "local-whisper")]
fn run_whisper_blocking(
    request: TranscriptionRequest,
    selection: LocalWhisperModelSelection,
) -> TranscriptionResult<TranscriptionOutput> {
    let samples = decode_audio_to_mono_16khz(&request)?;
    let context = cached_whisper_context(&selection.model_path)?;
    let mut state = context
        .create_state()
        .map_err(|error| TranscriptionError::Transcription(error.to_string()))?;

    let mut params =
        whisper_rs::FullParams::new(whisper_rs::SamplingStrategy::Greedy { best_of: 1 });
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_token_timestamps(true);
    params.set_split_on_word(true);
    params.set_no_context(true);
    if request.language == "auto" || request.language.trim().is_empty() {
        params.set_detect_language(true);
        params.set_language(None);
    } else {
        params.set_language(Some(&request.language));
    }

    state
        .full(params, &samples)
        .map_err(|error| TranscriptionError::Transcription(error.to_string()))?;

    let mut metadata = TranscriptionMetadata::from_request(&request);
    metadata.provenance.insert(
        "modelPath".to_string(),
        serde_json::Value::String(selection.model_path.display().to_string()),
    );
    metadata.provenance.insert(
        "whisperSampleRateHz".to_string(),
        serde_json::Value::Number(WHISPER_SAMPLE_RATE_HZ.into()),
    );
    #[cfg(target_os = "macos")]
    metadata.provenance.insert(
        "metalAccelerationRequested".to_string(),
        serde_json::Value::Bool(true),
    );

    let mut text = String::new();
    for segment in state.as_iter() {
        let segment_text = segment
            .to_str_lossy()
            .map_err(|error| TranscriptionError::Transcription(error.to_string()))?
            .trim()
            .to_string();
        if !segment_text.is_empty() {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&segment_text);
        }

        metadata.segments.push(TranscriptionSegment {
            start_ms: centiseconds_to_ms(segment.start_timestamp()),
            end_ms: centiseconds_to_ms(segment.end_timestamp()),
            text: segment_text,
            confidence: Some((1.0 - segment.no_speech_probability()).clamp(0.0, 1.0)),
        });
    }

    Ok(TranscriptionOutput::new(text, metadata).with_provider_version(provider_version(&context)))
}

#[cfg(feature = "local-whisper")]
fn cached_whisper_context(
    model_path: &Path,
) -> TranscriptionResult<Arc<whisper_rs::WhisperContext>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<whisper_rs::WhisperContext>>>> =
        OnceLock::new();

    let model_path = model_path.to_path_buf();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cache.lock().map_err(|_| {
        TranscriptionError::Transcription("local Whisper model cache is poisoned".to_string())
    })?;
    if let Some(context) = cache.get(&model_path) {
        return Ok(context.clone());
    }

    let mut params = whisper_rs::WhisperContextParameters::new();
    #[cfg(target_os = "macos")]
    params.use_gpu(true);

    let context = whisper_rs::WhisperContext::new_with_params(&model_path, params)
        .map(Arc::new)
        .map_err(|error| {
            TranscriptionError::ProviderUnavailable(format!(
                "failed to load local Whisper model {}: {error}",
                model_path.display()
            ))
        })?;
    cache.insert(model_path, context.clone());
    Ok(context)
}

#[cfg(feature = "local-whisper")]
fn provider_version(context: &whisper_rs::WhisperContext) -> String {
    context
        .model_type_readable_str_lossy()
        .map(|model| format!("whisper.cpp/{model}"))
        .unwrap_or_else(|_| "whisper.cpp".to_string())
}

#[cfg(feature = "local-whisper")]
fn centiseconds_to_ms(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default().saturating_mul(10)
}

#[cfg(all(feature = "local-whisper", target_os = "macos"))]
fn decode_audio_to_mono_16khz(request: &TranscriptionRequest) -> TranscriptionResult<Vec<f32>> {
    let source_rate = request
        .options
        .get(SAMPLE_RATE_OPTION)
        .and_then(Value::as_f64);
    let decoded = avfoundation_decode_audio_to_mono(&request.audio_path, source_rate)?;
    Ok(resample_linear(
        &decoded.samples,
        decoded.sample_rate_hz,
        WHISPER_SAMPLE_RATE_HZ,
    ))
}

#[cfg(all(feature = "local-whisper", not(target_os = "macos")))]
fn decode_audio_to_mono_16khz(_request: &TranscriptionRequest) -> TranscriptionResult<Vec<f32>> {
    Err(TranscriptionError::ProviderUnavailable(
        "local Whisper audio decoding is only implemented with AVFoundation on macOS in v1"
            .to_string(),
    ))
}

#[cfg(all(feature = "local-whisper", target_os = "macos"))]
#[derive(Debug)]
struct DecodedAudio {
    samples: Vec<f32>,
    sample_rate_hz: u32,
}

#[cfg(all(feature = "local-whisper", target_os = "macos"))]
fn avfoundation_decode_audio_to_mono(
    path: &Path,
    sample_rate_override: Option<f64>,
) -> TranscriptionResult<DecodedAudio> {
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

    let mut out = Vec::new();
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
        append_downmixed_f32(&mut out, &buffer, channels, frame_len)?;
        remaining = remaining.saturating_sub(frame_len as u64);
    }

    Ok(DecodedAudio {
        samples: out,
        sample_rate_hz,
    })
}

#[cfg(all(feature = "local-whisper", target_os = "macos"))]
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

#[cfg(any(test, feature = "local-whisper"))]
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
    use serde_json::json;

    #[test]
    fn resolves_model_path_from_request_option() {
        let request = TranscriptionRequest::new(
            "/tmp/audio.m4a",
            LOCAL_WHISPER_PROVIDER_ID,
            Some("base".to_string()),
            "auto",
        )
        .with_option(MODEL_PATH_OPTION, json!("/tmp/model.bin"));

        let selection = resolve_model_selection(&request, |_| None).expect("selection");
        assert_eq!(selection.model_id, "base");
        assert_eq!(selection.model_path, PathBuf::from("/tmp/model.bin"));
    }

    #[test]
    fn configured_provider_resolves_store_model_path() {
        let request = TranscriptionRequest::new(
            "/tmp/audio.m4a",
            LOCAL_WHISPER_PROVIDER_ID,
            Some("small".to_string()),
            "auto",
        );

        let models_dir = PathBuf::from("/tmp/models");
        let selection = resolve_model_selection(&request, |model_id| {
            LocalWhisperProvider::model_path_in_store(&models_dir, model_id).ok()
        })
        .expect("selection");

        assert_eq!(
            selection.model_path,
            PathBuf::from("/tmp/models/local_whisper/small/ggml-small.bin")
        );
    }

    #[test]
    fn rejects_mismatched_provider() {
        let request = TranscriptionRequest::new(
            "/tmp/audio.m4a",
            crate::PARAKEET_PROVIDER_ID,
            Some("base".to_string()),
            "auto",
        );

        let error = resolve_model_selection(&request, |_| None).expect_err("invalid request");
        assert!(matches!(error, TranscriptionError::InvalidRequest(_)));
    }

    #[test]
    fn resamples_to_target_rate() {
        let samples = vec![0.0, 1.0, 0.0, -1.0];
        let out = resample_linear(&samples, 4, 2);
        assert_eq!(out.len(), 2);
        assert!((out[0] - 0.0).abs() < 0.0001);
        assert!((out[1] - 0.0).abs() < 0.0001);
    }
}
