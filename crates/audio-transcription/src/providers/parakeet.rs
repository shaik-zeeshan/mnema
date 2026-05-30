use async_trait::async_trait;
#[cfg(feature = "parakeet-onnx")]
use ndarray::{s, ArrayD, Ix1, Ix3};
#[cfg(feature = "parakeet-onnx")]
use ort::{
    session::Session,
    value::{DynTensor, Tensor, TensorElementType, ValueType},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
#[cfg(feature = "parakeet-onnx")]
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

#[cfg(all(feature = "parakeet-onnx", target_os = "macos"))]
use crate::macos_audio_decode::decode_audio_to_mono_with_avassetreader_fallback;
#[cfg(any(test, feature = "parakeet-onnx"))]
use crate::macos_audio_decode::resample_linear;
use crate::{
    model_install_dir, TranscriptionError, TranscriptionOutput, TranscriptionProvider,
    TranscriptionRequest, TranscriptionResult, PARAKEET_PROVIDER_ID,
};
#[cfg(feature = "parakeet-onnx")]
use crate::{TranscriptionMetadata, TranscriptionSegment};

pub const PARAKEET_TDT_0_6B_V3_ONNX_MODEL_ID: &str = "parakeet-tdt-0.6b-v3-onnx";
pub const PARAKEET_TDT_0_6B_V3_ONNX_INT8_MODEL_ID: &str = "parakeet-tdt-0.6b-v3-onnx-int8";
const CONFIG_FILE_NAME: &str = "config.json";
const PREPROCESSOR_FILE_NAME: &str = "nemo128.onnx";
const ENCODER_FILE_NAME: &str = "encoder-model.onnx";
const ENCODER_DATA_FILE_NAME: &str = "encoder-model.onnx.data";
const ENCODER_INT8_FILE_NAME: &str = "encoder-model.int8.onnx";
const DECODER_JOINT_FILE_NAME: &str = "decoder_joint-model.onnx";
const DECODER_JOINT_INT8_FILE_NAME: &str = "decoder_joint-model.int8.onnx";
const VOCAB_FILE_NAME: &str = "vocab.txt";
const MODEL_PATH_OPTION: &str = "modelPath";
#[cfg(feature = "parakeet-onnx")]
const MEMORY_MODE_OPTION: &str = "parakeetOnnxMemoryMode";
#[cfg(feature = "parakeet-onnx")]
const MEMORY_MODE_FALLBACK_OPTION: &str = "memoryMode";
#[cfg(feature = "parakeet-onnx")]
const IDLE_UNLOAD_SECONDS_OPTION: &str = "parakeetOnnxIdleUnloadSeconds";
#[cfg(feature = "parakeet-onnx")]
const CHUNK_SECONDS_OPTION: &str = "parakeetOnnxChunkSeconds";
#[cfg(feature = "parakeet-onnx")]
const PARAKEET_SAMPLE_RATE_HZ: u32 = 16_000;
#[cfg(feature = "parakeet-onnx")]
const DEFAULT_MAX_TOKENS_PER_STEP: usize = 10;
#[cfg(feature = "parakeet-onnx")]
const DEFAULT_SUBSAMPLING_FACTOR: u64 = 8;
#[cfg(feature = "parakeet-onnx")]
const DEFAULT_IDLE_UNLOAD_SECONDS: u64 = 300;
#[cfg(feature = "parakeet-onnx")]
const DEFAULT_CHUNK_SECONDS: u64 = 30;

#[derive(Debug, Default, Clone, Copy)]
pub struct ParakeetProvider;

#[derive(Debug, Clone)]
pub struct ConfiguredParakeetProvider {
    models_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ParakeetAvailability {
    pub available: bool,
    pub status: ParakeetAvailabilityStatus,
    pub message: String,
    pub model_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ParakeetAvailabilityStatus {
    Available,
    ModelMissing,
    RuntimeUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParakeetModelSelection {
    pub model_id: String,
    pub model_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParakeetOnnxBundleLayout {
    pub config: PathBuf,
    pub preprocessor: PathBuf,
    pub encoder: PathBuf,
    pub encoder_data: Option<PathBuf>,
    pub decoder_joint: PathBuf,
    pub vocab: PathBuf,
}

impl ParakeetProvider {
    pub fn with_models_dir(models_dir: impl Into<PathBuf>) -> ConfiguredParakeetProvider {
        ConfiguredParakeetProvider {
            models_dir: models_dir.into(),
        }
    }

    pub fn model_path_in_store(
        models_dir: impl AsRef<Path>,
        model_id: &str,
    ) -> TranscriptionResult<PathBuf> {
        if !is_supported_parakeet_model_id(model_id) {
            return Err(TranscriptionError::InvalidRequest(format!(
                "unsupported Parakeet model id {model_id}"
            )));
        }
        model_install_dir(models_dir, PARAKEET_PROVIDER_ID, model_id)
            .map_err(|error| TranscriptionError::InvalidRequest(error.to_string()))
    }

    pub fn model_path_option_key() -> &'static str {
        MODEL_PATH_OPTION
    }

    pub fn expected_bundle_files() -> &'static [&'static str] {
        Self::expected_bundle_files_for_model(PARAKEET_TDT_0_6B_V3_ONNX_MODEL_ID)
    }

    pub fn expected_bundle_files_for_model(model_id: &str) -> &'static [&'static str] {
        if model_id == PARAKEET_TDT_0_6B_V3_ONNX_INT8_MODEL_ID {
            &[
                CONFIG_FILE_NAME,
                PREPROCESSOR_FILE_NAME,
                ENCODER_INT8_FILE_NAME,
                DECODER_JOINT_INT8_FILE_NAME,
                VOCAB_FILE_NAME,
            ]
        } else {
            &[
                CONFIG_FILE_NAME,
                PREPROCESSOR_FILE_NAME,
                ENCODER_FILE_NAME,
                ENCODER_DATA_FILE_NAME,
                DECODER_JOINT_FILE_NAME,
                VOCAB_FILE_NAME,
            ]
        }
    }

    pub fn bundle_layout(
        model_path: impl AsRef<Path>,
    ) -> TranscriptionResult<ParakeetOnnxBundleLayout> {
        Self::bundle_layout_for_model(model_path, PARAKEET_TDT_0_6B_V3_ONNX_MODEL_ID)
    }

    pub fn bundle_layout_for_model(
        model_path: impl AsRef<Path>,
        model_id: &str,
    ) -> TranscriptionResult<ParakeetOnnxBundleLayout> {
        let model_path = model_path.as_ref();
        let bundle_dir = if model_path.is_dir() {
            model_path.to_path_buf()
        } else {
            model_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| model_path.to_path_buf())
        };
        let layout = ParakeetOnnxBundleLayout {
            config: bundle_dir.join(CONFIG_FILE_NAME),
            preprocessor: bundle_dir.join(PREPROCESSOR_FILE_NAME),
            encoder: bundle_dir.join(if model_id == PARAKEET_TDT_0_6B_V3_ONNX_INT8_MODEL_ID {
                ENCODER_INT8_FILE_NAME
            } else {
                ENCODER_FILE_NAME
            }),
            encoder_data: (model_id != PARAKEET_TDT_0_6B_V3_ONNX_INT8_MODEL_ID)
                .then(|| bundle_dir.join(ENCODER_DATA_FILE_NAME)),
            decoder_joint: bundle_dir.join(
                if model_id == PARAKEET_TDT_0_6B_V3_ONNX_INT8_MODEL_ID {
                    DECODER_JOINT_INT8_FILE_NAME
                } else {
                    DECODER_JOINT_FILE_NAME
                },
            ),
            vocab: bundle_dir.join(VOCAB_FILE_NAME),
        };
        let missing = layout.missing_files();
        if !missing.is_empty() {
            return Err(TranscriptionError::ProviderUnavailable(format!(
                "Parakeet ONNX bundle is incomplete; missing files: {}",
                missing.join(", ")
            )));
        }
        Ok(layout)
    }

    pub fn availability_for_model_path(model_path: impl AsRef<Path>) -> ParakeetAvailability {
        Self::availability_for_model_path_and_id(model_path, PARAKEET_TDT_0_6B_V3_ONNX_MODEL_ID)
    }

    pub fn availability_for_model_path_and_id(
        model_path: impl AsRef<Path>,
        model_id: &str,
    ) -> ParakeetAvailability {
        let model_path = model_path.as_ref().to_path_buf();
        match Self::bundle_layout_for_model(&model_path, model_id) {
            Ok(_) => runtime_availability(model_path),
            Err(error) => ParakeetAvailability {
                available: false,
                status: ParakeetAvailabilityStatus::ModelMissing,
                message: error.to_string(),
                model_path: Some(model_path),
            },
        }
    }
}

impl ParakeetOnnxBundleLayout {
    #[cfg(feature = "parakeet-onnx")]
    fn bundle_dir(&self) -> PathBuf {
        self.config
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    }

    fn missing_files(&self) -> Vec<String> {
        let encoder_name = self
            .encoder
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(ENCODER_FILE_NAME);
        let decoder_name = self
            .decoder_joint
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(DECODER_JOINT_FILE_NAME);
        let mut required = vec![
            (CONFIG_FILE_NAME, &self.config),
            (PREPROCESSOR_FILE_NAME, &self.preprocessor),
            (encoder_name, &self.encoder),
            (decoder_name, &self.decoder_joint),
            (VOCAB_FILE_NAME, &self.vocab),
        ];
        if let Some(encoder_data) = &self.encoder_data {
            required.push((ENCODER_DATA_FILE_NAME, encoder_data));
        }
        required
            .into_iter()
            .filter_map(|(name, path)| (!path.is_file()).then(|| name.to_string()))
            .collect()
    }
}

impl ConfiguredParakeetProvider {
    pub fn models_dir(&self) -> &Path {
        &self.models_dir
    }

    pub fn availability_for_model(&self, model_id: &str) -> ParakeetAvailability {
        match ParakeetProvider::model_path_in_store(&self.models_dir, model_id) {
            Ok(path) => ParakeetProvider::availability_for_model_path_and_id(path, model_id),
            Err(error) => ParakeetAvailability {
                available: false,
                status: ParakeetAvailabilityStatus::ModelMissing,
                message: error.to_string(),
                model_path: None,
            },
        }
    }
}

#[async_trait]
impl TranscriptionProvider for ParakeetProvider {
    fn provider(&self) -> &'static str {
        PARAKEET_PROVIDER_ID
    }

    async fn transcribe(
        &self,
        request: TranscriptionRequest,
    ) -> TranscriptionResult<TranscriptionOutput> {
        transcribe_with_model_resolver(request, |_| None).await
    }
}

