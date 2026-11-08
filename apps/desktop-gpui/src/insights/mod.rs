//! The Insights surface (`routes/insights/+page.svelte`): a sticky 56 px bar of
//! four underline tabs (Overview · Journal · Subjects · Context) with counts and
//! the active tab's controls, one engine notice, then the tab. Holds view state
//! only; every read and write goes through `data::Backend` off the main thread.
//! The bar is a solid surface (founder ruling 2026-11-08: no in-window blur).

pub mod charts;
mod context;
pub mod data;
#[cfg(feature = "fixtures")]
pub mod fixture;
mod header;
pub mod journal;
pub mod model;
mod overview;
mod parts;
pub mod stats;
pub mod subjects;
pub mod undo;

use std::sync::Arc;
use std::time::Duration;

use capture_types::ActivityCategory;
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, Context, Entity, EventEmitter, Focusable, ScrollHandle, Subscription, Task, Window,
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
use undo::{DelayedDismiss, DISMISS_UNDO};

/// Range steps debounce the digest read (a changed range can cost a model call).
pub const DIGEST_DEBOUNCE: Duration = Duration::from_millis(500);

/// What Insights asks the shell to do.
#[derive(Clone, Debug, PartialEq)]
pub enum InsightsEvent {
    /// ⌘J / "Ask about …": a new chat with this question.
    Ask(String),
    /// "Open Timeline" from an empty Journal.
    OpenTimeline,
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
    pub highlight: Option<Option<ActivityCategory>>,
    // Journal
    pub day: Date,
    pub journal: Option<JournalData>,
    pub receipt: Option<journal::receipt::Receipt>,
    pub images: Entity<crate::timeline::image_cache::FrameImages>,
    // Subjects
    pub axis: Axis,
    pub subjects: Option<SubjectsData>,
    pub filter: Entity<InputState>,
    pub show_fading: bool,
    // Subject page
    pub subject: Option<String>,
    pub detail: Option<SubjectData>,
    pub open_conclusion: Option<i64>,
    pub undo: DelayedDismiss,
    undo_toast: Option<u64>,
    undo_timer: Option<Task<()>>,
    // Context
    pub context: Option<ContextData>,
    pub composer: Entity<TextareaState>,
    pub draft_topic: Option<&'static str>,
    pub composer_error: Option<String>,
    pub toasts: Entity<Toasts>,
    pub scroll: ScrollHandle,
    gens: [u64; 6],
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
            TextareaState::new(window, cx).auto_grow(2, 6).placeholder(
                "e.g. “I'm a designer — Figma time is real work even when it looks like browsing.”",
            )
        });
        let subs = vec![cx.subscribe(&filter, |_, _, e: &InputEvent, cx| {
            if matches!(e, InputEvent::Change) {
                cx.notify();
            }
        })];
        Self {
            backend,
            tab: Tab::Overview,
            status: Status::default(),
            period: (today, Unit::Week),
            overview: None,
            modal: None,
            highlight: None,
            day: today,
            journal: None,
            receipt: None,
            images: crate::timeline::image_cache::FrameImages::new(Default::default(), cx),
            axis: Axis::Conviction,
            subjects: None,
            filter,
            show_fading: false,
            subject: None,
            detail: None,
            open_conclusion: None,
            undo: DelayedDismiss::default(),
            undo_toast: None,
            undo_timer: None,
            context: None,
            composer,
            draft_topic: None,
            composer_error: None,
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
            Tab::Overview => self.load_overview(cx),
            Tab::Journal => self.load_journal(cx),
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
        let engine_on = overview::engine_on(&self.status);
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
                let unit_name = unit.name();
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
                    digest: engine_on.then(|| b.digest(unit_name, s, e, false)),
                    capture_hours: b.capture_hours(now - 120 * stats::DAY_MS, now, off_min),
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
        let engine_on = overview::engine_on(&self.status);
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
                    digest: engine_on.then(|| b.digest("day", s, e, false)),
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
            |this, d, _| this.detail = Some(d),
        );
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
        self.open_conclusion = None;
        self.load_subjects(cx);
        cx.notify();
    }

    /// Esc: the drawer, then the subject page, then a modal / receipt.
    pub fn close_top(&mut self, cx: &mut Context<Self>) -> bool {
        if self.modal.take().is_some() {
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
            self.day = next;
            self.receipt = None;
            self.load_journal(cx);
            cx.notify();
        }
    }

    pub fn set_day(&mut self, d: Date, cx: &mut Context<Self>) {
        self.day = d;
        self.receipt = None;
        self.load_journal(cx);
        cx.notify();
    }

    pub fn set_period(&mut self, d: Date, unit: Unit, cx: &mut Context<Self>) {
        self.period = (d, unit);
        self.load_overview(cx);
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

    pub fn ask(&mut self, cx: &mut Context<Self>) {
        if let Some(q) = self.ask_query(cx) {
            cx.emit(InsightsEvent::Ask(q));
        }
    }

    // ── Dismiss with Undo ───────────────────────────────────────────────

    /// Collapse the row to "Dismissed · Undo" and commit in `DISMISS_UNDO`.
    pub fn dismiss(&mut self, id: i64, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        self.undo.start(id, now);
        if self.open_conclusion == Some(id) {
            self.open_conclusion = None;
        }
        let this = cx.entity().downgrade();
        let toast =
            Toast::ok("Dismissed — it won't be re-inferred").action("Undo", true, move |_, cx| {
                this.update(cx, |i, cx| i.undo_dismiss(id, cx)).ok();
            });
        let toast = Toast {
            ttl: DISMISS_UNDO,
            ..toast
        };
        self.undo_toast = Some(self.toasts.update(cx, |t, cx| t.push(toast, cx)));
        self.arm_undo(cx);
        cx.notify();
    }

    pub fn undo_dismiss(&mut self, id: i64, cx: &mut Context<Self>) {
        if self.undo.undo(id) {
            if let Some(key) = self.undo_toast.take() {
                self.toasts.update(cx, |t, cx| t.dismiss(key, cx));
            }
            self.open_conclusion = Some(id);
            self.arm_undo(cx);
            cx.notify();
        }
    }

    /// One timer for the earliest pending deadline.
    fn arm_undo(&mut self, cx: &mut Context<Self>) {
        let Some(at) = self.undo.next_deadline() else {
            self.undo_timer = None;
            return;
        };
        let wait = at.saturating_duration_since(cx.background_executor().now());
        self.undo_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |this, cx| {
                let due = this.undo.due(cx.background_executor().now());
                this.commit_dismiss(due, cx);
                this.arm_undo(cx);
            })
            .ok();
        }));
    }

    fn commit_dismiss(&mut self, ids: Vec<i64>, cx: &mut Context<Self>) {
        for id in ids {
            self.write(
                cx,
                move |b| b.dismiss(id),
                |this, r, cx| match r {
                    Ok(()) => {
                        if this.subject.is_some() {
                            this.load_subject(cx);
                        }
                    }
                    Err(e) => {
                        log::warn!("insights: dismiss failed");
                        this.toasts.update(cx, |t, cx| {
                            t.push(Toast::danger(format!("Couldn't dismiss — {e}")), cx)
                        });
                    }
                },
            );
        }
    }

    pub fn toggle_pin(&mut self, id: i64, pinned: bool, cx: &mut Context<Self>) {
        self.write(
            cx,
            move |b| b.set_pinned(id, !pinned),
            move |this, r, cx| {
                let toast = match r {
                    Ok(()) if !pinned => Some(Toast::ok("Pinned — it won't fade")),
                    Ok(()) => None,
                    Err(e) => Some(Toast::danger(format!(
                        "Couldn't {} — {e}",
                        if pinned { "unpin" } else { "pin" }
                    ))),
                };
                if let Some(t) = toast {
                    this.toasts.update(cx, |ts, cx| ts.push(t, cx));
                }
                this.load_subject(cx);
            },
        );
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
            .child(
                div()
                    .w_full()
                    .max_w(px(1280.))
                    .mx_auto()
                    .px(space::S7)
                    .pt(space::S6)
                    .pb(space::S8)
                    .children(header::notice(self, &p, cx))
                    .child(match self.tab {
                        Tab::Overview => overview::render(self, &p, window, cx),
                        Tab::Journal => journal::render(self, &p, window, cx),
                        Tab::Subjects => subjects::render(self, &p, window, cx),
                        Tab::Context => context::render(self, &p, window, cx),
                    }),
            );
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
            .child(div().flex().flex_1().min_h_0().child(body).children(drawer))
            .children(overview::modal(self, &p, cx))
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

    /// The Context composer: add the draft as a standing statement.
    pub fn submit_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        let topic = self.draft_topic;
        self.composer_error = None;
        self.write(
            cx,
            move |b| b.add_authored(&text, topic),
            |this, r, cx| match r {
                Ok(_) => this.load_context(cx),
                Err(e) => this.composer_error = Some(e),
            },
        );
        self.composer
            .update(cx, |s, cx| s.set_value(String::new(), window, cx));
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
            |this, _, cx| this.load_context(cx),
        );
    }

    pub fn restore(&mut self, subject: String, statement: String, cx: &mut Context<Self>) {
        self.write(
            cx,
            move |b| b.restore_dismissed(&subject, &statement),
            |this, _, cx| this.load_context(cx),
        );
    }

    pub fn correct_category(&mut self, id: i64, c: ActivityCategory, cx: &mut Context<Self>) {
        if let Some(o) = &mut self.overview {
            if let Some(a) = o.activities.iter_mut().find(|a| a.id == id) {
                a.category = Some(c);
            }
        }
        self.write(
            cx,
            move |b| b.correct_category(id, c),
            |this, r, cx| {
                if let Err(e) = r {
                    this.toasts.update(cx, |t, cx| {
                        t.push(Toast::danger(format!("Couldn't update category — {e}")), cx)
                    });
                    this.load_overview(cx);
                }
            },
        );
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
