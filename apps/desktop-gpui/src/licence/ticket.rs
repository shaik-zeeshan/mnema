//! The device-limit receipt: a ticket that was never stamped. The same paper
//! as the ruler receipt (the tear line and its bites), with the three taken
//! device slots and the dashed one this Mac is waiting for, then the way out.
//! The reset note ("Devices freed…", "already reset recently…") lands inside it.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, AnyElement, Div, SharedString};

use super::parts::{self, Parts, DEVICE_CAP};
use crate::theme::{self, radius, size, space, text, Palette};
use crate::ui::{icon, kicker};

pub fn render(parts: Parts, p: &Palette) -> AnyElement {
    let Parts {
        copy,
        buttons,
        note,
        ..
    } = parts;
    let head = div()
        .flex()
        .flex_col()
        .gap(space::S2)
        .px(space::S5)
        .pt(space::S5)
        .pb(space::S5)
        .child(kicker("License receipt", p))
        .child(parts::headline(copy.title, text::H1, p).pt(space::S1))
        .children(copy.who.map(|w| parts::line(w, text::MD, p.text)));
    let rest = div()
        .flex()
        .flex_col()
        .gap(space::S3)
        .px(space::S5)
        .pt(space::S5)
        .pb(space::S5)
        .child(devices(p))
        .children(
            copy.sub
                .map(|s| parts::line(s, text::BASE, p.text_muted).pt(space::S1)),
        )
        .child(parts::actions(buttons, note).pt(space::S1));
    parts::body(copy.title)
        .child(parts::paper(head, p.surface_raised, rest, p))
        .test_support()
        .into_any_element()
}

/// One device slot: filled (in use) or the dashed one this Mac is waiting for.
fn slot(taken: bool, p: &Palette) -> Div {
    let base = div()
        .flex()
        .items_center()
        .justify_center()
        .w_full()
        .h(px(40.))
        .rounded(radius::MD)
        .border_1();
    if taken {
        base.bg(p.wash_strong)
            .border_color(p.border_strong)
            .text_color(p.text_muted)
    } else {
        base.border_dashed()
            .border_color(p.warn_border)
            .bg(p.warn_bg)
            .text_color(p.warn)
    }
    .child(icon(IconName::Laptop.path(), size::ICON))
}

/// Three taken slots and the dashed one this Mac waits for.
fn devices(p: &Palette) -> Div {
    let cell = |taken: bool, label: SharedString| {
        div()
            .flex_1()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(slot(taken, p))
            .child(
                parts::label(&label, p)
                    .when(!taken, |d| d.text_color(p.warn))
                    .flex()
                    .justify_center(),
            )
    };
    div()
        .flex()
        .gap(space::S2)
        .font_weight(theme::W_MEDIUM)
        .children((1..=DEVICE_CAP).map(|n| cell(true, format!("Device {n}").into())))
        .child(cell(false, "This Mac".into()).ml(space::S2))
}
