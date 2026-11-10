//! The keyboard-shortcuts sheet (`.mx-pop--up.mx-keys`, 560 px, two columns),
//! opened by the status bar's `?` or by `/` and `?`. Rows come from
//! [`keymap::help_groups`], built live from capture state and the surface.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, Context, Role};

use super::Shell;
use crate::keymap::{self, HelpGroup};
use crate::theme::{self, radius, size, space, text, Palette};
use crate::ui::{kbd, kicker, pop_in, popover};

fn group(g: &HelpGroup, p: &Palette) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .mt(space::S2)
                .mb(px(2.))
                .text_size(text::BASE)
                .font_weight(theme::W_MEDIUM)
                .text_color(p.text_subtle)
                .child(g.title),
        )
        .children(g.rows.iter().enumerate().map(|(i, r)| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(space::S3)
                .py(px(5.))
                .when(i > 0, |el| el.border_t_1().border_color(p.hairline))
                .text_size(text::BASE)
                .line_height(relative(1.3))
                .text_color(p.text)
                .child(div().flex_1().min_w_0().child(r.label.clone()))
                .child(kbd(r.keys.clone(), p).flex_none())
        }))
}

pub fn render(shell: &Shell, p: &Palette, cx: &mut Context<Shell>) -> impl IntoElement {
    let extra = (shell.surface == super::Surface::Timeline)
        .then(|| shell.timeline.read(cx).help_group())
        .flatten();
    let groups = keymap::help_groups(
        &shell.bindings,
        shell.help_state(),
        extra.into_iter().collect(),
    );
    // Two columns, groups flowing left then right (CSS grid 1fr 1fr).
    let (left, right): (Vec<_>, Vec<_>) = groups.iter().enumerate().partition(|(i, _)| i % 2 == 0);
    let column = |gs: Vec<(usize, &HelpGroup)>| {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .children(gs.into_iter().map(|(_, g)| group(g, p)))
    };
    let head = div()
        .flex()
        .items_center()
        .gap(space::S2)
        .pt(space::S3)
        .px(space::S4)
        .pb(space::S2)
        .child(kicker("Keyboard shortcuts", p))
        .child(div().flex_1())
        .child(
            div()
                .id("shortcuts-rebind")
                .role(Role::Button)
                .flex()
                .items_center()
                .h(size::H_SM)
                .px(px(9.))
                .rounded(radius::BTN_SM)
                .text_size(text::BASE)
                .font_weight(theme::W_MEDIUM)
                .text_color(p.text_muted)
                .cursor_pointer()
                .hover(|s| s.bg(p.wash_strong).text_color(p.text_strong))
                .child("Rebind the global ones")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.overlay = None;
                    cx.defer(|cx| crate::window::open_settings(Some("shortcuts"), cx));
                })),
        );
    pop_in(
        popover("shortcuts-popover", "Keyboard shortcuts", p)
            .right(space::S3)
            .w(size::POP_KEYS_W)
            .pb(space::S3)
            .child(head)
            .child(
                div()
                    .flex()
                    .gap(space::S4)
                    .px(space::S4)
                    .child(column(left))
                    .child(column(right)),
            ),
        size::STATUS_BAR + size::POP_GAP,
        true,
    )
}
