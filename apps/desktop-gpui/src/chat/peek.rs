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
use crate::theme::{radius, text, Palette};
use crate::timeline::ocr_overlay::{self, next_hover, outcome};
use crate::timeline::stage::image_rect;
use crate::timeline::{Ocr, OcrStatus};
use crate::ui::dialog::Modal;
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::{with_tip, Tone};

/// The panel's least height on a tiny window.
pub const HERO_H: f32 = 280.;
/// `.frame-modal__panel { width: min(1060px, 100%) }` inside the 24 px backdrop pad.
const PANEL_W: f32 = 1060.;
/// `max-height: calc(100vh - 100px)`.
const PANEL_GAP_H: f32 = 100.;

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

/// A view that shows the frame peek over itself (Chat's sources, Insights'
/// evidence): the state the peek reads and writes. The peek's behaviour is
/// [`PeekOps`], once for every host.
pub trait PeekHost: Sized + 'static {
    fn peek_src(&self) -> Option<&Source>;
    fn set_peek_src(&mut self, src: Option<Source>);
    fn peek_data(&self) -> &PeekData;
    fn peek_data_mut(&mut self) -> &mut PeekData;
    fn peek_gen(&mut self) -> &mut u64;
    fn peek_focus(&self) -> &gpui_kit::FocusHandle;
    fn hero_size(&self) -> &Rc<std::cell::Cell<gpui_kit::Size<gpui_kit::Pixels>>>;
    fn peek_images(&self) -> &gpui_kit::Entity<crate::timeline::image_cache::FrameImages>;
    fn peek_sources(&self) -> std::sync::Arc<dyn super::source_data::SourceBackend>;
}

impl PeekHost for Chat {
    fn peek_src(&self) -> Option<&Source> {
        self.peek.as_ref()
    }
    fn set_peek_src(&mut self, src: Option<Source>) {
        self.peek = src;
    }
    fn peek_data(&self) -> &PeekData {
        &self.peek_data
    }
    fn peek_data_mut(&mut self) -> &mut PeekData {
        &mut self.peek_data
    }
    fn peek_gen(&mut self) -> &mut u64 {
        &mut self.peek_gen
    }
    fn peek_focus(&self) -> &gpui_kit::FocusHandle {
        &self.peek_focus
    }
    fn hero_size(&self) -> &Rc<std::cell::Cell<gpui_kit::Size<gpui_kit::Pixels>>> {
        &self.hero_size
    }
    fn peek_images(&self) -> &gpui_kit::Entity<crate::timeline::image_cache::FrameImages> {
        &self.images
    }
    fn peek_sources(&self) -> std::sync::Arc<dyn super::source_data::SourceBackend> {
        self.sources.clone()
    }
}

impl<V: PeekHost> PeekOps for V {}

