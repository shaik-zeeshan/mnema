//! `.timeline__bar`: the jumper (+ latest) on the left; the Recall landing chip,
//! the OCR toggle and refresh on the right. Buttons are 28 px.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, SharedString};

use super::{jumper, OcrStatus, TimelineView};
use crate::theme::{self, radius, size, text, Palette, FONT_MONO};
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
        .overflow_hidden();
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
    let count = (v.ocr.status == OcrStatus::Success && !v.ocr.boxes.is_empty()).then(|| {
        div()
            .px(px(5.))
            .py(px(3.))
            .rounded(radius::XS)
            .font_family(FONT_MONO)
            .text_size(text::XS)
            .font_weight(theme::W_MEDIUM)
            .text_color(p.accent)
            .bg(p.accent_bg)
            .child(v.ocr.boxes.len().to_string())
    });
    let ocr_toggle = div().flex().items_center().gap(px(6.)).child(
        Button::new("ocr-toggle", ocr_label(v))
            .ghost()
            .sm()
            .h(size::CMD_H)
            .icon(IconName::ScanText.path())
            .spinner(v.ocr.status == OcrStatus::Running)
            .pressed(v.ocr.visible)
            .disabled(!active)
            .tip("Show recognized text for this frame  O")
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
                    d.child(
                        Button::new("ocr-rerun", "rerun OCR")
                            .ghost()
                            .sm()
                            .h(size::CMD_H)
                            .disabled(v.ocr.status == OcrStatus::Running)
                            .tip("Rerun OCR for the active frame with current settings")
                            .on_click(cx.listener(|this, _, _, cx| this.rerun_ocr(cx))),
                    )
                })
                .child(ocr_toggle.children(count))
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
