use std::path::{Path, PathBuf};

use image::{imageops::FilterType, GrayImage, Luma};
use serde::{de::DeserializeOwned, Deserialize};

#[cfg(feature = "tesseract-embedded")]
use tesseract_rs::{TessPageSegMode, TesseractAPI};

use crate::{
    normalize_candidate_text, OcrBoundingBox, OcrError, OcrObservation, OcrOutput, OcrRequest,
    OcrResult, OcrStructuredPayload, DEFAULT_TESSERACT_LANGUAGE, DEFAULT_TESSERACT_MODEL_ID,
    TESSERACT_PROVIDER_ID,
};

const MODEL_PATH_OPTION: &str = "modelPath";
const DEFAULT_TESSDATA_DIR: &str = "tessdata";
const EMBEDDED_TESSERACT_PROVIDER_VERSION: &str = "tesseract-rs 0.2.0";
const PAGE_SEGMENTATION_MODE_OPTION: &str = "pageSegmentationMode";
const PREPROCESS_MODE_OPTION: &str = "preprocessMode";
const UPSCALE_FACTOR_OPTION: &str = "upscaleFactor";
const CHAR_WHITELIST_OPTION: &str = "charWhitelist";
const MAX_PREPROCESSED_LONG_EDGE: u32 = 2400;

