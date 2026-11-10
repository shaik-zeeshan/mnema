//! The ruler receipt (every face but the device limit): the update window
//! drawn as the Timeline's tick ruler, from today to the last day of included
//! updates, on the paper's darker top half; the face's words and buttons
//! below the tear line.
//!
//! - Activated: once, the green playhead waits for the modal to arrive, scrubs
//!   from today to the end date (the readout counting the dates, every week
//!   it passes lit) and lands, the end tick growing into the active tick.
//! - Renewed: the same scrub from the old end date to the new one; the old end
//!   keeps its tick and the old date shows struck through below.
//! - Pending: the playhead parked at today, the provisional days a dashed stretch.
//! - Working / Failed: the same band with only today on it (a spinner or a
//!   warning where the date will be), so the receipt keeps its shape when the
//!   result lands. Reduce Motion paints the landed frame.

use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::AnimationExt as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{
    canvas, div, fill, point, px, relative, size as sz, Animation, AnyElement, Bounds, Div, Hsla,
    PathBuilder, SharedString, Window,
};
use time::{Date, Month, OffsetDateTime};

use super::banner::days;
use super::parts::{self, Parts, CELEBRATE};
use super::receipt::Face;
use crate::theme::{self, motion, size, space, text, Palette, FONT_MONO};
use crate::ui::chip::{chip, ChipKind};
use crate::ui::{alpha, icon, kicker, progress};

/// The playhead holds at today while the modal rises in...
const HOLD: Duration = Duration::from_millis(200);
/// ...scrubs to the end on the quart curve...
const SCRUB: Duration = Duration::from_millis(950);
/// ...and the end tick lands over the last stretch (ending at `CELEBRATE`).
const LAND: Duration = Duration::from_millis(350);

const RULER_H: f32 = 34.;
/// Tick heights: weekly (or daily), month start, the landed end.
const MINOR: f32 = 6.;
const MAJOR: f32 = 12.;
const LANDED: f32 = 24.;
const DAY_MS: i64 = 86_400_000;

/// Where the scrub stands `ms` into the celebration: the playhead's travel
/// (0..1 from where it starts) and the landing (0..1).
pub fn timing(ms: f32) -> (f32, f32) {
    let stage = |from: Duration, len: Duration| {
        motion::quart(((ms - from.as_millis() as f32) / len.as_millis() as f32).clamp(0., 1.))
    };
    (stage(HOLD, SCRUB), stage(CELEBRATE - LAND, LAND))
}

/// What the band draws.
#[derive(Clone)]
enum Band {
    /// Today → `to_ms`, scrubbing from `from` (0..1) to the end.
    Scrub { to_ms: i64, from: f32 },
    /// Today → today + `days`, the playhead parked at today.
    Parked { days: u32 },
    /// Only today: the result hasn't landed, never will, or has no end date.
    Empty(Status),
}

#[derive(Clone, Copy, PartialEq)]
enum Status {
    Waiting,
    Failed,
    /// A licence whose updates never end (no date to scrub to).
    Open,
}

