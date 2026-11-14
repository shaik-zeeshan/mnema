//! `.timeline__rail-wrap`: the 36 px rail (masked edges, 8 px slots against the
//! fixed centre caret, 1×8 / 16 major ticks), the audio lane, the paging note,
//! and the readout that flies 9 px over 190 ms in the scrub direction.
//! Bands and resting ticks paint in one canvas under the app icons, the
//! active tick, edge mask and carets in one over them; only the visible window
//! (half a viewport + `VIEWPORT_BUFFER` each side) is drawn.

use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{
    canvas, div, fill, linear_color_stop, linear_gradient, point, px, size, Animation,
    AnimationExt as _, AnyElement, Bounds, Context, ElementId, MouseButton, PathBuilder, Role,
    SharedString, Window,
};

use super::bands::{self, slot_x};
use super::model::{category_color, category_for_span, clock_label, day_label, local, SLOT_W};
use super::paging::{render_window, PREFETCH_AHEAD};
use super::TimelineView;
use crate::theme::{self, motion, radius, size as sz, text, Palette, FONT_MONO};
use crate::ui::app_icon::{app_icon, Src};
use crate::ui::{alpha, icon, mix, spread};

/// `.timeline-rail { height: 36px }`.
pub const RAIL_H: f32 = 36.;
/// `READOUT_FLY_OFFSET_PX` / `READOUT_FLY_DURATION_MS`.
pub const READOUT_FLY_PX: f32 = 9.;
pub const READOUT_FLY: Duration = Duration::from_millis(190);

/// One tick: `tw`×`th`, centred on (`x`, `mid`).
fn tick(x: f32, mid: f32, tw: f32, th: f32, color: gpui_kit::Hsla, window: &mut Window) {
    window.paint_quad(
        fill(
            Bounds::new(
                point(px(x - tw / 2.), px(mid - th / 2.)),
                size(px(tw), px(th)),
            ),
            color,
        )
        .corner_radii(px(0.5)),
    );
}

fn caret(down: bool, x: f32, y: f32, color: gpui_kit::Hsla, window: &mut Window) {
    let mut b = PathBuilder::fill();
    let (tip, base) = if down { (y + 5., y) } else { (y, y + 5.) };
    b.add_polygon(
        &[
            point(px(x - 4.), px(base)),
            point(px(x + 4.), px(base)),
            point(px(x), px(tip)),
        ],
        true,
    );
    if let Ok(path) = b.build() {
        window.paint_path(path, color);
    }
}

/// The CSS mask fades 5% at each end to the page behind (`bg`).
fn edge_fade(b: Bounds<gpui_kit::Pixels>, bg: gpui_kit::Hsla, window: &mut Window) {
    let edge = f32::from(b.size.width) * 0.05;
    for (x, from, to) in [(b.left(), 0., 1.), (b.right() - px(edge), 1., 0.)] {
        window.paint_quad(fill(
            Bounds::new(point(x, b.top()), size(px(edge), b.size.height)),
            linear_gradient(
                90.,
                linear_color_stop(alpha(bg, 1. - from), 0.),
                linear_color_stop(alpha(bg, 1. - to), 1.),
            ),
        ));
    }
}

