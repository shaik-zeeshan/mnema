//! `.mx-seg` + its sliding thumb (Segmented.svelte): sans labels, `mono()` for
//! data-valued options (1m · 5m), sm 26 / md 32. The thumb slides 420 ms expo.

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::AnimationExt as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, Animation, App, ElementId, Pixels, SharedString, Window};

use super::{
    focus_handle, focus_visible, icon, inset, last_change, nid, ring_box, text_width, with_tip,
    Force,
};
use crate::theme::{self, motion, radius, size, text, FONT_MONO, FONT_UI};

/// Padding of the track around the buttons (thumb inset).
const INSET: Pixels = px(2.);
const ICON: Pixels = px(14.);
const ICON_GAP: Pixels = px(6.);

#[derive(Clone)]
pub struct SegOption {
    pub label: SharedString,
    pub icon: Option<SharedString>,
    /// A mono count after the label (the developer window's job filter).
    pub count: Option<SharedString>,
    /// Hover text when it should say more than the label (the default).
    pub tip: Option<SharedString>,
}

impl SegOption {
    pub fn tip(mut self, t: impl Into<SharedString>) -> Self {
        self.tip = Some(t.into());
        self
    }
}

impl<S: Into<SharedString>> From<S> for SegOption {
    fn from(label: S) -> Self {
        Self {
            label: label.into(),
            icon: None,
            count: None,
            tip: None,
        }
    }
}

pub fn opt_icon(label: impl Into<SharedString>, icon: impl Into<SharedString>) -> SegOption {
    SegOption {
        label: label.into(),
        icon: Some(icon.into()),
        count: None,
        tip: None,
    }
}

pub fn opt_count(label: impl Into<SharedString>, count: impl Into<SharedString>) -> SegOption {
    SegOption {
        label: label.into(),
        icon: None,
        count: Some(count.into()),
        tip: None,
    }
}

type OnChange = Rc<dyn Fn(usize, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct Segmented {
    id: ElementId,
    aria: SharedString,
    options: Vec<SegOption>,
    selected: usize,
    md: bool,
    mono: bool,
    fill: bool,
    disabled: bool,
    /// Options that can't be picked (`disabledValues`).
    off: Vec<usize>,
    force: Option<(usize, Force)>,
    on_change: Option<OnChange>,
}

impl Segmented {
    pub fn new(
        id: impl Into<ElementId>,
        aria: impl Into<SharedString>,
        options: impl IntoIterator<Item = impl Into<SegOption>>,
        selected: usize,
    ) -> Self {
        Self {
            id: id.into(),
            aria: aria.into(),
            options: options.into_iter().map(Into::into).collect(),
            selected,
            md: false,
            mono: false,
            fill: false,
            disabled: false,
            off: Vec::new(),
            force: None,
            on_change: None,
        }
    }
    pub fn md(mut self) -> Self {
        self.md = true;
        self
    }
    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }
    /// Fill the parent's width with equal options (`.dv-tabs .mx-seg button { flex: 1 }`).
    pub fn fill(mut self) -> Self {
        self.fill = true;
        self
    }
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
    /// Grey out and refuse these options (`disabledValues`).
    pub fn disabled_options(mut self, ix: impl IntoIterator<Item = usize>) -> Self {
        self.off = ix.into_iter().collect();
        self
    }
    /// Pin option `ix` hovered / focused (gallery, goldens).
    pub fn force(mut self, ix: usize, f: Force) -> Self {
        self.force = Some((ix, f));
        self
    }
    pub fn on_change(mut self, f: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }
}

/// Thumb `(left, width)` for option `ix` given each option's width.
pub fn thumb_at(widths: &[Pixels], ix: usize) -> (Pixels, Pixels) {
    let ix = ix.min(widths.len().saturating_sub(1));
    let left = widths[..ix].iter().fold(INSET, |a, w| a + *w);
    (left, widths.get(ix).copied().unwrap_or_default())
}

impl RenderOnce for Segmented {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = theme::tokens(cx);
        let h = if self.md { size::H_MD } else { size::H_SM };
        let (pad, fs, family) = match (self.md, self.mono) {
            (_, true) => (if self.md { px(14.) } else { px(10.) }, text::SM, FONT_MONO),
            (true, false) => (px(14.), text::MD, FONT_UI),
            (false, false) => (px(10.), text::BASE, FONT_UI),
        };
        let widths: Vec<Pixels> = self
            .options
            .iter()
            .map(|o| {
                let icon_w = if o.icon.is_some() {
                    ICON + ICON_GAP
                } else {
                    px(0.)
                };
                let count_w = o.count.as_ref().map_or(px(0.), |c| {
                    ICON_GAP + text_width(c, FONT_MONO, theme::W_REGULAR, text::XS, 0., window)
                });
                pad * 2.
                    + icon_w
                    + count_w
                    + text_width(&o.label, family, theme::W_MEDIUM, fs, 0., window)
            })
            .collect();

