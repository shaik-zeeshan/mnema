//! The kit pieces every Insights tab is built from: the read (`.in-read`:
//! kicker · display headline · prose beside a 2×2 of `.mx-stat`s), section
//! heads, panels, the h2 and the label voice.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, AnyElement, Div, Hsla, Pixels, SharedString, Window};

use crate::theme::{self, breakpoint, radius, space, text, Palette, FONT_MONO, FONT_UI};
use crate::ui::baseline::{self, Line};
use crate::ui::kicker;
use crate::ui::Tracking as _;

/// `max-width: 24ch` at 40 px: Hanken Grotesk's `0` advances 0.56 em.
const DISPLAY_MAX_W: Pixels = px(24. * 0.56 * 40.);
const DISPLAY_TRACKING: f32 = -0.034;

/// `.mx-display` (40/1.04 semibold, 24ch, `text-wrap: balance`) with an
/// optional de-emphasised turn.
pub fn display(
    head: impl Into<SharedString>,
    em: Option<String>,
    p: &Palette,
    window: &Window,
) -> Div {
    let head: SharedString = head.into();
    let tail = em.filter(|e| !e.is_empty()).map(|e| format!(" {e}"));
    let mut runs = vec![(head.as_ref(), theme::W_SEMIBOLD)];
    runs.extend(tail.as_deref().map(|t| (t, theme::W_MEDIUM)));
    // shortcut: balances against 24ch, not the laid-out column; a column
    // narrower than 24ch wraps greedily inside the balanced width.
    let w = crate::ui::balanced_width(
        &runs,
        FONT_UI,
        text::DISPLAY,
        DISPLAY_TRACKING,
        DISPLAY_MAX_W,
        window,
    );
    let text_el = match &tail {
        None => crate::chat::inline_runs(&[(head.as_ref(), None)]),
        Some(t) => crate::chat::inline_runs(&[
            (head.as_ref(), None),
            (t.as_str(), Some((theme::W_MEDIUM, p.text_subtle))),
        ]),
    };
    div()
        .my(space::S4)
        .max_w(w)
        .text_size(text::DISPLAY)
        .tracking(DISPLAY_TRACKING)
        .line_height(relative(1.04))
        .font_weight(theme::W_SEMIBOLD)
        .text_color(p.text_strong)
        .child(text_el)
}

/// `.mx-prose`: 14 / 1.6, muted.
pub fn prose(s: impl Into<SharedString>, p: &Palette) -> Div {
    div()
        .text_size(text::PROSE)
        .line_height(relative(theme::leading::BODY))
        .text_color(p.text_muted)
        .child(s.into())
}

/// `.mx-label`.
pub fn label(s: impl Into<SharedString>, p: &Palette) -> Div {
    div()
        .text_size(text::BASE)
        .font_weight(theme::W_MEDIUM)
        .text_color(p.text_subtle)
        .child(s.into())
}

/// `.mx-label.num`: mono.
pub fn mono_label(s: impl Into<SharedString>, p: &Palette) -> Div {
    label(s, p)
        .font_family(FONT_MONO)
        .text_size(text::SM)
        .tracking(-0.01)
}

/// `.mx-h2`.
pub fn h2(s: impl Into<SharedString>, p: &Palette) -> Div {
    div()
        .text_size(text::XL)
        .font_weight(theme::W_SEMIBOLD)
        .tracking(-0.018)
        .text_color(p.text_strong)
        .child(s.into())
}

/// A stat value: numbers in 28 px mono, unit letters small (`"30h 47m"` → 30 h 47 m).
pub fn stat_value(v: &str, p: &Palette) -> Div {
    stat_value_at(v, px(28.), p)
}

/// `stat_value` with the numbers at `size` (the subject page's narrow 24 px).
pub fn stat_value_at(v: &str, size: Pixels, p: &Palette) -> Div {
    let mut num = String::new();
    let mut unit = String::new();
    let mut parts: Vec<(String, bool)> = Vec::new();
    for ch in v.chars() {
        if ch.is_ascii_digit() || ch == '—' || ch == '.' {
            if !unit.is_empty() {
                parts.push((std::mem::take(&mut unit), false));
            }
            num.push(ch);
        } else if ch == ' ' {
            if !num.is_empty() {
                parts.push((std::mem::take(&mut num), true));
            }
            if !unit.is_empty() {
                parts.push((std::mem::take(&mut unit), false));
            }
        } else {
            if !num.is_empty() {
                parts.push((std::mem::take(&mut num), true));
            }
            unit.push(ch);
        }
    }
    if !num.is_empty() {
        parts.push((num, true));
    }
    if !unit.is_empty() {
        parts.push((unit, false));
    }
    let only_text = parts.iter().all(|(_, n)| !n);
    // `.mx-stat__value` 28/1 mono with `small` units (13/1 sans) inline on its baseline.
    let items = parts.into_iter().enumerate().map(|(i, (s, is_num))| {
        if is_num || only_text {
            (
                Line::new(FONT_MONO, size, theme::leading::ONE),
                div()
                    .when(i > 0 && is_num, |d| d.ml(px(8.)))
                    .tracking(-0.05)
                    .child(s)
                    .into_any_element(),
            )
        } else {
            (
                Line::new(FONT_UI, text::MD, theme::leading::ONE),
                div()
                    .ml(px(3.))
                    .font_weight(theme::W_MEDIUM)
                    .text_color(p.text_subtle)
                    .child(s)
                    .into_any_element(),
            )
        }
    });
    baseline::row(items.collect()).text_color(p.text_strong)
}

