//! `.timeline__bar`: the jumper (+ latest) on the left; the Recall landing chip,
//! the OCR toggle and refresh on the right. Buttons are 28 px.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, AnyElement, Context, SharedString};

use super::{jumper, OcrStatus, TimelineView};
use crate::theme::{size, text, Palette};
use crate::ui::chip::{chip, ChipKind};
use crate::ui::{with_tip, Button};

fn landing_chip(
    v: &TimelineView,
    p: &Palette,
    cx: &mut Context<TimelineView>,
) -> Option<AnyElement> {
    let l = v.landing.as_ref()?;
    let label = chip(format!("from Recall · “{}”", l.query), ChipKind::Accent, p)
        .id("landing-chip")
        .max_w(px(300.))
        // `.tl-searchchip__label span`: a long query ends in "…" (the tooltip has
        // it whole). In the chip's flex row the text keeps its full width and is
        // clipped mid-glyph; as a column item it takes the chip's width.
        .flex_col()
        .justify_center()
        .truncate();
    Some(
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .min_w_0()
            .child(with_tip(label, SharedString::from(l.query.clone())))
            .child(
                Button::icon_only("landing-dismiss", IconName::X.path(), "Dismiss")
                    .ghost()
                    .sm()
                    .tip("Dismiss")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.landing = None;
                        this.match_boxes.clear();
                        cx.notify();
                    })),
            )
            .into_any_element(),
    )
}

fn ocr_label(v: &TimelineView) -> &'static str {
    match (v.ocr.status, v.ocr.visible) {
        (OcrStatus::Running, _) => "loading OCR…",
        (_, true) => "hide OCR",
        (_, false) => "show OCR",
    }
}

pub fn render(v: &TimelineView, p: &Palette, cx: &mut Context<TimelineView>) -> impl IntoElement {
    let active = v.paging.active_frame().is_some();
    let busy = v.paging.loading || v.paging.loading_more || v.audio_loading;
    let ocr_here = v.ocr.visible && v.ocr.frame_id == v.paging.active_frame().map(|f| f.id);
    let count = (v.ocr.status == OcrStatus::Success && !v.ocr.boxes.is_empty())
        .then(|| super::ocr_overlay::count_label(v.ocr.boxes.len()));
    let ocr_toggle = div().flex().items_center().gap(px(6.)).child(
        Button::new("ocr-toggle", ocr_label(v))
            .ghost()
            .sm()
            .h(size::CMD_H)
            .icon(IconName::ScanText.path())
            .spinner(v.ocr.status == OcrStatus::Running)
            .pressed(v.ocr.visible)
            .disabled(!active)
            .aria(if v.ocr.visible {
                "Hide OCR data for active frame"
            } else {
                "Show OCR data for active frame"
            })
            .tip(super::ocr_overlay::toggle_tip(v))
            .when_some(count, |b, n| b.count(n))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_ocr(cx))),
    );
    div()
        .flex()
        .flex_none()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap(px(10.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .min_w_0()
                .child(jumper::trigger(v, p, cx))
                .when(jumper::show_latest(v), |d| {
                    d.child(jumper::latest_button("bar-latest", v, cx))
                }),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .ml_auto()
                .min_w_0()
                .children(landing_chip(v, p, cx))
                .when(ocr_here, |d| {
                    let o = &v.ocr;
                    let label = if o.rerun_loading {
                        "rerunning…"
                    } else if o.status == OcrStatus::Missing {
                        "run OCR"
                    } else {
                        "rerun OCR"
                    };
                    let tip = if !v.settings.ocr_enabled {
                        "OCR is off. Turn it on in Settings to run OCR again."
                    } else if o.status == OcrStatus::Running {
                        "OCR is queued or still processing"
                    } else if o.status == OcrStatus::Missing {
                        "Run OCR for the active frame with current settings"
                    } else {
                        "Rerun OCR for the active frame with current settings"
                    };
                    d.children(o.provider.clone().map(|label| {
                        with_tip(
                            div()
                                .id("ocr-provider")
                                .text_size(text::SM)
                                .text_color(p.text_subtle)
                                .child(label),
                            "OCR provider",
                        )
                        .test_support()
                    }))
                    .child(
                        Button::new("ocr-rerun", label)
                            .ghost()
                            .sm()
                            .h(size::CMD_H)
                            .disabled(v.rerun_disabled())
                            .tip(tip)
                            .on_click(cx.listener(|this, _, _, cx| this.rerun_ocr(cx))),
                    )
                })
                .child(ocr_toggle)
                .child(
                    Button::new("timeline-refresh", "refresh")
                        .ghost()
                        .sm()
                        .h(size::CMD_H)
                        .icon(IconName::RotateCw.path())
                        .disabled(busy)
                        .tip("Refresh dashboard timeline  R")
                        .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                ),
        )
}