pub trait PeekOps: PeekHost {
    /// A frame chip: open the peek, then load the frame's size and image.
    /// A source without its app / window (an Insights evidence frame) gets
    /// them from the frame itself (`FrameDetailModal` loads the frame).
    fn peek_frame(&mut self, src: Source, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = src.frame_id else { return };
        // Esc reaches the modal only with focus inside it.
        self.peek_focus().clone().focus(window, cx);
        *self.peek_gen() += 1;
        let gen = *self.peek_gen();
        let bare = src.app_name.is_none() && src.window_title.is_none();
        self.set_peek_src(Some(src));
        *self.peek_data_mut() = PeekData::default();
        cx.notify();
        let b = self.peek_sources();
        let meta_b = b.clone();
        let meta =
            cx.background_spawn(async move { bare.then(|| meta_b.frame_meta(id)).flatten() });
        cx.spawn(async move |this, cx| {
            if let Some(m) = meta.await {
                let _ = this.update(cx, |this, cx| {
                    if *this.peek_gen() != gen {
                        return;
                    }
                    if let Some(s) = this.peek_src().cloned() {
                        this.set_peek_src(Some(Source {
                            app_name: m.0,
                            window_title: m.1,
                            started_at: m.2,
                            url: m.3,
                            ..s
                        }));
                    }
                    cx.notify();
                });
            }
        })
        .detach();
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
                if *this.peek_gen() != gen {
                    return; // closed, or another frame opened
                }
                let d = this.peek_data_mut();
                d.frame = match frame {
                    Ok(Some((w, h))) => FrameLoad::Ready { w, h },
                    Ok(None) => FrameLoad::Gone,
                    Err(e) => FrameLoad::Failed(e),
                };
                d.image = image;
                (d.redactions, d.bundle, d.icon) = (redactions, bundle, icon);
                cx.notify();
            });
        })
        .detach();
    }

    fn close_peek(&mut self, cx: &mut Context<Self>) {
        self.set_peek_src(None);
        *self.peek_gen() += 1;
        *self.peek_data_mut() = PeekData::default();
        cx.notify();
    }

    /// "copy image": the preview as PNG on the clipboard, then "copied" (or
    /// "copy failed") for [`COPY_REVERT`].
    fn copy_peek_image(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.peek_data().image.clone() else {
            return;
        };
        if self.peek_data().copy == CopyState::Copying {
            return;
        }
        self.peek_data_mut().copy = CopyState::Copying;
        self.peek_data_mut().copy_revert = None;
        cx.notify();
        let gen = *self.peek_gen();
        let task = cx.background_spawn(async move {
            std::fs::read(&path)
                .ok()
                .and_then(|b| crate::timeline::as_png(&b))
        });
        cx.spawn(async move |this, cx| {
            let png = task.await;
            let _ = this.update(cx, |this, cx| {
                if *this.peek_gen() != gen {
                    return;
                }
                this.peek_data_mut().copy = match png {
                    Some(b) => {
                        let image = gpui_kit::Image::from_bytes(gpui_kit::ImageFormat::Png, b);
                        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_image(&image));
                        CopyState::Copied
                    }
                    None => CopyState::Failed,
                };
                this.peek_data_mut().copy_revert = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(COPY_REVERT).await;
                    let _ = this.update(cx, |this, cx| {
                        this.peek_data_mut().copy = CopyState::Idle;
                        cx.notify();
                    });
                }));
                cx.notify();
            });
        })
        .detach();
    }

    /// "Show OCR": the first time loads it; after that it only shows / hides.
    fn toggle_peek_ocr(&mut self, cx: &mut Context<Self>) {
        let id = self.peek_src().and_then(|s| s.frame_id);
        let (Some(id), FrameLoad::Ready { .. }) = (id, &self.peek_data().frame) else {
            return;
        };
        let b = self.peek_sources();
        let gen = *self.peek_gen();
        let ocr = &mut self.peek_data_mut().ocr;
        ocr.visible = !ocr.visible;
        if ocr.visible && ocr.status == OcrStatus::Idle {
            ocr.status = OcrStatus::Running;
            ocr.frame_id = Some(id);
            let task = cx.background_spawn(async move { b.ocr(id) });
            cx.spawn(async move |this, cx| {
                let res = task.await;
                let _ = this.update(cx, |this, cx| {
                    if *this.peek_gen() != gen {
                        return;
                    }
                    let o = &mut this.peek_data_mut().ocr;
                    (o.status, o.boxes, o.error) = outcome(res);
                    cx.notify();
                });
            })
            .detach();
        }
        cx.notify();
    }
}

