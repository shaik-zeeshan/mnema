//! The Chat surface (`routes/chat/+page.svelte` + `lib/chat/*`): the chat list
//! beside one conversation. The backend owns every turn's render model and
//! streams versioned `ask_ai_update` ops; this entity holds a copy and only
//! renders (docs/agents/ask-ai-streaming.md). `data.rs` is the one backend door.

mod actions;
pub mod blocks;
pub mod clip;
mod composer;
pub mod data;
#[cfg(feature = "fixtures")]
pub mod fixture;
pub mod format;
mod icons;
pub(crate) mod list;
pub mod model;
pub mod peek;
pub mod prose;
pub mod scope;
pub mod source_data;
mod sources;
mod thread;
mod turn;
pub mod turn_error;

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use app_core::AppCore;
use capture_types::{Conversation, TurnSnapshot};
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::prelude::*;
use gpui_kit::{div, App, Context, Entity, Pixels, ScrollHandle, Size, Subscription, Task, Window};

use crate::bridge::events::Core;
use crate::theme::{self, size};
use crate::timeline::image_cache::FrameImages;
use crate::ui::clock;
use crate::ui::model_picker::{PoolModel, ProviderFailure};
use crate::ui::toast::Toasts;
pub use model::{AskAiUpdateEvent, ChatData, ChatTurn, History, Receive, Source};
pub use scope::Scope;

/// "Copied" on the answer's copy button reverts after this (AnswerProse:73-78).
/// The answer copy button (ChatTurn.svelte:83); 1200 ms is the code-block button.
pub const COPIED_REVERT: Duration = Duration::from_millis(1600);
/// The "Chat deleted" toast (`+page.svelte` showDeleted).
pub const DELETED_TOAST_TTL: Duration = Duration::from_secs(4);
/// The failed pin/unpin toast with Retry.
pub const PIN_FAIL_TOAST_TTL: Duration = Duration::from_secs(5);
pub(crate) const COMPOSER_PLACEHOLDER: &str =
    "Ask about anything you've seen, heard, or worked on…";
/// The list search waits this long after the last keystroke (`SEARCH_DEBOUNCE_MS`).
pub const SEARCH_DEBOUNCE: Duration = Duration::from_millis(220);
/// Titles are cut at this many chars (`TITLE_MAX`).
pub const TITLE_MAX: usize = 80;
/// Within this of the bottom counts as "at the bottom" (the pane follows the stream).
const BOTTOM_SLACK: f32 = 40.;

/// What the conversation pane shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Pane {
    New,
    Loading,
    LoadError(String),
    /// `get_conversation` found nothing under the id.
    Missing,
    Thread,
}

/// An inline rename in progress.
#[derive(Clone, Debug, PartialEq)]
pub enum Rename {
    Header,
    Row(String),
}

