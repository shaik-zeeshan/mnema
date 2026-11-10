//! The developer window (`routes/debug/+page.svelte`, `lib/debug/*`; 520×640 /
//! min 440×480, gated on developer options): strip, verdict, five tabs and one
//! scrolling body, or the 300×120 always-on-top mini view. One 1 s ticker runs
//! the polls; each [`Section`] fetches at its own cadence while its tab shows,
//! and nothing runs while live updates are paused.

mod ai;
mod capture;
mod chrome;
pub mod data;
mod logs;
pub mod model;
pub mod readouts;
mod tabs;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::{
    px, size, App, AppContext, ClipboardItem, Context, Entity, KeyDownEvent, Render, ScrollHandle,
    Task, Window,
};

use data::{DebugBackend, DebugData, Section};
use model::Tab;
use readouts::Level;

pub const SIZE: (f32, f32) = (520., 640.);
pub const MIN: (f32, f32) = (440., 480.);
pub const MINI: (f32, f32) = (300., 120.);
const TICK: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Full,
    Mini,
}

pub struct DebugWindow {
    backend: Arc<dyn DebugBackend>,
    pub data: DebugData,
    pub tab: Tab,
    pub mode: Mode,
    pub live: bool,
    pub paused_at_ms: Option<i64>,
    pub pinned: bool,
    pub menu_open: bool,
    pub level: Level,
    pub feature: usize,
    pub follow: bool,
    pub needle: Entity<InputState>,
    pub(crate) body_scroll: ScrollHandle,
    pub(crate) log_scroll: ScrollHandle,
    /// The full-mode size to return to from mini.
    restore: Option<gpui_kit::Size<gpui_kit::Pixels>>,
    last: HashMap<Section, Instant>,
    in_flight: HashSet<Section>,
    /// The last button's outcome: `(ok, line)`; and the one in flight.
    pub note: Option<(bool, String)>,
    pub acting: Option<data::Action>,
    _tick: Task<()>,
}

impl DebugWindow {
    pub fn new(
        backend: Arc<dyn DebugBackend>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let needle = cx.new(|cx| InputState::new(window, cx).placeholder("Filter lines"));
        cx.observe(&needle, |_, _, cx| cx.notify()).detach();
        let tick = cx.spawn(async move |this, cx| loop {
            if this.update(cx, |w, cx| w.poll(cx)).is_err() {
                break;
            }
            cx.background_executor().timer(TICK).await;
        });
        Self {
            backend,
            data: DebugData::default(),
            tab: Tab::Overview,
            mode: Mode::Full,
            live: true,
            paused_at_ms: None,
            pinned: false,
            menu_open: false,
            level: Level::All,
            feature: 0,
            follow: true,
            needle,
            body_scroll: ScrollHandle::new(),
            log_scroll: ScrollHandle::new(),
            restore: None,
            last: HashMap::new(),
            in_flight: HashSet::new(),
            note: None,
            acting: None,
            _tick: tick,
        }
    }

