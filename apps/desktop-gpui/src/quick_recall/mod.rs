//! Quick Recall (BUILD-PROMPT §3.4 / §3.6): the summon-anywhere bar. One
//! [`QuickRecall`] entity holds every piece of view state (query, pills, results,
//! the ask slot, Quick Look) inside a reused, never-destroyed NSPanel
//! (`panel.rs`). Behaviour is split by concern into `impl QuickRecall` blocks:
//! `search.rs`, `ask.rs`, `look.rs`, `keys.rs`; drawing into `view.rs`,
//! `field.rs`, `results.rs`, `states_view.rs`, `look_view.rs`, `ask_view.rs`.
//! Timers are GPUI tasks on the background executor's clock, so `advance_clock`
//! reaches every §5 constant.

mod ask;
mod ask_view;
pub mod data;
mod field;
#[cfg(feature = "fixtures")]
pub mod fixture;
mod keys;
mod look;
mod look_side;
mod look_view;
pub mod model;
pub mod panel;
mod results;
pub mod rules;
mod search;
mod states_view;
pub mod tokens;
mod view;

#[cfg(all(test, feature = "fixtures"))]
mod tests;

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use app_infra::RecentQuery;
use capture_types::RecordingSettings;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{App, AppContext as _, Context, Entity, Subscription, Task, Window};

pub use ask::AskState;
use data::{Availability, Backend};
pub use look::Look;
use model::{IndexStatus, Ranked};
pub use search::Results;
use tokens::{Forced, Parse, Pill};

/// Search waits this long after the last keystroke (`searchStore` `DEBOUNCE_MS`).
pub const SEARCH_DEBOUNCE: Duration = Duration::from_millis(250);
/// Recognised phrases lift into pills after this pause (`COMMIT_MS`).
pub const COMMIT: Duration = Duration::from_millis(700);
/// A question is answered once it holds still this long (`AskPane` `PAUSE_MS`).
pub const ASK_PAUSE: Duration = Duration::from_millis(750);
/// A blur this soon after a summon is AppKit's setup blur, not a click away.
pub const BLUR_GRACE: Duration = Duration::from_millis(300);
/// Hidden this long with something on it: back to the empty bar.
pub const RESET_AFTER_HIDE: Duration = Duration::from_millis(5000);
/// "Copied" on ⌘C (Quick Look 1400, the answer 1500).
pub const COPIED_LOOK: Duration = Duration::from_millis(1400);
pub const COPIED_ANSWER: Duration = Duration::from_millis(1500);

/// The bar (`recall.html` `.rc-float`): width, idle height, field, foot, body cap.
pub const WIDTH: f32 = 800.;
pub const IDLE_H: f32 = 60.;
pub const FIELD_H: f32 = 58.;
pub const FOOT_H: f32 = 32.;
pub const CAP_H: f32 = 640.;
pub const LOOK_H: f32 = 476.;
pub const CORNER: f32 = 16.;
/// Where the top edge sits, as a fraction of the work area's height.
pub const TOP_FRACTION: f32 = 0.15;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Search,
    Ask,
}

/// The ask slot: "Press ↵ to ask" (auto-answer off) or the answer itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AskView {
    Gate,
    Ask,
}

/// Which state replaces the results list once a search has answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmptyState {
    First,
    Error,
    None,
}

/// The pressed pill and its popover.
#[derive(Default)]
pub struct Filters {
    pub pill: Option<usize>,
    pub popover: bool,
    pub index: usize,
    /// Option counts keyed by `PillOption::op` (None until they arrive).
    pub counts: Option<Vec<(String, String)>>,
    gen: u64,
}

#[derive(Default)]
pub struct Recent {
    pub list: Vec<RecentQuery>,
    pub open: bool,
    pub index: usize,
}

#[derive(Default)]
struct Timers {
    debounce: Option<Task<()>>,
    commit: Option<Task<()>>,
    reset: Option<Task<()>>,
    copied: Option<Task<()>>,
}