#[derive(Debug, Clone)]
pub struct TesseractModelSelection {
    pub model_id: String,
    pub model_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct TesseractRuntimeLayout {
    pub tessdata_dir: PathBuf,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TesseractPageSegmentationMode {
    Auto,
    SingleBlock,
    SingleLine,
    SingleWord,
    SparseText,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TesseractPreprocessMode {
    Grayscale,
    Thresholded,
}

pub(crate) fn runtime_available() -> bool {
    cfg!(feature = "tesseract-embedded")
}

pub(crate) fn recognize(
    configured_models_dir: Option<&Path>,
    request: OcrRequest,
) -> OcrResult<OcrOutput> {
    let selection = resolve_model_selection(&request, configured_models_dir)?;
    let layout = runtime_layout(&selection.model_path)?;

    #[cfg(feature = "tesseract-embedded")]
    {
        run_tesseract_embedded(request, selection, layout)
    }

    #[cfg(not(feature = "tesseract-embedded"))]
    {
        let _ = (request, selection, layout);
        Err(OcrError::Provider(
            "embedded Tesseract runtime is not enabled in this build".to_string(),
        ))
    }
}

fn resolve_model_selection(
    request: &OcrRequest,
    configured_models_dir: Option<&Path>,
) -> OcrResult<TesseractModelSelection> {
    if request.provider != TESSERACT_PROVIDER_ID {
        return Err(OcrError::Provider(format!(
            "Tesseract provider received request for {}",
            request.provider
        )));
    }

    let model_id = request
        .model_id
        .clone()
        .unwrap_or_else(|| DEFAULT_TESSERACT_MODEL_ID.to_string());
    if model_id != DEFAULT_TESSERACT_MODEL_ID {
        return Err(OcrError::Provider(format!(
            "unsupported Tesseract model id {model_id}"
        )));
    }

    let model_path = request
        .options
        .get(MODEL_PATH_OPTION)
        .and_then(serde_json::Value::as_str)
        .map(PathBuf::from)
        .or_else(|| {
            configured_models_dir.map(|dir| dir.join(TESSERACT_PROVIDER_ID).join(&model_id))
        })
        .ok_or_else(|| {
            OcrError::Provider(
                "Tesseract needs either a configured models directory or a modelPath request option"
                    .to_string(),
            )
        })?;

    Ok(TesseractModelSelection {
        model_id,
        model_path,
    })
}

fn runtime_layout(model_path: &Path) -> OcrResult<TesseractRuntimeLayout> {
    let bundle_dir = if model_path.is_dir() {
        model_path.to_path_buf()
    } else {
        model_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| model_path.to_path_buf())
    };

    let tessdata_dir = if bundle_dir.join(DEFAULT_TESSDATA_DIR).is_dir() {
        bundle_dir.join(DEFAULT_TESSDATA_DIR)
    } else {
        bundle_dir.clone()
    };
    if !tessdata_dir.is_dir() {
        return Err(OcrError::Provider(format!(
            "Tesseract tessdata directory is missing at {}",
            tessdata_dir.display()
        )));
    }

    Ok(TesseractRuntimeLayout { tessdata_dir })
}

#[cfg(feature = "tesseract-embedded")]
fn run_tesseract_embedded(
    request: OcrRequest,
    selection: TesseractModelSelection,
    layout: TesseractRuntimeLayout,
) -> OcrResult<OcrOutput> {
    if !request.image_path.is_file() {
        return Err(OcrError::Provider(format!(
            "image file does not exist: {}",
            request.image_path.display()
        )));
    }

    let language = request
        .language
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_TESSERACT_LANGUAGE);
    let page_segmentation_mode = page_segmentation_mode_for_request(&request)?;
    let preprocess_mode = preprocess_mode_for_request(&request)?;
    let upscale_factor = upscale_factor_for_request(&request)?;
    let char_whitelist = char_whitelist_for_request(&request)?;

    let image = image::open(&request.image_path).map_err(|error| {
        OcrError::Provider(format!(
            "failed to open OCR image {}: {error}",
            request.image_path.display()
        ))
    })?;
    let preprocessed =
        preprocess_tesseract_image(&image.to_luma8(), upscale_factor, preprocess_mode);
    let image_width = preprocessed.width();
    let image_height = preprocessed.height();

    let api = TesseractAPI::new();
    api.init(&layout.tessdata_dir, language)
        .map_err(map_tesseract_error)?;
    api.set_page_seg_mode(page_segmentation_mode)
        .map_err(map_tesseract_error)?;
    if let Some(whitelist) = char_whitelist {
        api.set_variable("tessedit_char_whitelist", &whitelist)
            .map_err(map_tesseract_error)?;
    }
    api.set_image(
        preprocessed.as_raw(),
        image_width as i32,
        image_height as i32,
        1,
        image_width as i32,
    )
    .map_err(map_tesseract_error)?;
    api.set_source_resolution(300)
        .map_err(map_tesseract_error)?;
    api.recognize().map_err(map_tesseract_error)?;

    let text = api.get_utf8_text().map_err(map_tesseract_error)?;
    let observations = collect_word_observations(&api, image_width, image_height)?;
    let structured_payload = OcrStructuredPayload::new(
        TESSERACT_PROVIDER_ID,
        Some(selection.model_id.clone()),
        observations,
    );

    let provider_version = format!(
        "{} ({})",
        EMBEDDED_TESSERACT_PROVIDER_VERSION,
        TesseractAPI::version()
    );
    Ok(OcrOutput::new(text.trim().to_string(), structured_payload)
        .with_provider_version(provider_version))
}

#[cfg(feature = "tesseract-embedded")]
fn collect_word_observations(
    api: &TesseractAPI,
    image_width: u32,
    image_height: u32,
) -> OcrResult<Vec<OcrObservation>> {
    let iterator = api.get_iterator().map_err(map_tesseract_error)?;
    let mut observations = Vec::new();
    let mut is_first = true;

    loop {
        match iterator.get_current_word() {
            Ok((text, left, top, right, bottom, confidence)) => {
                if let Some(text) = normalize_candidate_text(&text) {
                    observations.push(OcrObservation::new(
                        text,
                        (confidence / 100.0).clamp(0.0, 1.0),
                        normalize_tesseract_box(
                            left.max(0) as u32,
                            top.max(0) as u32,
                            right.max(left) as u32,
                            bottom.max(top) as u32,
                            image_width,
                            image_height,
                        ),
                    ));
                }
            }
            Err(error) if is_first => {
                let _ = error;
                break;
            }
            Err(error) => return Err(map_tesseract_error(error)),
        }

        is_first = false;
        if !iterator.next_word().map_err(map_tesseract_error)? {
            break;
        }
    }

    Ok(observations)
}

#[cfg(feature = "tesseract-embedded")]
fn page_segmentation_mode_for_request(request: &OcrRequest) -> OcrResult<TessPageSegMode> {
    let mode =
        request_option::<TesseractPageSegmentationMode>(request, PAGE_SEGMENTATION_MODE_OPTION)?
            .unwrap_or(TesseractPageSegmentationMode::SingleBlock);
    Ok(match mode {
        TesseractPageSegmentationMode::Auto => TessPageSegMode::PSM_AUTO,
        TesseractPageSegmentationMode::SingleBlock => TessPageSegMode::PSM_SINGLE_BLOCK,
        TesseractPageSegmentationMode::SingleLine => TessPageSegMode::PSM_SINGLE_LINE,
        TesseractPageSegmentationMode::SingleWord => TessPageSegMode::PSM_SINGLE_WORD,
        TesseractPageSegmentationMode::SparseText => TessPageSegMode::PSM_SPARSE_TEXT,
    })
}

fn preprocess_mode_for_request(request: &OcrRequest) -> OcrResult<TesseractPreprocessMode> {
    Ok(
        request_option::<TesseractPreprocessMode>(request, PREPROCESS_MODE_OPTION)?
            .unwrap_or(TesseractPreprocessMode::Grayscale),
    )
}

fn upscale_factor_for_request(request: &OcrRequest) -> OcrResult<u8> {
    let factor = request
        .options
        .get(UPSCALE_FACTOR_OPTION)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(1);
    if !(1..=4).contains(&factor) {
        return Err(OcrError::Provider(format!(
            "Tesseract upscaleFactor must be between 1 and 4, got {factor}"
        )));
    }
    Ok(factor as u8)
}

fn char_whitelist_for_request(request: &OcrRequest) -> OcrResult<Option<String>> {
    let Some(value) = request.options.get(CHAR_WHITELIST_OPTION) else {
        return Ok(None);
    };
    let whitelist = value.as_str().ok_or_else(|| {
        OcrError::Provider("Tesseract charWhitelist option must be a string".to_string())
    })?;
    Ok(normalize_candidate_text(whitelist))
}

fn request_option<T: DeserializeOwned>(request: &OcrRequest, key: &str) -> OcrResult<Option<T>> {
    request
        .options
        .get(key)
        .cloned()
        .map(|value| {
            serde_json::from_value(value).map_err(|error| {
                OcrError::Provider(format!("failed to parse Tesseract option {key}: {error}"))
            })
        })
        .transpose()
}

fn preprocess_tesseract_image(
    source: &GrayImage,
    upscale_factor: u8,
    preprocess_mode: TesseractPreprocessMode,
) -> GrayImage {
    let (target_width, target_height) =
        bounded_preprocessed_dimensions(source.width(), source.height(), upscale_factor);
    let scaled = if target_width != source.width() || target_height != source.height() {
        image::imageops::resize(source, target_width, target_height, FilterType::CatmullRom)
    } else {
        source.clone()
    };

    match preprocess_mode {
        TesseractPreprocessMode::Grayscale => scaled,
        TesseractPreprocessMode::Thresholded => otsu_threshold(&scaled),
    }
}

fn bounded_preprocessed_dimensions(width: u32, height: u32, upscale_factor: u8) -> (u32, u32) {
    let target_width = width.saturating_mul(upscale_factor as u32).max(1);
    let target_height = height.saturating_mul(upscale_factor as u32).max(1);
    let longest = target_width.max(target_height);
    if longest <= MAX_PREPROCESSED_LONG_EDGE {
        return (target_width, target_height);
    }

    let scale = MAX_PREPROCESSED_LONG_EDGE as f64 / longest as f64;
    (
        ((target_width as f64 * scale).round() as u32).max(1),
        ((target_height as f64 * scale).round() as u32).max(1),
    )
}

fn otsu_threshold(source: &GrayImage) -> GrayImage {
    let mut histogram = [0u32; 256];
    for pixel in source.pixels() {
        histogram[pixel[0] as usize] = histogram[pixel[0] as usize].saturating_add(1);
    }

    let total = source.width().saturating_mul(source.height()) as f64;
    let mut sum = 0.0_f64;
    for (index, count) in histogram.iter().enumerate() {
        sum += index as f64 * *count as f64;
    }

    let mut sum_background = 0.0_f64;
    let mut weight_background = 0.0_f64;
    let mut best_threshold = 0u8;
    let mut best_variance = -1.0_f64;

    for (index, count) in histogram.iter().enumerate() {
        weight_background += *count as f64;
        if weight_background <= 0.0 {
            continue;
        }
        let weight_foreground = total - weight_background;
        if weight_foreground <= 0.0 {
            break;
        }
        sum_background += index as f64 * *count as f64;
        let mean_background = sum_background / weight_background;
        let mean_foreground = (sum - sum_background) / weight_foreground;
        let between = weight_background
            * weight_foreground
            * (mean_background - mean_foreground)
            * (mean_background - mean_foreground);
        if between > best_variance {
            best_variance = between;
            best_threshold = index as u8;
        }
    }

    let mut output = GrayImage::new(source.width(), source.height());
    for (x, y, pixel) in source.enumerate_pixels() {
        let value = if pixel[0] > best_threshold { 255 } else { 0 };
        output.put_pixel(x, y, Luma([value]));
    }
    output
}

#[cfg(feature = "tesseract-embedded")]
fn map_tesseract_error(error: tesseract_rs::TesseractError) -> OcrError {
    OcrError::Provider(error.to_string())
}

fn normalize_tesseract_box(
    left: u32,
    top: u32,
    right: u32,
    bottom: u32,
    image_width: u32,
    image_height: u32,
) -> OcrBoundingBox {
    let image_width = image_width.max(1) as f64;
    let image_height = image_height.max(1) as f64;
    let width = right.saturating_sub(left) as f64 / image_width;
    let height = bottom.saturating_sub(top) as f64 / image_height;
    let x = left as f64 / image_width;
    let y = 1.0 - (bottom as f64 / image_height);
    OcrBoundingBox::new(x, y.clamp(0.0, 1.0), width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_nested_tessdata_dir() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bundle_dir = temp.path().join("bundle");
        std::fs::create_dir_all(bundle_dir.join("tessdata")).expect("mkdir");
        let layout = runtime_layout(&bundle_dir).expect("layout");
        assert_eq!(layout.tessdata_dir, bundle_dir.join("tessdata"));
    }

    #[test]
    fn falls_back_to_bundle_dir_when_it_is_tessdata_dir() {
        let temp = tempfile::tempdir().expect("tempdir");
        let layout = runtime_layout(temp.path()).expect("layout");
        assert_eq!(layout.tessdata_dir, temp.path());
    }

    #[test]
    fn parses_tesseract_advanced_options() {
        let mut request = OcrRequest::new("/tmp/example.png", TESSERACT_PROVIDER_ID);
        request.options.insert(
            PAGE_SEGMENTATION_MODE_OPTION.to_string(),
            serde_json::Value::String("sparse_text".to_string()),
        );
        request.options.insert(
            PREPROCESS_MODE_OPTION.to_string(),
            serde_json::Value::String("thresholded".to_string()),
        );
        request.options.insert(
            UPSCALE_FACTOR_OPTION.to_string(),
            serde_json::Value::Number(serde_json::Number::from(2_u8)),
        );
        request.options.insert(
            CHAR_WHITELIST_OPTION.to_string(),
            serde_json::Value::String(" ABC123 ".to_string()),
        );

        assert_eq!(
            preprocess_mode_for_request(&request).expect("preprocess"),
            TesseractPreprocessMode::Thresholded
        );
        assert_eq!(upscale_factor_for_request(&request).expect("upscale"), 2);
        assert_eq!(
            char_whitelist_for_request(&request).expect("whitelist"),
            Some("ABC123".to_string())
        );
    }

    #[test]
    fn thresholding_produces_binary_pixels() {
        let image = GrayImage::from_fn(2, 2, |x, _| Luma([if x == 0 { 20 } else { 240 }]));
        let thresholded = otsu_threshold(&image);
        let values = thresholded
            .pixels()
            .map(|pixel| pixel[0])
            .collect::<Vec<_>>();
        assert!(values.iter().all(|value| *value == 0 || *value == 255));
    }

    #[test]
    fn preprocessing_honors_upscale_for_small_images() {
        let (width, height) = bounded_preprocessed_dimensions(320, 180, 2);

        assert_eq!((width, height), (640, 360));
    }

    #[test]
    fn preprocessing_caps_large_upscaled_screenshots() {
        let (width, height) = bounded_preprocessed_dimensions(1920, 1080, 4);

        assert_eq!(width, MAX_PREPROCESSED_LONG_EDGE);
        assert_eq!(height, 1350);
    }

    #[test]
    fn preprocessing_caps_native_large_screenshots() {
        let (width, height) = bounded_preprocessed_dimensions(3840, 2160, 1);

        assert_eq!(width, MAX_PREPROCESSED_LONG_EDGE);
        assert_eq!(height, 1350);
    }
}
