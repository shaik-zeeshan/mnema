//! `DateInput.svelte`: the scrubbable capsule (calendar icon, bold label +
//! mono caption, density ruler) over a calendar popover. `period(unit)` adds
//! the D · W · M switch sharing the capsule border. ←/→ ↑/↓, the wheel and a
//! drag step; Home/End jump to the latest, ↵/Space or a click opens, Esc
//! closes. `span(..)` is range mode (Chat scope): All time + presets beside
//! the calendar, where a drag across days (or two presses) picks a span.
//! The popover itself is `date_input_pop.rs`.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{
    div, linear_color_stop, linear_gradient, px, Animation, AnimationExt as _, App, ElementId,
    EmptyView, Entity, Hsla, MouseButton, Pixels, ScrollDelta, Window,
};
use time::{Date, Duration};

use super::calendar::{
    capsule_label, latest_span, month_of, period_of, periods_ago, relative_label, span_label,
    span_relative, step, step_span, Unit,
};
use super::date_input_pop::{content, release, Act, Commit, Pop, RangePick};
use super::{
    focus_handle, focus_visible, icon, last_change, nid, outline_bordered, popover, with_tip, Force,
};
use crate::theme::{self, motion, radius, size, text, Palette, FONT_MONO};
use crate::ui::Tracking as _;

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
    /// `align="end"`: the popover's right edge under the capsule's.
    align_end: bool,
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
            align_end: false,
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
    /// Right-align the popover under the capsule (`align="end"`).
    pub fn align_end(mut self) -> Self {
        self.align_end = true;
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

/// The capsule button's hover group: the ruler's edge fades follow its fill.
const BTN_GROUP: &str = "mx-date-btn";

/// Fade a ruler edge into the capsule (the CSS mask): `from` = capsule colour,
/// `hover` = its `:hover` fill, so the fade never shows as a lighter block.
fn edge(left: bool, from: Hsla, hover: Hsla) -> gpui_kit::Div {
    let angle = if left { 90. } else { 270. };
    let fade = move |c: Hsla| {
        linear_gradient(
            angle,
            linear_color_stop(c, 0.),
            linear_color_stop(Hsla { a: 0., ..c }, 1.),
        )
    };
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .w(px(22.))
        .when(left, |d| d.left_0())
        .when(!left, |d| d.right_0())
        .bg(fade(from))
        .group_hover(BTN_GROUP, move |s| s.bg(fade(hover)))
}

/// The drag payload while the capsule is scrubbed (its element id).
struct Scrub(String);

/// A scrub in progress: where it started, the span it started on, steps applied.
#[derive(Clone, Copy)]
struct Drag {
    x0: Pixels,
    start: (Date, Date),
    applied: i32,
    dragged: bool,
}

/// Pixels of drag per step (`stepPx`): a day per ruler tick.
fn step_px(unit: Option<Unit>) -> f32 {
    match unit {
        Some(Unit::Day) => PITCH,
        Some(Unit::Month) => 36.,
        _ => 24.,
    }
}

fn set<T: 'static>(e: &Entity<T>, v: T, cx: &mut App) {
    e.update(cx, |x, cx| {
        *x = v;
        cx.notify();
    })
}

fn cap(s: &str) -> String {
    format!("{}{}", s[..1].to_uppercase(), &s[1..])
}

