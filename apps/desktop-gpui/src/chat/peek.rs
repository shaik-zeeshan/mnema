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
use crate::theme::{radius, space, text, Palette};
use crate::timeline::ocr_overlay::{self, next_hover, outcome};
use crate::timeline::stage::image_rect;
use crate::timeline::{Ocr, OcrStatus};
use crate::ui::dialog::Modal;
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::link::Link;
use crate::ui::{Button, Tone};

/// The hero's least height (`.frame-hero`); it grows with the window, as the
/// Svelte panel fills `min(100vh - 100px)`.
pub const HERO_H: f32 = 280.;
/// `.frame-modal__panel { width: min(1060px, 100%) }` inside the 24 px backdrop pad.
const PANEL_W: f32 = 1060.;
/// The panel's chrome around the hero (head, meta line, actions, pads) plus
/// the 100 px the panel leaves of the window height.
const CHROME_H: f32 = 332.;

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

/// The hero's "copy image" (`copyState`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CopyState {
    #[default]
    Idle,
    Copying,
    Copied,
    Failed,
}

/// `copyRevertTimer`: how long "copied" / "copy failed" stays.
pub const COPY_REVERT: std::time::Duration = std::time::Duration::from_millis(1600);

#[derive(Default)]
pub struct PeekData {
    pub frame: FrameLoad,
    /// `get_frame_preview`.
    pub image: Option<PathBuf>,
    pub ocr: Ocr,
    pub copy: CopyState,
    /// `secretRedactionCount` of the preview: the hero says how many.
    pub redactions: u32,
    /// The app's bundle id and its icon (`resolve_app_icons`), for the meta avatar.
    pub bundle: Option<String>,
    pub icon: Option<PathBuf>,
    copy_revert: Option<gpui_kit::Task<()>>,
}

