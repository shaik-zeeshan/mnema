//! The picker's look (chat.html › Model picker): the trigger, then a 360 px
//! `.mx-pop` with the search, the rows and an optional footer. Keys: ↑↓ move,
//! ↵ activates, 1-9 pick a favorite while the search is empty, Esc closes.

use crate::ui::input::FieldText as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::input::Input;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{
    div, px, radians, AnyElement, App, Div, ElementId, Entity, SharedString, Transformation, Window,
};

use super::rows::{HeaderIcon, Mode, Pick, Row};
use super::{state, ModelPicker, OnSelect, PickerState, Spec, Trigger};
use crate::theme::{self, radius, size, space, text, Palette, FONT_MONO, FONT_UI};
use crate::ui::chip::{chip, ChipKind};
use crate::ui::{icon, nid, popover, skeleton, with_tip, FIELD_RADIUS};

/// `.ch-pop` width and `.ch-opts` max height.
const POP_W: gpui_kit::Pixels = px(360.);
const LIST_MAX_H: gpui_kit::Pixels = px(320.);

pub fn where_chip(local: bool, p: &Palette) -> Div {
    chip(
        if local { "local" } else { "cloud" },
        if local {
            ChipKind::Accent
        } else {
            ChipKind::Info
        },
        p,
    )
    .h(px(18.))
    .px(px(5.))
}

/// The 18 px provider letter tile.
pub fn glyph(letter: &str, p: &Palette) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(18.))
        .rounded(px(5.))
        .bg(p.surface_hover)
        .border_1()
        .border_color(p.border_strong)
        .font_family(FONT_UI)
        .font_weight(theme::W_SEMIBOLD)
        .text_size(text::XS)
        .text_color(p.text_strong)
        .child(SharedString::from(letter.to_string()))
}

fn close(st: &Entity<PickerState>, cx: &mut App) {
    st.update(cx, |s, cx| {
        s.open = false;
        cx.notify();
    });
}

fn open(st: &Entity<PickerState>, spec: &Spec, window: &mut Window, cx: &mut App) {
    let quick = super::rows::has_quick_view(&spec.favorites, &spec.recents);
    let search = st.update(cx, |s, cx| {
        s.open = true;
        s.highlight = 0;
        s.groups.clear();
        s.mode = if quick { Mode::Quick } else { Mode::Browse };
        cx.notify();
        s.search.clone()
    });
    search.update(cx, |s, cx| {
        s.set_value("", window, cx);
        s.focus(window, cx);
    });
}

/// Run a keyboard / click row: fold a group, switch view, or pick.
fn activate(
    row: &Row,
    st: &Entity<PickerState>,
    on_select: &Option<OnSelect>,
    window: &mut Window,
    cx: &mut App,
) {
    match row {
        Row::Group { provider, open, .. } => st.update(cx, |s, cx| {
            s.groups.insert(provider.clone(), !open);
            cx.notify();
        }),
        Row::Browse { .. } | Row::Back { .. } => st.update(cx, |s, cx| {
            s.mode = if matches!(row, Row::Browse { .. }) {
                Mode::Browse
            } else {
                Mode::Quick
            };
            s.highlight = 0;
            cx.notify();
        }),
        Row::Option(o) => {
            close(st, cx);
            let pick = match &o.pick {
                Pick::Sentinel => None,
                Pick::Pool(e) | Pick::Exact(e) => Some(e.clone()),
            };
            if let Some(f) = on_select {
                f(pick, window, cx);
            }
        }
        _ => {}
    }
}

impl RenderOnce for ModelPicker {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = theme::tokens(cx);
        let st = state(&self.id, window, cx);
        let s = st.read(cx);
        let is_open = s.open || self.force_open;
        let query = s.search.read(cx).value().to_string();
        let rows = self.spec.rows(&query, s.mode, &s.groups);
        let highlight = s.highlight;
        let mode = s.mode;
        let search = s.search.clone();
        let base = self.id.to_string();

