//! The frame peek (`FrameDetailModal`): the frame's image, its app, window and
//! time, the lazy OCR overlay (the Timeline's boxes, toggled by "Show OCR"), and
//! the way into the Timeline. A frame retention removed says so.

use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{canvas, div, img, px, Context, ObjectFit, SharedString, Window};

use super::{Chat, Source};
use crate::theme::{radius, space, text, Palette, FONT_MONO};
use crate::timeline::ocr_overlay::{self, next_hover, outcome};
use crate::timeline::stage::image_rect;
use crate::timeline::{Ocr, OcrStatus};
use crate::ui::dialog::Modal;
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::{Button, Tone};

/// The hero's height (`.frame-hero`).
pub const HERO_H: f32 = 280.;

#[derive(Clone, Debug, Default, PartialEq)]
pub enum FrameLoad {
    #[default]
    Loading,
    /// `get_frame` answered: the frame's pixel size.
    Ready {
        w: Option<u32>,
        h: Option<u32>,
    },
    /// `get_frame` found no row: retention (or a delete) removed it.
    Gone,
    Failed(String),
}

#[derive(Default)]
pub struct PeekData {
    pub frame: FrameLoad,
    /// `get_frame_preview`.
    pub image: Option<PathBuf>,
    pub ocr: Ocr,
}

