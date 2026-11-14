//! Context's writes (`Context.svelte`): add a statement (the draft stays put
//! until the add lands, so a failure keeps it), edit one in place (⌘↵ saves,
//! Cancel / Esc drop the edit), delete one after the native confirm, and restore
//! a dismissed belief. A failed edit or delete says so in Svelte's alert; a
//! failed restore says so on the archive. Every write re-lists.

use gpui_kit::component::input::{InputEvent, TextareaState};
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{Context, Entity, Subscription, Window};

use super::Insights;

/// Per-statement limit (ruling #15); the commands enforce it too.
pub const MAX_CHARS: usize = 280;

#[derive(Default)]
pub struct Ui {
    /// An add is in flight (`Adding…`).
    pub submitting: bool,
    /// The statement being edited, its editor and topic.
    pub editing: Option<i64>,
    pub editor: Option<Entity<TextareaState>>,
    pub edit_topic: Option<&'static str>,
    pub saving: bool,
    /// The dismissed belief being restored: `(subject, statement)`.
    pub restoring: Option<(String, String)>,
    pub dismissed_error: Option<String>,
    /// Subjects' "View dismissed": bring the archive into view once.
    pub show_dismissed: bool,
    _editor_sub: Option<Subscription>,
}

/// `maxlength`: cut a value past the cap (typed or pasted).
pub fn clamp(state: &Entity<TextareaState>, window: &mut Window, cx: &mut gpui_kit::App) {
    let v = state.read(cx).value().to_string();
    if v.chars().count() > MAX_CHARS {
        let cut: String = v.chars().take(MAX_CHARS).collect();
        state.update(cx, |s, cx| s.set_value(cut, window, cx));
    }
}

/// The topic chips' names, in `context-topics.ts` order.
pub const TOPICS: [&str; 4] = ["Work", "Preferences", "People", "Goals"];

/// Svelte's `message(…, { kind: "error" })`; logged when there is no host (tests).
fn alert(title: &str, e: String, cx: &mut gpui_kit::App) {
    log::warn!("insights: {title} ({} chars)", e.len());
    if let Some(core) = crate::chat::core(cx) {
        core.host().message(app_core::MessageDialog {
            title: title.into(),
            message: e,
            kind: app_core::DialogKind::Error,
        });
    }
}

impl Insights {
    /// The composer: add the draft as a standing statement.
    pub fn submit_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).value().trim().to_string();
        if text.is_empty() || self.ctx_ui.submitting {
            return;
        }
        let topic = self.draft_topic;
        self.ctx_ui.submitting = true;
        self.composer_error = None;
        let composer = self.composer.clone();
        let window = window.window_handle();
        self.write(
            cx,
            move |b| b.add_authored(&text, topic),
            move |this, r, cx| {
                this.ctx_ui.submitting = false;
                match r {
                    Ok(_) => {
                        let _ = window.update(cx, |_, window, cx| {
                            composer.update(cx, |s, cx| s.set_value(String::new(), window, cx))
                        });
                        this.load_context(cx);
                        this.reload_status(cx);
                    }
                    Err(e) => this.composer_error = Some(e),
                }
            },
        );
        cx.notify();
    }

    pub fn start_edit(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self
            .context
            .as_ref()
            .and_then(|d| d.authored.as_ref())
            .and_then(|rows| rows.iter().find(|r| r.item.id == id))
        else {
            return;
        };
        let (text, topic) = (row.item.text.clone(), row.item.topic.clone());
        let editor = cx.new(|cx| {
            let mut s = TextareaState::new(window, cx).auto_grow(2, 6);
            s.set_value(text, window, cx);
            s
        });
        editor.update(cx, |s, cx| s.focus(window, cx));
        let sub = cx.subscribe_in(
            &editor,
            window,
            move |this, state, e: &InputEvent, window, cx| {
                match e {
                    InputEvent::PressEnter {
                        secondary: true, ..
                    } => this.save_edit(cx),
                    InputEvent::Change => clamp(state, window, cx),
                    _ => {}
                }
                cx.notify();
            },
        );
        self.ctx_ui.editing = Some(id);
        self.ctx_ui.edit_topic = TOPICS.into_iter().find(|t| Some(*t) == topic.as_deref());
        self.ctx_ui.editor = Some(editor);
        self.ctx_ui._editor_sub = Some(sub);
        cx.notify();
    }

    pub fn cancel_edit(&mut self, cx: &mut Context<Self>) {
        self.ctx_ui.editing = None;
        self.ctx_ui.editor = None;
        self.ctx_ui._editor_sub = None;
        cx.notify();
    }

    pub fn save_edit(&mut self, cx: &mut Context<Self>) {
        let (Some(id), Some(editor)) = (self.ctx_ui.editing, &self.ctx_ui.editor) else {
            return;
        };
        let text = editor.read(cx).value().trim().to_string();
        if text.is_empty() || self.ctx_ui.saving {
            return;
        }
        let topic = self.ctx_ui.edit_topic;
        self.ctx_ui.saving = true;
        self.write(
            cx,
            move |b| b.update_authored(id, &text, topic),
            move |this, r, cx| {
                this.ctx_ui.saving = false;
                match r {
                    Ok(()) => {
                        this.cancel_edit(cx);
                        this.load_context(cx);
                    }
                    Err(e) => alert("Couldn't save context", e, cx),
                }
            },
        );
        cx.notify();
    }

    /// After the native confirm (`Host::confirm`), as the Svelte page does.
    /// Without an `AppCore` (tests, goldens) it deletes straight away.
    pub fn delete_statement(&mut self, id: i64, cx: &mut Context<Self>) {
        let Some(core) = crate::chat::core(cx) else {
            return self.delete_now(id, cx);
        };
        let answer = core.host().confirm(app_core::ConfirmDialog {
            title: Some("Delete context".into()),
            message: "Delete this statement? Mnema stops using it to steer your dossier.".into(),
            kind: app_core::DialogKind::Warning,
            ok_label: "OK".into(),
            cancel_label: "Cancel".into(),
        });
        cx.spawn(async move |this, cx| {
            if answer.await == Ok(true) {
                let _ = this.update(cx, |this, cx| this.delete_now(id, cx));
            }
        })
        .detach();
    }

    fn delete_now(&mut self, id: i64, cx: &mut Context<Self>) {
        self.write(
            cx,
            move |b| b.delete_authored(id),
            move |this, r, cx| match r {
                Ok(()) => {
                    if this.ctx_ui.editing == Some(id) {
                        this.cancel_edit(cx);
                    }
                    this.load_context(cx);
                    this.reload_status(cx);
                }
                Err(e) => alert("Couldn't delete context", e, cx),
            },
        );
    }

    pub fn restore(&mut self, subject: String, statement: String, cx: &mut Context<Self>) {
        if self.ctx_ui.restoring.is_some() {
            return;
        }
        self.ctx_ui.restoring = Some((subject.clone(), statement.clone()));
        self.write(
            cx,
            move |b| b.restore_dismissed(&subject, &statement),
            |this, r, cx| {
                this.ctx_ui.restoring = None;
                match r {
                    Ok(()) => {
                        this.ctx_ui.dismissed_error = None;
                        this.load_context(cx);
                    }
                    Err(e) => this.ctx_ui.dismissed_error = Some(e),
                }
            },
        );
        cx.notify();
    }
}