pub fn frame_modal<V: PeekHost>(
    chat: &V,
    s: &Source,
    p: &Palette,
    viewport: gpui_kit::Size<gpui_kit::Pixels>,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let close = cx.entity().downgrade();
    // `.frame-modal__panel`: min(1060, window − 2·24) wide; the fixture's frames
    // are large, so the panel sits at its max height, window − 100.
    let panel_h = (f32::from(viewport.height) - PANEL_GAP_H).max(HERO_H);
    let panel_w = PANEL_W.min(f32::from(viewport.width) - 48.);
    let frame_id = s.frame_id;
    let d = chat.peek_data();
    let hero = div()
        .id("chat-frame-image")
        .relative()
        .flex_1()
        .min_w_0()
        .h_full()
        .border_1()
        .border_color(p.border)
        .rounded(px(9.))
        .overflow_hidden()
        .bg(p.bg)
        .child({
            let cell = chat.hero_size().clone();
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
    let sized = f32::from(chat.hero_size().get().width) > 0.;
    let hero = match (&d.frame, &d.image) {
        (FrameLoad::Gone | FrameLoad::Failed(_), _) => hero.child(unavailable(&d.frame, p)),
        (FrameLoad::Ready { w, h }, Some(path)) if sized => {
            let rect = image_rect(chat.hero_size().get(), *w, *h);
            let o = &d.ocr;
            let entity = cx.entity().downgrade();
            hero.child(
                img(path.clone())
                    .image_cache(chat.peek_images())
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
                                    let h = next_hover(c.peek_data().ocr.hovered, i, on);
                                    c.peek_data_mut().ocr.hovered = h;
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
    let copy_label = match d.copy {
        CopyState::Idle => "copy image",
        CopyState::Copying => "copying…",
        CopyState::Copied => "copied",
        CopyState::Failed => "copy failed",
    };
    let dims = match d.frame {
        FrameLoad::Ready {
            w: Some(w),
            h: Some(h),
        } => Some(format!("{w}×{h}")),
        _ => None,
    };
    let offset = crate::ui::clock::now(cx).offset();
    let time = compact_time(&s.started_at, offset);
    let title = s
        .window_title
        .clone()
        .unwrap_or_else(|| "Captured frame".into());
    let head = header(&title, p, {
        let close = close.clone();
        move |_, cx| {
            let _ = close.update(cx, |c, cx| c.close_peek(cx));
        }
    });
    let sep = || div().flex_none().text_color(p.text_subtle).child("·");
    // `.frame-meta`: app (avatar + name) · window · time (accent, mono) · size.
    let meta = div()
        .id("chat-frame-meta")
        .flex()
        .items_center()
        .gap(px(8.))
        .min_w_0()
        .pt(px(12.))
        .px(px(18.))
        .pb(px(4.))
        .overflow_hidden()
        .whitespace_nowrap()
        .text_size(text::SM)
        .text_color(p.text_muted)
        .children(app_chip(s, d, p))
        .when(s.app_name.is_some() && s.window_title.is_some(), |m| {
            m.child(sep())
        })
        .children(
            s.window_title
                .clone()
                .map(|t| div().min_w_0().truncate().child(SharedString::from(t))),
        )
        .when(!time.is_empty(), |m| {
            m.child(sep()).child(
                div()
                    .flex_none()
                    .font_family(crate::theme::FONT_MONO)
                    .text_color(p.accent)
                    .child(SharedString::from(time)),
            )
        })
        .children(dims.map(|d| {
            div().flex().flex_none().gap(px(8.)).child(sep()).child(
                div()
                    .text_size(text::XS)
                    .text_color(p.text_subtle)
                    .child(SharedString::from(d)),
            )
        }));
    let ocr_on = d.ocr.visible;
    let count = (d.ocr.status == OcrStatus::Success).then(|| ocr_count(d.ocr.boxes.len()));
    let actions = div()
        .flex()
        .items_center()
        .gap(px(6.))
        .pt(px(8.))
        .px(px(14.))
        .pb(px(14.))
        .child(
            act(
                "chat-frame-copy",
                false,
                d.image.is_none() || d.copy == CopyState::Copying,
                p,
            )
            .child(copy_label)
            .on_click(cx.listener(|this, _, _, cx| this.copy_peek_image(cx)))
            .test_support(),
        )
        // `openFrameUrl`: the guarded URL is scheme-less host/path, always
        // http(s); re-add https.
        .children(s.url.clone().map(|u| {
            let href = if u.starts_with("http://") || u.starts_with("https://") {
                u.clone()
            } else {
                format!("https://{u}")
            };
            with_tip(
                act("chat-frame-url", false, false, p)
                    .max_w(px(260.))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .child(SharedString::from(u.clone())),
                    )
                    .on_click(move |_, _, cx| cx.open_url(&href)),
                SharedString::from(u),
            )
            .test_support()
        }))
        .child(
            act("chat-frame-ocr", ocr_on, !ready, p)
                .aria_toggled(if ocr_on {
                    gpui_kit::accesskit::Toggled::True
                } else {
                    gpui_kit::accesskit::Toggled::False
                })
                .child("show ocr")
                .children(count.map(|n| {
                    div()
                        .text_color(if ocr_on { p.accent } else { p.text_subtle })
                        .child(format!("· {n}"))
                }))
                .on_click(cx.listener(|this, _, _, cx| this.toggle_peek_ocr(cx)))
                .test_support(),
        )
        .child(div().flex_1())
        .child({
            let hover = p.text_muted;
            div()
                .id("chat-frame-open")
                .role(gpui_kit::Role::Link)
                .px(px(4.))
                .py(px(5.))
                .rounded(radius::XS)
                .cursor_pointer()
                .text_size(text::XS)
                .text_color(p.text_subtle)
                .hover(move |st| st.text_color(hover).underline())
                .child("show this frame in the timeline →")
                // `openInTimeline`: close the peek, then
                // `open_capture_result_in_main_window { kind: "frame" }`.
                .on_click(cx.listener(move |this, _, _, cx| {
                    let Some(id) = frame_id else { return };
                    this.close_peek(cx);
                    cx.defer(move |cx| crate::window::open_frame(id, cx));
                }))
                .test_support()
        });
    Modal::new("chat-frame-modal", title)
        .chromeless(px(18.))
        // `.frame-modal { background: var(--app-overlay-bg) }`; the CSS blur(10px)
        // has no GPUI equivalent (no in-window blur), the 78% wash is kept.
        .scrim(p.overlay_bg)
        .whole_window()
        .max_w(px(panel_w))
        .child(
            div()
                .id("chat-frame-body")
                .track_focus(chat.peek_focus())
                .flex()
                .flex_col()
                .h(px(panel_h))
                .bg(p.surface)
                .border_1()
                .border_color(p.border_strong)
                .rounded(px(18.))
                .overflow_hidden()
                .text_size(text::MD)
                .text_color(p.text)
                .child(head)
                .child(div().flex().flex_1().min_h_0().mx(px(16.)).child(hero))
                .child(meta)
                .child(actions),
        )
        .on_close(move |_, cx| {
            let _ = close.update(cx, |c, cx| c.close_peek(cx));
        })
}

/// `formatTimestampCompact`: `Oct 29, 10:14 AM` in local time.
pub fn compact_time(iso: &str, offset: time::UtcOffset) -> String {
    use crate::timeline::model::{clock_label, local};
    let Ok(d) = time::OffsetDateTime::parse(iso, &time::format_description::well_known::Rfc3339)
    else {
        return iso.to_string();
    };
    let t = local((d.unix_timestamp_nanos() / 1_000_000) as i64, offset);
    let month = t.month().to_string();
    format!("{} {}, {}", &month[..3], t.day(), clock_label(t, false))
}

/// `.frame-modal__header`: the accent "FRAME" eyebrow inline before a 13 px
/// title, and the 24 px close.
fn header(
    title: &str,
    p: &Palette,
    on_close: impl Fn(&mut Window, &mut gpui_kit::App) + 'static,
) -> impl IntoElement {
    let (hover_bg, hover_bd, hover_fg) = (p.surface_hover, p.border, p.text_strong);
    // No letter-spacing in GPUI: `.14em` tracking at 10 px is a 1.4 px gap per glyph.
    let eyebrow = div()
        .flex()
        .flex_none()
        .gap(px(1.4))
        .text_size(text::XS)
        .text_color(p.accent)
        .children("FRAME".chars().map(|c| div().child(c.to_string())));
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .pt(px(14.))
        .px(px(16.))
        .pb(px(10.))
        .child(eyebrow)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(text::MD)
                .font_weight(crate::theme::W_SEMIBOLD)
                .text_color(p.text_strong)
                .child(SharedString::from(title.to_string())),
        )
        .child(
            div()
                .id("chat-frame-close")
                .role(gpui_kit::Role::Button)
                .aria_label("Close frame")
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(24.))
                .rounded(px(6.))
                .border_1()
                .border_color(gpui_kit::transparent_black())
                .text_color(p.text_subtle)
                .cursor_pointer()
                .hover(move |st| st.bg(hover_bg).border_color(hover_bd).text_color(hover_fg))
                .child(crate::ui::icon(IconName::X.path(), px(12.)))
                .on_click(move |_, window, cx| on_close(window, cx))
                .test_support(),
        )
}

/// `.frame-act`: a small bordered button (11 px, muted); `on` = the OCR
/// toggle's pressed accent look; `off` = disabled at 0.55.
fn act(id: &'static str, on: bool, off: bool, p: &Palette) -> gpui_kit::Stateful<gpui_kit::Div> {
    let (hbg, hbd, hfg) = (p.surface_hover, p.border_strong, p.text_strong);
    let el = div()
        .id(id)
        .role(gpui_kit::Role::Button)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .px(px(10.))
        .py(px(5.))
        .rounded(px(6.))
        .border_1()
        .text_size(text::SM)
        .line_height(gpui_kit::relative(1.2));
    let el = if on {
        el.bg(p.accent_bg)
            .border_color(p.accent_border)
            .text_color(p.accent)
    } else {
        el.border_color(p.border).text_color(p.text_muted)
    };
    if off {
        el.opacity(0.55)
    } else {
        el.cursor_pointer().when(!on, |e| {
            e.hover(move |st| st.bg(hbg).border_color(hbd).text_color(hfg))
        })
    }
}

/// `.frame-meta__app`: a 16 px avatar (the app's icon, else its first letter)
/// and the app name, strong.
fn app_chip(s: &Source, d: &PeekData, p: &Palette) -> Option<impl IntoElement> {
    let name = s.app_name.clone()?;
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
    Some(
        div()
            .id("chat-frame-app")
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .text_color(p.text_strong)
            .child(avatar)
            .child(SharedString::from(name))
            .test_support(),
    )
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
