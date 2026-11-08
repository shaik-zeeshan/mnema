//! `.mx-seg` + its sliding thumb (Segmented.svelte): sans labels, `mono()` for
//! data-valued options (1m · 5m), sm 26 / md 32. The thumb slides 420 ms expo.

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::AnimationExt as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, Animation, App, ElementId, Pixels, SharedString, Window};

use super::{
    focus_handle, focus_visible, icon, inset, last_change, nid, ring_box, spread, text_width, Force,
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
}

impl<S: Into<SharedString>> From<S> for SegOption {
    fn from(label: S) -> Self {
        Self {
            label: label.into(),
            icon: None,
        }
    }
}

pub fn opt_icon(label: impl Into<SharedString>, icon: impl Into<SharedString>) -> SegOption {
    SegOption {
        label: label.into(),
        icon: Some(icon.into()),
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
    disabled: bool,
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
            disabled: false,
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
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
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
                pad * 2. + icon_w + text_width(&o.label, family, theme::W_MEDIUM, fs, window)
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
            .shadow(vec![spread(p.border_strong, px(1.))]);
        let thumb = if gen == 0 {
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
            div()
                .id(nid(&format!("{base}-opt"), ix as u64))
                .test_support()
                .role(gpui_kit::Role::RadioButton)
                .aria_label(o.label.clone())
                .aria_selected(on)
                .when(!self.disabled, |el| el.track_focus(&handle))
                .relative()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .gap(ICON_GAP)
                .w(widths[ix])
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
                .when(!self.disabled, |el| {
                    el.cursor_pointer().hover(move |s| s.text_color(strong))
                })
                .when_some(o.icon.clone(), |el, path| el.child(icon(path, ICON)))
                .child(o.label.clone())
                .when_some(on_change.filter(|_| !self.disabled), |el, f| {
                    el.on_click(move |_, window, cx| f(ix, window, cx))
                })
        });

        div()
            .id(self.id.clone())
            .test_support()
            .role(gpui_kit::Role::RadioGroup)
            .aria_label(self.aria.clone())
            .relative()
            .flex()
            .flex_none()
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