#[async_trait]
impl TranscriptionProvider for ConfiguredParakeetProvider {
    fn provider(&self) -> &'static str {
        PARAKEET_PROVIDER_ID
    }

    async fn transcribe(
        &self,
        request: TranscriptionRequest,
    ) -> TranscriptionResult<TranscriptionOutput> {
        let models_dir = self.models_dir.clone();
        transcribe_with_model_resolver(request, move |model_id| {
            ParakeetProvider::model_path_in_store(&models_dir, model_id).ok()
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
    let layout =
        ParakeetProvider::bundle_layout_for_model(&selection.model_path, &selection.model_id)?;

    #[cfg(feature = "parakeet-onnx")]
    {
        tokio::task::spawn_blocking(move || run_parakeet_onnx_blocking(request, selection, layout))
            .await
            .map_err(|error| {
                TranscriptionError::Transcription(format!(
                    "Parakeet ONNX worker failed to join: {error}"
                ))
            })?
    }

    #[cfg(not(feature = "parakeet-onnx"))]
    {
        let _ = (request, selection, layout);
        Err(TranscriptionError::ProviderUnavailable(
            "Parakeet ONNX runtime is not enabled in this build".to_string(),
        ))
    }
}

fn resolve_model_selection<F>(
    request: &TranscriptionRequest,
    model_path_for_id: F,
) -> TranscriptionResult<ParakeetModelSelection>
where
    F: FnOnce(&str) -> Option<PathBuf>,
{
    let model_id = validate_request(request)?.to_string();
    let model_path = request
        .options
        .get(MODEL_PATH_OPTION)
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .or_else(|| model_path_for_id(&model_id))
        .ok_or_else(|| {
            TranscriptionError::ProviderUnavailable(
                "Parakeet needs either a configured models directory or a modelPath request option"
                    .to_string(),
            )
        })?;

    Ok(ParakeetModelSelection {
        model_id,
        model_path,
    })
}

fn is_supported_parakeet_model_id(model_id: &str) -> bool {
    matches!(
        model_id,
        PARAKEET_TDT_0_6B_V3_ONNX_MODEL_ID | PARAKEET_TDT_0_6B_V3_ONNX_INT8_MODEL_ID
    )
}

fn validate_request(request: &TranscriptionRequest) -> TranscriptionResult<&str> {
    if request.provider != PARAKEET_PROVIDER_ID {
        return Err(TranscriptionError::InvalidRequest(format!(
            "Parakeet provider received request for {}",
            request.provider
        )));
    }
    let model_id = request.model_id.as_deref().ok_or_else(|| {
        TranscriptionError::InvalidRequest("Parakeet requires a model id".to_string())
    })?;
    if !is_supported_parakeet_model_id(model_id) {
        return Err(TranscriptionError::InvalidRequest(format!(
            "unsupported Parakeet model id {model_id}"
        )));
    }
    Ok(model_id)
}

#[cfg(feature = "parakeet-onnx")]
fn runtime_availability(model_path: PathBuf) -> ParakeetAvailability {
    ParakeetAvailability {
        available: true,
        status: ParakeetAvailabilityStatus::Available,
        message: "Parakeet ONNX runtime is available".to_string(),
        model_path: Some(model_path),
    }
}

#[cfg(not(feature = "parakeet-onnx"))]
fn runtime_availability(model_path: PathBuf) -> ParakeetAvailability {
    ParakeetAvailability {
        available: false,
        status: ParakeetAvailabilityStatus::RuntimeUnavailable,
        message: "Parakeet ONNX runtime is not enabled in this build".to_string(),
        model_path: Some(model_path),
    }
}

#[cfg(feature = "parakeet-onnx")]
#[derive(Debug)]
struct ParakeetOnnxRuntime {
    preprocessor: Mutex<Session>,
    encoder: Mutex<Session>,
    decoder_joint: Mutex<Session>,
    vocab: Vec<String>,
    blank_idx: usize,
    max_tokens_per_step: usize,
    subsampling_factor: u64,
    bundle_dir: PathBuf,
    encoder_execution_provider: &'static str,
}

#[cfg(feature = "parakeet-onnx")]
fn run_parakeet_onnx_blocking(
    request: TranscriptionRequest,
    selection: ParakeetModelSelection,
    layout: ParakeetOnnxBundleLayout,
) -> TranscriptionResult<TranscriptionOutput> {
    let runtime_options = ParakeetOnnxRuntimeOptions::from_request_options(&request.options)?;
    let samples = decode_audio_to_mono_16khz(&request)?;
    let runtime = cached_onnx_runtime(layout, runtime_options.memory)?;
    let decoded = runtime.transcribe_samples(&samples, runtime_options.chunk_seconds)?;
    release_cached_runtime_after_use(&runtime.bundle_dir, runtime_options.memory);

    let mut metadata = TranscriptionMetadata::from_request(&request);
    metadata.provenance.insert(
        "modelPath".to_string(),
        Value::String(runtime.bundle_dir.display().to_string()),
    );
    metadata.provenance.insert(
        "runtime".to_string(),
        Value::String("onnxruntime".to_string()),
    );
    metadata.provenance.insert(
        "sampleRateHz".to_string(),
        Value::Number(PARAKEET_SAMPLE_RATE_HZ.into()),
    );
    metadata.provenance.insert(
        "memoryMode".to_string(),
        Value::String(runtime_options.memory.mode_label().to_string()),
    );
    metadata.provenance.insert(
        "encoderExecutionProvider".to_string(),
        Value::String(runtime.encoder_execution_provider.to_string()),
    );
    metadata.provenance.insert(
        "chunkSeconds".to_string(),
        Value::Number(runtime_options.chunk_seconds.into()),
    );
    if !decoded.text.is_empty() {
        metadata.segments.push(TranscriptionSegment {
            start_ms: decoded.start_ms.unwrap_or(0),
            end_ms: decoded.end_ms.unwrap_or(0),
            text: decoded.text.clone(),
            confidence: None,
        });
    }

    Ok(TranscriptionOutput::new(decoded.text, metadata)
        .with_provider_version(parakeet_provider_version(&selection.model_id)))
}

#[cfg(any(feature = "parakeet-onnx", test))]
fn parakeet_provider_version(model_id: &str) -> String {
    format!("onnxruntime/{model_id}")
}

#[cfg(feature = "parakeet-onnx")]
#[derive(Debug, Clone)]
struct DecodedParakeetText {
    text: String,
    start_ms: Option<u64>,
    end_ms: Option<u64>,
}

#[cfg(feature = "parakeet-onnx")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParakeetOnnxMemoryMode {
    Performance,
    Balanced { idle_unload: Duration },
    LowMemory,
}

#[cfg(feature = "parakeet-onnx")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ParakeetOnnxRuntimeOptions {
    memory: ParakeetOnnxMemoryMode,
    chunk_seconds: u64,
}

