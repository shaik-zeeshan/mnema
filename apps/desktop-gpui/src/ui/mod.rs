//! The shared control set (kit.css `.mx-*` + `lib/components/*.svelte`), one file
//! per control, every value from `theme` tokens. API summary: the `gpui-mnema`
//! skill's "Shared controls" section. Controls are `RenderOnce` builders: the
//! caller owns the value and passes an `on_change`; open / animation / focus
//! bookkeeping lives in keyed element state.

pub mod button;
pub mod calendar;
pub mod chip;
pub mod choice;
pub mod clock;
pub mod combobox;
pub mod date_input;
pub mod dialog;
pub mod empty;
pub mod input;
pub mod kbd;
pub mod menu;
pub mod model_picker;
pub mod notice;
pub mod popover;
pub mod progress;
pub mod segmented;
pub mod select;
pub mod skeleton;
pub mod slider;
pub mod stepper;
pub mod switch;
pub mod toast;
pub mod tooltip;
pub mod window_strip;

#[cfg(test)]
mod controls_tests;

pub use button::{Button, ButtonSize, Variant};
pub use kbd::{kbd, keys};
pub use popover::{pop_in, popover, popover_shadow};
pub use tooltip::{with_tip, Tip};

use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{
    div, px, svg, App, BoxShadow, Div, ElementId, FocusHandle, Hsla, Pixels, Rgba, SharedString,
    Window,
};

use crate::theme::{self, radius, size, space, text, Palette, FONT_MONO};

/// A state the gallery and goldens pin instead of moving a real mouse or focus.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Force {
    #[default]
    None,
    Hover,
    Focus,
    Press,
}

/// The kit kicker: mono 10 uppercase, tracked, green `//`, muted text.
pub fn kicker(label: &str, p: &Palette) -> Div {
    div()
        .flex()
        .items_center()
        .gap(space::S2)
        .font_family(FONT_MONO)
        .text_size(text::XS)
        .font_weight(theme::W_MEDIUM)
        .text_color(p.text_subtle)
        .child(div().text_color(p.accent).child("//"))
        // No letter-spacing in GPUI: the 0.12em tracking is a native gap.
        .child(SharedString::from(label.to_uppercase()))
}

/// The 1×12 vertical separator (`mx-sep`).
pub fn sep(p: &Palette) -> Div {
    div().flex_none().w(px(1.)).h(px(12.)).bg(p.border_strong)
}

/// An icon by asset path at `size`, painted in the inherited text colour (so a
/// parent's hover / current colour reaches it; GPUI's `svg` doesn't inherit).
#[derive(IntoElement)]
pub struct Icon {
    path: SharedString,
    size: Pixels,
}

pub fn icon(path: impl Into<SharedString>, size: Pixels) -> Icon {
    Icon {
        path: path.into(),
        size,
    }
}

impl RenderOnce for Icon {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        svg()
            .path(self.path)
            .flex_none()
            .size(self.size)
            .text_color(window.text_style().color)
    }
}

/// An interactive element the helpers return. Its concrete type changes when
/// gpui-kit's `test-support` wraps it (`.test_support()` -> `Observed<_>`).
pub trait Control: StatefulInteractiveElement + ParentElement + Styled + IntoElement {}
impl<T: StatefulInteractiveElement + ParentElement + Styled + IntoElement> Control for T {}

/// The kit's ghost icon button (`mx-btn--ghost mx-btn--icon`), `h` square.
/// `current` = `aria-current` / pressed: the selected fill. Mouse-down stops here
/// so a titlebar drag never starts from a button.
pub fn icon_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    h: Pixels,
    current: bool,
    p: &Palette,
) -> impl Control {
    let (fg, bg) = if current {
        (p.text_strong, Some(p.wash_strong))
    } else {
        (p.text_muted, None)
    };
    let hover = (p.wash_strong, p.text_strong);
    div()
        .id(id)
        .test_support()
        .role(gpui_kit::Role::Button)
        .aria_label(label)
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(h)
        .rounded(px(5.))
        .text_color(fg)
        .when_some(bg, |el, bg| el.bg(bg))
        .cursor_pointer()
        .hover(move |s| s.bg(hover.0).text_color(hover.1))
        .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
            cx.stop_propagation()
        })
}

/// The 3 px accent-glow focus ring (`--app-ring`), as a box shadow.
pub fn ring(color: Hsla) -> BoxShadow {
    spread(color, size::FOCUS_RING)
}

/// `box-shadow: 0 0 0 <w> <color>` (outset hairline / ring).
pub fn spread(color: Hsla, w: Pixels) -> BoxShadow {
    BoxShadow {
        color,
        offset: gpui_kit::point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: w,
        inset: false,
    }
}

/// `box-shadow: inset 0 0 0 <w> <color>`.
pub fn inset(color: Hsla, w: Pixels) -> BoxShadow {
    BoxShadow {
        inset: true,
        ..spread(color, w)
    }
}

