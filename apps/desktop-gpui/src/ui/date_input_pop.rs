//! DateInput's popover (`.mx-date__pop`): range mode's presets column, period
//! mode's Day / Week / Month switch, and the calendar. A day / week / month /
//! preset commits and closes; the unit switch keeps it open. Range mode picks
//! by press: a drag across days, or one press then another.

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, App, Div, Entity, Window};
use time::Date;

use super::calendar::{
    calendar, presets, span_label, Body, CalSpec, Foot, OnPickDay, RangeHooks, Unit,
};
use super::nid;
use crate::theme::{self, radius, text, Palette, FONT_MONO};

pub(super) type Commit = Rc<dyn Fn((Date, Date), &mut Window, &mut App)>;
pub(super) type Act = Rc<dyn Fn(&mut Window, &mut App)>;

/// Range mode's in-progress pick (`anchor`, `hover`, `selecting`).
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct RangePick {
    pub anchor: Option<Date>,
    pub hover: Option<Date>,
    /// The button is still down since the anchor's press.
    pub selecting: bool,
}

impl RangePick {
    pub fn pending(&self) -> Option<(Date, Date)> {
        self.anchor.map(|x| {
            let y = self.hover.unwrap_or(x);
            (x.min(y), x.max(y))
        })
    }
}

pub(super) struct Pop {
    pub base: String,
    pub span: (Date, Date),
    pub unit: Unit,
    pub period: bool,
    pub ranged: bool,
    pub unbounded: bool,
    pub today: Date,
    pub min: Date,
    pub max: Date,
    pub density: Rc<dyn Fn(Date) -> u8>,
    pub view: Date,
    pub pick: Entity<RangePick>,
    /// `(first of the month shown, the selection's start it was set for)`.
    pub view_state: Entity<Option<(Date, Date)>>,
    pub commit: Commit,
    pub all_time: Act,
    pub switch_unit: Rc<dyn Fn(Unit, &mut Window, &mut App)>,
}

