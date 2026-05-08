#[path = "common/mod.rs"]
mod common;

use std::{error::Error, path::PathBuf};

use ocr::{
    OcrRequest, PaddleOcrProvider, DEFAULT_PADDLE_OCR_LANGUAGE, DEFAULT_PADDLE_OCR_MODEL_ID,
    PADDLE_OCR_PROVIDER_ID,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = common::parse_args("paddle_ocr_benchmark")?;
    let models_dir = if args.model_path.is_some() {
        PathBuf::from(".")
    } else {
        common::require_model_root(&args, "PaddleOCR")?
    };
    let provider = PaddleOcrProvider::with_models_dir(models_dir);

    common::run_provider_benchmark(&provider, &args, |image_path| {
        let mut request = OcrRequest::new(image_path, PADDLE_OCR_PROVIDER_ID);
        request.model_id = Some(DEFAULT_PADDLE_OCR_MODEL_ID.to_string());
        request.language = Some(DEFAULT_PADDLE_OCR_LANGUAGE.to_string());
        if let Some(model_path) = common::model_path_option(&args) {
            request.options.insert("modelPath".to_string(), model_path);
        }
        common::apply_common_request_options(&args, &mut request);
        request
    })
    .await
}
