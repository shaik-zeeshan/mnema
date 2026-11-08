//! Labelled panes standing in for the surfaces until their slices land
//! (Timeline S8, Insights S12, Chat S11, Settings S13).

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, Context, Role};

use super::model::Surface;
use super::Shell;
use crate::keymap::CloseOverlay;
use crate::theme::{self, radius, size, space, text, Palette};
use crate::ui::{icon, kicker};

pub fn render(shell: &Shell, p: &Palette, cx: &mut Context<Shell>) -> impl IntoElement {
    let (name, slice) = match shell.surface {
        Surface::Timeline => ("Timeline", "S8"),
        Surface::Insights => ("Insights", "S12"),
        Surface::Chat => ("Chat", "S11"),
        Surface::Settings => ("Settings", "S13"),
    };
    let back = (shell.surface == Surface::Settings).then(|| {
        div()
            .id("settings-back")
            .role(Role::Button)
            .absolute()
            .top(space::S4)
            .left(size::TRAFFIC_LIGHTS)
            .flex()
            .items_center()
            .gap(px(6.))
            .h(size::H_SM)
            .px(px(9.))
            .rounded(px(5.))
            .text_size(text::BASE)
            .font_weight(theme::W_MEDIUM)
            .text_color(p.text_muted)
            .cursor_pointer()
            .hover(|s| s.bg(p.wash_strong).text_color(p.text_strong))
            .child(icon(IconName::ArrowLeft.path(), size::ICON_SM))
            .child("Back to app")
            .on_click(
                cx.listener(|_, _, window, cx| window.dispatch_action(Box::new(CloseOverlay), cx)),
            )
    });
    let query = (shell.surface == Surface::Chat)
        .then(|| shell.chat_query.clone())
        .flatten()
        .map(|q| div().text_color(p.text_muted).child(format!("“{q}”")));
    div()
        .id(gpui_kit::SharedString::from(format!(
            "surface-{}",
            name.to_lowercase()
        )))
        .role(Role::Main)
        .aria_label(name)
        .relative()
        .flex()
        .flex_1()
        .min_h_0()
        .items_center()
        .justify_center()
        .children(back)
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(space::S2)
                .p(space::S6)
                .rounded(radius::LG)
                .border_1()
                .border_color(p.hairline)
                .child(kicker(name, p))
                .child(
                    div()
                        .text_size(text::LG)
                        .font_weight(theme::W_SEMIBOLD)
                        .text_color(p.text_strong)
                        .child(format!("{name} lands in {slice}")),
                )
                .children(query),
        )
}
