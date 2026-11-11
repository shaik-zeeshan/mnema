//! `.mx-menu` (a GPUI-drawn list inside a popover) and the option row Select /
//! Combobox share. Right-click context menus are OS-drawn: use gpui-kit's
//! [`NativeMenu`] (`NativeMenu::new().menu("Copy", Box::new(Copy)).show(pos, window, cx)`).

pub use gpui_kit::component::native_menu::NativeMenu;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, App, ClickEvent, Div, ElementId, SharedString, Window};

use super::{icon, popover, Force};
use crate::theme::{self, radius, space, text, Palette, FONT_MONO};

/// One `.mx-menu li`: label, optional mono meta on the right.
pub fn menu_item(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    meta: Option<SharedString>,
    force: Force,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    p: &Palette,
) -> impl IntoElement {
    let hover = (p.surface_hover, p.text_strong);
    let lit = force == Force::Hover;
    div()
        .id(id)
        .test_support()
        .role(gpui_kit::Role::MenuItem)
        .flex()
        .items_center()
        .gap(space::S2)
        .px(px(10.))
        .py(px(7.))
        .rounded(radius::SM)
        .text_color(if lit { hover.1 } else { p.text })
        .when(lit, |d| d.bg(hover.0))
        .cursor_pointer()
        .hover(move |s| s.bg(hover.0).text_color(hover.1))
        .child(label.into())
        .when_some(meta, |d, m| {
            d.child(
                div()
                    .ml_auto()
                    .font_family(FONT_MONO)
                    .text_size(text::XS)
                    .text_color(p.text_subtle)
                    .child(m),
            )
        })
        .on_click(on_click)
}

/// `.mx-pop` + `.mx-menu`: the surface with 4 px padding around the items.
pub fn menu(items: impl IntoIterator<Item = impl IntoElement>, p: &Palette) -> impl IntoElement {
    popover::surface(p)
        .id("menu")
        .role(gpui_kit::Role::Menu)
        .p(px(4.))
        .text_size(text::BASE)
        .children(items)
}

/// A Select / Combobox option: check slot, label; selected = the wash fill.
pub fn option_row(
    id: impl Into<ElementId>,
    label: impl IntoElement,
    selected: bool,
    highlighted: bool,
    p: &Palette,
) -> gpui_kit::Stateful<Div> {
    let (bg, fg) = if selected {
        (Some(p.wash_strong), p.text_strong)
    } else if highlighted {
        (Some(p.surface_hover), p.text_strong)
    } else {
        (None, p.text)
    };
    let hover = (p.surface_hover, p.text_strong);
    div()
        .id(id)
        .role(gpui_kit::Role::ListBoxOption)
        .aria_selected(selected)
        .flex()
        .items_center()
        .gap(px(8.))
        .w_full()
        .px(px(10.))
        .py(px(7.))
        .rounded(radius::SM)
        .text_size(text::MD)
        .line_height(gpui_kit::relative(1.3))
        .text_color(fg)
        .when_some(bg, |d, bg| d.bg(bg))
        .cursor_pointer()
        .when(!selected, |d| {
            d.hover(move |s| s.bg(hover.0).text_color(hover.1))
        })
        .child(
            div()
                .flex_none()
                .w(px(12.))
                .text_color(p.accent)
                .when(selected, |d| d.child(icon(IconName::Check.path(), px(12.)))),
        )
        .child(label)
}

/// The "Loading…" / "No matches" line of an empty option list.
pub fn options_empty(msg: impl Into<SharedString>, p: &Palette) -> Div {
    div()
        .px(px(10.))
        .py(px(14.))
        .text_center()
        .text_size(text::BASE)
        .line_height(gpui_kit::relative(1.3))
        .text_color(p.text_subtle)
        .font_weight(theme::W_REGULAR)
        .child(msg.into())
}
