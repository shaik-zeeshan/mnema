//! One answer's sources row (`ChatSources.svelte`): compact frame chips (the
//! frame's thumbnail, app, time), a host chip for a frame with a captured page,
//! and audio chips. A frame chip peeks in place (`peek.rs`), an audio chip opens
//! its receipt under the row (`clip.rs`).

use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, img, px, relative, Context, Div, ObjectFit, SharedString};

use super::{format, Chat, Source};
use crate::theme::{self, radius, space, text, Palette, FONT_MONO};
use crate::ui::chip::{chip, ChipKind};
use crate::ui::{alpha, icon, nid, with_tip};

/// Decoded thumbnails kept (≤200 px each): every chip in a long chat at once.
pub const THUMBS_DECODED: usize = 64;

/// `parseCapturedAt` + the weekday clock; the raw text when it doesn't parse.
pub(super) fn when(iso: &str, now: time::OffsetDateTime) -> String {
    match time::OffsetDateTime::parse(iso, &time::format_description::well_known::Rfc3339) {
        Ok(d) => format::weekday_clock((d.unix_timestamp_nanos() / 1_000_000) as i64, now),
        Err(_) => iso.to_string(),
    }
}

/// The 44×28 thumbnail slot: three placeholder lines while it loads (or when
/// the preview call failed), the frame's ≤200 px scrub preview once it lands,
/// and an image-off glyph when the frame has no preview any more.
fn thumb(chat: &Chat, frame_id: Option<i64>, p: &Palette) -> Div {
    let slot = div()
        .w(px(44.))
        .h(px(28.))
        .flex_none()
        .rounded(px(4.))
        .bg(p.surface_subtle)
        .border_1()
        .border_color(p.border)
        .overflow_hidden();
    match frame_id.and_then(|id| chat.thumbs.get(&id)) {
        Some(Some(path)) => slot.child(
            img(path.clone())
                .image_cache(&chat.thumb_images)
                .size_full()
                .object_fit(ObjectFit::Cover),
        ),
        Some(None) => slot
            .flex()
            .items_center()
            .justify_center()
            .text_color(p.text_faint)
            .child(icon(gpui_kit::assets::IconName::ImageOff.path(), px(12.))),
        None => slot
            .flex()
            .flex_col()
            .gap(px(3.))
            .p(px(4.))
            .children([0.8, 0.55, 0.7].map(|w| {
                div()
                    .h(px(2.))
                    .w(relative(w))
                    .rounded(px(1.))
                    .bg(alpha(p.fg, 0.18))
            })),
    }
}

impl Chat {
    /// Ask once for every shown frame chip's thumbnail (`get_frame_scrub_previews`);
    /// an answer for a chat that is no longer open is dropped.
    pub(super) fn request_thumbs(&mut self, cx: &mut Context<Self>) {
        let mut ids = Vec::new();
        for t in self.turns.iter().filter(|t| t.view.phase == "done") {
            for s in super::model::sources(&t.view.sources) {
                if let Some(id) = s.frame_id.filter(|_| s.kind == "frame") {
                    if self.thumbs_asked.insert(id) {
                        ids.push(id);
                    }
                }
            }
        }
        if ids.is_empty() {
            return;
        }
        let gen = self.thumbs_gen;
        let b = self.sources.clone();
        let task = cx.background_spawn(async move { b.thumbs(&ids) });
        cx.spawn(async move |this, cx| {
            let got = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.thumbs_gen != gen {
                    return;
                }
                for p in got {
                    this.thumbs.insert(p.frame_id, p.path);
                }
                cx.notify();
            });
        })
        .detach();
    }
}

pub fn row(chat: &Chat, turn: i64, sources: &[Source], p: &Palette, cx: &mut Context<Chat>) -> Div {
    let now = crate::ui::clock::now(cx);
    let frames = sources.iter().filter(|s| s.kind == "frame");
    let audio = sources.iter().filter(|s| s.kind == "audio");
    let base = format!("chat-src-{turn}");
    let mut kids: Vec<gpui_kit::AnyElement> = Vec::new();
    for (i, s) in frames.enumerate() {
        let tip = s
            .window_title
            .clone()
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "Peek this frame".into());
        let src = s.clone();
        kids.push(
            with_tip(
                div()
                    .id(nid(&format!("{base}-f"), i as u64))
                    .test_support()
                    .role(gpui_kit::Role::Button)
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .py(px(4.))
                    .pl(px(4.))
                    .pr(px(9.))
                    .rounded(radius::MD)
                    .border_1()
                    .border_color(p.border)
                    .bg(p.surface)
                    .cursor_pointer()
                    .hover(|st| st.border_color(p.border_hover))
                    .child(thumb(chat, s.frame_id, p))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(
                                div()
                                    .font_weight(theme::W_MEDIUM)
                                    .text_size(text::XS)
                                    .line_height(relative(1.))
                                    .text_color(p.text_strong)
                                    .child(SharedString::from(
                                        s.app_name.clone().unwrap_or_else(|| "Unknown app".into()),
                                    )),
                            )
                            .child(
                                div()
                                    .font_family(FONT_MONO)
                                    .text_size(text::MICRO)
                                    .line_height(relative(1.))
                                    .text_color(p.text_subtle)
                                    .child(SharedString::from(when(&s.started_at, now))),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        // A frame source always carries its id; without one there is
                        // nothing to peek or land on.
                        this.peek_frame(src.clone(), window, cx);
                    })),
                tip,
            )
            .into_any_element(),
        );
        if let Some(url) = &s.url {
            let host = url.split('/').next().unwrap_or(url).to_string();
            let opening = s.frame_id.is_some() && chat.opening_url == s.frame_id;
            kids.push(
                with_tip(
                    div()
                        .id(nid(&format!("{base}-u"), i as u64))
                        .test_support()
                        .role(gpui_kit::Role::Link)
                        .when(opening, |d| d.opacity(theme::DISABLED_OPACITY))
                        .cursor_pointer()
                        .child(chip(host, ChipKind::Neutral, p).gap(px(4.)).child(icon(
                            gpui_kit::assets::IconName::ExternalLink.path(),
                            px(10.),
                        ))),
                    url.clone(),
                )
                .on_click(cx.listener({
                    let frame_id = s.frame_id;
                    move |this, _, _, cx| this.open_captured_url(frame_id, cx)
                }))
                .into_any_element(),
            );
        }
    }
    for (i, s) in audio.enumerate() {
        let mic = s.source_kind.as_deref() == Some("microphone");
        let app = s
            .app_name
            .as_ref()
            .map(|a| format!("{a} · "))
            .unwrap_or_default();
        let label = format!(
            "{} · {app}{}",
            if mic { "mic" } else { "system" },
            when(&s.started_at, now)
        );
        kids.push(
            with_tip(
                div()
                    .id(nid(&format!("{base}-a"), i as u64))
                    .test_support()
                    .role(gpui_kit::Role::Button)
                    .cursor_pointer()
                    .child(chip(
                        label,
                        if mic {
                            ChipKind::Mic
                        } else {
                            ChipKind::SysAudio
                        },
                        p,
                    )),
                "Play this clip",
            )
            .on_click(cx.listener({
                let src = s.clone();
                move |this, _, window, cx| this.toggle_clip(turn, src.clone(), window, cx)
            }))
            .into_any_element(),
        );
    }
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(6.))
        .mt(space::S2)
        .child(
            div()
                .mr(px(4.))
                .font_weight(theme::W_MEDIUM)
                .text_size(text::XS)
                .text_color(p.text_subtle)
                .child("sources"),
        )
        .children(kids)
}