#[cfg(feature = "parakeet-onnx")]
impl ParakeetOnnxMemoryMode {
    fn mode_label(self) -> &'static str {
        match self {
            Self::Performance => "performance",
            Self::Balanced { .. } => "balanced",
            Self::LowMemory => "low_memory",
        }
    }

    fn idle_unload(self) -> Option<Duration> {
        match self {
            Self::Performance => None,
            Self::Balanced { idle_unload } => Some(idle_unload),
            Self::LowMemory => Some(Duration::ZERO),
        }
    }
}

#[cfg(feature = "parakeet-onnx")]
impl ParakeetOnnxRuntimeOptions {
    fn from_request_options(
        options: &BTreeMap<String, Value>,
    ) -> TranscriptionResult<ParakeetOnnxRuntimeOptions> {
        let mode = string_option(options, MEMORY_MODE_OPTION)
            .or_else(|| string_option(options, MEMORY_MODE_FALLBACK_OPTION))
            .or_else(|| std::env::var("MNEMA_PARAKEET_ONNX_MEMORY_MODE").ok())
            .unwrap_or_else(|| "balanced".to_string());
        let idle_unload_seconds = u64_option(options, IDLE_UNLOAD_SECONDS_OPTION)
            .or_else(|| env_u64("MNEMA_PARAKEET_ONNX_IDLE_UNLOAD_SECONDS"))
            .unwrap_or(DEFAULT_IDLE_UNLOAD_SECONDS);
        let memory = parse_memory_mode(&mode, idle_unload_seconds)?;
        let chunk_seconds = u64_option(options, CHUNK_SECONDS_OPTION)
            .or_else(|| env_u64("MNEMA_PARAKEET_ONNX_CHUNK_SECONDS"))
            .unwrap_or(DEFAULT_CHUNK_SECONDS);
        Ok(Self {
            memory,
            chunk_seconds,
        })
    }
}

