//! `Slider.svelte` on gpui-base's headless slider (drag, keys, a11y stay the
//! kit's): 4 px wash track, accent range, 14 px accent thumb ringed in the
//! surface. Optional label + mono value header.

use gpui_kit::base::slider::SliderState;
use gpui_kit::base::{Slider as BaseSlider, SliderIndicator, SliderThumb, SliderTrack};
use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, App, Entity, SharedString, Window};

use super::{focus_handle, focus_visible, nid, outline, ring_box, Force};
use crate::theme::{self, radius, size, text, FONT_MONO};

#[derive(IntoElement)]
pub struct Slider {
    state: Entity<SliderState>,
    aria: SharedString,
    label: Option<SharedString>,
    value_text: Option<SharedString>,
    disabled: bool,
    force: Force,
}

impl Slider {
    pub fn new(state: &Entity<SliderState>, aria: impl Into<SharedString>) -> Self {
        Self {
            state: state.clone(),
            aria: aria.into(),
            label: None,
            value_text: None,
            disabled: false,
            force: Force::None,
        }
    }
    /// The header row: a label and the formatted value (`"40%"`).
    pub fn label(mut self, label: impl Into<SharedString>, value: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self.value_text = Some(value.into());
        self
    }
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
    pub fn force(mut self, f: Force) -> Self {
        self.force = f;
        self
    }
}

impl RenderOnce for Slider {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = theme::tokens(cx);
        let pct = self.state.read(cx).percentage().end;
        // gpui-base's slider has no keyboard: ←/→ (↓/↑) step, Home/End jump.
        let id = nid("slider", self.state.entity_id().as_u64());
        let handle = focus_handle(id.clone(), window, cx);
        let focused = focus_visible(&handle, self.force, window);
        let keys = self.state.clone();
        // The 2 px surface ring, and the 2px-off accent outline on focus.
        let thumb = SliderThumb::new(&self.state)
            .disabled(self.disabled)
            .absolute()
            .top(px(-5.))
            .left(relative(pct))
            .ml(-(size::SLIDER_THUMB / 2.))
            .size(size::SLIDER_THUMB)
            .rounded_full()
            .bg(p.accent)
            .child(ring_box(p.surface, px(2.), radius::PILL))
            .when(focused, |el| el.child(outline(p.accent, radius::PILL)));
        let header = self.label.map(|label| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(text::BASE)
                        .font_weight(theme::W_MEDIUM)
                        .text_color(p.text_subtle)
                        .child(label),
                )
                .child(
                    div()
                        .font_family(FONT_MONO)
                        .text_size(text::SM)
                        .text_color(p.text_muted)
                        .children(self.value_text),
                )
        });
        div()
            .id(id)
            .flex()
            .flex_col()
            .gap(px(8.))
            .w_full()
            .when(!self.disabled, |el| {
                el.track_focus(&handle).on_key_down(move |e, window, cx| {
                    keys.update(cx, |s, cx| {
                        let (v, step) = (s.value().end(), s.step_value());
                        let next = match e.keystroke.key.as_str() {
                            "left" | "down" => v - step,
                            "right" | "up" => v + step,
                            "home" => s.min_value(),
                            "end" => s.max_value(),
                            _ => return,
                        };
                        let next = next.clamp(s.min_value(), s.max_value());
                        s.set_value(next, window, cx);
                        cx.emit(gpui_kit::base::slider::SliderEvent::Change(next.into()));
                        cx.notify();
                    });
                })
            })
            .when(self.disabled, |el| el.opacity(theme::DISABLED_OPACITY))
            .children(header)
            .child(
                BaseSlider::new(&self.state)
                    .accessibility_label(self.aria)
                    .disabled(self.disabled)
                    .flex()
                    .items_center()
                    .w_full()
                    .h(size::SWITCH_H)
                    .child(
                        SliderTrack::new(&self.state)
                            .disabled(self.disabled)
                            .flex()
                            .items_center()
                            .w_full()
                            .h(size::SWITCH_H)
                            .when(!self.disabled, |el| el.cursor_pointer())
                            .child(
                                SliderIndicator::new(&self.state)
                                    .relative()
                                    .w_full()
                                    .h(size::PROGRESS_H)
                                    .rounded(radius::PILL)
                                    .bg(p.wash_strong)
                                    .child(
                                        div()
                                            .absolute()
                                            .top_0()
                                            .bottom_0()
                                            .left_0()
                                            .w(relative(pct))
                                            .rounded(radius::PILL)
                                            .bg(p.accent),
                                    )
                                    .child(thumb),
                            ),
                    ),
            )
    }
}
