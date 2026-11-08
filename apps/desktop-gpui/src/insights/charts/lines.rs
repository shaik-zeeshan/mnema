//! Polylines on a canvas: `Sparkline.svelte` (120×32 box, pad 2, stretched)
//! and `SubjectChart.svelte` (confidence over time with a y-axis, gridlines,
//! the dashed display floor, grey lines, the selected line bold over a soft
//! area, and evidence dots).

use gpui_kit::prelude::*;
use gpui_kit::{canvas, div, point, px, Bounds, Div, Hsla, PathBuilder, Pixels, Window};

use crate::theme::{space, text, Palette, FONT_MONO};
use crate::ui::alpha;

/// One line: points in 0..1 (x across, y = confidence up), colour, width, dashed.
#[derive(Clone)]
pub struct Line {
    pub points: Vec<(f32, f32)>,
    pub color: Hsla,
    pub width: f32,
    pub dashed: bool,
}

fn stroke(
    b: Bounds<Pixels>,
    pts: &[(f32, f32)],
    color: Hsla,
    w: f32,
    dash: bool,
    pad: f32,
    window: &mut Window,
) {
    if pts.len() < 2 {
        return;
    }
    let (x0, y0) = (f32::from(b.left()) + pad, f32::from(b.top()) + pad);
    let (bw, bh) = (
        f32::from(b.size.width) - 2. * pad,
        f32::from(b.size.height) - 2. * pad,
    );
    let at = |(x, y): (f32, f32)| point(px(x0 + x * bw), px(y0 + (1. - y.clamp(0., 1.)) * bh));
    let mut builder = PathBuilder::stroke(px(w));
    if dash {
        builder = builder.dash_array(&[px(2.), px(2.)]);
    }
    builder.move_to(at(pts[0]));
    for p in &pts[1..] {
        builder.line_to(at(*p));
    }
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

/// `Sparkline.svelte`: each series' points evenly spaced; faded = 40% + dashed.
pub fn sparkline(series: Vec<(Hsla, bool, Vec<f64>)>) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |b, _, window, _| {
            for (c, faded, pts) in &series {
                let n = pts.len().max(2) as f32 - 1.;
                let pts: Vec<(f32, f32)> = pts
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (i as f32 / n, *v as f32))
                    .collect();
                let color = if *faded { alpha(*c, 0.4) } else { *c };
                stroke(b, &pts, color, 1.5, *faded, 2., window);
            }
        },
    )
    .w_full()
    .h_full()
}

/// `SubjectChart.svelte`: 230 px plot, 34 px y-axis, x labels under it.
/// `area` is filled under the first bold line; `dots` are evidence points
/// `(x, y, origin)`; `floor` is the dashed display floor (0..1).
pub fn line_chart(
    lines: Vec<Line>,
    dots: Vec<(f32, f32, bool)>,
    floor: f32,
    x_labels: [String; 3],
    p: &Palette,
) -> Div {
    const H: f32 = 230.;
    let ticks = [0, 25, 50, 75, 100];
    let (grid, base, floor_c, area) = (
        p.hairline,
        p.border,
        p.border_strong,
        alpha(p.text_strong, 0.07),
    );
    let fill_line = lines.iter().find(|l| l.width >= 2.).cloned();
    let plot = canvas(
        |_, _, _| {},
        move |b, _, window, _| {
            for v in [25., 50., 75., 100.] {
                stroke(
                    b,
                    &[(0., v / 100.), (1., v / 100.)],
                    grid,
                    1.,
                    false,
                    0.,
                    window,
                );
            }
            stroke(b, &[(0., 0.), (1., 0.)], base, 1., false, 0., window);
            let mut dash = PathBuilder::stroke(px(1.)).dash_array(&[px(3.), px(4.)]);
            let fy = f32::from(b.top()) + (1. - floor) * f32::from(b.size.height);
            dash.move_to(point(b.left(), px(fy)));
            dash.line_to(point(b.right(), px(fy)));
            if let Ok(path) = dash.build() {
                window.paint_path(path, floor_c);
            }
            if let Some(l) = &fill_line {
                let (x0, y0) = (f32::from(b.left()), f32::from(b.top()));
                let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
                let mut fill = PathBuilder::fill();
                let mut pts: Vec<_> = l
                    .points
                    .iter()
                    .map(|(x, y)| point(px(x0 + x * w), px(y0 + (1. - y) * h)))
                    .collect();
                if let (Some(first), Some(last)) = (l.points.first(), l.points.last()) {
                    pts.push(point(px(x0 + last.0 * w), px(y0 + h)));
                    pts.push(point(px(x0 + first.0 * w), px(y0 + h)));
                }
                fill.add_polygon(&pts, true);
                if let Ok(path) = fill.build() {
                    window.paint_path(path, area);
                }
            }
            for l in &lines {
                stroke(b, &l.points, l.color, l.width, l.dashed, 0., window);
            }
        },
    )
    .absolute()
    .inset_0();
    let label = |s: String| {
        div()
            .font_family(FONT_MONO)
            .text_size(text::XS)
            .text_color(p.text_faint)
            .child(s)
    };
    let dot_el = |(x, y, origin): (f32, f32, bool)| {
        div()
            .absolute()
            .left(gpui_kit::relative(x))
            .top(gpui_kit::relative(1. - y))
            .ml(px(-4.5))
            .mt(px(-4.5))
            .size(px(9.))
            .rounded_full()
            .border_2()
            .when(origin, |d| d.border_color(p.accent).bg(p.accent))
            .when(!origin, |d| d.border_color(p.text_strong).bg(p.surface))
    };
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .relative()
                .h(px(H))
                .mt(space::S5)
                .ml(px(34.))
                .child(
                    div()
                        .absolute()
                        .left(px(-34.))
                        .top_0()
                        .bottom_0()
                        .w(px(26.))
                        .children(ticks.map(|v| {
                            div()
                                .absolute()
                                .right_0()
                                .bottom(gpui_kit::relative(v as f32 / 100.))
                                .mb(px(-5.))
                                .child(label(v.to_string()))
                        })),
                )
                .child(plot)
                .child(
                    div()
                        .absolute()
                        .right_0()
                        .top(gpui_kit::relative(1. - floor))
                        .mt(px(-14.))
                        .child(label(format!("display floor {}%", (floor * 100.).round()))),
                )
                .children(dots.into_iter().map(dot_el)),
        )
        .child({
            let [a, b, c] = x_labels;
            div()
                .flex()
                .justify_between()
                .mt(space::S2)
                .ml(px(34.))
                .child(label(a))
                .child(label(b))
                .child(label(c))
        })
}