#[cfg(feature = "parakeet-onnx")]
fn cpu_onnx_session(model_path: &Path) -> TranscriptionResult<Session> {
    Session::builder()
        .map_err(ort_error)?
        .commit_from_file(model_path)
        .map_err(ort_error)
}

#[cfg(feature = "parakeet-onnx")]
fn encoder_onnx_session(
    layout: &ParakeetOnnxBundleLayout,
) -> TranscriptionResult<(Session, &'static str)> {
    Ok((cpu_onnx_session(&layout.encoder)?, "cpu"))
}

#[cfg(feature = "parakeet-onnx")]
impl ParakeetOnnxRuntime {
    fn load(layout: ParakeetOnnxBundleLayout) -> TranscriptionResult<Self> {
        let config = read_parakeet_config(&layout.config)?;
        let preprocessor = cpu_onnx_session(&layout.preprocessor)?;
        let (encoder, encoder_execution_provider) = encoder_onnx_session(&layout)?;
        let decoder_joint = cpu_onnx_session(&layout.decoder_joint)?;
        let vocab = read_vocab(&layout.vocab)?;
        let blank_idx = vocab
            .iter()
            .position(|token| token == "<blk>")
            .unwrap_or_else(|| vocab.len().saturating_sub(1));

        Ok(Self {
            preprocessor: Mutex::new(preprocessor),
            encoder: Mutex::new(encoder),
            decoder_joint: Mutex::new(decoder_joint),
            vocab,
            blank_idx,
            max_tokens_per_step: config
                .max_tokens_per_step
                .unwrap_or(DEFAULT_MAX_TOKENS_PER_STEP),
            subsampling_factor: config
                .subsampling_factor
                .unwrap_or(DEFAULT_SUBSAMPLING_FACTOR),
            bundle_dir: layout.bundle_dir(),
            encoder_execution_provider,
        })
    }

    fn transcribe_samples(
        &self,
        samples: &[f32],
        chunk_seconds: u64,
    ) -> TranscriptionResult<DecodedParakeetText> {
        if samples.is_empty() {
            return Ok(DecodedParakeetText {
                text: String::new(),
                start_ms: None,
                end_ms: None,
            });
        }

        let chunk_samples = chunk_sample_count(chunk_seconds);
        if chunk_samples == 0 || samples.len() <= chunk_samples {
            return self.transcribe_sample_chunk(samples);
        }

        let mut text_parts = Vec::new();
        let mut start_ms = None;
        let mut end_ms = None;
        for (chunk_index, chunk) in samples.chunks(chunk_samples).enumerate() {
            let mut decoded = self.transcribe_sample_chunk(chunk)?;
            if decoded.text.is_empty() {
                continue;
            }
            let offset_ms = samples_to_ms(chunk_index.saturating_mul(chunk_samples));
            decoded.start_ms = decoded
                .start_ms
                .map(|value| value.saturating_add(offset_ms));
            decoded.end_ms = decoded.end_ms.map(|value| value.saturating_add(offset_ms));
            start_ms.get_or_insert(decoded.start_ms.unwrap_or(offset_ms));
            end_ms = decoded.end_ms.or(end_ms);
            text_parts.push(decoded.text);
        }

        Ok(DecodedParakeetText {
            text: join_text_parts(&text_parts),
            start_ms,
            end_ms,
        })
    }

    fn transcribe_sample_chunk(&self, samples: &[f32]) -> TranscriptionResult<DecodedParakeetText> {
        let (features, feature_lens) = self.preprocess(samples)?;
        let (encoder_out, encoder_lens) = self.encode(features, feature_lens)?;
        self.decode(encoder_out, encoder_lens)
    }

    fn preprocess(&self, samples: &[f32]) -> TranscriptionResult<(ArrayD<f32>, ArrayD<i64>)> {
        let waveform = Tensor::from_array((
            vec![1_i64, samples.len() as i64],
            samples.to_vec().into_boxed_slice(),
        ))
        .map_err(ort_error)?;
        let mut session = self.preprocessor.lock().map_err(|_| {
            TranscriptionError::Transcription(
                "Parakeet preprocessor session is poisoned".to_string(),
            )
        })?;
        let waveform_lens = int_tensor_for_input(
            &session,
            "waveforms_lens",
            vec![1_i64],
            &[samples.len() as i64],
        )?;
        let outputs = session
            .run(ort::inputs![
                "waveforms" => waveform,
                "waveforms_lens" => waveform_lens,
            ])
            .map_err(ort_error)?;
        Ok((
            extract_f32_array(&outputs, "features")?,
            extract_int_array_as_i64(&outputs, "features_lens")?,
        ))
    }

    fn encode(
        &self,
        features: ArrayD<f32>,
        feature_lens: ArrayD<i64>,
    ) -> TranscriptionResult<(ArrayD<f32>, ArrayD<i64>)> {
        let feature_shape: Vec<i64> = features.shape().iter().map(|dim| *dim as i64).collect();
        let lens_shape: Vec<i64> = feature_lens.shape().iter().map(|dim| *dim as i64).collect();
        let feature_values = features
            .iter()
            .copied()
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let lens_values = feature_lens.iter().copied().collect::<Vec<_>>();
        let features_tensor =
            Tensor::from_array((feature_shape, feature_values)).map_err(ort_error)?;
        let mut session = self.encoder.lock().map_err(|_| {
            TranscriptionError::Transcription("Parakeet encoder session is poisoned".to_string())
        })?;
        let lens_tensor = int_tensor_for_input(&session, "length", lens_shape, &lens_values)?;
        let outputs = session
            .run(ort::inputs![
                "audio_signal" => features_tensor,
                "length" => lens_tensor,
            ])
            .map_err(ort_error)?;
        Ok((
            extract_f32_array(&outputs, "outputs")?,
            extract_int_array_as_i64(&outputs, "encoded_lengths")?,
        ))
    }