impl RenderOnce for DateInput {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = theme::tokens(cx);
        let base = self.id.to_string();
        let key = |s: &str| nid(&format!("{base}-{s}"), 0);
        let handle = focus_handle(key("focus"), window, cx);
        let open_state = window.use_keyed_state(key("open"), cx, |_, _| false);
        let view_state = window.use_keyed_state(key("view"), cx, |_, _| None::<(Date, Date)>);
        let pick_state = window.use_keyed_state(key("anchor"), cx, |_, _| RangePick::default());
        let drag_state = window.use_keyed_state(key("drag"), cx, |_, _| None::<Drag>);
        let wheel_state = window.use_keyed_state(key("wheel"), cx, |_, _| 0f32);
        let open = *open_state.read(cx) || self.force_open;
        let ring = focus_visible(&handle, self.force, window);
        let hover = self.force == Force::Hover;
        let dragging = cx.has_active_drag() && drag_state.read(cx).is_some_and(|d| d.dragged);
        let unit = self.period.unwrap_or(Unit::Day);
        let ranged = self.span.is_some();
        let unbounded = self.span == Some(None);
        // The span the control steps from (all time steps from today, like Svelte's seed).
        let (a, b) = match self.span {
            Some(Some(ab)) => ab,
            Some(None) => (self.today, self.today),
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

        // Ruler: whole history (day / range) scrolled so the selection sits under
        // the notch, or just the period's own days centred (week / month).
        let ruler_w = f32::from(size::DATE_RULER_W);
        let density = self.density.clone();
        let heat = |d: Date| -> i8 {
            if d < self.min || d > self.max {
                -1
            } else {
                density(d) as i8
            }
        };
        // The tape: ticks at their own offsets, the whole strip slid to `tape_x`.
        let (tape_x, ticks): (f32, Vec<(f32, f32, i8, bool)>) = if own {
            let (tw, pitch) = if unit == Unit::Month {
                (1., 2.)
            } else {
                (2., PITCH)
            };
            let n = (b - a).whole_days() as f32 + 1.;
            let x0 = (ruler_w - (n * pitch - (pitch - tw))) / 2.;
            let ticks = (0..n as i64)
                .map(|i| {
                    let h = heat(a + Duration::days(i));
                    (i as f32 * pitch, tw, h, h >= 0)
                })
                .collect();
            (x0, ticks)
        } else {
            let (sa, sb) = if unbounded {
                (self.min, self.max)
            } else {
                (a, b)
            };
            let mid = ((sa - self.min).whole_days() + (sb - self.min).whole_days()) as f32 / 2.;
            let days = (self.max - self.min).whole_days();
            let ticks = (0..=days)
                .map(|i| {
                    let d = self.min + Duration::days(i);
                    (i as f32 * PITCH, 2., heat(d).max(0), d >= sa && d <= sb)
                })
                .collect();
            (ruler_w / 2. - (mid * PITCH + 1.), ticks)
        };
        // `.mx-date__tape { transition: transform }`: 220 ms expo, 80 ms while dragging.
        let (from_x, gen) = last_change(key("tape-x"), tape_x, window, cx);
        let (lo, hi) = (from_x.min(tape_x), from_x.max(tape_x));
        let tape = div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(tape_x))
            // A probe at the tape's origin, so tests can watch it slide.
            .child(div().id(key("tape")).test_support())
            // Only ticks inside the ruler at either end of the slide.
            .children(
                ticks
                    .into_iter()
                    .filter(|(x, ..)| x + hi > -2. && x + lo < ruler_w)
                    .map(|(x, w, h, sel)| tick(x, w, h, sel, &p)),
            );
        let tape = if gen == 0 {
            tape.into_any_element()
        } else {
            let dur = if dragging { motion::SCRUB } else { motion::MED };
            tape.with_animation(
                nid(&format!("{base}-tape"), gen),
                Animation::new(dur).with_easing(motion::expo),
                move |el, t| el.left(px(from_x + (tape_x - from_x) * t)),
            )
            .into_any_element()
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
            .child(tape)
            .when(!own, |d| {
                d.child(edge(true, cap_bg, p.surface_hover))
                    .child(edge(false, cap_bg, p.surface_hover))
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

        // `apply`: every step, commit and unit switch reports the new span.
        let (on_change, on_span, period) =
            (self.on_change.clone(), self.on_span.clone(), self.period);
        let emit: Commit = Rc::new(move |(s, e), window, cx| {
            if ranged {
                if let Some(f) = &on_span {
                    f(Some((s, e)), window, cx);
                }
            } else if let Some(f) = &on_change {
                f(s, e, period, window, cx);
            }
        });
        let (min, max) = (self.min, self.max);
        let stepped = move |(a, b): (Date, Date), k: i32| {
            if ranged {
                step_span(a, b, k, min, max)
            } else {
                step(a, unit, k, min, max)
            }
        };
        let latest = if ranged {
            latest_span(a, b, max)
        } else {
            period_of(max, unit)
        };
        // A popover pick closes it and hands focus back to the capsule.
        let commit: Commit = {
            let (emit, close, focus) = (emit.clone(), open_state.clone(), handle.clone());
            Rc::new(move |span, window, cx| {
                set(&close, false, cx);
                window.focus(&focus, cx);
                emit(span, window, cx);
            })
        };
        let border = if open {
            p.accent_border
        } else if dragging {
            p.border_hover
        } else {
            p.border_strong
        };
        let hover_bg = p.surface_hover;
        let (toggle, keys, wheel_emit, key_emit) = (
            open_state.clone(),
            open_state.clone(),
            emit.clone(),
            emit.clone(),
        );
        let (toggle_view, toggle_pick) = (view_state.clone(), pick_state.clone());
        let (down_drag, move_drag, move_open, drag_emit) = (
            drag_state.clone(),
            drag_state.clone(),
            open_state.clone(),
            emit.clone(),
        );
        let scrub_id = base.clone();
        let btn = div()
            .id(self.id.clone())
            .test_support()
            .role(gpui_kit::Role::Button)
            .aria_label(format!(
                "{}: {label}, {rel}. Arrow keys or drag to step, Enter for calendar.",
                if ranged {
                    "Range".to_string()
                } else if self.period.is_some() {
                    cap(unit.name())
                } else {
                    "Day".to_string()
                }
            ))
            .aria_expanded(open)
            .track_focus(&handle)
            .group(BTN_GROUP)
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
                    .min_w(px(if self.period.is_some() {
                        108.
                    } else if ranged {
                        104.
                    } else {
                        92.
                    }))
                    .line_height(gpui_kit::relative(1.))
                    .child(
                        div()
                            .font_weight(theme::W_SEMIBOLD)
                            .text_size(text::MD)
                            .tracking(-0.01)
                            .child(label.clone()),
                    )
                    .child(
                        div()
                            .font_family(FONT_MONO)
                            .font_weight(theme::W_MEDIUM)
                            .text_size(text::MICRO)
                            .tracking(0.08)
                            .text_color(p.text_subtle)
                            .child(rel.to_uppercase()),
                    ),
            )
            .child(ruler)
            .when(ring, |d| d.child(outline_bordered(p.accent, px(6.), px(1.))))
            // Enter / Space land here too (GPUI's keyboard click on the focused capsule).
            .on_click(move |_, _, cx| {
                let opening = !*toggle.read(cx);
                if opening {
                    set(&toggle_view, None, cx);
                    set(&toggle_pick, RangePick::default(), cx);
                }
                set(&toggle, opening, cx);
            })
            .on_key_down(move |e, window, cx| {
                let k = match e.keystroke.key.as_str() {
                    _ if e.keystroke.modifiers.platform => return,
                    "left" | "down" => -1,
                    "right" | "up" => 1,
                    "home" | "end" => {
                        key_emit(latest, window, cx);
                        return cx.stop_propagation();
                    }
                    "escape" => return set(&keys, false, cx),
                    _ => return,
                };
                if let Some(span) = stepped((a, b), k) {
                    key_emit(span, window, cx);
                }
                cx.stop_propagation();
            })
            // The wheel steps once per 40 px (a line = a step), like `onWheel`.
            .on_scroll(move |e, window, cx| {
                let (dx, dy) = match e.delta {
                    ScrollDelta::Pixels(d) => (-f32::from(d.x), -f32::from(d.y)),
                    ScrollDelta::Lines(l) => (-l.x * 40., -l.y * 40.),
                };
                let acc = *wheel_state.read(cx) + if dx.abs() > dy.abs() { dx } else { dy };
                if acc.abs() < 40. {
                    return wheel_state.update(cx, |w, _| *w = acc);
                }
                wheel_state.update(cx, |w, _| *w = 0.);
                if let Some(span) = stepped((a, b), acc.signum() as i32) {
                    wheel_emit(span, window, cx);
                }
            })
            // Drag scrub: right pulls earlier days under the notch; a drag is not a click.
            .on_mouse_down(MouseButton::Left, move |e, _, cx| {
                down_drag.update(cx, |d, _| {
                    *d = Some(Drag {
                        x0: e.position.x,
                        start: (a, b),
                        applied: 0,
                        dragged: false,
                    })
                })
            })
            .on_drag(Scrub(scrub_id.clone()), |_, _, _, cx| cx.new(|_| EmptyView))
            .on_drag_move::<Scrub>(move |e, window, cx| {
                if e.drag(cx).0 != scrub_id {
                    return;
                }
                let Some(mut d) = *move_drag.read(cx) else {
                    return;
                };
                let x = e.event.position.x;
                let dx = f32::from(x - d.x0);
                if !d.dragged && dx.abs() > 3. {
                    d.dragged = true;
                    set(&move_open, false, cx);
                }
                let px_step = step_px(if ranged { None } else { Some(unit) });
                let want = -(dx / px_step).trunc() as i32;
                if want != d.applied {
                    let (mut cur, mut got) = (d.start, 0);
                    while got != want {
                        let dir = (want - got).signum();
                        match stepped(cur, dir) {
                            Some(n) => (cur, got) = (n, got + dir),
                            None => break,
                        }
                    }
                    if got != want {
                        d.x0 = x + px(got as f32 * px_step); // pinned at an edge: no debt
                    }
                    if got != d.applied {
                        d.applied = got;
                        drag_emit(cur, window, cx);
                    }
                }
                set(&move_drag, Some(d), cx);
            });

        let switch_unit: Rc<dyn Fn(Unit, &mut Window, &mut App)> = {
            let (change, today) = (self.on_change.clone(), self.today);
            Rc::new(move |u, window, cx| {
                if u == unit {
                    return;
                }
                // Lands on "now" if the current period showed, else on the old start.
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
        };
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
                            let switch = switch_unit.clone();
                            let hover = (p.wash, p.text_strong);
                            let el = div()
                                .id(nid(&format!("{base}-unit"), i as u64))
                                .test_support()
                                .role(gpui_kit::Role::Button)
                                .aria_label(cap(u.name()))
                                .aria_selected(on)
                                .flex()
                                .items_center()
                                .justify_center()
                                .min_w(px(22.))
                                .px(px(6.))
                                .rounded(radius::XS)
                                .cursor_pointer()
                                .font_weight(theme::W_MEDIUM)
                                .text_size(text::XS)
                                .text_color(if on { p.text_strong } else { p.text_subtle })
                                .when(on, |d| d.bg(p.wash_strong))
                                .when(!on, |d| d.hover(move |s| s.bg(hover.0).text_color(hover.1)))
                                .child(u.name()[..1].to_uppercase())
                                .on_click(move |_, window, cx| switch(u, window, cx));
                            with_tip(el, cap(u.name()))
                        }),
                )
        });

        let trigger = popover::Anchor::new(key("trigger"), window, cx);
        let pop_size = popover::Anchor::new(key("pop-size"), window, cx);
        // Up when the popover doesn't fit below and there's more room above
        // (`DateInput.svelte` placePop: e.g. Chat's docked composer).
        let up = trigger.opens_up(pop_size.height(cx) + size::POP_GAP * 2., window, cx);
        let pop = open.then(|| {
            let view = match *view_state.read(cx) {
                Some((v, at)) if at == a => v,
                _ => month_of(a, 0).0,
            };
            let all_time: Act = {
                let (f, close, focus) = (self.on_span.clone(), open_state.clone(), handle.clone());
                Rc::new(move |window, cx| {
                    set(&close, false, cx);
                    window.focus(&focus, cx);
                    if let Some(f) = &f {
                        f(None, window, cx);
                    }
                })
            };
            let (outside, outside_trigger) = (open_state.clone(), trigger.clone());
            let (up_pick, up_commit) = (pick_state.clone(), commit.clone());
            let (out_pick, out_commit) = (pick_state.clone(), commit.clone());
            let surface = popover::surface(&p)
                .id(key("pop"))
                .role(gpui_kit::Role::Dialog)
                .aria_label(if ranged {
                    "Choose a range"
                } else if self.period.is_some() {
                    "Choose a period"
                } else {
                    "Choose a day"
                })
                .p(px(4.))
                .on_mouse_down_out(move |e, _, cx| {
                    if outside_trigger.outside(e, cx) {
                        set(&outside, false, cx); // the trigger toggles it itself
                    }
                })
                .on_mouse_up(MouseButton::Left, move |_, window, cx| {
                    release(&up_pick, &up_commit, window, cx)
                })
                .on_mouse_up_out(MouseButton::Left, move |_, window, cx| {
                    release(&out_pick, &out_commit, window, cx)
                })
                .child(content(
                    Pop {
                        base: base.clone(),
                        span: (a, b),
                        unit,
                        period: self.period.is_some(),
                        ranged,
                        unbounded,
                        today: self.today,
                        min,
                        max,
                        density: self.density.clone(),
                        view,
                        pick: pick_state.clone(),
                        view_state: view_state.clone(),
                        commit: commit.clone(),
                        all_time,
                        switch_unit: switch_unit.clone(),
                    },
                    &p,
                    cx,
                ));
            let end = self.align_end;
            // Positioned by a measured wrapper: its height picks the side.
            popover::float(
                pop_size
                    .measure(div())
                    .absolute()
                    .map(|d| if end { d.right_0() } else { d.left_0() })
                    .map(|d| {
                        if up {
                            d.bottom_full().mb(size::POP_GAP)
                        } else {
                            d.top_full().mt(size::POP_GAP)
                        }
                    })
                    .child(surface),
            )
        });

        let esc = open_state.clone();
        popover::esc_closes(trigger.track(div()), open, move |_, cx| {
            set(&esc, false, cx)
        })
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
