//! `.mx-input` around gpui-kit's text input (editing, IME, selection stay the
//! kit's), plus `FieldWarning`. `num()` = mono digits. `invalid()` = danger border.

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, App, Div, ElementId, Entity, Focusable, Pixels, SharedString, Window};

use super::{ring, Force, FIELD_RADIUS};
use crate::theme::{self, size, space, text, Palette, FONT_MONO};

/// A single-line field's line box, as a multiple of its text size.
const CARET_LINE: f32 = 1.5;

/// Every single-line `Input` sets its text size through this. kit's `Input`
/// pins its line box to 1.25 rem whatever the text size, and GPUI draws the
/// caret 0.85 × that line (1 px wide): a 13.6 px stub in a 19 px field, sat
/// above the text. WebKit's caret is as tall as the font (~1.27 × its size),
/// which a 1.5 × line gives.
pub trait FieldText {
    fn field_text(self, size: Pixels) -> Self;
    /// The field's height. kit's own `Input::h` is multi-line only: on a
    /// single-line field it does nothing and the frame stays 2 rem tall.
    fn field_h(self, h: Pixels) -> Self;
}

impl FieldText for Input {
    fn field_text(self, size: Pixels) -> Self {
        self.text_size(size).line_height(size * CARET_LINE)
    }

    fn field_h(self, h: Pixels) -> Self {
        Styled::h(self, h)
    }
}

#[derive(IntoElement)]
pub struct TextInput {
    id: ElementId,
    aria: SharedString,
    state: Entity<InputState>,
    num: bool,
    center: bool,
    invalid: bool,
    disabled: bool,
    force: Force,
    width: Option<Pixels>,
    pad_right: Option<Pixels>,
}

impl TextInput {
    pub fn new(
        id: impl Into<ElementId>,
        aria: impl Into<SharedString>,
        state: &Entity<InputState>,
    ) -> Self {
        Self {
            id: id.into(),
            aria: aria.into(),
            state: state.clone(),
            num: false,
            center: false,
            invalid: false,
            disabled: false,
            force: Force::None,
            width: None,
            pad_right: None,
        }
    }
    pub fn num(mut self) -> Self {
        self.num = true;
        self
    }
    pub fn center(mut self) -> Self {
        self.center = true;
        self
    }
    pub fn invalid(mut self, on: bool) -> Self {
        self.invalid = on;
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
    pub fn w(mut self, w: Pixels) -> Self {
        self.width = Some(w);
        self
    }
    /// Room on the right for an overlaid suffix (the stepper's unit chip).
    pub fn pad_right(mut self, w: Pixels) -> Self {
        self.pad_right = Some(w);
        self
    }
}

impl RenderOnce for TextInput {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = theme::tokens(cx);
        let focused =
            self.force == Force::Focus || self.state.read(cx).focus_handle(cx).is_focused(window);
        let border = if self.invalid {
            p.danger_border
        } else if focused {
            p.accent_border
        } else if self.force == Force::Hover {
            p.border_hover
        } else {
            p.border_strong
        };
        let hover = p.border_hover;
        let live = !self.disabled && !focused && !self.invalid;
        let (family, fs) = if self.num {
            (FONT_MONO, text::BASE)
        } else {
            (theme::FONT_UI, text::MD)
        };
        div()
            .id(self.id)
            .test_support()
            .flex()
            .items_center()
            .when_some(self.width, |el, w| el.w(w))
            .when(self.width.is_none(), |el| el.w_full())
            .h(size::H_MD)
            .pl(space::S3)
            .pr(self.pad_right.unwrap_or(space::S3))
            .rounded(FIELD_RADIUS)
            .border_1()
            .border_color(border)
            .bg(p.surface_subtle)
            .text_color(p.text_strong)
            .font_family(family)
            .text_size(fs)
            .when(focused, |el| el.shadow(vec![ring(p.accent_glow)]))
            .when(live, |el| el.hover(move |s| s.border_color(hover)))
            .when(self.disabled, |el| el.opacity(theme::DISABLED_OPACITY))
            .child(
                Input::new(&self.state)
                    .aria_label(self.aria)
                    .appearance(false)
                    .disabled(self.disabled)
                    .px_0()
                    .field_text(fs)
                    .field_h(size::H_MD - px(2.))
                    .when(self.center, |el| el.text_center()),
            )
    }
}

/// `FieldWarning.svelte`: a danger line under a field, with a round `!` badge.
pub fn field_warning(msg: impl Into<SharedString>, p: &Palette) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(6.))
        .text_color(p.danger)
        .text_size(text::XS)
        .line_height(gpui_kit::relative(1.45))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .mt(px(0.5))
                .size(px(16.))
                .rounded_full()
                .border_1()
                .border_color(p.danger_border)
                .bg(p.danger_bg)
                .font_weight(gpui_kit::FontWeight::EXTRA_BOLD)
                .line_height(gpui_kit::relative(1.))
                .child("!"),
        )
        .child(div().min_w_0().pt(px(1.)).child(msg.into()))
}
