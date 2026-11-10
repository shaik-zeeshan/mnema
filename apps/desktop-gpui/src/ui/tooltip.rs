//! `.mx-tip` (lib/components/tooltip.ts): 350 ms hover dwell, instant on keyboard
//! focus, gone on any scroll, a click, the pointer leaving the window or the
//! window losing key status. Hover rides GPUI's tooltip (positioned at the
//! pointer, so placement differs from the Svelte one above the trigger); focus
//! draws the same body above the trigger.

use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{
    deferred, div, px, AnyView, App, Context, Div, Global, SharedString, Subscription, Window,
};

use crate::theme::{self, leading, radius, size, text, Palette};

/// Bumped by any scroll over a tip owner; a tip built before it renders nothing.
#[derive(Default)]
struct TipEpoch(u64);
impl Global for TipEpoch {}

fn epoch(cx: &App) -> u64 {
    cx.try_global::<TipEpoch>().map_or(0, |e| e.0)
}

/// Hide every visible tooltip (the kit hides on scroll, pointer-down and Esc;
/// GPUI already handles pointer-down and leaving the owner, but not leaving the window).
pub fn hide_all(window: &mut Window, cx: &mut App) {
    cx.default_global::<TipEpoch>().0 += 1;
    window.refresh();
}

/// The tooltip body view GPUI's hover tooltip shows.
pub struct Tip {
    text: SharedString,
    epoch: u64,
    /// Hides it when the window loses key status (GPUI keeps it up).
    _deactivate: Option<Subscription>,
}

impl Tip {
    pub fn new(text: impl Into<SharedString>, cx: &App) -> Self {
        Self {
            text: text.into(),
            epoch: epoch(cx),
            _deactivate: None,
        }
    }
}

impl Render for Tip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.epoch != epoch(cx) {
            return div().id("tooltip-hidden").test_support();
        }
        tip_body(self.text.clone(), &theme::tokens(cx))
            .id("tooltip")
            .test_support()
    }
}

/// The `.mx-tip` box.
pub fn tip_body(text_: SharedString, p: &Palette) -> Div {
    div()
        .max_w(size::TIP_MAX_W)
        .px(px(8.))
        .py(px(5.))
        .rounded(radius::SM)
        .bg(p.overlay_bg_strong)
        .border_1()
        .border_color(p.overlay_border)
        .shadow(super::popover_shadow(p))
        .font_family(theme::FONT_UI)
        .text_size(text::SM)
        .font_weight(theme::W_MEDIUM)
        .line_height(gpui_kit::relative(leading::SNUG))
        .text_color(p.text_strong)
        .child(text_)
}

/// A hover tooltip after the §5 dwell (350 ms), hidden by a scroll over `el` or
/// the pointer leaving the window from it (GPUI only hides on a move inside it).
pub fn with_tip<E: StatefulInteractiveElement>(el: E, tip: impl Into<SharedString>) -> E {
    let tip = tip.into();
    el.tooltip(move |window, cx: &mut App| -> AnyView {
        let tip = Tip::new(tip.clone(), cx);
        cx.new(|cx| Tip {
            _deactivate: Some(cx.observe_window_activation(window, |_, window, cx| {
                if !window.is_window_active() {
                    hide_all(window, cx);
                }
            })),
            ..tip
        })
        .into()
    })
    .tooltip_show_delay(theme::motion::TOOLTIP_DWELL)
    .on_scroll(|_, window, cx| hide_all(window, cx))
    .on_mouse_exit(|_, window, cx| hide_all(window, cx))
}

/// The keyboard-focus tooltip: no dwell, centred above a `relative()` trigger.
/// Render it as a child only while the trigger is focus-visible.
pub fn focus_tip(text_: SharedString, p: &Palette) -> impl IntoElement {
    div()
        .absolute()
        .left(px(-200.))
        .right(px(-200.))
        .bottom_full()
        .mb(px(6.))
        .flex()
        .justify_center()
        .child(deferred(
            tip_body(text_, p)
                .id("tooltip")
                .test_support()
                .whitespace_nowrap(),
        ))
}
