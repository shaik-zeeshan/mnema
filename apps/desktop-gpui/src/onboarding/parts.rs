//! The pieces every onboarding screen shares (`onboarding-shell.css` +
//! `docs/mockups/redesign/onboarding.html`): the 52 px strip, the scrolling
//! main column, the hairline action footer, and the kit type the screens use.

use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, relative, AnyElement, App, Div, Pixels, SharedString, Window};

use super::flow::Step;
use crate::theme::{self, size, space, text, Palette, FONT_MONO};
use crate::ui::{icon, mix};

/// `.ob-bar` height.
pub const STRIP_H: Pixels = px(52.);
/// `.ob-head .mx-display` (36 px) and the two motion bookends' titles.
pub const DISPLAY: Pixels = px(36.);
pub const HERO: Pixels = px(76.);
pub const FINALE_TITLE: Pixels = px(60.);
pub const PCT: Pixels = px(72.);
/// `.ob-fine` line height (× its 12 px size).
pub const FINE_LH: f32 = 1.5;
/// `.ob-col` / `.ob-change .ob-col` max widths.
pub const COL_W: Pixels = px(780.);
pub const COL_W_WIDE: Pixels = px(900.);
/// `.ob-foot` left padding (aligned with the 64 px main column gutter).
const FOOT_MIN_H: Pixels = px(60.);
const RULER_GAP: Pixels = px(16.);
const TICK_W: Pixels = px(2.);
const GROUP: &str = "ob-bar";

/// Extra side inset that centres the step's column on a wide window (founder
/// ruling 2026-11-10: centred, unlike Tauri). Zero up to the default width, so
/// the default and minimum layouts don't move; past it the column slides right
/// until it is centred. `wide` = the step's column is `COL_W_WIDE`.
pub fn inset(window_w: Pixels, wide: bool) -> Pixels {
    let col = if wide { COL_W_WIDE } else { COL_W };
    let w = f32::from(window_w);
    let centred = (w - 2. * f32::from(space::S8) - f32::from(col)) / 2.;
    px(centred.min(w - super::SIZE.0).max(0.))
}

/// Steps whose content runs at the wide column (Change, Finale) or uncapped (Voice).
pub fn wide(step: Step) -> bool {
    matches!(step, Step::Change | Step::Finale | Step::Voice)
}

/// The 52 px strip: drawn lights (the first closes), step name, tick ruler, `N / 8`.
pub fn strip(
    step: Step,
    inset: Pixels,
    on_close: impl Fn(&mut Window, &mut App) + 'static,
    p: &Palette,
) -> impl IntoElement {
    let colors = [p.status_running_dot, p.status_paused_dot, p.source_mic];
    let light = |i: usize| {
        let c = colors[i];
        div()
            .flex_none()
            .size(size::STRIP_LIGHT)
            .rounded_full()
            .bg(p.border_strong)
            .group_hover(GROUP, move |s| s.bg(c))
    };
    let pos = step.position();
    let ruler = div()
        .relative()
        .flex()
        .items_end()
        .gap(RULER_GAP)
        .h(px(16.))
        .px(px(4.))
        .children(Step::ALL.into_iter().map(|s| {
            let k = s.position();
            let h = if k == pos {
                px(15.)
            } else if s.suffix().is_some() {
                px(5.)
            } else {
                px(8.)
            };
            let c = if k == pos {
                p.accent
            } else if k < pos {
                p.chart_grey_4
            } else {
                p.chart_grey_2
            };
            div().flex_none().w(TICK_W).h(h).rounded(px(1.)).bg(c)
        }))
        .child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom(px(-4.))
                .h(px(1.))
                .bg(p.hairline),
        );
    div()
        .id("ob-strip")
        .group(GROUP)
        .flex()
        .flex_none()
        .items_center()
        .h(STRIP_H)
        .pr(space::S5 + inset)
        .border_b_1()
        .border_color(p.hairline)
        .on_mouse_down(gpui_kit::MouseButton::Left, |_, window, _| {
            window.start_window_move()
        })
        .child(
            div()
                .flex()
                .flex_none()
                .gap(px(8.))
                .w(size::TRAFFIC_LIGHTS)
                .pl(space::S4)
                .child(
                    light(0)
                        .id("ob-close")
                        .test_support()
                        .role(gpui_kit::Role::Button)
                        .aria_label("Close window")
                        .cursor_pointer()
                        .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                            cx.stop_propagation()
                        })
                        .on_click(move |_, window, cx| on_close(window, cx)),
                )
                .child(light(1))
                .child(light(2)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_baseline()
                // The inset on both sides keeps the ruler where it was.
                .pl(space::S2 + inset)
                .font_weight(theme::W_MEDIUM)
                .text_size(text::MD)
                .line_height(relative(1.))
                .text_color(p.text_strong)
                .child(step.label())
                .children(step.suffix().map(|s| {
                    div()
                        .ml(px(6.))
                        .font_weight(theme::W_REGULAR)
                        .text_color(p.text_subtle)
                        .child(format!("\u{b7} {s}"))
                })),
        )
        .child(ruler)
        .child(
            div()
                .id("ob-count")
                .role(gpui_kit::Role::Status)
                .aria_label(SharedString::from(format!(
                    "Step {pos} of {}: {}",
                    Step::COUNT,
                    step.label()
                )))
                .flex_1()
                .flex()
                .justify_end()
                .font_family(FONT_MONO)
                .font_weight(theme::W_MEDIUM)
                .text_size(text::SM)
                .line_height(relative(1.))
                .text_color(p.text_subtle)
                .child(div().text_color(p.text_strong).child(pos.to_string()))
                .child(format!("\u{a0}/ {}", Step::COUNT)),
        )
}