pub struct QuickRecall {
    backend: Arc<dyn Backend>,
    pub input: Entity<InputState>,
    /// The field's text (InputState's value, mirrored on every change).
    pub text: String,
    pub caret_at_end: bool,
    pub parse: Parse,
    pub pills: Vec<Pill>,
    pub apps: Option<Vec<String>>,
    apps_loading: bool,
    /// App id or name → icon file.
    pub icons: HashMap<String, PathBuf>,
    pub results: Results,
    /// Thumbnail frame id → preview file.
    pub thumbs: HashMap<i64, PathBuf>,
    pub filters: Filters,
    pub recent: Recent,
    /// No-results ways out with their counts (None = Ask), and the lit row.
    pub ways: Vec<(rules::WayOut, Option<String>)>,
    pub way_sel: usize,
    ways_gen: u64,
    pub index_status: Option<IndexStatus>,
    pub recording: Option<bool>,
    pub availability: Option<Availability>,
    pub settings: Option<RecordingSettings>,
    pub forced: Option<Forced>,
    pub ask: AskState,
    pub look: Option<Look>,
    /// Arrowed into the list: Space looks instead of typing a space.
    pub look_nav: bool,
    /// The panel is the key window.
    pub focused: bool,
    /// The panel is on screen (ordered in).
    pub visible: bool,
    pub online: bool,
    summoned_at: Option<Instant>,
    suppress_blur: bool,
    /// Dismissals by blur / Esc / toggle (tests read it).
    pub dismissals: u32,
    /// Text for the InputState, applied on the next render (it needs the window).
    pending_text: Option<String>,
    last_operator_query: String,
    last_ask_view: Option<AskView>,
    timers: Timers,
    pub images: Entity<crate::timeline::image_cache::FrameImages>,
    /// The shell's last measured height (the panel follows it).
    reported_h: Rc<Cell<f32>>,
    /// Fit the real panel frame (off for offscreen goldens: they resize the window).
    pub native_fit: bool,
    _subs: Vec<Subscription>,
}

