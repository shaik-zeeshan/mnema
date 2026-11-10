//! `.mx-choice` (Choice.svelte): options with descriptions instead of radio
//! groups. Tiles in a row by default, `list()` stacks rows. Selected = accent
//! hairline + tint + check badge (tiles) / the wash fill + check (list).
//! ←/→ (and ↑/↓) move the selection, as a radio group does.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, App, Div, ElementId, SharedString, Window};

use super::{focus_handle, focus_visible, icon, inset, mix, nid, outline, Force};
use crate::theme::{self, radius, space, text, Palette, FONT_MONO};

#[derive(Clone)]
pub struct ChoiceOption {
    pub label: SharedString,
    pub description: Option<SharedString>,
    /// Quiet mono note after the title ("default", "on-device · 800 MB").
    pub tag: Option<SharedString>,
    pub icon: Option<SharedString>,
    pub disabled: bool,
}

impl ChoiceOption {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            description: None,
            tag: None,
            icon: None,
            disabled: false,
        }
    }
    pub fn desc(mut self, d: impl Into<SharedString>) -> Self {
        self.description = Some(d.into());
        self
    }
    pub fn tag(mut self, t: impl Into<SharedString>) -> Self {
        self.tag = Some(t.into());
        self
    }
    pub fn icon(mut self, path: impl Into<SharedString>) -> Self {
        self.icon = Some(path.into());
        self
    }
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
}

type OnChange = Rc<dyn Fn(usize, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct Choice {
    id: ElementId,
    aria: SharedString,
    options: Vec<ChoiceOption>,
    selected: usize,
    list: bool,
    dense: bool,
    force: Option<(usize, Force)>,
    on_change: Option<OnChange>,
}

impl Choice {
    pub fn new(
        id: impl Into<ElementId>,
        aria: impl Into<SharedString>,
        options: Vec<ChoiceOption>,
        selected: usize,
    ) -> Self {
        Self {
            id: id.into(),
            aria: aria.into(),
            options,
            selected,
            list: false,
            dense: false,
            force: None,
            on_change: None,
        }
    }
    pub fn list(mut self) -> Self {
        self.list = true;
        self
    }
    /// List rows at 9 px padding with small descriptions (`.ac-scope`).
    pub fn dense(mut self) -> Self {
        self.dense = true;
        self
    }
    pub fn force(mut self, ix: usize, f: Force) -> Self {
        self.force = Some((ix, f));
        self
    }
    pub fn on_change(mut self, f: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }
}

/// The 16 px accent check badge.
fn check(p: &Palette) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(px(16.))
        .rounded_full()
        .bg(p.accent)
        .text_color(p.accent_contrast)
        .child(icon(IconName::Check.path(), px(11.)))
}

/// Next enabled option from `from` in direction `dir` (wrapping), if any.
pub fn next_enabled(options: &[ChoiceOption], from: usize, dir: isize) -> Option<usize> {
    let n = options.len() as isize;
    (1..=n)
        .map(|k| ((from as isize + dir * k).rem_euclid(n)) as usize)
        .find(|&i| !options[i].disabled)
}

