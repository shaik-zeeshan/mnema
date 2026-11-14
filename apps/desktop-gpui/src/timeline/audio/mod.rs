//! The Timeline's AudioDrawer (`AudioDrawer.svelte`, BUILD-PROMPT §3.6): a
//! peek sheet (50% of the window) that expands into a full reader, with the
//! karaoke transcript, the speaker-coloured waveform scrubber, the speaker repair
//! slide-over and native playback.
//!
//! The drawer owns its view state and playback; loads and writes go through
//! [`data::Backend`] on the background executor. Keys live in the `AudioDrawer`
//! context (the drawer takes focus on open), so ←/→ seek here instead of
//! stepping frames, exactly as the Svelte drawer's own keydown handler did.

pub mod data;
#[cfg(feature = "fixtures")]
pub mod fixture;
mod header;
pub mod model;
mod panels;
mod reader;
mod repair;
pub mod transport;
pub mod view_model;

use std::cell::Cell;
use std::collections::HashSet;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::{
    actions, div, px, Animation, AnimationExt as _, Bounds, Context, EventEmitter, FocusHandle,
    Pixels, Role, ScrollHandle, SharedString, Task, Window,
};

use crate::ui::Tracking as _;
use crate::theme::{self, motion, radius, space, text, FONT_MONO};
use data::{Backend, Transcript, Write};
use model::{Run, Segment};
use transport::{Player, Transport};
use view_model::{self as vm, Bar, Group, Marks, Panel, StripSpeaker};

actions!(
    audio_drawer,
    [
        PlayPause,
        SeekBack,
        SeekForward,
        SeekBackFast,
        SeekForwardFast,
        Escape
    ]
);

/// `AUDIO_TRANSCRIPT_POLL_INTERVAL_MS`: a queued/running job is re-read this often.
pub const POLL: Duration = Duration::from_millis(1000);
/// The drawer's key context: transport keys apply here, never while typing.
pub const CONTEXT: &str = "AudioDrawer && !Input";
/// ←/→ and ⇧←/⇧→, seconds.
pub const SEEK_STEP: f64 = 5.;
pub const SEEK_STEP_FAST: f64 = 30.;
/// Peek height (50vh), and the room the repair slide-over needs (76vh).
const PEEK: f32 = 0.5;
const PEEK_REPAIR: f32 = 0.76;
const RISE: Pixels = px(16.);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seek {
    By(f64),
    ToMs(i64),
    To(f64),
}

pub type OpenPlayer = Arc<dyn Fn(&Path) -> Result<Box<dyn Player + Send>, String> + Send + Sync>;

/// What a drawer is built from (the Timeline holds one; tests swap in fixtures).
#[derive(Clone)]
pub struct Deps {
    pub backend: Arc<dyn Backend>,
    pub open_player: OpenPlayer,
}

impl Deps {
    /// The real app: AppCore (when there is one) and AVAudioPlayer.
    pub fn live(cx: &gpui_kit::App) -> Self {
        let backend: Arc<dyn Backend> = match crate::chat::core(cx) {
            Some(core) => Arc::new(data::CoreBackend(core)),
            None => Arc::new(data::NoBackend),
        };
        Self {
            backend,
            open_player: Arc::new(|path| {
                crate::native::audio_player::AudioPlayer::open(path)
                    .map(|p| Box::new(p) as Box<dyn Player + Send>)
            }),
        }
    }
}

pub enum DrawerEvent {
    Close,
    /// A press outside the drawer (the Timeline ignores it over an audio bar).
    Dismiss,
}

/// The repair slide-over's target and form (`SpeakerRepairPanel`'s state).
pub struct Repair {
    pub index: usize,
    /// The cluster it was opened on: a merge shifts indices, the cluster stays.
    pub cluster: i64,
    pub line_scope: bool,
    pub pick: Option<repair::Pick>,
    pub move_to: Option<i64>,
    pub close_on_success: bool,
    pub last: Option<Write>,
}

