//! The OCR overlay (`.timeline__ocr-overlay`): one box per recognised line over
//! the painted image rect; only the hovered box shows its text chip (a chip in
//! every box was the Svelte page's measured slowdown). Plus the O toggle.
//! [`boxes`] and [`status_note`] are shared with Chat's frame peek.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{
    div, px, AnyElement, App, BackgroundExecutor, Bounds, Context, Pixels, Role, SharedString,
};

use super::data::{OcrCheck, OcrLoad, OCR_POLL, OCR_STILL_RUNNING, OCR_WAIT};
use super::model::OcrBox;
use super::Ocr;
use super::{OcrStatus, TimelineView};
use crate::theme::{radius, size, space, text, Palette, FONT_MONO};
use crate::ui::notice::inline;
use crate::ui::Tracking as _;
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

    /// A new active frame drops the last one's OCR and hides the overlay, so a
    /// stale box never sits on the wrong frame (`+page.svelte`'s reset effect).
    pub(super) fn reset_ocr_if_moved(&mut self) {
        let id = self.paging.active_frame().map(|f| f.id);
        if self.ocr.frame_id.is_some() && self.ocr.frame_id != id {
            self.ocr = Ocr::default();
        }
    }

    /// `loadOcrForActiveFrame`; the overlay's visibility is the caller's (the ⋯
    /// menu loads quietly so Copy text knows whether there is any).
    pub(super) fn load_ocr(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.paging.active_frame().map(|f| f.id) else {
            return;
        };
        if self.ocr.frame_id == Some(id) {
            return;
        }
        self.begin_ocr(id);
        let src = self.source.clone();
        self.finish_ocr(id, cx, move || src.ocr_load(id));
    }

    fn begin_ocr(&mut self, id: i64) {
        self.ocr.frame_id = Some(id);
        self.ocr.status = OcrStatus::Running;
        self.ocr.boxes.clear();
        self.ocr.error = None;
        self.ocr.provider = None;
        self.ocr.from_frame = None;
        self.ocr.hovered = None;
    }

    fn finish_ocr(
        &mut self,
        id: i64,
        cx: &mut Context<Self>,
        f: impl FnOnce() -> super::data::Res<OcrCheck> + Send + 'static,
    ) {
        let src = self.source.clone();
        self.ocr.task = Some(cx.spawn(async move |this, cx| {
            let ex = cx.background_executor().clone();
            let res = wait(ex, f, move |job| src.ocr_job(job)).await;
            this.update(cx, |view, cx| {
                if view.ocr.frame_id != Some(id) {
                    return; // the playhead moved on
                }
                view.ocr.rerun_loading = false;
                let (provider, from_frame) = match &res {
                    Ok(l) => (l.provider.clone(), l.from_frame),
                    Err(_) => (None, None),
                };
                (view.ocr.status, view.ocr.boxes, view.ocr.error) = outcome(res.map(|l| l.boxes));
                (view.ocr.provider, view.ocr.from_frame) = (provider, from_frame);
                cx.notify();
            })
            .ok();
        }));
    }

    /// `ocrRerunDisabled`.
    pub fn rerun_disabled(&self) -> bool {
        self.paging.active_frame().is_none()
            || !self.settings.ocr_enabled
            || self.ocr.rerun_loading
            || self.ocr.status == OcrStatus::Running
    }

    /// "run OCR" / "rerun OCR": reprocess the active frame, then show the new job's boxes.
    pub(super) fn rerun_ocr(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.paging.active_frame().map(|f| f.id) else {
            return;
        };
        if self.rerun_disabled() {
            return;
        }
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
        self.begin_ocr(id);
        self.ocr.visible = true;
        self.ocr.rerun_loading = true;
        cx.notify();
        let src = self.source.clone();
        self.finish_ocr(id, cx, move || src.rerun_ocr(id, source_image));
    }
}

/// `ocrCountLabel`: the overlay draws at most `MAX_BOXES`.
pub fn count_label(n: usize) -> String {
    if n > MAX_BOXES {
        format!("{MAX_BOXES}+")
    } else {
        n.to_string()
    }
}

