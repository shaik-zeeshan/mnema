//! The stage's top-right actions: play-this-moment (opens the S9 AudioDrawer)
//! and the ⋯ menu: Copy image (C), Download image (D), Copy text, Open host.
//! Copy / Download need the exact preview; a frame with redacted secrets asks first.

use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{
    div, px, AnyElement, ClipboardItem, Context, Image, ImageFormat, PromptLevel, Role,
    SharedString, Window,
};

use super::model::audio_moment;
use super::{OcrStatus, TimelineView};
use crate::theme::{radius, size, space, text, Palette, FONT_MONO};
use crate::ui::{icon, popover, Button};

/// `fileExtensionForMimeType`.
fn ext(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "png",
    }
}

/// The stored preview (WebP, or JPEG) as PNG: most apps can't paste a WebP
/// pasteboard item, so `FrameDetailModal.svelte` re-encodes it too (via RGBA).
pub fn as_png(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::load_from_memory(bytes)
        .ok()?
        .write_to(&mut out, image::ImageFormat::Png)
        .ok()?;
    Some(out.into_inner())
}

/// `activeFrameDownloadName`: `frame-<id>-<captured, punctuation dashed>.<ext>`.
pub fn download_name(id: i64, captured_at_ms: i64, mime: &str) -> String {
    let t = super::model::local(captured_at_ms, time::UtcOffset::UTC);
    let stamp = format!(
        "{:04}-{:02}-{:02}T{:02}-{:02}-{:02}-{:03}Z",
        t.year(),
        u8::from(t.month()),
        t.day(),
        t.hour(),
        t.minute(),
        t.second(),
        t.millisecond()
    );
    format!("frame-{id}-{stamp}.{}", ext(mime))
}

impl TimelineView {
    /// The exact preview of the active frame, if it's loaded.
    fn exact_active(&self) -> Option<(i64, i64, super::model::Preview)> {
        let f = self.paging.active_frame()?;
        Some((f.id, f.captured_at_ms, self.engine.exact(f.id)?.clone()))
    }

    /// Ask before handing out a frame whose preview had secrets redacted.
    fn confirm_redacted(
        &mut self,
        redactions: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
    ) {
        if redactions == 0 {
            return then(self, cx);
        }
        let answer = window.prompt(
            PromptLevel::Warning,
            "Open Original Capture",
            Some("Original capture may still contain redacted secrets. Continue?"),
            &["Continue", "Cancel"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if answer.await == Ok(0) {
                this.update(cx, |view, cx| then(view, cx)).ok();
            }
        })
        .detach();
    }