pub struct Chat {
    pub data: ChatData,
    pub active: Option<String>,
    pub title: String,
    pub created_at_ms: Option<i64>,
    pub turns: Vec<ChatTurn>,
    pub pane: Pane,
    pub streaming: bool,
    /// The per-chat engine pin.
    pub pin: Option<(String, String)>,
    /// The open chat's pinned flag when its list row is hidden.
    pub pinned: Option<bool>,
    pub scope: Scope,
    /// The docked composer's context panel.
    pub ctx_open: bool,
    /// A host chip's page is opening (`opening`).
    pub opening_url: Option<i64>,
    /// The turn whose copy button reads "Copied".
    pub copied: Option<i64>,
    copied_task: Option<Task<()>>,
    /// The ChatGPT sign-in code's copy button reads "Copied".
    pub code_copied: bool,
    code_copied_task: Option<Task<()>>,
    pub pool: Vec<PoolModel>,
    pub pool_failures: Vec<ProviderFailure>,
    pub pool_loading: bool,
    /// Provider-reported context windows for the token bar, per answering
    /// provider, listed once when its usage is on screen.
    pub usage_models: HashMap<String, Vec<PoolModel>>,
    usage_asked: HashSet<String>,
    /// The provider set (id, base URL) the pool was listed against.
    pool_sig: Option<Vec<(String, String)>>,
    pub peek: Option<Source>,
    /// The peeked frame's size, image and OCR, once loaded.
    pub peek_data: peek::PeekData,
    /// Drops a late peek / OCR answer for a frame no longer peeked.
    peek_gen: u64,
    peek_focus: gpui_kit::FocusHandle,
    /// An audio source's open receipt.
    pub clip: Option<clip::Clip>,
    clip_gen: u64,
    /// What the sources read (thumbnails, the peek, the receipt's strip).
    pub sources: Arc<dyn source_data::SourceBackend>,
    /// The clip's segment lookup and player (the Timeline drawer's).
    pub audio: crate::timeline::audio::Deps,
    /// Frame id → its thumbnail (`None` = no preview any more); absent = loading.
    pub thumbs: HashMap<i64, Option<PathBuf>>,
    thumbs_asked: HashSet<i64>,
    thumbs_gen: u64,
    /// App name (trimmed, lower-cased) → its icon, for tool rows and timeline chips.
    pub icons: HashMap<String, PathBuf>,
    icons_asked: HashSet<String>,
    /// The inline ChatGPT sign-in a failed turn offers (provider id, state).
    pub login: Option<(String, turn_error::Login)>,
    pub thumb_images: Entity<FrameImages>,
    /// The peek's hero and the receipt's screen, decoded to the hero's size.
    pub images: Entity<FrameImages>,
    pub hero_size: Rc<Cell<Size<Pixels>>>,
    pub steps_open: HashSet<i64>,
    pub reason_open: HashSet<i64>,
    pub renaming: Option<Rename>,
    pub composer: Entity<TextareaState>,
    pub search: Entity<InputState>,
    pub rename_input: Entity<InputState>,
    pub toasts: Entity<Toasts>,
    pub scroll: ScrollHandle,
    /// Goldens: render the model picker open.
    pub force_picker: bool,
    /// Which placeholder the composer shows (docked = "Ask a follow-up…").
    docked_placeholder: bool,
    /// `?q=` arrived before Ask AI's availability: send once it is known.
    pending_send: bool,
    /// Drops a stale list / search answer.
    load_generation: u64,
    /// The pending debounced search.
    search_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

/// Inline text runs in one wrapping paragraph (HTML `text <b>bold</b> text`);
/// a run's `(weight, colour)` overrides the inherited style.
pub(crate) fn inline_runs(
    runs: &[(&str, Option<(gpui_kit::FontWeight, gpui_kit::Hsla)>)],
) -> gpui_kit::StyledText {
    let mut text = String::new();
    let mut highlights = Vec::new();
    for (s, style) in runs {
        let start = text.len();
        text.push_str(s);
        if let Some((weight, color)) = style {
            highlights.push((
                start..text.len(),
                gpui_kit::HighlightStyle {
                    font_weight: Some(*weight),
                    color: Some(*color),
                    ..Default::default()
                },
            ));
        }
    }
    gpui_kit::StyledText::new(text).with_highlights(highlights)
}

pub(crate) fn core(cx: &App) -> Option<Arc<AppCore>> {
    cx.try_global::<Core>().map(|c| c.0.clone())
}

impl Chat {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 9)
                .submit_on_enter(true)
        });
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search chats"));
        let rename_input = cx.new(|cx| InputState::new(window, cx));
        let toasts = cx.new(|_| Toasts::default());
        let hero_size: Rc<Cell<Size<Pixels>>> = Rc::default();
        let subs = vec![
            cx.subscribe_in(&composer, window, |this, _, e: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { shift: false, .. } = e {
                    if this.streaming {
                        this.stop(cx);
                    } else {
                        this.send(None, window, cx);
                    }
                }
            }),
            cx.subscribe_in(&search, window, |this, _, e: &InputEvent, _, cx| {
                if matches!(e, InputEvent::Change) {
                    // `onSearchInput`: `search_conversations` after a pause; the
                    // list filters what it has until it answers.
                    this.search_task = Some(cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(SEARCH_DEBOUNCE).await;
                        let _ = this.update(cx, |this, cx| this.reload(cx));
                    }));
                    cx.notify();
                }
            }),
            cx.subscribe_in(
                &rename_input,
                window,
                |this, _, e: &InputEvent, window, cx| match e {
                    InputEvent::PressEnter { .. } | InputEvent::Blur => {
                        this.commit_rename(window, cx)
                    }
                    _ => {}
                },
            ),
        ];
        Self {
            data: ChatData::default(),
            active: None,
            title: String::new(),
            created_at_ms: None,
            turns: Vec::new(),
            pane: Pane::New,
            streaming: false,
            pin: None,
            pinned: None,
            scope: Scope::default(),
            ctx_open: false,
            opening_url: None,
            copied: None,
            copied_task: None,
            code_copied: false,
            code_copied_task: None,
            pool: Vec::new(),
            pool_failures: Vec::new(),
            pool_loading: false,
            pool_sig: None,
            usage_models: HashMap::new(),
            usage_asked: HashSet::new(),
            peek: None,
            peek_data: Default::default(),
            peek_gen: 0,
            peek_focus: cx.focus_handle(),
            clip: None,
            clip_gen: 0,
            sources: match core(cx) {
                Some(core) => Arc::new(source_data::CoreSources(core)),
                None => Arc::new(source_data::NoSources),
            },
            audio: crate::timeline::audio::Deps::live(cx),
            thumbs: HashMap::new(),
            thumbs_asked: HashSet::new(),
            thumbs_gen: 0,
            icons: HashMap::new(),
            icons_asked: HashSet::new(),
            login: None,
            thumb_images: FrameImages::with_decoded(
                Default::default(),
                sources::THUMBS_DECODED,
                cx,
            ),
            images: FrameImages::new(hero_size.clone(), cx),
            hero_size,
            steps_open: HashSet::new(),
            reason_open: HashSet::new(),
            renaming: None,
            composer,
            search,
            rename_input,
            toasts,
            scroll: ScrollHandle::new(),
            force_picker: false,
            docked_placeholder: false,
            pending_send: false,
            load_generation: 0,
            search_task: None,
            _subscriptions: subs,
        }
    }

    pub fn set_data(&mut self, data: ChatData, cx: &mut Context<Self>) {
        self.data = data;
        cx.notify();
    }

    /// Refill the list + engine facts off-thread.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        // `refreshHistory`: a search when the box has text, else the plain list;
        // a generation token drops an out-of-order answer.
        self.load_generation += 1;
        let generation = self.load_generation;
        let Some(core) = core(cx) else { return };
        let query = self.search.read(cx).value().trim().to_string();
        let task =
            crate::bridge::tokio::Tokio::spawn(cx, async move { data::load(&core, &query).await });
        cx.spawn(async move |this, cx| {
            if let Ok(data) = task.await {
                let _ = this.update(cx, |this, cx| {
                    if this.load_generation == generation {
                        this.set_data(data, cx)
                    }
                });
            }
        })
        .detach();
    }

    /// Chat opened (Svelte's `onMount`): warm the MCP connectors so a turn
    /// finds their tools ready. Fire-and-forget, off the main thread.
    pub fn warm_mcp(&self, cx: &mut Context<Self>) {
        let sources = self.sources.clone();
        cx.background_spawn(async move {
            if sources
                .settings()
                .is_some_and(|s| data::mcp_warm_wanted(&s))
            {
                sources.warm_mcp();
            }
        })
        .detach();
    }

    pub fn available(&self) -> Option<bool> {
        self.data.availability.as_ref().map(|a| a.available)
    }

    /// The pane's layout mode (`mode` in Chat.svelte).
    pub fn is_new(&self) -> bool {
        self.pane == Pane::New || (self.pane == Pane::Thread && self.turns.is_empty())
    }

    fn reset(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        self.active = id;
        self.title.clear();
        self.created_at_ms = None;
        self.turns.clear();
        self.pane = Pane::New;
        self.streaming = false;
        self.pin = None;
        self.pinned = None;
        self.renaming = None;
        self.pending_send = false;
        self.peek = None;
        self.peek_gen += 1;
        self.peek_data = Default::default();
        self.close_clip(cx);
        self.thumbs.clear();
        self.thumbs_asked.clear();
        self.thumbs_gen += 1;
        self.copied = None;
        // Like `ChatgptConnect` unmounting: the backend's poll runs on.
        self.login = None;
        self.steps_open.clear();
        self.reason_open.clear();
        cx.notify();
    }

    /// A fresh chat; `prefill` seeds the composer to review, and is sent only
    /// when `send` (the titlebar's ask field). Suggestions never send.
    pub fn new_chat(
        &mut self,
        prefill: Option<String>,
        send: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reset(Some(uuid::Uuid::new_v4().to_string()), cx);
        let text = prefill.unwrap_or_default();
        self.composer.update(cx, |s, cx| {
            s.set_value(text.clone(), window, cx);
            s.focus(window, cx);
        });
        // `?q=` sends once availability is known; if Ask AI can't run, the
        // question just stays in the composer.
        if send && !text.trim().is_empty() {
            match self.available() {
                Some(true) => self.send(None, window, cx),
                Some(false) => {}
                None => self.pending_send = true,
            }
        }
    }

    /// `conversation_changed`: re-list; a chat that read "deleted" before its
    /// first save landed is looked up again.
    pub fn on_conversation_changed(&mut self, cx: &mut Context<Self>) {
        self.reload(cx);
        if let (Pane::Missing, Some(id)) = (&self.pane, self.active.clone()) {
            self.open(id, cx);
        }
    }

    /// Show a loaded conversation, adopting the live snapshot when one runs.
    pub fn show(
        &mut self,
        convo: &Conversation,
        snap: Option<&TurnSnapshot>,
        cx: &mut Context<Self>,
    ) {
        self.reset(Some(convo.conversation_id.clone()), cx);
        self.title = convo.title.clone();
        self.created_at_ms = Some(convo.created_at_ms);
        self.pin = convo.provider.clone().zip(convo.model.clone());
        self.pinned = Some(convo.pinned);
        self.turns = convo.turns.iter().map(ChatTurn::hydrate).collect();
        self.streaming = self
            .turns
            .last()
            .is_some_and(|t| t.view.phase == "streaming");
        if let Some(snap) = snap.filter(|s| s.conversation_id == convo.conversation_id) {
            if let Some(t) = self
                .turns
                .iter_mut()
                .find(|t| t.view.turn_index == snap.view.turn_index)
            {
                t.adopt(snap);
                self.streaming = t.live();
            }
        }
        self.pane = Pane::Thread;
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    /// Open `id` from the list (or a hand-off).
    pub fn open(&mut self, id: String, cx: &mut Context<Self>) {
        if self.active.as_deref() == Some(id.as_str()) && !self.turns.is_empty() {
            return;
        }
        self.reset(Some(id.clone()), cx);
        self.pane = Pane::Loading;
        let Some(core) = core(cx) else { return };
        let task = crate::bridge::tokio::Tokio::spawn(cx, async move {
            let convo = data::open(&core, &id).await;
            let snap = data::snapshot(&core, &id).await;
            (id, convo, snap)
        });
        cx.spawn(async move |this, cx| {
            let Ok((id, convo, snap)) = task.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if this.active.as_deref() != Some(id.as_str()) {
                    return;
                }
                match convo {
                    Ok(Some(c)) => this.show(&c, snap.as_ref(), cx),
                    Ok(None) => {
                        this.pane = Pane::Missing;
                        cx.notify();
                    }
                    Err(e) => {
                        this.pane = Pane::LoadError(e);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn reconcile(&mut self, ix: i64) {
        if let Some(last) = self.turns.last() {
            if last.view.turn_index == ix {
                self.streaming = last.live();
            }
        }
    }

    fn at_bottom(&self) -> bool {
        let (off, max) = (self.scroll.offset().y, self.scroll.max_offset().y);
        f32::from(max + off) <= BOTTOM_SLACK
    }

    /// One `ask_ai_update` (the SOLE stream listener): exactly-next applies,
    /// a gap re-snapshots, stale/duplicate is ignored.
    pub fn on_update(&mut self, ev: AskAiUpdateEvent, cx: &mut Context<Self>) -> Option<Receive> {
        if self.active.as_deref() != Some(ev.conversation_id.as_str()) {
            return None;
        }
        let now = clock::now_ms(cx);
        let follow = self.at_bottom();
        let trailing = self.turns.last().map(|t| t.view.turn_index) == Some(ev.turn_index);
        let Some(turn) = self
            .turns
            .iter_mut()
            .find(|t| t.view.turn_index == ev.turn_index)
        else {
            // A turn started on this chat from another window: hydrate from the snapshot.
            self.resnapshot(ev.conversation_id, ev.turn_index, true, cx);
            return Some(Receive::Gap);
        };
        let r = model::receive(turn, ev.version, &ev.update, now);
        match r {
            Receive::Applied => {
                if matches!(ev.update, capture_types::TurnUpdate::Error { .. }) && trailing {
                    let q = turn.view.question.clone();
                    self.restore_question(q, cx);
                }
                self.reconcile(ev.turn_index);
                if follow {
                    self.scroll.scroll_to_bottom();
                }
                cx.notify();
            }
            Receive::Stale => {}
            Receive::Gap => self.resnapshot(ev.conversation_id, ev.turn_index, false, cx),
        }
        Some(r)
    }

    fn resnapshot(&mut self, id: String, ix: i64, insert: bool, cx: &mut Context<Self>) {
        let Some(core) = core(cx) else { return };
        let task = crate::bridge::tokio::Tokio::spawn(cx, {
            let id = id.clone();
            async move { data::snapshot(&core, &id).await }
        });
        cx.spawn(async move |this, cx| {
            let Ok(Some(snap)) = task.await else { return };
            let _ = this.update(cx, |this, cx| {
                if this.active.as_deref() != Some(id.as_str()) || snap.view.turn_index != ix {
                    return;
                }
                match this.turns.iter_mut().find(|t| t.view.turn_index == ix) {
                    Some(t) => t.adopt(&snap),
                    None if insert => {
                        let mut t = ChatTurn::new(ix, snap.view.question.clone(), "thinking");
                        t.adopt(&snap);
                        this.turns.push(t);
                        this.turns.sort_by_key(|t| t.view.turn_index);
                        if this.pane == Pane::Missing {
                            this.pane = Pane::Thread;
                        }
                    }
                    None => return,
                }
                this.reconcile(ix);
                cx.notify();
            });
        })
        .detach();
    }

    /// Put a failed question back, unless something new was typed.
    fn restore_question(&mut self, q: String, cx: &mut Context<Self>) {
        let empty = self.composer.read(cx).value().trim().is_empty();
        if empty {
            if let Some(window) = cx.active_window() {
                let composer = self.composer.clone();
                let _ = window.update(cx, |_, window, cx| {
                    composer.update(cx, |s, cx| s.set_value(q, window, cx))
                });
            }
        }
    }

    /// List each answering provider's models once its usage is on screen, for
    /// the context window the token bar measures against.
    fn request_usage_windows(&mut self, cx: &mut Context<Self>) {
        let wanted: Vec<_> = self
            .turns
            .iter()
            .filter(|t| t.view.context_tokens.is_some())
            .filter_map(|t| t.view.provider.clone())
            .filter(|id| !self.usage_asked.contains(id))
            .collect();
        for id in wanted {
            let Some(config) = self.data.engine.providers.iter().find(|p| p.id == id).cloned()
            else {
                continue;
            };
            self.usage_asked.insert(id.clone());
            let Some(core) = core(cx) else { return };
            let task = crate::bridge::tokio::Tokio::spawn(cx, async move {
                data::list_models(&core, Some(vec![config])).await.0
            });
            cx.spawn(async move |this, cx| {
                let Ok(models) = task.await else { return };
                let _ = this.update(cx, |this, cx| {
                    this.usage_models.insert(id, models);
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// The window of the model that wrote this answer (never the picker's):
    /// what its provider reports, else a name-based guess, but no guess for a
    /// local server, which picks its own window (Ollama: 4k–256k by GPU memory).
    pub fn context_window_of(&self, t: &ChatTurn) -> Option<u64> {
        let model = t.view.model.as_deref()?;
        let provider = t.view.provider.as_deref();
        let reported = provider
            .and_then(|p| self.usage_models.get(p))
            .and_then(|ms| ms.iter().find(|m| m.id == model))
            .and_then(|m| m.context_window);
        if reported.is_some() {
            return reported;
        }
        let config = provider.and_then(|id| self.data.engine.providers.iter().find(|p| p.id == id));
        if config.is_some_and(|c| format::provider_is_local(Some(c))) {
            return None;
        }
        format::context_window_for_model(model)
    }

    /// "Copied" on `turn`'s copy button for [`COPIED_REVERT`].
    pub fn copy_answer(&mut self, turn: i64, cx: &mut Context<Self>) {
        let Some(text) = self
            .turns
            .iter()
            .find(|t| t.view.turn_index == turn)
            .map(ChatTurn::plain_text)
        else {
            return;
        };
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text));
        self.copied = Some(turn);
        self.copied_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPIED_REVERT).await;
            let _ = this.update(cx, |this, cx| {
                this.copied = None;
                cx.notify();
            });
        }));
        cx.notify();
    }
}

impl Render for Chat {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = theme::tokens(cx);
        let docked = !self.is_new() && self.pane == Pane::Thread;
        if docked != self.docked_placeholder {
            self.docked_placeholder = docked;
            // Undocked, `composer::render` draws the placeholder itself: the
            // kit's is one unwrapped line, cut off on a narrow window.
            let text = if docked { "Ask a follow-up…" } else { "" };
            self.composer
                .update(cx, |s, cx| s.set_placeholder(text, window, cx));
        }
        if self.pending_send && self.available().is_some() {
            self.pending_send = false;
            if self.available() == Some(true) {
                self.send(None, window, cx);
            }
        }
        self.request_usage_windows(cx);
        self.request_thumbs(cx);
        self.request_icons(cx);
        self.tick_clip(window, cx);
        let narrow = window.viewport_size().width <= size::CHAT_LIST_BREAKPOINT;
        div()
            .id("surface-chat")
            .role(gpui_kit::Role::Main)
            .aria_label("Chat")
            .relative()
            .flex()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .bg(p.bg)
            .child(list::render(self, narrow, &p, window, cx))
            .child(thread::render(self, &p, window, cx))
            .children(
                self.peek
                    .clone()
                    .map(|s| peek::frame_modal(self, &s, &p, window.viewport_size(), cx)),
            )
            .child(self.toasts.clone())
    }
}

#[cfg(test)]
#[path = "chat_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "flow_tests.rs"]
mod flow_tests;

#[cfg(all(test, feature = "fixtures"))]
#[path = "sources_tests.rs"]
mod sources_tests;
