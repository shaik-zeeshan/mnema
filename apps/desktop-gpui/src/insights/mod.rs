//! The Insights surface (`routes/insights/+page.svelte`): a sticky 56 px bar of
//! four underline tabs (Overview · Journal · Subjects · Context) with counts and
//! the active tab's controls, one engine notice, then the tab. Holds view state
//! only; every read and write goes through `data::Backend` off the main thread.
//! The bar is a solid surface (founder ruling 2026-11-08: no in-window blur).

pub mod charts;
mod context;
mod context_ops;
pub mod data;
#[cfg(feature = "fixtures")]
pub mod fixture;
mod header;
pub mod journal;
mod live;
pub use live::on_event;
pub mod model;
mod overview;
mod parts;
pub mod read;
pub mod stats;
pub mod subjects;
pub mod undo;

use std::sync::Arc;
use std::time::Duration;

use capture_types::ActivityCategory;
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, Context, Entity, EventEmitter, Focusable, Pixels, ScrollHandle, Subscription, Task,
    Window,
};
use time::Date;

use crate::theme::{self, space};
use crate::ui::calendar::Unit;
use crate::ui::clock;
use crate::ui::toast::{Toast, Toasts};
use data::Backend;
pub use model::Tab;
use model::{ContextData, JournalData, OverviewData, Status, SubjectData, SubjectsData};
use subjects::tiers::Axis;
use undo::DelayedDismiss;

/// Range steps debounce the digest read (a changed range can cost a model call).
pub const DIGEST_DEBOUNCE: Duration = Duration::from_millis(500);

/// What Insights asks the shell to do.
#[derive(Clone, Debug, PartialEq)]
pub enum InsightsEvent {
    /// ⌘J / "Ask about …": a new chat with this question, scoped to these
    /// days when set (Overview's period; the shell owns Chat, so it applies them).
    Ask(String, Option<(Date, Date)>),
    /// "Open Timeline" from an empty Journal.
    OpenTimeline,
    /// Overview's idle "Start recording" / "Resume recording".
    StartRecording,
    /// The evidence drawer's "Open newest in Timeline": land on this frame.
    OpenFrame(i64),
}

/// Overview's three detail modals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modal {
    Categories,
    Apps,
    Focus,
}

