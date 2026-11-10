//! The 52 px content header (`SettingsBar.svelte`): a drag region with the
//! breadcrumb (group › section in view, the section only past the group's
//! first) and the one save status, with Retry when a save failed.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, MouseButton, Pixels};

use super::autosave::SaveStatus;
use super::groups::{self};
use super::{icons, Settings};
use crate::theme::{self, space, text};
use crate::ui::Button;

pub const BAR_H: f32 = 52.;

pub fn render(
    s: &Settings,
    scrolled: bool,
    inset: Pixels,
    cx: &mut Context<Settings>,
) -> AnyElement {
    let p = theme::tokens(cx);
    let g = groups::group(s.group);
    let query = s.inputs.search.read(cx).value().trim().to_string();
    let section = (query.is_empty() && g.sections[0].id != s.section)
        .then(|| groups::section_label(s.section));
    let crumb = div()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(text::MD)
        .text_color(p.text_strong)
        .child(div().font_weight(theme::W_SEMIBOLD).child(g.label))
        .children(section.map(|label| {
            div()
                .flex()
                .gap(px(6.))
                .child(div().text_color(p.text_faint).child("\u{203a}"))
                .child(div().text_color(p.text_subtle).child(label))
        }));
    let status = s.save_status();
    let (dot, fg, label) = match status {
        SaveStatus::Failed => (p.danger, p.danger_text, "Couldn\u{2019}t save"),
        SaveStatus::Blocked => (p.warn, p.text, "Resolve issues to save"),
        SaveStatus::Saving => (p.warn, p.text, "Saving\u{2026}"),
        SaveStatus::Saved => (p.source_mic, p.text_subtle, "All changes saved"),
    };
    let save = div()
        .id("settings-save-status")
        .role(if status == SaveStatus::Failed {
            gpui_kit::Role::Alert
        } else {
            gpui_kit::Role::Status
        })
        .ml_auto()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(text::SM)
        .text_color(fg)
        .child(div().flex_none().size(px(6.)).rounded_full().bg(dot))
        .child(label)
        .when(status == SaveStatus::Failed && s.failed.is_some(), |d| {
            d.child(
                Button::new("settings-retry", "Retry")
                    .ghost()
                    .sm()
                    .h(px(22.))
                    .icon(icons::RETRY)
                    .on_click(super::click(cx, |s, cx| s.retry(cx))),
            )
        });
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(space::S3)
        .h(px(BAR_H))
        // Aligned with the page column (`view::inset`), not the window edge.
        .pl(space::S7 + inset)
        .pr(space::S5 + inset)
        .border_b_1()
        .border_color(if scrolled {
            p.hairline
        } else {
            gpui_kit::transparent_black()
        })
        .when(scrolled, |d| d.bg(gpui_kit::Hsla { a: 0.8, ..p.bg }))
        .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
        .child(crumb)
        .child(save)
        .into_any_element()
}
