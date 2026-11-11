//! One model row (`.ch-opt`): the provider letter, the name with its search
//! match, "Make default", the key number or context size, the star and the tick.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, Div, SharedString};

use super::rows::{OptionRow, Pick};
use super::view::glyph;
use super::ModelPicker;
use crate::theme::{self, radius, text, Palette, FONT_MONO, FONT_UI};
use crate::ui::{icon, kbd, nid};

impl ModelPicker {
    pub(super) fn option(
        &self,
        o: &OptionRow,
        cursor: bool,
        base: &str,
        p: &Palette,
    ) -> gpui_kit::Stateful<Div> {
        let sentinel = o.pick == Pick::Sentinel;
        let lit = cursor;
        let fav = self.on_favorite.clone();
        let engine = match &o.pick {
            Pick::Pool(e) => Some(e.clone()),
            _ => None,
        };
        // One line: the name with its search match marked in the run (a child
        // div would stack under it), then Default's quiet resolved model.
        let name = crate::chat::inline_runs(&[
            (o.main.before.as_str(), None),
            (
                o.main.matched.as_str(),
                Some((gpui_kit::FontWeight::BOLD, p.accent)),
            ),
            (o.main.after.as_str(), None),
        ]);
        let main = div()
            .flex_1()
            .min_w_0()
            .flex()
            .items_baseline()
            .overflow_hidden()
            .whitespace_nowrap()
            .when(sentinel, |el| el.font_family(FONT_UI))
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(name),
            )
            .when_some(
                sentinel
                    .then(|| self.spec.sentinel.as_ref().and_then(|(_, r)| r.clone()))
                    .flatten(),
                |el, (_, m)| {
                    el.child(
                        div()
                            .flex_none()
                            .ml(px(6.))
                            .font_family(FONT_MONO)
                            .text_size(text::XS)
                            .text_color(p.text_subtle)
                            .child(SharedString::from(crate::chat::format::model_display_name(
                                &m,
                            ))),
                    )
                },
            );
        let make_default = (o.can_make_default)
            .then(|| engine.clone().zip(self.on_make_default.clone()))
            .flatten()
            .map(|(e, f)| {
                let (muted, strong) = (p.text_muted, p.text_strong);
                div()
                    .id(nid(&format!("{base}-mkdef"), o.index as u64))
                    .test_support()
                    .role(gpui_kit::Role::Button)
                    .flex_none()
                    .px(px(5.))
                    .rounded(px(4.))
                    .font_family(FONT_UI)
                    .font_weight(theme::W_MEDIUM)
                    .text_size(text::XS)
                    .text_color(muted)
                    .hover(move |s| s.text_color(strong))
                    .when(!lit, |el| el.invisible())
                    .child("Make default")
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        f(e.clone(), window, cx);
                    })
            });
        let meta = match (o.num, &o.sub) {
            (Some(n), _) => Some(kbd(n.to_string(), p).into_any_element()),
            (None, Some(s)) => Some(
                div()
                    .font_family(FONT_MONO)
                    .font_weight(theme::W_REGULAR)
                    .text_size(text::XS)
                    .text_color(p.text_subtle)
                    .whitespace_nowrap()
                    .child(SharedString::from(s.clone()))
                    .into_any_element(),
            ),
            _ => None,
        };
        let star = o.starred.zip(engine).map(|(on, e)| {
            div()
                .id(nid(&format!("{base}-star"), o.index as u64))
                .role(gpui_kit::Role::Button)
                .aria_label(if on { "Unstar" } else { "Star" })
                .flex()
                .items_center()
                .justify_center()
                .size(px(20.))
                .rounded(px(5.))
                .text_color(if on { p.warn } else { p.text_subtle })
                .when(!on && !lit, |el| el.invisible())
                .child(icon(IconName::Star.path(), px(12.)))
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    if let Some(f) = &fav {
                        f(e.clone(), window, cx);
                    }
                })
                .into_any_element()
        });
        div()
            .id(nid(&format!("{base}-opt"), o.index as u64))
            .role(gpui_kit::Role::ListBoxOption)
            .aria_selected(o.selected)
            .aria_label(SharedString::from(o.title.clone()))
            .flex()
            .items_center()
            .gap(px(8.))
            .w_full()
            .px(px(8.))
            .py(px(6.))
            .rounded(radius::SM)
            .cursor_pointer()
            .font_family(FONT_MONO)
            .font_weight(theme::W_MEDIUM)
            .text_size(text::SM)
            .text_color(if lit || o.selected {
                p.text_strong
            } else {
                p.text
            })
            .when(lit, |el| el.bg(p.surface_hover))
            .child(glyph(&o.glyph, p))
            .child(main)
            .children(make_default)
            .children(meta)
            .child(div().w(px(20.)).flex_none().children(star))
            .child(
                div()
                    .w(px(14.))
                    .flex_none()
                    .text_color(p.accent)
                    .when(o.selected, |el| {
                        el.child(icon(IconName::Check.path(), px(13.)))
                    }),
            )
    }
}