impl Chat {
    /// A frame chip: open the peek, then load the frame's size and image.
    /// (`pub` for the `chat-peek` golden, which opens it as a chip click does.)
    pub fn peek_frame(&mut self, src: Source, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = src.frame_id else { return };
        // Esc reaches the modal only with focus inside it.
        self.peek_focus.focus(window, cx);
        self.peek_gen += 1;
        let gen = self.peek_gen;
        self.peek = Some(src);
        self.peek_data = PeekData::default();
        cx.notify();
        let b = self.sources.clone();
        let task = cx.background_spawn(async move {
            let (redactions, bundle) = b.peek_extras(id);
            let icon = bundle.as_ref().and_then(|bid| {
                let icons = b.app_icons(std::slice::from_ref(bid));
                icons.into_iter().find(|(k, _)| k == bid).map(|(_, p)| p)
            });
            (b.frame(id), b.preview(id), redactions, bundle, icon)
        });
        cx.spawn(async move |this, cx| {
            let (frame, image, redactions, bundle, icon) = task.await;
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
                (
                    this.peek_data.redactions,
                    this.peek_data.bundle,
                    this.peek_data.icon,
                ) = (redactions, bundle, icon);
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

    /// "copy image": the preview as PNG on the clipboard, then "copied" (or
    /// "copy failed") for [`COPY_REVERT`].
    pub(super) fn copy_peek_image(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.peek_data.image.clone() else {
            return;
        };
        if self.peek_data.copy == CopyState::Copying {
            return;
        }
        self.peek_data.copy = CopyState::Copying;
        self.peek_data.copy_revert = None;
        cx.notify();
        let gen = self.peek_gen;
        let task = cx.background_spawn(async move {
            std::fs::read(&path)
                .ok()
                .and_then(|b| crate::timeline::as_png(&b))
        });
        cx.spawn(async move |this, cx| {
            let png = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.peek_gen != gen {
                    return;
                }
                this.peek_data.copy = match png {
                    Some(b) => {
                        let image = gpui_kit::Image::from_bytes(gpui_kit::ImageFormat::Png, b);
                        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_image(&image));
                        CopyState::Copied
                    }
                    None => CopyState::Failed,
                };
                this.peek_data.copy_revert = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(COPY_REVERT).await;
                    let _ = this.update(cx, |this, cx| {
                        this.peek_data.copy = CopyState::Idle;
                        cx.notify();
                    });
                }));
                cx.notify();
            });
        })
        .detach();
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
    viewport: gpui_kit::Size<gpui_kit::Pixels>,
    cx: &mut Context<Chat>,
) -> impl IntoElement {
    let close = cx.entity().downgrade();
    let hero_h = (f32::from(viewport.height) - CHROME_H).max(HERO_H);
    let panel_w = PANEL_W.min(f32::from(viewport.width) - 48.);
    let frame_id = s.frame_id;
    let d = &chat.peek_data;
    let hero = div()
        .id("chat-frame-image")
        .relative()
        .w_full()
        .h(px(hero_h))
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
    // `.frame-hero__redactions`: a warn dot and the count, top-right.
    let hero = hero.when(d.redactions > 0, |h| {
        let n = d.redactions;
        h.child(
            div()
                .id("chat-frame-redactions")
                .absolute()
                .top(px(8.))
                .right(px(8.))
                .flex()
                .items_center()
                .gap(px(5.))
                .px(px(7.))
                .py(px(3.))
                .rounded(px(5.))
                .border_1()
                .border_color(p.border_strong)
                .bg(p.overlay_bg)
                .text_size(text::XS)
                .text_color(p.warn)
                .child(div().size(px(5.)).rounded_full().bg(p.warn))
                .child(format!(
                    "{n} {} applied",
                    if n == 1 { "redaction" } else { "redactions" }
                ))
                .test_support(),
        )
    });
    let ready = matches!(d.frame, FrameLoad::Ready { .. });
    let ocr_label = match (d.ocr.status, d.ocr.boxes.len()) {
        (OcrStatus::Success, n) => format!("show ocr · {}", ocr_count(n)),
        _ => "show ocr".to_string(),
    };
    let copy_label = match d.copy {
        CopyState::Idle => "copy image",
        CopyState::Copying => "copying…",
        CopyState::Copied => "copied",
        CopyState::Failed => "copy failed",
    };
    // `frame-meta`: app · window · time · size, one line.
    let dims = match d.frame {
        FrameLoad::Ready {
            w: Some(w),
            h: Some(h),
        } => Some(format!("{w}×{h}")),
        _ => None,
    };
    let rest: Vec<String> = [s.window_title.clone(), Some(when(s, cx)), dims]
        .into_iter()
        .flatten()
        .collect();
    // `.frame-meta__app`: a 16 px avatar (the app's icon, else its first letter).
    let app = s.app_name.clone().map(|name| {
        let fallback = name
            .trim()
            .chars()
            .next()
            .or_else(|| d.bundle.as_deref().and_then(|b| b.trim().chars().next()))
            .unwrap_or('?')
            .to_uppercase()
            .to_string();
        let avatar = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(16.))
            .rounded(px(4.))
            .overflow_hidden()
            .bg(p.surface_hover)
            .child(match &d.icon {
                Some(path) => {
                    crate::ui::app_icon::app_icon(crate::ui::app_icon::Src::File(path.clone()), 16.)
                        .into_any_element()
                }
                None => div()
                    .text_size(px(9.))
                    .font_weight(crate::theme::W_SEMIBOLD)
                    .text_color(p.text_muted)
                    .child(fallback)
                    .into_any_element(),
            });
        div()
            .id("chat-frame-app")
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .text_color(p.text_strong)
            .child(avatar)
            .child(SharedString::from(name))
            .test_support()
    });
    let has_app = app.is_some();
    Modal::new(
        "chat-frame-modal",
        s.window_title
            .clone()
            .unwrap_or_else(|| "Captured frame".into()),
    )
    .eyebrow("frame")
    .max_w(px(panel_w))
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
            .child(
                div()
                    .id("chat-frame-meta")
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(text::SM)
                    .text_color(p.text_muted)
                    .children(app)
                    .when(has_app && !rest.is_empty(), |d| d.child("·"))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .child(SharedString::from(rest.join(" · "))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space::S2)
                    .child(
                        Button::new("chat-frame-copy", copy_label)
                            .sm()
                            .ghost()
                            .icon(IconName::Copy.path())
                            .disabled(d.image.is_none() || d.copy == CopyState::Copying)
                            .on_click(cx.listener(|this, _, _, cx| this.copy_peek_image(cx))),
                    )
                    // `openFrameUrl`: the guarded URL is scheme-less host/path, always
                    // http(s); re-add https.
                    .children(s.url.clone().map(|u| {
                        let href = if u.starts_with("http://") || u.starts_with("https://") {
                            u.clone()
                        } else {
                            format!("https://{u}")
                        };
                        Link::new("chat-frame-url", u, href).quiet()
                    }))
                    .child(
                        Button::new("chat-frame-ocr", ocr_label)
                            .sm()
                            .ghost()
                            .icon(IconName::ScanText.path())
                            .pressed(d.ocr.visible)
                            .disabled(!ready)
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_peek_ocr(cx))),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("chat-frame-open", "show this frame in the timeline →")
                            .sm()
                            .ghost()
                            // `openInTimeline`: close the peek, then
                            // `open_capture_result_in_main_window { kind: "frame" }`.
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let Some(id) = frame_id else { return };
                                this.close_peek(cx);
                                cx.defer(move |cx| crate::window::open_frame(id, cx));
                            })),
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
