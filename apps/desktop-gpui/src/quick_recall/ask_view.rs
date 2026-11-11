//! The ask slot's drawing (`AskPane.svelte`): "Press ↵ to ask" (the gate), a
//! notice when Ask AI can't answer, or the answer streaming in place (prose via
//! Chat's `prose`, bars / dossier / timeline blocks) with its numbered evidence
//! rows under it, then the moments it didn't cite.

use capture_types::AnswerBlock;
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, relative, AnyElement, Context, Div, Role, SharedString};

use super::model::Cite;
use super::results::{marked, source_glyph};
use super::rules::NoticeTone;
use super::view::{act, section};
use super::{AskView, QuickRecall};
use crate::chat::{blocks, prose};
use crate::theme::{self, radius, size, space, text, Palette, FONT_MONO};
use crate::ui::{inset, kbd, notice, skeleton, Button, Tone};

/// The answer never scrolls: past this it fades and points to Chat.
const CLIP_H: f32 = 148.;

fn live_tag(label: String, idle: bool, p: &Palette) -> Div {
    div()
        .ml_auto()
        .flex()
        .items_center()
        .gap(px(7.))
        .text_size(text::BASE)
        .font_weight(theme::W_MEDIUM)
        .text_color(p.text_muted)
        .child(div().size(px(6.)).rounded_full().map(|d| {
            if idle {
                d.bg(p.chart_grey_4)
            } else {
                d.bg(p.accent)
                    .shadow(vec![crate::ui::spread(p.accent_glow, px(3.))])
            }
        }))
        .child(label)
}