/// `.mx-stat`: mono caps label, value, delta (green when `up`).
pub fn stat(
    label: &str,
    value: impl IntoElement,
    delta: Option<String>,
    up: bool,
    p: &Palette,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(7.))
        .min_w(px(132.))
        .child(
            div()
                .font_family(FONT_MONO)
                .text_size(text::XS)
                .font_weight(theme::W_MEDIUM)
                .tracking(0.1)
                .text_color(p.text_subtle)
                .child(label.to_uppercase()),
        )
        .child(value)
        .children(delta.map(|d| {
            div()
                .font_family(FONT_MONO)
                .text_size(text::SM)
                .font_weight(theme::W_MEDIUM)
                .text_color(if up { p.focus_deep } else { p.text_muted })
                .child(d)
        }))
}

/// A stat whose value is plain text.
pub fn stat_text(label: &str, value: &str, delta: Option<String>, p: &Palette) -> Div {
    stat(label, stat_value(value, p), delta, false, p)
}

/// The top-category value: swatch + 22 px name.
pub fn top_value(name: &str, c: Hsla, p: &Palette) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(9.))
        .text_size(px(22.))
        .line_height(relative(1.))
        .font_weight(theme::W_SEMIBOLD)
        .tracking(-0.02)
        .text_color(p.text_strong)
        .child(div().size(px(10.)).rounded(px(2.)).bg(c))
        .child(name.to_string())
}

/// `.in-read`: the lede on the left, the stats 2×2 on the right, hairline below.
/// `width` is what the Svelte rule queries (the window): at ≤1000 px the stats
/// drop under the lede four across, at ≤760 px two across.
pub fn read(lede: impl IntoElement, stats: Vec<AnyElement>, width: Pixels, p: &Palette) -> Div {
    read_at(
        lede,
        stats,
        width,
        (breakpoint::READ_STACK, breakpoint::READ_NARROW),
        p,
    )
}

/// `read` with its own `(stack, two_across)` breakpoints: the subject page's
/// `@container sd` 760 / 560, measured on its column.
pub fn read_at(
    lede: impl IntoElement,
    stats: Vec<AnyElement>,
    width: Pixels,
    (stack, narrow): (Pixels, Pixels),
    p: &Palette,
) -> Div {
    let base = div()
        .pt(space::S4)
        .pb(space::S6)
        .mb(space::S6)
        .border_b_1()
        .border_color(p.hairline);
    let lede = div().flex_1().min_w_0().flex().flex_col().child(lede);
    if width > stack {
        // `repeat(2, minmax(132px, auto))` (each stat has the 132 floor): the
        // rows share column widths, so the second column lines up.
        let grid = div()
            .grid()
            .grid_cols_max_content(2)
            .gap_x(space::S6)
            .gap_y(space::S5)
            .pb(px(4.))
            .children(stats);
        return base
            .flex()
            .items_end()
            .gap(space::S7)
            .child(lede)
            .child(grid);
    }
    let per_row = if width > narrow { 4 } else { 2 };
    let mut grid = div().flex().flex_col().gap(space::S4).pb(px(4.));
    let mut it = stats.into_iter().peekable();
    while it.peek().is_some() {
        let mut row = div().flex().gap(space::S4);
        for _ in 0..per_row {
            // Equal cells (`repeat(n, minmax(0, 1fr))`), an empty one keeps a short row aligned.
            row = row.child(div().flex_1().min_w_0().children(it.next()));
        }
        grid = grid.child(row);
    }
    base.flex()
        .flex_col()
        .gap(space::S5)
        .child(lede)
        .child(grid)
}

