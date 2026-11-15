//! What shows instead of (or over) the results: Recent + Try (`RecentRows`),
//! first run / failed / no results with ways out (`EmptyStates`), the one notice
//! line over partial results, the paused-results notice, and the pressed pill's
//! popover (`FilterPicker`).

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, AnyElement, Context, Div, HighlightStyle, Role, SharedString, StyledText};

use super::rules::{NoticeIcon, WayOut};
use super::tokens::{parse_natural, pill_label, pill_operators, Pill, Src};
use super::view::{act, section, tipped};
use super::{EmptyState, QuickRecall};
use crate::theme::{self, radius, size, space, text, Palette, FONT_MONO};
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::{icon, notice, popover, Button, Tone};

/// The popover's floor, so the bar grows to hold it over a short list.
pub const POP_MIN_H: f32 = 300.;
const ROW_H: f32 = 34.;

fn row_base(id: (&'static str, usize), selected: bool, p: &Palette) -> gpui_kit::Stateful<Div> {
    div()
        .id(id)
        .role(Role::ListBoxOption)
        .aria_selected(selected)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(10.))
        .h(px(ROW_H))
        .px(px(10.))
        .rounded(radius::MD)
        .text_color(p.text_subtle)
        .when(selected, |d| d.bg(p.wash_strong))
        .when(!selected, |d| d.hover(|s| s.bg(p.wash)))
}

fn time_cell(t: String, selected: bool, p: &Palette) -> Div {
    div()
        .min_w(px(64.))
        .flex()
        .justify_end()
        .font_family(FONT_MONO)
        .text_size(text::SM)
        .text_color(if selected { p.text } else { p.text_subtle })
        .child(t)
}

