//! `Select.svelte`: a 32 px trigger (value or placeholder, chevron that flips
//! open) over a popover list of options (selected = wash + green check).
//! `ActionSelect` is the same control with `placeholder` kept after a pick.
//! Keys: ↑↓ highlight, ↵/Space pick or open, Esc closes.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{
    deferred, div, px, radians, svg, App, ElementId, Entity, SharedString, Transformation, Window,
};

use super::menu::{option_row, options_empty};
use super::{focus_handle, focus_visible, nid, popover, progress, ring, Force, FIELD_RADIUS};
use crate::theme::{self, size, space, text, Palette};

pub type OnPick = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// Open + highlighted row, kept across frames per control.
#[derive(Default, Clone, Copy)]
pub struct ListState {
    pub open: bool,
    pub highlight: Option<usize>,
}

pub fn list_state(base: &str, window: &mut Window, cx: &mut App) -> Entity<ListState> {
    window.use_keyed_state(nid(&format!("{base}-list"), 0), cx, |_, _| {
        ListState::default()
    })
}

/// The shared `.select-trigger` / `.combobox-trigger` look.
pub struct TriggerLook {
    pub open: bool,
    pub focused: bool,
    pub hover: bool,
    pub warn: bool,
    pub busy: bool,
    pub compact: bool,
}

pub fn trigger(
    id: ElementId,
    value: Option<SharedString>,
    placeholder: SharedString,
    look: TriggerLook,
    p: &Palette,
) -> impl super::Control {
    let border = if look.focused {
        p.accent_border
    } else if look.warn {
        p.warn_border
    } else if look.open || look.hover {
        p.border_hover
    } else {
        p.border_strong
    };
    let hover = p.border_hover;
    let chevron = svg()
        .path(IconName::ChevronDown.path())
        .flex_none()
        .size(px(14.))
        .text_color(p.text_subtle)
        .when(look.open, |s| {
            s.with_transformation(Transformation::rotate(radians(std::f32::consts::PI)))
        });
    div()
        .id(id)
        .test_support()
        .role(gpui_kit::Role::ComboBox)
        .aria_expanded(look.open)
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.))
        .w_full()
        .h(if look.compact { size::H_SM } else { size::H_MD })
        .pl(space::S3)
        .pr(px(10.))
        .bg(p.surface_raised)
        .border_1()
        .border_color(border)
        .rounded(FIELD_RADIUS)
        .text_size(text::MD)
        .line_height(gpui_kit::relative(1.))
        .cursor_pointer()
        .when(look.focused, |el| el.shadow(vec![ring(p.accent_glow)]))
        .when(!look.focused && !look.warn, |el| {
            el.hover(move |s| s.border_color(hover))
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_color(if value.is_some() {
                    p.text_strong
                } else {
                    p.text_subtle
                })
                .child(value.unwrap_or(placeholder)),
        )
        .map(|el| {
            if look.busy {
                el.child(
                    div()
                        .text_color(p.text_subtle)
                        .child(progress::spinner(size::SPIN_SM)),
                )
            } else {
                el.child(chevron)
            }
        })
}

/// The `.select-content` list box: raised surface, 4 px padding, 220 px max.
pub fn list_box(id: ElementId, p: &Palette) -> impl super::Control {
    popover::surface(p)
        .id(id)
        .test_support()
        .role(gpui_kit::Role::ListBox)
        .absolute()
        .top_full()
        .mt(px(4.))
        .left_0()
        .right_0()
        .p(px(4.))
        .bg(p.surface_raised)
        .max_h(size::SELECT_MAX_H)
        .overflow_y_scroll()
}

#[derive(IntoElement)]
pub struct Select {
    id: ElementId,
    aria: SharedString,
    options: Vec<SharedString>,
    selected: Option<usize>,
    placeholder: SharedString,
    warn: bool,
    disabled: bool,
    busy: bool,
    compact: bool,
    force: Force,
    force_open: bool,
    on_change: Option<OnPick>,
}