/// One preset row (`.mx-date__presets button`).
fn preset(
    base: &str,
    i: usize,
    l: String,
    small: String,
    on: bool,
    p: &Palette,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui_kit::AnyElement {
    let hover = (p.wash_strong, p.text_strong);
    div()
        .id(nid(&format!("{base}-preset"), i as u64))
        .role(gpui_kit::Role::Button)
        .aria_selected(on)
        .flex()
        .flex_col()
        .gap(px(3.))
        .px(px(9.))
        .py(px(7.))
        .rounded(radius::SM)
        .cursor_pointer()
        .text_size(text::BASE)
        .font_weight(theme::W_MEDIUM)
        .line_height(gpui_kit::relative(1.2))
        .text_color(if on { p.text_strong } else { p.text })
        .when(on, |d| d.bg(p.wash_strong))
        .hover(move |s| s.bg(hover.0).text_color(hover.1))
        .child(l)
        .child(
            div()
                .font_family(FONT_MONO)
                .font_weight(theme::W_REGULAR)
                .text_size(text::XS)
                .line_height(gpui_kit::relative(1.))
                .text_color(p.text_subtle)
                .child(small),
        )
        .on_click(on_click)
        .test_support()
        .into_any_element()
}

/// The popover's content: `[presets |] [units /] calendar`.
pub(super) fn content(pop: Pop, p: &Palette, cx: &App) -> Div {
    let (a, b) = pop.span;
    let base = pop.base.clone();
    let presets_col = pop.ranged.then(|| {
        let all = pop.all_time.clone();
        div()
            .flex()
            .flex_col()
            .gap(px(1.))
            .w(px(156.))
            .p(px(6.))
            .border_r_1()
            .border_color(p.hairline)
            .child(preset(
                &base,
                0,
                "All time".into(),
                "no limit".into(),
                pop.unbounded,
                p,
                move |_, window, cx| all(window, cx),
            ))
            .children(
                presets(pop.today)
                    .into_iter()
                    .enumerate()
                    .map(|(i, (l, s, e))| {
                        let commit = pop.commit.clone();
                        preset(
                            &base,
                            i + 1,
                            l.into(),
                            span_label(s, e),
                            !pop.unbounded && (s, e) == (a, b),
                            p,
                            move |_, window, cx| commit((s, e), window, cx),
                        )
                    }),
            )
    });

    // `.mx-date__pop > .mx-date__units`: the same switch, full width, 26 px tall.
    let units = pop.period.then(|| {
        div()
            .flex()
            .gap(px(1.))
            .mt(px(6.))
            .mx(px(10.))
            .mb(px(2.))
            .children(
                [Unit::Day, Unit::Week, Unit::Month]
                    .into_iter()
                    .enumerate()
                    .map(|(i, u)| {
                        let on = u == pop.unit;
                        let switch = pop.switch_unit.clone();
                        let hover = (p.wash, p.text_strong);
                        let name = u.name();
                        div()
                            .id(nid(&format!("{base}-pop-unit"), i as u64))
                            .test_support()
                            .role(gpui_kit::Role::Button)
                            .aria_selected(on)
                            .flex()
                            .flex_1()
                            .items_center()
                            .justify_center()
                            .h(px(26.))
                            .rounded(radius::XS)
                            .cursor_pointer()
                            .font_weight(theme::W_MEDIUM)
                            .text_size(text::BASE)
                            .line_height(gpui_kit::relative(1.))
                            .text_color(if on { p.text_strong } else { p.text_subtle })
                            .when(on, |d| d.bg(p.wash_strong))
                            .when(!on, |d| d.hover(move |s| s.bg(hover.0).text_color(hover.1)))
                            .child(format!("{}{}", name[..1].to_uppercase(), &name[1..]))
                            .on_click(move |_, window, cx| switch(u, window, cx))
                    }),
            )
    });

    let state = *pop.pick.read(cx);
    // All time marks no days; a first press marks just its anchor.
    let marks = state.pending().or((!pop.unbounded).then_some((a, b)));
    let range = pop.ranged.then(|| {
        let (press, over) = (pop.pick.clone(), pop.pick.clone());
        let commit = pop.commit.clone();
        RangeHooks {
            marks,
            on_press: Rc::new(move |d: Date, window: &mut Window, cx: &mut App| {
                let st = *press.read(cx);
                match st.anchor {
                    Some(x) if !st.selecting => {
                        press.update(cx, |s, _| *s = RangePick::default());
                        commit((x.min(d), x.max(d)), window, cx);
                    }
                    _ => press.update(cx, |s, cx| {
                        *s = RangePick {
                            anchor: Some(d),
                            hover: Some(d),
                            selecting: true,
                        };
                        cx.notify();
                    }),
                }
            }) as OnPickDay,
            on_hover: Rc::new(move |d: Date, _: &mut Window, cx: &mut App| {
                over.update(cx, |s, cx| {
                    if s.anchor.is_some() && s.hover != Some(d) {
                        s.hover = Some(d);
                        cx.notify();
                    }
                })
            }),
        }
    });
    let body = match (pop.period, pop.unit) {
        (true, Unit::Week) => Body::Weeks,
        (true, Unit::Month) => Body::Months,
        _ => Body::Days,
    };
    let view_state = pop.view_state.clone();
    let pick_commit = pop.commit.clone();
    let unit = pop.unit;
    let cal = calendar(CalSpec {
        id: format!("{base}-cal").into(),
        view: pop.view,
        today: pop.today,
        min: pop.min,
        max: pop.max,
        body,
        sel: (!pop.ranged).then_some(a),
        range,
        here: None,
        grid: None,
        density: pop.density.clone(),
        foot: if pop.ranged {
            Foot::Hint(if state.anchor.is_some() {
                "pick an end day"
            } else {
                "drag across days"
            })
        } else {
            Foot::Reset {
                label: if pop.period && unit != Unit::Day {
                    format!("This {}", unit.name()).into()
                } else {
                    "Today".into()
                },
                disabled: pop.period && super::calendar::periods_ago(a, unit, pop.today) == 0,
            }
        },
        on_pick: Rc::new(move |d, window, cx| {
            pick_commit(super::calendar::period_of(d, unit), window, cx)
        }),
        on_nav: Rc::new(move |k, _, cx| {
            let first = super::calendar::month_of(pop.view, k).0;
            view_state.update(cx, |v, cx| {
                *v = Some((first, a));
                cx.notify();
            })
        }),
    });
    div()
        .flex()
        .children(presets_col)
        .child(div().flex().flex_col().children(units).child(cal))
}

/// Range mode's release (`onWindowPointerUp`): a drag that left its anchor commits.
pub(super) fn release(
    pick: &Entity<RangePick>,
    commit: &Commit,
    window: &mut Window,
    cx: &mut App,
) {
    let st = *pick.read(cx);
    if !st.selecting {
        return;
    }
    pick.update(cx, |s, _| s.selecting = false);
    if let (Some(span), true) = (st.pending(), st.hover != st.anchor) {
        pick.update(cx, |s, _| *s = RangePick::default());
        commit(span, window, cx);
    }
}