/// The readout bubble: app tile + name (fly-in keyed on the app), time, date.
fn readout(
    v: &TimelineView,
    idx: usize,
    x: f32,
    p: &Palette,
    cx: &mut Context<TimelineView>,
) -> AnyElement {
    let f = &v.paging.frames[idx];
    let o = crate::ui::clock::now(cx).offset();
    let t = local(f.captured_at_ms, o);
    let c = category_color(
        category_for_span(f.captured_at_ms, f.captured_at_ms, &v.episodes),
        p,
    );
    let meta = div()
        .line_height(gpui_kit::relative(1.))
        .flex()
        .items_baseline()
        .gap(px(6.))
        .child(
            div()
                .font_family(FONT_MONO)
                .text_size(text::BASE)
                .font_weight(theme::W_MEDIUM)
                .text_color(p.text_strong)
                .child(clock_label(t, true)),
        )
        .child(
            div()
                .text_size(px(9.))
                .font_weight(theme::W_SEMIBOLD)
                .text_color(p.text_muted)
                .child(day_label(t)),
        );
    let body = match f.app_label() {
        Some(name) => {
            let key = f.app_identity().unwrap_or_default();
            let dir = v.readout_dir;
            let fly = move |el: gpui_kit::Div, part: &str| {
                el.with_animation(
                    ElementId::Name(format!("readout-{part}-{key}").into()),
                    Animation::new(READOUT_FLY).with_easing(motion::quart),
                    move |el, t| el.left(px(-dir * READOUT_FLY_PX * (1. - t))).opacity(t),
                )
            };
            let path = f
                .app_bundle_id
                .as_ref()
                .and_then(|b| v.icons.get(b))
                .cloned();
            let inner = match path {
                Some(path) => app_icon(Src::File(path), 18.).into_any_element(),
                None => SharedString::from(f.app_fallback()).into_any_element(),
            };
            let tile = div()
                .relative()
                .size(px(24.))
                .flex_none()
                .overflow_hidden()
                .rounded(px(6.))
                .bg(mix(c, p.surface_raised, 0.34))
                .shadow(vec![spread(alpha(c, 0.55), px(1.))])
                .text_size(text::XS)
                .font_weight(gpui_kit::FontWeight::EXTRA_BOLD)
                .text_color(p.text_strong)
                .child(
                    fly(
                        div()
                            .relative()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size_full()
                            .child(inner),
                        "icon",
                    )
                    .into_any_element(),
                );
            div()
                .flex()
                .items_center()
                .gap(px(9.))
                .child(tile)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .min_w_0()
                        .child(
                            fly(
                                div()
                                    .relative()
                                    .truncate()
                                    .text_size(text::MD)
                                    .line_height(gpui_kit::relative(1.05))
                                    .font_weight(theme::W_SEMIBOLD)
                                    .text_color(p.text_strong)
                                    .child(SharedString::from(name.to_string()))
                                    // A 1 px marker the tests read the fly offset from.
                                    .child(
                                        div()
                                            .id("readout-name")
                                            .absolute()
                                            .left_0()
                                            .top_0()
                                            .size(px(1.))
                                            .test_support(),
                                    ),
                                "name",
                            )
                            .into_any_element(),
                        )
                        .child(meta),
                )
                .into_any_element()
        }
        None => div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(meta)
            .into_any_element(),
    };
    let bubble = div()
        .id("rail-readout")
        .role(Role::Tooltip)
        .min_w(px(204.))
        .max_w(px(340.))
        .min_h(px(40.))
        .pl(px(7.))
        .pr(px(10.))
        .py(px(7.))
        .rounded(radius::MD)
        .border_1()
        .border_color(p.overlay_border)
        .bg(p.pop_bg) // solid, not glass (no in-window blur: native-by-design)
        .shadow(crate::ui::popover_shadow(p))
        .child(body);
    // Anchor the bubble's bottom 6 px above the rail, centred on `x`.
    let arrow = canvas(|_, _, _| {}, {
        let c = p.overlay_border;
        move |b, _, window, _| caret(true, f32::from(b.center().x), f32::from(b.top()), c, window)
    })
    .w_full()
    .h(px(4.));
    div()
        .absolute()
        .top(px(-6. - 200.))
        .left(px(x - 200.))
        .w(px(400.))
        .h(px(200. + 4.))
        .flex()
        .flex_col()
        .justify_end()
        .items_center()
        .child(bubble)
        .child(arrow)
        .into_any_element()
}

