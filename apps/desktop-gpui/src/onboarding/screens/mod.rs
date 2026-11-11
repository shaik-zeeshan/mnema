//! One file per step; each adds an `impl Onboarding` block that returns the
//! step's main column and its footer.

mod ai;
mod capture;
pub(super) mod capture_apps;
mod change;
mod finale;
mod models;
mod permissions;
mod setup;
mod voice;
mod welcome;
mod your_settings;

use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, ClickEvent, Context, SharedString, Window};

use super::parts;
use super::Onboarding;
pub(super) use capture_apps::AppPick;
use crate::ui::Button;

/// The footer's primary "next" button, with the trailing arrow.
pub(super) fn primary(
    id: &'static str,
    label: impl Into<SharedString>,
    cx: &mut Context<Onboarding>,
    f: impl Fn(&mut Onboarding, &mut Window, &mut Context<Onboarding>) + 'static,
) -> Button {
    Button::new(id, label)
        .primary()
        .icon(parts::ARROW)
        .icon_after()
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| f(this, window, cx)))
}

/// A plain / ghost footer or inline button.
pub(super) fn button(
    id: impl Into<gpui_kit::ElementId>,
    label: impl Into<SharedString>,
    cx: &mut Context<Onboarding>,
    f: impl Fn(&mut Onboarding, &mut Window, &mut Context<Onboarding>) + 'static,
) -> Button {
    Button::new(id, label)
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| f(this, window, cx)))
}

pub(super) fn back(cx: &mut Context<Onboarding>) -> AnyElement {
    button("ob-back", "Back", cx, |this, _, cx| this.back(cx))
        .ghost()
        .into_any_element()
}

/// A `Segmented` / `Choice` change (`Fn(usize, ..)`) routed to this entity.
pub(super) fn on_ix(
    cx: &mut Context<Onboarding>,
    f: impl Fn(&mut Onboarding, usize, &mut Context<Onboarding>) + 'static,
) -> impl Fn(usize, &mut Window, &mut gpui_kit::App) + 'static {
    let this = cx.entity().downgrade();
    move |ix, _, cx| {
        let _ = this.update(cx, |s, cx| f(s, ix, cx));
    }
}