pub fn render(face: &Face, parts: Parts, p: &Palette) -> AnyElement {
    let Parts {
        copy,
        buttons,
        note,
        now,
    } = parts;
    let now_ms = now.unix_timestamp() * 1000;
    let x_of = |ms: i64, to_ms: i64| {
        ((ms - now_ms) as f32 / (to_ms - now_ms).max(DAY_MS) as f32).clamp(0., 1.)
    };
    // A window that already ended has nowhere to scrub: it shows landed.
    let scrub = |to_ms: i64, from: f32| match parts::date(to_ms) {
        Some(_) if to_ms <= now_ms => Band::Scrub { to_ms, from: 1. },
        Some(_) => Band::Scrub { to_ms, from },
        None => Band::Empty(Status::Open),
    };
    let (kick, band) = match face {
        Face::Activated {
            update_through_ms, ..
        } => ("Updates included through", scrub(*update_through_ms, 0.)),
        Face::Renewed {
            update_through_ms,
            was_ms,
        } => (
            "Updates included through",
            scrub(
                *update_through_ms,
                was_ms.map_or(0., |w| x_of(w, *update_through_ms)),
            ),
        ),
        Face::Pending {
            provisional_days_left,
            ..
        } => (
            "Confirming this Mac",
            Band::Parked {
                days: *provisional_days_left,
            },
        ),
        Face::Failed { .. } => ("Updates included through", Band::Empty(Status::Failed)),
        _ => ("Updates included through", Band::Empty(Status::Waiting)),
    };
    // Activated says what the ruler can't; every other face keeps its copy.
    let sub = match face {
        Face::Activated { .. } => {
            Some("Mnema is yours to keep, with every update until then.".to_string())
        }
        _ => copy.sub,
    };
    let words = div()
        .flex()
        .flex_col()
        .gap(space::S1)
        .px(space::S5)
        .pt(space::S5)
        .pb(space::S5)
        .children(copy.badge.map(|b| {
            div()
                .flex()
                .pb(space::S1)
                .child(chip(b, ChipKind::Neutral, p))
        }))
        .child(parts::headline(copy.title, text::XL, p).pb(space::S1))
        .children(copy.who.map(|w| parts::line(w, text::MD, p.text)))
        .children(sub.map(|s| parts::line(s, text::BASE, p.text_muted)))
        .children(
            copy.was
                .map(|w| parts::line(w, text::BASE, p.text_subtle).line_through()),
        )
        .child(parts::actions(buttons, note).pt(space::S3));
    parts::body(copy.title)
        .child(parts::paper(
            band_view(kick, band, now, p),
            p.surface_subtle,
            words,
            p,
        ))
        .test_support()
        .into_any_element()
}

/// Weeks and month starts between `from` and `to_ms`, as 0..1 positions.
fn marks(from: OffsetDateTime, to_ms: i64) -> (Vec<f32>, Vec<(f32, SharedString)>) {
    let from_ms = from.unix_timestamp() * 1000;
    let span = (to_ms - from_ms).max(DAY_MS) as f32;
    let at = |ms: i64| (ms - from_ms) as f32 / span;
    let weeks = (1..)
        .map(|w| from_ms + w * 7 * DAY_MS)
        .take_while(|&ms| ms < to_ms)
        .map(at)
        .collect();
    let mut months = Vec::new();
    let (mut y, mut m) = (from.year(), from.month());
    loop {
        (y, m) = if m == Month::December {
            (y + 1, Month::January)
        } else {
            (y, m.next())
        };
        let Ok(d) = Date::from_calendar_date(y, m, 1) else {
            break;
        };
        let ms = d.midnight().assume_offset(from.offset()).unix_timestamp() * 1000;
        if ms >= to_ms || months.len() > 48 {
            break;
        }
        let label = if m == Month::January {
            y.to_string()
        } else {
            m.to_string()[..3].to_uppercase()
        };
        months.push((at(ms), label.into()));
    }
    // More than about a month per 30 px crowds the labels: keep every other.
    let step = months.len().div_ceil(12).max(1);
    let months = months.into_iter().step_by(step).collect();
    (weeks, months)
}

fn tick(x: f32, base: f32, h: f32, w: f32, color: Hsla, window: &mut Window) {
    window.paint_quad(
        fill(
            Bounds::new(point(px(x - w / 2.), px(base - h)), sz(px(w), px(h))),
            color,
        )
        .corner_radii(px(0.5)),
    );
}

/// One frame of the ruler.
#[derive(Clone, Copy)]
struct Frame {
    /// The playhead (0..1 across the ruler).
    e: f32,
    /// Where the scrub started (the renewed face's old end).
    from: f32,
    land: f32,
}

const STILL: Frame = Frame {
    e: 0.,
    from: 0.,
    land: 0.,
};

#[derive(Clone, Default)]
struct Ticks {
    minor: Vec<f32>,
    major: Vec<f32>,
}

