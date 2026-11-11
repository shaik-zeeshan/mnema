//! Context (`Context.svelte` + `ContextSide.svelte`): what the user told Mnema
//! in their own words. The lede and four counts; the composer (280 chars, a
//! topic, ⌘↵); the standing statements (delete; older ones past the prompt cap
//! read "not sent"); beside them what the dossier is built from, the strongest
//! inferred views, the guardrail and the dismissed beliefs (Restore).

use gpui_kit::assets::IconName;
use gpui_kit::component::input::Textarea;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, AnyElement, Context, Div, Role, Window};

use super::charts::swatch;
use super::model::ContextData;
use super::parts::{
    display, h2, inset, label, lede, mono_label, panel, prose, read, sec, split, stat_text,
};
use super::subjects::tiers::pct;
use super::Insights;
use crate::theme::{self, radius, space, text, Palette, FONT_MONO};
use crate::ui::chip::{chip, ChipKind};
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::{clock, icon, kicker, nid, Button, Tone};

const MAX_CHARS: usize = 280;
const PLACEHOLDER: &str =
    "e.g. \u{201c}I'm a designer \u{2014} Figma time is real work even when it looks like browsing.\u{201d}";
const WORDS: [&str; 10] = [
    "Nothing",
    "One thing",
    "Two things",
    "Three things",
    "Four things",
    "Five things",
    "Six things",
    "Seven things",
    "Eight things",
    "Nine things",
];
/// guardrail.rs `SensitiveCategory`, in enum order.
const SENSITIVE: [&str; 5] = [
    "health & mental health",
    "sexuality",
    "religion",
    "politics",
    "other protected",
];

/// The topic swatches, in `context_ops::TOPICS` order.
pub fn topic_colors(p: &Palette) -> [gpui_kit::Hsla; 4] {
    topics(p).map(|t| t.1)
}

