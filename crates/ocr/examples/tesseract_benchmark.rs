#[path = "common/mod.rs"]
mod common;

use std::{error::Error, path::PathBuf};

use ocr::{
    OcrRequest, TesseractProvider, DEFAULT_TESSERACT_LANGUAGE, DEFAULT_TESSERACT_MODEL_ID,
    TESSERACT_PROVIDER_ID,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = common::parse_args("tesseract_benchmark")?;
    let models_dir = if args.model_path.is_some() {
        PathBuf::from(".")
    } else {
        common::require_model_root(&args, "Tesseract")?
    };
    let provider = TesseractProvider::with_models_dir(models_dir);

    common::run_provider_benchmark(&provider, &args, |image_path| {
        let mut request = OcrRequest::new(image_path, TESSERACT_PROVIDER_ID);
        request.model_id = Some(DEFAULT_TESSERACT_MODEL_ID.to_string());
        request.language = Some(DEFAULT_TESSERACT_LANGUAGE.to_string());
        request.options.insert(
            "pageSegmentationMode".to_string(),
            serde_json::Value::String("sparse_text".to_string()),
        );
        request.options.insert(
            "preprocessMode".to_string(),
            serde_json::Value::String("grayscale".to_string()),
        );
        request.options.insert(
            "upscaleFactor".to_string(),
            serde_json::Value::Number(1.into()),
        );
        if let Some(model_path) = common::model_path_option(&args) {
            request.options.insert("modelPath".to_string(), model_path);
        }
        common::apply_common_request_options(&args, &mut request);
        request
    })
    .await
}
