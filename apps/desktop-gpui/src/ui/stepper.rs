//! `Stepper.svelte`: `[-] .mx-input.num [+]`, an optional mono unit chip, the
//! bound buttons disabled at the limits. The value is the raw text of an
//! `InputState` so a half-typed number never gets rewritten mid-keystroke; the
//! owner calls [`clamp_raw`] on blur / Enter (Svelte's `commit`).

use gpui_kit::assets::IconName;
use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, App, ElementId, Entity, SharedString, Window};

use super::input::TextInput;
use super::{nid, Button, Force};
use crate::theme::{self, space, text, FONT_MONO};

/// An integer, or `None` for blank / non-numeric text.
pub fn parse_raw(raw: &str) -> Option<i64> {
    let t = raw.trim();
    let digits = t.strip_prefix('-').unwrap_or(t);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    t.parse().ok()
}

fn clamp(v: i64, min: Option<i64>, max: Option<i64>) -> i64 {
    let v = min.map_or(v, |m| v.max(m));
    max.map_or(v, |m| v.min(m))
}

/// Blank stays blank, non-numeric is left for the validator, numbers clamp.
pub fn clamp_raw(raw: &str, min: Option<i64>, max: Option<i64>) -> String {
    if raw.trim().is_empty() {
        return String::new();
    }
    parse_raw(raw).map_or(raw.to_string(), |v| clamp(v, min, max).to_string())
}

/// The +/- result: steps from the parsed value, or seeds from `min` (then 0).
pub fn step_raw(raw: &str, dir: i64, step: i64, min: Option<i64>, max: Option<i64>) -> String {
    let next = match parse_raw(raw) {
        Some(v) => v + dir * step,
        None => min.unwrap_or(0),
    };
    clamp(next, min, max).to_string()
}

#[derive(IntoElement)]
pub struct Stepper {
    id: ElementId,
    aria: SharedString,
    state: Entity<InputState>,
    min: Option<i64>,
    max: Option<i64>,
    step: i64,
    unit: Option<SharedString>,
    disabled: bool,
    invalid: bool,
    force: Force,
}

impl Stepper {
    pub fn new(
        id: impl Into<ElementId>,
        aria: impl Into<SharedString>,
        state: &Entity<InputState>,
    ) -> Self {
        Self {
            id: id.into(),
            aria: aria.into(),
            state: state.clone(),
            min: None,
            max: None,
            step: 1,
            unit: None,
            disabled: false,
            invalid: false,
            force: Force::None,
        }
    }
    pub fn range(mut self, min: Option<i64>, max: Option<i64>) -> Self {
        self.min = min;
        self.max = max;
        self
    }
    pub fn step(mut self, step: i64) -> Self {
        self.step = step;
        self
    }
    pub fn unit(mut self, unit: impl Into<SharedString>) -> Self {
        self.unit = Some(unit.into());
        self
    }
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
    pub fn invalid(mut self, on: bool) -> Self {
        self.invalid = on;
        self
    }
    /// Pin the field's state (gallery, goldens).
    pub fn force(mut self, f: Force) -> Self {
        self.force = f;
        self
    }
}

impl RenderOnce for Stepper {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = theme::tokens(cx);
        let raw = self.state.read(cx).value().to_string();
        let v = parse_raw(&raw);
        let at_min = matches!((self.min, v), (Some(m), Some(v)) if v <= m);
        let at_max = matches!((self.max, v), (Some(m), Some(v)) if v >= m);
        let base = self.id.to_string();
        let aria = self.aria.clone();
        let bump = |dir: i64, id: u64, icon: IconName, label: &str, at_bound: bool| {
            let (state, step, min, max) = (self.state.clone(), self.step, self.min, self.max);
            Button::icon_only(nid(&base, id), icon.path(), format!("{label} {aria}"))
                .disabled(self.disabled || at_bound)
                .on_click(move |_, window, cx| {
                    state.update(cx, |s, cx| {
                        let next = step_raw(&s.value(), dir, step, min, max);
                        s.set_value(next, window, cx);
                    })
                })
        };
        div()
            .id(self.id.clone())
            .role(gpui_kit::Role::SpinButton)
            .aria_label(self.aria.clone())
            .when_some(v, |el, v| el.aria_numeric_value(v as f64))
            .flex()
            .items_center()
            .gap(px(6.))
            .min_w_0()
            .when(self.disabled, |el| el.opacity(theme::DISABLED_OPACITY))
            .child(bump(-1, 0, IconName::Minus, "Decrease", at_min))
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .child({
                        let field = TextInput::new(nid(&base, 2), self.aria.clone(), &self.state)
                            .num()
                            .invalid(self.invalid)
                            .disabled(self.disabled)
                            .force(self.force);
                        match self.unit {
                            Some(_) => field.pad_right(px(52.)),
                            None => field.center(),
                        }
                    })
                    .when_some(self.unit.clone(), |el, unit| {
                        el.child(
                            div()
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .right(space::S3)
                                .flex()
                                .items_center()
                                .font_family(FONT_MONO)
                                .text_size(text::XS)
                                .font_weight(theme::W_MEDIUM)
                                .text_color(p.text_subtle)
                                .child(unit),
                        )
                    }),
            )
            .child(bump(1, 1, IconName::Plus, "Increase", at_max))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_values_clamp_only_numbers_and_step_from_min_when_blank() {
        assert_eq!(parse_raw(" 12 "), Some(12));
        assert_eq!(parse_raw("-3"), Some(-3));
        assert_eq!(parse_raw("1.5"), None);
        assert_eq!(parse_raw(""), None);
        assert_eq!(clamp_raw("", Some(1), Some(9)), "");
        assert_eq!(clamp_raw("abc", Some(1), Some(9)), "abc");
        assert_eq!(clamp_raw("42", Some(1), Some(9)), "9");
        assert_eq!(step_raw("", 1, 5, Some(10), None), "10");
        assert_eq!(step_raw("8", 1, 5, None, Some(10)), "10");
        assert_eq!(step_raw("8", -1, 5, Some(5), None), "5");
    }
}