impl QuickRecall {
    pub fn new(backend: Arc<dyn Backend>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx));
        let sub = cx.subscribe_in(&input, window, |this, _, ev, window, cx| {
            if let InputEvent::Change = ev {
                this.on_input(window, cx);
            }
        });
        // No size limit: the best-moment crop zooms in, so it needs full-resolution frames.
        let images = crate::timeline::image_cache::FrameImages::new(Default::default(), cx);
        let activation = cx.observe_window_activation(window, |qr, window, cx| {
            qr.on_activation(window.is_window_active(), cx)
        });
        // The WKWebView's window capture listener: keys reach the bar ahead of
        // the field's bindings (Tab, Enter, arrows, Esc, ⌫ at the start).
        let me = cx.entity().downgrade();
        let win = window.window_handle().window_id();
        let keys = cx.intercept_keystrokes(move |ev, window, cx| {
            if window.window_handle().window_id() != win {
                return;
            }
            let used = me.update(cx, |qr, cx| qr.key(&ev.keystroke, cx)).unwrap_or(false);
            if used {
                cx.stop_propagation();
            }
        });
        let mut qr = Self {
            backend,
            input,
            text: String::new(),
            caret_at_end: true,
            parse: Parse::default(),
            pills: Vec::new(),
            apps: None,
            apps_loading: false,
            icons: HashMap::new(),
            results: Results::default(),
            thumbs: HashMap::new(),
            filters: Filters::default(),
            recent: Recent::default(),
            ways: Vec::new(),
            way_sel: 0,
            ways_gen: 0,
            index_status: None,
            recording: None,
            availability: None,
            settings: None,
            forced: None,
            ask: AskState::default(),
            look: None,
            look_nav: false,
            focused: true,
            visible: false,
            online: true,
            summoned_at: None,
            suppress_blur: false,
            dismissals: 0,
            pending_text: None,
            last_operator_query: String::new(),
            last_ask_view: None,
            timers: Timers::default(),
            images,
            reported_h: Rc::new(Cell::new(IDLE_H)),
            native_fit: true,
            _subs: vec![sub, activation, keys],
        };
        qr.load_ambient(cx);
        qr.ensure_apps(cx);
        qr
    }

    // ── Derived state ───────────────────────────────────────────────────────

    pub fn effective_pills(&self) -> Vec<Pill> {
        let spans: Vec<Pill> = self.parse.spans.iter().map(|s| s.pill.clone()).collect();
        tokens::merge_pills(&self.pills, &spans)
    }

    pub fn operator_query(&self) -> String {
        tokens::operator_query(&self.parse.residual, &self.effective_pills())
    }

    pub fn is_empty(&self) -> bool {
        self.operator_query().trim().is_empty()
    }

    pub fn below_min(&self) -> bool {
        self.operator_query().trim().chars().count() < rules::MIN_QUERY_LENGTH
    }

    pub fn engine(&self) -> rules::AskEngine {
        rules::ask_engine(self.settings.as_ref())
    }

    pub fn ask_available(&self) -> bool {
        self.availability.as_ref().is_some_and(|a| a.available)
    }

    fn question(&self) -> bool {
        tokens::is_question(&self.parse.residual)
    }

    pub fn mode(&self) -> Mode {
        let auto = self.question() && rules::auto_answer_allowed(self.settings.as_ref(), &self.engine());
        let ask = match self.forced {
            Some(f) => f == Forced::Ask,
            None => auto,
        };
        if !self.is_empty() && ask {
            Mode::Ask
        } else {
            Mode::Search
        }
    }

    pub fn ask_view(&self) -> Option<AskView> {
        if self.mode() == Mode::Ask {
            Some(AskView::Ask)
        } else if !self.is_empty() && self.forced.is_none() && self.question() && self.ask_available() {
            Some(AskView::Gate)
        } else {
            None
        }
    }

    pub fn show_recent(&self) -> bool {
        self.recent.open && self.is_empty()
    }

    /// Idle = the bare 60 px bar.
    pub fn show_body(&self) -> bool {
        self.ask_view().is_some() || !self.below_min() || self.filters.popover || self.show_recent() || self.look.is_some()
    }

    pub fn empty_state(&self) -> Option<EmptyState> {
        let r = &self.results;
        if self.below_min() || r.loading || self.results_paused() {
            return None;
        }
        if r.error.is_some() {
            return Some(EmptyState::Error);
        }
        if r.index_empty && !r.has_results() {
            return Some(EmptyState::First);
        }
        let none = !r.has_results() && !r.query.is_empty();
        none.then_some(EmptyState::None)
    }

    /// The token the backend rejected, while the shown results are this query's.
    pub fn bad_token(&self) -> Option<String> {
        let e = self.results.parse_errors.first()?;
        (self.results.query == self.operator_query().trim()).then(|| e.token.clone())
    }

    pub fn results_paused(&self) -> bool {
        let r = &self.results;
        !self.below_min() && !r.loading && r.error.is_none() && !r.parse_errors.is_empty()
    }

    pub fn visible_results(&self) -> &[Ranked] {
        let n = if self.results.expanded {
            self.results.ranked.len()
        } else {
            self.results.ranked.len().min(rules::VISIBLE_CAP)
        };
        &self.results.ranked[..n]
    }

    /// The bar's last measured height (what the panel was asked to be).
    pub fn content_height(&self) -> f32 {
        self.reported_h.get()
    }

    pub fn now(&self, cx: &App) -> time::OffsetDateTime {
        crate::ui::clock::now(cx)
    }

    // ── Plumbing ────────────────────────────────────────────────────────────

    /// Run `f` on the background executor, then `then` on the entity.
    fn fetch<T: Send + 'static>(
        &self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&dyn Backend) -> T + Send + 'static,
        then: impl FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
    ) {
        let backend = self.backend.clone();
        cx.spawn(async move |this, cx| {
            let out = cx.background_executor().spawn(async move { f(backend.as_ref()) }).await;
            this.update(cx, |qr, cx| {
                then(qr, out, cx);
                qr.refresh(cx);
            })
            .ok();
        })
        .detach();
    }

    /// A timer on the executor clock that runs `f` once it fires.
    fn after(&self, d: Duration, cx: &mut Context<Self>, f: impl FnOnce(&mut Self, &mut Context<Self>) + 'static) -> Task<()> {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(d).await;
            this.update(cx, |qr, cx| {
                f(qr, cx);
                qr.refresh(cx);
            })
            .ok();
        })
    }

    /// Re-read what the bar says around the results: Ask AI, the settings, the
    /// indexing backlog and whether Mnema records (mount, every summon, a setting change).
    pub fn load_ambient(&mut self, cx: &mut Context<Self>) {
        self.fetch(
            cx,
            |b| (b.availability(), b.settings()),
            |qr, (a, s), _| {
                qr.availability = Some(a);
                if s.is_some() {
                    qr.settings = s;
                }
            },
        );
        self.load_index_status(cx);
    }

    fn load_index_status(&mut self, cx: &mut Context<Self>) {
        self.fetch(
            cx,
            |b| (b.index_status(), b.recording()),
            |qr, (s, r), _| {
                qr.index_status = s;
                if r.is_some() {
                    qr.recording = r;
                }
            },
        );
    }

    /// The captured-app names behind "in Arc", the ghost and the app pill.
    pub fn ensure_apps(&mut self, cx: &mut Context<Self>) {
        if self.apps.is_some() || self.apps_loading {
            return;
        }
        self.apps_loading = true;
        self.fetch(
            cx,
            |b| {
                let apps = b.apps();
                let icons = apps.as_ref().map(|a| b.app_icons(a)).unwrap_or_default();
                (apps, icons)
            },
            |qr, (apps, icons), cx| {
                qr.apps_loading = false;
                qr.icons.extend(icons);
                if apps.is_some() {
                    qr.apps = apps;
                    qr.reparse(cx);
                }
            },
        );
    }

    fn ensure_icons(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        let ids: Vec<String> = ids.into_iter().filter(|i| !self.icons.contains_key(i)).collect();
        if ids.is_empty() {
            return;
        }
        self.fetch(cx, move |b| b.app_icons(&ids), |qr, icons, _| qr.icons.extend(icons));
    }

    /// Text changed (typed, or set by us): re-tint, then let the pauses run.
    fn on_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.input.read(cx);
        self.text = input.value().to_string();
        self.caret_at_end = input.cursor() == self.text.len() && input.selected_range().is_empty();
        self.filters.pill = None;
        self.filters.popover = false;
        if !self.text.is_empty() {
            self.recent.open = false;
        }
        if self.look.is_some() {
            self.look = None;
            self.look_nav = false;
        }
        self.reparse(cx);
        self.schedule_commit(cx);
        let _ = window;
        self.refresh(cx);
    }

    fn reparse(&mut self, cx: &App) {
        let apps = self.apps.clone().unwrap_or_default();
        self.parse = tokens::parse_natural(&self.text, self.now(cx), &apps);
    }

    /// Set the field's text from code (commit, Recent, ways out, reset). The
    /// InputState takes it on the next render, which has the window.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.text = text.to_string();
        self.caret_at_end = true;
        self.pending_text = Some(text.to_string());
        self.reparse(cx);
    }

    /// The one place derived reactions run (the Svelte `$effect`s): search when the
    /// operator query moves, re-highlight when the ask slot changes, re-arm the
    /// ask pause, and the reset-after-hide countdown.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.is_empty() {
            self.forced = None;
        }
        let q = self.operator_query();
        if q != self.last_operator_query {
            self.last_operator_query = q.clone();
            self.schedule_search(&q, cx);
        }
        let view = self.ask_view();
        if view != self.last_ask_view {
            self.last_ask_view = view;
            self.reselect();
        }
        self.ask_sync(cx);
        self.sync_reset(cx);
        cx.notify();
    }

    /// Something to reset: anything but the pristine empty bar.
    fn clearable(&self) -> bool {
        !self.operator_query().trim().is_empty()
            || self.results.has_results()
            || self.results.error.is_some()
            || !self.results.query.is_empty()
    }

    /// Arm the 5 s reset while the panel is away with something on it and nothing
    /// running; an answer streaming, unseen or handed to main is never wiped.
    fn sync_reset(&mut self, cx: &mut Context<Self>) {
        let running = self.results.loading || self.ask.live();
        let arm = !self.focused && self.clearable() && !running && !self.ask.pending();
        if !arm {
            self.timers.reset = None;
        } else if self.timers.reset.is_none() {
            self.timers.reset = Some(self.after(RESET_AFTER_HIDE, cx, |qr, cx| {
                qr.timers.reset = None;
                qr.clear_state(cx);
            }));
        }
    }

    /// Back to the just-summoned empty bar.
    pub fn clear_state(&mut self, cx: &mut Context<Self>) {
        self.forced = None;
        self.clear_search_state(cx);
        self.ask_reset(cx);
        self.look = None;
        self.look_nav = false;
        self.recent.open = false;
        self.set_text("", cx);
        cx.notify();
    }
}

pub fn now_instant(cx: &App) -> Instant {
    cx.background_executor().now()
}
