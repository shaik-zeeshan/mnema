//! The titlebar's one door for recall and ask (`.mx-cmd`): Tab flips the mode,
//! ↵ in recall hands the text to Quick Recall, ↵ in ask (or ⌘↵) asks in Chat,
//! Esc drops back to recall and blurs. Hints hide below 1000 px.

use crate::ui::input::FieldText as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, relative, Context, Entity, EventEmitter, FocusHandle, Focusable, Pixels, Role,
    SharedString, Subscription, Window,
};

use crate::keymap::{Dismiss, FlipMode, COMMAND_FIELD_CONTEXT};
use crate::theme::{self, breakpoint, radius, size, space, text, FONT_MONO};
use crate::ui::{icon, kbd, ring};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Recall,
    Ask,
}

impl Mode {
    fn placeholder(self) -> &'static str {
        match self {
            Mode::Recall => "Recall anything…",
            Mode::Ask => "Ask about anything you’ve seen or heard…",
        }
    }
}

/// What the shell does with a submitted query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandFieldEvent {
    Recall(String),
    Ask(String),
}

pub struct CommandField {
    pub mode: Mode,
    input: Entity<InputState>,
    _subscription: Subscription,
}

impl EventEmitter<CommandFieldEvent> for CommandField {}

impl CommandField {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder(Mode::Recall.placeholder()));
        let subscription = cx.subscribe_in(&input, window, |this, _, event, window, cx| {
            if let InputEvent::PressEnter { secondary, .. } = event {
                this.submit(*secondary, window, cx);
            }
        });
        Self {
            mode: Mode::Recall,
            input,
            _subscription: subscription,
        }
    }

    pub fn query(&self, cx: &gpui_kit::App) -> SharedString {
        self.input.read(cx).value()
    }

    /// ⌘K / ⌘J: focus the field in that mode.
    pub fn focus_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        self.set_mode(mode, window, cx);
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    fn set_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = mode;
        self.input.update(cx, |input, cx| {
            input.set_placeholder(mode.placeholder(), window, cx)
        });
        cx.notify();
    }

    fn flip(&mut self, _: &FlipMode, window: &mut Window, cx: &mut Context<Self>) {
        let next = match self.mode {
            Mode::Recall => Mode::Ask,
            Mode::Ask => Mode::Recall,
        };
        self.set_mode(next, window, cx);
    }

    fn dismiss(&mut self, _: &Dismiss, window: &mut Window, cx: &mut Context<Self>) {
        self.set_mode(Mode::Recall, window, cx);
        window.blur(cx);
    }

    fn submit(&mut self, cmd: bool, _window: &mut Window, cx: &mut Context<Self>) {
        let text = self.query(cx).trim().to_string();
        if cmd || self.mode == Mode::Ask {
            cx.emit(CommandFieldEvent::Ask(text));
        } else if !text.is_empty() {
            cx.emit(CommandFieldEvent::Recall(text));
        }
    }
}

impl Focusable for CommandField {
    fn focus_handle(&self, cx: &gpui_kit::App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

/// `clamp(260px, 32vw, 440px)`.
fn field_width(viewport: Pixels) -> Pixels {
    (viewport * 0.32).clamp(size::CMD_MIN_W, size::CMD_MAX_W)
}

impl Render for CommandField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = theme::tokens(cx);
        let focused = self.input.focus_handle(cx).contains_focused(window, cx);
        let ask = self.mode == Mode::Ask;
        let viewport = window.viewport_size().width;

        let (bg, border) = match (focused, ask) {
            (true, true) => (p.surface_subtle, p.accent_border),
            (true, false) => (p.surface_subtle, p.border_hover),
            _ => (p.wash, p.hairline),
        };
        let chip = div()
            .id("command-mode")
            .role(Role::Button)
            .aria_label("Switch between recall and ask (Tab)")
            .flex()
            .flex_none()
            .items_center()
            .gap(px(5.))
            .h(px(20.))
            .px(px(5.))
            .rounded(radius::XS)
            .cursor_pointer()
            .text_size(text::SM)
            .font_weight(theme::W_SEMIBOLD)
            .map(|el| {
                if ask {
                    el.bg(p.accent_bg)
                        .text_color(p.accent)
                        .border_1()
                        .border_color(p.accent_border)
                        .child(icon(IconName::Sparkle.path(), size::ICON_SM))
                        .child("Ask")
                } else {
                    el.text_color(p.text_subtle)
                        .hover(|s| s.bg(p.wash_strong).text_color(p.text))
                        .child(icon(IconName::Search.path(), px(14.)))
                }
            })
            .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                cx.stop_propagation()
            })
            .on_click(cx.listener(|this, _, window, cx| {
                let next = if this.mode == Mode::Ask {
                    Mode::Recall
                } else {
                    Mode::Ask
                };
                this.focus_mode(next, window, cx);
            }));

        let hint = (viewport >= breakpoint::CMD_HINTS).then(|| {
            let row = div()
                .flex()
                .flex_none()
                .items_center()
                .gap(space::S1)
                .pr(px(1.))
                .font_family(FONT_MONO)
                .text_size(text::XS)
                .font_weight(theme::W_MEDIUM)
                .text_color(p.text_subtle);
            match (focused, ask) {
                (false, _) => row.child(kbd("⌘K", &p)),
                (true, false) => row.child(kbd("⇥", &p)).child("to ask"),
                (true, true) => row.child(kbd("↵", &p)).child("ask in chat"),
            }
        });

        div()
            .id("command-field")
            .key_context(COMMAND_FIELD_CONTEXT)
            .on_action(cx.listener(Self::flip))
            .on_action(cx.listener(Self::dismiss))
            .flex()
            .items_center()
            .gap(px(7.))
            .w(field_width(viewport))
            .h(size::CMD_H)
            .px(space::S1)
            .rounded(radius::MD)
            .bg(bg)
            .border_1()
            .border_color(border)
            .when(focused, |el| el.shadow(vec![ring(p.accent_glow)]))
            .when(!focused, |el| el.hover(|s| s.border_color(p.border_strong)))
            .cursor_text()
            .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                cx.stop_propagation()
            })
            .child(chip)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(text::MD)
                    .line_height(relative(1.))
                    .child(
                        Input::new(&self.input)
                            .id("command-input")
                            .aria_label(if ask {
                                "Ask — Tab to recall instead"
                            } else {
                                "Recall anything — Tab to ask instead"
                            })
                            .appearance(false)
                            .px_0()
                            .field_text(text::MD)
                            .h(px(26.)),
                    ),
            )
            .children(hint)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_clamps_to_260_440() {
        assert_eq!(field_width(px(600.)), px(260.));
        assert_eq!(field_width(px(1000.)), px(320.));
        assert_eq!(field_width(px(1600.)), px(440.));
    }
}