impl Chat {
    /// A frame chip: open the peek, then load the frame's size and image.
    pub(super) fn peek_frame(&mut self, src: Source, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = src.frame_id else { return };
        // Esc reaches the modal only with focus inside it.
        self.peek_focus.focus(window, cx);
        self.peek_gen += 1;
        let gen = self.peek_gen;
        self.peek = Some(src);
        self.peek_data = PeekData::default();
        cx.notify();
        let b = self.sources.clone();
        let task = cx.background_spawn(async move { (b.frame(id), b.preview(id)) });
        cx.spawn(async move |this, cx| {
            let (frame, image) = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.peek_gen != gen {
                    return; // closed, or another frame opened
                }
                this.peek_data.frame = match frame {
                    Ok(Some((w, h))) => FrameLoad::Ready { w, h },
                    Ok(None) => FrameLoad::Gone,
                    Err(e) => FrameLoad::Failed(e),
                };
                this.peek_data.image = image;
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn close_peek(&mut self, cx: &mut Context<Self>) {
        self.peek = None;
        self.peek_gen += 1;
        self.peek_data = PeekData::default();
        cx.notify();
    }

    /// "Show OCR": the first time loads it; after that it only shows / hides.
    pub(super) fn toggle_peek_ocr(&mut self, cx: &mut Context<Self>) {
        let id = self.peek.as_ref().and_then(|s| s.frame_id);
        let (Some(id), FrameLoad::Ready { .. }) = (id, &self.peek_data.frame) else {
            return;
        };
        let ocr = &mut self.peek_data.ocr;
        ocr.visible = !ocr.visible;
        if ocr.visible && ocr.status == OcrStatus::Idle {
            ocr.status = OcrStatus::Running;
            ocr.frame_id = Some(id);
            let gen = self.peek_gen;
            let b = self.sources.clone();
            let task = cx.background_spawn(async move { b.ocr(id) });
            cx.spawn(async move |this, cx| {
                let res = task.await;
                let _ = this.update(cx, |this, cx| {
                    if this.peek_gen != gen {
                        return;
                    }
                    let o = &mut this.peek_data.ocr;
                    (o.status, o.boxes, o.error) = outcome(res);
                    cx.notify();
                });
            })
            .detach();
        }
        cx.notify();
    }
}

fn when(s: &Source, cx: &Context<Chat>) -> String {
    super::sources::when(&s.started_at, crate::ui::clock::now(cx))
}

pub fn frame_modal(
    chat: &Chat,
    s: &Source,
    p: &Palette,
    cx: &mut Context<Chat>,
) -> impl IntoElement {
    let close = cx.entity().downgrade();
    let frame_id = s.frame_id;
    let d = &chat.peek_data;
    let hero = div()
        .id("chat-frame-image")
        .relative()
        .w_full()
        .h(px(HERO_H))
        .rounded(radius::MD)
        .overflow_hidden()
        .bg(p.surface_subtle)
        .child({
            let cell = chat.hero_size.clone();
            canvas(
                move |b, window, _| {
                    if cell.get() != b.size {
                        cell.set(b.size);
                        window.refresh();
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full()
        });
    // Decode only once the hero has a size, so the image is fitted to it.
    let sized = f32::from(chat.hero_size.get().width) > 0.;
    let hero = match (&d.frame, &d.image) {
        (FrameLoad::Gone | FrameLoad::Failed(_), _) => hero.child(unavailable(&d.frame, p)),
        (FrameLoad::Ready { w, h }, Some(path)) if sized => {
            let rect = image_rect(chat.hero_size.get(), *w, *h);
            let o = &d.ocr;
            let entity = cx.entity().downgrade();
            hero.child(
                img(path.clone())
                    .image_cache(&chat.images)
                    .object_fit(ObjectFit::Contain)
                    .absolute()
                    .inset_0()
                    .size_full(),
            )
            .children(
                rect.filter(|_| o.visible && o.status == OcrStatus::Success && !o.boxes.is_empty())
                    .map(|rect| {
                        ocr_overlay::boxes(
                            &o.boxes,
                            o.hovered,
                            rect,
                            None,
                            p,
                            Rc::new(move |i, on, cx| {
                                let _ = entity.update(cx, |c, cx| {
                                    c.peek_data.ocr.hovered =
                                        next_hover(c.peek_data.ocr.hovered, i, on);
                                    cx.notify();
                                });
                            }),
                        )
                    }),
            )
            .children(
                o.visible
                    .then(|| ocr_overlay::status_note(o.status, o.error.as_deref(), p))
                    .flatten(),
            )
        }
        (FrameLoad::Ready { .. }, None) => hero.child(placeholder("No preview for this frame", p)),
        _ => hero.child(placeholder("Loading frame…", p)),
    };
    let ready = matches!(d.frame, FrameLoad::Ready { .. });
    let ocr_label = match (d.ocr.status, d.ocr.boxes.len()) {
        (OcrStatus::Success, n) => format!("Show OCR · {}", ocr_count(n)),
        _ => "Show OCR".to_string(),
    };
    Modal::new(
        "chat-frame-modal",
        s.app_name
            .clone()
            .unwrap_or_else(|| "Captured frame".into()),
    )
    .child(
        div()
            .id("chat-frame-body")
            .track_focus(&chat.peek_focus)
            .flex()
            .flex_col()
            .gap(space::S3)
            .p(space::S5)
            .text_size(text::MD)
            .text_color(p.text)
            .child(hero)
            .children(
                s.window_title
                    .clone()
                    .map(|t| div().text_color(p.text_strong).child(SharedString::from(t))),
            )
            .child(
                div()
                    .font_family(FONT_MONO)
                    .text_size(text::BASE)
                    .text_color(p.text_subtle)
                    .child(SharedString::from(when(s, cx))),
            )
            .child(
                div()
                    .flex()
                    .gap(space::S2)
                    .child(
                        Button::new("chat-frame-ocr", ocr_label)
                            .sm()
                            .ghost()
                            .icon(IconName::ScanText.path())
                            .pressed(d.ocr.visible)
                            .disabled(!ready)
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_peek_ocr(cx))),
                    )
                    .child(
                        Button::new("chat-frame-open", "Open in timeline")
                            .sm()
                            // `open_capture_result_in_main_window { kind: "frame" }`.
                            .on_click(move |_, _, cx| {
                                let Some(id) = frame_id else { return };
                                cx.defer(move |cx| crate::window::open_frame(id, cx));
                            }),
                    ),
            ),
    )
    .on_close(move |_, cx| {
        let _ = close.update(cx, |c, cx| c.close_peek(cx));
    })
}

/// `ocrCountLabel`: the overlay draws at most `MAX_BOXES`.
pub fn ocr_count(n: usize) -> String {
    if n > ocr_overlay::MAX_BOXES {
        format!("{}+", ocr_overlay::MAX_BOXES)
    } else {
        n.to_string()
    }
}

fn placeholder(msg: &'static str, p: &Palette) -> impl IntoElement {
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .text_size(text::SM)
        .text_color(p.text_subtle)
        .child(msg)
}

/// `Frame unavailable`: why, honestly (removed vs. couldn't be read).
fn unavailable(load: &FrameLoad, p: &Palette) -> impl IntoElement {
    let (tone, body) = match load {
        FrameLoad::Gone => (
            Tone::Neutral,
            "This frame is no longer stored. Retention or a delete removed it.",
        ),
        _ => (Tone::Danger, "Reading this frame failed. Try again later."),
    };
    div()
        .id("chat-frame-unavailable")
        .test_support()
        .role(gpui_kit::Role::Status)
        .aria_label("Frame unavailable")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .child(empty(
            EmptyLayout::Compact,
            tone,
            IconName::ImageOff.path(),
            "Frame unavailable",
            body,
            Vec::new(),
            p,
        ))
}
