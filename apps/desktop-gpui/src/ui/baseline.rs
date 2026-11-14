//! Mixed-size text on one line (CSS `align-items: baseline`).
//!
//! GPUI's text leaves report no baseline to taffy, so `items_baseline()` falls
//! back to each box's bottom edge: a smaller or differently-leaded line sits
//! lower than its neighbour. `row` pads each item from its font's metrics so
//! every baseline lands the same height above the row's bottom.

use std::sync::OnceLock;

use gpui_kit::prelude::*;
use gpui_kit::{div, font, px, relative, AnyElement, App, Div, Pixels};

use crate::theme::{FONT_MONO, FONT_UI};

/// (ascent, descent) per em of `[FONT_UI, FONT_MONO]`. The fonts are embedded, so
/// these are read once and views need no `cx` to line text up.
static METRICS: OnceLock<[(f32, f32); 2]> = OnceLock::new();

/// Read the font metrics. `theme::init` calls it once the fonts are registered.
pub fn init(cx: &App) {
    let (ts, em) = (cx.text_system(), px(1000.));
    METRICS.get_or_init(|| {
        [FONT_UI, FONT_MONO].map(|f| {
            let id = ts.resolve_font(&font(f));
            // Font metrics store the descent as a negative offset.
            (ts.ascent(id, em) / em, ts.descent(id, em).abs() / em)
        })
    });
}

/// The single line of text inside one row item. The wrapper sets these on the
/// item, so they must match any the item sets on itself.
#[derive(Clone, Copy)]
pub struct Line {
    pub family: &'static str,
    pub size: Pixels,
    /// Line height as a multiple of `size`.
    pub lh: f32,
}

impl Line {
    pub fn new(family: &'static str, size: Pixels, lh: f32) -> Self {
        Self { family, size, lh }
    }
}

/// Distance from the bottom of a one-line text box to its baseline, the way
/// GPUI paints it: half the leading, then the descent.
pub fn below(l: Line) -> Pixels {
    // Before `init` (a test with no theme) the metrics are zero, which centres the items.
    let m = METRICS.get().copied().unwrap_or_default();
    let (ascent, descent) = m[(l.family == FONT_MONO) as usize];
    // `TextStyle::line_height_in_pixels` rounds to whole pixels.
    let line = (l.size * l.lh).round();
    (line - l.size * ascent + l.size * descent) / 2.
}

/// A flex row whose items share one baseline (the last line's, if one wraps).
pub fn row(items: Vec<(Line, AnyElement)>) -> Div {
    let below: Vec<Pixels> = items.iter().map(|(l, _)| below(*l)).collect();
    let max = below.iter().copied().fold(Pixels::ZERO, Pixels::max);
    div()
        .flex()
        .items_end()
        .children(items.into_iter().zip(below).map(|((l, el), b)| {
            div()
                .font_family(l.family)
                .text_size(l.size)
                .line_height(relative(l.lh))
                .pb(max - b)
                .child(el)
        }))
}

/// GPUI's default line height (`phi()`), for an item that inherits it.
pub const PHI: f32 = 1.618_034;

/// Top padding per item so every first-line baseline lands at the same depth.
fn tops(lines: &[Line]) -> Vec<Pixels> {
    let above: Vec<Pixels> = lines
        .iter()
        .map(|l| (l.size * l.lh).round() - below(*l))
        .collect();
    let max = above.iter().copied().fold(Pixels::ZERO, Pixels::max);
    above.into_iter().map(|a| max - a).collect()
}

/// `row` aligned on each item's FIRST line (CSS), for items that wrap or must
/// keep their own flex sizing: styles and pads the item itself, no wrapper.
/// Each pad is given back below the item, so the row stays as tall as a
/// top-aligned one and nothing around it moves.
pub fn row_top(items: Vec<(Line, Div)>) -> Div {
    let lines: Vec<Line> = items.iter().map(|(l, _)| *l).collect();
    div()
        .flex()
        .items_start()
        .children(items.into_iter().zip(tops(&lines)).map(|((l, el), t)| {
            el.font_family(l.family)
                .text_size(l.size)
                .line_height(relative(l.lh))
                .pt(t)
                .mb(-t)
        }))
}

/// Bottom padding that lifts `l`'s baseline level with the highest of `all` in
/// an `items_end` row. For items that keep their own flex sizing (a truncating
/// name), which `row`'s wrappers would drop.
pub fn lift(l: Line, all: &[Line]) -> Pixels {
    let max = all
        .iter()
        .map(|&x| below(x))
        .fold(Pixels::ZERO, Pixels::max);
    max - below(l)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{self, text};
    use capture_types::AppearanceSetting;
    use gpui_kit::{TestAppContext, WindowAppearance};

    #[gpui_kit::test]
    fn first_lines_share_a_baseline(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            theme::init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
            let ls = [
                Line::new(FONT_UI, text::MD, PHI),
                Line::new(FONT_MONO, text::XS, 1.),
            ];
            let t = tops(&ls);
            // The taller line is not padded; each baseline sits at pad + above.
            assert_eq!(t[0], Pixels::ZERO);
            let depth = |i: usize| t[i] + (ls[i].size * ls[i].lh).round() - below(ls[i]);
            assert!((depth(0) - depth(1)).abs() < px(0.01), "{t:?}");
        });
    }

    #[gpui_kit::test]
    fn smaller_type_sits_closer_to_the_box_bottom(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            theme::init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
            let small = below(Line::new(FONT_MONO, text::XS, 1.6));
            let big = below(Line::new(FONT_MONO, text::MD, 1.6));
            let ui = below(Line::new(FONT_UI, text::BASE, 1.6));
            // Without the pad, XS mono beside MD mono sits this much lower.
            assert!(big - small > px(0.5), "{small:?} vs {big:?}");
            assert!(ui > Pixels::ZERO && ui < text::BASE);
        });
    }
}