fn ruler(band: &Band, ticks: Ticks, f: Frame, p: Palette) -> impl IntoElement {
    let band = band.clone();
    canvas(
        |_, _, _| {},
        move |b, _, window, _| {
            let (x0, w) = (f32::from(b.left()), f32::from(b.size.width));
            let base = f32::from(b.top()) + RULER_H - 6.;
            let x = |t: f32| x0 + t * w;
            let quad = |window: &mut Window, to: f32, top: f32, h: f32, c: Hsla| {
                window.paint_quad(fill(
                    Bounds::new(point(px(x0), px(top)), sz(px(to * w), px(h))),
                    c,
                ));
            };
            let scrub = matches!(band, Band::Scrub { .. });
            if scrub {
                // The covered weeks: a soft wash, the baseline green behind the playhead.
                quad(
                    window,
                    f.e,
                    base - LANDED,
                    LANDED,
                    alpha(p.accent_glow, 0.5),
                );
            }
            quad(window, 1., base, 1., p.border_strong);
            match band {
                Band::Scrub { .. } => quad(window, f.e, base - 0.5, 2., p.accent),
                Band::Parked { .. } => {
                    let mut dash = PathBuilder::stroke(px(2.)).dash_array(&[px(4.), px(3.)]);
                    dash.move_to(point(px(x(0.)), px(base)));
                    dash.line_to(point(px(x(1.)), px(base)));
                    if let Ok(path) = dash.build() {
                        window.paint_path(path, p.accent);
                    }
                }
                Band::Empty(_) => {}
            }
            let lit = |t: f32, on: Hsla, off: Hsla| if scrub && t <= f.e { on } else { off };
            for &t in &ticks.minor {
                let c = lit(t, p.accent_strong, p.chart_grey_2);
                tick(x(t), base, MINOR, 1., c, window);
            }
            for &t in &ticks.major {
                let c = lit(t, p.accent, p.chart_grey_3);
                tick(x(t), base, MAJOR, 1.5, c, window);
            }
            tick(x(0.), base, MAJOR, 1.5, p.chart_grey_4, window);
            if f.from > 0. {
                // The renewed face's old end keeps its mark.
                tick(x(f.from), base, MAJOR + 4., 1.5, p.text_faint, window);
            }
            match band {
                Band::Scrub { .. } => {
                    // The end tick grows into the Timeline's active tick as the playhead lands.
                    let th = MAJOR + (LANDED - MAJOR) * f.land;
                    for (pad, a) in [(5., 0.35 * f.land), (1., f.land)] {
                        window.paint_quad(
                            fill(
                                Bounds::new(
                                    point(px(x(1.) - 1. - pad), px(base - th - pad)),
                                    sz(px(2. + pad * 2.), px(th + pad * 2.)),
                                ),
                                alpha(p.accent_glow, a * p.accent_glow.a),
                            )
                            .corner_radii(px(pad)),
                        );
                    }
                    let c = if f.land > 0. {
                        p.accent
                    } else {
                        p.chart_grey_4
                    };
                    tick(x(1.), base, th, 2., c, window);
                    if f.land < 1. {
                        playhead(x(f.e), base, alpha(p.accent, 1. - f.land), window);
                    }
                }
                Band::Parked { .. } => {
                    tick(x(1.), base, MAJOR, 1.5, p.chart_grey_4, window);
                    playhead(x(0.), base, p.accent, window);
                }
                Band::Empty(status) => {
                    if status == Status::Waiting {
                        playhead(x(0.), base, p.chart_grey_4, window);
                    }
                }
            }
        },
    )
    .absolute()
    .inset_0()
}

/// Welcome's playhead: a 1.5 px line under a 9.5×6 cap.
fn playhead(x: f32, base: f32, c: Hsla, window: &mut Window) {
    let top = base - LANDED - 4.;
    window.paint_quad(fill(
        Bounds::new(point(px(x - 0.75), px(top)), sz(px(1.5), px(LANDED + 4.))),
        c,
    ));
    window.paint_quad(
        fill(
            Bounds::new(point(px(x - 4.75), px(top)), sz(px(9.5), px(6.))),
            c,
        )
        .corner_radii(gpui_kit::Corners {
            bottom_left: px(5.),
            bottom_right: px(5.),
            ..Default::default()
        }),
    );
}

