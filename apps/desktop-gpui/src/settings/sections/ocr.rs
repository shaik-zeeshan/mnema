//! Intelligence › OCR & Previews (`Ocr.svelte`): engine, provider-specific
//! options, the model row, delete unused and the preview cache.

use capture_types::{
    OcrProvider, OcrRecognitionMode, OcrTesseractPageSegmentationMode, OcrTesseractPreprocessMode,
};
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context};

use super::model_row::{delete_unused_row, model_files, no_status_row, status_row, ModelRow};
use super::{ix, languages};
use crate::settings::data::Family;
use crate::settings::model::{format_bytes, selected_model};
use crate::settings::row::{row, section};
use crate::settings::{on, reload, Settings};
use crate::theme::Palette;
use crate::ui::chip::ChipKind;
use crate::ui::combobox::Combobox;
use crate::ui::input::TextInput;
use crate::ui::segmented::Segmented;
use crate::ui::select::Select;
use crate::ui::switch::Switch;

const PROVIDERS: [OcrProvider; 2] = [OcrProvider::AppleVision, OcrProvider::Tesseract];
const MODES: [OcrRecognitionMode; 2] = [OcrRecognitionMode::Fast, OcrRecognitionMode::Accurate];
const PSM: [OcrTesseractPageSegmentationMode; 5] = [
    OcrTesseractPageSegmentationMode::Auto,
    OcrTesseractPageSegmentationMode::SingleBlock,
    OcrTesseractPageSegmentationMode::SingleLine,
    OcrTesseractPageSegmentationMode::SingleWord,
    OcrTesseractPageSegmentationMode::SparseText,
];
const PRE: [OcrTesseractPreprocessMode; 2] = [
    OcrTesseractPreprocessMode::Grayscale,
    OcrTesseractPreprocessMode::Thresholded,
];
const TTL: [(u64, &str); 6] = [
    (0, "Disabled"),
    (300, "5 minutes"),
    (900, "15 minutes"),
    (3600, "1 hour (default)"),
    (21600, "6 hours"),
    (86400, "24 hours"),
];

fn provider_id(p: OcrProvider) -> &'static str {
    match p {
        OcrProvider::AppleVision => "apple_vision",
        OcrProvider::Tesseract => "tesseract",
        OcrProvider::PaddleOcr => "paddle_ocr",
    }
}

