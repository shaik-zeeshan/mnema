//! The OCR overlay (`.timeline__ocr-overlay`): one box per recognised line over
//! the painted image rect; only the hovered box shows its text chip (a chip in
//! every box was the Svelte page's measured slowdown). Plus the O toggle.
//! [`boxes`] and [`status_note`] are shared with Chat's frame peek.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, AnyElement, App, Bounds, Context, Pixels, Role, SharedString};

use super::model::OcrBox;
use super::{OcrStatus, TimelineView};
use crate::theme::{radius, size, space, text, Palette, FONT_MONO};
use crate::ui::notice::inline;
use crate::ui::{alpha, icon, nid, Tone};

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
                (view.ocr.status, view.ocr.boxes, view.ocr.error) = outcome(res);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl TimelineView {
    /// "rerun OCR": reprocess the active frame, then show the new job's boxes.
    pub(super) fn rerun_ocr(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.paging.active_frame().map(|f| f.id) else {
            return;
        };
        // Only pixels that belong to this frame: its own artifact or its exact
        // video position, never another frame's fallback image.
        let source_image = self
            .engine
            .display()
            .filter(|d| {
                d.frame_id == id
                    && d.exact
                    && matches!(
                        d.source_kind,
                        super::model::SourceKind::OriginalFrame
                            | super::model::SourceKind::VideoFallback
                    )
            })
            .map(|d| d.path.to_path_buf());
        self.ocr.frame_id = Some(id);
        self.ocr.visible = true;
        self.ocr.status = OcrStatus::Running;
        self.ocr.boxes.clear();
        self.ocr.error = None;
        self.ocr.hovered = None;
        cx.notify();
        let src = self.source.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move { src.rerun_ocr(id, source_image) })
                .await;
            this.update(cx, |view, cx| {
                if view.ocr.frame_id != Some(id) {
                    return; // the playhead moved on
                }
                (view.ocr.status, view.ocr.boxes, view.ocr.error) = outcome(res);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

/// The hover rule: entering `i` lights it; leaving `i` clears it only if it is lit.
pub fn next_hover(current: Option<usize>, i: usize, on: bool) -> Option<usize> {
    on.then_some(i).or(current.filter(|h| *h != i))
}

/// `(status, boxes, error)` from an OCR load (`None` = never OCR'd).
pub fn outcome(
    res: super::data::Res<Option<Vec<OcrBox>>>,
) -> (OcrStatus, Vec<OcrBox>, Option<String>) {
    match res {
        Ok(None) => (OcrStatus::Missing, Vec::new(), None),
        Ok(Some(b)) if b.is_empty() => (OcrStatus::Empty, b, None),
        Ok(Some(b)) => (OcrStatus::Success, b, None),
        Err(e) => (OcrStatus::Error, Vec::new(), Some(e)),
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
    let this = cx.entity().downgrade();
    Some(boxes(
        &v.ocr.boxes,
        v.ocr.hovered,
        rect,
        Some("hover to read · copy all in ⋯"),
        p,
        Rc::new(move |i, on, cx| {
            this.update(cx, |this, cx| {
                this.ocr.hovered = next_hover(this.ocr.hovered, i, on);
                cx.notify();
            })
            .ok();
        }),
    ))
}

/// One box per recognised line over the painted image `rect` (Timeline stage,
/// Chat's frame peek: `FrameOcrOverlay`); only the hovered box shows its text.
/// `on_hover(i, entered)` keeps the owner's hovered index.
pub fn boxes(
    boxes: &[OcrBox],
    hovered: Option<usize>,
    rect: Bounds<Pixels>,
    hint: Option<&'static str>,
    p: &Palette,
    on_hover: Rc<dyn Fn(usize, bool, &mut App)>,
) -> AnyElement {
    let (w, h) = (f32::from(rect.size.width), f32::from(rect.size.height));
    let boxes = boxes.iter().take(MAX_BOXES).enumerate().map(|(i, b)| {
        let hot = hovered == Some(i);
        let box_h = b.h * h;
        let on_hover = on_hover.clone();
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
            .on_hover(move |on: &bool, _, cx| on_hover(i, *on, cx))
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
        .children(hint.map(|hint| {
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
                .child(hint)
        }))
        .test_support()
        .into_any_element()
}

/// The non-success OCR note (loading, no text, never OCR'd, failed), pinned to
/// the image's bottom-left (`.timeline__ocr-status` / `.frame-ocr-status`).
pub fn status_note(status: OcrStatus, error: Option<&str>, p: &Palette) -> Option<AnyElement> {
    let (tone, lead, msg): (Tone, AnyElement, String) = match status {
        OcrStatus::Idle | OcrStatus::Success => return None,
        OcrStatus::Running => (
            Tone::Neutral,
            crate::ui::progress::spinner(size::SPIN_SM).into_any_element(),
            "Loading OCR…".into(),
        ),
        OcrStatus::Empty => (
            Tone::Neutral,
            icon(IconName::ScanText.path(), size::ICON_SM).into_any_element(),
            "No text detected on this frame".into(),
        ),
        OcrStatus::Missing => (
            Tone::Neutral,
            icon(IconName::ScanText.path(), size::ICON_SM).into_any_element(),
            "No OCR for this frame yet".into(),
        ),
        OcrStatus::Error => (
            Tone::Danger,
            icon(IconName::TriangleAlert.path(), size::ICON_SM).into_any_element(),
            error.unwrap_or("OCR failed").to_string(),
        ),
    };
    Some(
        div()
            .absolute()
            .left(space::S3)
            .bottom(space::S3)
            .px(px(9.))
            .py(px(4.))
            .rounded(radius::SM)
            .border_1()
            .border_color(p.overlay_border)
            .bg(p.overlay_bg_strong)
            .child(inline(
                tone,
                Some(lead),
                vec![SharedString::from(msg).into_any_element()],
                p,
            ))
            .into_any_element(),
    )
}