pub struct Insights {
    pub backend: Arc<dyn Backend>,
    pub tab: Tab,
    pub status: Status,
    // Overview
    pub period: (Date, Unit),
    pub overview: Option<OverviewData>,
    pub modal: Option<Modal>,
    pub ov_ui: overview::Ui,
    pub highlight: Option<Option<ActivityCategory>>,
    // Journal
    pub day: Date,
    pub journal: Option<JournalData>,
    pub receipt: Option<journal::receipt::Receipt>,
    /// The card under the pointer, lit on the spine and the river (`hot`).
    pub hot: Option<i64>,
    /// Each card's painted bounds, for the spine's click-to-card.
    pub card_bounds: std::rc::Rc<std::cell::RefCell<std::collections::HashMap<i64, gpui_kit::Bounds<Pixels>>>>,
    /// A capture-only day's runs' top apps: `(day, per run)`.
    pub run_apps: Option<(Date, Vec<Vec<String>>)>,
    pub images: Entity<crate::timeline::image_cache::FrameImages>,
    /// The receipt's audio: the Timeline drawer's backend and player factory.
    pub audio: crate::timeline::audio::Deps,
    // Subjects
    pub axis: Axis,
    pub subjects: Option<SubjectsData>,
    pub filter: Entity<InputState>,
    pub show_fading: bool,
    /// Each section's `(page, size)`: the tiers, the matches, a subject's tiers.
    pub pages: std::collections::HashMap<String, (usize, usize)>,
    /// The section whose rows-per-page menu is open.
    pub pager_menu: Option<String>,
    /// The subject last opened from the index: marked, and paged back to.
    pub last_opened: Option<String>,
    /// Back from that subject's page: turn its tier to it once.
    pub reveal_opened: bool,
    /// Where the subjects list last painted ("Go to the list").
    pub list_anchor: crate::ui::popover::Anchor,
    pub subj_ui: subjects::Ui,
    /// The evidence frame peek (Chat's `FrameDetailModal`).
    pub peek: subjects::peek::Peek,
    // Subject page
    pub subject: Option<String>,
    pub detail: Option<SubjectData>,
    pub open_conclusion: Option<i64>,
    /// The evidence drawer's page, for the conclusion it was turned on.
    pub evidence_page: (i64, usize),
    pub undo: DelayedDismiss,
    undo_toast: Option<u64>,
    undo_timer: Option<Task<()>>,
    // Context
    pub context: Option<ContextData>,
    pub composer: Entity<TextareaState>,
    pub draft_topic: Option<&'static str>,
    pub composer_error: Option<String>,
    pub ctx_ui: context_ops::Ui,
    pub toasts: Entity<Toasts>,
    pub scroll: ScrollHandle,
    /// The shell's capture state: `(running, user paused)`.
    pub recording: (bool, bool),
    /// The capture session's start (`recording_started_at_unix_ms`), while recording.
    pub recording_since: Option<i64>,
    /// The window's width this frame, for the Svelte page's `@media` rules.
    pub width: Pixels,
    gens: [u64; 6],
    /// Each page's read (the digest), loaded on its own.
    pub ov_read: read::Read,
    pub day_read: read::Read,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<InsightsEvent> for Insights {}

/// Generation slots: a load whose slot moved on is stale.
const G_STATUS: usize = 0;
const G_OVERVIEW: usize = 1;
const G_JOURNAL: usize = 2;
const G_SUBJECTS: usize = 3;
const G_SUBJECT: usize = 4;
const G_CONTEXT: usize = 5;

impl Insights {
    pub fn new(backend: Arc<dyn Backend>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let today = clock::now(cx).date();
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter subjects"));
        let composer = cx.new(|cx| {
            // The placeholder is drawn by `context::composer` (the kit's is one
            // unwrapped line in the global muted colour).
            TextareaState::new(window, cx).auto_grow(2, 6)
        });
        let subs = vec![
            // Typing in the bar's filter over a subject's page returns to the
            // index (`onFilter`), and the matches start on their first page.
            cx.subscribe(&filter, |this, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    this.schedule_filter(cx);
                    if this.subject.is_some() {
                        this.close_subject(cx);
                    }
                    cx.notify();
                }
            }),
            // ⌘↵ adds the draft; the 280-char cap holds for typing and pasting.
            cx.subscribe_in(&composer, window, |this, state, e: &InputEvent, window, cx| {
                match e {
                    InputEvent::PressEnter { secondary: true, .. } => this.submit_draft(window, cx),
                    InputEvent::Change => context_ops::clamp(state, window, cx),
                    _ => {}
                }
                cx.notify();
            }),
        ];
        Self {
            backend,
            ov_read: Default::default(),
            day_read: Default::default(),
            tab: Tab::Overview,
            status: Status::default(),
            recording: (false, false),
            recording_since: None,
            width: window.viewport_size().width,
            period: (today, Unit::Week),
            overview: None,
            modal: None,
            ov_ui: Default::default(),
            highlight: None,
            day: today,
            journal: None,
            receipt: None,
            hot: None,
            card_bounds: Default::default(),
            run_apps: None,
            images: crate::timeline::image_cache::FrameImages::new(Default::default(), cx),
            audio: crate::timeline::audio::Deps::live(cx),
            axis: Axis::Conviction,
            subjects: None,
            filter,
            show_fading: false,
            pages: Default::default(),
            pager_menu: None,
            last_opened: None,
            reveal_opened: false,
            list_anchor: crate::ui::popover::Anchor::owned(cx),
            subj_ui: Default::default(),
            peek: subjects::peek::Peek::new(cx),
            subject: None,
            detail: None,
            open_conclusion: None,
            evidence_page: (0, 0),
            undo: DelayedDismiss::default(),
            undo_toast: None,
            undo_timer: None,
            context: None,
            composer,
            draft_topic: None,
            composer_error: None,
            ctx_ui: Default::default(),
            toasts: cx.new(|_| Toasts::default()),
            scroll: ScrollHandle::new(),
            gens: [0; 6],
            _subscriptions: subs,
        }
    }

    /// Run `f` against the backend off the main thread; `apply` lands it unless
    /// a newer load of the same slot overtook it.
    fn load<T: Send + 'static>(
        &mut self,
        slot: usize,
        cx: &mut Context<Self>,
        f: impl FnOnce(&dyn Backend) -> T + Send + 'static,
        apply: impl FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
    ) {
        self.gens[slot] += 1;
        let gen = self.gens[slot];
        let b = self.backend.clone();
        let task = cx.background_spawn(async move { f(&*b) });
        cx.spawn(async move |this, cx| {
            let v = task.await;
            this.update(cx, |this, cx| {
                if this.gens[slot] == gen {
                    apply(this, v, cx);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Fire-and-forget write; `then` runs on the main thread with the result.
    fn write<T: Send + 'static>(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&dyn Backend) -> T + Send + 'static,
        then: impl FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
    ) {
        let b = self.backend.clone();
        let task = cx.background_spawn(async move { f(&*b) });
        cx.spawn(async move |this, cx| {
            let v = task.await;
            this.update(cx, |this, cx| {
                then(this, v, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Re-read the page status and the active tab (mount, a context change).
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.load(
            G_STATUS,
            cx,
            |b| b.status(),
            |this, s, cx| {
                this.status = s;
                this.load_tab(cx);
            },
        );
    }

    pub fn load_tab(&mut self, cx: &mut Context<Self>) {
        match self.tab {
            Tab::Overview => {
                self.load_overview(cx);
                self.load_read(read::Of::Overview, cx);
            }
            Tab::Journal => {
                self.load_journal(cx);
                self.load_read(read::Of::Journal, cx);
            }
            Tab::Subjects => {
                self.load_subjects(cx);
                if self.subject.is_some() {
                    self.load_subject(cx);
                }
            }
            Tab::Context => self.load_context(cx),
        }
    }

    fn offset(&self, cx: &Context<Self>) -> time::UtcOffset {
        clock::now(cx).offset()
    }

    pub fn overview_window(&self, cx: &Context<Self>) -> (i64, i64) {
        stats::window(self.period.0, self.period.1, self.offset(cx))
    }

    fn load_overview(&mut self, cx: &mut Context<Self>) {
        let off = self.offset(cx);
        let (s, e) = self.overview_window(cx);
        let unit = self.period.1;
        let prev_anchor = crate::ui::calendar::step(self.period.0, unit, -1, Date::MIN, Date::MAX)
            .map_or(self.period.0, |(a, _)| a);
        let (ps, pe) = stats::window(prev_anchor, unit, off);
        let now = clock::now_ms(cx);
        // An in-progress period compares pace with pace.
        let pe = if now >= e {
            pe
        } else {
            (ps + (now - s).max(0)).min(pe)
        };
        let cap_only = overview::cap_only(&self.status);
        let off_min = off.whole_minutes() as i32;
        self.load(
            G_OVERVIEW,
            cx,
            move |b| {
                let usage = b.usage(s, e);
                let activities = if cap_only {
                    Ok(Vec::new())
                } else {
                    b.activities(s, e)
                };
                let conclusions = if cap_only {
                    Ok(Vec::new())
                } else {
                    b.conclusions(Some((s, e)))
                };
                let engine_error = activities
                    .as_ref()
                    .err()
                    .or(conclusions.as_ref().err())
                    .cloned();
                let conclusions = conclusions.unwrap_or_default();
                OverviewData {
                    usage_error: usage.as_ref().err().cloned(),
                    usage: usage.ok(),
                    prev_usage: b.usage(ps, pe).ok(),
                    presence: b.presence(s, e),
                    prev_activities: if cap_only {
                        Vec::new()
                    } else {
                        b.activities(ps, pe).unwrap_or_default()
                    },
                    activities: activities.unwrap_or_default(),
                    histories: data::histories(b, conclusions.iter().map(|c| c.subject.clone())),
                    conclusions,
                    engine_error,
                    capture_hours: b.capture_hours(now - 120 * stats::DAY_MS, now, off_min),
                    stale: false,
                }
            },
            |this, d, _| this.overview = Some(d),
        );
    }

    fn load_journal(&mut self, cx: &mut Context<Self>) {
        let off = self.offset(cx);
        let (s, e) = stats::window(self.day, Unit::Day, off);
        let (ws, _) = stats::window(self.day, Unit::Week, off);
        let now = clock::now_ms(cx);
        self.load(
            G_JOURNAL,
            cx,
            move |b| match b.activities(s, e) {
                Err(e) => JournalData {
                    error: Some(e),
                    ..Default::default()
                },
                Ok(activities) => JournalData {
                    activities,
                    week_activities: b.activities(ws, ws + 7 * stats::DAY_MS).unwrap_or_default(),
                    meta: b.activity_meta(s, e),
                    frames: b.frames(s, e).unwrap_or_default(),
                    audio: b.audio(s, e),
                    failed: b.failed_windows(s, e),
                    usage: b.usage(s, e).ok(),
                    history: b.usage(now - 120 * stats::DAY_MS, now).ok(),
                    stale: false,
                    error: None,
                },
            },
            |this, d, _| this.journal = Some(d),
        );
    }

    fn load_subjects(&mut self, cx: &mut Context<Self>) {
        self.load(
            G_SUBJECTS,
            cx,
            |b| data::subjects(b),
            |this, d, _| this.subjects = Some(d),
        );
    }

    fn load_subject(&mut self, cx: &mut Context<Self>) {
        let Some(name) = self.subject.clone() else {
            return;
        };
        self.load(
            G_SUBJECT,
            cx,
            move |b| match b.subject(&name) {
                Err(e) => SubjectData {
                    error: Some(e),
                    ..Default::default()
                },
                Ok(view) => {
                    let wanted: Vec<i64> = view
                        .conclusions
                        .iter()
                        .flat_map(|c| c.evidence.iter().map(|e| e.activity_id))
                        .collect();
                    let mut activities = std::collections::HashMap::new();
                    // Best-effort paged scan of recent Activities (SD-04).
                    for page in 0..6 {
                        let batch = b.recent_activities(200, page * 200);
                        let n = batch.len();
                        activities.extend(
                            batch
                                .into_iter()
                                .filter(|a| wanted.contains(&a.id))
                                .map(|a| (a.id, a)),
                        );
                        if n < 200 || activities.len() >= wanted.len() {
                            break;
                        }
                    }
                    SubjectData {
                        view: Some(view),
                        activities,
                        error: None,
                    }
                }
            },
            |this, d, cx| match (&d.error, this.detail.as_ref().and_then(|v| v.view.as_ref())) {
                // A failed background refresh keeps what's on screen (SD-02).
                (Some(e), Some(_)) => {
                    let subject = this.subject.clone().unwrap_or_default();
                    let t = Toast::danger(format!("Couldn't refresh {subject} — {e}"));
                    this.toasts.update(cx, |ts, cx| ts.push(t, cx));
                }
                _ => this.detail = Some(d),
            },
        );
    }

    /// The subject page's Try again.
    pub fn reload_subject(&mut self, cx: &mut Context<Self>) {
        self.detail = None;
        self.load_subject(cx);
        cx.notify();
    }

    fn load_context(&mut self, cx: &mut Context<Self>) {
        self.load(
            G_CONTEXT,
            cx,
            |b| data::context(b),
            |this, d, _| this.context = Some(d),
        );
    }

    // ── Navigation ──────────────────────────────────────────────────────

    pub fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        if tab != self.tab {
            self.leave(cx);
            self.tab = tab;
            self.modal = None;
            if tab != Tab::Subjects {
                self.subject = None;
                self.open_conclusion = None;
            }
            self.scroll.set_offset(gpui_kit::point(px(0.), px(0.)));
        }
        self.load_tab(cx);
        cx.notify();
    }

    pub fn open_subject(&mut self, subject: String, cx: &mut Context<Self>) {
        if self.subject.as_ref() != Some(&subject) {
            self.leave(cx);
        }
        if self.tab == Tab::Subjects && self.subject.is_none() {
            self.last_opened = Some(subject.clone());
        }
        self.tab = Tab::Subjects;
        self.subject = Some(subject);
        self.detail = None;
        self.open_conclusion = None;
        self.scroll.set_offset(gpui_kit::point(px(0.), px(0.)));
        self.load_subject(cx);
        cx.notify();
    }

    pub fn close_subject(&mut self, cx: &mut Context<Self>) {
        self.leave(cx);
        self.subject = None;
        self.reveal_opened = true;
        self.open_conclusion = None;
        self.load_subjects(cx);
        cx.notify();
    }

    /// Esc: the drawer, then the subject page, then a modal / receipt.
    pub fn close_top(&mut self, cx: &mut Context<Self>) -> bool {
        if self.peek.src.is_some() {
            use crate::chat::peek::PeekOps as _;
            self.close_peek(cx);
        } else if self.ov_ui.adjusting.take().is_some() || self.pager_menu.take().is_some() {
        } else if self.modal.take().is_some() {
        } else if self.open_conclusion.take().is_some() {
        } else if self.subject.is_some() {
            self.close_subject(cx);
        } else if self.receipt.take().is_some() {
        } else {
            return false;
        }
        cx.notify();
        true
    }

    /// The shell left Insights, or the view changed under a pending Dismiss:
    /// commit every pending Dismiss now.
    pub fn leave(&mut self, cx: &mut Context<Self>) {
        let ids = self.undo.flush();
        self.undo_timer = None;
        if let Some(key) = self.undo_toast.take() {
            self.toasts.update(cx, |t, cx| t.dismiss(key, cx));
        }
        self.commit_dismiss(ids, cx);
    }

    /// `[` / `]`: step the Journal a day (never past today).
    pub fn step_day(&mut self, k: i64, cx: &mut Context<Self>) {
        let today = clock::now(cx).date();
        let next = self.day + time::Duration::days(k);
        if next <= today && next != self.day {
            self.set_day(next, cx);
        }
    }

    /// A day step: the old day's cards go (skeleton until this day lands; the
    /// picker's ~120-day history stays), its read is debounced.
    pub fn set_day(&mut self, d: Date, cx: &mut Context<Self>) {
        self.day = d;
        self.receipt = None;
        self.journal = self.journal.take().map(|j| JournalData {
            history: j.history,
            stale: true,
            ..Default::default()
        });
        self.load_journal(cx);
        self.schedule_read(read::Of::Journal, cx);
        cx.notify();
    }

    /// A period step: never another range's numbers under this range's label
    /// (OV-13), so skeletons until it lands; the read is debounced.
    pub fn set_period(&mut self, d: Date, unit: Unit, cx: &mut Context<Self>) {
        self.period = (d, unit);
        self.overview = self.overview.take().map(|o| OverviewData {
            capture_hours: o.capture_hours,
            stale: true,
            ..Default::default()
        });
        self.modal = None;
        self.load_overview(cx);
        self.schedule_read(read::Of::Overview, cx);
        cx.notify();
    }

    /// The question ⌘J asks here, if Ask can run (Overview / a subject page).
    pub fn ask_query(&self, cx: &Context<Self>) -> Option<String> {
        if !overview::engine_ready(&self.status) {
            return None;
        }
        match (self.tab, &self.subject) {
            (Tab::Subjects, Some(s)) => Some(format!("What do you know about {s}?")),
            (Tab::Overview, _) if !overview::cap_only(&self.status) => {
                Some(format!("What did {} add up to?", overview::scope(self, cx)))
            }
            _ => None,
        }
    }

    /// The days "Ask about this week" scopes Chat to (Overview's period,
    /// `?from=&to=`); a subject's Ask keeps Chat's own scope.
    pub fn ask_scope(&self) -> Option<(Date, Date)> {
        (self.tab == Tab::Overview && self.subject.is_none())
            .then(|| crate::ui::calendar::period_of(self.period.0, self.period.1))
    }

    pub fn ask(&mut self, cx: &mut Context<Self>) {
        let Some(q) = self.ask_query(cx) else {
            return;
        };
        // No `window::shell` here: ⌘J calls this inside the shell's own update.
        cx.emit(InsightsEvent::Ask(q, self.ask_scope()));
    }

    /// Typing in one of the tab's fields (the bare-key shortcuts stand down).
    pub fn typing(&self, window: &Window, cx: &gpui_kit::App) -> bool {
        self.filter.focus_handle(cx).is_focused(window)
            || self.composer.focus_handle(cx).is_focused(window)
    }
}

impl Render for Insights {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = theme::tokens(cx);
        self.width = window.viewport_size().width;
        if self.status.loaded && !self.loaded_tab() {
            // A tab shown before its first load (e.g. tab switched by a test): load it.
            self.load_tab(cx);
        }
        let body = div()
            .id("insights-view")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            // Back at the top, a staged Subjects reload swaps in on its own.
            .on_scroll_wheel(cx.listener(|this, _, _, cx| {
                if this.subj_ui.staged.is_some() && this.at_top() {
                    this.apply_staged(cx);
                }
            }))
            .child(
                div()
                    .w_full()
                    .max_w(px(1280.))
                    .mx_auto()
                    .px(space::S7)
                    .pr(space::S7 + subjects::drawer_push(self))
                    .pt(space::S6)
                    .pb(space::S8)
                    .children(header::notice(self, &p, cx))
                    .child(match self.tab {
                        Tab::Overview => overview::render(self, &p, window, cx),
                        Tab::Journal => journal::render(self, &p, window, cx),
                        Tab::Subjects => subjects::render(self, &p, window, cx),
                        Tab::Context => context::render(self, &p, window, cx),
                    }),
            )
            .child(crate::ui::scroll::bar(&self.scroll));
        let drawer = subjects::drawer(self, &p, cx);
        div()
            .id("insights")
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .bg(p.bg)
            .child(header::bar(self, &p, window, cx))
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(body)
                    .children(drawer)
                    .children(subjects::refresh::pill(self, &p, cx)),
            )
            .children(overview::modal(self, &p, cx))
            .children(self.peek.src.clone().map(|s| {
                crate::chat::peek::frame_modal(self, &s, &p, window.viewport_size(), cx)
                    .into_any_element()
            }))
            .child(self.toasts.clone())
    }
}

impl Insights {
    fn loaded_tab(&self) -> bool {
        match self.tab {
            Tab::Overview => self.overview.is_some() || self.gens[G_OVERVIEW] > 0,
            Tab::Journal => self.journal.is_some() || self.gens[G_JOURNAL] > 0,
            Tab::Subjects => self.subjects.is_some() || self.gens[G_SUBJECTS] > 0,
            Tab::Context => self.context.is_some() || self.gens[G_CONTEXT] > 0,
        }
    }
}

/// Bare keys Insights reads off the shell root (the focus path): `[` / `]`
/// step the Journal, ↑/↓ walk a subject's conclusions. Never while typing.
pub fn register_keys(
    el: gpui_kit::Stateful<gpui_kit::Div>,
    view: &Entity<Insights>,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let view = view.clone();
    el.on_key_down(move |e: &gpui_kit::KeyDownEvent, window, cx| {
        let m = &e.keystroke.modifiers;
        if m.platform || m.control || m.alt || view.read(cx).typing(window, cx) {
            return;
        }
        let key = e.keystroke.key.clone();
        let handled = view.update(cx, |this, cx| match key.as_str() {
            "[" | "]" => journal::on_key(this, &key, cx),
            "up" | "down" if this.subject.is_some() => {
                subjects::step_conclusion(this, if key == "down" { 1 } else { -1 }, cx)
            }
            // ←/→ turn a tier's page (the index, or a subject's page).
            "left" | "right" if this.tab == Tab::Subjects && this.modal.is_none() => {
                let d = if key == "right" { 1 } else { -1 };
                if this.subject.is_some() {
                    subjects::turn_subject(this, d, cx)
                } else {
                    subjects::turn_list(this, d, cx)
                }
            }
            _ => false,
        });
        if handled {
            cx.stop_propagation();
        }
    })
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
#[cfg(test)]
mod flow_tests;
#[cfg(test)]
mod link_tests;