        let trigger = self.render_trigger(is_open, &st, &p);
        let trigger_at =
            popover::Anchor::new(ElementId::Name(format!("{base}-at").into()), window, cx);
        let pop = is_open.then(|| {
            let trigger_at = trigger_at.clone();
            let nav: Vec<Row> = rows
                .iter()
                .filter(|r| r.index().is_some())
                .cloned()
                .collect();
            let (st_k, sel_k, spec_k) = (st.clone(), self.on_select.clone(), self.spec.clone());
            let fav_k = self.on_favorite.clone();
            let st_out = st.clone();
            // "Loading models…" under Default while the quick view's pool lists.
            let loading_note = self.spec.loading
                && query.trim().is_empty()
                && mode == Mode::Quick
                && super::rows::has_quick_view(&self.spec.favorites, &self.spec.recents);
            let list: Vec<AnyElement> = rows
                .iter()
                .enumerate()
                .flat_map(|(i, row)| {
                    let note = (i == 1 && loading_note).then(|| {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .px(px(8.))
                            .pt(px(6.))
                            .pb(px(2.))
                            .text_size(text::SM)
                            .text_color(p.text_subtle)
                            .child(crate::ui::progress::spinner(size::SPIN_SM))
                            .child("Loading models…")
                            .into_any_element()
                    });
                    note.into_iter().chain(std::iter::once(
                        self.render_row(row, highlight, &st, &base, &p),
                    ))
                })
                .collect();
            let anchor = popover::surface(&p)
                .id(ElementId::Name(format!("{base}-pop").into()))
                .test_support()
                .role(gpui_kit::Role::Dialog)
                .aria_label("Choose model")
                .absolute()
                .left_0()
                .w(POP_W)
                .bg(p.surface_raised)
                .border_color(p.border_strong)
                .map(|el| {
                    if self.up {
                        el.bottom_full().mb(px(6.))
                    } else {
                        el.top_full().mt(px(6.))
                    }
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .mx(space::S2)
                        .mt(space::S2)
                        .px(px(10.))
                        // `.ch-pop .mx-search`: the kit search field, 32 px. Its
                        // input holds focus whenever the picker is open, so it
                        // always wears the `:focus-within` look.
                        .h(size::H_MD)
                        .rounded(FIELD_RADIUS)
                        .border_1()
                        .border_color(p.accent_border)
                        .shadow(vec![crate::ui::ring(p.accent_glow)])
                        .bg(p.surface_subtle)
                        .child(
                            div()
                                .text_color(p.accent)
                                .child(icon(IconName::Search.path(), px(13.))),
                        )
                        .child(
                            div().flex_1().min_w_0().child(
                                Input::new(&search)
                                    .aria_label("Search models")
                                    .appearance(false)
                                    .px_0()
                                    .field_h(size::H_MD - px(2.))
                                    .field_text(text::MD),
                            ),
                        ),
                )
                .child(crate::ui::scroll::area(
                    div()
                        .id(ElementId::Name(format!("{base}-list").into()))
                        .role(gpui_kit::Role::ListBox)
                        .aria_label("Model")
                        .flex()
                        .flex_col()
                        .max_h(LIST_MAX_H)
                        .overflow_y_scroll()
                        .p(px(4.))
                        .children(list)
                        .when(!self.spec.loading && rows.is_empty(), |el| {
                            el.child(note("No models yet.", &p))
                        }),
                ))
                .when_some(self.spec.foot.clone(), |el, foot| {
                    let link = self.foot_link.clone().map(|(label, f)| {
                        let (st, muted, strong) = (st.clone(), p.text_muted, p.text_strong);
                        div()
                            .id(ElementId::Name(format!("{base}-foot-link").into()))
                            .test_support()
                            .role(gpui_kit::Role::Link)
                            .ml_auto()
                            .cursor_pointer()
                            .text_color(muted)
                            .hover(move |s| s.text_color(strong))
                            .child(label)
                            .on_click(move |_, window, cx| {
                                close(&st, cx);
                                f(window, cx)
                            })
                    });
                    el.child(
                        div()
                            .flex()
                            .items_center()
                            .px(px(12.))
                            .py(px(9.))
                            .border_t_1()
                            .border_color(p.overlay_border)
                            .text_size(text::SM)
                            .text_color(p.text_subtle)
                            .child(foot)
                            .children(link),
                    )
                })
                .capture_key_down(move |e, window, cx| {
                    let key = e.keystroke.key.as_str();
                    let empty = st_k.read(cx).search.read(cx).value().is_empty();
                    let mods = &e.keystroke.modifiers;
                    let digit = key.len() == 1 && key.as_bytes()[0].is_ascii_digit() && key != "0";
                    if digit
                        && empty
                        && !mods.platform
                        && !mods.control
                        && !mods.alt
                        && st_k.read(cx).mode == Mode::Quick
                    {
                        let n: usize = key.parse().unwrap_or(0);
                        if let Some(fav) = spec_k.favorites.get(n - 1).cloned() {
                            cx.stop_propagation();
                            close(&st_k, cx);
                            if let Some(f) = &sel_k {
                                f(Some(fav), window, cx);
                            }
                        }
                        return;
                    }
                    let last = nav.len().saturating_sub(1);
                    if key == "d" && (mods.platform || mods.control) {
                        cx.stop_propagation();
                        let h = st_k.read(cx).highlight;
                        let star = nav.iter().find_map(|r| match r {
                            Row::Option(o) if o.index == h && o.starred.is_some() => {
                                match &o.pick {
                                    Pick::Pool(e) => Some(e.clone()),
                                    _ => None,
                                }
                            }
                            _ => None,
                        });
                        if let (Some(e), Some(f)) = (star, &fav_k) {
                            f(e, window, cx);
                        }
                        return;
                    }
                    match key {
                        "down" | "up" => {
                            cx.stop_propagation();
                            st_k.update(cx, |s, cx| {
                                s.highlight = if key == "down" {
                                    (s.highlight + 1).min(last)
                                } else {
                                    s.highlight.saturating_sub(1)
                                };
                                cx.notify();
                            });
                        }
                        "enter" => {
                            cx.stop_propagation();
                            let h = st_k.read(cx).highlight;
                            if let Some(row) = nav.iter().find(|r| r.index() == Some(h)) {
                                activate(row, &st_k, &sel_k, window, cx);
                            }
                        }
                        "escape" => {
                            cx.stop_propagation();
                            close(&st_k, cx);
                        }
                        // Typing re-filters the list: the top row is lit again.
                        _ if (e.keystroke.key_char.is_some()
                            && !mods.platform
                            && !mods.control)
                            || key == "backspace" =>
                        {
                            st_k.update(cx, |s, _| s.highlight = 0);
                        }
                        _ => {}
                    }
                })
                .on_mouse_down_out(move |e, _, cx| {
                    if trigger_at.outside(e, cx) {
                        close(&st_out, cx)
                    }
                });
            popover::float(anchor)
        });
        let esc = st.clone();
        popover::esc_closes(trigger_at.track(div()), is_open, move |_, cx| {
            close(&esc, cx)
        })
        .relative()
        .flex_none()
        .child(trigger)
        .children(pop)
    }
}