/// `.ob-main` + `.ob-col`: the scrolling, vertically centred column.
fn main_outer(id: &'static str, top: bool, inset: Pixels) -> gpui_kit::Stateful<Div> {
    div()
        .id(id)
        .debug_selector(|| "ob-main".into())
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .when(!top, |d| d.justify_center())
        .px(space::S8 + inset)
        .pt(if top { space::S5 } else { space::S7 })
        .pb(space::S6)
}

/// `.ob-main > .ob-split` (Voice): the split sits straight in the main area,
/// no `.ob-col` cap, so its columns run to the gutters.
pub fn main_split(id: &'static str, inset: Pixels, split: AnyElement) -> AnyElement {
    main_outer(id, false, inset).child(split).into_any_element()
}

/// `.ob-stage`'s `mx-texture`: 1 px dots on an 18 px grid (centred on the grid
/// lines) over the 140 px top sheen, painted as one canvas layer behind the stage.
pub fn texture(p: &Palette) -> impl IntoElement {
    let (dot, sheen) = (p.dot, p.sheen);
    gpui_kit::canvas(
        |_, _, _| {},
        move |b, _, window, _| {
            let top = gpui_kit::Bounds::new(b.origin, gpui_kit::size(b.size.width, px(140.)));
            window.paint_quad(gpui_kit::fill(
                top,
                gpui_kit::linear_gradient(
                    180.,
                    gpui_kit::linear_color_stop(sheen, 0.),
                    // Fade the same colour out: a stop at transparent black greys the middle.
                    gpui_kit::linear_color_stop(sheen.opacity(0.), 1.),
                ),
            ));
            let step = 18.;
            let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
            let mut y = 0.;
            while y <= h {
                let mut x = 0.;
                while x <= w {
                    let o = b.origin + gpui_kit::point(px(x - 1.), px(y - 1.));
                    window.paint_quad(
                        gpui_kit::fill(
                            gpui_kit::Bounds::new(o, gpui_kit::size(px(2.), px(2.))),
                            dot,
                        )
                        .corner_radii(px(1.)),
                    );
                    x += step;
                }
                y += step;
            }
        },
    )
    .absolute()
    .size_full()
}

pub fn main_col(
    id: &'static str,
    wide: bool,
    top: bool,
    inset: Pixels,
    children: Vec<AnyElement>,
) -> AnyElement {
    main_outer(id, top, inset)
        .child(
            div()
                .debug_selector(|| "ob-col".into())
                .flex()
                .flex_col()
                // The dense Change settings column runs tighter (`.change > .ob-col`).
                .gap(if wide && top { space::S5 } else { space::S6 })
                .min_w_0()
                .max_w(if wide { COL_W_WIDE } else { COL_W })
                .children(children),
        )
        .into_any_element()
}

/// `.ob-foot`: a state line on the left, the actions on the right.
pub fn footer(note: Option<AnyElement>, acts: Vec<AnyElement>, inset: Pixels, p: &Palette) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(space::S2)
        .min_h(FOOT_MIN_H)
        .py(space::S3)
        .pr(space::S5 + inset)
        .pl(space::S8 + inset)
        .border_t_1()
        .border_color(p.hairline)
        .bg(mix(p.bg, p.surface, 0.86))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(text::BASE)
                .line_height(relative(1.4))
                .text_color(p.text_subtle)
                .children(note),
        )
        .children(acts)
}

