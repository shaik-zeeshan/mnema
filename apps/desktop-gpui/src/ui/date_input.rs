//! `DateInput.svelte`: the scrubbable capsule (calendar icon, bold label +
//! mono caption, density ruler) over a calendar popover. `period(unit)` adds
//! the D · W · M switch sharing the capsule border. ←/→ (and the wheel) step,
//! Home/End jump to the latest, ↵/Space or a click opens, Esc closes.
//! `span(..)` is range mode (Chat scope): All time + presets beside the
//! calendar, where two day clicks pick a span.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{
    deferred, div, linear_color_stop, linear_gradient, px, App, ElementId, Hsla, SharedString,
    Window,
};
use time::{Date, Duration};

use super::calendar::{
    calendar, capsule_label, month_of, period_of, periods_ago, presets, relative_label, span_label,
    span_relative, step, CalSpec, Unit,
};
use super::{focus_handle, focus_visible, icon, nid, outline, popover, Force};
use crate::theme::{self, radius, size, text, Palette, FONT_MONO};

/// `(start, end, period unit or None for day mode)`.
pub type OnDate = Rc<dyn Fn(Date, Date, Option<Unit>, &mut Window, &mut App)>;
/// Range mode's pick: inclusive days, `None` = all time.
pub type OnSpan = Rc<dyn Fn(Option<(Date, Date)>, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct DateInput {
    id: ElementId,
    value: Date,
    period: Option<Unit>,
    today: Date,
    min: Date,
    max: Date,
    density: Rc<dyn Fn(Date) -> u8>,
    force: Force,
    force_open: bool,
    on_change: Option<OnDate>,
    /// Range mode: `Some(None)` = all time.
    span: Option<Option<(Date, Date)>>,
    on_span: Option<OnSpan>,
}

impl DateInput {
    /// Day mode on `value`; `today` comes from `ui::clock::now`.
    pub fn new(id: impl Into<ElementId>, value: Date, today: Date) -> Self {
        Self {
            id: id.into(),
            value,
            period: None,
            today,
            min: today - Duration::days(120),
            max: today,
            density: Rc::new(|_| 0),
            force: Force::None,
            force_open: false,
            on_change: None,
            span: None,
            on_span: None,
        }
    }
    /// Range mode on `span` (`None` = all time), with an All time preset.
    pub fn span(
        mut self,
        span: Option<(Date, Date)>,
        f: impl Fn(Option<(Date, Date)>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.span = Some(span);
        self.on_span = Some(Rc::new(f));
        self
    }
    /// Period mode: one whole day / week / month holding `value`.
    pub fn period(mut self, unit: Unit) -> Self {
        self.period = Some(unit);
        self
    }
    pub fn range(mut self, min: Date, max: Date) -> Self {
        self.min = min;
        self.max = max;
        self
    }
    /// Captured density per day, 0..=3.
    pub fn density(mut self, f: impl Fn(Date) -> u8 + 'static) -> Self {
        self.density = Rc::new(f);
        self
    }
    pub fn force(mut self, f: Force) -> Self {
        self.force = f;
        self
    }
    pub fn force_open(mut self) -> Self {
        self.force_open = true;
        self
    }
    pub fn on_change(
        mut self,
        f: impl Fn(Date, Date, Option<Unit>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }
}

const PITCH: f32 = 7.;

/// One ruler tick: `h` = density (-1 = a day still ahead), `sel` = accent.
fn tick(x: f32, w: f32, h: i8, sel: bool, p: &Palette) -> gpui_kit::Div {
    let (ht, c) = match h {
        -1 => (2., p.chart_grey_2),
        0 => (2., p.chart_grey_2),
        1 => (5., p.chart_grey_3),
        2 => (8., p.chart_grey_3),
        _ => (12., p.chart_grey_4),
    };
    div()
        .absolute()
        .bottom(px(4.))
        .left(px(x))
        .w(px(w))
        .h(px(ht))
        .rounded(px(1.))
        .bg(if sel { p.accent } else { c })
        .when(h < 0, |d| d.opacity(0.4))
}

/// Fade a ruler edge into the capsule (the CSS mask): `from` = capsule colour.
fn edge(left: bool, from: Hsla) -> gpui_kit::Div {
    let clear = Hsla { a: 0., ..from };
    let (angle, a, b) = if left {
        (90., from, clear)
    } else {
        (270., from, clear)
    };
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .w(px(22.))
        .when(left, |d| d.left_0())
        .when(!left, |d| d.right_0())
        .bg(linear_gradient(
            angle,
            linear_color_stop(a, 0.),
            linear_color_stop(b, 1.),
        ))
}

impl RenderOnce for DateInput {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = theme::tokens(cx);
        let base = self.id.to_string();
        let handle = focus_handle(nid(&format!("{base}-focus"), 0), window, cx);
        let open_state = window.use_keyed_state(nid(&format!("{base}-open"), 0), cx, |_, _| false);
        let view_state = window.use_keyed_state(nid(&format!("{base}-view"), 0), cx, |_, _| 0i32);
        let open = *open_state.read(cx) || self.force_open;
        let ring = focus_visible(&handle, self.force, window);
        let hover = self.force == Force::Hover;
        let unit = self.period.unwrap_or(Unit::Day);
        let ranged = self.span.is_some();
        let unbounded = self.span == Some(None);
        let (a, b) = match self.span {
            Some(Some(ab)) => ab,
            Some(None) => (self.min, self.max),
            None => period_of(self.value, unit),
        };
        let own = matches!(self.period, Some(Unit::Week | Unit::Month));
        let (label, rel) = match self.span {
            Some(None) => ("All time".to_string(), "no limit".to_string()),
            Some(Some(_)) => (span_label(a, b), span_relative(a, b, self.today)),
            None => (
                capsule_label(self.period, a, b, self.today),
                relative_label(self.period, a, self.today),
            ),
        };

        // Ruler: whole history (day) scrolled so the selection sits under the
        // notch, or just the period's own days centred (week / month).
        let ruler_w = f32::from(size::DATE_RULER_W);
        let density = self.density.clone();
        let heat = |d: Date| -> i8 {
            if d < self.min || d > self.max {
                -1
            } else {
                density(d) as i8
            }
        };
        let ticks: Vec<_> = if own {
            let (tw, pitch) = if unit == Unit::Month {
                (1., 2.)
            } else {
                (2., PITCH)
            };
            let n = (b - a).whole_days() as f32 + 1.;
            let x0 = (ruler_w - (n * pitch - (pitch - tw))) / 2.;
            (0..n as i64)
                .map(|i| {
                    let d = a + Duration::days(i);
                    let h = heat(d);
                    tick(x0 + i as f32 * pitch, tw, h, h >= 0, &p)
                })
                .collect()
        } else {
            let mid = ((a - self.min).whole_days() + (b - self.min).whole_days()) as f32 / 2.;
            let x0 = ruler_w / 2. - (mid * PITCH + 1.);
            let days = (self.max - self.min).whole_days();
            (0..=days)
                .filter_map(|i| {
                    let x = x0 + i as f32 * PITCH;
                    (x > -2. && x < ruler_w).then(|| {
                        let d = self.min + Duration::days(i);
                        tick(x, 2., heat(d).max(0), d >= a && d <= b, &p)
                    })
                })
                .collect()
        };
        let cap_bg = if hover {
            p.surface_hover
        } else {
            p.surface_raised
        };
        let ruler = div()
            .relative()
            .flex_none()
            .w(size::DATE_RULER_W)
            .h(size::DATE_RULER_H)
            .overflow_hidden()
            .rounded(radius::XS)
            .when(!own, |d| d.bg(p.wash))
            .children(ticks)
            .when(!own, |d| {
                d.child(edge(true, cap_bg))
                    .child(edge(false, cap_bg))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left(px(ruler_w / 2. - 0.5))
                            .w(px(1.))
                            .h(px(3.))
                            .bg(p.text_subtle),
                    )
            });

        let change = self.on_change.clone();
        let (min, max, period) = (self.min, self.max, self.period);
        let commit = move |d: Date, window: &mut Window, cx: &mut App| {
            if let Some(f) = &change {
                let (s, e) = period_of(d, unit);
                f(s, e, period, window, cx);
            }
        };
        let commit = Rc::new(commit);
        let border = if open {
            p.accent_border
        } else {
            p.border_strong
        };
        let hover_bg = p.surface_hover;
        let (toggle, keys, wheel) = (open_state.clone(), open_state.clone(), commit.clone());
        let key_commit = commit.clone();
        let value = self.value;
        let btn = div()
            .id(self.id.clone())
            .test_support()
            .role(gpui_kit::Role::Button)
            .aria_label(format!(
                "{}: {label}, {rel}. Arrow keys to step, Enter for calendar.",
                if ranged {
                    "Range"
                } else if self.period.is_some() {
                    unit.name()
                } else {
                    "Day"
                }
            ))
            .aria_expanded(open)
            .track_focus(&handle)
            .relative()
            .flex()
            .items_center()
            .gap(px(9.))
            .h(if self.period.is_some() {
                size::H_MD - px(2.)
            } else {
                size::H_MD
            })
            .pl(px(10.))
            .pr(px(6.))
            // In the period capsule only the outer (left) corners round, inside its 1 px border.
            .when(self.period.is_none(), |d| d.rounded(radius::SM))
            .when(self.period.is_some(), |d| d.rounded_l(radius::SM - px(1.)))
            .when(self.period.is_none(), |d| {
                d.border_1().border_color(border).bg(cap_bg)
            })
            .when(self.period.is_some() && hover, |d| d.bg(p.surface_hover))
            .text_color(p.text_strong)
            .cursor_pointer()
            .hover(move |s| s.bg(hover_bg))
            .child(
                div()
                    .text_color(p.text_subtle)
                    .child(icon(IconName::Calendar.path(), px(14.))),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .min_w(px(if self.period.is_some() { 108. } else { 92. }))
                    .line_height(gpui_kit::relative(1.))
                    .child(
                        div()
                            .font_weight(theme::W_SEMIBOLD)
                            .text_size(text::MD)
                            .child(label.clone()),
                    )
                    .child(
                        div()
                            .font_family(FONT_MONO)
                            .font_weight(theme::W_MEDIUM)
                            .text_size(text::MICRO)
                            .text_color(p.text_subtle)
                            .child(rel.to_uppercase()),
                    ),
            )
            .child(ruler)
            .when(ring, |d| d.child(outline(p.accent, px(6.))))
            .on_click(move |_, _, cx| {
                toggle.update(cx, |o, cx| {
                    *o = !*o;
                    cx.notify();
                })
            })
            .on_key_down(move |e, window, cx| {
                let k = match e.keystroke.key.as_str() {
                    // shortcut: range mode doesn't step a span by keys/wheel/drag yet (Tauri's
                    // stepSpan); upgrade if Chat scope users ask for it.
                    "left" | "right" | "home" | "end" if ranged => return,
                    "left" => -1,
                    "right" => 1,
                    "home" | "end" => {
                        key_commit(max, window, cx);
                        return cx.stop_propagation();
                    }
                    "enter" | "space" => {
                        keys.update(cx, |o, cx| {
                            *o = !*o;
                            cx.notify();
                        });
                        return cx.stop_propagation();
                    }
                    "escape" => {
                        keys.update(cx, |o, cx| {
                            *o = false;
                            cx.notify();
                        });
                        return;
                    }
                    _ => return,
                };
                if let Some((s, _)) = step(value, unit, k, min, max) {
                    key_commit(s, window, cx);
                }
                cx.stop_propagation();
            })
            .on_scroll(move |e, window, cx| {
                if ranged {
                    return;
                }
                let dy = e.delta.pixel_delta(px(16.)).y;
                let k = if dy > px(0.) {
                    -1
                } else if dy < px(0.) {
                    1
                } else {
                    return;
                };
                if let Some((s, _)) = step(value, unit, k, min, max) {
                    wheel(s, window, cx);
                }
            });

        let units = self.period.map(|_| {
            div()
                .flex()
                .items_stretch()
                .gap(px(1.))
                .p(px(3.))
                .border_l_1()
                .border_color(p.hairline)
                .children(
                    [Unit::Day, Unit::Week, Unit::Month]
                        .into_iter()
                        .enumerate()
                        .map(|(i, u)| {
                            let on = u == unit;
                            let (change, today) = (self.on_change.clone(), self.today);
                            let hover = (p.wash, p.text_strong);
                            div()
                                .id(nid(&format!("{base}-unit"), i as u64))
                                .test_support()
                                .role(gpui_kit::Role::Button)
                                .aria_label(u.name())
                                .aria_selected(on)
                                .flex()
                                .items_center()
                                .justify_center()
                                .min_w(px(22.))
                                .px(px(6.))
                                .rounded(radius::XS)
                                .font_weight(theme::W_MEDIUM)
                                .text_size(text::XS)
                                .text_color(if on { p.text_strong } else { p.text_subtle })
                                .when(on, |d| d.bg(p.wash_strong))
                                .when(!on, |d| {
                                    d.cursor_pointer()
                                        .hover(move |s| s.bg(hover.0).text_color(hover.1))
                                })
                                .child(u.name()[..1].to_uppercase())
                                .on_click(move |_, window, cx| {
                                    // Switch lands on "now" if the current period showed, else on the old start.
                                    let focus = if periods_ago(a, unit, today) == 0 {
                                        today
                                    } else {
                                        a.max(min)
                                    };
                                    let (s, e) = period_of(focus, u);
                                    if let Some(f) = &change {
                                        f(s, e, Some(u), window, cx);
                                    }
                                })
                        }),
                )
        });

        let anchor_state =
            window.use_keyed_state(nid(&format!("{base}-anchor"), 0), cx, |_, _| None::<Date>);
        let pop = open.then(|| {
            let shift = *view_state.read(cx);
            let (view, _) = month_of(b, shift);
            let (pick, nav, close, outside) = (
                commit.clone(),
                view_state.clone(),
                open_state.clone(),
                open_state.clone(),
            );
            let anchor = *anchor_state.read(cx);
            let outside_anchor = anchor_state.clone();
            // All time marks no days; a first click marks just its anchor.
            let sel = match (anchor, unbounded) {
                (Some(x), _) => (x, x),
                (None, true) => (Date::MIN, Date::MIN),
                _ => (a, b),
            };
            let on_span = self.on_span.clone();
            let presets_col =
                ranged.then(|| {
                    let pick_span = |s: Option<(Date, Date)>| {
                        let (f, close) = (on_span.clone(), open_state.clone());
                        move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| {
                            close.update(cx, |o, cx| {
                                *o = false;
                                cx.notify();
                            });
                            if let Some(f) = &f {
                                f(s, window, cx);
                            }
                        }
                    };
                    let row = |i: usize, l: String, small: String, on: bool| {
                        let hover = p.wash_strong;
                        div()
                            .id(nid(&format!("{base}-preset"), i as u64))
                            .test_support()
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
                            .text_color(if on { p.text_strong } else { p.text })
                            .when(on, |d| d.bg(p.wash_strong))
                            .hover(move |s| s.bg(hover))
                            .child(l)
                            .child(
                                div()
                                    .font_family(FONT_MONO)
                                    .font_weight(theme::W_REGULAR)
                                    .text_size(text::XS)
                                    .text_color(p.text_subtle)
                                    .child(small),
                            )
                    };
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(1.))
                        .w(px(156.))
                        .p(px(6.))
                        .border_r_1()
                        .border_color(p.hairline)
                        .child(
                            row(0, "All time".into(), "no limit".into(), unbounded)
                                .on_click(pick_span(None)),
                        )
                        .children(presets(self.today).into_iter().enumerate().map(
                            |(i, (l, s, e))| {
                                row(
                                    i + 1,
                                    l.into(),
                                    span_label(s, e),
                                    !unbounded && (s, e) == (a, b),
                                )
                                .on_click(pick_span(Some((s, e))))
                            },
                        ))
                });
            let span_pick = (self.on_span.clone(), anchor_state.clone());
            let on_pick: super::calendar::OnPickDay = if ranged {
                let close = close.clone();
                Rc::new(move |d, window, cx| {
                    let (f, anchor) = (&span_pick.0, &span_pick.1);
                    match anchor.read(cx).as_ref().copied() {
                        None => anchor.update(cx, |x, cx| {
                            *x = Some(d);
                            cx.notify();
                        }),
                        Some(x) => {
                            anchor.update(cx, |x, _| *x = None);
                            close.update(cx, |o, cx| {
                                *o = false;
                                cx.notify();
                            });
                            if let Some(f) = f {
                                f(Some((x.min(d), x.max(d))), window, cx);
                            }
                        }
                    }
                })
            } else {
                Rc::new(move |d, window, cx| {
                    close.update(cx, |o, cx| {
                        *o = false;
                        cx.notify();
                    });
                    pick(d, window, cx);
                })
            };
            deferred(
                popover::surface(&p)
                    .id(nid(&format!("{base}-pop"), 0))
                    .role(gpui_kit::Role::Dialog)
                    .aria_label(if ranged {
                        "Choose a range"
                    } else {
                        "Choose a day"
                    })
                    .absolute()
                    .top_full()
                    .mt(size::POP_GAP)
                    .left_0()
                    .p(px(4.))
                    .flex()
                    .children(presets_col)
                    .on_mouse_down_out(move |_, _, cx| {
                        outside_anchor.update(cx, |x, _| *x = None);
                        outside.update(cx, |o, cx| {
                            *o = false;
                            cx.notify();
                        })
                    })
                    .child(calendar(
                        CalSpec {
                            id: format!("{base}-cal").into(),
                            view,
                            today: self.today,
                            min: self.min,
                            max: self.max,
                            sel,
                            density: self.density.clone(),
                            reset_label: SharedString::from(
                                if self.period.is_some() && unit != Unit::Day {
                                    format!("This {}", unit.name())
                                } else {
                                    "Today".into()
                                },
                            ),
                            at_now: ranged || periods_ago(a, unit, self.max) == 0,
                            on_pick,
                            on_nav: Rc::new(move |k, _, cx| {
                                nav.update(cx, |v, cx| {
                                    *v += k;
                                    cx.notify();
                                })
                            }),
                        },
                        &p,
                    )),
            )
            .with_priority(1)
        });

        div()
            .relative()
            .flex()
            .flex_none()
            .when(self.period.is_some(), |d| {
                d.items_stretch()
                    .h(size::H_MD)
                    .rounded(radius::SM)
                    .border_1()
                    .border_color(border)
                    .bg(p.surface_raised)
            })
            .child(btn)
            .children(units)
            .children(pop)
    }
}