pub struct AudioDrawer {
    deps: Deps,
    pub segment: Segment,
    pub data: Transcript,
    pub transport: Transport,
    pub media_loading: bool,
    pub media_error: Option<String>,
    pub load_error: Option<String>,
    pub rerun_loading: bool,
    pub rerun_error: Option<String>,
    pub speaker_retry_loading: bool,
    pub developer: bool,
    pub expanded: bool,
    pub show_timestamps: bool,
    pub follow_detached: bool,
    pub ignore_speaker_failure: bool,
    pub selected_cluster: Option<i64>,
    pub only_selected: bool,
    pub repair: Option<Repair>,
    pub busy_cluster: Option<i64>,
    pub correction_error: Option<String>,
    pub soft_dismissed: Vec<i64>,
    pub groups: Vec<Group>,
    pub strip: Vec<StripSpeaker>,
    pub marks: Marks,
    pub bars: Vec<Bar>,
    peaks: Vec<f32>,
    /// Each paragraph's timed words, shared (not cloned) into every render.
    karaoke: Vec<Option<Rc<[Run]>>>,
    pub(crate) wave_bounds: Rc<Cell<Bounds<Pixels>>>,
    pub(crate) wave_hover: Option<Pixels>,
    pending_seek: Option<i64>,
    followed: Option<usize>,
    pub(crate) scroll: ScrollHandle,
    pub(crate) name: gpui_kit::Entity<InputState>,
    pub focus: FocusHandle,
    /// Where focus goes back to when the drawer closes.
    pub return_focus: Option<FocusHandle>,
    opened: (i64, i64),
    gen: u64,
    load_task: Option<Task<()>>,
    poll_task: Option<Task<()>>,
    /// Polls the player while it plays (`transport::poll_playback`).
    play_task: Option<Task<()>>,
    media_task: Option<Task<()>>,
    segment_task: Option<Task<()>>,
    peaks_task: Option<Task<()>>,
    write_task: Option<Task<()>>,
}

impl EventEmitter<DrawerEvent> for AudioDrawer {}

impl AudioDrawer {
    pub fn new(
        deps: Deps,
        segment: Segment,
        seek_ms: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Type a name…"));
        cx.subscribe(
            &name,
            |this: &mut Self, _, e: &gpui_kit::component::input::InputEvent, cx| {
                if matches!(e, gpui_kit::component::input::InputEvent::Change) {
                    if let Some(r) = this.repair.as_mut() {
                        r.pick = None;
                    }
                    cx.notify();
                }
            },
        )
        .detach();
        let focus = cx.focus_handle();
        let return_focus = window.focused(cx);
        window.focus(&focus, cx);
        let mut d = Self {
            deps,
            segment: segment.clone(),
            data: Transcript::default(),
            transport: Transport::default(),
            media_loading: false,
            media_error: None,
            load_error: None,
            rerun_loading: false,
            rerun_error: None,
            speaker_retry_loading: false,
            developer: crate::shell::developer_options(cx),
            expanded: false,
            show_timestamps: false,
            follow_detached: false,
            ignore_speaker_failure: false,
            selected_cluster: None,
            only_selected: false,
            repair: None,
            busy_cluster: None,
            correction_error: None,
            soft_dismissed: Vec::new(),
            groups: Vec::new(),
            strip: Vec::new(),
            marks: Marks::new(),
            bars: Vec::new(),
            peaks: Vec::new(),
            karaoke: Vec::new(),
            wave_bounds: Rc::default(),
            wave_hover: None,
            pending_seek: None,
            followed: None,
            scroll: ScrollHandle::new(),
            name,
            focus,
            return_focus,
            opened: (segment.id, seek_ms),
            gen: 0,
            load_task: None,
            poll_task: None,
            play_task: None,
            media_task: None,
            segment_task: None,
            peaks_task: None,
            write_task: None,
        };
        d.open(segment, seek_ms, cx);
        d
    }

