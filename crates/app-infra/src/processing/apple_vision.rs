use super::{OcrOutput, OcrRequest};
use crate::{AppInfraError, Result};

#[cfg(target_os = "macos")]
use cidre::{ns, objc, vn};

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

    let text = vision_request
        .results()
        .map(|results| join_recognized_text(&results))
        .unwrap_or_default();

    Ok(OcrOutput::new(text)
        .with_engine_version(ns::ProcessInfo::current().os_version_string().to_string()))
}

#[cfg(target_os = "macos")]
fn join_recognized_text(results: &ns::Array<vn::RecognizedTextObservation>) -> String {
    results
        .iter()
        .filter_map(|observation| {
            observation
                .top_candidates(1)
                .first()
                .map(|candidate| candidate.string())
        })
        .map(|candidate| candidate.to_string())
        .map(|candidate| candidate.trim().to_owned())
        .filter(|candidate| !candidate.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
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