    fn decode(
        &self,
        encoder_out: ArrayD<f32>,
        encoder_lens: ArrayD<i64>,
    ) -> TranscriptionResult<DecodedParakeetText> {
        let encoder = encoder_out.into_dimensionality::<Ix3>().map_err(|error| {
            TranscriptionError::Transcription(format!(
                "Parakeet encoder output must be rank 3: {error}"
            ))
        })?;
        let lens = encoder_lens.into_dimensionality::<Ix1>().map_err(|error| {
            TranscriptionError::Transcription(format!(
                "Parakeet encoded lengths must be rank 1: {error}"
            ))
        })?;
        let encoded_len = usize::try_from(lens[0].max(0)).unwrap_or_default();
        let time_axis = encoder.shape()[2];
        let hidden_size = encoder.shape()[1];
        let encoded_len = encoded_len.min(time_axis);

        let mut decoder = self.decoder_joint.lock().map_err(|_| {
            TranscriptionError::Transcription("Parakeet decoder session is poisoned".to_string())
        })?;
        let (mut state1, mut state2) = initial_decoder_states(&decoder)?;
        let state1_shape: Vec<i64> = state1.shape().iter().map(|dim| *dim as i64).collect();
        let state2_shape: Vec<i64> = state2.shape().iter().map(|dim| *dim as i64).collect();

        let mut tokens = Vec::new();
        let mut timestamps = Vec::new();
        let mut t = 0_usize;
        let mut emitted_tokens = 0_usize;
        while t < encoded_len {
            let encoder_frame = encoder.slice(s![0, .., t]).to_owned();
            let encoder_tensor = Tensor::from_array((
                vec![1_i64, hidden_size as i64, 1_i64],
                encoder_frame
                    .iter()
                    .copied()
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            ))
            .map_err(ort_error)?;
            let previous_token = tokens.last().copied().unwrap_or(self.blank_idx) as i64;
            let target_tensor =
                int_tensor_for_input(&decoder, "targets", vec![1_i64, 1_i64], &[previous_token])?;
            let target_length = int_tensor_for_input(&decoder, "target_length", vec![1_i64], &[1])?;
            let state1_tensor = Tensor::from_array((
                state1_shape.clone(),
                state1
                    .iter()
                    .copied()
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            ))
            .map_err(ort_error)?;
            let state2_tensor = Tensor::from_array((
                state2_shape.clone(),
                state2
                    .iter()
                    .copied()
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            ))
            .map_err(ort_error)?;

            let outputs = decoder
                .run(ort::inputs![
                    "encoder_outputs" => encoder_tensor,
                    "targets" => target_tensor,
                    "target_length" => target_length,
                    "input_states_1" => state1_tensor,
                    "input_states_2" => state2_tensor,
                ])
                .map_err(ort_error)?;
            let output = extract_f32_array(&outputs, "outputs")?;
            let output_values: Vec<f32> = output.iter().copied().collect();
            let vocab_logits = output_values.get(..self.vocab.len()).ok_or_else(|| {
                TranscriptionError::Transcription(
                    "Parakeet decoder output is smaller than vocab".to_string(),
                )
            })?;
            let token = argmax(vocab_logits);
            let step = output_values
                .get(self.vocab.len()..)
                .map(argmax)
                .unwrap_or_default();
            let next_state1 = extract_f32_array(&outputs, "output_states_1")?;
            let next_state2 = extract_f32_array(&outputs, "output_states_2")?;

            if token != self.blank_idx {
                state1 = next_state1;
                state2 = next_state2;
                tokens.push(token);
                timestamps.push(t);
                emitted_tokens += 1;
            }

            if step > 0 {
                t = t.saturating_add(step);
                emitted_tokens = 0;
            } else if token == self.blank_idx || emitted_tokens == self.max_tokens_per_step {
                t = t.saturating_add(1);
                emitted_tokens = 0;
            }
        }

        let text = decode_tokens(&self.vocab, &tokens);
        let start_ms = timestamps
            .first()
            .map(|index| timestamp_ms(*index, self.subsampling_factor));
        let end_ms = timestamps
            .last()
            .map(|index| timestamp_ms(index.saturating_add(1), self.subsampling_factor));
        Ok(DecodedParakeetText {
            text,
            start_ms,
            end_ms,
        })
    }
}

#[cfg(feature = "parakeet-onnx")]
#[derive(Debug)]
struct CachedParakeetRuntime {
    runtime: Arc<ParakeetOnnxRuntime>,
    last_used: Instant,
    unload_worker_running: bool,
}

#[cfg(feature = "parakeet-onnx")]
type ParakeetRuntimeCache = Arc<Mutex<HashMap<PathBuf, CachedParakeetRuntime>>>;

#[cfg(feature = "parakeet-onnx")]
fn parakeet_runtime_cache() -> &'static ParakeetRuntimeCache {
    static CACHE: OnceLock<ParakeetRuntimeCache> = OnceLock::new();
    CACHE.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

#[cfg(feature = "parakeet-onnx")]
fn cached_onnx_runtime(
    layout: ParakeetOnnxBundleLayout,
    memory_mode: ParakeetOnnxMemoryMode,
) -> TranscriptionResult<Arc<ParakeetOnnxRuntime>> {
    let bundle_dir = layout.bundle_dir();
    let now = Instant::now();
    let cache = parakeet_runtime_cache();
    let mut cache_guard = cache.lock().map_err(|_| {
        TranscriptionError::Transcription("Parakeet ONNX runtime cache is poisoned".to_string())
    })?;
    if let Some(entry) = cache_guard.get_mut(&bundle_dir) {
        entry.last_used = now;
        return Ok(Arc::clone(&entry.runtime));
    }
    let runtime = Arc::new(ParakeetOnnxRuntime::load(layout)?);
    cache_guard.insert(
        bundle_dir,
        CachedParakeetRuntime {
            runtime: Arc::clone(&runtime),
            last_used: now,
            unload_worker_running: false,
        },
    );
    drop(cache_guard);
    if let Some(idle_unload) = memory_mode.idle_unload() {
        schedule_cached_runtime_unload(runtime.bundle_dir.clone(), idle_unload);
    }
    Ok(runtime)
}