/// `ocrToggleTitle`: what the O button would do, or what the last read found.
pub fn toggle_tip(v: &TimelineView) -> String {
    let o = &v.ocr;
    let reused = |s: &str| match o.from_frame {
        Some(f) => format!("{s} (reused from frame {f})"),
        None => s.to_string(),
    };
    let body = if !v.settings.ocr_enabled && !o.visible {
        "OCR is off. Saved OCR text can still be viewed.".to_string()
    } else if let Some(e) = &o.error {
        e.clone()
    } else if o.visible {
        "Hide OCR data for the active frame".into()
    } else {
        match o.status {
            OcrStatus::Success => {
                let n = o.boxes.len();
                let mut t = reused(&format!(
                    "{} text region{} detected",
                    count_label(n),
                    if n == 1 { "" } else { "s" }
                ));
                if let Some(p) = &o.provider {
                    t.push_str(&format!(" · {p}"));
                }
                t
            }
            OcrStatus::Empty => reused("no text detected"),
            OcrStatus::Missing => "no OCR data for this frame".into(),
            _ => "Show OCR data for the active frame".into(),
        }
    };
    format!("{body} (O)")
}

/// The hover rule: entering `i` lights it; leaving `i` clears it only if it is lit.
pub fn next_hover(current: Option<usize>, i: usize, on: bool) -> Option<usize> {
    on.then_some(i).or(current.filter(|h| *h != i))
}

/// `(status, boxes, error)` from an OCR load (`None` = never OCR'd).
/// One OCR load (`first`), then a queued / running job re-checked with `job`
/// every [`OCR_POLL`] on the executor's timer (never a sleeping thread) until
/// [`OCR_WAIT`] runs out. Shared with Chat's frame peek.
pub async fn wait(
    ex: BackgroundExecutor,
    first: impl FnOnce() -> super::data::Res<OcrCheck> + Send + 'static,
    job: impl Fn(i64) -> super::data::Res<OcrCheck> + Send + Sync + 'static,
) -> super::data::Res<OcrLoad> {
    let job = std::sync::Arc::new(job);
    let started = ex.now();
    let mut check = ex.spawn(async move { first() }).await;
    let mut from = None;
    loop {
        match check {
            Ok(OcrCheck::Done(l)) => {
                return Ok(OcrLoad {
                    from_frame: l.from_frame.or(from),
                    ..l
                })
            }
            Ok(OcrCheck::Pending { job_id, from_frame }) => {
                from = from_frame.or(from);
                if ex.now().duration_since(started) > OCR_WAIT {
                    return Err(OCR_STILL_RUNNING.to_string());
                }
                ex.timer(OCR_POLL).await;
                let job = job.clone();
                check = ex.spawn(async move { job(job_id) }).await;
            }
            Err(e) => return Err(e),
        }
    }
}

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
            // `0 0 0 1px hover-shadow, inset 0 0 0 1px hover-inset`: the box is
            // translucent, so the outer ring is a border box around it.
            .when(hot, |d| {
                d.shadow(vec![crate::ui::inset(p.ocr_hover_inset, px(1.))])
                    .child(crate::ui::ring_box(p.ocr_hover_shadow, px(1.), px(2.)))
            })
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
                        .tracking(-0.01)
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
                .rounded(px(3.))
                .border_1()
                .border_color(p.ocr_chip_border)
                .bg(p.ocr_chip_bg)
                .text_color(p.ocr_chip_text)
                .text_size(text::XS)
                .tracking(0.02)
                .opacity(0.72)
                .child(hint)
        }))
        .test_support()
        .into_any_element()
}

/// The non-success OCR note (loading, no text, never OCR'd, failed), pinned to
/// the image's bottom-left (`.timeline__ocr-status` / `.frame-ocr-status`).
pub fn status_note(status: OcrStatus, error: Option<&str>, p: &Palette) -> Option<AnyElement> {
    status_note_from(status, error, None, p)
}

/// [`status_note`]; `reused` names the earlier frame an empty result came from.
pub fn status_note_from(
    status: OcrStatus,
    error: Option<&str>,
    reused: Option<i64>,
    p: &Palette,
) -> Option<AnyElement> {
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
            match reused {
                Some(f) => format!("No text detected (reused from frame {f})"),
                None => "No text detected on this frame".into(),
            },
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
    Some(note_box(
        inline(
            tone,
            Some(lead),
            vec![SharedString::from(msg).into_any_element()],
            p,
        ),
        p,
    ))
}

/// `ocrSuccessUnpositionable`: text came back but the frame has no size, so
/// the boxes can't be placed; point at ⋯ → Copy text instead.
pub fn unpositioned_note(p: &Palette) -> AnyElement {
    note_box(
        inline(
            Tone::Info,
            Some(icon(IconName::Info.path(), size::ICON_SM).into_any_element()),
            vec![
                SharedString::from("Text detected but can’t be positioned — use ⋯ → Copy text")
                    .into_any_element(),
            ],
            p,
        ),
        p,
    )
}

fn note_box(el: impl IntoElement, p: &Palette) -> AnyElement {
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
        .child(el)
        .into_any_element()
}
