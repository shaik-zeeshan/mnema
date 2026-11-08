//! The OCR overlay (`.timeline__ocr-overlay`): one box per recognised line over
//! the painted image rect; only the hovered box shows its text chip (a chip in
//! every box was the Svelte page's measured slowdown). Plus the O toggle.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Bounds, Context, Pixels, Role, SharedString};

use super::{OcrStatus, TimelineView};
use crate::theme::{radius, text, Palette, FONT_MONO};
use crate::ui::{alpha, nid};

/// `MAX_OCR_BOXES` (lib/frame-ocr.ts): the overlay draws at most this many.
pub const MAX_BOXES: usize = 2000;

impl TimelineView {
    /// O: show or hide the active frame's OCR (loading it the first time).
    pub fn toggle_ocr(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.paging.active_frame().map(|f| f.id) else {
            return;
        };
        if self.ocr.visible {
            self.ocr.visible = false;
        } else if self.ocr.frame_id == Some(id) && self.ocr.status != OcrStatus::Idle {
            self.ocr.visible = true;
        } else {
            self.ocr.visible = true;
            self.load_ocr(cx);
        }
        cx.notify();
    }

    pub(super) fn load_ocr(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.paging.active_frame().map(|f| f.id) else {
            return;
        };
        if self.ocr.frame_id == Some(id) {
            return;
        }
        self.ocr.frame_id = Some(id);
        self.ocr.status = OcrStatus::Running;
        self.ocr.boxes.clear();
        self.ocr.hovered = None;
        let src = self.source.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move { src.ocr(id) })
                .await;
            this.update(cx, |view, cx| {
                if view.ocr.frame_id != Some(id) {
                    return; // the playhead moved on
                }
                (view.ocr.status, view.ocr.boxes, view.ocr.error) = match res {
                    Ok(None) => (OcrStatus::Missing, Vec::new(), None),
                    Ok(Some(b)) if b.is_empty() => (OcrStatus::Empty, b, None),
                    Ok(Some(b)) => (OcrStatus::Success, b, None),
                    Err(e) => (OcrStatus::Error, Vec::new(), Some(e)),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

pub fn render(
    v: &TimelineView,
    rect: Option<Bounds<Pixels>>,
    p: &Palette,
    cx: &mut Context<TimelineView>,
) -> Option<AnyElement> {
    let rect = rect?;
    let here = v.paging.active_frame().map(|f| f.id) == v.ocr.frame_id;
    if !(v.ocr.visible && here && v.ocr.status == OcrStatus::Success && !v.ocr.boxes.is_empty()) {
        return None;
    }
    let (w, h) = (f32::from(rect.size.width), f32::from(rect.size.height));
    let boxes = v
        .ocr
        .boxes
        .iter()
        .take(MAX_BOXES)
        .enumerate()
        .map(|(i, b)| {
            let hot = v.ocr.hovered == Some(i);
            let box_h = b.h * h;
            div()
                .id(nid("ocr-box", i as u64))
                .role(Role::ListItem)
                .aria_label(SharedString::from(format!(
                    "{} ({:.0}% confidence)",
                    b.text,
                    b.confidence * 100.
                )))
                .absolute()
                .left(px(b.x * w))
                .top(px(b.y * h))
                .w(px((b.w * w).max(1.)))
                .h(px(box_h.max(1.)))
                .rounded(px(2.))
                .border_1()
                .border_color(if hot { p.ocr_box_hover } else { p.ocr_box })
                .bg(if hot {
                    p.ocr_box_fill
                } else {
                    alpha(p.ocr_box_fill, 0.18)
                })
                .on_hover(cx.listener(move |this, on: &bool, _, cx| {
                    this.ocr.hovered = on.then_some(i).or(this.ocr.hovered.filter(|h| *h != i));
                    cx.notify();
                }))
                .when(hot, |d| {
                    d.child(
                        div()
                            .absolute()
                            .left_0()
                            .top_0()
                            .h_full()
                            .min_w_full()
                            .flex()
                            .items_center()
                            .px(px(4.))
                            .rounded(px(2.))
                            .border_1()
                            .border_color(p.ocr_chip_border)
                            .bg(p.ocr_chip_bg)
                            .text_color(p.ocr_chip_text)
                            .font_family(FONT_MONO)
                            .text_size(px((box_h * 0.8).clamp(8., 18.)))
                            .whitespace_nowrap()
                            .child(SharedString::from(b.text.clone())),
                    )
                })
        });
    Some(
        div()
            .id("ocr-overlay")
            .role(Role::List)
            .aria_label("Recognized on-screen text")
            .absolute()
            .left(rect.origin.x)
            .top(rect.origin.y)
            .w(rect.size.width)
            .h(rect.size.height)
            .overflow_hidden()
            .children(boxes)
            .child(
                div()
                    .absolute()
                    .right(px(4.))
                    .bottom(px(4.))
                    .px(px(6.))
                    .py(px(2.))
                    .rounded(radius::XS)
                    .border_1()
                    .border_color(p.ocr_chip_border)
                    .bg(p.ocr_chip_bg)
                    .text_color(p.ocr_chip_text)
                    .text_size(text::XS)
                    .opacity(0.72)
                    .child("hover to read · copy all in ⋯"),
            )
            .into_any_element(),
    )
}