/// Topic name + its swatch (`context-topics.ts`).
fn topics(p: &Palette) -> [(&'static str, gpui_kit::Hsla); 4] {
    [
        ("Work", p.cat_creating),
        ("Preferences", p.cat_organizing),
        ("People", p.cat_communication),
        ("Goals", p.cat_learning),
    ]
}

pub fn render(
    this: &mut Insights,
    p: &Palette,
    window: &mut Window,
    cx: &mut Context<Insights>,
) -> AnyElement {
    let Some(d) = this.context.clone() else {
        return div()
            .id("insights-context-loading")
            .child(crate::ui::skeleton::lines(4, p))
            .into_any_element();
    };
    let off = clock::now(cx).offset();
    if std::mem::take(&mut this.ctx_ui.show_dismissed) {
        // Arriving from Subjects' "View dismissed": the archive ends the page.
        this.scroll.scroll_to_bottom();
    }
    let failed = d.authored.is_none();
    let rows = d.authored.clone().unwrap_or_default();
    let n = rows.len();
    let unsent = rows.iter().filter(|r| !r.in_prompt).count();
    let (head, em) = if failed {
        (
            "What you've told Mnema.".to_string(),
            "It steers the dossier, and never fades.",
        )
    } else if n > 0 {
        (
            format!(
                "{} you've told Mnema.",
                WORDS
                    .get(n)
                    .map_or(format!("{n} things"), |w| w.to_string())
            ),
            if unsent > 0 {
                "They steer the dossier first — the newest ones are sent."
            } else {
                "They steer the dossier first, and they never fade."
            },
        )
    } else {
        (
            "Nothing you've told Mnema yet.".into(),
            "What you write here steers the dossier, and never fades.",
        )
    };
    let lede_el = lede("Context · in your own words", p).child(display(head, Some(em.into()), p)).child(prose(
        "Everything else Mnema knows is inferred from activity and scored on confidence. What you write here is taken as given — and outranks an inference that disagrees.",
        p,
    ));
    let inferred = d.inferred.as_ref().map(Vec::len);
    let stats = vec![
        stat_text(
            "Authored",
            &if failed { "—".into() } else { n.to_string() },
            Some(
                if failed {
                    "couldn't load"
                } else {
                    "never fades"
                }
                .into(),
            ),
            p,
        )
        .into_any_element(),
        stat_text(
            "Inferred",
            &inferred.map_or("—".into(), |v| v.to_string()),
            Some("scored · can fade".into()),
            p,
        )
        .into_any_element(),
        stat_text(
            "Dismissed",
            &d.dismissed.len().to_string(),
            Some("won't be re-inferred".into()),
            p,
        )
        .into_any_element(),
        stat_text(
            "Guarded",
            &SENSITIVE.len().to_string(),
            Some("categories never inferred".into()),
            p,
        )
        .into_any_element(),
    ];
    let left = div()
        .flex()
        .flex_col()
        .gap(space::S6)
        .child(composer(this, p, window, cx))
        .child(standing(this, &d, unsent, p, off, cx));
    div()
        .flex()
        .flex_col()
        .child(read(lede_el, stats, this.width, p))
        .child(split(left, side(this, &d, p, cx), this.width).items_start())
        .into_any_element()
}

fn composer(this: &Insights, p: &Palette, _: &mut Window, cx: &mut Context<Insights>) -> Div {
    let len = this.composer.read(cx).value().chars().count();
    let topic_chips = div()
        .flex()
        .gap(px(4.))
        .children(topics(p).into_iter().enumerate().map(|(i, (name, c))| {
            let on = this.draft_topic == Some(name);
            div()
                .id(nid("insights-topic", i as u64))
                .role(Role::Button)
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(24.))
                .px(px(9.))
                .rounded(theme::radius::SM)
                .border_1()
                .border_color(if on { p.border_strong } else { p.border })
                .bg(if on { p.wash_strong } else { p.surface_raised })
                .cursor_pointer()
                .text_size(text::BASE)
                .font_weight(theme::W_MEDIUM)
                .text_color(if on { p.text_strong } else { p.text_muted })
                .child(swatch(c, 8.))
                .child(name)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.draft_topic = if this.draft_topic == Some(name) {
                        None
                    } else {
                        Some(name)
                    };
                    cx.notify();
                }))
        }));
    let provider = this
        .status
        .ai
        .as_ref()
        .and_then(|a| a.default_provider_kind.zip(a.default_model.as_ref()));
    let priv_line = provider.map(|(kind, m)| {
        let name = crate::chat::format::provider_label_by_id(this.status.settings.as_ref().map_or(&[][..], |s| &s.ai_runtime.providers), &m.provider);
        let (ic, t) = if kind.is_local() {
            (IconName::Laptop.path(), format!("Stays on this Mac — {name} forms conclusions locally."))
        } else {
            (IconName::Cloud.path(), format!("Goes to {name} each time Mnema forms conclusions · stays on this Mac with a local model."))
        };
        div().flex().items_center().gap(px(6.)).text_size(text::SM).text_color(p.text_subtle).child(icon(ic, px(13.))).child(t)
    });
    panel(p)
        .bg(p.surface_raised)
        .p(space::S5)
        .flex()
        .flex_col()
        .gap(space::S3)
        .child(kicker("Add context", p))
        .child(
            div()
                .relative()
                .min_h(px(76.))
                .text_size(text::LG)
                .line_height(relative(1.5))
                .text_color(p.text_strong)
                .child(
                    // The kit's multi-line editor insets its text 10 × 8 px; pull it
                    // back so the text (and the drawn placeholder) sit on the kicker's edge.
                    div().mx(px(-10.)).my(px(-8.)).child(
                        Textarea::new(&this.composer)
                            .aria_label("Add context")
                            .appearance(false)
                            .px_0()
                            .py_0()
                            .text_size(text::LG),
                    ),
                )
                .when(len == 0, |d| {
                    d.child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .right(px(72.))
                            .text_color(p.text_faint)
                            .child(PLACEHOLDER),
                    )
                })
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .child(mono_label(format!("{len} / {MAX_CHARS}"), p)),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(space::S3)
                .pt(space::S3)
                .border_t_1()
                .border_color(p.hairline)
                .child(label("Topic", p))
                .child(topic_chips)
                .child(div().flex_1())
                .child(if this.ctx_ui.submitting {
                    Button::new("insights-add-context", "Adding…")
                        .primary()
                        .sm()
                        .spinner(true)
                        .disabled(true)
                } else {
                    Button::new("insights-add-context", "Add context")
                        .primary()
                        .sm()
                        .icon(IconName::Plus.path())
                        .kbd("⌘↵")
                        .disabled(this.composer.read(cx).value().trim().is_empty())
                        .on_click(cx.listener(|this, _, window, cx| this.submit_draft(window, cx)))
                }),
        )
        .children(priv_line)
        .children(this.composer_error.as_ref().map(|e| {
            crate::ui::notice::inline(
                Tone::Danger,
                None,
                vec![
                    div()
                        .child(format!(
                            "Couldn't save that statement — {e}. Your text is still here."
                        ))
                        .into_any_element(),
                    Button::new("insights-add-retry", "Retry")
                        .ghost()
                        .sm()
                        .on_click(cx.listener(|this, _, window, cx| this.submit_draft(window, cx)))
                        .into_any_element(),
                ],
                p,
            )
        }))
}

