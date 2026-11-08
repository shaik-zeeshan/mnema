//! The Timeline surface (`routes/+page.svelte`, BUILD-PROMPT §3.6): the stage,
//! its overlays, the rail with app bands and the audio lane, the jumper and the
//! frame actions. `TimelineView` holds view state only; `paging` and
//! `scrub_engine` are the pure state machines, `controller` runs their work off
//! the main thread, `data` / `fixture` answer it.
//!
//! A lane bar, the headphones button or P sets [`TimelineView::selected_audio`];
//! render opens (or switches) the [`audio::AudioDrawer`] on it.

pub mod audio;
mod bands;
mod controller;
pub mod data;
#[cfg(feature = "fixtures")]
pub mod fixture;
mod frame_actions;
pub mod image_cache;
mod jumper;
mod match_overlay;
pub mod model;
mod ocr_overlay;
pub mod paging;
mod rail;
pub mod scrub_engine;
mod stage;
mod top_bar;

use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use app_infra::MatchBox;
use capture_types::Activity;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{actions, div, px, Context, Entity, Pixels, Role, Size, Task, Window};

use crate::keymap::{HelpGroup, HelpRow};
use crate::theme::{self, space};
use data::Source;
use image_cache::FrameImages;
use model::{AudioSegment, Landing, OcrBox};
use paging::Paging;
use scrub_engine::ScrubEngine;

actions!(
    timeline,
    [
        OlderFrame,
        NewerFrame,
        OlderFrameFast,
        NewerFrameFast,
        OpenJumper,
        JumpLatest,
        ToggleOcr,
        Refresh,
        CopyFrame,
        DownloadFrame,
        PlayMoment,
    ]
);

/// A stage note (`frameActionStatus`): neutral ones clear after 2.2 s, errors stay.
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub message: String,
    pub detail: Option<String>,
    pub error: bool,
}

/// `STATUS_TTL`: how long a neutral stage note stays (`scheduleFrameActionStatusDismiss`).
pub const STATUS_TTL: std::time::Duration = std::time::Duration::from_millis(2200);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OcrStatus {
    #[default]
    Idle,
    Running,
    Success,
    Empty,
    Missing,
    Error,
}

#[derive(Default)]
pub struct Ocr {
    pub visible: bool,
    pub status: OcrStatus,
    pub frame_id: Option<i64>,
    pub boxes: Vec<OcrBox>,
    pub error: Option<String>,
    pub hovered: Option<usize>,
}

/// What the empty stage says depends on capture (set by the shell).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CaptureHint {
    pub capturing: bool,
    pub screen: bool,
}

pub struct TimelineView {
    pub(crate) source: Arc<dyn Source>,
    pub paging: Paging,
    pub engine: ScrubEngine,
    pub images: Entity<FrameImages>,
    /// Continuous rail scroll from the newest slot, px (active = round(/8)).
    pub(crate) advance_px: f32,
    last_wheel: Option<(Instant, f32)>,
    /// Newer → older is +1; the readout flies in from this side.
    pub(crate) readout_dir: f32,
    pub(crate) hovered: Option<(usize, Pixels)>,
    pub audio: Vec<AudioSegment>,
    pub audio_loading: bool,
    pub audio_error: Option<String>,
    pub episodes: Vec<Activity>,
    pub icons: HashMap<String, PathBuf>,
    pub landing: Option<Landing>,
    pub match_boxes: Vec<MatchBox>,
    pub ocr: Ocr,
    pub menu_open: bool,
    pub busy_action: Option<&'static str>,
    pub jumper: jumper::JumperState,
    pub status: Option<Status>,
    pub capture: CaptureHint,
    pub developer: bool,
    /// The AudioDrawer's segment and the seek into it (ms); `None` closes it.
    pub selected_audio: Option<(i64, i64)>,
    pub drawer: Option<Entity<audio::AudioDrawer>>,
    /// What a drawer is built from (goldens and tests swap in fixtures).
    pub drawer_deps: audio::Deps,
    /// The audio bar under the pointer: a press there switches, never dismisses.
    pub(crate) bar_hover: Option<i64>,
    _drawer_sub: Option<gpui_kit::Subscription>,
    /// The last display whose image decoded: painted until the next one lands.
    pub(crate) painted: Option<scrub_engine::Display>,
    /// Stage and rail sizes from the last layout (the overlays and slots need them).
    pub(crate) stage_size: Rc<Cell<Size<Pixels>>>,
    pub(crate) rail_w: Rc<Cell<Pixels>>,
    pub(crate) rail_x: Rc<Cell<f32>>,
    pub(crate) runs: bands::RunsCache,
    pub max_gap_ms: i64,
    engine_timer: Option<Task<()>>,
    status_timer: Option<Task<()>>,
    _poll: Option<Task<()>>,
}