#[cfg(feature = "parakeet-onnx")]
fn release_cached_runtime_after_use(bundle_dir: &Path, memory_mode: ParakeetOnnxMemoryMode) {
    if matches!(memory_mode, ParakeetOnnxMemoryMode::LowMemory) {
        if let Ok(mut cache) = parakeet_runtime_cache().lock() {
            cache.remove(bundle_dir);
        }
    } else if let Some(idle_unload) = memory_mode.idle_unload() {
        touch_cached_runtime(bundle_dir, Instant::now());
        schedule_cached_runtime_unload(bundle_dir.to_path_buf(), idle_unload);
    }
}

#[cfg(feature = "parakeet-onnx")]
fn touch_cached_runtime(bundle_dir: &Path, used_at: Instant) {
    if let Ok(mut cache) = parakeet_runtime_cache().lock() {
        if let Some(entry) = cache.get_mut(bundle_dir) {
            entry.last_used = used_at;
        }
    }
}

#[cfg(feature = "parakeet-onnx")]
fn schedule_cached_runtime_unload(bundle_dir: PathBuf, idle_unload: Duration) {
    if idle_unload.is_zero() {
        if let Ok(mut cache) = parakeet_runtime_cache().lock() {
            cache.remove(&bundle_dir);
        }
        return;
    }

    let should_spawn = {
        let Ok(mut cache) = parakeet_runtime_cache().lock() else {
            return;
        };
        let Some(entry) = cache.get_mut(&bundle_dir) else {
            return;
        };
        if entry.unload_worker_running {
            false
        } else {
            entry.unload_worker_running = true;
            true
        }
    };
    if !should_spawn {
        return;
    }

    let cache = Arc::clone(parakeet_runtime_cache());
    if std::thread::Builder::new()
        .name("parakeet-onnx-idle-unload".to_string())
        .spawn({
            let bundle_dir = bundle_dir.clone();
            move || {
                loop {
                    enum NextStep {
                        Sleep(Duration),
                        Exit,
                    }

                    let next = {
                        let Ok(mut cache) = cache.lock() else {
                            return;
                        };
                        let Some(entry) = cache.get_mut(&bundle_dir) else {
                            return;
                        };
                        let idle_elapsed = entry.last_used.elapsed();
                        if idle_elapsed < idle_unload {
                            NextStep::Sleep(idle_unload - idle_elapsed)
                        } else if Arc::strong_count(&entry.runtime) == 1 {
                            cache.remove(&bundle_dir);
                            NextStep::Exit
                        } else {
                            NextStep::Sleep(idle_unload)
                        }
                    };

                    match next {
                        NextStep::Sleep(duration) => std::thread::sleep(duration),
                        NextStep::Exit => break,
                    }
                }

                if let Ok(mut cache) = cache.lock() {
                    if let Some(entry) = cache.get_mut(&bundle_dir) {
                        entry.unload_worker_running = false;
                    }
                }
            }
        })
        .is_err()
    {
        if let Ok(mut cache) = parakeet_runtime_cache().lock() {
            if let Some(entry) = cache.get_mut(&bundle_dir) {
                entry.unload_worker_running = false;
            }
        }
    }
}

#[cfg(feature = "parakeet-onnx")]
fn parse_memory_mode(
    mode: &str,
    idle_unload_seconds: u64,
) -> TranscriptionResult<ParakeetOnnxMemoryMode> {
    match mode.trim().to_ascii_lowercase().as_str() {
        "performance" | "cache" | "keep_loaded" | "keep-loaded" => Ok(ParakeetOnnxMemoryMode::Performance),
        "balanced" | "idle_unload" | "idle-unload" => Ok(ParakeetOnnxMemoryMode::Balanced {
            idle_unload: Duration::from_secs(idle_unload_seconds),
        }),
        "low_memory" | "low-memory" | "low" | "unload" | "unload_after_use" | "unload-after-use" => {
            Ok(ParakeetOnnxMemoryMode::LowMemory)
        }
        other => Err(TranscriptionError::InvalidRequest(format!(
            "unsupported Parakeet ONNX memory mode {other}; expected balanced, performance, or low_memory"
        ))),
    }
}

#[cfg(feature = "parakeet-onnx")]
fn string_option(options: &BTreeMap<String, Value>, key: &str) -> Option<String> {
    options.get(key).and_then(Value::as_str).map(str::to_string)
}

#[cfg(feature = "parakeet-onnx")]
fn u64_option(options: &BTreeMap<String, Value>, key: &str) -> Option<u64> {
    options.get(key).and_then(|value| match value {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.parse::<u64>().ok(),
        _ => None,
    })
}

#[cfg(feature = "parakeet-onnx")]
fn env_u64(key: &str) -> Option<u64> {
    std::env::var(key).ok()?.parse::<u64>().ok()
}

#[cfg(feature = "parakeet-onnx")]
fn chunk_sample_count(chunk_seconds: u64) -> usize {
    chunk_seconds
        .checked_mul(PARAKEET_SAMPLE_RATE_HZ as u64)
        .and_then(|samples| usize::try_from(samples).ok())
        .unwrap_or(usize::MAX)
}

#[cfg(feature = "parakeet-onnx")]
fn samples_to_ms(samples: usize) -> u64 {
    ((samples as u128) * 1_000 / (PARAKEET_SAMPLE_RATE_HZ as u128)) as u64
}

#[cfg(feature = "parakeet-onnx")]
fn join_text_parts(parts: &[String]) -> String {
    parts
        .iter()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(feature = "parakeet-onnx")]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct ParakeetConfig {
    #[serde(default)]
    subsampling_factor: Option<u64>,
    #[serde(default)]
    max_tokens_per_step: Option<usize>,
}

#[cfg(feature = "parakeet-onnx")]
fn read_parakeet_config(path: &Path) -> TranscriptionResult<ParakeetConfig> {
    let bytes = std::fs::read(path).map_err(|error| {
        TranscriptionError::ProviderUnavailable(format!(
            "failed to read Parakeet config {}: {error}",
            path.display()
        ))
    })?;
    serde_json::from_slice(&bytes).map_err(|error| {
        TranscriptionError::ProviderUnavailable(format!(
            "failed to parse Parakeet config {}: {error}",
            path.display()
        ))
    })
}

