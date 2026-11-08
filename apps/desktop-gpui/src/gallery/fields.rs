//! Segmented, switch, text input, stepper, slider, field warning.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, Window};

use super::{cell, row, Gallery};
use crate::theme::{self, space};
use crate::ui::input::{field_warning, TextInput};
use crate::ui::segmented::{opt_icon, Segmented};
use crate::ui::slider::Slider;
use crate::ui::stepper::Stepper;
use crate::ui::switch::Switch;
use crate::ui::Force;

pub(super) fn render(g: &mut Gallery, _: &mut Window, cx: &mut Context<Gallery>) -> AnyElement {
    let p = theme::tokens(cx);
    let this = cx.weak_entity();
    let seg = |k: usize| {
        let this = this.clone();
        move |ix: usize, _: &mut Window, cx: &mut gpui_kit::App| {
            this.update(cx, |g, cx| {
                g.seg[k] = ix;
                cx.notify();
            })
            .ok();
        }
    };
    let range = ["Day", "Week", "Month"];
    let segmented = vec![
        cell(
            "sm",
            Segmented::new("seg-range", "Range", range, g.seg[0]).on_change(seg(0)),
            &p,
        ),
        cell(
            "mono",
            Segmented::new("seg-mono", "Segment", ["1m", "2m", "5m"], g.seg[1])
                .mono()
                .on_change(seg(1)),
            &p,
        ),
        cell(
            "md + icons",
            Segmented::new(
                "seg-theme",
                "Theme",
                [
                    opt_icon("Dark", IconName::Moon.path()),
                    opt_icon("Light", IconName::Sun.path()),
                    "System".into(),
                ],
                g.seg[2],
            )
            .md()
            .on_change(seg(2)),
            &p,
        ),
    ];
    let seg_states = vec![
        cell(
            "hover",
            Segmented::new("seg-h", "Range", range, 1).force(0, Force::Hover),
            &p,
        ),
        cell(
            "focus",
            Segmented::new("seg-f", "Range", range, 1).force(2, Force::Focus),
            &p,
        ),
        cell(
            "disabled",
            Segmented::new("seg-d", "Range", range, 1).disabled(true),
            &p,
        ),
    ];
    let sw = |k: usize| {
        let this = this.clone();
        move |on: bool, _: &mut Window, cx: &mut gpui_kit::App| {
            this.update(cx, |g, cx| {
                g.switches[k] = on;
                cx.notify();
            })
            .ok();
        }
    };
    let switches = vec![
        cell(
            "on",
            Switch::new("sw-on", "Enabled", g.switches[0]).on_toggle(sw(0)),
            &p,
        ),
        cell(
            "off",
            Switch::new("sw-off", "Enabled", g.switches[1]).on_toggle(sw(1)),
            &p,
        ),
        cell(
            "focus",
            Switch::new("sw-f", "Enabled", true).force(Force::Focus),
            &p,
        ),
        cell(
            "disabled on",
            Switch::new("sw-d1", "Enabled", true).disabled(true),
            &p,
        ),
        cell(
            "disabled off",
            Switch::new("sw-d0", "Enabled", false).disabled(true),
            &p,
        ),
    ];
    let i = &g.inputs;
    let w = px(120.);
    let inputs = vec![
        cell(
            "placeholder",
            TextInput::new("in-0", "Label", &i[0]).w(w),
            &p,
        ),
        cell(
            "hover",
            TextInput::new("in-1", "Label", &i[1])
                .w(w)
                .force(Force::Hover),
            &p,
        ),
        cell(
            "focus",
            TextInput::new("in-2", "Label", &i[2])
                .w(w)
                .force(Force::Focus),
            &p,
        ),
        cell(
            "error",
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(TextInput::new("in-3", "Email", &i[3]).w(w).invalid(true))
                .child(field_warning("Enter a valid email.", &p)),
            &p,
        ),
        cell(
            "disabled",
            TextInput::new("in-4", "Label", &i[4]).w(w).disabled(true),
            &p,
        ),
        cell(
            "num",
            TextInput::new("in-5", "Frames", &i[5]).w(px(80.)).num(),
            &p,
        ),
    ];
    let sw_ = px(180.);
    let mut steppers = vec![
        cell(
            "unit",
            div().w(sw_).child(
                Stepper::new("st-0", "Retention", &i[6])
                    .range(Some(1), Some(365))
                    .unit("days"),
            ),
            &p,
        ),
        cell(
            "no unit",
            div().w(px(150.)).child(
                Stepper::new("st-1", "Width", &i[7])
                    .range(Some(320), None)
                    .step(10),
            ),
            &p,
        ),
        cell(
            "focus",
            div().w(sw_).child(
                Stepper::new("st-2", "Bitrate", &i[8])
                    .unit("Mbps")
                    .force(Force::Focus),
            ),
            &p,
        ),
        cell(
            "at max · invalid",
            div().w(sw_).child(
                Stepper::new("st-3", "Interval", &i[9])
                    .range(Some(1), Some(90))
                    .unit("sec")
                    .invalid(true),
            ),
            &p,
        ),
        cell(
            "disabled",
            div().w(sw_).child(
                Stepper::new("st-4", "Retention", &i[10])
                    .unit("days")
                    .disabled(true),
            ),
            &p,
        ),
    ];
    let steppers_a: Vec<AnyElement> = steppers.drain(..3).collect();
    let s = &g.sliders;
    let sliders = vec![
        cell(
            "labelled",
            div()
                .w(px(220.))
                .child(Slider::new(&s[0], "Volume").label("Volume", "40%")),
            &p,
        ),
        cell(
            "focus",
            div()
                .w(px(180.))
                .child(Slider::new(&s[1], "Threshold").force(Force::Focus)),
            &p,
        ),
        cell(
            "disabled",
            div()
                .w(px(180.))
                .child(Slider::new(&s[2], "Threshold").disabled(true)),
            &p,
        ),
    ];
    div()
        .flex()
        .flex_col()
        .gap(space::S5)
        .child(row("segmented", segmented, &p))
        .child(row("states", seg_states, &p))
        .child(row("switch", switches, &p))
        .child(row("input", inputs, &p).items_start())
        .child(row("stepper", steppers_a, &p))
        .child(row("", steppers, &p))
        .child(row("slider", sliders, &p))
        .into_any_element()
}
