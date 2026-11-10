//! `.mx-btn`: ONE button system. Flat, one 1px border (none on primary / ghost).
//! Hover = fill shift, press = 1 px down, focus = 2 px outline 2 px off.
//! States: pressed (`aria-pressed`), disabled, soft-disabled (`aria-disabled`:
//! keeps its tooltip, blocks the click), busy (`aria-busy`, 0.6).

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{
    div, px, App, ClickEvent, ElementId, FocusHandle, Hsla, Pixels, SharedString, Window,
};

use super::{focus_handle, focus_visible, icon, mix, outline, progress, tooltip, Force};
use crate::theme::{self, size, text, Palette};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Variant {
    /// Surface + 1px border (the default).
    #[default]
    Secondary,
    /// The ONLY green fill: one per view.
    Primary,
    /// Text only; wash on hover. Toolbars, inline actions.
    Ghost,
    /// Neutral surface, danger ink; tints on hover.
    Danger,
    /// Text only in danger ink (`.mx-btn--ghost.mx-btn--danger`).
    GhostDanger,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonSize {
    /// 32 px.
    #[default]
    Md,
    /// 26 px.
    Sm,
}

pub type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: Option<SharedString>,
    aria: Option<SharedString>,
    icon: Option<SharedString>,
    icon_after: bool,
    accent_icon: bool,
    kbd: Option<SharedString>,
    variant: Variant,
    size: ButtonSize,
    height: Option<Pixels>,
    icon_only: bool,
    pressed: bool,
    disabled: bool,
    soft_disabled: bool,
    busy: bool,
    spinner: bool,
    force: Force,
    tip: Option<SharedString>,
    block: bool,
    focus: Option<FocusHandle>,
    on_click: Option<ClickHandler>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            label: Some(label.into()),
            ..Self::icon_only(id, "", "")
        }
    }

    /// Square icon-only button (`mx-btn--icon`); `aria` names it.
    pub fn icon_only(
        id: impl Into<ElementId>,
        icon: impl Into<SharedString>,
        aria: impl Into<SharedString>,
    ) -> Self {
        let icon: SharedString = icon.into();
        Self {
            id: id.into(),
            label: None,
            aria: Some(aria.into()),
            icon_only: !icon.is_empty(),
            icon: (!icon.is_empty()).then_some(icon),
            icon_after: false,
            accent_icon: false,
            kbd: None,
            variant: Variant::Secondary,
            size: ButtonSize::Md,
            height: None,
            pressed: false,
            disabled: false,
            soft_disabled: false,
            busy: false,
            spinner: false,
            force: Force::None,
            tip: None,
            block: false,
            focus: None,
            on_click: None,
        }
    }

    /// Full width, content at the start, the kbd pushed right (`.ch-side__tools .mx-btn`).
    pub fn block(mut self) -> Self {
        self.block = true;
        self
    }

    pub fn variant(mut self, v: Variant) -> Self {
        self.variant = v;
        self
    }
    pub fn primary(self) -> Self {
        self.variant(Variant::Primary)
    }
    pub fn ghost(self) -> Self {
        self.variant(Variant::Ghost)
    }
    pub fn danger(self) -> Self {
        self.variant(Variant::Danger)
    }
    pub fn ghost_danger(self) -> Self {
        self.variant(Variant::GhostDanger)
    }
    pub fn sm(mut self) -> Self {
        self.size = ButtonSize::Sm;
        self
    }
    /// Override the height (status bar's 22 px buttons: `--_h: 22px`).
    pub fn h(mut self, h: Pixels) -> Self {
        self.height = Some(h);
        self
    }
    /// Leading icon (asset path). `.icon_after()` moves it behind the label.
    pub fn icon(mut self, path: impl Into<SharedString>) -> Self {
        self.icon = Some(path.into());
        self
    }
    pub fn icon_after(mut self) -> Self {
        self.icon_after = true;
        self
    }
    /// The sparkle-accent icon of an "Ask about …" action.
    pub fn accent_icon(mut self) -> Self {
        self.accent_icon = true;
        self
    }
    /// Quiet mono shortcut hint after the label (no box).
    pub fn kbd(mut self, k: impl Into<SharedString>) -> Self {
        self.kbd = Some(k.into());
        self
    }
    pub fn pressed(mut self, on: bool) -> Self {
        self.pressed = on;
        self
    }
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
    /// `aria-disabled`: dimmed, click blocked, tooltip still shows the why.
    pub fn soft_disabled(mut self, on: bool) -> Self {
        self.soft_disabled = on;
        self
    }
    pub fn busy(mut self, on: bool) -> Self {
        self.busy = on;
        self
    }
    /// A small spinner before the label (`Installing…`).
    pub fn spinner(mut self, on: bool) -> Self {
        self.spinner = on;
        self
    }
    pub fn force(mut self, f: Force) -> Self {
        self.force = f;
        self
    }
    /// Use the owner's focus handle (a view that moves focus here itself).
    pub fn focus_handle(mut self, h: &FocusHandle) -> Self {
        self.focus = Some(h.clone());
        self
    }
    pub fn tip(mut self, t: impl Into<SharedString>) -> Self {
        self.tip = Some(t.into());
        self
    }
    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

