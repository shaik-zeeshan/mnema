use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{
    normalize_candidate_text, OcrBoundingBox, OcrError, OcrObservation, OcrOutput, OcrRequest,
    OcrResult, OcrStructuredPayload, DEFAULT_PADDLE_OCR_LANGUAGE, DEFAULT_PADDLE_OCR_MODEL_ID,
    PADDLE_OCR_PROVIDER_ID,
};

#[cfg(feature = "paddle-rs")]
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

#[cfg(feature = "paddle-rs")]
use image::DynamicImage;
#[cfg(feature = "paddle-rs")]
use paddle_ocr_rs::{DetOptions, OcrEngine, OcrEngineConfig, RecOptions};

const MODEL_PATH_OPTION: &str = "modelPath";
const DEFAULT_DET_MODEL_PATH: &str = "det/model.mnn";
const DEFAULT_REC_MODEL_PATH: &str = "rec/model.mnn";
const DEFAULT_CHARSET_PATH: &str = "rec/charset.txt";
const DEFAULT_DET_MAX_SIDE_LEN: u32 = 960;
const DEFAULT_DET_SCORE_THRESHOLD: f32 = 0.30;
const DEFAULT_DET_MIN_AREA: u32 = 16;
const DEFAULT_DET_BOX_BORDER: u32 = 5;
const DEFAULT_DET_MAX_BOXES: usize = 128;
const DEFAULT_REC_TARGET_HEIGHT: u32 = 48;
const DEFAULT_REC_MIN_SCORE: f32 = 0.30;
const DEFAULT_REC_BATCH_SIZE: usize = 8;

