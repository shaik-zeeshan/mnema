//! The main window: titlebar, the current surface, the status bar, and the three
//! popovers. Holds view state only; `data.rs` fills [`ShellModel`] from AppCore.

mod capture;
pub mod command_field;
pub mod data;
#[cfg(feature = "fixtures")]
pub mod fixture;
pub mod health;
pub mod model;
mod notifications_pop;
mod pop_rows;
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

/// The last loaded developer-options switch, for windows outside the shell
/// (the small windows' sharingType, the developer window's gate).
#[derive(Clone, Copy, Debug, Default)]
pub struct DeveloperOptions(pub bool);

impl gpui_kit::Global for DeveloperOptions {}

pub fn developer_options(cx: &gpui_kit::App) -> bool {
    cx.try_global::<DeveloperOptions>().is_some_and(|d| d.0)
}

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
    pub insights: Entity<crate::insights::Insights>,
    /// The licence banner and the deep-link receipt.
    pub licence: Entity<crate::licence::Licence>,
    focus: FocusHandle,
    _subscriptions: [Subscription; 8],
    tick: Option<Task<()>>,
    /// The last failed start's message ("couldn't start · Retry"), until the next start.
    start_error: Option<String>,
    /// A capture command is in flight (`loadingStart/Stop/Pause`).
    capture_busy: bool,
    /// The 2 s live feed while recording or while the status popover is open.
    live: Option<Task<()>>,
    /// The 60 s beat: Insights' tracked time, and every 5th the bar's "today".
    _beat: Task<()>,
    /// What the beat re-read: the bar's "today", Insights (tests count them).
    pub(crate) beat_reads: (u32, u32),
    /// "PAUSED 00:12" counts from when the shell saw the pause begin.
    pub pause_clock: health::PauseClock,
}