impl RenderOnce for Choice {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = theme::tokens(cx);
        let base = self.id.to_string();
        let list = self.list;
        let dense = self.dense;
        let all = Rc::new(self.options.clone());
        let count = self.options.len();
        let opts = self.options.iter().enumerate().map(|(ix, o)| {
            let forced = self
                .force
                .filter(|(f, _)| *f == ix)
                .map_or(Force::None, |(_, f)| f);
            let id = nid(&format!("{base}-opt"), ix as u64);
            let handle = focus_handle(id.clone(), window, cx);
            let on = ix == self.selected;
            let ring = focus_visible(&handle, forced, window);
            let hover = forced == Force::Hover;
            let icon_box = o.icon.clone().map(|path| {
                let (bg, fg, edge) = if on {
                    (p.accent_bg, p.accent, p.accent_border)
                } else {
                    (p.wash, p.text_muted, p.hairline)
                };
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(28.))
                    .when(!list, |d| d.mb(px(6.)))
                    .rounded(radius::MD)
                    .bg(bg)
                    .text_color(fg)
                    .shadow(vec![inset(edge, px(1.))])
                    .child(icon(path, px(15.)))
            });
            let title = div()
                .flex()
                .items_baseline()
                .gap(px(6.))
                .pr(px(18.))
                .font_weight(theme::W_SEMIBOLD)
                .text_size(text::MD)
                .line_height(gpui_kit::relative(1.3))
                .text_color(p.text_strong)
                .child(o.label.clone())
                .when_some(o.tag.clone(), |d, t| {
                    d.child(
                        div()
                            .font_family(FONT_MONO)
                            .font_weight(theme::W_MEDIUM)
                            .text_size(text::XS)
                            .line_height(gpui_kit::relative(1.))
                            .text_color(p.text_subtle)
                            .child(t),
                    )
                });
            let desc = o.description.clone().map(|d| {
                div()
                    .text_size(if dense { text::SM } else { text::BASE })
                    .line_height(gpui_kit::relative(1.45))
                    .text_color(p.text_muted)
                    .child(d)
            });
            let words = div()
                .flex()
                .flex_col()
                .gap(px(if list { 2. } else { 4. }))
                // Row-wise only: in a tile's column, flex-basis 0 clips wrapped text.
                .when(list, |d| d.flex_1())
                .min_w_0()
                .child(title)
                .children(desc);
            let (on_change, opts) = (self.on_change.clone(), all.clone());
            let key_change = self.on_change.clone();
            let hover_border = p.border_hover;
            let wash = p.wash;
            let el = div()
                .id(id)
                .test_support()
                .role(gpui_kit::Role::RadioButton)
                .aria_label(o.label.clone())
                .aria_selected(on)
                .when(!o.disabled, |el| el.track_focus(&handle).cursor_pointer())
                .relative()
                .flex()
                .when(o.disabled, |el| el.opacity(theme::DISABLED_OPACITY));
            let el = if list {
                el.items_center()
                    .gap(space::S3)
                    .pl(px(14.))
                    .pr(px(44.))
                    .py(px(if dense { 9. } else { 11. }))
                    .when(ix > 0, |el| el.border_t_1().border_color(p.hairline))
                    // GPUI clips to a square, so the end rows round their own fill
                    // and focus ring to the stack's inner corner (12 px − 1 px border).
                    .when(ix == 0, |el| el.rounded_t(radius::LG - px(1.)))
                    .when(ix + 1 == count, |el| el.rounded_b(radius::LG - px(1.)))
                    .when(on, |el| el.bg(p.wash_strong))
                    .when(!on && hover, |el| el.bg(p.wash))
                    .when(!on && !o.disabled, |el| el.hover(move |s| s.bg(wash)))
                    .when(ring, |el| el.shadow(vec![inset(p.accent, px(2.))]))
                    .children(icon_box)
                    .child(words)
                    .when(on, |el| {
                        el.child(
                            div()
                                .absolute()
                                .right(px(16.))
                                .top_0()
                                .bottom_0()
                                .flex()
                                .items_center()
                                .child(check(&p)),
                        )
                    })
            } else {
                let border = if on {
                    p.accent_border
                } else if hover {
                    p.border_hover
                } else {
                    p.border
                };
                el.flex_col()
                    .flex_1()
                    .min_w(px(168.))
                    .pt(px(12.))
                    .px(px(14.))
                    .pb(px(14.))
                    .rounded(radius::LG)
                    .bg(if on {
                        mix(p.accent, p.surface, 0.05)
                    } else {
                        p.surface
                    })
                    .border_1()
                    .border_color(border)
                    .when(!on && !o.disabled, |el| {
                        el.hover(move |s| s.border_color(hover_border))
                    })
                    .children(icon_box)
                    .child(words)
                    .when(on, |el| {
                        el.child(check(&p).absolute().top(px(11.)).right(px(11.)))
                    })
                    .when(ring, |el| el.child(outline(p.accent, radius::LG)))
            };
            el.when_some(on_change.filter(|_| !o.disabled), |el, f| {
                el.on_click(move |_, window, cx| f(ix, window, cx))
            })
            .when_some(key_change.filter(|_| !o.disabled), |el, f| {
                el.on_key_down(move |e, window, cx| {
                    let dir = match e.keystroke.key.as_str() {
                        "left" | "up" => -1,
                        "right" | "down" => 1,
                        _ => return,
                    };
                    if let Some(next) = next_enabled(&opts, ix, dir) {
                        cx.stop_propagation();
                        f(next, window, cx);
                    }
                })
            })
        });
        div()
            .id(self.id.clone())
            .test_support()
            .role(gpui_kit::Role::RadioGroup)
            .aria_label(self.aria.clone())
            .flex()
            .when(list, |d| {
                d.flex_col()
                    .border_1()
                    .border_color(p.border)
                    .rounded(radius::LG)
                    .bg(p.surface)
                    .overflow_hidden()
            })
            .when(!list, |d| d.gap(space::S2))
            .children(opts)
    }
}