    /// Show another segment: per-segment view state resets, density preferences stay.
    pub fn open(&mut self, segment: Segment, seek_ms: i64, cx: &mut Context<Self>) {
        self.gen += 1;
        self.opened = (segment.id, seek_ms);
        self.segment = segment;
        self.data = Transcript {
            status: vm::TranscriptStatus::Loading,
            ..Default::default()
        };
        self.transport = Transport::default();
        self.peaks.clear();
        self.follow_detached = false;
        self.followed = None;
        self.repair = None;
        self.ignore_speaker_failure = false;
        self.selected_cluster = None;
        self.only_selected = false;
        self.soft_dismissed.clear();
        self.media_error = None;
        self.load_error = None;
        self.rerun_error = None;
        self.rerun_loading = false;
        self.correction_error = None;
        self.pending_seek = (seek_ms > 0).then_some(seek_ms);
        self.derive();
        self.load(cx);
        self.load_segment(cx);
        self.load_peaks(cx);
        cx.notify();
    }

    /// The `(segment, seek)` this drawer was last asked to show.
    pub fn opened(&self) -> (i64, i64) {
        self.opened
    }

    /// Play-this-moment into the segment already open.
    pub fn seek_into(&mut self, seek_ms: i64, cx: &mut Context<Self>) {
        self.opened.1 = seek_ms;
        if self.transport.playable() {
            self.seek_to_ms(seek_ms);
        } else {
            self.pending_seek = Some(seek_ms);
        }
        cx.notify();
    }

    /// Collapse the repair panel; false when there was none.
    pub fn collapse_repair(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.repair.take().is_some();
        cx.notify();
        open
    }

