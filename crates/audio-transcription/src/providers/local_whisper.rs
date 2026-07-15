use async_trait::async_trait;
use serde_json::Value;
#[cfg(feature = "local-whisper")]
use std::collections::HashMap;
use std::path::{Path, PathBuf};
#[cfg(feature = "local-whisper")]
use std::sync::{Arc, Mutex, OnceLock};

#[cfg(all(feature = "local-whisper", target_os = "macos"))]
use crate::macos_audio_decode::decode_audio_to_mono_with_avassetreader_fallback;
#[cfg(any(test, feature = "local-whisper"))]
use crate::macos_audio_decode::resample_linear;
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
#[cfg(feature = "local-whisper")]
type WhisperContextCache = Mutex<HashMap<PathBuf, Arc<whisper_rs::WhisperContext>>>;

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
    // A segment that decodes to no audio (e.g. a silent capture) is a successful
    // no-speech job, not a failure: whisper.cpp's `full()` rejects an empty input
    // buffer, so short-circuit to an empty transcription before invoking it.
    if samples.is_empty() {
        let metadata = build_whisper_metadata(&request, &selection);
        return Ok(TranscriptionOutput::no_speech(metadata));
    }
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
        // `set_detect_language(true)` looks equivalent to `set_language(None)` in
        // whisper-rs docs, but against real Mnema microphone captures it causes
        // whisper.cpp to auto-detect the language and then emit zero segments.
        // Leaving language unset still auto-detects correctly and returns text.
        params.set_language(None);
    } else {
        params.set_language(Some(&request.language));
    }

    state
        .full(params, &samples)
        .map_err(|error| TranscriptionError::Transcription(error.to_string()))?;

    let mut metadata = build_whisper_metadata(&request, &selection);

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
fn whisper_context_cache() -> &'static WhisperContextCache {
    static CACHE: OnceLock<WhisperContextCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(feature = "local-whisper")]
pub fn unload_all_cached_contexts() -> TranscriptionResult<usize> {
    let mut cache = whisper_context_cache().lock().map_err(|_| {
        TranscriptionError::Transcription("local Whisper model cache is poisoned".to_string())
    })?;
    let unloaded = cache.len();
    cache.clear();
    Ok(unloaded)
}

#[cfg(not(feature = "local-whisper"))]
pub fn unload_all_cached_contexts() -> TranscriptionResult<usize> {
    Ok(0)
}

#[cfg(feature = "local-whisper")]
fn cached_whisper_context(
    model_path: &Path,
) -> TranscriptionResult<Arc<whisper_rs::WhisperContext>> {
    let model_path = model_path.to_path_buf();
    let mut cache = whisper_context_cache().lock().map_err(|_| {
        TranscriptionError::Transcription("local Whisper model cache is poisoned".to_string())
    })?;
    if let Some(context) = cache.get(&model_path) {
        return Ok(context.clone());
    }

    // CPU-only everywhere except macOS, which opts into Metal. No whisper-rs GPU
    // feature (CUDA/Vulkan) is enabled on Windows in v1, so the params stay
    // immutable there.
    #[cfg(target_os = "macos")]
    let params = {
        let mut params = whisper_rs::WhisperContextParameters::new();
        params.use_gpu(true);
        params
    };
    #[cfg(not(target_os = "macos"))]
    let params = whisper_rs::WhisperContextParameters::new();

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

/// Build the provenance metadata shared by the transcribed and no-speech paths.
///
/// Keeping this in one place ensures a no-speech (empty-audio) job emits the same
/// `modelPath`/`whisperSampleRateHz` provenance as a job that produced text, so
/// downstream span/search plumbing sees a consistent payload regardless of
/// whether whisper.cpp actually ran.
#[cfg(feature = "local-whisper")]
fn build_whisper_metadata(
    request: &TranscriptionRequest,
    selection: &LocalWhisperModelSelection,
) -> TranscriptionMetadata {
    let mut metadata = TranscriptionMetadata::from_request(request);
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
    metadata
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

#[cfg(all(feature = "local-whisper", target_os = "windows"))]
fn decode_audio_to_mono_16khz(request: &TranscriptionRequest) -> TranscriptionResult<Vec<f32>> {
    // Windows routes through the shared `media-decode` MF Source Reader seam
    // (ADR 0024) rather than AVFoundation, then resamples the native-rate mono
    // output to Whisper's 16 kHz with the same in-crate resampler macOS uses.
    let decoded = media_decode::decode_to_mono_f32(&request.audio_path)
        .map_err(|error| TranscriptionError::Transcription(error.to_string()))?;
    Ok(resample_linear(
        &decoded.samples,
        decoded.sample_rate_hz,
        WHISPER_SAMPLE_RATE_HZ,
    ))
}

#[cfg(all(
    feature = "local-whisper",
    not(target_os = "macos"),
    not(target_os = "windows")
))]
fn decode_audio_to_mono_16khz(_request: &TranscriptionRequest) -> TranscriptionResult<Vec<f32>> {
    Err(TranscriptionError::ProviderUnavailable(
        "local Whisper audio decoding is only implemented with AVFoundation on macOS and the \
         media-decode seam on Windows in v1"
            .to_string(),
    ))
}