fn standing(
    this: &Insights,
    d: &ContextData,
    unsent: usize,
    p: &Palette,
    off: time::UtcOffset,
    cx: &mut Context<Insights>,
) -> Div {
    let rows = d.authored.clone().unwrap_or_default();
    let n = rows.len();
    let note = if d.authored.is_none() {
        String::new()
    } else if unsent > 0 {
        format!(
            "{} of {n} statements used — older ones aren't sent",
            n - unsent
        )
    } else {
        format!(
            "{n} statement{}{}",
            if n == 1 { "" } else { "s" },
            if n > 0 { " · all sent" } else { "" }
        )
    };
    let head = sec(
        div()
            .flex()
            .items_end()
            .gap(space::S3)
            .child(h2("Standing context", p))
            .child(mono_label(note, p)),
        None,
    );
    if d.authored.is_none() {
        return div().child(head).child(empty(
            EmptyLayout::Panel,
            Tone::Danger,
            IconName::TriangleAlert.path(),
            "Couldn't load your context.",
            "Reading your statements failed. Nothing is lost.",
            vec![Button::new("insights-context-retry", "Try again")
                .sm()
                .icon(IconName::RotateCcw.path())
                .on_click(cx.listener(|this, _, _, cx| this.reload(cx)))
                .into_any_element()],
            p,
        ));
    }
    if n == 0 {
        return div().child(head).child(empty(
            EmptyLayout::Panel,
            Tone::Neutral,
            IconName::Inbox.path(),
            "No standing context yet.",
            "Add a short statement above — your role, what you're working on, how you work, what you care about. It steers the dossier, and it never fades.",
            Vec::new(),
            p,
        ));
    }
    let tcolor = |t: Option<&str>| {
        topics(p)
            .into_iter()
            .find(|x| Some(x.0) == t)
            .map_or(p.chart_grey_3, |x| x.1)
    };
    div().child(head).children(rows.into_iter().map(|r| {
        let s = r.item;
        let id = s.id;
        let when = if s.updated_at_ms > s.created_at_ms + 1000 {
            format!("edited {}", super::stats::month_day(s.updated_at_ms, off))
        } else {
            format!("authored {}", super::stats::month_day(s.created_at_ms, off))
        };
        let editor = this
            .ctx_ui
            .editor
            .clone()
            .filter(|_| this.ctx_ui.editing == Some(id));
        let editing = editor.is_some();
        div()
            .id(nid("insights-stmt", id as u64))
            .flex()
            .items_start()
            .group("insights-stmt")
            .gap(space::S4)
            .py(px(14.))
            .px(space::S3)
            .border_t_1()
            .border_color(p.hairline)
            .rounded(radius::SM)
            .hover(|s| s.bg(p.wash))
            .when(!r.in_prompt && !editing, |d| d.opacity(0.38))
            .when(editing, |d| d.bg(p.wash))
            .child(
                div()
                    .w(px(120.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .text_size(text::BASE)
                    .font_weight(theme::W_MEDIUM)
                    .line_height(relative(1.7))
                    .text_color(p.text_subtle)
                    .children(s.topic.as_deref().map(|t| {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(swatch(tcolor(Some(t)), 8.))
                            .child(t.to_string())
                    })),
            )
            .when_some(editor, |row, ed| {
                row.child(super::context_ops::edit_body(this, &ed, p, cx))
            })
            .when(!editing, |row| {
                row.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(
                            div()
                                .text_size(text::LG)
                                .line_height(relative(1.45))
                                .text_color(p.text_strong)
                                .child(s.text.clone()),
                        )
                        .child(
                            div()
                                .text_size(text::SM)
                                .line_height(relative(1.))
                                .text_color(p.text_subtle)
                                .child(format!(
                                    "{when} · {}",
                                    if r.in_prompt {
                                        "never fades"
                                    } else {
                                        "not sent"
                                    }
                                )),
                        ),
                )
            })
            .when(!editing, |row| {
                row.child(
                    // Acts show on row hover (`.in-stmt:hover .in-stmt__acts`).
                    div()
                        .flex()
                        .opacity(0.)
                        .group_hover("insights-stmt", |s| s.opacity(1.))
                        .child(
                            Button::icon_only(
                                nid("insights-stmt-edit", id as u64),
                                IconName::Pencil.path(),
                                "Edit",
                            )
                            .ghost()
                            .sm()
                            .on_click(cx.listener(
                                move |this, _, window, cx| this.start_edit(id, window, cx),
                            )),
                        )
                        .child(
                            Button::icon_only(
                                nid("insights-stmt-delete", id as u64),
                                IconName::Trash.path(),
                                "Delete",
                            )
                            .ghost()
                            .sm()
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.delete_statement(id, cx)),
                            ),
                        ),
                )
            })
    }))
}