pub fn render(
    v: &TimelineView,
    p: &Palette,
    _window: &mut Window,
    cx: &mut Context<TimelineView>,
) -> impl IntoElement {
    let frames = &v.paging.frames;
    let w = f32::from(v.rail_w.get());
    let centre = w / 2.;
    let wrap = div()
        .id("rail-wrap")
        .relative()
        .flex()
        .flex_none()
        .flex_col()
        .gap(px(4.))
        .w_full()
        .min_w_0();
    if frames.is_empty() {
        return wrap
            .child(
                // `.timeline-rail--placeholder`: the same rail, its 1 px border and
                // the edge mask in one canvas (a div's border paints over children).
                {
                    let (bg, surface, border, r) = (p.bg, p.surface, p.border, radius::MD);
                    canvas(
                        |_, _, _| {},
                        move |b, _, window, _| {
                            window.paint_quad(
                                fill(b, surface)
                                    .corner_radii(r)
                                    .border_widths(px(1.))
                                    .border_color(border),
                            );
                            edge_fade(b, bg, window);
                        },
                    )
                    .w_full()
                    .h(px(RAIL_H))
                },
            )
            .child(bands::audio_lane(v, w, p, cx));
    }
    let active = v.paging.active;
    let range = render_window(active, frames.len(), w);
    let runs = v.runs.get(v);
    let visible_runs: Vec<_> = runs
        .iter()
        .filter(|r| r.end >= range.start && r.start < range.end)
        .map(|r| (r.start, r.end, bands::run_color(v, r, p)))
        .collect();
    let tick_runs = runs.clone();
    let ticks = range.clone();
    let hovered = v.hovered.map(|h| h.0);
    let advance = v.advance_px;
    let rail_w = v.rail_w.clone();
    let rail_x = v.rail_x.clone();
    let (border_strong, subtle, muted, accent, glow, bg) = (
        p.border_strong,
        p.text_subtle,
        p.text_muted,
        p.accent,
        p.accent_glow,
        p.bg,
    );
    let (surface, border, r) = (p.surface, p.border, radius::MD);
    let paint = canvas(
        move |b, window, _| {
            rail_x.set(f32::from(b.left()));
            if rail_w.get() != b.size.width {
                rail_w.set(b.size.width);
                window.refresh();
            }
        },
        // One layer: every primitive shares its z-order, so ~300 tick quads skip
        // the scene's per-primitive overlap search (same pixels, painted in order).
        // This is the layer *under* the app icons (`.tl-group` and its resting ticks).
        move |b, _, window, _| {
            window.paint_layer(b, |window| {
                // The rail's fill and 1 px border sit under the edge fade (CSS masks
                // them with the rest): `b` is inside the rail's transparent border.
                window.paint_quad(
                    fill(b.dilate(px(1.)), surface)
                        .corner_radii(r)
                        .border_widths(px(1.))
                        .border_color(border),
                );
                let c = f32::from(b.left()) + f32::from(b.size.width) / 2.;
                let track = Bounds::new(
                    point(b.left(), b.top() + px(1.)),
                    size(b.size.width, px(RAIL_H - 2.)),
                );
                bands::paint_bands(track, c, advance, &visible_runs, window);
                let mid = f32::from(b.top()) + 1. + 8. + 9.;
                // The buffer slots past each edge are clipped anyway: don't paint them.
                let (lo, hi) = (f32::from(b.left()) - SLOT_W, f32::from(b.right()) + SLOT_W);
                for i in ticks.clone() {
                    let x = slot_x(c, advance, i);
                    if x < lo || x > hi || i == active || hovered == Some(i) {
                        continue;
                    }
                    // A major tick sits at the oldest frame of a run an older run follows.
                    let major = tick_runs
                        .binary_search_by_key(&i, |r| r.end)
                        .is_ok_and(|k| tick_runs[k].boundary_frame_id.is_some());
                    let (th, color) = if major {
                        (16., subtle)
                    } else {
                        (8., border_strong)
                    };
                    tick(x, mid, 1., th, color, window);
                }
            })
        },
    )
    .absolute()
    .inset_0();
    // Over the icons: the active and hovered ticks, the edge mask and the carets.
    let over = canvas(
        |_, _, _| {},
        move |b, _, window, _| {
            let c = f32::from(b.left()) + f32::from(b.size.width) / 2.;
            let mid = f32::from(b.top()) + 1. + 8. + 9.;
            if let Some(i) = hovered.filter(|&h| h != active) {
                tick(slot_x(c, advance, i), mid, 1., 12., muted, window);
            }
            let x = slot_x(c, advance, active);
            let (tw, th) = (2., 22.);
            // `0 0 0 1px glow, 0 0 10px glow`: a ring plus a soft blurred halo.
            window.paint_drop_shadows(
                Bounds::new(
                    point(px(x - tw / 2.), px(mid - th / 2.)),
                    size(px(tw), px(th)),
                ),
                gpui_kit::Corners::all(px(0.5)),
                &[
                    spread(glow, px(1.)),
                    gpui_kit::BoxShadow {
                        blur_radius: px(10.),
                        ..spread(glow, px(0.))
                    },
                ],
            );
            tick(x, mid, tw, th, accent, window);
            edge_fade(b.dilate(px(1.)), bg, window);
            caret(true, c, f32::from(b.top()) - 1., subtle, window);
            caret(false, c, f32::from(b.top()) + 30., subtle, window);
        },
    )
    .absolute()
    .inset_0();
    let n = frames.len();
    let readout_text = {
        let f = &frames[active];
        let t = local(f.captured_at_ms, crate::ui::clock::now(cx).offset());
        format!(
            "Frame {} of {n}{} — captured {} {}",
            active + 1,
            if v.paging.exhausted { "" } else { "+" },
            day_label(t),
            clock_label(t, true)
        )
    };
    let rail = div()
        .id("rail")
        .role(Role::Slider)
        .aria_label("Timeline scrubber")
        .aria_numeric_value((active + 1) as f64)
        .aria_value(readout_text)
        // `onTimelineKeyDown`: the focused rail also takes Home / End / Page keys.
        .track_focus(&v.rail_focus)
        .on_key_down(cx.listener(move |this, e: &gpui_kit::KeyDownEvent, _, cx| {
            if e.keystroke.modifiers.modified() {
                return;
            }
            let delta = match e.keystroke.key.as_str() {
                "pageup" => -10,
                "pagedown" => 10,
                "home" => -(n as i64),
                "end" => n as i64,
                _ => return,
            };
            cx.stop_propagation();
            this.step(delta, cx);
        }))
        .relative()
        .w_full()
        .h(px(RAIL_H))
        .rounded(radius::MD)
        // Transparent, so the clip is the whole box and `paint` draws the border.
        .border_1()
        .border_color(gpui_kit::transparent_black())
        .overflow_hidden()
        .cursor_pointer()
        .child(paint)
        .children(bands::icons(v, &runs, range, centre, p))
        .children(bands::match_mark(v, centre, p))
        .child(over)
        .on_mouse_move(
            cx.listener(move |this, e: &gpui_kit::MouseMoveEvent, _, cx| {
                let w = f32::from(this.rail_w.get());
                let local_x = f32::from(e.position.x) - this.rail_x.get();
                let off = ((w / 2. - local_x) / SLOT_W).round() as i64;
                let i = (this.paging.active as i64 + off).clamp(0, n as i64 - 1) as usize;
                if this.hovered.map(|h| h.0) != Some(i) {
                    this.hovered = Some((i, px(local_x)));
                    cx.notify();
                }
            }),
        )
        .on_hover(cx.listener(|this, on: &bool, _, cx| {
            if !on {
                this.hovered = None;
                cx.notify();
            }
        }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, e: &gpui_kit::MouseDownEvent, window, cx| {
                // Arrow / Home / End scrubbing works next without a Tab back.
                window.focus(&this.rail_focus, cx);
                let w = f32::from(this.rail_w.get());
                let off = ((w / 2. - (f32::from(e.position.x) - this.rail_x.get())) / SLOT_W)
                    .round() as i64;
                let i = (this.paging.active as i64 + off).clamp(0, n as i64 - 1) as usize;
                this.advance_px = i as f32 * SLOT_W;
                this.engine.velocity = 0.;
                this.scrub_to(i, true, cx);
            }),
        );
    let note: Option<AnyElement> = if v.paging.loading_more {
        Some(
            crate::ui::notice::inline(
                crate::ui::Tone::Neutral,
                Some(crate::ui::progress::spinner(sz::SPIN_SM).into_any_element()),
                vec!["Loading older frames…".into_any_element()],
                p,
            )
            .into_any_element(),
        )
    } else if v.paging.exhausted && n - active <= PREFETCH_AHEAD {
        let text = match v.settings.keep_days {
            Some(days) => format!("Start of history · frames older than {days} days are removed"),
            None => "Start of history".to_string(),
        };
        Some(
            crate::ui::notice::inline(
                crate::ui::Tone::Neutral,
                Some(icon(IconName::Info.path(), sz::ICON_SM).into_any_element()),
                vec![text.into_any_element()],
                p,
            )
            .into_any_element(),
        )
    } else {
        None
    };
    let (ri, rx) = match v.hovered {
        Some((i, x)) => (i, f32::from(x)),
        None => (active, centre),
    };
    wrap.child(rail)
        .child(bands::audio_lane(v, w, p, cx))
        .children(note.map(|n| {
            div()
                .absolute()
                .left(px(10.))
                .top(px(6.))
                .h(px(24.))
                .px(px(10.))
                .flex()
                .items_center()
                .rounded(radius::SM)
                .border_1()
                .border_color(p.border)
                .bg(p.surface_raised)
                .child(n)
        }))
        .when(ri < n && w > 0., |d| d.child(readout(v, ri, rx, p, cx)))
}
