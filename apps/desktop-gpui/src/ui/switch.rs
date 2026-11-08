//! `.mx-switch`: 30×18 pill, 14 px knob; on = accent. The knob slides 220 ms
//! expo. Light mode's off knob is the surface with a 0.5 px border ring.

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::AnimationExt as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, Animation, App, ElementId, SharedString, Window};

use super::{focus_handle, focus_visible, last_change, mix, nid, outline, spread, Force};
use crate::theme::{self, motion, radius, size, Mode};

type OnToggle = Rc<dyn Fn(bool, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct Switch {
    id: ElementId,
    aria: SharedString,
    on: bool,
    disabled: bool,
    force: Force,
    on_toggle: Option<OnToggle>,
}

impl Switch {
    pub fn new(id: impl Into<ElementId>, aria: impl Into<SharedString>, on: bool) -> Self {
        Self {
            id: id.into(),
            aria: aria.into(),
            on,
            disabled: false,
            force: Force::None,
            on_toggle: None,
        }
    }
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
    pub fn force(mut self, f: Force) -> Self {
        self.force = f;
        self
    }
    pub fn on_toggle(mut self, f: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Rc::new(f));
        self
    }
}

/// Knob travel: 30 - 14 - 2·2.
const TRAVEL: f32 = 12.;
const PAD: f32 = 2.;

impl RenderOnce for Switch {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = theme::tokens(cx);
        let light = theme::mode(cx) == Mode::Light;
        let handle = focus_handle(self.id.clone(), window, cx);
        let ring = focus_visible(&handle, self.force, window);
        let base = self.id.to_string();
        let (prev, gen) = last_change(nid(&format!("{base}-on"), 0), self.on, window, cx);
        let knob_off = if light { p.surface } else { p.fg };
        let knob = |on: bool| if on { p.accent_contrast } else { knob_off };
        let track = |on: bool| if on { p.accent } else { p.border_strong };
        let x = |on: bool| if on { TRAVEL } else { 0. };

        let knob_el = div()
            .absolute()
            .top(px(PAD))
            .left(px(PAD + x(self.on)))
            .size(size::SWITCH_KNOB)
            .rounded_full()
            .bg(knob(self.on))
            .when(light && !self.on, |el| {
                el.shadow(vec![spread(p.border_strong, px(0.5))])
            });
        let knob_el = if gen == 0 || prev == self.on {
            knob_el.into_any_element()
        } else {
            let (from, to) = (x(prev), x(self.on));
            let (c0, c1) = (knob(prev), knob(self.on));
            knob_el
                .with_animation(
                    nid(&format!("{base}-knob"), gen),
                    Animation::new(motion::MED).with_easing(motion::expo),
                    move |el, t| el.left(px(PAD + from + (to - from) * t)).bg(mix(c1, c0, t)),
                )
                .into_any_element()
        };
        let on = self.on;
        let toggle = self.on_toggle.clone().filter(|_| !self.disabled);
        div()
            .id(self.id.clone())
            .test_support()
            .role(gpui_kit::Role::Switch)
            .aria_label(self.aria.clone())
            .aria_toggled(if on {
                gpui_kit::Toggled::True
            } else {
                gpui_kit::Toggled::False
            })
            .when(!self.disabled, |el| {
                el.track_focus(&handle).cursor_pointer()
            })
            .relative()
            .flex_none()
            .w(size::SWITCH_W)
            .h(size::SWITCH_H)
            .rounded(radius::PILL)
            .bg(track(on))
            .when(self.disabled, |el| el.opacity(theme::DISABLED_OPACITY))
            .child(knob_el)
            .when(ring, |el| el.child(outline(p.accent, size::SWITCH_H / 2.)))
            .when_some(toggle, |el, f| {
                el.on_click(move |_, window, cx| f(!on, window, cx))
            })
    }
}
