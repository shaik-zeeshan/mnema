#[path = "common/mod.rs"]
mod common;

#[cfg(target_os = "macos")]
use std::error::Error;

#[cfg(target_os = "macos")]
use ocr::{AppleVisionProvider, OcrRecognitionMode, OcrRequest, APPLE_VISION_PROVIDER_ID};

#[cfg(target_os = "macos")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = common::parse_args("apple_vision_benchmark")?;
    let provider = AppleVisionProvider::new();

    common::run_provider_benchmark(&provider, &args, |image_path| {
        let mut request = OcrRequest::new(image_path, APPLE_VISION_PROVIDER_ID);
        request.language = Some("en-US".to_string());
        request.options.insert(
            "recognitionMode".to_string(),
            serde_json::to_value(OcrRecognitionMode::Accurate).expect("serialize recognition mode"),
        );
        request.options.insert(
            "languageCorrection".to_string(),
            serde_json::Value::Bool(false),
        );
        common::apply_common_request_options(&args, &mut request);
        request
    })
    .await
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("Apple Vision OCR benchmark is only available on macOS.");
    std::process::exit(1);
}