impl QuickRecall {
    pub(super) fn recent_rows(&self, cx: &mut Context<Self>, p: &Palette) -> Div {
        let now = self.now(cx);
        let apps = self.apps.clone().unwrap_or_default();
        let tint = |text: &str| {
            let mut hl = Vec::new();
            for s in parse_natural(text, now, &apps).spans {
                hl.push((
                    s.start..s.end,
                    HighlightStyle {
                        background_color: Some(p.wash_strong),
                        color: Some(p.text_strong),
                        ..Default::default()
                    },
                ));
            }
            StyledText::new(text.to_string()).with_highlights(hl)
        };
        let mut rows: Vec<(String, &'static str, String)> = self
            .recent
            .list
            .iter()
            .map(|q| {
                let glyph = if q.ask {
                    "icons/sparkle.svg"
                } else {
                    "icons/rotate-ccw-clock.svg"
                };
                (q.text.clone(), glyph, super::rules::ago_label(q.at, now))
            })
            .collect();
        let n_recent = rows.len();
        rows.extend(self.tries().into_iter().map(|(t, ask)| {
            (
                t,
                if ask {
                    "icons/sparkle.svg"
                } else {
                    "icons/search.svg"
                },
                if ask { "ask".into() } else { String::new() },
            )
        }));
        let mut col = div()
            .id("qr-recent")
            .role(Role::ListBox)
            .aria_label("Recent and suggested searches")
            .flex()
            .flex_col();
        for (i, (t, glyph, when)) in rows.into_iter().enumerate() {
            if i == 0 && n_recent > 0 {
                col = col.child(section("Recent", None, None, p));
            }
            if i == n_recent {
                col = col.child(section("Try", None, None, p));
            }
            let sel = self.recent.index == i;
            col = col.child(
                row_base(("qr-recent-row", i), sel, p)
                    .child(icon(glyph, px(14.)))
                    .child(
                        tipped(("qr-recent-text", i), &t)
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_size(text::MD)
                            .text_color(if sel { p.text_strong } else { p.text })
                            .child(tint(&t)),
                    )
                    .child(time_cell(when, sel, p))
                    .on_mouse_move(cx.listener(move |qr, _, _, cx| {
                        if qr.recent.index != i {
                            qr.recent.index = i;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |qr, _, _, cx| {
                        qr.use_recent(i, cx);
                        qr.refresh(cx);
                    })),
            );
        }
        div().child(col)
    }

    /// The one line over results that may be partial (or under "no results").
    pub(super) fn notice_line(
        &self,
        always: bool,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> Option<AnyElement> {
        if !always && !(self.results.has_results() && self.empty_state().is_none()) {
            return None;
        }
        let r = &self.results;
        let n = super::rules::search_notice(
            self.index_status.as_ref(),
            r.semantic_degraded,
            r.untranscribed,
            self.recording,
            crate::ui::clock::now_ms(cx),
        )?;
        let tone = if n.warn {
            Tone::Warn
        } else if n.info {
            Tone::Info
        } else {
            Tone::Neutral
        };
        let glyph: SharedString = match n.icon {
            NoticeIcon::Info => IconName::Info.path(),
            NoticeIcon::Off => "icons/ban.svg".into(),
            NoticeIcon::Alert => IconName::TriangleAlert.path(),
        };
        let mut parts: Vec<AnyElement> = vec![div().child(n.text).into_any_element()];
        if n.settings {
            parts.push(
                Button::new("qr-meaning-settings", "Turn on in Settings →")
                    .ghost()
                    .inline()
                    .on_click(act(cx, |qr, cx| {
                        super::panel::open_settings("semanticSearch", cx);
                        qr.dismiss(cx);
                    }))
                    .into_any_element(),
            );
        }
        let line = notice::inline(
            tone,
            Some(icon(glyph, size::ICON_SM).into_any_element()),
            parts,
            p,
        )
        .text_size(text::SM);
        Some(
            div()
                .flex()
                .items_center()
                .min_h(px(30.))
                .pt(px(2.))
                .px(px(10.))
                .pb(px(6.))
                .child(line)
                .into_any_element(),
        )
    }

    /// Results paused: the backend rejected an operator.
    pub(super) fn paused_notice(&self, cx: &mut Context<Self>, p: &Palette) -> Option<AnyElement> {
        if !self.results_paused() {
            return None;
        }
        let e = self.results.parse_errors.first()?;
        let friendly = super::rules::friendly_parse_error(e);
        let (why, hint) = match friendly.find(". ") {
            Some(i) => (
                friendly[..i].to_string(),
                Some(friendly[i + 2..].to_string()),
            ),
            None => (friendly.trim_end_matches('.').to_string(), None),
        };
        let bad = e.token.clone();
        let acts = if bad.contains(':') {
            vec![Button::new("qr-as-text", "Search it as text")
                .ghost()
                .sm()
                .on_click(act(cx, move |qr, cx| {
                    let plain = bad[bad.find(':').map_or(0, |i| i + 1)..].to_string();
                    let t = qr.text.replace(&bad, &plain);
                    qr.set_text(&t, cx);
                }))
                .into_any_element()]
        } else {
            Vec::new()
        };
        let n = notice::notice(
            Tone::Warn,
            IconName::Calendar.path(),
            format!("Results paused — {why}"),
            hint.map(Into::into),
            acts,
            p,
        );
        Some(
            div()
                .mt(px(6.))
                .mx(px(2.))
                .mb(px(4.))
                .child(n)
                .into_any_element(),
        )
    }

    pub(super) fn empty_states(&self, cx: &mut Context<Self>, p: &Palette) -> Div {
        let center = |el: Div| {
            div()
                .py(space::S5)
                .px(space::S4)
                .flex()
                .justify_center()
                .child(el)
        };
        match self.empty_state() {
            Some(EmptyState::First) => center(empty(
                EmptyLayout::Center,
                Tone::Neutral,
                IconName::Inbox.path(),
                "Nothing captured yet",
                "Quick Recall searches what Mnema records. Start recording and this fills in as you work.",
                vec![Button::new("qr-open-mnema", "Open Mnema")
                    .sm()
                    .on_click(act(cx, |qr, cx| {
                        cx.defer(|cx| crate::window::toggle_main(cx));
                        qr.dismiss(cx);
                    }))
                    .into_any_element()],
                p,
            )),
            Some(EmptyState::Error) => center(empty(
                EmptyLayout::Center,
                Tone::Danger,
                IconName::TriangleAlert.path(),
                "Search failed",
                "The search index didn’t answer. Nothing is lost — try again.",
                vec![Button::new("qr-retry", "Retry")
                    .sm()
                    .icon("icons/rotate-ccw.svg")
                    .on_click(act(cx, |qr, cx| {
                        let q = qr.results.query.clone();
                        qr.run_search(q, cx);
                    }))
                    .into_any_element()],
                p,
            )),
            Some(EmptyState::None) => {
                let text = self.parse.residual.trim().to_string();
                let pills = self.effective_pills();
                let mut line = String::from("Nothing ");
                let mut hl = Vec::new();
                let bold = HighlightStyle { color: Some(p.text_strong), font_weight: Some(theme::W_SEMIBOLD), ..Default::default() };
                if text.is_empty() {
                    line.push_str("was captured");
                } else {
                    line.push_str("matches ");
                    let s = line.len();
                    line.push_str(&format!("“{text}”"));
                    hl.push((s..line.len(), bold));
                }
                if !pills.is_empty() {
                    line.push_str(" in ");
                    for (i, pl) in pills.iter().enumerate() {
                        if i > 0 {
                            line.push_str(" · ");
                        }
                        let s = line.len();
                        line.push_str(&pill_label(pl));
                        hl.push((s..line.len(), bold));
                    }
                }
                line.push('.');
                if !self.ways.is_empty() {
                    line.push_str(" Closest ways out:");
                }
                let mut col = div()
                    .flex()
                    .flex_col()
                    .child(div().pt(px(14.)).px(px(12.)).pb(px(4.)).text_size(text::PROSE).text_color(p.text_muted).child(StyledText::new(line).with_highlights(hl)))
                    .children(self.notice_line(true, cx, p));
                for (i, (way, n)) in self.ways.iter().enumerate() {
                    let sel = self.way_sel == i;
                    let glyph = if matches!(way, WayOut::Ask { .. }) { "icons/sparkle.svg" } else { "icons/arrow-right.svg" };
                    let when = match n {
                        None => "⇥".to_string(),
                        Some(n) => format!("{n} moment{}", if n == "1" { "" } else { "s" }),
                    };
                    col = col.child(
                        row_base(("qr-way", i), sel, p)
                            .child(icon(glyph, px(14.)))
                            .child(tipped(("qr-way-label", i), way.label()).flex_1().min_w_0().overflow_hidden().text_ellipsis().whitespace_nowrap().text_size(text::MD).font_weight(theme::W_MEDIUM).text_color(p.text_strong).child(way.label().to_string()))
                            .child(time_cell(when, sel, p))
                            .on_mouse_move(cx.listener(move |qr, _, _, cx| {
                                if qr.way_sel != i {
                                    qr.way_sel = i;
                                    cx.notify();
                                }
                            }))
                            .on_click(cx.listener(move |qr, _, _, cx| {
                                qr.apply_way(i, cx);
                                qr.refresh(cx);
                            })),
                    );
                }
                col
            }
            None => div(),
        }
    }

    /// The pressed pill's popover: its operators, the alternatives (with counts),
    /// Edit as text and Remove ⌫. It hangs under its pill.
    pub(super) fn filter_picker(&self, cx: &mut Context<Self>, p: &Palette) -> impl IntoElement {
        let Some(pill) = self.filters.pill.and_then(|i| self.pills.get(i)).cloned() else {
            return div().into_any_element();
        };
        let current = pill_operators(&pill);
        // Hang under the pill: the field row inset (13) + chip (30) + gap (10), less the
        // body inset (8), then the pills before it.
        // shortcut: pill widths are estimated from their labels (no layout read-back).
        let left: f32 = 13. + 30. + 10. - 8.
            + self.pills[..self.filters.pill.unwrap_or(0)]
                .iter()
                .map(|pl| pill_width(pl) + 6.)
                .sum::<f32>();
        let options = self.popover_options();
        let counts = self.filters.counts.clone().unwrap_or_default();
        let items = options.into_iter().enumerate().map(|(i, o)| {
            let active = self.filters.index == i;
            let on = o.op == current;
            let count = counts
                .iter()
                .find(|(k, _)| *k == o.op)
                .map(|(_, n)| n.clone());
            let lead: AnyElement = match &o.pill {
                None => icon("icons/x.svg", size::ICON_SM).into_any_element(),
                Some(Pill::App(a)) => self.app_glyph(None, a, 15., None, p),
                Some(Pill::Source(Src::Mic)) => {
                    icon(IconName::Mic.path(), size::ICON_SM).into_any_element()
                }
                Some(Pill::Source(Src::System)) => {
                    icon("icons/speaker.svg", size::ICON_SM).into_any_element()
                }
                Some(Pill::Source(Src::Screen)) => {
                    icon("icons/monitor.svg", size::ICON_SM).into_any_element()
                }
                Some(Pill::Date { .. }) => {
                    icon(IconName::Calendar.path(), size::ICON_SM).into_any_element()
                }
            };
            let o2 = o.clone();
            div()
                .id(("qr-pop-opt", i))
                .test_support()
                .role(Role::MenuItem)
                .flex()
                .items_center()
                .gap(space::S2)
                .px(px(10.))
                .py(px(7.))
                .mx(px(4.))
                .rounded(radius::SM)
                .text_size(text::BASE)
                .cursor_pointer()
                .text_color(if active { p.text_strong } else { p.text })
                .when(active, |d| d.bg(p.wash_strong))
                .child(lead)
                .child(
                    tipped(("qr-pop-opt-label", i), &o.label)
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(o.label.clone()),
                )
                .children(count.map(|c| {
                    div()
                        .font_family(FONT_MONO)
                        .text_size(text::XS)
                        .text_color(p.text_subtle)
                        .child(c)
                }))
                .child(
                    div()
                        .flex_none()
                        .w(px(13.))
                        .text_color(p.accent)
                        .when(on, |d| d.child(icon(IconName::Check.path(), size::ICON_SM))),
                )
                .on_mouse_move(cx.listener(move |qr, _, _, cx| {
                    if qr.filters.index != i {
                        qr.filters.index = i;
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |qr, _, _, cx| {
                    qr.apply_option(Some(o2.clone()));
                    qr.refresh(cx);
                }))
        });
        let foot = div()
            .flex()
            .gap(px(4.))
            .p(px(6.))
            .border_t_1()
            .border_color(p.hairline)
            .child(
                Button::new("qr-pop-edit", "Edit as text")
                    .ghost()
                    .sm()
                    .on_click(act(cx, |qr, cx| qr.edit_as_text(cx))),
            )
            .child(div().flex_1())
            .child(
                Button::new("qr-pop-remove", "Remove")
                    .ghost()
                    .sm()
                    .kbd("⌫")
                    .on_click(act(cx, |qr, _| qr.remove_pill(None))),
            );
        popover::surface(p)
            .id("qr-pop")
            .role(Role::Dialog)
            .aria_label("Edit filter")
            .ml(px(left.max(0.)))
            .w(px(240.)) // shortcut: CSS shrinks to fit up to 320; GPUI pins the 240 floor
            .pt(px(4.))
            .flex()
            .flex_col()
            .child(
                div()
                    .pt(px(8.))
                    .px(px(14.))
                    .pb(px(4.))
                    .font_family(FONT_MONO)
                    .text_size(text::XS)
                    .text_color(p.text_subtle)
                    .child(current.clone()),
            )
            .child(crate::ui::scroll::area(
                div()
                    .id("qr-pop-menu")
                    .role(Role::Menu)
                    .flex()
                    .flex_col()
                    .max_h(px(232.))
                    .overflow_y_scroll()
                    .child(div().flex().flex_col().children(items)),
            ))
            .child(foot)
            .on_mouse_down_out(cx.listener(|qr, _, _, cx| {
                qr.filters.popover = false;
                cx.notify();
            }))
            .into_any_element()
    }
}

/// A pill's drawn width, to hang the popover under it (12 px type, 9 px pad, icon 15 + gap 6).
fn pill_width(p: &Pill) -> f32 {
    let label = pill_label(p);
    18. + 15. + 6. + label.chars().count() as f32 * 6.6 + 2.
}
