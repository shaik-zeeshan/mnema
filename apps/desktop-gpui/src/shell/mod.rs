//! The main window: titlebar, the current surface, the status bar, and the three
//! popovers. Holds view state only; `data.rs` fills [`ShellModel`] from AppCore.

pub mod command_field;
pub mod data;
#[cfg(feature = "fixtures")]
pub mod fixture;
pub mod model;
mod notifications_pop;
mod placeholder;
mod shortcuts_pop;
mod status_bar;
mod status_pop;
mod surface_pill;
mod titlebar;

use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, Context, Entity, FocusHandle, Subscription, Task, Window};

use crate::keymap::{self, Bindings, HelpState};
use crate::theme::{self, text, FONT_UI};
use command_field::{CommandField, CommandFieldEvent, Mode};
pub use model::{ShellModel, Surface};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlay {
    Notifications,
    Shortcuts,
    Status,
}

pub struct Shell {
    pub model: ShellModel,
    pub surface: Surface,
    /// Where the surface-pill ink slides from.
    ink_from: Surface,
    /// Timeline or Insights: where the gear returns from Settings.
    last_main: Surface,
    pub overlay: Option<Overlay>,
    pub bindings: Bindings,
    /// The question handed to Chat (`/chat?q=`).
    pub chat_query: Option<String>,
    pub command: Entity<CommandField>,
    pub timeline: Entity<crate::timeline::TimelineView>,
    pub chat: Entity<crate::chat::Chat>,
    pub settings: Entity<crate::settings::Settings>,
    focus: FocusHandle,
    _subscriptions: [Subscription; 2],
    tick: Option<Task<()>>,
}

impl Shell {
    pub fn new(model: ShellModel, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let source = std::sync::Arc::new(crate::timeline::data::CoreSource);
        Self::with_timeline_source(model, source, window, cx)
    }

    /// The shell over a given Timeline source (goldens and tests pass the parity fixture).
    pub fn with_timeline_source(
        model: ShellModel,
        source: std::sync::Arc<dyn crate::timeline::data::Source>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let command = cx.new(|cx| CommandField::new(window, cx));
        let timeline = cx.new(|cx| crate::timeline::TimelineView::new(source, cx));
        let chat = cx.new(|cx| crate::chat::Chat::new(window, cx));
        let settings = cx.new(|cx| crate::settings::Settings::new(window, cx));
        let subscriptions = [
            cx.subscribe_in(&command, window, Self::on_command),
            // System appearance follows the OS (theme::set_system ignores it when pinned).
            cx.observe_window_appearance(window, |_, window, cx| {
                theme::set_system(window.appearance(), cx)
            }),
        ];
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let mut shell = Self {
            model: ShellModel::default(),
            surface: Surface::Timeline,
            ink_from: Surface::Timeline,
            last_main: Surface::Timeline,
            overlay: None,
            bindings: Bindings::default(),
            chat_query: None,
            command,
            timeline,
            chat,
            settings,
            focus,
            _subscriptions: subscriptions,
            tick: None,
        };
        shell.set_model(model, cx);
        shell
    }