#[cfg(all(feature = "local-whisper", target_os = "macos"))]
fn avfoundation_decode_audio_to_mono(
    path: &Path,
    sample_rate_override: Option<f64>,
) -> TranscriptionResult<crate::macos_audio_decode::DecodedAudio> {
    decode_audio_to_mono_with_avassetreader_fallback(path, sample_rate_override)
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

    // No-speech-is-success: a segment that decodes to no audio must yield an
    // empty (but successful) transcription whose provenance still records the
    // model and Whisper sample rate, never a failed job.
    #[cfg(feature = "local-whisper")]
    #[test]
    fn empty_decode_yields_successful_no_speech_output() {
        let request = TranscriptionRequest::new(
            "/tmp/audio.m4a",
            LOCAL_WHISPER_PROVIDER_ID,
            Some("base".to_string()),
            "auto",
        );
        let selection = LocalWhisperModelSelection {
            model_id: "base".to_string(),
            model_path: PathBuf::from("/tmp/models/local_whisper/base/ggml-base.bin"),
        };

        // Mirror the empty-samples short-circuit in `run_whisper_blocking`.
        let metadata = build_whisper_metadata(&request, &selection);
        let output = TranscriptionOutput::no_speech(metadata);

        assert!(output.text.is_empty());
        assert!(output.metadata.segments.is_empty());
        assert_eq!(
            output.metadata.provenance.get("whisperSampleRateHz"),
            Some(&serde_json::Value::Number(WHISPER_SAMPLE_RATE_HZ.into()))
        );
        assert_eq!(
            output.metadata.provenance.get("modelPath"),
            Some(&serde_json::Value::String(
                selection.model_path.display().to_string()
            ))
        );
    }

    // Exercises the Windows decode->resample wiring end to end through the real
    // `media-decode` MF Source Reader seam: a synthesized 8 kHz mono PCM WAV
    // (MF reads WAV natively, no AAC fixture needed) decodes to native-rate mono
    // and resamples to Whisper's 16 kHz. Decoding a captured `.m4a` on real
    // hardware is the operator-deferred gap.
    #[cfg(all(feature = "local-whisper", target_os = "windows"))]
    #[test]
    fn windows_decode_resamples_to_16khz_through_seam() {
        let source_rate_hz = 8_000u32;
        // A short non-empty ramp so the resampled output is observably non-empty.
        let pcm_i16: Vec<i16> = (0..16).map(|i| (i * 1000) as i16).collect();
        let wav = build_mono_pcm16_wav(source_rate_hz, &pcm_i16);

        let path = std::env::temp_dir().join(format!(
            "local-whisper-decode-test-{}.wav",
            std::process::id()
        ));
        std::fs::write(&path, &wav).expect("write temp wav");

        let request = TranscriptionRequest::new(
            path.to_string_lossy().to_string(),
            LOCAL_WHISPER_PROVIDER_ID,
            Some("base".to_string()),
            "auto",
        );
        let samples = decode_audio_to_mono_16khz(&request);
        let _ = std::fs::remove_file(&path);
        let samples = samples.expect("Windows seam should decode and resample the WAV");

        assert!(!samples.is_empty(), "resampled 16 kHz output must be non-empty");
        // Upsampling 8 kHz -> 16 kHz roughly doubles the sample count.
        assert!(
            samples.len() >= pcm_i16.len(),
            "expected at least as many samples after upsampling, got {}",
            samples.len()
        );
        assert!(samples.iter().all(|s| (-1.0..=1.0).contains(s)));
    }

    /// Build a minimal canonical 44-byte-header mono 16-bit PCM WAV.
    #[cfg(all(feature = "local-whisper", target_os = "windows"))]
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