#[cfg(feature = "parakeet-onnx")]
fn read_vocab(path: &Path) -> TranscriptionResult<Vec<String>> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        TranscriptionError::ProviderUnavailable(format!(
            "failed to read Parakeet vocab {}: {error}",
            path.display()
        ))
    })?;
    let mut entries = Vec::<(usize, String)>::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let Some((token, id)) = line.rsplit_once(' ') else {
            return Err(TranscriptionError::ProviderUnavailable(format!(
                "invalid Parakeet vocab line: {line}"
            )));
        };
        let id = id.parse::<usize>().map_err(|error| {
            TranscriptionError::ProviderUnavailable(format!(
                "invalid Parakeet vocab id in line '{line}': {error}"
            ))
        })?;
        entries.push((id, token.replace('▁', " ")));
    }
    entries.sort_by_key(|(id, _)| *id);
    Ok(entries.into_iter().map(|(_, token)| token).collect())
}

#[cfg(feature = "parakeet-onnx")]
fn initial_decoder_states(decoder: &Session) -> TranscriptionResult<(ArrayD<f32>, ArrayD<f32>)> {
    Ok((
        zero_state_for_input(decoder, "input_states_1")?,
        zero_state_for_input(decoder, "input_states_2")?,
    ))
}

#[cfg(feature = "parakeet-onnx")]
fn zero_state_for_input(decoder: &Session, name: &str) -> TranscriptionResult<ArrayD<f32>> {
    let input = decoder
        .inputs()
        .iter()
        .find(|input| input.name() == name)
        .ok_or_else(|| {
            TranscriptionError::ProviderUnavailable(format!(
                "Parakeet decoder is missing input {name}"
            ))
        })?;
    let ort::value::ValueType::Tensor { shape, .. } = input.dtype() else {
        return Err(TranscriptionError::ProviderUnavailable(format!(
            "Parakeet decoder input {name} is not a tensor"
        )));
    };
    let shape = shape
        .iter()
        .map(|dim| if *dim < 0 { 1 } else { *dim as usize })
        .collect::<Vec<_>>();
    Ok(ArrayD::zeros(shape))
}

#[cfg(feature = "parakeet-onnx")]
fn int_tensor_for_input(
    session: &Session,
    input_name: &str,
    shape: Vec<i64>,
    values: &[i64],
) -> TranscriptionResult<DynTensor> {
    match tensor_type_for_input(session, input_name)? {
        TensorElementType::Int32 => {
            let values = values
                .iter()
                .map(|value| {
                    i32::try_from(*value).map_err(|_| {
                        TranscriptionError::Transcription(format!(
                            "Parakeet ONNX input {input_name} value {value} does not fit int32"
                        ))
                    })
                })
                .collect::<TranscriptionResult<Vec<_>>>()?
                .into_boxed_slice();
            Tensor::from_array((shape, values))
                .map(Tensor::upcast)
                .map_err(ort_error)
        }
        TensorElementType::Int64 => Tensor::from_array((shape, values.to_vec().into_boxed_slice()))
            .map(Tensor::upcast)
            .map_err(ort_error),
        other => Err(TranscriptionError::ProviderUnavailable(format!(
            "Parakeet ONNX input {input_name} expects unsupported integer type {other}"
        ))),
    }
}

#[cfg(feature = "parakeet-onnx")]
fn tensor_type_for_input(
    session: &Session,
    input_name: &str,
) -> TranscriptionResult<TensorElementType> {
    let input = session
        .inputs()
        .iter()
        .find(|input| input.name() == input_name)
        .ok_or_else(|| {
            TranscriptionError::ProviderUnavailable(format!(
                "Parakeet ONNX session is missing input {input_name}"
            ))
        })?;
    let ValueType::Tensor { ty, .. } = input.dtype() else {
        return Err(TranscriptionError::ProviderUnavailable(format!(
            "Parakeet ONNX input {input_name} is not a tensor"
        )));
    };
    Ok(*ty)
}

#[cfg(feature = "parakeet-onnx")]
fn extract_f32_array(
    outputs: &ort::session::SessionOutputs<'_>,
    name: &str,
) -> TranscriptionResult<ArrayD<f32>> {
    outputs
        .get(name)
        .ok_or_else(|| {
            TranscriptionError::Transcription(format!("Parakeet ONNX output {name} is missing"))
        })?
        .try_extract_array::<f32>()
        .map(|array| array.to_owned())
        .map_err(ort_error)
}

#[cfg(feature = "parakeet-onnx")]
fn extract_int_array_as_i64(
    outputs: &ort::session::SessionOutputs<'_>,
    name: &str,
) -> TranscriptionResult<ArrayD<i64>> {
    let value = outputs.get(name).ok_or_else(|| {
        TranscriptionError::Transcription(format!("Parakeet ONNX output {name} is missing"))
    })?;
    match value.dtype() {
        ValueType::Tensor {
            ty: TensorElementType::Int32,
            ..
        } => value
            .try_extract_array::<i32>()
            .map(|array| array.mapv(i64::from))
            .map_err(ort_error),
        ValueType::Tensor {
            ty: TensorElementType::Int64,
            ..
        } => value
            .try_extract_array::<i64>()
            .map(|array| array.to_owned())
            .map_err(ort_error),
        other => Err(TranscriptionError::Transcription(format!(
            "Parakeet ONNX output {name} has unsupported integer type {other:?}"
        ))),
    }
}

#[cfg(feature = "parakeet-onnx")]
fn argmax(values: &[f32]) -> usize {
    values
        .iter()
        .enumerate()
        .max_by(|(_, left), (_, right)| left.total_cmp(right))
        .map(|(index, _)| index)
        .unwrap_or_default()
}

#[cfg(feature = "parakeet-onnx")]
fn decode_tokens(vocab: &[String], tokens: &[usize]) -> String {
    let raw = tokens
        .iter()
        .filter_map(|id| vocab.get(*id))
        .filter(|token| !token.starts_with('<') || !token.ends_with('>'))
        .cloned()
        .collect::<String>();
    normalize_token_spacing(&raw)
}

#[cfg(feature = "parakeet-onnx")]
fn normalize_token_spacing(raw: &str) -> String {
    let mut out = String::new();
    let mut previous_space = false;
    for ch in raw.chars() {
        if ch.is_whitespace() {
            if !previous_space && !out.is_empty() {
                out.push(' ');
            }
            previous_space = true;
        } else {
            out.push(ch);
            previous_space = false;
        }
    }
    out.trim().to_string()
}

