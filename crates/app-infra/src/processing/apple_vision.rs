use super::{OcrOutput, OcrRequest};
use crate::{AppInfraError, Result};

#[cfg(target_os = "macos")]
use cidre::{cg, cv, ns, objc, vn};

#[cfg(target_os = "macos")]
use image::{DynamicImage, GenericImageView, imageops::FilterType};

#[cfg(target_os = "macos")]
use std::{cell::OnceCell, ffi::c_void};

#[cfg(target_os = "macos")]
use std::thread_local;

#[cfg(any(target_os = "macos", test))]
use serde::{Deserialize, Serialize};

#[cfg(any(target_os = "macos", test))]
const APPLE_VISION_PAYLOAD_SCHEMA_VERSION: u8 = 1;

#[cfg(any(target_os = "macos", test))]
const APPLE_VISION_COORDINATE_SPACE: &str = "normalized";

#[cfg(any(target_os = "macos", test))]
const APPLE_VISION_COORDINATE_ORIGIN: &str = "lower_left";

#[cfg(target_os = "macos")]
const APPLE_VISION_MAX_IMAGE_DIMENSION: u32 = 1200;

#[cfg(target_os = "macos")]
const APPLE_VISION_DEFAULT_LANGUAGE: &str = "en-US";

#[cfg(target_os = "macos")]
thread_local! {
    static DEFAULT_RECOGNITION_LANGS: OnceCell<cidre::arc::R<ns::Array<ns::String>>> = const { OnceCell::new() };
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppleVisionStructuredPayload {
    schema_version: u8,
    coordinate_space: String,
    coordinate_origin: String,
    observations: Vec<AppleVisionStructuredObservation>,
}

#[cfg(any(target_os = "macos", test))]
impl AppleVisionStructuredPayload {
    fn new(observations: Vec<AppleVisionStructuredObservation>) -> Self {
        Self {
            schema_version: APPLE_VISION_PAYLOAD_SCHEMA_VERSION,
            coordinate_space: APPLE_VISION_COORDINATE_SPACE.to_owned(),
            coordinate_origin: APPLE_VISION_COORDINATE_ORIGIN.to_owned(),
            observations,
        }
    }
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppleVisionStructuredObservation {
    text: String,
    confidence: f32,
    bounding_box: AppleVisionBoundingBox,
}

#[cfg(any(target_os = "macos", test))]
impl AppleVisionStructuredObservation {
    fn new(text: impl Into<String>, confidence: f32, bounding_box: AppleVisionBoundingBox) -> Self {
        Self {
            text: text.into(),
            confidence,
            bounding_box,
        }
    }

    #[cfg(target_os = "macos")]
    fn from_vision_observation(observation: &vn::RecognizedTextObservation) -> Option<Self> {
        let candidates = observation.top_candidates(1);
        let candidate = candidates.first()?;
        let text = normalize_candidate_text(&candidate.string().to_string())?;

        Some(Self::new(
            text,
            candidate.confidence(),
            AppleVisionBoundingBox::from_rect(observation.bounding_box()),
        ))
    }
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AppleVisionBoundingBox {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[cfg(any(target_os = "macos", test))]
impl AppleVisionBoundingBox {
    fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
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

#[cfg(target_os = "macos")]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AppleVisionRecognitionMode {
    Fast,
    Accurate,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppleVisionRequestPayload {
    language: Option<String>,
    recognition_mode: Option<AppleVisionRecognitionMode>,
    language_correction: Option<bool>,
}

#[cfg(target_os = "macos")]
impl AppleVisionRecognitionMode {
    fn to_vision_level(&self) -> vn::RequestTextRecognitionLevel {
        match self {
            Self::Fast => vn::RequestTextRecognitionLevel::Fast,
            Self::Accurate => vn::RequestTextRecognitionLevel::Accurate,
        }
    }
}

#[cfg(target_os = "macos")]
struct PreparedVisionImage {
    pixel_buf: cidre::arc::R<cv::PixelBuf>,
}

#[cfg(target_os = "macos")]
impl PreparedVisionImage {
    fn from_path(image_path: &std::path::Path) -> Result<Self> {
        let decoded = image::open(image_path).map_err(image_decode_error_to_app_error)?;
        let grayscale = resize_for_ocr(decoded).to_luma8();
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
        .map_err(pixel_buffer_error_to_app_error)?;

        Ok(Self { pixel_buf })
    }
}

#[cfg(target_os = "macos")]
extern "C" fn release_grayscale_pixel_buffer_bytes(
    release_ref_con: *mut c_void,
    _base_address: *const *const c_void,
) {
    if !release_ref_con.is_null() {
        // CoreVideo invokes this once it no longer needs the backing grayscale bytes.
        unsafe {
            drop(Box::from_raw(
                release_ref_con as *mut Box<[u8]>,
            ));
        }
    }
}

#[cfg(target_os = "macos")]
fn resize_for_ocr(image: DynamicImage) -> DynamicImage {
    let (width, height) = image.dimensions();
    let longest_dimension = width.max(height);

    if longest_dimension <= APPLE_VISION_MAX_IMAGE_DIMENSION {
        return image;
    }

    let scale = APPLE_VISION_MAX_IMAGE_DIMENSION as f64 / longest_dimension as f64;
    let resized_width = ((width as f64 * scale).round() as u32).max(1);
    let resized_height = ((height as f64 * scale).round() as u32).max(1);

    image.resize_exact(resized_width, resized_height, FilterType::Triangle)
}

#[cfg(target_os = "macos")]
fn cached_recognition_langs(payload_json: Option<&str>) -> Result<cidre::arc::R<ns::Array<ns::String>>> {
    let Some(language) = requested_language(payload_json)? else {
        return Ok(default_recognition_langs());
    };

    let langs: cidre::arc::R<ns::Array<ns::String>> = [language.as_str()].as_slice().into();
    Ok(langs)
}

#[cfg(target_os = "macos")]
fn requested_language(payload_json: Option<&str>) -> Result<Option<String>> {
    let Some(payload_json) = payload_json else {
        return Ok(None);
    };

    let payload: AppleVisionRequestPayload = serde_json::from_str(payload_json)?;
    Ok(payload
        .language
        .and_then(|language| normalize_recognition_language(&language)))
}

#[cfg(target_os = "macos")]
fn requested_recognition_mode(
    payload_json: Option<&str>,
) -> Result<vn::RequestTextRecognitionLevel> {
    let Some(payload_json) = payload_json else {
        return Ok(vn::RequestTextRecognitionLevel::Fast);
    };

    let payload: AppleVisionRequestPayload = serde_json::from_str(payload_json)?;
    Ok(payload
        .recognition_mode
        .unwrap_or(AppleVisionRecognitionMode::Fast)
        .to_vision_level())
}

#[cfg(target_os = "macos")]
fn requested_language_correction(payload_json: Option<&str>) -> Result<bool> {
    let Some(payload_json) = payload_json else {
        return Ok(false);
    };

    let payload: AppleVisionRequestPayload = serde_json::from_str(payload_json)?;
    Ok(payload.language_correction.unwrap_or(false))
}

#[cfg(target_os = "macos")]
fn normalize_recognition_language(language: &str) -> Option<String> {
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
fn pixel_buffer_error_to_app_error(error: cidre::os::Error) -> AppInfraError {
    AppInfraError::OcrEngine(format!("Apple Vision OCR pixel buffer setup failed: {error}"))
}

#[cfg(target_os = "macos")]
fn image_decode_error_to_app_error(error: image::ImageError) -> AppInfraError {
    AppInfraError::OcrEngine(format!("Apple Vision OCR image decode failed: {error}"))
}

#[cfg(target_os = "macos")]
pub(super) fn recognize(request: OcrRequest) -> Result<OcrOutput> {
    objc::ar_pool(|| recognize_impl(request).map_err(|error| error.to_string()))
        .map_err(AppInfraError::OcrEngine)
}

#[cfg(target_os = "macos")]
fn recognize_impl(request: OcrRequest) -> Result<OcrOutput> {
    let recognition_level = requested_recognition_mode(request.payload_json.as_deref())?;
    let language_correction = requested_language_correction(request.payload_json.as_deref())?;
    let recognition_langs = cached_recognition_langs(request.payload_json.as_deref())?;
    let prepared_image = PreparedVisionImage::from_path(&request.image_path)?;

    let mut vision_request = vn::RecognizeTextRequest::new();
    vision_request.set_recognition_level(recognition_level);
    vision_request.set_uses_lang_correction(language_correction);
    vision_request.set_recognition_langs(recognition_langs.as_ref());

    let requests = ns::Array::<vn::Request>::from_slice(&[&vision_request]);
    let handler = vn::ImageRequestHandler::with_cv_pixel_buf(prepared_image.pixel_buf.as_ref(), None)
        .ok_or_else(|| {
            AppInfraError::OcrEngine(
                "Apple Vision OCR failed to create image request handler".to_string(),
            )
        })?;

    handler
        .perform(&requests)
        .map_err(vision_error_to_app_error)?;

    let observations = vision_request
        .results()
        .map(|results| recognized_observations(&results))
        .unwrap_or_default();
    let text = join_observation_text(&observations);
    let structured_payload_json = serialize_structured_payload(observations)?;

    Ok(OcrOutput::new(text)
        .with_structured_payload_json(structured_payload_json)
        .with_engine_version(ns::ProcessInfo::current().os_version_string().to_string()))
}

#[cfg(target_os = "macos")]
fn recognized_observations(
    results: &ns::Array<vn::RecognizedTextObservation>,
) -> Vec<AppleVisionStructuredObservation> {
    results
        .iter()
        .filter_map(AppleVisionStructuredObservation::from_vision_observation)
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
fn join_observation_text(observations: &[AppleVisionStructuredObservation]) -> String {
    observations
        .iter()
        .map(|observation| observation.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(any(target_os = "macos", test))]
fn serialize_structured_payload(
    observations: Vec<AppleVisionStructuredObservation>,
) -> Result<String> {
    serde_json::to_string(&AppleVisionStructuredPayload::new(observations)).map_err(Into::into)
}

#[cfg(target_os = "macos")]
fn vision_error_to_app_error(error: &ns::Error) -> AppInfraError {
    AppInfraError::OcrEngine(format!("Apple Vision OCR failed: {error}"))
}

#[cfg(not(target_os = "macos"))]
pub(super) fn recognize(_request: OcrRequest) -> Result<OcrOutput> {
    Err(AppInfraError::OcrEngine(
        "Apple Vision OCR is only available on macOS".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn structured_payload_serializes_observations_with_coordinate_metadata() {
        let payload_json = serialize_structured_payload(vec![
            AppleVisionStructuredObservation::new(
                "Hello",
                0.5,
                AppleVisionBoundingBox::new(0.25, 0.5, 0.125, 0.25),
            ),
            AppleVisionStructuredObservation::new(
                "World",
                0.75,
                AppleVisionBoundingBox::new(0.5, 0.25, 0.25, 0.125),
            ),
        ])
        .expect("structured payload should serialize");

        let payload: serde_json::Value =
            serde_json::from_str(&payload_json).expect("payload should parse as json");

        assert_eq!(
            payload,
            json!({
                "schemaVersion": 1,
                "coordinateSpace": "normalized",
                "coordinateOrigin": "lower_left",
                "observations": [
                    {
                        "text": "Hello",
                        "confidence": 0.5,
                        "boundingBox": {
                            "x": 0.25,
                            "y": 0.5,
                            "width": 0.125,
                            "height": 0.25,
                        },
                    },
                    {
                        "text": "World",
                        "confidence": 0.75,
                        "boundingBox": {
                            "x": 0.5,
                            "y": 0.25,
                            "width": 0.25,
                            "height": 0.125,
                        },
                    },
                ],
            })
        );
    }

    #[test]
    fn candidate_text_normalization_trims_and_drops_empty_values() {
        assert_eq!(
            normalize_candidate_text("  hello world  "),
            Some("hello world".to_string())
        );
        assert_eq!(normalize_candidate_text("   \n\t  "), None);
    }

    #[test]
    fn requested_language_extracts_optional_language_hint() {
        assert_eq!(
            requested_language(Some("{\"language\":\"eng\"}"))
                .expect("payload should parse"),
            Some("en-US".to_string())
        );
        assert_eq!(
            requested_language(Some("{\"language\":\"fra\"}"))
                .expect("payload should parse"),
            Some("fr-FR".to_string())
        );
        assert_eq!(
            requested_language(Some("{\"language\":\"   \"}"))
                .expect("payload should parse"),
            None
        );
        assert_eq!(requested_language(None).expect("missing payload should succeed"), None);
    }

    #[test]
    fn requested_recognition_mode_defaults_to_fast() {
        assert_eq!(
            requested_recognition_mode(None).expect("missing payload should succeed"),
            vn::RequestTextRecognitionLevel::Fast
        );
        assert_eq!(
            requested_recognition_mode(Some("{\"recognitionMode\":\"accurate\"}"))
                .expect("payload should parse"),
            vn::RequestTextRecognitionLevel::Accurate
        );
    }

    #[test]
    fn requested_language_correction_defaults_to_false() {
        assert!(
            !requested_language_correction(None).expect("missing payload should succeed")
        );
        assert!(
            requested_language_correction(Some("{\"languageCorrection\":true}"))
                .expect("payload should parse")
        );
    }
}