/// `.ex` / `.feed` / `.in-feed` / `.in-ex`: `main` (1.45fr) beside `side` (1fr),
/// gap S6; one column when the window is ≤1180 px wide.
pub fn split(main: Div, side: Div, width: Pixels) -> Div {
    if width <= breakpoint::INSIGHTS_ONE_COL {
        return div()
            .flex()
            .flex_col()
            .gap(space::S6)
            .child(main.w_full())
            .child(side.w_full());
    }
    div()
        .flex()
        .gap(space::S6)
        .child(main.flex_grow(1.45).flex_basis(px(0.)).min_w_0())
        .child(side.flex_grow(1.).flex_basis(px(0.)).min_w_0())
}

/// The lede column: kicker, then whatever follows.
pub fn lede(kicker_text: &str, p: &Palette) -> Div {
    div().flex().flex_col().child(kicker(kicker_text, p))
}

pub use crate::ui::baseline::{lift, PHI};
/// The section-head voices as `baseline::Line`s: the size each helper sets, the
/// line height it inherits.
pub const KICKER: Line = Line {
    family: FONT_MONO,
    size: text::XS,
    lh: PHI,
};
pub const LABEL: Line = Line {
    family: FONT_UI,
    size: text::BASE,
    lh: PHI,
};
pub const MONO_LABEL: Line = Line {
    family: FONT_MONO,
    size: text::SM,
    lh: PHI,
};
pub const H2: Line = Line {
    family: FONT_UI,
    size: text::XL,
    lh: PHI,
};

/// `.sec` / `.in-sec` when every item is one line of text: they share a baseline
/// (`align-items: baseline`). `.justify_between()` stands in for the spacer.
pub fn sec_line(items: Vec<(Line, AnyElement)>) -> Div {
    baseline::row(items).gap(space::S3).mb(space::S3)
}

/// `.sec`: a kicker (or h2) head with the right side pushed out.
pub fn sec(left: impl IntoElement, right: Option<AnyElement>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(space::S3)
        .mb(space::S3)
        .child(left)
        .child(div().flex_1())
        .children(right)
}

/// `.mx-panel` (surface, border, radius lg).
pub fn panel(p: &Palette) -> Div {
    div()
        .bg(p.surface)
        .border_1()
        .border_color(p.border)
        .rounded(radius::LG)
}

/// A `.mx-panel--inset` (recessed well).
pub fn inset(p: &Palette) -> Div {
    div()
        .bg(p.surface_subtle)
        .border_1()
        .border_color(p.border)
        .rounded(radius::LG)
}

/// `.group` / `.in-group__k`: mono caps line with a trailing hairline. The
/// 0.1em tracking is the line's, so `right` (pager, links) inherits it as in CSS.
pub fn group_head(s: &str, right: Option<AnyElement>, p: &Palette) -> Div {
    div()
        .tracking(0.1)
        .flex()
        .items_center()
        .gap(space::S2)
        .mt(space::S4)
        .mb(space::S1)
        .child(
            div()
                .flex_none()
                .font_family(FONT_MONO)
                .text_size(text::XS)
                .font_weight(theme::W_MEDIUM)
                .text_color(p.text_subtle)
                .child(s.to_uppercase()),
        )
        .child(div().flex_1().h(px(1.)).bg(p.hairline))
        .children(right)
}

/// `retentionVerdict`: whether missing footage in `[start, end)` is
/// retention's doing: `Some((removed, days))` ("removed" = the span ends before
/// the cutoff; else it straddles it). shortcut: a guess from today's policy,
/// as Svelte's; the upgrade is a per-segment "deleted by retention" record.
pub fn retention(
    start: i64,
    end: i64,
    policy: Option<&capture_types::RetentionPolicy>,
    now: i64,
) -> Option<(bool, i64)> {
    use capture_types::RetentionPolicy as R;
    let days = match policy? {
        R::Never => return None,
        R::Days7 => 7,
        R::Days14 => 14,
        R::Days30 => 30,
    };
    let cutoff = now - days * super::stats::DAY_MS;
    if end <= cutoff {
        Some((true, days))
    } else if start < cutoff {
        Some((false, days))
    } else {
        None
    }
}

/// `mx-inline` with the small spinner: "Writing the read…".
pub fn busy_line(s: impl Into<SharedString>, p: &Palette) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(text::BASE)
        .text_color(p.text_muted)
        .child(crate::ui::progress::spinner(crate::theme::size::SPIN_SM))
        .child(s.into())
}

/// The read's placeholder while it is written (two title bars, two lines).
pub fn writing_skel(p: &Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .pt(crate::theme::space::S4)
        .child(
            crate::ui::skeleton::skel(p)
                .h(px(34.))
                .w(gpui_kit::relative(0.78)),
        )
        .child(
            crate::ui::skeleton::skel(p)
                .h(px(34.))
                .w(gpui_kit::relative(0.52)),
        )
        .child(crate::ui::skeleton::lines(2, p))
}