#[cfg(feature = "parakeet-onnx")]
fn timestamp_ms(index: usize, subsampling_factor: u64) -> u64 {
    (index as u64)
        .saturating_mul(subsampling_factor)
        .saturating_mul(10)
}

#[cfg(feature = "parakeet-onnx")]
fn ort_error(error: ort::Error) -> TranscriptionError {
    TranscriptionError::Transcription(error.to_string())
}

#[cfg(all(feature = "parakeet-onnx", target_os = "macos"))]
fn decode_audio_to_mono_16khz(request: &TranscriptionRequest) -> TranscriptionResult<Vec<f32>> {
    let source_rate = request.options.get("sampleRate").and_then(Value::as_f64);
    let decoded = avfoundation_decode_audio_to_mono(&request.audio_path, source_rate)?;
    Ok(resample_linear(
        &decoded.samples,
        decoded.sample_rate_hz,
        PARAKEET_SAMPLE_RATE_HZ,
    ))
}

#[cfg(all(feature = "parakeet-onnx", not(target_os = "macos")))]
fn decode_audio_to_mono_16khz(_request: &TranscriptionRequest) -> TranscriptionResult<Vec<f32>> {
    Err(TranscriptionError::ProviderUnavailable(
        "Parakeet ONNX audio decoding is only implemented with AVFoundation on macOS in v1"
            .to_string(),
    ))
}

#[cfg(all(feature = "parakeet-onnx", target_os = "macos"))]
fn avfoundation_decode_audio_to_mono(
    path: &Path,
    sample_rate_override: Option<f64>,
) -> TranscriptionResult<crate::macos_audio_decode::DecodedAudio> {
    decode_audio_to_mono_with_avassetreader_fallback(path, sample_rate_override)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_provider_reports_missing_model_path() {
        let provider = ParakeetProvider::with_models_dir("/tmp/models");
        let availability = provider.availability_for_model(PARAKEET_TDT_0_6B_V3_ONNX_MODEL_ID);

        assert!(!availability.available);
        assert_eq!(
            availability.status,
            ParakeetAvailabilityStatus::ModelMissing
        );
        assert_eq!(
            availability.model_path,
            Some(PathBuf::from(
                "/tmp/models/parakeet/parakeet-tdt-0.6b-v3-onnx"
            ))
        );
    }

    #[test]
    fn bundle_layout_reports_missing_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        let error = ParakeetProvider::bundle_layout(temp.path()).expect_err("incomplete bundle");
        assert!(error.to_string().contains("encoder-model.onnx"));
        assert!(error.to_string().contains("vocab.txt"));

        let error = ParakeetProvider::bundle_layout_for_model(
            temp.path(),
            PARAKEET_TDT_0_6B_V3_ONNX_INT8_MODEL_ID,
        )
        .expect_err("incomplete int8 bundle");
        assert!(error.to_string().contains("encoder-model.int8.onnx"));
        assert!(error.to_string().contains("decoder_joint-model.int8.onnx"));
    }

    #[test]
    fn rejects_unknown_model() {
        let request = TranscriptionRequest::new(
            "/tmp/audio.m4a",
            PARAKEET_PROVIDER_ID,
            Some("other".to_string()),
            "auto",
        );
        let error = validate_request(&request).expect_err("unsupported model");
        assert!(matches!(error, TranscriptionError::InvalidRequest(_)));
    }

    #[test]
    fn provider_version_uses_selected_model_id() {
        assert_eq!(
            parakeet_provider_version(PARAKEET_TDT_0_6B_V3_ONNX_MODEL_ID),
            "onnxruntime/parakeet-tdt-0.6b-v3-onnx"
        );
        assert_eq!(
            parakeet_provider_version(PARAKEET_TDT_0_6B_V3_ONNX_INT8_MODEL_ID),
            "onnxruntime/parakeet-tdt-0.6b-v3-onnx-int8"
        );
    }

    #[test]
    fn resamples_to_target_rate() {
        let samples = vec![0.0, 1.0, 0.0, -1.0];
        let out = resample_linear(&samples, 4, 2);
        assert_eq!(out.len(), 2);
        assert!((out[0] - 0.0).abs() < 0.0001);
        assert!((out[1] - 0.0).abs() < 0.0001);
    }

    #[cfg(feature = "parakeet-onnx")]
    #[test]
    fn decodes_vocab_tokens_with_spacing() {
        let vocab = vec![
            "<blk>".to_string(),
            " hello".to_string(),
            " world".to_string(),
            "<pad>".to_string(),
        ];
        assert_eq!(decode_tokens(&vocab, &[1, 2, 3]), "hello world");
    }

    #[cfg(feature = "parakeet-onnx")]
    #[test]
    fn parses_runtime_memory_options() {
        let mut options = BTreeMap::new();
        options.insert(
            MEMORY_MODE_OPTION.to_string(),
            Value::String("low-memory".to_string()),
        );
        options.insert(
            CHUNK_SECONDS_OPTION.to_string(),
            Value::Number(12_u64.into()),
        );
        let parsed = ParakeetOnnxRuntimeOptions::from_request_options(&options)
            .expect("options should parse");
        assert_eq!(parsed.memory, ParakeetOnnxMemoryMode::LowMemory);
        assert_eq!(parsed.chunk_seconds, 12);

        let mut options = BTreeMap::new();
        options.insert(
            MEMORY_MODE_FALLBACK_OPTION.to_string(),
            Value::String("performance".to_string()),
        );
        let parsed = ParakeetOnnxRuntimeOptions::from_request_options(&options)
            .expect("fallback memory option should parse");
        assert_eq!(parsed.memory, ParakeetOnnxMemoryMode::Performance);
    }

    #[cfg(feature = "parakeet-onnx")]
    #[test]
    fn chunk_helpers_respect_disabled_and_seconds() {
        assert_eq!(chunk_sample_count(0), 0);
        assert_eq!(chunk_sample_count(2), 32_000);
        assert_eq!(samples_to_ms(8_000), 500);
        assert_eq!(
            join_text_parts(&[" hello ".to_string(), "".to_string(), "world".to_string()]),
            "hello world"
        );
    }
}
