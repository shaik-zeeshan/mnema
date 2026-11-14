//! BarShell (`BarShell.svelte`): the 58 px field row, the body (≤ 640 with the
//! bar and foot) and the 32 px key row. The shell measures itself every frame and
//! asks the panel to match, top edge pinned, so the bar grows downward.

use gpui_kit::prelude::*;
use gpui_kit::{canvas, div, px, AnyElement, App, ClickEvent, Context, Div, Render, Role, Window};

use super::{QuickRecall, CAP_H, FIELD_H, FOOT_H, LOOK_H};
use crate::theme::{self, space, text, Palette, FONT_UI};
use crate::ui::baseline::{self, Line};

/// A click handler that runs `f` on the bar.
pub(super) fn act(
    cx: &Context<QuickRecall>,
    f: impl Fn(&mut QuickRecall, &mut Context<QuickRecall>) + 'static,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let this = cx.entity().downgrade();
    move |_, _, cx| {
        let _ = this.update(cx, |qr, cx| {
            f(qr, cx);
            qr.refresh(cx);
        });
    }
}

/// `.rc-sec`: a section label with a mono count (and an optional right-hand one).
pub(super) fn section(
    label: &str,
    count: Option<String>,
    right: Option<String>,
    p: &Palette,
) -> Div {
    let mono = Line::new(theme::FONT_MONO, text::XS, baseline::PHI);
    let mut items = vec![(
        Line::new(FONT_UI, text::BASE, baseline::PHI),
        div().child(label.to_string()),
    )];
    items.extend(count.map(|c| (mono, div().child(c))));
    items.extend(right.map(|c| (mono, div().ml_auto().child(c))));
    // The mono counts share the label's baseline.
    baseline::row_top(items)
        .gap(space::S2)
        .mx(px(10.))
        .mt(px(10.))
        .mb(px(4.))
        .font_weight(theme::W_MEDIUM)
        .text_color(p.text_subtle)
}

impl QuickRecall {
    fn body(&mut self, window: &mut Window, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        if self.look.is_some() {
            return self.look_view(window, cx, p).into_any_element();
        }
        let ask = self.ask_view();
        let mut col = div().flex().flex_col();
        if ask.is_some() {
            col = col.child(self.ask_slot(cx, p));
        }
        if ask.is_none() || self.moments_below() {
            if let Some(paused) = self.paused_notice(cx, p) {
                col = col.child(paused);
            }
            col = col.children(self.notice_line(false, cx, p));
            let list: AnyElement = if self.show_recent() {
                self.recent_rows(cx, p).into_any_element()
            } else if self.empty_state().is_some() {
                self.empty_states(cx, p).into_any_element()
            } else if !self.results_paused() {
                self.results_list(ask.is_some(), window, cx, p)
                    .into_any_element()
            } else {
                div().into_any_element()
            };
            // The pressed pill's popover lies over the results (same cell): the
            // bar grows to whichever is taller.
            let cell = div()
                .relative()
                .child(list)
                .when(self.filters.popover, |d| {
                    d.child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                            .child(self.filter_picker(cx, p)),
                    )
                    .min_h(px(super::states_view::POP_MIN_H))
                });
            col = col.child(cell);
        }
        col.into_any_element()
    }

    fn foot(&self, cx: &App, p: &Palette) -> Div {
        let keys = if self.ask_view().is_some() && self.look.is_none() && !self.filters.popover {
            self.ask_foot_keys()
        } else {
            self.search_keys()
        };
        self.field_foot(keys, cx, p)
    }
}

impl Render for QuickRecall {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        theme::request_fade_frames(window, cx);
        crate::feel::frame_rendered(window);
        if let Some(t) = self.pending_text.take() {
            self.input.update(cx, |i, cx| i.set_value(t, window, cx));
        }
        let p = theme::tokens(cx);
        let body = self.show_body().then(|| self.body(window, cx, &p));
        let look = self.look.is_some();
        let reported = self.reported_h.clone();
        let native = self.native_fit;
        let measure = canvas(
            move |bounds, window, cx| {
                let h = f32::from(bounds.size.height).ceil();
                if (h - reported.get()).abs() >= 0.5 {
                    reported.set(h);
                    window.defer(cx, move |w, cx| super::panel::fit(w, Some(h), native, cx));
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        let shell = div()
            .id("quick-recall")
            .role(Role::Dialog)
            .aria_label("Quick Recall")
            .relative()
            .flex()
            .flex_col()
            .flex_none()
            .w_full()
            .max_h(px(CAP_H))
            .overflow_hidden()
            .rounded(px(super::CORNER))
            .border_1()
            .border_color(p.border_strong)
            .bg(p.bg)
            .text_color(p.text)
            .font_family(FONT_UI)
            .child(self.field_row(window, cx, &p).h(px(FIELD_H)))
            .when_some(body, |d, body| {
                let scroll = div()
                    .id("quick-recall-body")
                    .flex()
                    .flex_col()
                    .flex_shrink(1.)
                    .min_h_0()
                    .overflow_y_scroll()
                    .border_t_1()
                    .border_color(p.hairline)
                    .map(|b| {
                        if look {
                            b.h(px(LOOK_H))
                        } else {
                            b.pt(px(8.)).px(px(8.)).pb(px(10.))
                        }
                    })
                    .child(body);
                d.child(crate::ui::scroll::area(scroll))
                    .child(self.foot(cx, &p).h(px(FOOT_H)))
            })
            .child(measure);
        div().w_full().child(shell)
    }
}