impl QuickRecall {
    pub(super) fn ask_slot(&mut self, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        if self.ask_view() == Some(AskView::Gate) {
            let e = self.engine();
            let why = if e.local
                || matches!(
                    self.settings
                        .as_ref()
                        .map(|s| &s.access.quick_recall_auto_answer),
                    Some(capture_types::QuickRecallAutoAnswer::Never)
                ) {
                "Answers only when you ask"
            } else {
                "Cloud model — answers only when you ask"
            };
            return div()
                .id("qr-gate")
                .test_support()
                .role(Role::Button)
                .flex()
                .items_center()
                .gap(px(10.))
                .w_full()
                .py(px(10.))
                .pl(px(14.))
                .pr(px(10.))
                .rounded(radius::LG)
                .bg(p.wash_strong)
                .cursor_pointer()
                .child(
                    div()
                        .text_color(p.text_subtle)
                        .child(crate::ui::icon(IconName::Sparkle.path(), size::ICON)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .text_size(text::MD)
                        .font_weight(theme::W_MEDIUM)
                        .text_color(p.text)
                        .child("Press")
                        .child(kbd("↵", p))
                        .child("to ask")
                        .child(
                            div()
                                .font_weight(theme::W_SEMIBOLD)
                                .text_color(p.text_strong)
                                .child(e.model.unwrap_or_else(|| "Ask AI".into())),
                        ),
                )
                .child(
                    div()
                        .ml_auto()
                        .text_size(text::SM)
                        .text_color(p.text_subtle)
                        .whitespace_nowrap()
                        .child(why),
                )
                .on_click(cx.listener(|qr, _, _, cx| {
                    qr.ask_now(cx);
                }))
                .into_any_element();
        }
        if let Some(n) = self.ask_notice() {
            let (tone, glyph): (Tone, SharedString) = match n.tone {
                NoticeTone::Off => (Tone::Off, "icons/ban.svg".into()),
                NoticeTone::Warn => (Tone::Warn, "icons/cloud-off.svg".into()),
                NoticeTone::Danger => (Tone::Danger, IconName::TriangleAlert.path()),
                NoticeTone::Plain => (Tone::Neutral, IconName::Sparkle.path()),
            };
            let acts = n
                .action
                .map(|(label, primary)| {
                    let b = Button::new("qr-notice-act", label)
                        .sm()
                        .on_click(act(cx, |qr, cx| {
                            super::panel::open_settings("intelligence", cx);
                            qr.dismiss(cx);
                        }));
                    vec![if primary { b } else { b.ghost() }.into_any_element()]
                })
                .unwrap_or_default();
            return div()
                .flex_none()
                .p(px(6.))
                .rounded(radius::LG)
                .bg(p.wash_strong)
                .child(notice::notice(
                    tone,
                    glyph,
                    n.title,
                    Some(n.text.into()),
                    acts,
                    p,
                ))
                .into_any_element();
        }
        let mut col = div().flex().flex_col().child(self.answer(cx, p));
        let sources = self.ask.sources();
        let more = self.more_moments();
        if sources.len() + more.len() > 0 {
            col = col.child(section(
                "Evidence",
                None,
                Some(format!("{} cited · {} more", sources.len(), more.len())),
                p,
            ));
            for (i, s) in sources.iter().enumerate() {
                col = col.child(self.cite_row(i, s, cx, p));
            }
            for (k, r) in more.iter().enumerate() {
                let row = sources.len() + k + 1;
                col =
                    col.child(self.result_row(("qr-ask-row", row), r, self.ask.sel == row, cx, p));
            }
        }
        col.into_any_element()
    }

    fn answer(&mut self, cx: &mut Context<Self>, p: &Palette) -> Div {
        let a = &self.ask;
        let waiting = self.ask_waiting();
        let forced = self.forced == Some(super::tokens::Forced::Ask);
        let live = a.live();
        let mut head = div()
            .flex()
            .items_center()
            .gap(space::S2)
            .min_h(px(26.))
            .text_size(text::MD)
            .font_weight(theme::W_SEMIBOLD)
            .text_color(p.text_strong)
            .child(
                div()
                    .text_color(p.accent)
                    .child(crate::ui::icon(IconName::Sparkle.path(), size::ICON)),
            )
            .child("Answer");
        let turn = a.turn.as_ref();
        head = match (waiting || turn.is_none(), turn) {
            (true, _) => head.child(live_tag(
                if forced {
                    "press ↵ to ask"
                } else {
                    "answers when you pause"
                }
                .into(),
                true,
                p,
            )),
            (_, Some(t)) if t.view.phase == "thinking" => head.child(live_tag(
                t.view
                    .live_activity
                    .as_ref()
                    .map_or("Thinking…".into(), |e| e.label.clone()),
                false,
                p,
            )),
            _ if live => head.child(
                div().ml_auto().child(
                    Button::new("qr-stop", "Stop")
                        .ghost()
                        .sm()
                        .icon(crate::assets::STOP_SQUARE)
                        .kbd("esc")
                        .on_click(act(cx, |qr, cx| qr.ask_stop(cx))),
                ),
            ),
            (_, Some(t)) if t.view.phase == "error" => head.child(
                div()
                    .ml_auto()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .text_size(text::BASE)
                    .font_weight(theme::W_REGULAR)
                    .text_color(p.danger_text)
                    .child(crate::ui::icon(
                        IconName::TriangleAlert.path(),
                        size::ICON_SM,
                    ))
                    .child("Stopped partway")
                    .child(
                        Button::new("qr-retry-ask", "Retry")
                            .ghost()
                            .sm()
                            .icon("icons/rotate-ccw.svg")
                            .on_click(act(cx, |qr, cx| {
                                let q = qr.ask.asked.clone();
                                qr.ask_start(q, cx);
                            })),
                    ),
            ),
            _ => {
                let n = a.sources().len();
                let from = if a.stopped_early {
                    "stopped".to_string()
                } else if n == 0 {
                    String::new()
                } else {
                    format!("from {n} moment{}", if n == 1 { "" } else { "s" })
                };
                head.child(
                    div()
                        .font_family(FONT_MONO)
                        .font_weight(theme::W_REGULAR)
                        .text_size(text::SM)
                        .text_color(p.text_subtle)
                        .child(from),
                )
                .when(a.thread_id.is_some(), |h| {
                    h.child(
                        div().ml_auto().child(
                            Button::new("qr-continue", "Continue in Chat")
                                .sm()
                                .when(a.sel == 0, |b| b.primary())
                                .icon("icons/message-circle.svg")
                                .kbd("↵")
                                .on_click(act(cx, |qr, cx| qr.continue_in_chat(cx))),
                        ),
                    )
                })
            }
        };
        let mut art = div()
            .id("qr-answer")
            .role(Role::Article)
            .flex()
            .flex_col()
            .flex_none()
            .pt(px(14.))
            .px(px(16.))
            .pb(px(12.))
            .rounded(radius::LG)
            .when(a.sel == 0, |d| d.bg(p.wash_strong))
            .child(head);
        if forced && turn.is_none() {
            return div().child(art);
        }
        let body = match turn {
            Some(t)
                if !(t.view.phase == "thinking"
                    || (t.view.blocks.is_empty()
                        && t.view.phase != "error"
                        && t.view.phase != "done")) =>
            {
                let mut prose_col = div().flex().flex_col().gap(px(10.)).min_w_0();
                let mut timelines = Vec::new();
                let last = t.view.blocks.len().saturating_sub(1);
                for (bi, b) in t.view.blocks.iter().enumerate() {
                    prose_col = match b {
                        AnswerBlock::Prose { markdown } => prose_col.child(
                            prose::prose_on_link(
                                ("qr-prose", bi),
                                markdown,
                                live && bi == last,
                                p,
                                self.link_opener(),
                            )
                            .text_size(text::PROSE)
                            .line_height(relative(1.65)),
                        ),
                        AnswerBlock::Bars { title, items } => prose_col.child(blocks::panel(
                            title.as_deref(),
                            None,
                            blocks::bars(items, p).into_any_element(),
                            p,
                        )),
                        AnswerBlock::Dossier { items } => {
                            prose_col.child(blocks::dossier(items, p))
                        }
                        AnswerBlock::Timeline { title, items } => {
                            timelines.push(blocks::timeline(title.as_deref(), items, p));
                            prose_col
                        }
                    };
                }
                if t.view.phase == "error" {
                    let raw = t.view.error_message.clone().unwrap_or_default();
                    let raw = raw.trim();
                    // `turnErrorCopy`: a reason code is never shown raw.
                    let code = !raw.is_empty()
                        && !raw.contains(char::is_whitespace)
                        && raw.split(':').next().is_some_and(|h| {
                            h.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
                        });
                    let text =
                        match crate::insights::model::reason_copy(raw, self.settings.as_ref()) {
                            crate::insights::model::Engine::Unreachable(t)
                            | crate::insights::model::Engine::Fix(t, _)
                                if code =>
                            {
                                t
                            }
                            _ if raw.is_empty() => "The engine couldn't answer.".to_string(),
                            _ => raw.to_string(),
                        };
                    prose_col = prose_col
                        .child(div().text_size(text::PROSE).text_color(p.text).child(text));
                }
                if t.view.phase == "done" && t.view.blocks.is_empty() {
                    prose_col = prose_col.child(
                        div()
                            .text_size(text::BASE)
                            .text_color(p.text_muted)
                            .child("No answer came back. Press ↵ in a moment, or ask again."),
                    );
                }
                // Measure the answer's natural height behind the clip.
                let cell = a.prose_h.clone();
                let prose_col = prose_col.relative().child(
                    gpui_kit::canvas(
                        move |bounds, window, _| {
                            let h = f32::from(bounds.size.height);
                            if (cell.get() - h).abs() > 0.5 {
                                cell.set(h);
                                window.refresh();
                            }
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left_0()
                    .right_0(),
                );
                let clip = div()
                    .max_h(px(CLIP_H))
                    .overflow_hidden()
                    .min_w_0()
                    .child(prose_col);
                // Past the clip the glance panel never scrolls: it points to Chat.
                let clipped = a.prose_h.get() > CLIP_H + 1.;
                let more = (clipped && !live && a.thread_id.is_some()).then(|| {
                    div()
                        .id("qr-ans-more")
                        .test_support()
                        .role(Role::Link)
                        .mt(px(6.))
                        .cursor_pointer()
                        .font_weight(theme::W_MEDIUM)
                        .text_size(text::BASE)
                        .text_color(p.accent)
                        .child("Read the full answer in Chat →")
                        .on_click(cx.listener(|qr, _, _, cx| qr.continue_in_chat(cx)))
                });
                div()
                    .flex()
                    .gap(px(18.))
                    .items_start()
                    .child(div().flex_1().min_w_0().child(clip).children(more))
                    .children((!timelines.is_empty()).then(|| {
                        div()
                            .flex_none()
                            .w(px(320.))
                            .pt(px(10.))
                            .px(px(12.))
                            .pb(px(6.))
                            .rounded(radius::MD)
                            .bg(p.surface)
                            .border_1()
                            .border_color(p.border)
                            .children(timelines)
                    }))
            }
            _ => div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .pt(px(4.))
                .pb(px(2.))
                .child(skeleton::line(0.92, p).h(px(11.)))
                .child(skeleton::line(0.78, p).h(px(11.)))
                .child(skeleton::line(0.54, p).h(px(11.))),
        };
        art = art.child(div().mt(px(10.)).child(body));
        div().child(art)
    }

    fn cite_row(
        &self,
        i: usize,
        s: &Cite,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> impl IntoElement {
        let row = i + 1;
        let selected = self.ask.sel == row;
        let hot = self.ask.cite == row;
        let now = self.now(cx);
        let when = crate::chat::format::weekday_clock(super::model::ms(&s.started_at), now);
        let title = s
            .window_title
            .clone()
            .filter(|t| !t.is_empty())
            .or_else(|| s.app_name.clone())
            .unwrap_or_else(|| {
                if s.kind == "audio" {
                    "Recording".into()
                } else {
                    "Screen".into()
                }
            });
        let audio = s.kind == "audio";
        let detail: AnyElement = match &s.snippet {
            Some(sn) if !sn.is_empty() => marked(sn, audio, p).into_any_element(),
            _ => {
                let t = if audio {
                    if s.mic() {
                        "microphone".to_string()
                    } else {
                        "system audio".to_string()
                    }
                } else {
                    s.url.clone().unwrap_or_else(|| {
                        if s.window_title.is_some() {
                            s.app_name.clone().unwrap_or_default()
                        } else {
                            String::new()
                        }
                    })
                };
                div().child(t).into_any_element()
            }
        };
        let glyph = if audio {
            source_glyph(s.mic(), p)
        } else {
            self.app_glyph(None, s.app_name.as_deref().unwrap_or("?"), 18., p)
        };
        let looks = self.looks();
        div()
            .id(("qr-cite", row))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_selected(selected)
            .group("qr-cite")
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .h(px(34.))
            .pl(px(10.))
            .pr(px(4.))
            .rounded(radius::MD)
            .text_color(p.text)
            .when(selected, |d| d.bg(p.wash_strong))
            .when(!selected && hot, |d| d.bg(p.wash))
            .when(!selected, |d| d.hover(|st| st.bg(p.wash)))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(18.))
                    .rounded(radius::XS)
                    .font_family(FONT_MONO)
                    .text_size(text::XS)
                    .font_weight(theme::W_MEDIUM)
                    .map(|d| {
                        if hot {
                            d.bg(p.text_strong).text_color(p.bg)
                        } else {
                            d.text_color(p.text_muted)
                                .shadow(vec![inset(p.border_strong, px(1.))])
                        }
                    })
                    .child(row.to_string()),
            )
            .child(div().flex_none().child(glyph))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .gap(space::S2)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(text::MD)
                    .child(
                        div()
                            .flex_none()
                            .font_weight(theme::W_MEDIUM)
                            .text_color(p.text_strong)
                            .child(title),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_color(p.text_muted)
                            .child(detail),
                    ),
            )
            .child(
                div()
                    .min_w(px(64.))
                    .flex()
                    .justify_end()
                    .font_family(FONT_MONO)
                    .text_size(text::SM)
                    .text_color(if selected { p.text } else { p.text_subtle })
                    .child(when),
            )
            .child(
                div()
                    .w(px(26.))
                    .flex()
                    .justify_center()
                    .invisible()
                    .when(selected, |d| d.visible())
                    .group_hover("qr-cite", |st| st.visible())
                    .child(
                        Button::icon_only(
                            ("qr-cite-look", row),
                            IconName::Eye.path(),
                            "Quick Look",
                        )
                        .ghost()
                        .sm()
                        .on_click(act(cx, move |qr, cx| {
                            qr.look_cited(looks.clone(), i, false, cx)
                        })),
                    ),
            )
            .on_click(cx.listener(move |qr, _, _, cx| {
                qr.open_cite(i, false, cx);
                qr.refresh(cx);
            }))
    }
}