impl Shell {
    pub fn new(model: ShellModel, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let source: std::sync::Arc<dyn crate::timeline::data::Source> = match crate::chat::core(cx)
        {
            Some(core) => std::sync::Arc::new(crate::timeline::data::CoreSource(core)),
            None => std::sync::Arc::new(crate::timeline::data::NoSource),
        };
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
        let backend: std::sync::Arc<dyn crate::insights::data::Backend> =
            match crate::chat::core(cx) {
                Some(core) => std::sync::Arc::new(crate::insights::data::CoreBackend(core)),
                None => std::sync::Arc::new(crate::insights::data::NoBackend),
            };
        let insights = cx.new(|cx| crate::insights::Insights::new(backend, window, cx));
        let licence = cx.new(crate::licence::Licence::new);
        let subscriptions = [
            cx.subscribe_in(&command, window, Self::on_command),
            cx.subscribe_in(&insights, window, Self::on_insights),
            cx.observe(&licence, |_, _, cx| cx.notify()),
            // The empty Timeline's "Start recording".
            cx.subscribe(&timeline, |this, _, event, cx| match event {
                crate::timeline::TimelineEvent::StartRecording => this.start_recording(cx),
            }),
            // System appearance follows the OS (theme::set_system ignores it when pinned).
            cx.observe_window_appearance(window, |_, window, cx| {
                theme::set_system(window.appearance(), cx)
            }),
            // Window focus: re-read what the bar shows and stamp the local UTC
            // offset (`+layout.svelte`'s focus handlers).
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.on_focus(cx);
                }
            }),
            cx.subscribe(&settings, |this, _, event, cx| match event {
                crate::settings::SettingsEvent::Back => this.go(this.last_main, cx),
                crate::settings::SettingsEvent::OpenDebug => this.open_debug(cx),
            }),
            // The focused element left the tree (Settings closed, a popover or the
            // drawer went away): focus the root again, or every shell key goes dead.
            cx.on_focus_lost(window, |this, window, cx| window.focus(&this.focus, cx)),
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
            insights,
            licence,
            focus,
            _subscriptions: subscriptions,
            tick: None,
            start_error: None,
            capture_busy: false,
            live: None,
            _beat: Self::start_beat(cx),
            beat_reads: (0, 0),
            pause_clock: health::PauseClock::default(),
        };
        shell.set_model(model, cx);
        shell
    }

    pub fn set_model(&mut self, mut model: ShellModel, cx: &mut Context<Self>) {
        // A reload doesn't know about the command in flight or the last failed start.
        if model.status.running {
            self.start_error = None;
        }
        if model.status.start_error.is_none() {
            model.status.start_error = self.start_error.clone();
        }
        model.status.loading_start |= self.model.status.loading_start && self.capture_busy;
        // The 2 s feed isn't part of a reload, and a failed read keeps the last
        // good value (status-live.svelte.ts): keep the readings until the next poll.
        if model.status.health.is_none() {
            model.status.health = self.model.status.health.take();
        }
        if model.backlog.is_none() {
            model.backlog = self.model.backlog.take();
        }
        let paused = model.status.running && model.status.user_paused;
        self.pause_clock.observe(paused, cx.background_executor().now());
        self.model = model;
        self.arm_live_poll(cx);
        let s = &self.model.status;
        let hint = crate::timeline::CaptureHint {
            capturing: s.running,
            screen: s.selected[0],
        };
        let developer = self.model.developer_options;
        cx.set_global(DeveloperOptions(developer));
        let status = self.model.status.clone();
        self.settings.update(cx, |s, cx| {
            s.status = status;
            cx.notify();
        });
        let low_disk = self.model.status.low_disk;
        self.licence.update(cx, |l, _| l.low_disk = low_disk);
        let recording = (self.model.status.running, self.model.status.user_paused);
        self.insights.update(cx, |i, cx| {
            i.recording = recording;
            cx.notify();
        });
        self.timeline.update(cx, |t, cx| {
            t.capture = hint;
            t.developer = developer;
            cx.notify();
        });
        // The REC / PAUSED clock ticks once a second while it shows (not while
        // the window is hidden or every screen is asleep).
        let ticking = model::rec_view(&self.model.status).clock.is_some();
        self.tick = ticking.then(|| {
            cx.spawn(async move |this, cx| loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let ticked = this.update(cx, |_, cx| {
                    if !crate::window::render_idle(cx) {
                        cx.notify();
                    }
                });
                if ticked.is_err() {
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
        // Leaving Insights commits a pending Dismiss now; arriving re-reads it.
        if self.surface == Surface::Insights {
            self.insights.update(cx, |i, cx| i.leave(cx));
        }
        if surface == Surface::Insights {
            self.insights.update(cx, |i, cx| i.reload(cx));
        }
        if matches!(surface, Surface::Timeline | Surface::Insights) {
            self.last_main = surface;
        }
        self.surface = surface;
        self.overlay = None;
        if surface == Surface::Settings {
            self.settings.update(cx, |s, cx| s.reload_all(cx));
        }
        if surface == Surface::Chat {
            self.chat.update(cx, |c, cx| c.warm_mcp(cx));
        }
        cx.notify();
    }

    /// The gear and ⌘,: Settings, or back to where it was opened from.
    pub fn toggle_settings(&mut self, cx: &mut Context<Self>) {
        let to = if self.surface == Surface::Settings {
            self.last_main
        } else {
            Surface::Settings
        };
        self.go(to, cx);
    }

    /// Gated on developer options, like `+layout.svelte`'s /debug gate.
    fn open_debug(&mut self, cx: &mut Context<Self>) {
        if self.model.developer_options {
            crate::small::debug::open(cx);
        }
    }

    pub fn toggle_overlay(&mut self, overlay: Overlay, cx: &mut Context<Self>) {
        self.overlay = (self.overlay != Some(overlay)).then_some(overlay);
        if self.overlay == Some(Overlay::Status) {
            // Opening reads now (health + backlog), as StatusPop.svelte's open poll.
            self.live = None;
        }
        self.arm_live_poll(cx);
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
            // `summon_quick_recall_window_command { query }`: the panel takes the text.
            CommandFieldEvent::Recall(text) => {
                let text = text.clone();
                cx.defer(move |cx| crate::quick_recall::panel::summon_with_query(text, cx));
            }
        }
    }

    fn on_insights(
        &mut self,
        _: &Entity<crate::insights::Insights>,
        event: &crate::insights::InsightsEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            crate::insights::InsightsEvent::Ask(q) => {
                let q = Some(q.clone());
                self.chat
                    .update(cx, |chat, cx| chat.new_chat(q, true, window, cx));
                self.go(Surface::Chat, cx);
            }
            crate::insights::InsightsEvent::OpenTimeline => self.go(Surface::Timeline, cx),
            crate::insights::InsightsEvent::OpenFrame(frame_id) => {
                let id = *frame_id;
                self.go(Surface::Timeline, cx);
                self.timeline.update(cx, |t, cx| t.jump_to(id, cx));
            }
            crate::insights::InsightsEvent::StartRecording => {
                if self.model.status.running && self.model.status.user_paused {
                    self.toggle_pause(cx);
                } else {
                    self.start_recording(cx);
                }
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
        s.selected[i] = !s.selected[i];
        let selected = s.selected[i];
        cx.notify();
        self.save_source(i, selected, cx);
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
            let insights_ask = this.surface == Surface::Insights
                && this.insights.update(cx, |i, cx| i.ask_query(cx)).is_some();
            if insights_ask {
                this.insights.update(cx, |i, cx| i.ask(cx));
            } else if this.surface == Surface::Chat {
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
        .on_action(
            cx.listener(|this, _: &keymap::ToggleSettings, _, cx| this.toggle_settings(cx)),
        )
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
            if this.licence.update(cx, |l, cx| l.close_receipt(cx)) {
            } else if this.overlay.is_some() {
                this.overlay = None;
                cx.notify();
            } else if this.surface == Surface::Settings {
                this.go(this.last_main, cx);
            } else if this.surface == Surface::Insights
                && this.insights.update(cx, |i, cx| i.close_top(cx))
            {
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
        .on_action(cx.listener(|this, _: &keymap::OpenDebug, _, cx| this.open_debug(cx)))
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
            .when(self.surface == Surface::Insights, |el| {
                crate::insights::register_keys(el, &self.insights)
            })
            .when(chrome, |el| {
                el.child(titlebar::render(self, &p, window, cx))
            })
            .children(crate::licence::render_banner(&self.licence, cx))
            .map(|el| match self.surface {
                Surface::Timeline => el.child(self.timeline.clone()),
                Surface::Chat => el.child(self.chat.clone()),
                Surface::Settings => el.child(self.settings.clone()),
                Surface::Insights => el.child(self.insights.clone()),
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
            .children(crate::licence::render_receipt(&self.licence, cx))
    }
}

#[cfg(test)]
#[path = "shell_tests.rs"]
mod tests;