/// The band: kicker, the big mono readout, the ruler and its labels.
fn band_view(kick: &'static str, band: Band, now: OffsetDateTime, p: &Palette) -> Div {
    let p = *p;
    let now_ms = now.unix_timestamp() * 1000;
    let offset = now.offset();
    let readout = move |child: AnyElement| {
        div()
            .flex()
            .items_center()
            .h(px(28.))
            .font_family(FONT_MONO)
            .font_weight(theme::W_MEDIUM)
            .text_size(text::XL)
            .text_color(p.text_strong)
            .child(child)
    };
    let strip = || div().relative().h(px(RULER_H));
    let label = move || {
        div()
            .absolute()
            .font_family(FONT_MONO)
            .text_size(text::MICRO)
            .text_color(p.text_subtle)
    };
    let mut labels = div()
        .relative()
        .h(px(12.))
        .child(label().left_0().child("TODAY"));
    let column = || div().flex().flex_col().gap(space::S2);
    let animated: AnyElement = match band.clone() {
        Band::Scrub { to_ms, from } => {
            let (weeks, months) = marks(now, to_ms);
            let ticks = Ticks {
                minor: weeks,
                major: months.iter().map(|(t, _)| *t).collect(),
            };
            // A month too close to TODAY would sit on its label.
            labels = labels.children(
                months
                    .into_iter()
                    .filter(|(t, _)| *t > 0.1)
                    .map(|(t, l)| label().left(relative(t)).ml(px(-9.)).child(l)),
            );
            let date_at = move |e: f32| {
                let ms = now_ms + ((to_ms - now_ms) as f64 * e as f64) as i64;
                parts::date(ms)
                    .map(|t| parts::stamp_date(t.to_offset(offset)))
                    .unwrap_or_default()
            };
            column()
                .with_animation("receipt-scrub", Animation::new(CELEBRATE), move |el, u| {
                    let (travel, land) = timing(u * CELEBRATE.as_millis() as f32);
                    let e = from + (1. - from) * travel;
                    let f = Frame { e, from, land };
                    let date = date_at(e);
                    let shown = readout(date.clone().into_any_element())
                        .id("receipt-readout")
                        .aria_label(date)
                        .test_support();
                    el.child(shown)
                        .child(strip().child(ruler(&band, ticks.clone(), f, p)))
                })
                .into_any_element()
        }
        Band::Parked { days: n } => {
            let end = now + time::Duration::days(n as i64);
            let end_label = format!("{} {:02}", parts::short_month(end), end.day());
            labels = labels.child(label().right_0().child(SharedString::from(end_label)));
            let minor = (1..n).map(|d| d as f32 / n as f32).collect();
            let words = format!("{} to connect", days(n)).to_uppercase();
            column()
                .child(readout(SharedString::from(words).into_any_element()))
                .child(strip().child(ruler(
                    &band,
                    Ticks {
                        minor,
                        major: vec![],
                    },
                    STILL,
                    p,
                )))
                .into_any_element()
        }
        Band::Empty(status) => {
            let glyph = match status {
                Status::Waiting => progress::spinner(size::SPIN_LG).into_any_element(),
                Status::Failed => div()
                    .text_color(p.warn)
                    .child(icon(IconName::TriangleAlert.path(), size::SPIN_LG))
                    .into_any_element(),
                Status::Open => div()
                    .text_color(p.accent)
                    .child(icon(IconName::Check.path(), size::SPIN_LG))
                    .into_any_element(),
            };
            column()
                .child(readout(glyph))
                .child(strip().child(ruler(&band, Ticks::default(), STILL, p)))
                .into_any_element()
        }
    };
    div()
        .flex()
        .flex_col()
        .gap(space::S3)
        .px(space::S5)
        .pt(space::S5)
        .pb(space::S5)
        .child(kicker(kick, &p))
        .child(animated)
        .child(labels)
}