/// `(bg, border, fg, hover bg, hover border, hover fg, press bg, ring)`.
struct Skin {
    bg: Hsla,
    bd: Hsla,
    fg: Hsla,
    hover: (Hsla, Hsla, Hsla),
    press: Hsla,
    ring: Hsla,
}

fn skin(v: Variant, pressed: bool, p: &Palette) -> Skin {
    let none = gpui_kit::transparent_black();
    let mut s = match v {
        Variant::Secondary => Skin {
            bg: p.surface_raised,
            bd: p.border_strong,
            fg: p.text_strong,
            hover: (p.surface_hover, p.border_strong, p.text_strong),
            press: p.surface_hover,
            ring: p.accent,
        },
        Variant::Primary => Skin {
            bg: p.accent,
            bd: none,
            fg: p.accent_contrast,
            hover: (mix(p.accent, p.text_strong, 0.88), none, p.accent_contrast),
            press: mix(p.accent, p.text_strong, 0.80),
            ring: p.accent,
        },
        Variant::Ghost => Skin {
            bg: none,
            bd: none,
            fg: p.text_muted,
            hover: (p.wash_strong, none, p.text_strong),
            press: p.wash_strong,
            ring: p.accent,
        },
        Variant::GhostDanger => Skin {
            bg: none,
            bd: none,
            fg: p.danger_text,
            hover: (p.danger_bg, none, p.danger_text),
            press: p.danger_bg,
            ring: p.danger,
        },
        Variant::Danger => Skin {
            bg: p.surface_raised,
            bd: p.border_strong,
            fg: p.danger_text,
            hover: (p.danger_bg, p.danger_border, p.danger_text),
            press: p.danger_bg,
            ring: p.danger,
        },
    };
    if pressed {
        s.bg = p.wash_strong;
        if v == Variant::Ghost {
            s.fg = p.text_strong;
        }
    }
    s
}