/// The edit-in-place body (`.in-stmt__edit`): the statement's editor, its
/// topic chips, the count, Cancel and Save (⌘↵).
pub fn edit_body(
    this: &Insights,
    editor: &Entity<TextareaState>,
    p: &crate::theme::Palette,
    window: &Window,
    cx: &mut Context<Insights>,
) -> gpui_kit::Div {
    use crate::theme::{radius, space, text};
    use crate::ui::{nid, Button};
    use gpui_kit::component::input::Textarea;
    use gpui_kit::{div, px, Focusable as _};
    let len = editor.read(cx).value().chars().count();
    let blank = editor.read(cx).value().trim().is_empty();
    let focused = editor.focus_handle(cx).is_focused(window);
    let colors = super::context::topic_colors(p);
    let chips = div()
        .flex()
        .gap(px(4.))
        .children(TOPICS.into_iter().enumerate().map(|(i, name)| {
            let on = this.ctx_ui.edit_topic == Some(name);
            super::charts::swatch_chip(
                nid("insights-edit-topic", i as u64),
                name,
                colors[i],
                on,
                true,
                p,
            )
            .test_support()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.ctx_ui.edit_topic = if this.ctx_ui.edit_topic == Some(name) {
                    None
                } else {
                    Some(name)
                };
                cx.notify();
            }))
        }));
    let save = if this.ctx_ui.saving {
        Button::new("insights-edit-save", "Saving…")
            .primary()
            .sm()
            .spinner(true)
            .disabled(true)
    } else {
        Button::new("insights-edit-save", "Save")
            .primary()
            .sm()
            .kbd("⌘↵")
            .disabled(blank)
            .on_click(cx.listener(|this, _, _, cx| this.save_edit(cx)))
    };
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(space::S3)
        .child(
            // `.mx-input`: hover border, `:focus-visible` accent border + ring.
            div()
                .rounded(radius::MD)
                .border_1()
                .bg(p.surface_subtle)
                .map(|d| {
                    if focused {
                        d.border_color(p.accent_border)
                            .shadow(vec![crate::ui::ring(p.accent_glow)])
                    } else {
                        d.border_color(p.border_strong)
                            .hover(|s| s.border_color(p.border_hover))
                    }
                })
                .text_size(text::LG)
                .line_height(gpui_kit::relative(1.45))
                .text_color(p.text_strong)
                .child(
                    Textarea::new(editor)
                        .aria_label("Edit statement")
                        .appearance(false)
                        .text_size(text::LG)
                        .line_height(gpui_kit::relative(1.45))
                        .min_h(px(64.)),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(space::S2)
                .child(chips)
                .child(div().flex_1())
                .child(super::parts::mono_label(format!("{len} / {MAX_CHARS}"), p))
                .child(
                    Button::new("insights-edit-cancel", "Cancel")
                        .ghost()
                        .sm()
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_edit(cx))),
                )
                .child(save),
        )
}