/// `.ob-head`: kicker + display, 12 px apart.
pub fn head(children: Vec<AnyElement>) -> Div {
    div().flex().flex_col().gap(space::S3).children(children)
}

/// `.mx-display` at the onboarding's 36 px.
pub fn display(s: impl Into<SharedString>, p: &Palette) -> Div {
    div()
        .font_weight(theme::W_SEMIBOLD)
        .text_size(DISPLAY)
        .line_height(relative(1.04))
        .text_color(p.text_strong)
        .child(s.into())
}

/// `.mx-prose`: 14/1.6 muted.
pub fn prose(s: impl Into<SharedString>, p: &Palette) -> Div {
    div()
        .text_size(text::PROSE)
        .line_height(relative(1.6))
        .text_color(p.text_muted)
        .child(s.into())
}

/// `.ob-fine`: 12/1.5 subtle.
pub fn fine(s: impl Into<SharedString>, p: &Palette) -> Div {
    div()
        .text_size(text::BASE)
        .line_height(relative(FINE_LH))
        .text_color(p.text_subtle)
        .child(s.into())
}

/// `.mx-label`.
pub fn label(s: impl Into<SharedString>, p: &Palette) -> Div {
    div()
        .font_weight(theme::W_MEDIUM)
        .text_size(text::BASE)
        .line_height(relative(1.35))
        .text_color(p.text_subtle)
        .child(s.into())
}

/// `.ob-acts`: a wrapping row of actions.
pub fn acts(children: Vec<AnyElement>) -> Div {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(space::S2)
        .children(children)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    On,
    Half,
    Off,
}

/// `.ob-mark`: check (granted / on), dashed ring (unconfirmed), ring (not yet).
pub fn mark(m: Mark, p: &Palette) -> Div {
    let (path, c) = match m {
        Mark::On => ("icons/check.svg", p.accent),
        Mark::Half => ("icons/circle-dashed.svg", p.text_faint),
        Mark::Off => ("icons/circle.svg", p.text_faint),
    };
    div().flex_none().text_color(c).child(icon(path, px(14.)))
}

/// The primary "next" button's trailing arrow (`.ob-btn-arrow`).
pub const ARROW: &str = "icons/arrow-right.svg";

/// A hairline-ruled list (`.ob-ledger`, `.ob-rows`, `.ob-work`).
pub fn ruled(max_w: Option<Pixels>, rows: Vec<AnyElement>, p: &Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .border_t_1()
        .border_color(p.hairline)
        .when_some(max_w, |d, w| d.max_w(w))
        .children(
            rows.into_iter()
                .map(|r| div().border_b_1().border_color(p.hairline).child(r)),
        )
}

/// Mono figure (`b.fig`, `.num`).
pub fn mono(s: impl Into<SharedString>, size: Pixels, color: gpui_kit::Hsla) -> Div {
    div()
        .font_family(FONT_MONO)
        .text_size(size)
        .line_height(relative(1.))
        .text_color(color)
        .child(s.into())
}

/// A 3 px progress track (`.ob-track`), `frac` 0..=1.
pub fn track(frac: f32, stalled: bool, max_w: Option<Pixels>, p: &Palette) -> Div {
    div()
        .w_full()
        .when_some(max_w, |d, w| d.max_w(w))
        .h(px(3.))
        .rounded(px(2.))
        .bg(p.surface_hover)
        .overflow_hidden()
        .child(
            div()
                .h_full()
                .w(relative(frac.clamp(0., 1.)))
                .bg(if stalled { p.chart_grey_4 } else { p.accent }),
        )
}

/// The level meter (`.ob-meter`): one bar per sample, `h(level)` tall.
pub fn meter(levels: &[f32], bar_w: Pixels, max_h: f32, color: gpui_kit::Hsla, sqrt: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(if bar_w > px(3.) { 3. } else { 2. }))
        .h(px(max_h + 8.))
        .children(levels.iter().map(|l| {
            let v = if sqrt { l.max(0.).sqrt() } else { *l };
            div()
                .flex_none()
                .w(bar_w)
                .h(px(4. + (v * max_h).round()))
                .rounded(px(2.))
                .bg(color)
        }))
}
