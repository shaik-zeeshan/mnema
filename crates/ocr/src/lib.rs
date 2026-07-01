#![allow(unexpected_cfgs)]

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read},
    path::{Component, Path, PathBuf},
};
use thiserror::Error;

#[cfg(target_os = "macos")]
use cidre::{cg, cv, ns, objc, vn};
#[cfg(target_os = "macos")]
use image::{imageops::FilterType, DynamicImage, GenericImageView};
#[cfg(target_os = "macos")]
use std::thread_local;
#[cfg(target_os = "macos")]
use std::{cell::OnceCell, ffi::c_void};

mod paddle;
mod tesseract;

pub const MODEL_STORE_DIR_NAME: &str = "ocr-models";
pub const INSTALLED_MARKER_FILE_NAME: &str = ".installed.json";
pub const FAILED_MARKER_FILE_NAME: &str = ".failed.json";
pub const DOWNLOADING_MARKER_FILE_NAME: &str = ".download-in-progress";

pub const APPLE_VISION_PROVIDER_ID: &str = "apple_vision";
pub const TESSERACT_PROVIDER_ID: &str = "tesseract";
pub const PADDLE_OCR_PROVIDER_ID: &str = "paddle_ocr";

pub const DEFAULT_TESSERACT_MODEL_ID: &str = "tesseract-5.5.2";
pub const DEFAULT_TESSERACT_LANGUAGE: &str = "eng";
pub const DEFAULT_PADDLE_OCR_MODEL_ID: &str = "en-ppocrv5-mobile";
pub const DEFAULT_PADDLE_OCR_LANGUAGE: &str = "en";

const MANIFEST_VERSION: u32 = 1;
const OCR_SCHEMA_VERSION: u32 = 2;
const OCR_COORDINATE_SPACE: &str = "normalized";
const OCR_COORDINATE_ORIGIN: &str = "lower_left";

#[derive(Debug, Error)]
pub enum OcrError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("ocr provider error: {0}")]
    Provider(String),
}

pub type OcrResult<T> = std::result::Result<T, OcrError>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum OcrProviderKind {
    AppleVision,
    Tesseract,
    PaddleOcr,
}