/// A `w` ring hugging the outside of a `relative()` box of radius `r`: the
/// `--app-ring` / `0 0 0 Npx` shadow for boxes with no opaque fill (GPUI paints
/// a shadow under the whole box, so a clear box would fill with it).
pub fn ring_box(color: Hsla, w: Pixels, r: Pixels) -> Div {
    div()
        .absolute()
        .top(-w)
        .left(-w)
        .right(-w)
        .bottom(-w)
        .rounded(r + w)
        .border(w)
        .border_color(color)
}

/// CSS `outline: 2px solid <color>; outline-offset: 2px` around a box of corner
/// radius `r`: an absolute child, so the parent must be `relative()`.
pub fn outline(color: Hsla, r: Pixels) -> Div {
    let off = -(size::OUTLINE + size::OUTLINE_OFFSET);
    div()
        .absolute()
        .top(off)
        .left(off)
        .right(off)
        .bottom(off)
        .rounded(r + size::OUTLINE + size::OUTLINE_OFFSET)
        .border(size::OUTLINE)
        .border_color(color)
}

/// CSS `color-mix(in srgb, a <wa>, b)`: premultiplied, so a translucent side
/// weighs by its alpha exactly as the browser does.
pub fn mix(a: Hsla, b: Hsla, wa: f32) -> Hsla {
    let (a, b) = (Rgba::from(a), Rgba::from(b));
    let wb = 1. - wa;
    let alpha = a.a * wa + b.a * wb;
    if alpha <= 0. {
        return gpui_kit::transparent_black();
    }
    let ch = |x: f32, y: f32| (x * a.a * wa + y * b.a * wb) / alpha;
    Rgba {
        r: ch(a.r, b.r),
        g: ch(a.g, b.g),
        b: ch(a.b, b.b),
        a: alpha,
    }
    .into()
}

/// `color-mix(in srgb, c <w>, transparent)`.
pub fn alpha(c: Hsla, w: f32) -> Hsla {
    Hsla { a: c.a * w, ..c }
}

/// The focus handle a `RenderOnce` control keeps across frames under `id`
/// (a Tab stop, like a DOM button).
pub fn focus_handle(id: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> FocusHandle {
    window
        .use_keyed_state(id, cx, |_, cx| cx.focus_handle().tab_stop(true))
        .read(cx)
        .clone()
}

/// CSS `:focus-visible`: focused, and the last input was the keyboard.
pub fn focus_visible(handle: &FocusHandle, force: Force, window: &Window) -> bool {
    force == Force::Focus || (handle.is_focused(window) && window.last_input_was_keyboard())
}

/// The value before the latest change of `value`, and a generation that bumps
/// on every change (0 = never changed, so nothing animates on first paint).
/// Controls key their transition animation on the generation.
/// shortcut: a change mid-transition restarts from the previous end state rather
/// than the in-flight position; upgrade if a control is ever toggled that fast.
pub fn last_change<T: Copy + PartialEq + 'static>(
    id: impl Into<ElementId>,
    value: T,
    window: &mut Window,
    cx: &mut App,
) -> (T, u64) {
    let track = window.use_keyed_state(id, cx, |_, _| (value, value, 0u64));
    track.update(cx, |(prev, cur, gen), _| {
        if *cur != value {
            *prev = *cur;
            *cur = value;
            *gen += 1;
        }
        (*prev, *gen)
    })
}

/// Element id `<base>-<n>` (animation keys, list rows).
pub fn nid(base: &str, n: u64) -> ElementId {
    ElementId::NamedInteger(SharedString::from(base.to_string()), n)
}

/// The mx tone vocabulary (`[data-tone]`): absent = neutral.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    #[default]
    Neutral,
    Info,
    Ok,
    Warn,
    Danger,
    Off,
}

impl Tone {
    /// `(--tone, --tone-bg, --tone-bd)`.
    pub fn colors(self, p: &Palette) -> (Hsla, Hsla, Hsla) {
        match self {
            Tone::Neutral | Tone::Off => (p.text_muted, p.wash, p.border_strong),
            Tone::Info => (p.info, p.info_bg, p.info_border),
            Tone::Ok => (p.accent, p.accent_bg, p.accent_border),
            Tone::Warn => (p.warn, p.warn_bg, p.warn_border),
            Tone::Danger => (p.danger_text, p.danger_bg, p.danger_border),
        }
    }
}

/// Corner radius used by every `.mx-input`-family box.
pub const FIELD_RADIUS: Pixels = radius::MD;

/// Rendered width of one line of text (controls that size themselves to their
/// labels, like the segmented thumb).
pub fn text_width(
    s: &str,
    family: &'static str,
    weight: gpui_kit::FontWeight,
    font_size: Pixels,
    window: &Window,
) -> Pixels {
    let run = gpui_kit::TextRun {
        len: s.len(),
        font: gpui_kit::Font {
            weight,
            ..gpui_kit::font(family)
        },
        color: gpui_kit::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window
        .text_system()
        .shape_line(s.to_string().into(), font_size, &[run], None);
    px(f32::from(line.width).ceil())
}
