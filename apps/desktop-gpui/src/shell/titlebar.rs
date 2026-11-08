//! The 38 px main-window titlebar (`.mx-titlebar`): surface pill · command field ·
//! Chats, bell (badge = worst severity), gear (toggles Settings). macOS draws the
//! traffic lights in the 78 px left inset; the bar's background drags the window.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, Context, MouseButton, Window};

use super::model::{Severity, Surface};
use super::{surface_pill, Overlay, Shell};
use crate::keymap::{self, OpenChats, ToggleSettings};
use crate::theme::{size, space, Palette};
use crate::ui::{icon, icon_button, with_tip};

pub fn render(
    shell: &Shell,
    p: &Palette,
    _window: &mut Window,
    cx: &mut Context<Shell>,
) -> impl IntoElement {
    let badge = shell
        .model
        .notifications
        .iter()
        .map(|n| match n.severity {
            Severity::Error => (2, p.danger),
            Severity::Warning => (1, p.warn),
            Severity::Info => (0, p.accent),
        })
        .max_by_key(|(rank, _)| *rank)
        .map(|(_, color)| {
            div()
                .absolute()
                // A 5px dot with a 1.5px ring outside it: 8px with an inside border.
                .top(px(3.5))
                .right(px(3.5))
                .size(px(8.))
                .rounded_full()
                .bg(color)
                .border(px(1.5))
                .border_color(p.titlebar_bg)
        });
    let count = shell.model.notifications.len();
    let bell_label = match count {
        0 => "Notifications".to_string(),
        1 => "Notifications — 1 notification".to_string(),
        n => format!("Notifications — {n} notifications"),
    };
    let settings_key = keymap::display(&shell.bindings.open_settings);

    let trail =
        div()
            .flex()
            .flex_1()
            .flex_basis(px(0.))
            .min_w_0()
            .h_full()
            .items_center()
            .justify_end()
            .gap(px(2.))
            .child(
                with_tip(
                    icon_button(
                        "titlebar-chats",
                        "Chats",
                        size::H_SM,
                        shell.surface == Surface::Chat,
                        p,
                    ),
                    "Chats",
                )
                .child(icon(IconName::MessageCircle.path(), size::ICON))
                .on_click(
                    cx.listener(|_, _, window, cx| window.dispatch_action(Box::new(OpenChats), cx)),
                ),
            )
            .child(
                with_tip(
                    icon_button("titlebar-bell", bell_label, size::H_SM, false, p),
                    "Notifications",
                )
                .aria_expanded(shell.overlay == Some(Overlay::Notifications))
                .child(icon(IconName::Bell.path(), size::ICON))
                .children(badge)
                .on_click(
                    cx.listener(|this, _, _, cx| this.toggle_overlay(Overlay::Notifications, cx)),
                ),
            )
            .child(
                with_tip(
                    icon_button("titlebar-settings", "Settings", size::H_SM, false, p),
                    format!("Settings  {settings_key}"),
                )
                .child(icon(IconName::Settings.path(), size::ICON))
                .on_click(cx.listener(|_, _, window, cx| {
                    window.dispatch_action(Box::new(ToggleSettings), cx)
                })),
            );

    div()
        .id("titlebar")
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .gap(space::S3)
        .h(size::TITLEBAR)
        .pl(size::TRAFFIC_LIGHTS)
        .pr(space::S2)
        .min_w_0()
        // `titlebar-bg` 82% over the window; GPUI has no in-window backdrop blur.
        .bg(p.titlebar_glass)
        .border_b_1()
        .border_color(p.hairline)
        .on_mouse_down(MouseButton::Left, |event, window, _| {
            if event.click_count == 2 {
                window.titlebar_double_click();
            } else {
                window.start_window_move();
            }
        })
        .child(
            div()
                .flex()
                .flex_1()
                .flex_basis(px(0.))
                .min_w_0()
                .h_full()
                .items_center()
                .child(surface_pill::render(shell, p, cx)),
        )
        .child(shell.command.clone())
        .child(trail)
}
