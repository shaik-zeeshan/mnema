//! `Combobox.svelte`: the Select trigger over a list headed by a search field
//! (hairline under it). Typing filters case-insensitively; "No matches" when
//! nothing does. The search text is gpui-kit's input.

use crate::ui::input::FieldText as _;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::prelude::*;
use gpui_kit::{deferred, div, px, App, ElementId, SharedString, Window};

use super::menu::{option_row, options_empty};
use super::select::{list_box, list_state, trigger, ListState, OnPick, TriggerLook};
use super::{focus_handle, focus_visible, icon, nid, Force};
use crate::theme::{self, radius, text};

/// Indices of `options` whose label contains `query` (case-insensitive).
pub fn filter(options: &[SharedString], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    (0..options.len())
        .filter(|&i| q.is_empty() || options[i].to_lowercase().contains(&q))
        .collect()
}

#[derive(IntoElement)]
pub struct Combobox {
    id: ElementId,
    aria: SharedString,
    options: Vec<SharedString>,
    selected: Option<usize>,
    placeholder: SharedString,
    search_placeholder: SharedString,
    warn: bool,
    disabled: bool,
    force: Force,
    force_open: Option<SharedString>,
    on_change: Option<OnPick>,
}

impl Combobox {
    pub fn new(
        id: impl Into<ElementId>,
        aria: impl Into<SharedString>,
        options: impl IntoIterator<Item = impl Into<SharedString>>,
        selected: Option<usize>,
    ) -> Self {
        Self {
            id: id.into(),
            aria: aria.into(),
            options: options.into_iter().map(Into::into).collect(),
            selected,
            placeholder: "Select…".into(),
            search_placeholder: "Search…".into(),
            warn: false,
            disabled: false,
            force: Force::None,
            force_open: None,
            on_change: None,
        }
    }
    pub fn placeholder(mut self, s: impl Into<SharedString>) -> Self {
        self.placeholder = s.into();
        self
    }
    pub fn search_placeholder(mut self, s: impl Into<SharedString>) -> Self {
        self.search_placeholder = s.into();
        self
    }
    pub fn warn(mut self, on: bool) -> Self {
        self.warn = on;
        self
    }
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
    pub fn force(mut self, f: Force) -> Self {
        self.force = f;
        self
    }
    /// Render open with `query` typed (gallery, goldens).
    pub fn force_open(mut self, query: impl Into<SharedString>) -> Self {
        self.force_open = Some(query.into());
        self
    }
    pub fn on_change(mut self, f: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Combobox {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = theme::tokens(cx);
        let base = self.id.to_string();
        let handle = focus_handle(nid(&format!("{base}-focus"), 0), window, cx);
        let state = list_state(&base, window, cx);
        let placeholder = self.search_placeholder.clone();
        let forced_query = self.force_open.clone();
        let search = window.use_keyed_state(nid(&format!("{base}-search"), 0), cx, |window, cx| {
            let s = InputState::new(window, cx).placeholder(placeholder);
            match forced_query {
                Some(q) => s.default_value(q),
                None => s,
            }
        });
        let ls = *state.read(cx);
        let open = (ls.open || self.force_open.is_some()) && !self.disabled;
        let query = search.read(cx).value().to_string();
        let shown = filter(&self.options, &query);
        let look = TriggerLook {
            open,
            focused: focus_visible(&handle, self.force, window),
            hover: self.force == Force::Hover,
            warn: self.warn,
            busy: false,
            compact: false,
        };
        let value = self.selected.and_then(|i| self.options.get(i).cloned());
        let list = open.then(|| {
            let rows = shown.iter().map(|&i| {
                let (pick, state) = (self.on_change.clone(), state.clone());
                option_row(
                    nid(&format!("{base}-opt"), i as u64),
                    self.options[i].clone(),
                    self.selected == Some(i),
                    false,
                    &p,
                )
                .on_click(move |_, window, cx| {
                    state.update(cx, |s, cx| {
                        *s = ListState::default();
                        cx.notify();
                    });
                    if let Some(f) = &pick {
                        f(i, window, cx)
                    }
                })
            });
            let (close, esc) = (state.clone(), state.clone());
            deferred(
                list_box(nid(&format!("{base}-listbox"), 0), &p)
                    .max_h(px(260.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .px(px(9.))
                            .py(px(6.))
                            .mt(px(-4.))
                            .mx(px(-4.))
                            .mb(px(4.))
                            .border_b_1()
                            .border_color(p.hairline)
                            .rounded_t(radius::LG)
                            .child(
                                div()
                                    .text_color(p.text_subtle)
                                    .child(icon(IconName::Search.path(), px(13.))),
                            )
                            .child(
                                div().flex_1().min_w_0().text_size(text::MD).child(
                                    Input::new(&search)
                                        .aria_label(self.aria.clone())
                                        .appearance(false)
                                        .px_0()
                                        .h(px(18.))
                                        .field_text(text::MD),
                                ),
                            ),
                    )
                    .when(shown.is_empty(), |el| {
                        el.child(options_empty("No matches", &p))
                    })
                    .children(rows)
                    .on_key_down(move |e, _, cx| {
                        if e.keystroke.key == "escape" {
                            esc.update(cx, |s, cx| {
                                s.open = false;
                                cx.notify();
                            });
                        }
                    })
                    .on_mouse_down_out(move |_, _, cx| {
                        close.update(cx, |s, cx| {
                            s.open = false;
                            cx.notify();
                        })
                    }),
            )
            .with_priority(1)
        });
        let (toggle, input) = (state.clone(), search.clone());
        div()
            .relative()
            .w_full()
            .when(self.disabled, |el| el.opacity(theme::DISABLED_OPACITY))
            .child(
                trigger(self.id.clone(), value, self.placeholder.clone(), look, &p)
                    .aria_label(self.aria.clone())
                    .when(!self.disabled, |el| {
                        el.track_focus(&handle).on_click(move |_, window, cx| {
                            let now_open = toggle.update(cx, |s, cx| {
                                s.open = !s.open;
                                cx.notify();
                                s.open
                            });
                            if now_open {
                                input.update(cx, |s, cx| s.focus(window, cx));
                            }
                        })
                    }),
            )
            .children(list)
    }
}