impl TimelineView {
    pub fn new(source: Arc<dyn Source>, cx: &mut Context<Self>) -> Self {
        let stage_size: Rc<Cell<Size<Pixels>>> = Rc::default();
        let mut view = Self {
            source,
            paging: Paging::default(),
            engine: ScrubEngine::default(),
            images: FrameImages::new(stage_size.clone(), cx),
            advance_px: 0.,
            last_wheel: None,
            readout_dir: 1.,
            hovered: None,
            audio: Vec::new(),
            audio_loading: false,
            audio_error: None,
            episodes: Vec::new(),
            icons: HashMap::new(),
            landing: None,
            match_boxes: Vec::new(),
            ocr: Ocr::default(),
            menu_open: false,
            busy_action: None,
            jumper: jumper::JumperState::default(),
            status: None,
            capture: CaptureHint::default(),
            developer: false,
            selected_audio: None,
            drawer: None,
            drawer_deps: audio::Deps::live(cx),
            bar_hover: None,
            _drawer_sub: None,
            painted: None,
            stage_size,
            rail_w: Rc::new(Cell::new(px(0.))),
            rail_x: Rc::default(),
            runs: bands::RunsCache::default(),
            max_gap_ms: model::app_group_max_gap_ms(None),
            engine_timer: None,
            status_timer: None,
            _poll: None,
        };
        view.start(cx);
        view
    }

    /// The help sheet's Timeline group (only while frames are loaded).
    pub fn help_group(&self) -> Option<HelpGroup> {
        if self.paging.frames.is_empty() {
            return None;
        }
        let row = |label: &str, keys: &str| HelpRow {
            label: label.into(),
            keys: keys.into(),
        };
        Some(HelpGroup {
            title: "Timeline",
            rows: vec![
                row("Older frame", "←"),
                row("Newer frame", "→"),
                row("10 frames older", "⇧←"),
                row("10 frames newer", "⇧→"),
                row("Open jump picker", "J"),
                row("Jump to latest", "L"),
                row("Toggle OCR panel", "O"),
                row("Refresh timeline", "R"),
                row("Copy active frame image", "C"),
                row("Download active frame image", "D"),
                row("Play audio at this moment", "P"),
            ],
        })
    }

    pub fn set_status(
        &mut self,
        message: Option<String>,
        detail: Option<String>,
        error: bool,
        cx: &mut Context<Self>,
    ) {
        self.status = message.map(|message| Status {
            message,
            detail,
            error,
        });
        self.status_timer = (self.status.as_ref().is_some_and(|s| !s.error)).then(|| {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(STATUS_TTL).await;
                this.update(cx, |this, cx| {
                    this.status = None;
                    cx.notify();
                })
                .ok();
            })
        });
        cx.notify();
    }

    /// Esc: close the jumper, then the frame menu. True when something closed.
    pub fn close_top(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.jumper.open || self.menu_open;
        self.jumper.open = false;
        self.menu_open = false;
        cx.notify();
        open
    }

    /// The frame the stage has painted (goldens wait for it).
    pub fn painted_frame(&self) -> Option<i64> {
        self.painted.as_ref().map(|d| d.frame_id)
    }

    pub(crate) fn hold_status(&mut self) {
        self.status_timer = None;
    }
}

impl TimelineView {
    /// Open, switch or seek the drawer to match `selected_audio`.
    fn sync_drawer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, seek)) = self.selected_audio else {
            self.drawer = None;
            self._drawer_sub = None;
            return;
        };
        if let Some(d) = &self.drawer {
            let open = d.read(cx).opened();
            if open == (id, seek) {
                return;
            }
            if open.0 == id {
                d.update(cx, |d, cx| d.seek_into(seek, cx));
                return;
            }
        }
        let Some(seg) = self.audio.iter().find(|s| s.id == id) else {
            self.selected_audio = None;
            self.drawer = None;
            return;
        };
        let seg = audio::model::Segment::seed(seg);
        if let Some(d) = &self.drawer {
            d.update(cx, |d, cx| d.open(seg, seek, cx));
            return;
        }
        let deps = self.drawer_deps.clone();
        let d = cx.new(|cx| audio::AudioDrawer::new(deps, seg, seek, window, cx));
        self._drawer_sub = Some(cx.subscribe_in(&d, window, |this, d, e, window, cx| {
            // A press on another audio bar is a switch, not a dismissal.
            if matches!(e, audio::DrawerEvent::Dismiss) && this.bar_hover.is_some() {
                return;
            }
            if let Some(f) = d.read(cx).return_focus.clone() {
                window.focus(&f, cx);
            }
            this.close_drawer(cx);
        }));
        self.drawer = Some(d);
    }

    pub fn close_drawer(&mut self, cx: &mut Context<Self>) {
        self.selected_audio = None;
        self.drawer = None;
        self._drawer_sub = None;
        cx.notify();
    }
}

