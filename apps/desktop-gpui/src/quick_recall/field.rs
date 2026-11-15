//! The field row (`Field.svelte`): the mode chip (search ⇄ Ask, "auto" while a
//! detected question answers on its own), the pills, the text with recognised
//! phrases tinted in place (a mirror under a transparent input) and the ghost,
//! the right-hand status; plus the foot's operator query and live keys.

use crate::ui::input::FieldText as _;
use crate::ui::Tracking as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::input::Input;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{
    div, px, transparent_black, App, Context, Div, HighlightStyle, Role, SharedString, StyledText,
    UnderlineStyle, Window,
};

use super::tokens::{mirror_segments, pill_label, pill_operators, Pill, Seg};
use super::view::tipped;
use super::{EmptyState, Mode, QuickRecall};
use crate::theme::{self, radius, size, space, text, Palette, FONT_MONO};
use crate::ui::app_icon::{app_icon, Src};
use crate::ui::{alpha, icon, kbd, mix, progress};

/// The field's type: 19 px / 34 px line.
const FIELD_TEXT: f32 = 19.;
const FIELD_LINE: f32 = 34.;

impl QuickRecall {
    /// An app's icon, or its initial on a tile: the field pill's wash tile
    /// (`tint: None`), or the 18 px row / citation tile (`.rc-app` in
    /// `SearchResultCard` / `AskPane`): the letter in `tint`, an 18% fill of it
    /// over the surface and a 30% inner ring.
    pub(super) fn app_glyph(
        &self,
        id: Option<&str>,
        name: &str,
        s: f32,
        tint: Option<gpui_kit::Hsla>,
        p: &Palette,
    ) -> gpui_kit::AnyElement {
        let path = id
            .and_then(|i| self.icons.get(i))
            .or_else(|| self.icons.get(name));
        match path {
            Some(path) => app_icon(Src::File(path.clone()), s).into_any_element(),
            None => div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(s))
                .font_weight(gpui_kit::FontWeight::BOLD)
                .map(|d| match tint {
                    None => d
                        .rounded(px(4.))
                        .bg(p.wash_strong)
                        .text_color(p.text_strong)
                        .text_size(px(s * 0.57)),
                    Some(c) => d
                        .rounded(radius::BTN_SM)
                        .bg(crate::ui::mix(c, p.surface, 0.18))
                        .shadow(vec![crate::ui::inset(crate::ui::alpha(c, 0.3), px(1.))])
                        .text_color(c)
                        .text_size(text::XS),
                })
                .child(
                    name.chars()
                        .next()
                        .map(|c| c.to_uppercase().to_string())
                        .unwrap_or_default(),
                )
                .into_any_element(),
        }
    }

    fn pill(&self, i: usize, pill: &Pill, cx: &mut Context<Self>, p: &Palette) -> impl IntoElement {
        let pressed = self.filters.pill == Some(i);
        let lead = match pill {
            Pill::App(a) => self.app_glyph(None, a, 15., None, p),
            Pill::Date { .. } => div()
                .text_color(p.text_subtle)
                .child(icon(IconName::Calendar.path(), size::ICON_SM))
                .into_any_element(),
            Pill::Source(s) => {
                let c = match s {
                    super::tokens::Src::Mic => p.source_mic,
                    super::tokens::Src::System => p.source_sysaudio,
                    super::tokens::Src::Screen => p.source_screen,
                };
                div()
                    .flex_none()
                    .size(px(7.))
                    .rounded(px(2.))
                    .bg(c)
                    .into_any_element()
            }
        };
        let label = pill_label(pill);
        div()
            .id(("qr-pill", i))
            .role(Role::Button)
            .aria_label(SharedString::from(format!(
                "{label} — {}. Click to change",
                pill_operators(pill)
            )))
            .aria_toggled(if pressed {
                gpui_kit::Toggled::True
            } else {
                gpui_kit::Toggled::False
            })
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .h(size::H_SM)
            .max_w(px(220.))
            .px(px(9.))
            .rounded(radius::BTN_SM)
            .border_1()
            .border_color(if pressed {
                p.border_hover
            } else {
                p.border_strong
            })
            .bg(if pressed {
                p.wash_strong
            } else {
                p.surface_raised
            })
            .text_size(text::BASE)
            .font_weight(theme::W_MEDIUM)
            .text_color(p.text_strong)
            .cursor_pointer()
            .hover(|s| s.bg(p.surface_hover))
            // The full label, but not over this pill's own open popover.
            .when(!(self.filters.popover && pressed), |d| {
                crate::ui::with_tip(d, label.clone())
            })
            .child(lead)
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(label),
            )
            .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                cx.stop_propagation()
            })
            .on_click(cx.listener(move |qr, _, window, cx| {
                if qr.filters.popover && qr.filters.pill == Some(i) {
                    qr.select_pill(None);
                } else {
                    qr.open_popover(i, cx);
                }
                qr.input.update(cx, |input, cx| input.focus(window, cx));
                qr.refresh(cx);
            }))
    }

    /// The mirror under the input: tinted phrases, the rejected token, the ghost.
    fn mirror(&self, cx: &App, p: &Palette) -> StyledText {
        let bad = self.bad_token();
        let mut full = String::new();
        let mut hl: Vec<(std::ops::Range<usize>, HighlightStyle)> = Vec::new();
        for (t, k) in mirror_segments(&self.text, &self.parse.spans, bad.as_deref()) {
            let start = full.len();
            full.push_str(&t);
            let style = match k {
                Seg::Plain => continue,
                Seg::Live => HighlightStyle {
                    background_color: Some(p.wash_strong),
                    ..Default::default()
                },
                Seg::Bad => HighlightStyle {
                    color: Some(p.danger_text),
                    underline: Some(UnderlineStyle {
                        thickness: px(1.),
                        color: Some(p.danger_border),
                        wavy: true,
                    }),
                    ..Default::default()
                },
            };
            hl.push((start..full.len(), style));
        }
        if full.is_empty() && self.pills.is_empty() {
            let ph = "Recall anything you saw or heard…";
            return StyledText::new(ph).with_highlights([(
                0..ph.len(),
                HighlightStyle {
                    color: Some(p.text_subtle),
                    ..Default::default()
                },
            )]);
        }
        if let Some(g) = self.ghost() {
            let start = full.len();
            full.push_str(&g.shown);
            hl.push((
                start..full.len(),
                HighlightStyle {
                    color: Some(alpha(p.text_subtle, 0.7)),
                    ..Default::default()
                },
            ));
        }
        let _ = cx;
        StyledText::new(full).with_highlights(hl)
    }

    pub(super) fn field_row(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> Div {
        let mode = self.mode();
        let ask = mode == Mode::Ask;
        let auto = ask && self.forced.is_none();
        let chip = div()
            .id("qr-mode")
            .test_support()
            .role(Role::Button)
            .aria_label(if ask {
                "Asking — Tab to search"
            } else {
                "Searching — Tab to ask"
            })
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .h(px(30.))
            .min_w(px(30.))
            .px(px(7.))
            .rounded(radius::MD)
            .text_size(text::MD)
            .font_weight(theme::W_SEMIBOLD)
            .cursor_pointer()
            .map(|c| {
                if ask {
                    c.bg(p.accent_bg)
                        .text_color(p.accent)
                        .border_1()
                        .border_color(p.accent_border)
                        .child(icon(IconName::Sparkle.path(), px(16.)))
                        .child("Ask")
                        .when(auto, |c| {
                            c.child(
                                div()
                                    .font_family(FONT_MONO)
                                    .text_size(text::XS)
                                    .font_weight(theme::W_MEDIUM)
                                    .text_color(mix(p.accent, p.text_subtle, 0.7))
                                    .child("auto"),
                            )
                        })
                } else {
                    c.text_color(p.text_subtle)
                        .hover(|s| s.bg(p.wash_strong).text_color(p.text))
                        .child(icon(IconName::Search.path(), px(17.)))
                }
            })
            .on_click(cx.listener(|qr, _, window, cx| {
                qr.toggle_mode(cx);
                qr.input.update(cx, |i, cx| i.focus(window, cx));
            }));
        let pills: Vec<_> = self
            .pills
            .clone()
            .iter()
            .enumerate()
            .map(|(i, pl)| self.pill(i, pl, cx, p))
            .collect();
        let scroll_x = self.input.read(cx).scroll_offset().x;
        let field = div()
            .id("qr-field")
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap(px(6.))
            .overflow_hidden()
            .children(pills)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w(px(120.))
                    .h(px(FIELD_LINE))
                    .overflow_hidden()
                    .text_size(px(FIELD_TEXT))
                    // `.rc-mirror`; the transparent input over it inherits the same.
                    .tracking(-0.012)
                    .line_height(px(FIELD_LINE))
                    .text_color(p.text_strong)
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left(scroll_x)
                            .h(px(FIELD_LINE))
                            .flex()
                            .items_center()
                            .whitespace_nowrap()
                            .child(self.mirror(cx, p)),
                    )
                    .child(
                        Input::new(&self.input)
                            .id("qr-input")
                            .aria_label("Recall anything — Tab to ask")
                            .appearance(false)
                            .px_0()
                            .field_h(px(FIELD_LINE))
                            .field_text(px(FIELD_TEXT))
                            .text_color(transparent_black()),
                    ),
            );
        let _ = window;
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .pl(px(13.))
            .pr(px(14.))
            .child(chip)
            .child(field)
            .child(self.right_status(p))
    }

    fn right_status(&self, p: &Palette) -> Div {
        let row = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(space::S3)
            .font_family(FONT_MONO)
            .text_size(text::SM)
            .text_color(p.text_subtle)
            .whitespace_nowrap();
        let pair = |k: &str, l: &str| {
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .child(kbd(k.to_string(), p))
                .child(l.to_string())
        };
        if let (Some(_), Some((i, n))) = (&self.look, self.look_index()) {
            return row
                .child(
                    div()
                        .flex()
                        .gap(px(4.))
                        .child(div().text_color(p.text).child(format!("{}", i + 1)))
                        .child(format!("of {n}")),
                )
                .child(pair("esc", "back"));
        }
        if self.mode() == Mode::Ask {
            return row.child(pair("⇥", "search"));
        }
        if self.results.loading {
            return row.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .child(progress::spinner(size::SPIN_SM))
                    .child("searching…"),
            );
        }
        if self.show_recent() {
            return row.child(pair("esc", "close"));
        }
        if self.is_empty() {
            return row.child(pair("↓", "recent"));
        }
        let total = self.results.ranked.len();
        row.when(total > 0, |r| {
            let capped = self.results.capped();
            let s = if total == 1 && !capped { "" } else { "s" };
            r.child(
                div()
                    .flex()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_color(p.text)
                            .child(super::rules::count_label(total, capped)),
                    )
                    .child(format!("moment{s}")),
            )
        })
        .when(self.ask_available(), |r| r.child(pair("⇥", "ask")))
    }

    /// Keys for search mode, most specific surface first.
    pub(super) fn search_keys(&self) -> Vec<(&'static str, &'static str)> {
        if self.filters.popover {
            return vec![("↑↓", "pick"), ("↵", "apply"), ("esc", "close")];
        }
        if self.filters.pill.is_some() {
            return vec![("↵", "change"), ("⌫", "remove"), ("esc", "back")];
        }
        if let Some(k) = self.look_keys() {
            return k;
        }
        if self.show_recent() {
            let mut k = vec![("↑↓", "pick"), ("↵", "use")];
            if self.recent.index < self.recent.list.len() {
                k.push(("⌫", "remove"));
            }
            return k;
        }
        match self.empty_state() {
            Some(EmptyState::None) => return vec![("↵", "apply"), ("esc", "clear")],
            Some(EmptyState::Error) => return vec![("↵", "retry"), ("esc", "clear")],
            _ => {}
        }
        let mut k = Vec::new();
        if self.ghost().is_some() {
            k.push(("→", "complete"));
        }
        if !self.visible_results().is_empty() && !self.results.loading {
            k.push(("↵", "open"));
            k.push((if self.look_nav { "space" } else { "↓ space" }, "look"));
            if self
                .results
                .selected
                .and_then(|i| self.visible_results().get(i))
                .and_then(|r| r.page_frame())
                .is_some()
            {
                k.push(("⌘↵", "page"));
            }
            k.push(("⌘C", "copy"));
            return k;
        }
        k.push(("esc", "clear"));
        k
    }

    /// The foot: the real operator query (or the hint), then the live keys.
    pub(super) fn field_foot(
        &self,
        keys: Vec<(&'static str, &'static str)>,
        cx: &App,
        p: &Palette,
    ) -> Div {
        let q = self.operator_query();
        let query = if q.is_empty() {
            div().min_w_0().overflow_hidden().text_ellipsis().child(
                "Type naturally — “in Arc”, “yesterday afternoon”, “on a call” — or a question.",
            )
        } else {
            let residual = self.parse.residual.clone();
            let ops: Vec<String> = self.effective_pills().iter().map(pill_operators).collect();
            let mut full = residual.clone();
            for o in &ops {
                full.push(' ');
                full.push_str(o);
            }
            let full = full.trim_start().to_string();
            let mut hl = vec![];
            if !residual.is_empty() {
                hl.push((
                    0..residual.len(),
                    HighlightStyle {
                        color: Some(p.text),
                        ..Default::default()
                    },
                ));
                if let Some(bad) = self.bad_token() {
                    if let Some(at) = residual.find(&bad) {
                        hl[0] = (
                            0..at,
                            HighlightStyle {
                                color: Some(p.text),
                                ..Default::default()
                            },
                        );
                        hl.push((
                            at..at + bad.len(),
                            HighlightStyle {
                                color: Some(p.danger_text),
                                ..Default::default()
                            },
                        ));
                        hl.push((
                            at + bad.len()..residual.len(),
                            HighlightStyle {
                                color: Some(p.text),
                                ..Default::default()
                            },
                        ));
                        hl.retain(|h| !h.0.is_empty());
                    }
                }
            }
            div()
                .flex()
                .min_w_0()
                .items_center()
                .gap(space::S2)
                .overflow_hidden()
                .child(icon(IconName::Code.path(), size::ICON_SM))
                .child(
                    tipped("qr-foot-query", &full)
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .font_family(FONT_MONO)
                        .text_color(p.text_muted)
                        .child(StyledText::new(full).with_highlights(hl)),
                )
        };
        let _ = cx;
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(space::S4)
            .px(px(14.))
            .border_t_1()
            .border_color(p.hairline)
            // Inside the shell's 1 px border: its fill must not cross the rounded corners.
            .rounded_b(px(super::CORNER - 1.))
            .bg(alpha(p.surface, 0.55))
            .text_size(text::SM)
            .text_color(p.text_subtle)
            .whitespace_nowrap()
            .child(query.flex_1())
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(space::S3)
                    .children(keys.into_iter().map(|(k, l)| {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .child(kbd(k, p).h(px(17.)).min_w(px(17.)).px(px(4.)))
                            .child(l)
                    })),
            )
    }
}