    /// Run a button's command off the main thread, show what came back, then
    /// re-read everything at once.
    pub fn act(&mut self, action: data::Action, cx: &mut Context<Self>) {
        if self.acting.is_some() {
            return;
        }
        self.acting = Some(action);
        let backend = self.backend.clone();
        let job = cx.background_spawn(async move { backend.act(action) });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |w, cx| {
                w.acting = None;
                w.note = match result {
                    Ok(line) => line.map(|l| (true, l)),
                    Err(e) => Some((false, e)),
                };
                w.last.clear();
                w.poll(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The sections the current view reads (the health rollup in both modes).
    fn sections(&self) -> Vec<Section> {
        let mut s = vec![Section::Health];
        match (self.mode, self.tab) {
            (Mode::Mini, _) | (Mode::Full, Tab::Pipeline) => {
                s.push(Section::Pipeline);
                if self.mode == Mode::Full {
                    s.push(Section::Housekeeping);
                }
            }
            (Mode::Full, Tab::Capture) => s.push(Section::Capture),
            (Mode::Full, Tab::Ai) => s.push(Section::Ai),
            (Mode::Full, Tab::Logs) => s.push(Section::Logs),
            (Mode::Full, Tab::Overview) => {}
        }
        s
    }

    /// Fetch every due section (skipped while paused, or while its last call is in flight).
    pub fn poll(&mut self, cx: &mut Context<Self>) {
        if !self.live {
            return;
        }
        let now = cx.background_executor().now();
        let now_ms = crate::ui::clock::now_ms(cx);
        for section in self.sections() {
            let due = self
                .last
                .get(&section)
                .is_none_or(|at| now.duration_since(*at) >= section.every());
            if !due || !self.in_flight.insert(section) {
                continue;
            }
            self.last.insert(section, now);
            let backend = self.backend.clone();
            let job = cx.background_spawn(async move { backend.fetch(section, now_ms) });
            cx.spawn(async move |this, cx| {
                let got = job.await;
                let _ = this.update(cx, |w, cx| {
                    w.in_flight.remove(&section);
                    w.data.merge(section, got);
                    if section == Section::Logs && w.follow {
                        w.log_scroll.scroll_to_bottom();
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// Switch tab; the new tab's reads start now.
    pub fn go(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        self.menu_open = false;
        self.poll(cx);
        cx.notify();
    }

    pub fn toggle_live(&mut self, cx: &mut Context<Self>) {
        self.live = !self.live;
        self.paused_at_ms = (!self.live).then(|| crate::ui::clock::now_ms(cx));
        if self.live {
            // Resume refreshes everything at once rather than waiting out each interval.
            self.last.clear();
            self.poll(cx);
        }
        cx.notify();
    }

    pub fn set_pinned(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.pinned = on;
        crate::native::set_always_on_top(window, on);
        cx.notify();
    }

    /// Lower the min size before shrinking, or the shrink is clamped.
    pub fn to_mini(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == Mode::Mini {
            return;
        }
        self.menu_open = false;
        self.mode = Mode::Mini;
        self.restore = Some(window.viewport_size());
        crate::native::set_min_size(window, MINI.0, MINI.1);
        window.resize(size(px(MINI.0), px(MINI.1)));
        crate::native::set_always_on_top(window, true);
        self.poll(cx);
        cx.notify();
    }

    /// Restore the size before raising the min, then restore the pin.
    pub fn expand(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == Mode::Full {
            return;
        }
        let back = self.restore.take().unwrap_or(size(px(SIZE.0), px(SIZE.1)));
        window.resize(back);
        crate::native::set_min_size(window, MIN.0, MIN.1);
        crate::native::set_always_on_top(window, self.pinned);
        self.mode = Mode::Full;
        self.poll(cx);
        cx.notify();
    }

    /// Health, versions, queues and the last 200 log lines, for a bug report.
    pub fn diagnostics(&self) -> String {
        let mut out = format!("Mnema {}\n\n# Health\n", self.data.version);
        for r in &self.data.health {
            out.push_str(&format!(
                "{}: {} \u{b7} {}\n",
                r["feature"].as_str().unwrap_or(""),
                r["severity"].as_str().unwrap_or(""),
                r["reason"].as_str().unwrap_or("")
            ));
        }
        out.push_str("\n# Queues\n");
        for l in &self.data.lanes {
            out.push_str(&format!(
                "{}: {} queued, {} running, {} failed (24 h)\n",
                l["processor"].as_str().unwrap_or(""),
                l["queued"],
                l["running"],
                l["failedLast24h"]
            ));
        }
        if let Some(lines) = self
            .data
            .log_tail
            .as_ref()
            .and_then(|t| t["lines"].as_array())
        {
            out.push_str("\n# Log\n");
            for l in lines.iter().rev().take(200).rev() {
                out.push_str(l.as_str().unwrap_or(""));
                out.push('\n');
            }
        }
        out
    }

    pub fn copy_diagnostics(&mut self, cx: &mut Context<Self>) {
        self.menu_open = false;
        cx.write_to_clipboard(ClipboardItem::new_string(self.diagnostics()));
        cx.notify();
    }

    fn on_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        if k.key == "escape" {
            cx.stop_propagation();
            if self.menu_open {
                self.menu_open = false;
                cx.notify();
            } else {
                window.remove_window();
            }
        } else if k.key == "c" && k.modifiers.platform && k.modifiers.shift && !k.modifiers.alt {
            cx.stop_propagation();
            self.copy_diagnostics(cx);
        }
    }
}

impl Render for DebugWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::theme::request_fade_frames(window, cx);
        let p = crate::theme::tokens(cx);
        let root = super::parts::frame(&p)
            .key_context("DebugWindow")
            .on_key_down(cx.listener(Self::on_key));
        match self.mode {
            Mode::Mini => root.child(chrome::mini(self, cx, &p)),
            Mode::Full => root
                .child(chrome::strip(self, cx, &p))
                .child(chrome::verdict(self, cx, &p))
                .child(chrome::tab_bar(self, cx, &p))
                .child(tabs::body(self, window, cx, &p))
                .children(chrome::more_menu(self, cx, &p)),
        }
    }
}

/// Open the developer window (or bring it forward). Callers check developer options.
pub fn open(cx: &mut App) {
    let Some(core) = crate::chat::core(cx) else {
        log::warn!("debug: no app core; the developer window can't open");
        return;
    };
    super::open_or_focus::<DebugWindow>(
        super::Kind::Debug,
        cx,
        |window, cx| {
            cx.new(|cx| {
                // Closing it (×, Esc) brings main forward, as the Tauri app does.
                cx.on_release(|_, cx| cx.defer(crate::window::focus_main_if_visible))
                    .detach();
                DebugWindow::new(Arc::new(data::CoreDebug(core)), window, cx)
            })
        },
        |_, _, _| {},
    );
}