#[derive(Debug, Clone)]
pub struct PaddleOcrModelSelection {
    pub model_id: String,
    pub model_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct PaddleOcrBundleLayout {
    pub bundle_dir: PathBuf,
    pub manifest: PaddleOcrBundleManifest,
    pub det_model_path: PathBuf,
    pub rec_model_path: PathBuf,
    pub charset_path: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaddleOcrBundleManifest {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub detection: PaddleDetectionConfig,
    #[serde(default)]
    pub recognition: PaddleRecognitionConfig,
}

impl Default for PaddleOcrBundleManifest {
    fn default() -> Self {
        Self {
            version: None,
            detection: PaddleDetectionConfig::default(),
            recognition: PaddleRecognitionConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaddleDetectionConfig {
    #[serde(default = "default_det_model_path")]
    pub model_path: String,
    #[serde(default = "default_det_max_side_len")]
    pub max_side_len: u32,
    #[serde(default = "default_det_score_threshold")]
    pub score_threshold: f32,
    #[serde(default = "default_det_min_area")]
    pub min_area: u32,
    #[serde(default = "default_det_box_border")]
    pub box_border: u32,
    #[serde(default = "default_det_max_boxes")]
    pub max_boxes: usize,
}

impl Default for PaddleDetectionConfig {
    fn default() -> Self {
        Self {
            model_path: default_det_model_path(),
            max_side_len: default_det_max_side_len(),
            score_threshold: default_det_score_threshold(),
            min_area: default_det_min_area(),
            box_border: default_det_box_border(),
            max_boxes: default_det_max_boxes(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaddleRecognitionConfig {
    #[serde(default = "default_rec_model_path")]
    pub model_path: String,
    #[serde(default = "default_charset_path")]
    pub charset_path: String,
    #[serde(default = "default_rec_target_height")]
    pub target_height: u32,
    #[serde(default = "default_rec_min_score")]
    pub min_score: f32,
    #[serde(default = "default_rec_batch_size")]
    pub batch_size: usize,
}

impl Default for PaddleRecognitionConfig {
    fn default() -> Self {
        Self {
            model_path: default_rec_model_path(),
            charset_path: default_charset_path(),
            target_height: default_rec_target_height(),
            min_score: default_rec_min_score(),
            batch_size: default_rec_batch_size(),
        }
    }
}

fn default_det_model_path() -> String {
    DEFAULT_DET_MODEL_PATH.to_string()
}
fn default_rec_model_path() -> String {
    DEFAULT_REC_MODEL_PATH.to_string()
}
fn default_charset_path() -> String {
    DEFAULT_CHARSET_PATH.to_string()
}
fn default_det_max_side_len() -> u32 {
    DEFAULT_DET_MAX_SIDE_LEN
}
fn default_det_score_threshold() -> f32 {
    DEFAULT_DET_SCORE_THRESHOLD
}
fn default_det_min_area() -> u32 {
    DEFAULT_DET_MIN_AREA
}
fn default_det_box_border() -> u32 {
    DEFAULT_DET_BOX_BORDER
}
fn default_det_max_boxes() -> usize {
    DEFAULT_DET_MAX_BOXES
}
fn default_rec_target_height() -> u32 {
    DEFAULT_REC_TARGET_HEIGHT
}
fn default_rec_min_score() -> f32 {
    DEFAULT_REC_MIN_SCORE
}
fn default_rec_batch_size() -> usize {
    DEFAULT_REC_BATCH_SIZE
}

pub(crate) fn runtime_available() -> bool {
    cfg!(feature = "paddle-rs")
}

pub(crate) async fn recognize(
    configured_models_dir: Option<&Path>,
    request: OcrRequest,
) -> OcrResult<OcrOutput> {
    let selection = resolve_model_selection(&request, configured_models_dir)?;
    let layout = bundle_layout(&selection.model_path)?;

    #[cfg(feature = "paddle-rs")]
    {
        tokio::task::spawn_blocking(move || run_paddle_ocr_blocking(request, selection, layout))
            .await
            .map_err(|error| {
                OcrError::Provider(format!("PaddleOCR worker failed to join: {error}"))
            })?
    }

    #[cfg(not(feature = "paddle-rs"))]
    {
        let _ = (request, selection, layout);
        Err(OcrError::Provider(
            "PaddleOCR runtime is not enabled in this build".to_string(),
        ))
    }
}

fn resolve_model_selection(
    request: &OcrRequest,
    configured_models_dir: Option<&Path>,
) -> OcrResult<PaddleOcrModelSelection> {
    if request.provider != PADDLE_OCR_PROVIDER_ID {
        return Err(OcrError::Provider(format!(
            "PaddleOCR provider received request for {}",
            request.provider
        )));
    }

    let model_id = request
        .model_id
        .clone()
        .unwrap_or_else(|| DEFAULT_PADDLE_OCR_MODEL_ID.to_string());
    if model_id != DEFAULT_PADDLE_OCR_MODEL_ID {
        return Err(OcrError::Provider(format!(
            "unsupported PaddleOCR model id {model_id}"
        )));
    }

    let model_path = request
        .options
        .get(MODEL_PATH_OPTION)
        .and_then(serde_json::Value::as_str)
        .map(PathBuf::from)
        .or_else(|| {
            configured_models_dir.map(|dir| dir.join(PADDLE_OCR_PROVIDER_ID).join(&model_id))
        })
        .ok_or_else(|| {
            OcrError::Provider(
                "PaddleOCR needs either a configured models directory or a modelPath request option"
                    .to_string(),
            )
        })?;

    Ok(PaddleOcrModelSelection {
        model_id,
        model_path,
    })
}

fn bundle_layout(model_path: &Path) -> OcrResult<PaddleOcrBundleLayout> {
    let bundle_dir = if model_path.is_dir() {
        model_path.to_path_buf()
    } else {
        model_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| model_path.to_path_buf())
    };
    let manifest_path = bundle_dir.join("manifest.json");
    let manifest = if manifest_path.is_file() {
        serde_json::from_slice(&std::fs::read(&manifest_path).map_err(|error| {
            OcrError::Provider(format!(
                "failed to read PaddleOCR bundle manifest {}: {error}",
                manifest_path.display()
            ))
        })?)
        .map_err(|error| {
            OcrError::Provider(format!(
                "failed to parse PaddleOCR bundle manifest {}: {error}",
                manifest_path.display()
            ))
        })?
    } else {
        PaddleOcrBundleManifest::default()
    };

    let det_model_path = bundle_dir.join(&manifest.detection.model_path);
    let rec_model_path = bundle_dir.join(&manifest.recognition.model_path);
    let charset_path = bundle_dir.join(&manifest.recognition.charset_path);
    for required in [&det_model_path, &rec_model_path, &charset_path] {
        if !required.is_file() {
            return Err(OcrError::Provider(format!(
                "PaddleOCR bundle file is missing at {}",
                required.display()
            )));
        }
    }

    Ok(PaddleOcrBundleLayout {
        bundle_dir,
        manifest,
        det_model_path,
        rec_model_path,
        charset_path,
    })
}

#[cfg(feature = "paddle-rs")]
struct PaddleOcrRuntime {
    engine: Mutex<OcrEngine>,
    layout: PaddleOcrBundleLayout,
}

#[cfg(feature = "paddle-rs")]
type PaddleRuntimeCache = Mutex<HashMap<PathBuf, Arc<PaddleOcrRuntime>>>;

#[cfg(feature = "paddle-rs")]
fn paddle_runtime_cache() -> &'static PaddleRuntimeCache {
    static CACHE: OnceLock<PaddleRuntimeCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(feature = "paddle-rs")]
fn cached_runtime(layout: PaddleOcrBundleLayout) -> OcrResult<Arc<PaddleOcrRuntime>> {
    let cache_key = layout.bundle_dir.clone();
    let mut cache = paddle_runtime_cache()
        .lock()
        .map_err(|_| OcrError::Provider("PaddleOCR runtime cache is poisoned".to_string()))?;
    if let Some(runtime) = cache.get(&cache_key) {
        return Ok(runtime.clone());
    }
    let runtime = Arc::new(PaddleOcrRuntime::load(layout)?);
    cache.insert(cache_key, runtime.clone());
    Ok(runtime)
}

#[cfg(feature = "paddle-rs")]
impl PaddleOcrRuntime {
    fn load(layout: PaddleOcrBundleLayout) -> OcrResult<Self> {
        let config = paddle_engine_config(&layout.manifest);
        let engine = OcrEngine::new(
            &layout.det_model_path,
            &layout.rec_model_path,
            &layout.charset_path,
            Some(config),
        )
        .map_err(paddle_error)?;
        Ok(Self {
            engine: Mutex::new(engine),
            layout,
        })
    }

    fn recognize_image(&self, image: &DynamicImage) -> OcrResult<Vec<OcrObservation>> {
        let engine = self
            .engine
            .lock()
            .map_err(|_| OcrError::Provider("PaddleOCR MNN engine is poisoned".to_string()))?;
        let mut results = engine.recognize(image).map_err(paddle_error)?;
        let max_boxes = self.layout.manifest.detection.max_boxes;
        if max_boxes > 0 && results.len() > max_boxes {
            results.sort_by(|left, right| {
                right
                    .confidence
                    .total_cmp(&left.confidence)
                    .then_with(|| right.bbox.area().cmp(&left.bbox.area()))
            });
            results.truncate(max_boxes);
        }
        results.sort_by_key(|result| (result.bbox.rect.top(), result.bbox.rect.left()));

        let observations = results
            .into_iter()
            .filter_map(|result| {
                let text = normalize_candidate_text(&result.text)?;
                Some(OcrObservation::new(
                    text,
                    result.confidence.clamp(0.0, 1.0),
                    normalize_box(&result.bbox, image.width(), image.height()),
                ))
            })
            .collect();
        Ok(observations)
    }
}

#[cfg(feature = "paddle-rs")]
fn paddle_engine_config(manifest: &PaddleOcrBundleManifest) -> OcrEngineConfig {
    let det = &manifest.detection;
    let rec = &manifest.recognition;
    let thread_count = paddle_thread_count();
    OcrEngineConfig::fast()
        .with_threads(thread_count)
        .with_parallel(false)
        .with_det_options(
            DetOptions::new()
                .with_max_side_len(det.max_side_len)
                .with_score_threshold(det.score_threshold)
                .with_min_area(det.min_area)
                .with_box_border(det.box_border),
        )
        .with_rec_options(
            RecOptions::new()
                .with_target_height(rec.target_height)
                .with_min_score(rec.min_score)
                .with_batch_size(rec.batch_size.max(1))
                .with_batch(true),
        )
}

#[cfg(feature = "paddle-rs")]
fn paddle_thread_count() -> i32 {
    std::env::var("MNEMA_PADDLE_OCR_THREADS")
        .ok()
        .and_then(|value| value.trim().parse::<i32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(2)
}

#[cfg(feature = "paddle-rs")]
fn run_paddle_ocr_blocking(
    request: OcrRequest,
    selection: PaddleOcrModelSelection,
    layout: PaddleOcrBundleLayout,
) -> OcrResult<OcrOutput> {
    if !request.image_path.is_file() {
        return Err(OcrError::Provider(format!(
            "image file does not exist: {}",
            request.image_path.display()
        )));
    }
    let _language = request
        .language
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_PADDLE_OCR_LANGUAGE);
    let image = image::open(&request.image_path).map_err(|error| {
        OcrError::Provider(format!(
            "failed to open OCR image {}: {error}",
            request.image_path.display()
        ))
    })?;
    let runtime = cached_runtime(layout.clone())?;
    let observations = runtime.recognize_image(&image)?;
    let text = observations
        .iter()
        .map(|observation| observation.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let structured_payload = OcrStructuredPayload::new(
        PADDLE_OCR_PROVIDER_ID,
        Some(selection.model_id.clone()),
        observations,
    );
    let version = layout
        .manifest
        .version
        .clone()
        .unwrap_or_else(|| "paddle-ocr-rs-mnn".to_string());
    Ok(OcrOutput::new(text, structured_payload).with_provider_version(version))
}

#[cfg(feature = "paddle-rs")]
fn paddle_error(error: impl std::fmt::Display) -> OcrError {
    OcrError::Provider(error.to_string())
}

#[cfg(feature = "paddle-rs")]
fn normalize_box(
    bbox: &paddle_ocr_rs::postprocess::TextBox,
    image_width: u32,
    image_height: u32,
) -> OcrBoundingBox {
    let image_width = image_width.max(1) as f64;
    let image_height = image_height.max(1) as f64;
    let left = bbox.rect.left().max(0) as f64;
    let top = bbox.rect.top().max(0) as f64;
    let width = bbox.rect.width() as f64;
    let height = bbox.rect.height() as f64;
    OcrBoundingBox::new(
        left / image_width,
        1.0 - ((top + height) / image_height),
        width / image_width,
        height / image_height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paddle_manifest_defaults_to_mnn_layout() {
        let manifest = PaddleOcrBundleManifest::default();
        assert_eq!(manifest.detection.model_path, "det/model.mnn");
        assert_eq!(manifest.recognition.model_path, "rec/model.mnn");
        assert_eq!(manifest.recognition.charset_path, "rec/charset.txt");
    }
}
