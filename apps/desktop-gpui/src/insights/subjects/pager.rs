//! A section's pager, drawn inside its kicker heading (`Pager.svelte`):
//! `// FORMING · 69 ──── 11–20 OF 69 ‹ ›`. The range text opens a small
//! rows-per-page menu (10 / 25 / 50 / Show all N); nothing draws when the
//! section fits its smallest page. Pages and sizes live on `Insights::pages`
//! per section key, so every tier (and the evidence drawer) keeps its own.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, AnyElement, Context, Role, SharedString};

use super::tiers::{page_label, page_slice};
use crate::insights::Insights;
use crate::theme::{self, radius, text, FONT_MONO, FONT_UI};
use crate::ui::Tracking as _;
use crate::ui::{icon, nid, popover};

/// `TIER_PAGE_SIZES`; `usize::MAX` is "Show all".
pub const SIZES: [usize; 3] = [10, 25, 50];
pub const SHOW_ALL: usize = usize::MAX;

/// `(page, size)` of a section; a missing entry is page 0 at `default`.
pub fn state(this: &Insights, key: &str, default: usize) -> (usize, usize) {
    this.pages.get(key).copied().unwrap_or((0, default))
}

/// `pageSlice` for a section: `(from, to, page, pages)`.
pub fn slice(
    this: &Insights,
    key: &str,
    total: usize,
    default: usize,
) -> (usize, usize, usize, usize) {
    let (page, size) = state(this, key, default);
    page_slice(total, page, size)
}

/// `pageHolding`: the page holding row `index` at `size` rows a page.
pub fn holding(index: usize, size: usize) -> usize {
    if size == SHOW_ALL {
        0
    } else {
        index / size.max(1)
    }
}

impl Insights {
    pub fn turn_page(&mut self, key: &str, page: usize, default: usize, cx: &mut Context<Self>) {
        let size = state(self, key, default).1;
        self.pages.insert(key.to_string(), (page, size));
        cx.notify();
    }

    /// A new size keeps the first row you were looking at on the page.
    fn resize_page(&mut self, key: &str, total: usize, size: usize, cx: &mut Context<Self>) {
        let (from, ..) = slice(self, key, total, SIZES[0]);
        self.pages
            .insert(key.to_string(), (holding(from, size), size));
        self.pager_menu = None;
        cx.notify();
    }
}