impl RenderOnce for Button {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = theme::tokens(cx);
        let sm = self.size == ButtonSize::Sm;
        let h = self
            .height
            .unwrap_or(if sm { size::H_SM } else { size::H_MD });
        let (pad, fs, r, gap) = if sm {
            (px(9.), text::BASE, px(5.), px(6.))
        } else {
            (px(12.), text::MD, px(6.), px(7.))
        };
        let icon_px = if sm {
            size::BTN_ICON_SM
        } else {
            size::BTN_ICON_MD
        };
        let s = skin(self.variant, self.pressed, &p);
        let blocked = self.disabled || self.soft_disabled || self.busy;
        let live = !blocked;
        let handle = match &self.focus {
            Some(h) => h.clone(),
            None => focus_handle(self.id.clone(), window, cx),
        };
        let ring = focus_visible(&handle, self.force, window);
        let opacity = if self.disabled || self.soft_disabled {
            theme::DISABLED_OPACITY
        } else if self.busy {
            theme::BUSY_OPACITY
        } else {
            1.
        };
        // CSS fades the whole button as one layer; GPUI's `.opacity()` fades the
        // fill and the label separately, so the label would show the faded fill
        // through it. Fade the fill/border by alpha and pre-composite the ink over
        // the surface the button usually sits on (raised), which is what CSS paints.
        // A fill-less button (ghost) has nothing under its ink: alpha is exact there.
        let (bg, bd, fg) = match self.force {
            Force::Hover if live => s.hover,
            Force::Press if live => (s.press, s.hover.1, s.hover.2),
            _ => (s.bg, s.bd, s.fg),
        };
        let under = p.surface_raised;
        let ink = |c: Hsla| {
            if opacity >= 1. {
                c
            } else if bg.a == 0. {
                super::alpha(c, opacity)
            } else {
                mix(c, under, opacity)
            }
        };
        let (bg, bd, fg) = (
            super::alpha(bg, opacity),
            super::alpha(bd, opacity),
            ink(fg),
        );
        let kbd_fg = ink(if self.variant == Variant::Primary {
            super::alpha(p.accent_contrast, 0.7)
        } else {
            p.text_subtle
        });
        let icon_el = self.icon.clone().map(|path| {
            div()
                .flex_none()
                .when(self.accent_icon, |d| d.text_color(ink(p.accent)))
                .child(icon(path, icon_px))
        });
        let (before, after) = if self.icon_after {
            (None, icon_el)
        } else {
            (icon_el, None)
        };
        let on_click = self.on_click.clone();
        let hover = s.hover;
        let press = s.press;
        let tip = self.tip.clone();
        let el = div()
            .id(self.id.clone())
            .test_support()
            .role(gpui_kit::Role::Button)
            .when_some(self.aria.clone().filter(|a| !a.is_empty()), |el, a| {
                el.aria_label(a)
            })
            .when(!self.disabled, |el| el.track_focus(&handle))
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .when(self.block, |el| el.w_full().justify_start())
            .when(!self.block, |el| el.justify_center())
            .gap(gap)
            .h(h)
            .when(self.icon_only, |el| el.w(h))
            .when(!self.icon_only, |el| el.px(pad))
            .rounded(r)
            .bg(bg)
            .border_1()
            .border_color(bd)
            .text_color(fg)
            .font_family(theme::FONT_UI)
            .text_size(fs)
            .font_weight(theme::W_MEDIUM)
            .line_height(gpui_kit::relative(1.))
            .whitespace_nowrap()
            .when(self.force == Force::Press && live, |el| el.top(px(1.)))
            .when(live, |el| {
                el.cursor_pointer()
                    .hover(move |st| st.bg(hover.0).border_color(hover.1).text_color(hover.2))
                    .active(move |st| st.bg(press).top(px(1.)))
            })
            .when(self.spinner, |el| {
                el.child(progress::spinner(size::SPIN_SM))
            })
            .children(before)
            .children(self.label.clone())
            .children(after)
            .when_some(self.kbd.clone(), |el, k| {
                el.child(
                    div()
                        .ml(px(2.))
                        .when(self.block, |d| d.ml_auto())
                        .font_family(theme::FONT_MONO)
                        .text_size(text::XS)
                        .text_color(kbd_fg)
                        .child(k),
                )
            })
            .when(ring, |el| {
                el.child(outline(super::alpha(s.ring, opacity), r))
            })
            .when(ring && tip.is_some(), |el| {
                el.child(tooltip::focus_tip(tip.clone().unwrap_or_default(), &p))
            })
            .when_some(on_click.filter(|_| live), |el, f| {
                el.on_click(move |e, window, cx| f(e, window, cx))
            });
        match self.tip {
            Some(t) if !self.disabled => tooltip::with_tip(el, t).into_any_element(),
            _ => el.into_any_element(),
        }
    }
}
