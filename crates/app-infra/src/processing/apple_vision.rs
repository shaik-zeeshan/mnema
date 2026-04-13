use super::{OcrOutput, OcrRequest};
use crate::{AppInfraError, Result};

#[cfg(target_os = "macos")]
use cidre::{cg, ns, objc, vn};

#[cfg(any(target_os = "macos", test))]
use serde::{Deserialize, Serialize};

#[cfg(any(target_os = "macos", test))]
const APPLE_VISION_PAYLOAD_SCHEMA_VERSION: u8 = 1;

#[cfg(any(target_os = "macos", test))]
const APPLE_VISION_COORDINATE_SPACE: &str = "normalized";

#[cfg(any(target_os = "macos", test))]
const APPLE_VISION_COORDINATE_ORIGIN: &str = "lower_left";

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
pub(super) fn recognize(request: OcrRequest) -> Result<OcrOutput> {
    let _pool = objc::AutoreleasePoolPage::push();
    recognize_impl(request)
}

#[cfg(target_os = "macos")]
fn recognize_impl(request: OcrRequest) -> Result<OcrOutput> {
    let image_path = request.image_path;
    let image_path_str = image_path.to_string_lossy().into_owned();
    let image_url = ns::Url::with_fs_path_str(&image_path_str, false);

    let mut vision_request = vn::RecognizeTextRequest::new();
    vision_request.set_recognition_level(vn::RequestTextRecognitionLevel::Accurate);
    vision_request.set_uses_lang_correction(true);

    let requests = ns::Array::<vn::Request>::from_slice(&[&vision_request]);
    let handler = vn::ImageRequestHandler::with_url(&image_url, None);

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
}
