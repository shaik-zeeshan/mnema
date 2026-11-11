//! A copy control that says so: a click writes its text to the clipboard and
//! the control reads "Copied" (label, aria label, tip, Copy → Check icon) for
//! a moment. The flash lives in keyed element state, so the owner keeps no
//! field or timer. Never log what it copies.

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{App, ClipboardItem, ElementId, Pixels, SharedString, Task, Window};

use super::Button;

/// `LogsTab.svelte` / `AnswerProse.svelte`'s revert; sites with their own
/// Svelte timing pass it through [`CopyButton::revert`].
pub const COPIED_FOR: Duration = Duration::from_millis(1200);

#[derive(Default)]
struct Flash {
    on: bool,
    _revert: Option<Task<()>>,
}

type OnCopied = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct CopyButton {
    id: ElementId,
    text: SharedString,
    aria: SharedString,
    label: Option<SharedString>,
    tip: SharedString,
    h: Option<Pixels>,
    ghost: bool,
    revert: Duration,
    on_copied: Option<OnCopied>,
}

impl CopyButton {
    /// The quiet icon-only form: ghost, small, tip "Copy" → "Copied".
    pub fn icon(
        id: impl Into<ElementId>,
        aria: impl Into<SharedString>,
        text: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            aria: aria.into(),
            label: None,
            tip: "Copy".into(),
            h: None,
            ghost: true,
            revert: COPIED_FOR,
            on_copied: None,
        }
    }

    /// A small labelled button whose label reads "Copied" while it flashes.
    pub fn label(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        text: impl Into<SharedString>,
    ) -> Self {
        let label = label.into();
        Self {
            aria: label.clone(),
            label: Some(label),
            ghost: false,
            ..Self::icon(id, "", text)
        }
    }

    pub fn ghost(mut self) -> Self {
        self.ghost = true;
        self
    }

    pub fn aria(mut self, aria: impl Into<SharedString>) -> Self {
        self.aria = aria.into();
        self
    }

    /// The icon form's idle tip (it reads "Copied" while flashing).
    pub fn tip(mut self, tip: impl Into<SharedString>) -> Self {
        self.tip = tip.into();
        self
    }

    /// A height other than `size::H_SM` (a log line's 18 px row).
    pub fn h(mut self, h: Pixels) -> Self {
        self.h = Some(h);
        self
    }

    pub fn revert(mut self, after: Duration) -> Self {
        self.revert = after;
        self
    }

    /// Runs after a copy (an owner that also marks what was copied).
    pub fn on_copied(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_copied = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for CopyButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let flash = window.use_keyed_state(self.id.clone(), cx, |_, _| Flash::default());
        let copied = flash.read(cx).on;
        let button = match &self.label {
            Some(label) => Button::new(
                self.id.clone(),
                if copied {
                    "Copied".into()
                } else {
                    label.clone()
                },
            )
            .aria(if copied {
                "Copied".into()
            } else {
                self.aria.clone()
            }),
            None => Button::icon_only(
                self.id.clone(),
                if copied {
                    IconName::Check.path()
                } else {
                    IconName::Copy.path()
                },
                if copied {
                    "Copied".into()
                } else {
                    self.aria.clone()
                },
            )
            .tip(if copied {
                "Copied".into()
            } else {
                self.tip.clone()
            }),
        };
        let (text, revert, then) = (self.text, self.revert, self.on_copied);
        button
            .sm()
            .when(self.ghost, |b| b.ghost())
            .when_some(self.h, |b, h| b.h(h))
            .disabled(text.is_empty())
            .on_click(move |_, window, cx| {
                // Inside a clickable row, the copy is the whole click.
                cx.stop_propagation();
                cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
                flash.update(cx, |f, cx| {
                    f.on = true;
                    // A second click restarts the revert rather than racing the first.
                    f._revert = Some(cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(revert).await;
                        let _ = this.update(cx, |f, cx| {
                            f.on = false;
                            cx.notify();
                        });
                    }));
                    cx.notify();
                });
                if let Some(f) = &then {
                    f(window, cx);
                }
            })
    }
}

/// `AnswerProse.svelte`'s code-block header for a markdown `TextView`
/// (`.code_block_actions(copy::code_actions)`): the language, then Copy. The
/// kit scopes the `"copy"` id per block.
pub fn code_actions(
    block: &gpui_kit::base::text::CodeBlock,
    _: &mut Window,
    cx: &mut App,
) -> impl IntoElement {
    let p = crate::theme::tokens(cx);
    gpui_kit::div()
        .flex()
        .items_center()
        .gap(gpui_kit::px(4.))
        .pl(gpui_kit::px(6.))
        .text_size(crate::theme::text::XS)
        .text_color(p.text_subtle)
        .children(block.lang().filter(|l| !l.is_empty()))
        .child(
            CopyButton::label("copy", "Copy", block.code())
                .ghost()
                .aria("Copy code")
                .h(gpui_kit::px(22.)),
        )
}