impl OcrProviderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AppleVision => APPLE_VISION_PROVIDER_ID,
            Self::Tesseract => TESSERACT_PROVIDER_ID,
            Self::PaddleOcr => PADDLE_OCR_PROVIDER_ID,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::AppleVision => "Apple Vision",
            Self::Tesseract => "Tesseract",
            Self::PaddleOcr => "PaddleOCR",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OcrRecognitionMode {
    Fast,
    Accurate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OcrBoundingBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl OcrBoundingBox {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    #[cfg(target_os = "macos")]
    fn from_rect(rect: cg::Rect) -> Self {
        let rect = rect.standardized();
        Self::new(
            rect.origin.x,
            rect.origin.y,
            rect.size.width,
            rect.size.height,
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OcrObservation {
    pub text: String,
    pub confidence: f32,
    pub bounding_box: OcrBoundingBox,
}

impl OcrObservation {
    pub fn new(text: impl Into<String>, confidence: f32, bounding_box: OcrBoundingBox) -> Self {
        Self {
            text: text.into(),
            confidence,
            bounding_box,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OcrProvenance {
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OcrStructuredPayload {
    pub schema_version: u32,
    pub coordinate_space: String,
    pub coordinate_origin: String,
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    pub observations: Vec<OcrObservation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<OcrProvenance>,
}

impl OcrStructuredPayload {
    pub fn new(
        provider: impl Into<String>,
        model_id: Option<String>,
        observations: Vec<OcrObservation>,
    ) -> Self {
        let provider = provider.into();
        let provenance = Some(OcrProvenance {
            provider: provider.clone(),
            model_id: model_id.clone(),
        });
        Self {
            schema_version: OCR_SCHEMA_VERSION,
            coordinate_space: OCR_COORDINATE_SPACE.to_string(),
            coordinate_origin: OCR_COORDINATE_ORIGIN.to_string(),
            provider,
            model_id,
            observations,
            provenance,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OcrRequest {
    pub image_path: PathBuf,
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub options: BTreeMap<String, serde_json::Value>,
}

impl OcrRequest {
    pub fn new(image_path: impl Into<PathBuf>, provider: impl Into<String>) -> Self {
        Self {
            image_path: image_path.into(),
            provider: provider.into(),
            model_id: None,
            language: None,
            options: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OcrOutput {
    pub text: String,
    pub structured_payload: OcrStructuredPayload,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_version: Option<String>,
}

impl OcrOutput {
    pub fn new(text: impl Into<String>, structured_payload: OcrStructuredPayload) -> Self {
        Self {
            text: text.into(),
            structured_payload,
            provider_version: None,
        }
    }

    pub fn with_provider_version(mut self, version: impl Into<String>) -> Self {
        self.provider_version = Some(version.into());
        self
    }

    pub fn structured_payload_json(&self) -> OcrResult<String> {
        // Round geometry + confidence to 3 decimals before serialize: overlay boxes
        // sit on a ~1500px screen, so 18-digit float mantissas are pure waste. Rounding
        // also strips high-entropy mantissa noise so the downstream zstd compresses far
        // better (geometry compression, app-infra Slice 1).
        let mut payload = self.structured_payload.clone();
        for observation in &mut payload.observations {
            observation.confidence = round3_f32(observation.confidence);
            let bbox = &mut observation.bounding_box;
            bbox.x = round3_f64(bbox.x);
            bbox.y = round3_f64(bbox.y);
            bbox.width = round3_f64(bbox.width);
            bbox.height = round3_f64(bbox.height);
        }
        serde_json::to_string(&payload).map_err(Into::into)
    }
}

fn round3_f64(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

fn round3_f32(value: f32) -> f32 {
    (value * 1000.0).round() / 1000.0
}

#[async_trait]
pub trait OcrProvider: Send + Sync {
    fn provider(&self) -> &'static str;

    async fn recognize(&self, request: OcrRequest) -> OcrResult<OcrOutput>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FrozenOcrPayload {
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub options: BTreeMap<String, serde_json::Value>,
}

impl FrozenOcrPayload {
    pub fn new(provider: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model_id: None,
            language: None,
            options: BTreeMap::new(),
        }
    }

    pub fn from_payload_json(payload_json: Option<&str>) -> OcrResult<Self> {
        let Some(payload_json) = payload_json else {
            return Ok(Self::new(APPLE_VISION_PROVIDER_ID));
        };

        match serde_json::from_str::<Self>(payload_json) {
            Ok(payload) => Ok(payload),
            Err(_) => {
                let legacy: LegacyAppleVisionPayload = serde_json::from_str(payload_json)?;
                let mut payload = Self::new(APPLE_VISION_PROVIDER_ID);
                payload.language = normalize_optional_language(legacy.language);
                if let Some(mode) = legacy.recognition_mode {
                    payload
                        .options
                        .insert("recognitionMode".to_string(), serde_json::to_value(mode)?);
                }
                if let Some(language_correction) = legacy.language_correction {
                    payload.options.insert(
                        "languageCorrection".to_string(),
                        serde_json::Value::Bool(language_correction),
                    );
                }
                Ok(payload)
            }
        }
    }

    pub fn to_request(&self, image_path: impl Into<PathBuf>) -> OcrRequest {
        OcrRequest {
            image_path: image_path.into(),
            provider: self.provider.clone(),
            model_id: self.model_id.clone(),
            language: self.language.clone(),
            options: self.options.clone(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyAppleVisionPayload {
    language: Option<String>,
    recognition_mode: Option<OcrRecognitionMode>,
    language_correction: Option<bool>,
}

fn normalize_optional_language(language: Option<String>) -> Option<String> {
    language
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[derive(Debug, Default)]
pub struct AppleVisionProvider;

impl AppleVisionProvider {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl OcrProvider for AppleVisionProvider {
    fn provider(&self) -> &'static str {
        APPLE_VISION_PROVIDER_ID
    }

    async fn recognize(&self, request: OcrRequest) -> OcrResult<OcrOutput> {
        recognize_apple_vision(request)
    }
}

#[derive(Debug, Clone)]
pub struct TesseractProvider {
    models_dir: PathBuf,
}

impl TesseractProvider {
    pub fn with_models_dir(models_dir: impl Into<PathBuf>) -> Self {
        Self {
            models_dir: models_dir.into(),
        }
    }
}

#[async_trait]
impl OcrProvider for TesseractProvider {
    fn provider(&self) -> &'static str {
        TESSERACT_PROVIDER_ID
    }

    async fn recognize(&self, request: OcrRequest) -> OcrResult<OcrOutput> {
        tesseract::recognize(Some(&self.models_dir), request)
    }
}

#[derive(Debug, Clone)]
pub struct PaddleOcrProvider {
    models_dir: PathBuf,
}

impl PaddleOcrProvider {
    pub fn with_models_dir(models_dir: impl Into<PathBuf>) -> Self {
        Self {
            models_dir: models_dir.into(),
        }
    }
}

#[async_trait]
impl OcrProvider for PaddleOcrProvider {
    fn provider(&self) -> &'static str {
        PADDLE_OCR_PROVIDER_ID
    }

    async fn recognize(&self, request: OcrRequest) -> OcrResult<OcrOutput> {
        paddle::recognize(Some(&self.models_dir), request).await
    }
}

#[cfg(any(target_os = "macos", test))]
const APPLE_VISION_MAX_IMAGE_DIMENSION: u32 = 1800;
#[cfg(target_os = "macos")]
const APPLE_VISION_DEFAULT_LANGUAGE: &str = "en-US";

#[cfg(target_os = "macos")]
thread_local! {
    static DEFAULT_RECOGNITION_LANGS: OnceCell<cidre::arc::R<ns::Array<ns::String>>> = const { OnceCell::new() };
}

#[cfg(target_os = "macos")]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppleVisionRequestOptions {
    recognition_mode: Option<OcrRecognitionMode>,
    language_correction: Option<bool>,
    max_image_dimension: Option<u32>,
    tile_rows: Option<u32>,
    tile_columns: Option<u32>,
}

#[cfg(target_os = "macos")]
struct PreparedVisionImage {
    pixel_buf: cidre::arc::R<cv::PixelBuf>,
}

#[cfg(target_os = "macos")]
impl PreparedVisionImage {
    fn from_grayscale(grayscale: image::GrayImage) -> OcrResult<Self> {
        let width = grayscale.width() as usize;
        let height = grayscale.height() as usize;
        let bytes_per_row = width;
        let bytes = grayscale.into_raw().into_boxed_slice();
        let base_address = bytes.as_ptr() as *mut c_void;
        let release_ref_con = Box::into_raw(Box::new(bytes)) as *mut c_void;

        let pixel_buf = cv::PixelBuf::with_bytes(
            width,
            height,
            base_address,
            bytes_per_row,
            release_grayscale_pixel_buffer_bytes,
            release_ref_con,
            cv::PixelFormat::_8_INDEXED_GREY_WHITE_IS_ZERO,
            None,
        )
        .map_err(|error| {
            OcrError::Provider(format!(
                "Apple Vision OCR pixel buffer setup failed: {error}"
            ))
        })?;

        Ok(Self { pixel_buf })
    }
}

#[cfg(target_os = "macos")]
extern "C" fn release_grayscale_pixel_buffer_bytes(
    release_ref_con: *mut c_void,
    _base_address: *const *const c_void,
) {
    if !release_ref_con.is_null() {
        unsafe {
            drop(Box::from_raw(release_ref_con as *mut Box<[u8]>));
        }
    }
}

#[cfg(target_os = "macos")]
fn resize_for_ocr(image: DynamicImage, max_image_dimension: u32) -> DynamicImage {
    let (width, height) = image.dimensions();
    let longest_dimension = width.max(height);
    if longest_dimension <= max_image_dimension {
        return image;
    }
    let scale = max_image_dimension as f64 / longest_dimension as f64;
    let resized_width = ((width as f64 * scale).round() as u32).max(1);
    let resized_height = ((height as f64 * scale).round() as u32).max(1);
    image.resize_exact(resized_width, resized_height, FilterType::Triangle)
}

#[cfg(target_os = "macos")]
fn recognize_apple_vision(request: OcrRequest) -> OcrResult<OcrOutput> {
    objc::ar_pool(|| recognize_apple_vision_impl(request).map_err(|error| error.to_string()))
        .map_err(OcrError::Provider)
}

#[cfg(target_os = "macos")]
fn recognize_apple_vision_impl(request: OcrRequest) -> OcrResult<OcrOutput> {
    let options: AppleVisionRequestOptions = serde_json::from_value(
        serde_json::to_value(&request.options).unwrap_or_else(|_| serde_json::Value::Null),
    )
    .unwrap_or(AppleVisionRequestOptions {
        recognition_mode: None,
        language_correction: None,
        max_image_dimension: None,
        tile_rows: None,
        tile_columns: None,
    });

    let recognition_level = match options.recognition_mode.unwrap_or(OcrRecognitionMode::Fast) {
        OcrRecognitionMode::Fast => vn::RequestTextRecognitionLevel::Fast,
        OcrRecognitionMode::Accurate => vn::RequestTextRecognitionLevel::Accurate,
    };
    let language_correction = options.language_correction.unwrap_or(false);
    let recognition_langs = cached_recognition_langs(request.language.as_deref())?;
    let max_image_dimension = options
        .max_image_dimension
        .unwrap_or(APPLE_VISION_MAX_IMAGE_DIMENSION)
        .max(1);
    let decoded = image::open(&request.image_path).map_err(|error| {
        OcrError::Provider(format!("Apple Vision OCR image decode failed: {error}"))
    })?;
    let grayscale = resize_for_ocr(decoded, max_image_dimension).to_luma8();
    let observations = recognize_apple_vision_with_options(
        &grayscale,
        recognition_level,
        language_correction,
        recognition_langs.as_ref(),
        options.tile_rows.unwrap_or(1),
        options.tile_columns.unwrap_or(1),
    )?;
    let text = join_observation_text(&observations);
    let structured_payload =
        OcrStructuredPayload::new(APPLE_VISION_PROVIDER_ID, None, observations);

    Ok(OcrOutput::new(text, structured_payload)
        .with_provider_version(ns::ProcessInfo::current().os_version_string().to_string()))
}

#[cfg(target_os = "macos")]
fn recognize_apple_vision_with_options(
    grayscale: &image::GrayImage,
    recognition_level: vn::RequestTextRecognitionLevel,
    language_correction: bool,
    recognition_langs: &ns::Array<ns::String>,
    tile_rows: u32,
    tile_columns: u32,
) -> OcrResult<Vec<OcrObservation>> {
    let tile_rows = tile_rows.max(1);
    let tile_columns = tile_columns.max(1);
    if tile_rows == 1 && tile_columns == 1 {
        return perform_apple_vision_request(
            grayscale,
            recognition_level,
            language_correction,
            recognition_langs,
        );
    }

    let full_width = grayscale.width();
    let full_height = grayscale.height();
    let tile_width = full_width.div_ceil(tile_columns).max(1);
    let tile_height = full_height.div_ceil(tile_rows).max(1);
    let mut observations = Vec::new();

    for row in 0..tile_rows {
        for column in 0..tile_columns {
            let origin_x = column.saturating_mul(tile_width);
            let origin_y = row.saturating_mul(tile_height);
            if origin_x >= full_width || origin_y >= full_height {
                continue;
            }
            let width = (full_width - origin_x).min(tile_width);
            let height = (full_height - origin_y).min(tile_height);
            let tile =
                image::imageops::crop_imm(grayscale, origin_x, origin_y, width, height).to_image();
            let tile_observations = perform_apple_vision_request(
                &tile,
                recognition_level,
                language_correction,
                recognition_langs,
            )?;
            observations.extend(tile_observations.into_iter().map(|observation| {
                remap_tiled_observation(
                    observation,
                    origin_x,
                    origin_y,
                    width,
                    height,
                    full_width,
                    full_height,
                )
            }));
        }
    }

    observations.sort_by(|left, right| {
        let left_top = left.bounding_box.y + left.bounding_box.height;
        let right_top = right.bounding_box.y + right.bounding_box.height;
        right_top
            .partial_cmp(&left_top)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                left.bounding_box
                    .x
                    .partial_cmp(&right.bounding_box.x)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });
    Ok(observations)
}

#[cfg(target_os = "macos")]
fn perform_apple_vision_request(
    grayscale: &image::GrayImage,
    recognition_level: vn::RequestTextRecognitionLevel,
    language_correction: bool,
    recognition_langs: &ns::Array<ns::String>,
) -> OcrResult<Vec<OcrObservation>> {
    let prepared_image = PreparedVisionImage::from_grayscale(grayscale.clone())?;
    let mut vision_request = vn::RecognizeTextRequest::new();
    vision_request.set_recognition_level(recognition_level);
    vision_request.set_uses_lang_correction(language_correction);
    vision_request.set_recognition_langs(recognition_langs);

    let requests = ns::Array::<vn::Request>::from_slice(&[&vision_request]);
    let handler =
        vn::ImageRequestHandler::with_cv_pixel_buf(prepared_image.pixel_buf.as_ref(), None)
            .ok_or_else(|| {
                OcrError::Provider(
                    "Apple Vision OCR failed to create image request handler".to_string(),
                )
            })?;

    handler
        .perform(&requests)
        .map_err(|error| OcrError::Provider(format!("Apple Vision OCR failed: {error}")))?;

    Ok(vision_request
        .results()
        .map(|results| recognized_observations(&results))
        .unwrap_or_default())
}

#[cfg(target_os = "macos")]
fn remap_tiled_observation(
    mut observation: OcrObservation,
    origin_x: u32,
    origin_y: u32,
    tile_width: u32,
    tile_height: u32,
    full_width: u32,
    full_height: u32,
) -> OcrObservation {
    let tile_width_f = tile_width as f64;
    let tile_height_f = tile_height as f64;
    let full_width_f = full_width as f64;
    let full_height_f = full_height as f64;
    let x_pixels = observation.bounding_box.x * tile_width_f + origin_x as f64;
    let y_pixels =
        observation.bounding_box.y * tile_height_f + (full_height - origin_y - tile_height) as f64;
    let width_pixels = observation.bounding_box.width * tile_width_f;
    let height_pixels = observation.bounding_box.height * tile_height_f;
    observation.bounding_box = OcrBoundingBox::new(
        x_pixels / full_width_f,
        y_pixels / full_height_f,
        width_pixels / full_width_f,
        height_pixels / full_height_f,
    );
    observation
}

#[cfg(target_os = "macos")]
fn cached_recognition_langs(
    requested_language: Option<&str>,
) -> OcrResult<cidre::arc::R<ns::Array<ns::String>>> {
    let Some(language) = requested_language.and_then(normalize_apple_vision_recognition_language)
    else {
        return Ok(default_recognition_langs());
    };
    let langs: cidre::arc::R<ns::Array<ns::String>> = [language.as_str()].as_slice().into();
    Ok(langs)
}

#[cfg(target_os = "macos")]
fn normalize_apple_vision_recognition_language(language: &str) -> Option<String> {
    let normalized = language.trim();
    if normalized.is_empty() {
        return None;
    }
    let normalized = match normalized {
        "eng" | "en" => "en-US",
        "fra" | "fre" | "fr" => "fr-FR",
        "deu" | "ger" | "de" => "de-DE",
        "spa" | "es" => "es-ES",
        "ita" | "it" => "it-IT",
        "por" | "pt" => "pt-PT",
        other => other,
    };
    Some(normalized.to_string())
}

#[cfg(target_os = "macos")]
fn default_recognition_langs() -> cidre::arc::R<ns::Array<ns::String>> {
    DEFAULT_RECOGNITION_LANGS.with(|cache| {
        cache
            .get_or_init(|| [APPLE_VISION_DEFAULT_LANGUAGE].as_slice().into())
            .clone()
    })
}

#[cfg(target_os = "macos")]
fn recognized_observations(
    results: &ns::Array<vn::RecognizedTextObservation>,
) -> Vec<OcrObservation> {
    results
        .iter()
        .filter_map(|observation| {
            let candidates = observation.top_candidates(1);
            let candidate = candidates.first()?;
            let text = normalize_candidate_text(&candidate.string().to_string())?;
            Some(OcrObservation::new(
                text,
                candidate.confidence(),
                OcrBoundingBox::from_rect(observation.bounding_box()),
            ))
        })
        .collect()
}

#[cfg(any(target_os = "macos", test))]
fn normalize_candidate_text(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_owned())
    }
}

#[cfg(any(target_os = "macos", test))]
fn join_observation_text(observations: &[OcrObservation]) -> String {
    observations
        .iter()
        .map(|observation| observation.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(not(target_os = "macos"))]
fn recognize_apple_vision(_request: OcrRequest) -> OcrResult<OcrOutput> {
    Err(OcrError::Provider(
        "Apple Vision OCR is only available on macOS".to_string(),
    ))
}

#[derive(Debug, Error)]
pub enum ModelStatusError {
    #[error("model descriptor for provider {provider} is missing an app-managed model id")]
    MissingAppManagedModelId { provider: String },
    #[error("unsafe path component in {field}: {value}")]
    UnsafePathComponent { field: &'static str, value: String },
    #[error("failed to read marker {path}: {source}")]
    ReadMarker {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to parse marker {path}: {source}")]
    ParseMarker {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to encode marker {path}: {source}")]
    EncodeMarker {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to write marker {path}: {source}")]
    WriteMarker {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to create model directory {path}: {source}")]
    CreateModelDir {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

#[derive(Debug, Error)]
pub enum ModelInstallError {
    #[error(transparent)]
    Status(#[from] ModelStatusError),
    #[error("app-managed model descriptor for provider {provider} is missing an artifact")]
    MissingArtifact { provider: String },
    #[error("OS-managed model {provider} cannot be installed by the app")]
    OsManagedModel { provider: String },
    #[error("unsafe archive entry path: {path}")]
    UnsafeArchiveEntry { path: String },
    #[error("failed to read archive {path}: {source}")]
    ReadArchive {
        path: PathBuf,
        #[source]
        source: zip::result::ZipError,
    },
    #[error("downloaded artifact checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("installed model layout is incomplete; missing files: {missing_files:?}")]
    IncompleteInstalledLayout { missing_files: Vec<String> },
    #[error("failed to create directory {path}: {source}")]
    CreateDir {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to read file {path}: {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to copy file from {from} to {to}: {source}")]
    CopyFile {
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to create file {path}: {source}")]
    CreateFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to remove file {path}: {source}")]
    RemoveFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to remove directory {path}: {source}")]
    RemoveDir {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OcrModelManifest {
    pub version: u32,
    pub models: Vec<OcrModelDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OcrModelDescriptor {
    pub provider: String,
    pub model_id: Option<String>,
    pub display_name: String,
    pub description: String,
    pub license_label: Option<String>,
    pub source_url: Option<String>,
    pub management: ModelManagement,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelManagement {
    AppManaged {
        expected_layout: InstalledModelLayout,
        artifact: Option<ModelArtifact>,
    },
    OsManaged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModelLayout {
    pub marker_file_name: String,
    pub required_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelArtifact {
    pub url: String,
    pub byte_size: u64,
    pub sha256: String,
    pub shape: ModelArtifactShape,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelArtifactFile {
    pub relative_path: String,
    pub url: String,
    pub byte_size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelArtifactShape {
    SingleFile { file_name: String },
    Archive,
    MultiFile { files: Vec<ModelArtifactFile> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModelMarker {
    pub manifest_version: u32,
    pub provider: String,
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FailedModelMarker {
    pub manifest_version: u32,
    pub provider: String,
    pub model_id: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OcrModelStatus {
    pub provider: String,
    pub model_id: Option<String>,
    pub status: ModelStatusKind,
    pub install_path: Option<PathBuf>,
    pub missing_files: Vec<String>,
    pub failure_message: Option<String>,
}

impl OcrModelStatus {
    pub fn is_available(&self) -> bool {
        matches!(
            self.status,
            ModelStatusKind::Installed | ModelStatusKind::OsManaged
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelStatusKind {
    Installed,
    Missing,
    Downloading,
    Failed,
    OsManaged,
}

pub fn ocr_models_dir(app_data_dir: impl AsRef<Path>) -> PathBuf {
    app_data_dir.as_ref().join(MODEL_STORE_DIR_NAME)
}

fn mnema_release_archive_artifact(
    url_key: &str,
    byte_size_key: &str,
    sha256_key: &str,
) -> Option<ModelArtifact> {
    let url = std::env::var(url_key)
        .ok()
        .filter(|value| !value.trim().is_empty())?;
    let byte_size = std::env::var(byte_size_key)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())?;
    let sha256 = std::env::var(sha256_key)
        .ok()
        .filter(|value| value.len() == 64)?;

    Some(ModelArtifact {
        url,
        byte_size,
        sha256,
        shape: ModelArtifactShape::Archive,
    })
}

fn builtin_tesseract_artifact() -> Option<ModelArtifact> {
    mnema_release_archive_artifact(
        "MNEMA_OCR_TESSERACT_BUNDLE_URL",
        "MNEMA_OCR_TESSERACT_BUNDLE_BYTE_SIZE",
        "MNEMA_OCR_TESSERACT_BUNDLE_SHA256",
    )
    .or_else(|| {
        Some(ModelArtifact {
            url: "https://github.com/tesseract-ocr/tessdata_fast".to_string(),
            byte_size: 23_143_206,
            sha256: "02a21f73272441ac9f07b5e140b5362f63be60ff87dd84ec3d965f204bea9016"
                .to_string(),
            shape: ModelArtifactShape::MultiFile {
                files: vec![
                    ModelArtifactFile {
                        relative_path: "tessdata/eng.traineddata".to_string(),
                        url: "https://github.com/tesseract-ocr/tessdata_fast/raw/4.1.0/eng.traineddata".to_string(),
                        byte_size: 4_113_088,
                        sha256: "7d4322bd2a7749724879683fc3912cb542f19906c83bcc1a52132556427170b2".to_string(),
                    },
                    ModelArtifactFile {
                        relative_path: "tessdata/osd.traineddata".to_string(),
                        url: "https://github.com/tesseract-ocr/tessdata_fast/raw/4.1.0/osd.traineddata".to_string(),
                        byte_size: 10_562_727,
                        sha256: "9cf5d576fcc47564f11265841e5ca839001e7e6f38ff7f7aacf46d15a96b00ff".to_string(),
                    },
                ],
            },
        })
    })
}

fn builtin_paddle_ocr_artifact() -> Option<ModelArtifact> {
    mnema_release_archive_artifact(
        "MNEMA_OCR_PADDLE_BUNDLE_URL",
        "MNEMA_OCR_PADDLE_BUNDLE_BYTE_SIZE",
        "MNEMA_OCR_PADDLE_BUNDLE_SHA256",
    )
    .or_else(|| {
        Some(ModelArtifact {
            url: "https://github.com/zibo-chen/rust-paddle-ocr".to_string(),
            byte_size: 8_729_500,
            sha256: "ae73b89b0dec9f8ea2e8e61f2794970ce1938679d89e0fe89eb4115509acc013".to_string(),
            shape: ModelArtifactShape::MultiFile {
                files: vec![
                    ModelArtifactFile {
                        relative_path: "det/model.mnn".to_string(),
                        url: "https://raw.githubusercontent.com/zibo-chen/rust-paddle-ocr/b7141e7d0289eff67d4a97e79a00d9db72345d89/models/PP-OCRv5_mobile_det.mnn".to_string(),
                        byte_size: 4_760_244,
                        sha256: "326f846bb5c903282e116ea089e8796b67921586726cca9457730436a79684c3".to_string(),
                    },
                    ModelArtifactFile {
                        relative_path: "rec/model.mnn".to_string(),
                        url: "https://raw.githubusercontent.com/zibo-chen/rust-paddle-ocr/b7141e7d0289eff67d4a97e79a00d9db72345d89/models/en_PP-OCRv5_mobile_rec_infer.mnn".to_string(),
                        byte_size: 3_967_840,
                        sha256: "c5e747fb69275e9d99fbe54f97642c65681c3e4244383f825d1d7668e9aece81".to_string(),
                    },
                    ModelArtifactFile {
                        relative_path: "rec/charset.txt".to_string(),
                        url: "https://raw.githubusercontent.com/zibo-chen/rust-paddle-ocr/b7141e7d0289eff67d4a97e79a00d9db72345d89/models/ppocr_keys_en.txt".to_string(),
                        byte_size: 1_416,
                        sha256: "e025a66d31f327ba0c232e03f407ae8d105e1e709e7ccb3f408aa778c24e70d6".to_string(),
                    },
                ],
            },
        })
    })
}

pub fn builtin_model_manifest() -> OcrModelManifest {
    OcrModelManifest {
        version: MANIFEST_VERSION,
        models: vec![
            OcrModelDescriptor {
                provider: APPLE_VISION_PROVIDER_ID.to_string(),
                model_id: None,
                display_name: "Apple Vision".to_string(),
                description: "OS-managed Apple Vision OCR executed through Vision.framework. No app-managed download.".to_string(),
                license_label: None,
                source_url: Some("https://developer.apple.com/documentation/vision".to_string()),
                management: ModelManagement::OsManaged,
            },
            OcrModelDescriptor {
                provider: TESSERACT_PROVIDER_ID.to_string(),
                model_id: Some(DEFAULT_TESSERACT_MODEL_ID.to_string()),
                display_name: "Tesseract English".to_string(),
                description: "Embedded Tesseract engine in the app build with Mnema-managed eng/osd tessdata downloads.".to_string(),
                license_label: Some("Apache-2.0".to_string()),
                source_url: Some("https://tesseract-ocr.github.io/tessdoc/Installation.html".to_string()),
                management: ModelManagement::AppManaged {
                    expected_layout: InstalledModelLayout {
                        marker_file_name: INSTALLED_MARKER_FILE_NAME.to_string(),
                        required_files: vec![
                            "tessdata/eng.traineddata".to_string(),
                            "tessdata/osd.traineddata".to_string(),
                        ],
                    },
                    artifact: builtin_tesseract_artifact(),
                },
            },
            OcrModelDescriptor {
                provider: PADDLE_OCR_PROVIDER_ID.to_string(),
                model_id: Some(DEFAULT_PADDLE_OCR_MODEL_ID.to_string()),
                display_name: "PaddleOCR English mobile v5".to_string(),
                description: "Mnema-managed PaddleOCR English detector/recognizer bundle backed by pinned MNN artifacts from ocr-rs.".to_string(),
                license_label: Some("Apache-2.0".to_string()),
                source_url: Some("https://github.com/zibo-chen/rust-paddle-ocr".to_string()),
                management: ModelManagement::AppManaged {
                    expected_layout: InstalledModelLayout {
                        marker_file_name: INSTALLED_MARKER_FILE_NAME.to_string(),
                        required_files: vec![
                            "det/model.mnn".to_string(),
                            "rec/model.mnn".to_string(),
                            "rec/charset.txt".to_string(),
                        ],
                    },
                    artifact: builtin_paddle_ocr_artifact(),
                },
            },
        ],
    }
}

pub fn find_model_descriptor<'a>(
    manifest: &'a OcrModelManifest,
    provider: &str,
    model_id: Option<&str>,
) -> Option<&'a OcrModelDescriptor> {
    manifest.models.iter().find(|descriptor| {
        descriptor.provider == provider && descriptor.model_id.as_deref() == model_id
    })
}

pub fn model_install_dir(
    models_dir: impl AsRef<Path>,
    provider: &str,
    model_id: &str,
) -> Result<PathBuf, ModelStatusError> {
    assert_safe_path_component("provider", provider)?;
    assert_safe_path_component("model_id", model_id)?;
    Ok(models_dir.as_ref().join(provider).join(model_id))
}

pub fn detect_model_status(
    models_dir: impl AsRef<Path>,
    descriptor: &OcrModelDescriptor,
) -> Result<OcrModelStatus, ModelStatusError> {
    match &descriptor.management {
        ModelManagement::OsManaged => Ok(OcrModelStatus {
            provider: descriptor.provider.clone(),
            model_id: descriptor.model_id.clone(),
            status: ModelStatusKind::OsManaged,
            install_path: None,
            missing_files: Vec::new(),
            failure_message: None,
        }),
        ModelManagement::AppManaged {
            expected_layout, ..
        } => {
            let model_id = descriptor.model_id.as_deref().ok_or_else(|| {
                ModelStatusError::MissingAppManagedModelId {
                    provider: descriptor.provider.clone(),
                }
            })?;
            let install_path = model_install_dir(models_dir, &descriptor.provider, model_id)?;
            let missing_files = missing_required_files(&install_path, expected_layout);
            let installed_marker = install_path.join(&expected_layout.marker_file_name);
            let downloading_marker = install_path.join(DOWNLOADING_MARKER_FILE_NAME);
            let failed_marker = install_path.join(FAILED_MARKER_FILE_NAME);

            if installed_marker_matches(&installed_marker, &descriptor.provider, model_id)
                && missing_files.is_empty()
            {
                return Ok(OcrModelStatus {
                    provider: descriptor.provider.clone(),
                    model_id: descriptor.model_id.clone(),
                    status: ModelStatusKind::Installed,
                    install_path: Some(install_path),
                    missing_files,
                    failure_message: None,
                });
            }

            if downloading_marker.exists() {
                return Ok(OcrModelStatus {
                    provider: descriptor.provider.clone(),
                    model_id: descriptor.model_id.clone(),
                    status: ModelStatusKind::Downloading,
                    install_path: Some(install_path),
                    missing_files,
                    failure_message: None,
                });
            }

            if failed_marker.is_file() {
                let message = read_failed_marker(&failed_marker)
                    .map(|marker| marker.message)
                    .unwrap_or_else(|error| error.to_string());
                return Ok(OcrModelStatus {
                    provider: descriptor.provider.clone(),
                    model_id: descriptor.model_id.clone(),
                    status: ModelStatusKind::Failed,
                    install_path: Some(install_path),
                    missing_files,
                    failure_message: Some(message),
                });
            }

            Ok(OcrModelStatus {
                provider: descriptor.provider.clone(),
                model_id: descriptor.model_id.clone(),
                status: ModelStatusKind::Missing,
                install_path: Some(install_path),
                missing_files,
                failure_message: None,
            })
        }
    }
}

pub fn list_model_statuses(
    models_dir: impl AsRef<Path>,
    manifest: &OcrModelManifest,
) -> Result<Vec<OcrModelStatus>, ModelStatusError> {
    manifest
        .models
        .iter()
        .map(|descriptor| detect_model_status(&models_dir, descriptor))
        .collect()
}

pub fn write_downloading_marker(
    models_dir: impl AsRef<Path>,
    provider: &str,
    model_id: &str,
) -> Result<PathBuf, ModelStatusError> {
    let install_path = model_install_dir(models_dir, provider, model_id)?;
    fs::create_dir_all(&install_path).map_err(|source| ModelStatusError::CreateModelDir {
        path: install_path.clone(),
        source,
    })?;
    let path = install_path.join(DOWNLOADING_MARKER_FILE_NAME);
    fs::write(&path, b"").map_err(|source| ModelStatusError::WriteMarker {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

pub fn write_installed_marker(
    models_dir: impl AsRef<Path>,
    provider: &str,
    model_id: &str,
) -> Result<PathBuf, ModelStatusError> {
    let install_path = model_install_dir(models_dir, provider, model_id)?;
    fs::create_dir_all(&install_path).map_err(|source| ModelStatusError::CreateModelDir {
        path: install_path.clone(),
        source,
    })?;
    let path = install_path.join(INSTALLED_MARKER_FILE_NAME);
    let marker = InstalledModelMarker {
        manifest_version: MANIFEST_VERSION,
        provider: provider.to_string(),
        model_id: model_id.to_string(),
    };
    let bytes =
        serde_json::to_vec_pretty(&marker).map_err(|source| ModelStatusError::EncodeMarker {
            path: path.clone(),
            source,
        })?;
    fs::write(&path, bytes).map_err(|source| ModelStatusError::WriteMarker {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

pub fn write_failed_marker(
    models_dir: impl AsRef<Path>,
    provider: &str,
    model_id: &str,
    message: impl Into<String>,
) -> Result<PathBuf, ModelStatusError> {
    let install_path = model_install_dir(models_dir, provider, model_id)?;
    fs::create_dir_all(&install_path).map_err(|source| ModelStatusError::CreateModelDir {
        path: install_path.clone(),
        source,
    })?;
    let path = install_path.join(FAILED_MARKER_FILE_NAME);
    let marker = FailedModelMarker {
        manifest_version: MANIFEST_VERSION,
        provider: provider.to_string(),
        model_id: model_id.to_string(),
        message: message.into(),
    };
    let bytes =
        serde_json::to_vec_pretty(&marker).map_err(|source| ModelStatusError::EncodeMarker {
            path: path.clone(),
            source,
        })?;
    fs::write(&path, bytes).map_err(|source| ModelStatusError::WriteMarker {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

pub fn remove_model_file_if_exists(path: impl AsRef<Path>) -> Result<(), ModelInstallError> {
    let path = path.as_ref();
    if path.exists() {
        fs::remove_file(path).map_err(|source| ModelInstallError::RemoveFile {
            path: path.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

pub fn remove_model_dir_if_exists(path: impl AsRef<Path>) -> Result<(), ModelInstallError> {
    let path = path.as_ref();
    if path.exists() {
        fs::remove_dir_all(path).map_err(|source| ModelInstallError::RemoveDir {
            path: path.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

pub fn sha256_file(path: impl AsRef<Path>) -> Result<String, ModelInstallError> {
    let path = path.as_ref();
    let mut file = fs::File::open(path).map_err(|source| ModelInstallError::ReadFile {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| ModelInstallError::ReadFile {
                path: path.to_path_buf(),
                source,
            })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn validate_artifact_sha256(
    artifact_path: impl AsRef<Path>,
    expected_sha256: &str,
) -> Result<String, ModelInstallError> {
    let actual = sha256_file(artifact_path)?;
    if !actual.eq_ignore_ascii_case(expected_sha256) {
        return Err(ModelInstallError::ChecksumMismatch {
            expected: expected_sha256.to_string(),
            actual,
        });
    }
    Ok(actual)
}

pub fn install_downloaded_model_artifact(
    models_dir: impl AsRef<Path>,
    descriptor: &OcrModelDescriptor,
    artifact_path: impl AsRef<Path>,
) -> Result<PathBuf, ModelInstallError> {
    let (expected_layout, artifact) = match &descriptor.management {
        ModelManagement::AppManaged {
            expected_layout,
            artifact: Some(artifact),
        } => (expected_layout, artifact),
        ModelManagement::AppManaged { artifact: None, .. } => {
            return Err(ModelInstallError::MissingArtifact {
                provider: descriptor.provider.clone(),
            })
        }
        ModelManagement::OsManaged => {
            return Err(ModelInstallError::OsManagedModel {
                provider: descriptor.provider.clone(),
            })
        }
    };

    let model_id = descriptor.model_id.as_deref().ok_or_else(|| {
        ModelStatusError::MissingAppManagedModelId {
            provider: descriptor.provider.clone(),
        }
    })?;
    let models_dir = models_dir.as_ref();
    let install_dir = model_install_dir(models_dir, &descriptor.provider, model_id)?;

    fs::create_dir_all(&install_dir).map_err(|source| ModelInstallError::CreateDir {
        path: install_dir.clone(),
        source,
    })?;
    remove_model_file_if_exists(install_dir.join(INSTALLED_MARKER_FILE_NAME))?;
    remove_model_file_if_exists(install_dir.join(FAILED_MARKER_FILE_NAME))?;

    match &artifact.shape {
        ModelArtifactShape::SingleFile { file_name } => {
            assert_safe_path_component("artifact.file_name", file_name)?;
            let destination = install_dir.join(file_name);
            fs::copy(artifact_path.as_ref(), &destination).map_err(|source| {
                ModelInstallError::CopyFile {
                    from: artifact_path.as_ref().to_path_buf(),
                    to: destination,
                    source,
                }
            })?;
        }
        ModelArtifactShape::Archive => extract_zip_artifact(artifact_path.as_ref(), &install_dir)?,
        ModelArtifactShape::MultiFile { .. } => {
            return Err(ModelInstallError::MissingArtifact {
                provider: descriptor.provider.clone(),
            })
        }
    }

    let missing_files = missing_required_files(&install_dir, expected_layout);
    if !missing_files.is_empty() {
        return Err(ModelInstallError::IncompleteInstalledLayout { missing_files });
    }

    remove_model_file_if_exists(install_dir.join(DOWNLOADING_MARKER_FILE_NAME))?;
    Ok(write_installed_marker(
        models_dir,
        &descriptor.provider,
        model_id,
    )?)
}

fn extract_zip_artifact(artifact_path: &Path, install_dir: &Path) -> Result<(), ModelInstallError> {
    let file = fs::File::open(artifact_path).map_err(|source| ModelInstallError::ReadFile {
        path: artifact_path.to_path_buf(),
        source,
    })?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|source| ModelInstallError::ReadArchive {
            path: artifact_path.to_path_buf(),
            source,
        })?;

    for index in 0..archive.len() {
        let mut entry =
            archive
                .by_index(index)
                .map_err(|source| ModelInstallError::ReadArchive {
                    path: artifact_path.to_path_buf(),
                    source,
                })?;
        let enclosed = entry
            .enclosed_name()
            .ok_or_else(|| ModelInstallError::UnsafeArchiveEntry {
                path: entry.name().to_string(),
            })?
            .to_path_buf();
        let destination = install_dir.join(enclosed);

        if entry.is_dir() {
            fs::create_dir_all(&destination).map_err(|source| ModelInstallError::CreateDir {
                path: destination,
                source,
            })?;
            continue;
        }

        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|source| ModelInstallError::CreateDir {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let mut output =
            fs::File::create(&destination).map_err(|source| ModelInstallError::CreateFile {
                path: destination.clone(),
                source,
            })?;
        io::copy(&mut entry, &mut output).map_err(|source| ModelInstallError::CopyFile {
            from: artifact_path.to_path_buf(),
            to: destination,
            source,
        })?;
    }

    Ok(())
}

fn installed_marker_matches(path: &Path, provider: &str, model_id: &str) -> bool {
    if !path.is_file() {
        return false;
    }
    read_installed_marker(path).is_ok_and(|marker| {
        marker.manifest_version == MANIFEST_VERSION
            && marker.provider == provider
            && marker.model_id == model_id
    })
}

fn read_installed_marker(path: &Path) -> Result<InstalledModelMarker, ModelStatusError> {
    let bytes = fs::read(path).map_err(|source| ModelStatusError::ReadMarker {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(|source| ModelStatusError::ParseMarker {
        path: path.to_path_buf(),
        source,
    })
}

fn read_failed_marker(path: &Path) -> Result<FailedModelMarker, ModelStatusError> {
    let bytes = fs::read(path).map_err(|source| ModelStatusError::ReadMarker {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(|source| ModelStatusError::ParseMarker {
        path: path.to_path_buf(),
        source,
    })
}

fn missing_required_files(model_dir: &Path, expected_layout: &InstalledModelLayout) -> Vec<String> {
    expected_layout
        .required_files
        .iter()
        .filter(|relative_path| !model_dir.join(relative_path).is_file())
        .cloned()
        .collect()
}

fn assert_safe_path_component(field: &'static str, value: &str) -> Result<(), ModelStatusError> {
    if value.is_empty()
        || Path::new(value)
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ModelStatusError::UnsafePathComponent {
            field,
            value: value.to_string(),
        });
    }
    Ok(())
}

pub fn provider_runtime_available(provider: &str) -> bool {
    match provider {
        APPLE_VISION_PROVIDER_ID => cfg!(target_os = "macos"),
        TESSERACT_PROVIDER_ID => tesseract::runtime_available(),
        PADDLE_OCR_PROVIDER_ID => paddle::runtime_available(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(model_id: &str) -> OcrModelDescriptor {
        OcrModelDescriptor {
            provider: TESSERACT_PROVIDER_ID.to_string(),
            model_id: Some(model_id.to_string()),
            display_name: "Test".to_string(),
            description: "Test".to_string(),
            license_label: None,
            source_url: None,
            management: ModelManagement::AppManaged {
                expected_layout: InstalledModelLayout {
                    marker_file_name: INSTALLED_MARKER_FILE_NAME.to_string(),
                    required_files: vec!["runtime.json".to_string()],
                },
                artifact: None,
            },
        }
    }

    #[test]
    fn frozen_payload_reads_legacy_apple_payload() {
        let payload = FrozenOcrPayload::from_payload_json(Some(
            r#"{"recognitionMode":"accurate","languageCorrection":true}"#,
        ))
        .expect("legacy payload should parse");
        assert_eq!(payload.provider, APPLE_VISION_PROVIDER_ID);
        assert_eq!(payload.model_id, None);
        assert_eq!(
            payload.options.get("recognitionMode"),
            Some(&serde_json::Value::String("accurate".to_string()))
        );
        assert_eq!(
            payload.options.get("languageCorrection"),
            Some(&serde_json::Value::Bool(true))
        );
    }

    #[test]
    fn structured_payload_carries_provenance() {
        let payload = OcrStructuredPayload::new(
            TESSERACT_PROVIDER_ID,
            Some(DEFAULT_TESSERACT_MODEL_ID.to_string()),
            vec![OcrObservation::new(
                "hello",
                0.9,
                OcrBoundingBox::new(0.1, 0.2, 0.3, 0.4),
            )],
        );
        let json = serde_json::to_value(&payload).expect("payload serializes");
        assert_eq!(json["provider"], TESSERACT_PROVIDER_ID);
        assert_eq!(json["modelId"], DEFAULT_TESSERACT_MODEL_ID);
        assert_eq!(json["provenance"]["provider"], TESSERACT_PROVIDER_ID);
    }

    #[test]
    fn model_status_tracks_missing_downloading_failed_and_installed() {
        let temp = tempfile::tempdir().expect("tempdir");
        let descriptor = descriptor(DEFAULT_TESSERACT_MODEL_ID);
        let status = detect_model_status(temp.path(), &descriptor).expect("status");
        assert_eq!(status.status, ModelStatusKind::Missing);

        write_downloading_marker(
            temp.path(),
            TESSERACT_PROVIDER_ID,
            DEFAULT_TESSERACT_MODEL_ID,
        )
        .expect("marker");
        let status = detect_model_status(temp.path(), &descriptor).expect("status");
        assert_eq!(status.status, ModelStatusKind::Downloading);

        let install_dir = model_install_dir(
            temp.path(),
            TESSERACT_PROVIDER_ID,
            DEFAULT_TESSERACT_MODEL_ID,
        )
        .expect("install dir");
        fs::remove_file(install_dir.join(DOWNLOADING_MARKER_FILE_NAME)).expect("remove marker");
        write_failed_marker(
            temp.path(),
            TESSERACT_PROVIDER_ID,
            DEFAULT_TESSERACT_MODEL_ID,
            "boom",
        )
        .expect("failed marker");
        let status = detect_model_status(temp.path(), &descriptor).expect("status");
        assert_eq!(status.status, ModelStatusKind::Failed);
        assert_eq!(status.failure_message.as_deref(), Some("boom"));

        fs::remove_file(install_dir.join(FAILED_MARKER_FILE_NAME)).expect("remove failed marker");
        fs::write(install_dir.join("runtime.json"), b"{}").expect("runtime file");
        write_installed_marker(
            temp.path(),
            TESSERACT_PROVIDER_ID,
            DEFAULT_TESSERACT_MODEL_ID,
        )
        .expect("installed marker");
        let status = detect_model_status(temp.path(), &descriptor).expect("status");
        assert_eq!(status.status, ModelStatusKind::Installed);
    }

    #[test]
    fn apple_vision_options_accept_benchmark_tuning_fields() {
        let options: AppleVisionRequestOptions = serde_json::from_value(serde_json::json!({
            "recognitionMode": "fast",
            "languageCorrection": false,
            "maxImageDimension": 1600,
            "tileRows": 2,
            "tileColumns": 2,
        }))
        .expect("options should deserialize");
        assert_eq!(options.max_image_dimension, Some(1600));
        assert_eq!(options.tile_rows, Some(2));
        assert_eq!(options.tile_columns, Some(2));
    }

    #[test]
    fn remap_tiled_observation_offsets_into_full_image_coordinates() {
        let remapped = remap_tiled_observation(
            OcrObservation::new("hello", 0.9, OcrBoundingBox::new(0.25, 0.5, 0.5, 0.25)),
            50,
            20,
            100,
            40,
            200,
            100,
        );

        assert_eq!(remapped.text, "hello");
        assert!((remapped.bounding_box.x - 0.375).abs() < f64::EPSILON);
        assert!((remapped.bounding_box.y - 0.6).abs() < f64::EPSILON);
        assert!((remapped.bounding_box.width - 0.25).abs() < f64::EPSILON);
        assert!((remapped.bounding_box.height - 0.1).abs() < f64::EPSILON);
    }
}