fn side(this: &Insights, d: &ContextData, p: &Palette, cx: &mut Context<Insights>) -> Div {
    let off = clock::now(cx).offset();
    let rows = d.authored.clone().unwrap_or_default();
    let n = rows.len();
    let inferred = d.inferred.clone().unwrap_or_default();
    let ni = inferred.len();
    let mut by_topic: Vec<(&str, gpui_kit::Hsla, usize)> = topics(p)
        .into_iter()
        .map(|(t, c)| {
            (
                t,
                c,
                rows.iter()
                    .filter(|r| r.item.topic.as_deref() == Some(t))
                    .count(),
            )
        })
        .collect();
    let other = n - by_topic.iter().map(|t| t.2).sum::<usize>();
    if other > 0 {
        by_topic.push(("Other", p.chart_grey_3, other));
    }
    let max_t = by_topic.iter().map(|t| t.2).max().unwrap_or(1).max(1);
    let total = div()
        .flex()
        .gap(px(3.))
        .h(px(8.))
        .mt(space::S3)
        .child(
            div()
                .flex_grow(n.max(1) as f32 * if n == 0 { 0.001 } else { 1. })
                .flex_basis(px(0.))
                .rounded(px(2.))
                .bg(p.text_strong),
        )
        .child(
            div()
                .flex_grow(ni.max(1) as f32 * if ni == 0 { 0.001 } else { 1. })
                .flex_basis(px(0.))
                .rounded(px(2.))
                .bg(p.chart_grey_3),
        );
    let mix = panel(p)
        .p(space::S5)
        .child(kicker("What the dossier is built from", p))
        .child(total)
        .child(
            div()
                .flex()
                .gap(space::S4)
                .mt(space::S2)
                .text_size(text::SM)
                .text_color(p.text_subtle)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .child(swatch(p.text_strong, 8.))
                        .child(format!("authored {n}")),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .child(swatch(p.chart_grey_4, 8.))
                        .child(format!("inferred {ni}")),
                ),
        )
        .when(n > 0, |d| {
            d.child(
                div()
                    .mt(space::S3)
                    .children(by_topic.into_iter().map(|(t, c, k)| {
                        div()
                            .flex()
                            .items_center()
                            .gap(space::S3)
                            .py(px(7.))
                            .border_t_1()
                            .border_color(p.hairline)
                            .text_size(text::BASE)
                            .child(
                                div()
                                    .w(px(100.))
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .child(swatch(c, 8.))
                                    .child(t),
                            )
                            .child(
                                div().flex_1().child(
                                    div()
                                        .h(px(14.))
                                        .w(relative(k as f32 / max_t as f32))
                                        .rounded(px(2.))
                                        .bg(c),
                                ),
                            )
                            .child(
                                div()
                                    .w(px(24.))
                                    .text_right()
                                    .font_family(FONT_MONO)
                                    .text_size(text::SM)
                                    .child(k.to_string()),
                            )
                    })),
            )
        });
    let mut strongest = inferred.clone();
    strongest.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
    strongest.truncate(3);
    let views =
        div()
            .child(sec(
                kicker("Strongest inferred views", p),
                Some(label("by confidence", p).into_any_element()),
            ))
            .child(if strongest.is_empty() {
                empty(
                    EmptyLayout::Compact,
                    Tone::Neutral,
                    IconName::Inbox.path(),
                    "No inferred views yet",
                    "As Mnema forms views, the strongest show here.",
                    Vec::new(),
                    p,
                )
            } else {
                div()
                    .flex()
                    .flex_col()
                    .children(strongest.into_iter().enumerate().map(|(i, c)| {
                        let subject = c.subject.clone();
                        div()
                            .id(nid("insights-steer", i as u64))
                            .role(Role::Button)
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .py(px(8.))
                            .border_t_1()
                            .border_color(p.hairline)
                            .cursor_pointer()
                            .text_color(p.text_strong)
                            .hover(|s| s.text_color(p.accent))
                            .child(
                                div()
                                    .flex()
                                    .gap(space::S2)
                                    .text_size(text::BASE)
                                    .child(div().flex_1().child(c.statement.clone()))
                                    .child(
                                        div()
                                            .font_family(FONT_MONO)
                                            .text_size(text::SM)
                                            .child(format!("{}%", pct(c.confidence))),
                                    ),
                            )
                            .child(div().text_size(text::SM).text_color(p.text_subtle).child(
                                format!(
                                    "{} · inferred — say it here to make it standing",
                                    c.subject
                                ),
                            ))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_subject(subject.clone(), cx)
                            }))
                    }))
            });
    let guard = inset(p)
        .p(space::S5)
        .flex()
        .flex_col()
        .gap(space::S3)
        .text_size(text::BASE)
        .text_color(p.text_muted)
        .child(div().flex().items_center().gap(px(6.)).child(icon(IconName::Lock.path(), px(12.))).child(kicker("Sensitive Category Guardrail", p)))
        .child(div().flex().flex_wrap().gap(px(6.)).children(SENSITIVE.map(|s| chip(s, ChipKind::Neutral, p))))
        .child("Mnema never infers these categories about you. What you write here is sent as written.");
    div()
        .flex()
        .flex_col()
        .gap(space::S6)
        .child(mix)
        .child(views)
        .child(guard)
        .when(
            !d.dismissed.is_empty() || this.ctx_ui.dismissed_error.is_some(),
            |el| {
                el.child(
                    div()
                        .child(sec(
                            kicker("Dismissed beliefs", p),
                            Some(label("won't be re-inferred", p).into_any_element()),
                        ))
                        .children(this.ctx_ui.dismissed_error.clone().map(|e| {
                            crate::ui::notice::inline(
                                Tone::Danger,
                                None,
                                vec![div().child(e).into_any_element()],
                                p,
                            )
                        }))
                        .children(d.dismissed.iter().enumerate().map(|(i, v)| {
                            let (s, st) = (v.subject.clone(), v.statement.clone());
                            div()
                                .flex()
                                .items_center()
                                .gap(space::S3)
                                .py(px(8.))
                                .border_t_1()
                                .border_color(p.hairline)
                                .child(
                                    div()
                                        .flex_1()
                                        .flex()
                                        .flex_col()
                                        .gap(px(2.))
                                        .child(
                                            div()
                                                .text_size(text::BASE)
                                                .text_color(p.text)
                                                .child(v.statement.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_size(text::SM)
                                                .text_color(p.text_subtle)
                                                .child(format!(
                                                    "{} · dismissed {}",
                                                    v.subject,
                                                    super::stats::month_day(v.dismissed_at_ms, off)
                                                )),
                                        ),
                                )
                                .child({
                                    let busy = this.ctx_ui.restoring.as_ref()
                                        == Some(&(v.subject.clone(), v.statement.clone()));
                                    Button::new(
                                        nid("insights-restore", i as u64),
                                        if busy { "Restoring…" } else { "Restore" },
                                    )
                                    .ghost()
                                    .sm()
                                    .spinner(busy)
                                    .disabled(busy)
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.restore(s.clone(), st.clone(), cx)
                                        },
                                    ))
                                })
                        })),
                )
            },
        )
}