    pub fn copy_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy_action.is_some() {
            return;
        }
        let Some((id, _, preview)) = self.exact_active() else {
            return self.set_status(Some("Frame preview not ready yet".into()), None, false, cx);
        };
        self.confirm_redacted(preview.redactions, window, cx, move |view, cx| {
            view.busy_action = Some("copy");
            view.set_status(Some("Copying…".into()), None, false, cx);
            let path = preview.file_path.clone();
            cx.spawn(async move |this, cx| {
                let png = cx
                    .background_executor()
                    .spawn(async move { std::fs::read(&path).ok().and_then(|b| as_png(&b)) })
                    .await;
                this.update(cx, |view, cx| {
                    view.busy_action = None;
                    match png {
                        Some(b) => {
                            let image = Image::from_bytes(ImageFormat::Png, b);
                            cx.write_to_clipboard(ClipboardItem::new_image(&image));
                            view.menu_open = false;
                            view.set_status(Some(format!("Copied frame {id}")), None, false, cx);
                        }
                        None => view.set_status(
                            Some("Copy failed: clipboard write was rejected".into()),
                            None,
                            false,
                            cx,
                        ),
                    }
                })
                .ok();
            })
            .detach();
        });
    }

    pub fn download_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy_action.is_some() {
            return;
        }
        let Some((id, at, preview)) = self.exact_active() else {
            return self.set_status(Some("Frame preview not ready yet".into()), None, false, cx);
        };
        self.confirm_redacted(preview.redactions, window, cx, move |view, cx| {
            view.busy_action = Some("download");
            view.set_status(Some("Saving…".into()), None, false, cx);
            let from = preview.file_path.clone();
            let to = PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join("Downloads")
                .join(download_name(id, at, &preview.mime_type));
            cx.spawn(async move |this, cx| {
                let res = cx
                    .background_executor()
                    .spawn(async move { std::fs::copy(&from, &to) })
                    .await;
                this.update(cx, |view, cx| {
                    view.busy_action = None;
                    let msg = match res {
                        Ok(_) => {
                            view.menu_open = false;
                            format!("Saved frame {id} to Downloads")
                        }
                        Err(_) => "Download failed: file write was rejected".into(),
                    };
                    view.set_status(Some(msg), None, false, cx);
                })
                .ok();
            })
            .detach();
        });
    }

    /// `open_captured_url` for the active frame; the dashboard shows its own
    /// status instead of a dialog (`openCapturedUrl(.., { silent: true })`).
    fn open_captured_url(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.paging.active_frame().map(|f| f.id) else {
            return;
        };
        self.menu_open = false;
        cx.notify();
        let src = self.source.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move { src.open_captured_url(id) })
                .await;
            this.update(cx, |view, cx| match res {
                Ok(true) => {}
                Ok(false) => view.set_status(
                    Some("No openable page for this result.".into()),
                    None,
                    false,
                    cx,
                ),
                Err(e) => view.set_status(Some("Couldn't open URL".into()), Some(e), true, cx),
            })
            .ok();
        })
        .detach();
    }

    fn copy_text(&mut self, cx: &mut Context<Self>) {
        let text = self
            .ocr
            .boxes
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if text.trim().is_empty() {
            return self.set_status(Some("No recognized text to copy".into()), None, false, cx);
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text.trim().to_string()));
        let n = self.ocr.boxes.len();
        self.menu_open = false;
        self.set_status(
            Some(format!(
                "Copied {n} text region{}",
                if n == 1 { "" } else { "s" }
            )),
            None,
            false,
            cx,
        );
    }
}

fn item(
    id: &'static str,
    glyph: SharedString,
    label: impl Into<SharedString>,
    meta: Option<&'static str>,
    enabled: bool,
    p: &Palette,
) -> impl crate::ui::Control {
    let hover = (p.surface_hover, p.text_strong);
    div()
        .id(id)
        .test_support()
        .role(Role::MenuItem)
        .flex()
        .items_center()
        .gap(space::S2)
        .px(px(10.))
        .py(px(7.))
        .rounded(radius::SM)
        .text_size(text::MD)
        .text_color(p.text)
        .when(!enabled, |d| d.opacity(crate::theme::DISABLED_OPACITY))
        .when(enabled, |d| {
            d.cursor_pointer()
                .hover(move |s| s.bg(hover.0).text_color(hover.1))
        })
        .child(div().text_color(p.text_subtle).child(icon(glyph, px(14.))))
        .child(div().min_w_0().truncate().child(label.into()))
        .children(meta.map(|m| {
            div()
                .ml_auto()
                .font_family(FONT_MONO)
                .text_size(text::XS)
                .text_color(p.text_subtle)
                .child(m)
        }))
}