        // The thumb slides from the previous selection to this one.
        let base = self.id.to_string();
        let (prev, gen) = last_change(nid(&format!("{base}-sel"), 0), self.selected, window, cx);
        let (from, to) = (thumb_at(&widths, prev), thumb_at(&widths, self.selected));
        let thumb = div()
            .id("seg-thumb")
            .test_support()
            .absolute()
            .top(INSET)
            .bottom(INSET)
            .left(to.0)
            .w(to.1)
            .rounded(radius::SM)
            .bg(p.surface_raised)
            .child(ring_box(p.border_strong, px(1.), radius::SM));
        let n = self.options.len().max(1) as f32;
        let (from_f, to_f) = (
            prev.min(n as usize - 1) as f32 / n,
            self.selected as f32 / n,
        );
        let thumb = if self.fill {
            // Equal options: the thumb is a fraction of the inset track.
            let t = div()
                .id("seg-thumb")
                .test_support()
                .absolute()
                .top_0()
                .bottom_0()
                .left(gpui_kit::relative(to_f))
                .w(gpui_kit::relative(1. / n))
                .rounded(radius::SM)
                .bg(p.surface_raised)
                .child(ring_box(p.border_strong, px(1.), radius::SM));
            let t = if gen == 0 {
                t.into_any_element()
            } else {
                t.with_animation(
                    nid(&format!("{base}-thumb"), gen),
                    Animation::new(motion::SLOW).with_easing(motion::expo),
                    move |el, k| el.left(gpui_kit::relative(from_f + (to_f - from_f) * k)),
                )
                .into_any_element()
            };
            div()
                .absolute()
                .top(INSET)
                .bottom(INSET)
                .left(INSET)
                .right(INSET)
                .child(t)
                .into_any_element()
        } else if gen == 0 {
            thumb.into_any_element()
        } else {
            thumb
                .with_animation(
                    nid(&format!("{base}-thumb"), gen),
                    Animation::new(motion::SLOW).with_easing(motion::expo),
                    move |el, t| {
                        el.left(from.0 + (to.0 - from.0) * t)
                            .w(from.1 + (to.1 - from.1) * t)
                    },
                )
                .into_any_element()
        };

        let buttons = self.options.iter().enumerate().map(|(ix, o)| {
            let forced = self
                .force
                .filter(|(f, _)| *f == ix)
                .map_or(Force::None, |(_, f)| f);
            let handle = focus_handle(nid(&format!("{base}-opt"), ix as u64), window, cx);
            let on = ix == self.selected;
            let fg = if on || forced == Force::Hover {
                p.text_strong
            } else {
                p.text_muted
            };
            let strong = p.text_strong;
            let on_change = self.on_change.clone();
            let off = self.disabled || self.off.contains(&ix);
            // Every option tips its label (Segmented.svelte); a dead group shows none.
            let tip = (!self.disabled)
                .then(|| o.tip.clone().unwrap_or_else(|| o.label.clone()))
                .filter(|t| !t.is_empty());
            div()
                .id(nid(&format!("{base}-opt"), ix as u64))
                .test_support()
                .role(gpui_kit::Role::RadioButton)
                .aria_label(o.label.clone())
                .aria_selected(on)
                .when(!off, |el| el.track_focus(&handle))
                .when(off && !self.disabled, |el| {
                    el.opacity(theme::DISABLED_OPACITY)
                })
                .relative()
                .flex()
                .items_center()
                .justify_center()
                .gap(ICON_GAP)
                .when(self.fill, |el| el.flex_1())
                .when(!self.fill, |el| el.flex_none().w(widths[ix]))
                .h_full()
                .rounded(radius::SM)
                .text_color(fg)
                .font_family(family)
                .text_size(fs)
                .font_weight(theme::W_MEDIUM)
                .line_height(gpui_kit::relative(1.))
                .whitespace_nowrap()
                .when(focus_visible(&handle, forced, window), |el| {
                    el.child(ring_box(p.accent_glow, size::FOCUS_RING, radius::SM))
                })
                .when(!off, |el| {
                    el.cursor_pointer().hover(move |s| s.text_color(strong))
                })
                .when_some(o.icon.clone(), |el, path| el.child(icon(path, ICON)))
                .child(o.label.clone())
                .when_some(o.count.clone(), |el, c| {
                    el.child(
                        div()
                            .font_family(FONT_MONO)
                            .font_weight(theme::W_REGULAR)
                            .text_size(text::XS)
                            .text_color(p.text_subtle)
                            .child(c),
                    )
                })
                .when_some(on_change.filter(|_| !off), |el, f| {
                    el.on_click(move |_, window, cx| f(ix, window, cx))
                })
                .when_some(tip, |el, t| with_tip(el, t))
        });

        let hug = widths.iter().fold(INSET * 2., |a, w| a + *w);
        div()
            .id(self.id.clone())
            .test_support()
            .role(gpui_kit::Role::RadioGroup)
            .aria_label(self.aria.clone())
            .relative()
            .flex()
            .when(self.fill, |el| el.w_full())
            // `width: fit-content`: hug the options even in a stretching column.
            .when(!self.fill, |el| el.flex_none().w(hug))
            .h(h)
            .p(INSET)
            .bg(p.wash)
            .rounded(radius::MD)
            .shadow(vec![inset(p.hairline, px(1.))])
            .when(self.disabled, |el| el.opacity(theme::DISABLED_OPACITY))
            .child(thumb)
            .children(buttons)
    }
}