impl Select {
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
            warn: false,
            disabled: false,
            busy: false,
            compact: false,
            force: Force::None,
            force_open: false,
            on_change: None,
        }
    }
    pub fn placeholder(mut self, s: impl Into<SharedString>) -> Self {
        self.placeholder = s.into();
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
    /// In flight (an async ActionSelect pick): 0.6, spinner for the chevron.
    pub fn busy(mut self, on: bool) -> Self {
        self.busy = on;
        self
    }
    /// The 26 px trigger beside dense inline buttons.
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }
    pub fn force(mut self, f: Force) -> Self {
        self.force = f;
        self
    }
    /// Render open (gallery, goldens).
    pub fn force_open(mut self) -> Self {
        self.force_open = true;
        self
    }
    pub fn on_change(mut self, f: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }
}

/// ↑/↓ from `cur` over `n` rows (wrapping), starting at `selected`.
pub fn step_highlight(
    cur: Option<usize>,
    selected: Option<usize>,
    n: usize,
    down: bool,
) -> Option<usize> {
    if n == 0 {
        return None;
    }
    Some(match cur.or(selected) {
        None => {
            if down {
                0
            } else {
                n - 1
            }
        }
        Some(i) if down => (i + 1) % n,
        Some(i) => (i + n - 1) % n,
    })
}

impl RenderOnce for Select {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = theme::tokens(cx);
        let base = self.id.to_string();
        let handle = focus_handle(nid(&format!("{base}-focus"), 0), window, cx);
        let state = list_state(&base, window, cx);
        let ls = *state.read(cx);
        let open = (ls.open || self.force_open) && !self.disabled;
        let live = !self.disabled && !self.busy;
        let look = TriggerLook {
            open,
            focused: focus_visible(&handle, self.force, window),
            hover: self.force == Force::Hover,
            warn: self.warn,
            busy: self.busy,
            compact: self.compact,
        };
        let value = self.selected.and_then(|i| self.options.get(i).cloned());
        let n = self.options.len();
        let pick = self.on_change.clone();
        let selected = self.selected;
        let rows = self.options.iter().enumerate().map(|(i, label)| {
            let (pick, state) = (pick.clone(), state.clone());
            option_row(
                nid(&format!("{base}-opt"), i as u64),
                label.clone(),
                selected == Some(i),
                ls.highlight == Some(i),
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
        let list = open.then(|| {
            let close = state.clone();
            deferred(crate::ui::scroll::area(
                list_box(nid(&format!("{base}-listbox"), 0), &p)
                    .when(n == 0, |el| {
                        el.child(options_empty("Nothing to choose", &p))
                    })
                    .children(rows)
                    .on_mouse_down_out(move |_, _, cx| {
                        close.update(cx, |s, cx| {
                            s.open = false;
                            cx.notify();
                        })
                    }),
            ))
            .with_priority(1)
        });
        let (toggle, keys) = (state.clone(), state.clone());
        let pick_key = self.on_change.clone();
        div()
            .relative()
            .w_full()
            .when(self.disabled, |el| el.opacity(theme::DISABLED_OPACITY))
            .when(self.busy, |el| el.opacity(theme::BUSY_OPACITY))
            .child(
                trigger(self.id.clone(), value, self.placeholder.clone(), look, &p)
                    .aria_label(self.aria.clone())
                    .when(live, |el| {
                        el.track_focus(&handle)
                            .on_click(move |_, _, cx| {
                                toggle.update(cx, |s, cx| {
                                    s.open = !s.open;
                                    s.highlight = None;
                                    cx.notify();
                                })
                            })
                            .on_key_down(move |e, window, cx| {
                                let key = e.keystroke.key.as_str();
                                let mut picked = None;
                                keys.update(cx, |s, cx| {
                                    match key {
                                        "down" | "up" => {
                                            s.open = true;
                                            s.highlight = step_highlight(
                                                s.highlight,
                                                selected,
                                                n,
                                                key == "down",
                                            );
                                        }
                                        "enter" | "space" if s.open => {
                                            picked = s.highlight;
                                            *s = ListState::default();
                                        }
                                        "enter" | "space" => s.open = true,
                                        "escape" => *s = ListState::default(),
                                        _ => return,
                                    }
                                    cx.notify();
                                });
                                cx.stop_propagation();
                                if let (Some(i), Some(f)) = (picked, &pick_key) {
                                    f(i, window, cx);
                                }
                            })
                    }),
            )
            .children(list)
    }
}