/// Bind the Timeline's keys on the shell root (the dispatch path runs through
/// the shell's focus, not the surface's div).
pub fn register_actions(
    el: gpui_kit::Stateful<gpui_kit::Div>,
    view: &Entity<TimelineView>,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let v = view.clone();
    let on = move |f: fn(&mut TimelineView, &mut Window, &mut Context<TimelineView>)| {
        let v = v.clone();
        move |window: &mut Window, cx: &mut gpui_kit::App| v.update(cx, |t, cx| f(t, window, cx))
    };
    let (a, b, c, d, e, f, g, h, i, j, k) = (
        on(|t, _, cx| t.step(1, cx)),
        on(|t, _, cx| t.step(-1, cx)),
        on(|t, _, cx| t.step(10, cx)),
        on(|t, _, cx| t.step(-10, cx)),
        on(|t, _, cx| t.open_jumper(cx)),
        on(|t, _, cx| t.jump_latest(cx)),
        on(|t, _, cx| t.toggle_ocr(cx)),
        on(|t, _, cx| t.refresh(cx)),
        on(|t, w, cx| t.copy_image(w, cx)),
        on(|t, w, cx| t.download_image(w, cx)),
        on(|t, _, cx| t.play_moment(cx)),
    );
    el.on_action(move |_: &OlderFrame, w, cx| a(w, cx))
        .on_action(move |_: &NewerFrame, w, cx| b(w, cx))
        .on_action(move |_: &OlderFrameFast, w, cx| c(w, cx))
        .on_action(move |_: &NewerFrameFast, w, cx| d(w, cx))
        .on_action(move |_: &OpenJumper, w, cx| e(w, cx))
        .on_action(move |_: &JumpLatest, w, cx| f(w, cx))
        .on_action(move |_: &ToggleOcr, w, cx| g(w, cx))
        .on_action(move |_: &Refresh, w, cx| h(w, cx))
        .on_action(move |_: &CopyFrame, w, cx| i(w, cx))
        .on_action(move |_: &DownloadFrame, w, cx| j(w, cx))
        .on_action(move |_: &PlayMoment, w, cx| k(w, cx))
}

/// The Timeline's bindings: unmodified keys, suppressed in inputs and buttons.
pub fn bind_keys(cx: &mut gpui_kit::App) {
    use gpui_kit::KeyBinding;
    let ctx = Some(crate::keymap::APP_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("left", OlderFrame, ctx),
        KeyBinding::new("right", NewerFrame, ctx),
        KeyBinding::new("shift-left", OlderFrameFast, ctx),
        KeyBinding::new("shift-right", NewerFrameFast, ctx),
        KeyBinding::new("j", OpenJumper, ctx),
        KeyBinding::new("l", JumpLatest, ctx),
        KeyBinding::new("o", ToggleOcr, ctx),
        KeyBinding::new("r", Refresh, ctx),
        KeyBinding::new("c", CopyFrame, ctx),
        KeyBinding::new("d", DownloadFrame, ctx),
        KeyBinding::new("p", PlayMoment, ctx),
    ]);
}

impl Render for TimelineView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = theme::tokens(cx);
        self.settle_painted(window, cx);
        self.sync_drawer(window, cx);
        div()
            .id("timeline")
            .role(Role::Main)
            .aria_label("Timeline")
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .w_full()
            .gap(space::S3)
            .px(space::S4)
            .py(space::S3)
            .overflow_hidden()
            .on_scroll(cx.listener(|this, e, window, cx| this.on_wheel(e, window, cx)))
            .child(top_bar::render(self, &p, cx))
            .child(stage::render(self, &p, window, cx))
            .child(rail::render(self, &p, window, cx))
            .children(jumper::render(self, &p, cx))
            .children(self.drawer.clone())
            .test_support()
    }
}

#[cfg(all(test, feature = "fixtures"))]
mod tests;
