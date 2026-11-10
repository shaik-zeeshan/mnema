//! Mixed-size text on one line (CSS `align-items: baseline`).
//!
//! GPUI's text leaves report no baseline to taffy, so `items_baseline()` falls
//! back to each box's bottom edge: a smaller or differently-leaded line sits
//! lower than its neighbour. `row` pads each item from its font's metrics so
//! every baseline lands the same height above the row's bottom.

use gpui_kit::prelude::*;
use gpui_kit::{div, font, relative, AnyElement, App, Div, Pixels};

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
pub fn below(l: Line, cx: &App) -> Pixels {
    let ts = cx.text_system();
    let id = ts.resolve_font(&font(l.family));
    let ascent = ts.ascent(id, l.size);
    // Font metrics store the descent as a negative offset.
    let descent = ts.descent(id, l.size).abs();
    // `TextStyle::line_height_in_pixels` rounds to whole pixels.
    let line = (l.size * l.lh).round();
    (line - ascent + descent) / 2.
}

/// A flex row whose items share one baseline (the last line's, if one wraps).
pub fn row(items: Vec<(Line, AnyElement)>, cx: &App) -> Div {
    let below: Vec<Pixels> = items.iter().map(|(l, _)| below(*l, cx)).collect();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{self, text, FONT_MONO, FONT_UI};
    use capture_types::AppearanceSetting;
    use gpui_kit::{px, TestAppContext, WindowAppearance};

    #[gpui_kit::test]
    fn smaller_type_sits_closer_to_the_box_bottom(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            theme::init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
            let small = below(Line::new(FONT_MONO, text::XS, 1.6), cx);
            let big = below(Line::new(FONT_MONO, text::MD, 1.6), cx);
            let ui = below(Line::new(FONT_UI, text::BASE, 1.6), cx);
            // Without the pad, XS mono beside MD mono sits this much lower.
            assert!(big - small > px(0.5), "{small:?} vs {big:?}");
            assert!(ui > Pixels::ZERO && ui < text::BASE);
        });
    }
}