fn note(t: impl Into<SharedString>, p: &Palette) -> Div {
    div()
        .px(px(8.))
        .pt(px(6.))
        .pb(px(2.))
        .text_size(text::SM)
        .text_color(p.text_subtle)
        .child(t.into())
}

impl ModelPicker {
    fn render_trigger(&self, is_open: bool, st: &Entity<PickerState>, p: &Palette) -> AnyElement {
        let (st, spec, on_open) = (st.clone(), self.spec.clone(), self.on_open.clone());
        let chevron = |size: gpui_kit::Pixels| {
            gpui_kit::svg()
                .path(IconName::ChevronDown.path())
                .flex_none()
                .size(size)
                .text_color(p.text_subtle)
                .when(is_open, |s| {
                    s.with_transformation(Transformation::rotate(radians(std::f32::consts::PI)))
                })
        };
        let base = div()
            .id(self.id.clone())
            .test_support()
            .role(gpui_kit::Role::Button)
            .aria_label("Choose model")
            .aria_expanded(is_open)
            .flex()
            .items_center()
            .when(self.disabled, |el| el.opacity(theme::DISABLED_OPACITY))
            .when(!self.disabled, |el| {
                el.cursor_pointer().on_click(move |_, window, cx| {
                    if st.read(cx).open {
                        close(&st, cx);
                    } else {
                        open(&st, &spec, window, cx);
                        if let Some(f) = &on_open {
                            f(window, cx);
                        }
                    }
                })
            });
        match &self.trigger {
            Trigger::Inline {
                glyph: g,
                label,
                local,
                title,
            } => {
                let hover = (p.wash_strong, p.text_strong);
                with_tip(
                    base.gap(px(6.))
                        .h(size::H_MD)
                        .px(px(8.))
                        // `.mx-btn--ghost.ch-model`: the kit button's 6 px corner.
                        .rounded(radius::SM)
                        .text_color(if is_open { p.text_strong } else { p.text_muted })
                        .when(is_open, |el| el.bg(p.wash_strong))
                        .hover(move |s| s.bg(hover.0).text_color(hover.1))
                        .children(g.as_deref().map(|g| glyph(g, p)))
                        .child(
                            div()
                                .max_w(px(240.))
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .font_family(FONT_MONO)
                                .font_weight(theme::W_MEDIUM)
                                .text_size(text::SM)
                                .child(SharedString::from(label.clone())),
                        )
                        .children(local.map(|l| where_chip(l, p)))
                        .child(chevron(px(12.))),
                    title.clone(),
                )
                .into_any_element()
            }
            Trigger::Block { label, placeholder } => with_tip(
                base.w_full()
                    .justify_between()
                    .gap(px(8.))
                    .h(size::H_MD)
                    .px(space::S3)
                    .rounded(radius::MD)
                    .border_1()
                    .border_color(if is_open {
                        p.accent_border
                    } else {
                        p.border_strong
                    })
                    .bg(p.surface_subtle)
                    .font_family(FONT_MONO)
                    .text_size(text::BASE)
                    .text_color(if *placeholder {
                        p.text_faint
                    } else {
                        p.text_strong
                    })
                    // `.mpm-current`: one line, ellipsis.
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(SharedString::from(label.clone())),
                    )
                    .child(chevron(px(14.))),
                SharedString::from(label.clone()),
            )
            .into_any_element(),
        }
    }

    fn render_row(
        &self,
        row: &Row,
        highlight: usize,
        st: &Entity<PickerState>,
        base: &str,
        p: &Palette,
    ) -> AnyElement {
        let cursor = row.index() == Some(highlight);
        let lit = (p.surface_hover, p.text_strong);
        let hover_to = |el: gpui_kit::Stateful<Div>, ix: usize| {
            let st = st.clone();
            el.on_hover(move |on, _, cx| {
                if *on {
                    st.update(cx, |s, cx| {
                        s.highlight = ix;
                        cx.notify();
                    })
                }
            })
        };
        let click = |row: Row| {
            let (st, sel) = (st.clone(), self.on_select.clone());
            move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| {
                activate(&row, &st, &sel, window, cx)
            }
        };
        match row {
            Row::Header {
                provider,
                label,
                icon: ic,
                hint,
            } => div()
                .flex()
                .items_center()
                .gap(px(6.))
                .mx(px(8.))
                .mt(px(10.))
                .mb(px(4.))
                .font_weight(theme::W_SEMIBOLD)
                .text_size(text::SM)
                .text_color(p.text_muted)
                .when(*ic != HeaderIcon::None, |el| {
                    let path = if *ic == HeaderIcon::Star {
                        IconName::Star.path()
                    } else {
                        "icons/clock.svg".into()
                    };
                    el.child(div().text_color(p.text_subtle).child(icon(path, px(11.))))
                })
                .child(with_tip(
                    div()
                        .id(SharedString::from(format!("{base}-hdr-{label}")))
                        .min_w_0()
                        .truncate()
                        .child(SharedString::from(label.clone())),
                    SharedString::from(label.clone()),
                ))
                .children(
                    provider
                        .as_deref()
                        .map(|id| where_chip(self.spec.is_local(id), p)),
                )
                .when_some(
                    provider
                        .as_deref()
                        .map(|id| {
                            crate::chat::format::provider_via(
                                self.spec.providers.iter().find(|x| x.id == id),
                            )
                        })
                        .or_else(|| hint.clone()),
                    |el, small| {
                        el.child(
                            div()
                                .ml_auto()
                                .font_weight(theme::W_REGULAR)
                                .text_size(text::XS)
                                .text_color(p.text_subtle)
                                .child(small),
                        )
                    },
                )
                .into_any_element(),
            Row::Group {
                provider,
                label,
                count,
                open,
                index,
            } => hover_to(
                with_tip(
                    div()
                        .id(nid(&format!("{base}-grp"), *index as u64))
                        .role(gpui_kit::Role::ListBoxOption)
                        .aria_label(SharedString::from(label.clone()))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .mt(px(2.))
                        .p(px(8.))
                        .rounded(radius::SM)
                        .cursor_pointer()
                        .font_weight(theme::W_SEMIBOLD)
                        .text_size(text::SM)
                        .text_color(if cursor { lit.1 } else { p.text_muted })
                        .when(cursor, |el| el.bg(lit.0))
                        .child(
                            gpui_kit::svg()
                                .path(IconName::ChevronRight.path())
                                .flex_none()
                                .size(px(12.))
                                .text_color(p.text_subtle)
                                .when(*open, |s| {
                                    s.with_transformation(Transformation::rotate(radians(
                                        std::f32::consts::FRAC_PI_2,
                                    )))
                                }),
                        )
                        // A long provider name ends in "…" instead of pushing the chip out.
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .child(SharedString::from(label.clone())),
                        )
                        .child(where_chip(self.spec.is_local(provider), p))
                        .child(
                            div()
                                .ml_auto()
                                .font_family(FONT_MONO)
                                .font_weight(theme::W_REGULAR)
                                .text_size(text::XS)
                                .text_color(p.text_subtle)
                                .child(count.map_or(String::new(), |n| {
                                    format!("{n} model{}", if n == 1 { "" } else { "s" })
                                })),
                        )
                        .on_click(click(row.clone())),
                    SharedString::from(label.clone()),
                ),
                *index,
            )
            .into_any_element(),
            Row::Browse { index, .. } | Row::Back { index } => {
                let (label, detail, back) = match row {
                    Row::Browse { label, detail, .. } => (label.clone(), detail.clone(), false),
                    _ => ("Favorites".to_string(), None, true),
                };
                let el = hover_to(
                    div()
                        .id(nid(&format!("{base}-act"), *index as u64))
                        .role(gpui_kit::Role::ListBoxOption)
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .px(px(8.))
                        .py(px(7.))
                        .rounded(radius::SM)
                        .cursor_pointer()
                        .font_weight(theme::W_MEDIUM)
                        .text_size(text::SM)
                        .text_color(if cursor { lit.1 } else { p.text_muted })
                        .when(cursor, |el| el.bg(lit.0))
                        .when(back, |el| {
                            el.child(icon(IconName::ArrowLeft.path(), px(12.)))
                        })
                        .child(SharedString::from(label))
                        .when_some(detail, |el, d| {
                            el.child(
                                div()
                                    .ml_auto()
                                    .font_family(FONT_MONO)
                                    .font_weight(theme::W_REGULAR)
                                    .text_size(text::XS)
                                    .text_color(p.text_subtle)
                                    .child(d),
                            )
                        })
                        .when(!back, |el| {
                            el.child(icon(IconName::ChevronRight.path(), px(12.)))
                        })
                        .on_click(click(row.clone())),
                    *index,
                );
                div()
                    .flex()
                    .flex_col()
                    .when(!back, |d| {
                        d.child(div().h(px(1.)).mx(px(4.)).my(px(6.)).bg(p.overlay_border))
                    })
                    .child(el)
                    .into_any_element()
            }
            Row::Loading { .. } => div()
                .px(px(8.))
                .pt(px(4.))
                .pb(px(6.))
                .child(skeleton::lines(2, p))
                .into_any_element(),
            Row::Failure { text: t } => {
                let loading = self.spec.loading;
                let retry = self.on_retry.clone().map(|f| {
                    crate::ui::Button::new(
                        ElementId::Name(format!("{base}-retry").into()),
                        if loading { "Retrying…" } else { "Retry" },
                    )
                    .ghost()
                    .inline()
                    .disabled(loading)
                    .on_click(move |_, window, cx| f(window, cx))
                });
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(6.))
                    .px(px(8.))
                    .pt(px(4.))
                    .pb(px(6.))
                    .text_size(text::SM)
                    .text_color(p.warn)
                    .child(icon(IconName::TriangleAlert.path(), px(13.)))
                    // The provider's error wraps inside the 360 px list.
                    .child(div().min_w_0().child(SharedString::from(t.clone())))
                    .children(retry)
                    .into_any_element()
            }
            Row::Note { text: t } => note(t.clone(), p).into_any_element(),
            Row::Option(o) => hover_to(self.option(o, cursor, base, p), o.index)
                .on_click(click(row.clone()))
                .into_any_element(),
        }
    }
}