    pub fn set_model(&mut self, model: ShellModel, cx: &mut Context<Self>) {
        self.model = model;
        let s = &self.model.status;
        let hint = crate::timeline::CaptureHint {
            capturing: s.running,
            screen: s.selected[0],
        };
        let developer = self.model.developer_options;
        let status = self.model.status.clone();
        self.settings.update(cx, |s, cx| {
            s.status = status;
            cx.notify();
        });
        self.timeline.update(cx, |t, cx| {
            t.capture = hint;
            t.developer = developer;
            cx.notify();
        });
        // The REC clock ticks once a second while it shows.
        // shortcut: no render-idle gate yet (hidden window still ticks); S7's
        // window visibility tracking pauses it.
        let ticking = model::rec_view(&self.model.status).elapsed_clock;
        self.tick = ticking.then(|| {
            cx.spawn(async move |this, cx| loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            })
        });
        cx.notify();
    }

    pub fn go(&mut self, surface: Surface, cx: &mut Context<Self>) {
        if surface == self.surface {
            return;
        }
        self.ink_from = self.surface;
        if matches!(surface, Surface::Timeline | Surface::Insights) {
            self.last_main = surface;
        }
        self.surface = surface;
        self.overlay = None;
        if surface == Surface::Settings {
            self.settings.update(cx, |s, cx| s.reload_all(cx));
        }
        cx.notify();
    }

    pub fn toggle_overlay(&mut self, overlay: Overlay, cx: &mut Context<Self>) {
        self.overlay = (self.overlay != Some(overlay)).then_some(overlay);
        cx.notify();
    }

    pub fn help_state(&self) -> HelpState {
        let s = &self.model.status;
        HelpState {
            capturing: s.running,
            user_paused: s.user_paused,
            can_toggle_recording: !s.read_only,
            can_toggle_sources: !s.running && !s.read_only,
            developer_options: self.model.developer_options,
        }
    }

    fn on_command(
        &mut self,
        _: &Entity<CommandField>,
        event: &CommandFieldEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            CommandFieldEvent::Ask(text) => {
                self.chat_query = (!text.is_empty()).then(|| text.clone());
                // `/chat?q=`: a new chat that sends once (if Ask AI can run).
                let q = self.chat_query.clone();
                self.chat
                    .update(cx, |chat, cx| chat.new_chat(q, true, window, cx));
                self.go(Surface::Chat, cx);
            }
            // shortcut: Quick Recall is S10; until then ↵ in recall keeps the text.
            CommandFieldEvent::Recall(_) => {
                log::debug!("command field: recall submitted (Quick Recall lands in S10)")
            }
        }
    }

    fn focus_field(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        if self.surface == Surface::Settings {
            return; // no titlebar on Settings
        }
        self.command
            .update(cx, |field, cx| field.focus_mode(mode, window, cx));
    }

    fn toggle_source(&mut self, i: usize, cx: &mut Context<Self>) {
        let s = &mut self.model.status;
        if s.running || s.read_only {
            return;
        }
        // shortcut: persisting the selection needs recording settings in app-core (S2).
        s.selected[i] = !s.selected[i];
        cx.notify();
    }

    fn register_actions(
        &self,
        el: gpui_kit::Stateful<gpui_kit::Div>,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        el.on_action(
            cx.listener(|this, _: &keymap::FocusRecall, w, cx| {
                this.focus_field(Mode::Recall, w, cx)
            }),
        )
        .on_action(cx.listener(|this, _: &keymap::FocusAsk, w, cx| {
            // ⌘J on Chat = a new chat in place; elsewhere it focuses Ask.
            if this.surface == Surface::Chat {
                this.chat
                    .update(cx, |chat, cx| chat.new_chat(None, false, w, cx));
            } else {
                this.focus_field(Mode::Ask, w, cx)
            }
        }))
        .on_action(
            cx.listener(|this, _: &keymap::GoTimeline, _, cx| this.go(Surface::Timeline, cx)),
        )
        .on_action(
            cx.listener(|this, _: &keymap::GoInsights, _, cx| this.go(Surface::Insights, cx)),
        )
        .on_action(cx.listener(|this, _: &keymap::OpenChats, _, cx| this.go(Surface::Chat, cx)))
        .on_action(cx.listener(|this, _: &keymap::ToggleSettings, _, cx| {
            let to = if this.surface == Surface::Settings {
                this.last_main
            } else {
                Surface::Settings
            };
            this.go(to, cx);
        }))
        .on_action(cx.listener(|this, _: &keymap::ToggleShortcutsHelp, w, cx| {
            // `/` on Settings focuses its search instead.
            if this.surface == Surface::Settings {
                let search = this.settings.read(cx).inputs.search.clone();
                search.update(cx, |i, cx| i.focus(w, cx));
            } else {
                this.toggle_overlay(Overlay::Shortcuts, cx)
            }
        }))
        .on_action(cx.listener(|this, _: &keymap::CloseOverlay, _, cx| {
            if this.overlay.is_some() {
                this.overlay = None;
                cx.notify();
            } else if this.surface == Surface::Settings {
                this.go(this.last_main, cx);
            } else if this.surface == Surface::Timeline
                && this.timeline.update(cx, |t, cx| t.close_top(cx))
            {
            } else {
                cx.propagate();
            }
        }))
        .on_action(
            cx.listener(|this, _: &keymap::ToggleSourceScreen, _, cx| this.toggle_source(0, cx)),
        )
        .on_action(
            cx.listener(|this, _: &keymap::ToggleSourceMicrophone, _, cx| {
                this.toggle_source(1, cx)
            }),
        )
        .on_action(
            cx.listener(|this, _: &keymap::ToggleSourceSystemAudio, _, cx| {
                this.toggle_source(2, cx)
            }),
        )
        // shortcut: the developer window is S15.
        .on_action(cx.listener(|_, _: &keymap::OpenDebug, _, _| {
            log::debug!("open debug: developer window lands in S15")
        }))
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        theme::request_fade_frames(window, cx);
        crate::feel::frame_rendered(window);
        let p = theme::tokens(cx);
        let chrome = self.surface != Surface::Settings;
        let overlay = self.overlay.filter(|_| chrome);
        let root = div()
            .id("shell")
            .key_context(keymap::SHELL_CONTEXT)
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(p.bg)
            .text_color(p.fg)
            .font_family(FONT_UI)
            .text_size(text::MD);
        let timeline = (self.surface == Surface::Timeline).then(|| self.timeline.clone());
        self.register_actions(root, cx)
            .when_some(timeline, |el, t| crate::timeline::register_actions(el, &t))
            .when(chrome, |el| {
                el.child(titlebar::render(self, &p, window, cx))
            })
            .map(|el| match self.surface {
                Surface::Timeline => el.child(self.timeline.clone()),
                Surface::Chat => el.child(self.chat.clone()),
                Surface::Settings => el.child(self.settings.clone()),
                _ => el.child(placeholder::render(self, &p, cx)),
            })
            .when(chrome, |el| {
                el.child(status_bar::render(self, &p, window, cx))
            })
            .when_some(overlay, |el, overlay| {
                // Outside clicks close it (the backdrop eats that click, unlike the web
                // popover, which also lets it through).
                let backdrop = div()
                    .id("overlay-backdrop")
                    .test_support()
                    .absolute()
                    .inset_0()
                    .occlude()
                    .on_mouse_down(
                        gpui_kit::MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.overlay = None;
                            cx.notify();
                        }),
                    );
                let pop = match overlay {
                    Overlay::Notifications => {
                        notifications_pop::render(self, &p, cx).into_any_element()
                    }
                    Overlay::Shortcuts => shortcuts_pop::render(self, &p, cx).into_any_element(),
                    Overlay::Status => status_pop::render(self, &p, cx).into_any_element(),
                };
                el.child(backdrop).child(pop)
            })
    }
}

#[cfg(test)]
#[path = "shell_tests.rs"]
mod tests;