pub fn render(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let d = s.d();
    let o = &d.ocr;
    let off = !o.enabled;
    let mut rows = vec![
        row("Enable OCR")
            .desc("Queue OCR for captured screen frames when the selected engine is available. While on, recording waits for the engine.")
            .control(Switch::new("ocr-enabled", "Enable OCR", o.enabled).on_toggle(on(cx, |s, v: bool, cx| s.edit(|d| d.ocr.enabled = v, cx))))
            .build(p)
            .into_any_element(),
        row("Provider")
            .desc("Existing OCR results stay visible after switching.")
            .control(
                Segmented::new("ocr-provider", "OCR provider", ["Apple Vision", "Tesseract"], ix(&PROVIDERS, &o.provider))
                    .md()
                    .disabled(off)
                    .on_change(on(cx, |s, i: usize, cx| {
                        s.edit(
                            |d| {
                                d.ocr.provider = PROVIDERS[i];
                                d.ocr.model_id = None;
                            },
                            cx,
                        )
                    })),
            )
            .build(p)
            .into_any_element(),
    ];
    if o.provider == OcrProvider::AppleVision {
        rows.push(
            row("Recognition mode")
                .control(
                    Segmented::new(
                        "ocr-mode",
                        "Recognition mode",
                        ["Fast", "Accurate"],
                        ix(&MODES, &o.recognition_mode),
                    )
                    .md()
                    .disabled(off)
                    .on_change(on(cx, |s, i: usize, cx| {
                        s.edit(|d| d.ocr.recognition_mode = MODES[i].clone(), cx)
                    })),
                )
                .build(p)
                .into_any_element(),
        );
        rows.push(
            row("Language correction")
                .control(
                    Switch::new(
                        "ocr-correction",
                        "Language correction",
                        o.language_correction,
                    )
                    .disabled(off)
                    .on_toggle(on(cx, |s, v: bool, cx| {
                        s.edit(|d| d.ocr.language_correction = v, cx)
                    })),
                )
                .build(p)
                .into_any_element(),
        );
    } else if o.provider == OcrProvider::Tesseract {
        let stored = o
            .language
            .as_deref()
            .filter(|l| !l.is_empty())
            .unwrap_or("eng");
        let (sel, shown) = languages::pick(&languages::OCR, stored);
        rows.push(
            row("Language")
                .control(
                    div().w(px(240.)).child(
                        Combobox::new(
                            "ocr-language",
                            "OCR language",
                            languages::OCR.iter().map(|(_, l)| *l),
                            sel,
                        )
                        .placeholder(shown)
                        .search_placeholder("Search languages\u{2026}")
                        .disabled(off)
                        .on_change(on(cx, |s, i: usize, cx| {
                            s.edit(|d| d.ocr.language = Some(languages::OCR[i].0.into()), cx)
                        })),
                    ),
                )
                .build(p)
                .into_any_element(),
        );
        rows.push(
            row("Page segmentation")
                .desc("Auto for mixed layouts; Sparse text for screenshots with scattered text.")
                .control(
                    div().w(px(200.)).child(
                        Select::new(
                            "ocr-psm",
                            "Page segmentation",
                            [
                                "Auto",
                                "Single block",
                                "Single line",
                                "Single word",
                                "Sparse text",
                            ],
                            Some(ix(&PSM, &o.tesseract_page_segmentation_mode)),
                        )
                        .disabled(off)
                        .on_change(on(cx, |s, i: usize, cx| {
                            s.edit(
                                |d| d.ocr.tesseract_page_segmentation_mode = PSM[i].clone(),
                                cx,
                            )
                        })),
                    ),
                )
                .build(p)
                .into_any_element(),
        );
        rows.push(
            row("Image preprocessing")
                .desc("Grayscale suits clean UI text. Thresholded black/white helps when edges are muddy or contrast is weak.")
                .control(
                    Segmented::new("ocr-pre", "Image preprocessing", ["Grayscale", "Thresholded"], ix(&PRE, &o.tesseract_preprocess_mode))
                        .md()
                        .disabled(off)
                        .on_change(on(cx, |s, i: usize, cx| s.edit(|d| d.ocr.tesseract_preprocess_mode = PRE[i].clone(), cx))),
                )
                .build(p)
                .into_any_element(),
        );
        rows.push(
            row("Upscale before OCR")
                .desc("Tesseract works best around 300 DPI. For tiny text try 2\u{d7} before 3\u{d7} or 4\u{d7}.")
                .control(
                    Segmented::new("ocr-upscale", "Upscale before OCR", ["1\u{d7}", "2\u{d7}", "3\u{d7}", "4\u{d7}"], (o.tesseract_upscale_factor.clamp(1, 4) - 1) as usize)
                        .md()
                        .mono()
                        .disabled(off)
                        .on_change(on(cx, |s, i: usize, cx| s.edit(|d| d.ocr.tesseract_upscale_factor = i as u8 + 1, cx))),
                )
                .build(p)
                .into_any_element(),
        );
        rows.push(
            row("Character whitelist")
                .desc("Optional. Only these characters are recognised.")
                .control(
                    TextInput::new(
                        "ocr-whitelist",
                        "Tesseract character whitelist",
                        &s.inputs.whitelist,
                    )
                    .num()
                    .disabled(off)
                    .w(px(240.)),
                )
                .build(p)
                .into_any_element(),
        );
    }
    let status = s.data.ocr_models.as_ref();
    let m = selected_model(status, provider_id(o.provider), o.model_id.as_deref());
    let folder = status
        .and_then(|v| v["modelsDirectory"].as_str())
        .map(str::to_string);
    let model = match &m {
        None => no_status_row(
            s,
            Family::Ocr,
            "Model status",
            "Couldn\u{2019}t load OCR model status: ",
            "Checking installed OCR models\u{2026}",
            "No OCR model status is available for the selected provider.",
        ),
        Some(m) if m.os_managed => ModelRow {
            label: "Model status",
            desc: Some("Managed by macOS \u{2014} nothing to download.".into()),
            status: m.available.then_some("available"),
            chip: (!m.available).then(|| ("unavailable".into(), ChipKind::Warn)),
            warning: m.runtime_message.clone(),
            files: model_files(m),
            ..Default::default()
        },
        Some(m) => {
            let failed = m.status == "failed";
            let desc = if !m.available && !failed && m.missing_files > 0 {
                format!(
                    "{} {} missing \u{2014} download to complete this model.",
                    m.missing_files,
                    if m.missing_files == 1 {
                        "file"
                    } else {
                        "files"
                    }
                )
            } else {
                // `ocrStatusLabel`, lower-cased.
                let word = if m.available {
                    "installed"
                } else if failed {
                    "not installed"
                } else {
                    match m.status.as_str() {
                        "os_managed" => "os managed",
                        "installed" => "installed",
                        "downloading" => "downloading",
                        _ => "missing",
                    }
                };
                let size = m
                    .download_bytes
                    .map(|b| format!(" \u{b7} {}", format_bytes(b as f64)))
                    .unwrap_or_default();
                format!("{} \u{b7} {word}{size}", m.display_name)
            };
            let mut r = status_row("Model status", m, failed, "available");
            r.desc = Some(desc);
            // OCR's chip: "missing" only for a missing model, else "unavailable".
            if !m.available && !failed {
                r.chip = Some((
                    if m.status == "missing" {
                        "missing"
                    } else {
                        "unavailable"
                    }
                    .into(),
                    ChipKind::Warn,
                ));
            }
            r.warning = m.runtime_message.clone().or_else(|| {
                (!m.available && m.download_bytes.is_none()).then(|| {
                    if m.provider == "tesseract" {
                        "This provider still needs a Mnema-published self-contained runtime bundle before in-app download can work."
                    } else {
                        "This app-managed OCR bundle is missing, and the current manifest does not ship a downloadable artifact yet."
                    }
                    .to_string()
                })
            });
            r.folder = folder.clone();
            r.name = Some(m.display_name.clone());
            r.start = m
                .model_id
                .clone()
                .map(|id| (Family::Ocr, m.provider.clone(), id));
            r
        }
    };
    rows.push(model.build("ocr-model", s, p, cx));
    if m.is_some() {
        rows.push(delete_unused_row(
            "ocr-delete-unused",
            Family::Ocr,
            "Removes app-managed OCR model files except the selected one.",
            p,
            cx,
        ));
    }
    let ttl = d.preview_cache_ttl_seconds;
    rows.push(
        row("Cache duration")
            .desc(if ttl == 0 {
                "Disabled \u{2014} previews are fetched fresh every time."
            } else {
                "In-memory cache for frame and image previews."
            })
            .control(
                div().w(px(200.)).child(
                    Select::new(
                        "preview-ttl",
                        "Cache duration",
                        TTL.iter().map(|(_, l)| *l),
                        TTL.iter().position(|(v, _)| *v == ttl),
                    )
                    .on_change(on(cx, |s, i: usize, cx| {
                        s.edit(|d| d.preview_cache_ttl_seconds = TTL[i].0, cx)
                    })),
                ),
            )
            .build(p)
            .into_any_element(),
    );
    section(
        "OCR & Previews",
        Some("Engine, model and preview caching."),
        vec![reload::button(
            s,
            "reload-ocr",
            "Refresh OCR model status",
            s.ui.models_loading,
            |s, cx| s.reload_all(cx),
            cx,
        )],
        false,
        rows,
        p,
    )
    .into_any_element()
}
