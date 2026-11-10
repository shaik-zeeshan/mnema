//! One answer's sources row (`ChatSources.svelte`): compact frame chips (a
//! thumbnail slot, app, time), a host chip for a frame with a captured page,
//! and audio chips. A frame chip peeks in place (FrameDetailModal).

use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, relative, Context, Div, SharedString};

use super::{format, Chat, Source};
use crate::theme::{self, radius, space, text, Palette, FONT_MONO};
use crate::ui::chip::{chip, ChipKind};
use crate::ui::dialog::Modal;
use crate::ui::{alpha, icon, nid, with_tip, Button};

/// `parseCapturedAt` + the weekday clock; the raw text when it doesn't parse.
fn when(iso: &str, now: time::OffsetDateTime) -> String {
    match time::OffsetDateTime::parse(iso, &time::format_description::well_known::Rfc3339) {
        Ok(d) => format::weekday_clock((d.unix_timestamp_nanos() / 1_000_000) as i64, now),
        Err(_) => iso.to_string(),
    }
}

/// The 44×28 thumbnail slot: three placeholder lines until previews load.
/// shortcut: placeholder lines, not the frame's thumbnail (`get_frame_scrub_previews`
/// isn't read here; the peek shows the real image).
fn thumb(p: &Palette) -> Div {
    div()
        .w(px(44.))
        .h(px(28.))
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(3.))
        .p(px(4.))
        .rounded(px(4.))
        .bg(p.surface_subtle)
        .border_1()
        .border_color(p.border)
        .overflow_hidden()
        .children([0.8, 0.55, 0.7].map(|w| {
            div()
                .h(px(2.))
                .w(relative(w))
                .rounded(px(1.))
                .bg(alpha(p.fg, 0.18))
        }))
}

pub fn row(turn: i64, sources: &[Source], p: &Palette, cx: &mut Context<Chat>) -> Div {
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
                    .child(thumb(p))
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
                    .on_click(cx.listener(move |this, _, _, cx| {
                        // A frame source always carries its id; without one there is
                        // nothing to peek or land on.
                        if src.frame_id.is_some() {
                            this.peek_frame(src.clone(), cx);
                        }
                    })),
                tip,
            )
            .into_any_element(),
        );
        if let Some(url) = &s.url {
            let host = url.split('/').next().unwrap_or(url).to_string();
            kids.push(
                with_tip(
                    div()
                        .id(nid(&format!("{base}-u"), i as u64))
                        .role(gpui_kit::Role::Link)
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
            // shortcut: no inline ReceiptViewer in Chat yet (Insights has one in
            // `insights/journal/receipt.rs`); the chip is a label until it is shared.
            .on_click(|_, _, _| log::debug!("chat: audio source has no inline player yet"))
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

/// FrameDetailModal: the frame's image, its app, window and time, the
/// recognised text, and the way into the Timeline.
/// shortcut: no OCR box overlay or match outlines here (the text shows as a
/// block); the Timeline, one click away, draws both.
pub fn frame_modal(
    chat: &Chat,
    s: &Source,
    p: &Palette,
    cx: &mut Context<Chat>,
) -> impl IntoElement {
    let now = crate::ui::clock::now(cx);
    let close = cx.entity().downgrade();
    let frame_id = s.frame_id;
    let image = chat.peek_data.image.clone().map(|path| {
        div()
            .id("chat-frame-image")
            .w_full()
            .h(px(280.))
            .rounded(radius::MD)
            .overflow_hidden()
            .bg(p.surface_subtle)
            .child(
                gpui_kit::img(path)
                    .size_full()
                    .object_fit(gpui_kit::ObjectFit::Contain),
            )
    });
    let recognised = chat.peek_data.text.clone().map(|t| {
        div()
            .id("chat-frame-text")
            .max_h(px(140.))
            .overflow_y_scroll()
            .p(space::S3)
            .rounded(radius::MD)
            .bg(p.surface_subtle)
            .font_family(FONT_MONO)
            .text_size(text::BASE)
            .text_color(p.text_muted)
            .child(SharedString::from(t))
    });
    Modal::new(
        "chat-frame-modal",
        s.app_name
            .clone()
            .unwrap_or_else(|| "Captured frame".into()),
    )
    .child(
        div()
            .flex()
            .flex_col()
            .gap(space::S3)
            .p(space::S5)
            .text_size(text::MD)
            .text_color(p.text)
            .children(image)
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
                    .child(SharedString::from(when(&s.started_at, now))),
            )
            .children(recognised)
            .child(
                div().child(
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
        let _ = close.update(cx, |c, cx| {
            c.peek = None;
            cx.notify();
        });
    })
}