fn menu(
    v: &TimelineView,
    at: popover::Anchor,
    p: &Palette,
    cx: &mut Context<TimelineView>,
) -> AnyElement {
    let frame = v.paging.active_frame();
    let ready = frame.is_some_and(|f| v.engine.exact(f.id).is_some()) && v.busy_action.is_none();
    let here = frame.map(|f| f.id) == v.ocr.frame_id;
    let has_text = here && v.ocr.status == OcrStatus::Success && !v.ocr.boxes.is_empty();
    let reading = here && v.ocr.status == OcrStatus::Running;
    let busy = v.busy_action;
    let copy_label = if busy == Some("copy") {
        "Copying…"
    } else {
        "Copy image"
    };
    let save_label = if busy == Some("download") {
        "Saving…"
    } else {
        "Download image"
    };
    let text_label = if reading {
        "Reading text…"
    } else {
        "Copy text"
    };
    let host = frame.and_then(|f| f.host()).map(String::from);
    let pop = popover::surface(p)
        .id("frame-actions-menu")
        .role(Role::Menu)
        .aria_label("Frame actions")
        .absolute()
        .top(size::H_MD + size::POP_GAP)
        .right_0()
        .min_w(px(200.))
        .p(px(4.))
        .on_mouse_down_out(cx.listener(move |this, e, _, cx| {
            // The ⋯ trigger toggles it.
            if at.outside(e, cx) {
                this.menu_open = false;
                cx.notify();
            }
        }))
        .child(
            item(
                "action-copy",
                IconName::Copy.path(),
                copy_label,
                Some("C"),
                ready,
                p,
            )
            .when(ready, |d| {
                d.on_click(cx.listener(|this, _, w, cx| this.copy_image(w, cx)))
            }),
        )
        .child(
            item(
                "action-download",
                IconName::Download.path(),
                save_label,
                Some("D"),
                ready,
                p,
            )
            .when(ready, |d| {
                d.on_click(cx.listener(|this, _, w, cx| this.download_image(w, cx)))
            }),
        )
        .child(
            item(
                "action-copy-text",
                IconName::ScanText.path(),
                text_label,
                None,
                has_text,
                p,
            )
            .when(has_text, |d| {
                d.on_click(cx.listener(|this, _, _, cx| this.copy_text(cx)))
            }),
        )
        .children(host.map(|h| {
            item(
                "action-open-host",
                IconName::ExternalLink.path(),
                h,
                None,
                true,
                p,
            )
            .on_click(cx.listener(|this, _, _, cx| this.open_captured_url(cx)))
        }));
    popover::float(pop).into_any_element()
}

/// The headphones + ⋯ cluster at the stage's top-right.
pub fn render(
    v: &TimelineView,
    p: &Palette,
    window: &mut Window,
    cx: &mut Context<TimelineView>,
) -> AnyElement {
    let at = popover::Anchor::new("frame-actions-at", window, cx);
    let moment = v
        .paging
        .active_frame()
        .and_then(|f| audio_moment(&v.audio, f.captured_at_ms))
        .map(|(s, _)| s.source.label());
    div()
        .absolute()
        .top(px(10.))
        .right(px(10.))
        .flex()
        .items_center()
        .gap(px(6.))
        .children(moment.map(|src| {
            Button::icon_only(
                "play-moment",
                IconName::Headphones.path(),
                SharedString::from(format!("Play audio at this moment ({src})")),
            )
            .tip("Play audio at this moment (P)")
            .on_click(cx.listener(|this, _, _, cx| this.play_moment(cx)))
        }))
        .child(
            at.track(div())
                .relative()
                .child(
                    Button::icon_only("frame-actions", IconName::Ellipsis.path(), "Frame actions")
                        .pressed(v.menu_open)
                        .tip("Frame actions (C copy, D download)")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.menu_open = !this.menu_open;
                            // The menu's Copy text needs this frame's OCR.
                            if this.menu_open {
                                this.load_ocr(cx);
                            }
                            cx.notify();
                        })),
                )
                .when(v.menu_open, |d| d.child(menu(v, at.clone(), p, cx))),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    #[test]
    fn download_names_match_the_svelte_page() {
        // capturedAt "2026-11-04T09:01:56.000Z" → "2026-11-04T09-01-56-000Z".
        assert_eq!(
            super::download_name(4824, 1_793_782_916_000, "image/webp"),
            "frame-4824-2026-11-04T09-01-56-000Z.webp"
        );
    }
}