/// The pager for section `key` of `total` rows; `sizes` = false is a fixed
/// size with a static range (the evidence drawer). `label` names the section.
pub fn pager(
    this: &Insights,
    key: &str,
    label: &str,
    total: usize,
    default: usize,
    sizes: bool,
    cx: &mut Context<Insights>,
) -> Option<AnyElement> {
    let p = theme::tokens(cx);
    let smallest = if sizes { SIZES[0] } else { default };
    if total <= smallest {
        return None;
    }
    let (from, to, page, pages) = slice(this, key, total, default);
    let size = state(this, key, default).1;
    let range = SharedString::from(page_label(from, to, total).to_uppercase());
    let open = this.pager_menu.as_deref() == Some(key);
    let id = |part: &str| SharedString::from(format!("insights-pager-{key}-{part}"));
    let k = key.to_string();
    let range_el = div()
        .id(id("range"))
        .test_support()
        .role(Role::Button)
        .aria_label(SharedString::from(format!(
            "{label}: showing {}. Rows per page",
            page_label(from, to, total)
        )))
        .flex()
        .items_center()
        .h(px(22.))
        .px(px(6.))
        .rounded(radius::XS)
        .whitespace_nowrap()
        .text_color(if open { p.text_strong } else { p.text_subtle })
        .when(open, |d| d.bg(p.wash_strong))
        .when(sizes, |d| {
            d.cursor_pointer()
                .hover(|s| s.bg(p.wash_strong).text_color(p.text_strong))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.pager_menu = if this.pager_menu.as_deref() == Some(k.as_str()) {
                        None
                    } else {
                        Some(k.clone())
                    };
                    cx.notify();
                }))
        })
        .child(range);
    let menu = open.then(|| {
        let choices = SIZES
            .into_iter()
            .chain([SHOW_ALL])
            .enumerate()
            .map(|(i, s)| {
                let k = key.to_string();
                let on = s == size;
                div()
                    .id(nid(&format!("insights-pager-{key}-size"), i as u64))
                    .test_support()
                    .role(Role::MenuItemRadio)
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .h(px(26.))
                    .px(px(8.))
                    .rounded(radius::SM)
                    .cursor_pointer()
                    .font_family(FONT_UI)
                    .text_size(text::SM)
                    .font_weight(theme::W_MEDIUM)
                    .text_color(if on { p.text_strong } else { p.text_muted })
                    .hover(|st| st.bg(p.wash_strong).text_color(p.text_strong))
                    .child(if s == SHOW_ALL {
                        format!("Show all {total}")
                    } else {
                        s.to_string()
                    })
                    .child(
                        div()
                            .when(!on, |d| d.invisible())
                            .child(icon(IconName::Check.path(), px(13.))),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.resize_page(&k, total, s, cx)))
            });
        popover::float(
            div()
                .id(id("menu"))
                .role(Role::Menu)
                .aria_label("Rows per page")
                .absolute()
                .top(px(26.))
                .right_0()
                .min_w(px(140.))
                .p(px(4.))
                .flex()
                .flex_col()
                .gap(px(1.))
                .bg(p.surface_raised)
                .border_1()
                .border_color(p.border_strong)
                .rounded(radius::MD)
                .tracking(0.)
                .shadow(popover::popover_shadow(&p))
                .occlude()
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.pager_menu = None;
                    cx.notify();
                }))
                .child(
                    div()
                        .mx(px(8.))
                        .mt(px(5.))
                        .mb(px(6.))
                        .font_family(FONT_MONO)
                        .text_size(px(10.))
                        .tracking(0.08)
                        .text_color(p.text_faint)
                        .child("ROWS PER PAGE"),
                )
                .children(choices),
        )
    });
    let step = |part: &str, ic: IconName, aria: String, to_page: Option<usize>| {
        let k = key.to_string();
        div()
            .id(id(part))
            .test_support()
            .role(Role::Button)
            .aria_label(SharedString::from(aria))
            .size(px(22.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(radius::XS)
            .text_color(p.text_muted)
            .when(to_page.is_none(), |d| d.opacity(0.3))
            .when_some(to_page, |d, n| {
                d.cursor_pointer()
                    .hover(|s| s.bg(p.wash_strong).text_color(p.text_strong))
                    .on_click(cx.listener(move |this, _, _, cx| this.turn_page(&k, n, default, cx)))
            })
            .child(icon(ic.path(), px(15.)))
    };
    let close_key = key.to_string();
    let wrapper = popover::esc_closes(div().relative().flex(), open, {
        let me = cx.entity().downgrade();
        move |_, cx| {
            let _ = me.update(cx, |this, cx| {
                if this.pager_menu.as_deref() == Some(close_key.as_str()) {
                    this.pager_menu = None;
                    cx.notify();
                }
            });
        }
    });
    Some(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.))
            .child(wrapper.child(range_el).children(menu))
            .when(pages > 1, |d| {
                d.child(
                    div()
                        .flex()
                        .gap(px(2.))
                        .mr(px(-3.))
                        .child(step(
                            "prev",
                            IconName::ChevronLeft,
                            format!("{label}: previous page"),
                            page.checked_sub(1),
                        ))
                        .child(step(
                            "next",
                            IconName::ChevronRight,
                            format!("{label}: next page"),
                            (page + 1 < pages).then_some(page + 1),
                        )),
                )
            })
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_size_keeps_the_first_row_and_show_all_is_one_page() {
        assert_eq!(holding(20, 25), 0);
        assert_eq!(holding(30, 25), 1);
        assert_eq!(holding(49, SHOW_ALL), 0);
        assert_eq!(page_slice(69, 1, 10), (10, 20, 1, 7));
        assert_eq!(page_slice(69, 0, SHOW_ALL), (0, 69, 0, 1));
    }
}