    /// `get_audio_segment` (the file path and session), then the player.
    fn load_segment(&mut self, cx: &mut Context<Self>) {
        let (b, id, gen) = (self.deps.backend.clone(), self.segment.id, self.gen);
        self.media_loading = true;
        let task = cx.background_spawn(async move { b.segment(id) });
        self.segment_task = Some(cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |d, cx| {
                if d.gen != gen {
                    return;
                }
                match res {
                    Ok(Some(seg)) => {
                        d.segment = seg;
                        d.load_media(cx);
                    }
                    Ok(None) => {
                        d.media_loading = false;
                        d.media_error = Some(format!("audio segment {id} not found"));
                    }
                    Err(e) => {
                        d.media_loading = false;
                        d.media_error = Some(e);
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let (b, seg, gen) = (self.deps.backend.clone(), self.segment.clone(), self.gen);
        self.poll_task = None;
        let task = cx.background_spawn(async move { data::load(&*b, &seg) });
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let t = task.await;
            this.update(cx, |d, cx| {
                if d.gen == gen {
                    d.apply(t, cx);
                }
            })
            .ok();
        }));
    }

    fn apply(&mut self, t: Transcript, cx: &mut Context<Self>) {
        let pending = t.pending.is_some();
        self.data = t;
        self.derive();
        if pending {
            self.poll_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(POLL).await;
                this.update(cx, |d, cx| d.load(cx)).ok();
            }));
        }
        cx.notify();
    }

    fn load_media(&mut self, cx: &mut Context<Self>) {
        let (open, path, gen) = (
            self.deps.open_player.clone(),
            self.segment.file_path.clone(),
            self.gen,
        );
        self.media_loading = true;
        let task = cx.background_spawn(async move { open(&path) });
        self.media_task = Some(cx.spawn(async move |this, cx| {
            let player = task.await;
            this.update(cx, |d, cx| {
                if d.gen != gen {
                    return;
                }
                d.media_loading = false;
                match player {
                    Ok(p) => {
                        d.transport.set_player(p);
                        if let Some(ms) = d.pending_seek.take() {
                            d.seek_to_ms(ms);
                        }
                        d.derive();
                    }
                    Err(e) => d.media_error = Some(e),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn load_peaks(&mut self, cx: &mut Context<Self>) {
        let (b, id, gen) = (self.deps.backend.clone(), self.segment.id, self.gen);
        let task = cx.background_spawn(async move { b.peaks(id, vm::WAVEFORM_BUCKETS) });
        self.peaks_task = Some(cx.spawn(async move |this, cx| {
            let peaks = task.await;
            this.update(cx, |d, cx| {
                if d.gen == gen {
                    d.peaks = peaks;
                    d.derive();
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// Rebuild the reading model (groups, strip, marks, karaoke, bars) from the data.
    fn derive(&mut self) {
        let t = &self.data;
        let speaker = vm::speaker_groups(&t.turns, &t.clusters);
        self.groups = if speaker.is_empty() {
            vm::fallback_groups(&t.runs, t.text.as_deref(), self.segment.duration_s())
        } else {
            speaker.clone()
        };
        self.strip = vm::strip(&speaker, &t.turns, &t.clusters, &t.profiles);
        let owners: HashSet<i64> = self
            .strip
            .iter()
            .filter(|s| s.owner)
            .map(|s| s.cluster_id)
            .collect();
        self.marks = vm::assign_marks(self.groups.iter().map(|g| g.cluster_id), &owners);
        self.karaoke = self
            .groups
            .iter()
            .map(|g| vm::karaoke(&t.words, g).map(Rc::from))
            .collect();
        let duration_ms = self.transport.duration.max(self.segment.duration_s()) * 1000.;
        self.bars = vm::wave_bars(&self.peaks, duration_ms, &t.turns);
        if self
            .selected_cluster
            .is_some_and(|c| !self.strip.iter().any(|s| s.cluster_id == c))
        {
            self.selected_cluster = None;
        }
        // The repair panel follows its cluster across a merge, and closes if it's gone.
        if let Some(r) = self.repair.as_mut() {
            match self.groups.iter().position(|g| g.cluster_id == r.cluster) {
                Some(ix) => r.index = ix,
                None => self.repair = None,
            }
        }
    }

    pub fn panel(&self) -> Panel {
        vm::panel(
            self.data.status,
            self.data.speakers_failed,
            self.ignore_speaker_failure,
            self.groups.len(),
        )
    }

    pub fn active_group(&self) -> Option<usize> {
        vm::active_group(
            &self.groups,
            self.transport.current_ms(),
            self.transport.engaged(),
        )
    }

    // ── transport ───────────────────────────────────────────────────────────

    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        if !self.transport.toggle() && self.transport.playable() {
            self.load_error =
                Some("Failed to play audio. The file opened, but playback could not start.".into());
        }
        cx.notify();
    }

    /// A seek the user aimed at a word re-attaches follow mode.
    pub fn seek_to_ms(&mut self, ms: i64) {
        self.transport.seek(Seek::ToMs(ms));
        self.follow_detached = false;
    }

    fn seek_by(&mut self, delta: f64, cx: &mut Context<Self>) {
        self.transport.seek(Seek::By(delta));
        cx.notify();
    }

    /// Esc, layered: the repair panel, then follow mode, then the drawer.
    pub fn escape(&mut self, cx: &mut Context<Self>) {
        if self.repair.is_some() {
            self.repair = None;
        } else if self.follow_detached {
            self.follow_detached = false;
        } else {
            cx.emit(DrawerEvent::Close);
        }
        cx.notify();
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        cx.emit(DrawerEvent::Close);
    }

    // ── writes ──────────────────────────────────────────────────────────────

    /// One speaker write at a time (they have no backend inverse); the turns
    /// reload after it, and a failure keeps the panel open with the error.
    pub fn write(&mut self, cluster: i64, w: Write, close_repair: bool, cx: &mut Context<Self>) {
        if self.busy_cluster.is_some() {
            return;
        }
        self.busy_cluster = Some(cluster);
        self.correction_error = None;
        if let Some(r) = self.repair.as_mut() {
            r.last = Some(w.clone());
            r.close_on_success = close_repair;
        }
        let (b, id, gen) = (self.deps.backend.clone(), self.segment.id, self.gen);
        let task = cx.background_spawn(async move {
            b.write(&w)?;
            let mut t = Transcript::default();
            data::turns_into(&*b, id, &mut t).map(|_| t)
        });
        self.write_task = Some(cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |d, cx| {
                if d.gen != gen {
                    return;
                }
                d.busy_cluster = None;
                match res {
                    Ok(t) => {
                        d.data.turns = t.turns;
                        d.data.clusters = t.clusters;
                        d.data.profiles = t.profiles;
                        if d.repair.as_ref().is_some_and(|r| r.close_on_success) {
                            d.repair = None;
                        } else if let Some(r) = d.repair.as_mut() {
                            r.last = None;
                        }
                        d.derive();
                    }
                    Err(e) => d.correction_error = Some(e),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Run / Rerun transcription (speech detection first for system audio).
    pub fn rerun(&mut self, cx: &mut Context<Self>) {
        self.reprocess(
            Write::Rerun {
                segment: self.segment.id,
                src: self.segment.source,
            },
            cx,
        );
    }

    pub fn retry_speakers(&mut self, cx: &mut Context<Self>) {
        self.reprocess(
            Write::RetrySpeakers {
                segment: self.segment.id,
            },
            cx,
        );
    }

    fn reprocess(&mut self, w: Write, cx: &mut Context<Self>) {
        let speakers = matches!(w, Write::RetrySpeakers { .. });
        if speakers {
            self.speaker_retry_loading = true;
        } else {
            self.rerun_loading = true;
            self.rerun_error = None;
        }
        let (b, gen) = (self.deps.backend.clone(), self.gen);
        let task = cx.background_spawn(async move { b.write(&w) });
        self.write_task = Some(cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |d, cx| {
                if d.gen != gen {
                    return;
                }
                d.rerun_loading = false;
                d.speaker_retry_loading = false;
                match res {
                    Ok(()) => d.load(cx),
                    Err(e) if speakers => d.data.speakers_error = Some(e),
                    Err(e) => d.rerun_error = Some(e),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// What playback moves on screen: the transport bar, the paragraph, and
    /// that paragraph's lit and played words.
    fn play_shown(&self) -> ((bool, i64, i64), Option<usize>, Option<usize>, usize) {
        let w = f64::from(f32::from(self.wave_bounds.get().size.width));
        let now = self.transport.current_ms();
        let group = self.active_group();
        let words = group.and_then(|ix| self.karaoke.get(ix)?.as_deref());
        (
            self.transport
                .shown(if w > 0. { w } else { transport::TRACK_STEPS }),
            group,
            words.and_then(|w| vm::active_word(w, now)),
            words.map_or(0, |w| w.iter().filter(|w| w.end_ms <= now).count()),
        )
    }

    /// One playback poll: repaint only when something shown moved.
    fn poll_play(&mut self, cx: &mut Context<Self>) -> bool {
        let before = self.play_shown();
        self.transport.tick();
        if self.play_shown() != before {
            cx.notify();
        }
        if !self.transport.playing {
            self.play_task = None;
        }
        self.transport.playing
    }

    /// Keep the active paragraph in view while following playback.
    fn follow(&mut self) {
        let active = self.active_group();
        if self.follow_detached || active == self.followed {
            return;
        }
        self.followed = active;
        if let Some(ix) = active {
            self.scroll.scroll_to_item(ix);
        }
    }
}

fn error_row(label: &'static str, msg: &str, p: &theme::Palette) -> gpui_kit::Div {
    div()
        .flex()
        .items_start()
        .gap(px(10.))
        .mx(space::S3)
        .mt(space::S2)
        .px(px(10.))
        .py(space::S2)
        .rounded(radius::XS)
        .border_1()
        .border_color(p.danger_border)
        .bg(p.danger_bg_soft)
        .text_size(text::SM)
        .text_color(p.danger_text)
        .child(
            div()
                .flex_none()
                .text_size(text::XS)
                .font_weight(theme::W_SEMIBOLD)
                .tracking(0.14)
                .text_color(p.danger)
                .child(label.to_uppercase()),
        )
        .child(div().flex_1().font_family(FONT_MONO).child(msg.to_string()))
}

impl Render for AudioDrawer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = theme::tokens(cx);
        if self.transport.playing && self.play_task.is_none() {
            self.play_task = Some(transport::poll_playback(cx, Self::poll_play));
        }
        self.follow();
        let viewport = window.viewport_size().height;
        let cap = if self.repair.is_some() {
            PEEK_REPAIR
        } else {
            PEEK
        };
        let panel = self.panel();
        let stage = div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(match panel {
                Panel::Reader => reader::render(
                    self,
                    window.viewport_size().width <= theme::breakpoint::TRANSCRIPT_STACK,
                    &p,
                    cx,
                ),
                other => panels::render(self, other, &p, cx),
            })
            .children(repair::render(self, &p, window, cx));
        let errors = [
            ("playback unavailable", self.media_error.clone()),
            ("playback error", self.load_error.clone()),
            ("rerun failed", self.rerun_error.clone()),
        ];
        let el = div()
            .id("audio-drawer")
            .role(Role::Dialog)
            .aria_label(SharedString::from(format!(
                "Audio segment player — {} #{}",
                self.segment.source.label(),
                self.segment.segment_index
            )))
            .key_context("AudioDrawer")
            .track_focus(&self.focus)
            .on_action(cx.listener(|d, _: &PlayPause, _, cx| d.toggle(cx)))
            .on_action(cx.listener(|d, _: &SeekBack, _, cx| d.seek_by(-SEEK_STEP, cx)))
            .on_action(cx.listener(|d, _: &SeekForward, _, cx| d.seek_by(SEEK_STEP, cx)))
            .on_action(cx.listener(|d, _: &SeekBackFast, _, cx| d.seek_by(-SEEK_STEP_FAST, cx)))
            .on_action(cx.listener(|d, _: &SeekForwardFast, _, cx| d.seek_by(SEEK_STEP_FAST, cx)))
            .on_action(cx.listener(|d, _: &Escape, _, cx| d.escape(cx)))
            // Outside press: the repair panel collapses first, then the drawer goes.
            .on_mouse_down_out(cx.listener(|d, _, _, cx| {
                if !d.collapse_repair(cx) {
                    cx.emit(DrawerEvent::Dismiss);
                }
            }))
            // The timeline scrubs on wheel; a wheel inside the drawer is the drawer's.
            .on_scroll(|_, _, cx| cx.stop_propagation())
            .absolute()
            .left(space::S4)
            .right(space::S4)
            .bottom(space::S3)
            .when(self.expanded, |el| el.top(space::S3))
            .when(!self.expanded, |el| el.max_h(viewport * cap))
            .flex()
            .flex_col()
            .overflow_hidden()
            // No in-window blur (founder ruling): a solid surface, not 94% glass.
            .bg(p.surface_raised)
            .border_1()
            .border_color(p.border_strong)
            .rounded(radius::LG)
            .shadow(crate::ui::popover_shadow(&p))
            .font_family(theme::FONT_UI)
            .child(header::render(self, &p, cx))
            .children(
                errors
                    .into_iter()
                    .filter_map(|(l, m)| m.map(|m| error_row(l, &m, &p))),
            )
            .child(stage)
            .child(transport::render(self, &p, cx))
            .children(
                (!self.expanded && panel == Panel::Reader && self.repair.is_none())
                    .then(|| {
                        crate::ui::Button::new("drawer-expand", "open reader")
                            .sm()
                            .icon(gpui_kit::assets::IconName::Maximize2.path())
                            .on_click(cx.listener(|d, _, _, cx| {
                                d.expanded = true;
                                cx.notify();
                            }))
                            .into_any_element()
                    })
                    .map(|b| div().absolute().right(px(18.)).bottom(px(60.)).child(b)),
            );
        el.with_animation(
            ("drawer-rise", self.gen),
            Animation::new(motion::SLOW).with_easing(motion::expo),
            // Rises 16 px into place (CSS `translateY(16px)` → 0).
            |el, t| el.opacity(t).bottom(space::S3 - RISE * (1. - t)),
        )
    }
}

#[cfg(all(test, feature = "fixtures"))]
mod tests;
