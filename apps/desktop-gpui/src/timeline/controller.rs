//! Runs `paging` and `scrub_engine` work: each fetch goes to the background
//! executor and its answer comes back through `this.update` with the generation
//! it was issued under. One timer task wakes the engine at its next deadline.

use std::sync::Arc;

use gpui_kit::{px, Context, Resource, ScrollEvent, Window};

use super::data::Source;
use super::model::{audio_moment, Landing, SLOT_W};
use super::paging::{self, AfterPoll, HEAD_POLL, PAGE_SIZE, POLL_PAGE};
use super::scrub_engine::Effect;
use super::TimelineView;

/// `AUDIO_SEGMENT_RANGE_PADDING_MS`: segments straddling the loaded edges still show.
pub const AUDIO_PADDING_MS: i64 = 60_000;
/// `LIVE_EDGE_PAD_MS`: episodes are fetched this far past the newest frame.
const EPISODE_EDGE_PAD_MS: i64 = 10 * 60_000;

/// Run `f` with the source on the background executor, then `then` on the view.
fn fetch_with<T: Send + 'static>(
    src: Arc<dyn Source>,
    cx: &mut Context<TimelineView>,
    f: impl FnOnce(&dyn Source) -> T + Send + 'static,
    then: impl FnOnce(&mut TimelineView, T, &mut Context<TimelineView>) + 'static,
) {
    cx.spawn(async move |this, cx| {
        let out = cx
            .background_executor()
            .spawn(async move { f(src.as_ref()) })
            .await;
        this.update(cx, |view, cx| {
            then(view, out, cx);
            cx.notify();
        })
        .ok();
    })
    .detach();
}

impl TimelineView {
    fn fetch<T: Send + 'static>(
        &self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&dyn Source) -> T + Send + 'static,
        then: impl FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
    ) {
        fetch_with(self.source.clone(), cx, f, then);
    }

    /// Recording settings changed: re-read follow-live and the retention window.
    pub fn settings_changed(&mut self, cx: &mut Context<Self>) {
        self.fetch(
            cx,
            |s| s.settings(),
            |view, settings, _| view.settings = settings,
        );
    }

    /// The Timeline's invalidations (`+page.svelte`'s three listeners).
    pub fn on_core_event(&mut self, event: &app_core::CoreEvent, cx: &mut Context<Self>) {
        use app_core::CoreEvent as E;
        match event {
            E::AudioSegmentsChanged => self.refresh_side_data(cx),
            // The cache grew: re-ask for the stills around the playhead.
            E::ScrubPreviewCacheChanged(_) => {
                if crate::window::render_idle(cx) {
                    return;
                }
                let now = cx.background_executor().now();
                self.engine
                    .cache_changed(now, &self.paging.frames, self.paging.active);
                self.arm_timer(cx);
            }
            E::TimelineDataChanged(p)
                if matches!(p.reason.as_str(), "retention" | "delete_recent_capture") =>
            {
                let active = self.paging.active_frame().map(|f| f.id);
                let before = self.paging.frames.len();
                self.paging
                    .frames
                    .retain(|f| !p.deleted_frame_ids.contains(&f.id));
                if self.paging.frames.len() != before {
                    let kept =
                        active.and_then(|id| self.paging.frames.iter().position(|f| f.id == id));
                    let last = self.paging.frames.len().saturating_sub(1);
                    self.paging.active = kept.unwrap_or(self.paging.active.min(last));
                    self.sync_scroll();
                    if kept.is_none() && !self.paging.frames.is_empty() {
                        self.set_status(
                            Some("This capture was deleted — showing the nearest frame.".into()),
                            None,
                            true,
                            cx,
                        );
                    }
                    self.after_active(false, cx);
                }
                self.audio
                    .retain(|s| !p.deleted_audio_segment_ids.contains(&s.id));
                if self
                    .selected_audio
                    .is_some_and(|(id, _)| p.deleted_audio_segment_ids.contains(&id))
                {
                    self.selected_audio = None;
                }
                cx.notify();
            }
            _ => {}
        }
    }

    pub(super) fn start(&mut self, cx: &mut Context<Self>) {
        self.settings_changed(cx);
        self.fetch(
            cx,
            |s| s.take_landing(),
            |view, landing, cx| match landing {
                Some(l) => {
                    let id = l.frame_id;
                    view.landing = Some(l);
                    view.jump_to(id, cx);
                }
                None => view.reload(cx),
            },
        );
        self._poll = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(HEAD_POLL).await;
            // Nothing to show while the window is hidden or every screen sleeps.
            let polled = this.update(cx, |view, cx| {
                if !crate::window::render_idle(cx) {
                    view.poll(cx);
                }
            });
            if polled.is_err() {
                break;
            }
        }));
    }

    /// Reset to the newest page (refresh, Latest, first load).
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let generation = self.paging.begin_reset();
        cx.notify();
        self.fetch(
            cx,
            |s| s.list_frames(None, PAGE_SIZE),
            move |view, page, cx| {
                if view.paging.finish_reset(generation, page) {
                    view.new_frames(cx);
                }
            },
        );
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if !(self.paging.loading || self.paging.loading_more || self.audio_loading) {
            self.reload(cx);
        }
    }

    pub fn jump_latest(&mut self, cx: &mut Context<Self>) {
        self.jumper.open = false;
        if !(self.paging.loading || self.paging.loading_more || self.jumper.jumping) {
            self.reload(cx);
        }
    }

    /// A Quick Recall hand-off: land on its frame with the matched words outlined.
    pub fn land(&mut self, landing: Landing, cx: &mut Context<Self>) {
        let id = landing.frame_id;
        self.match_boxes.clear();
        self.landing = Some(landing);
        self.jump_to(id, cx);
    }

    /// Focus the window around `frame_id` (jumper commit, Recall landing).
    pub fn jump_to(&mut self, frame_id: i64, cx: &mut Context<Self>) {
        let generation = self.paging.begin_jump();
        self.jumper.jumping = true;
        cx.notify();
        self.fetch(
            cx,
            move |s| s.window_around(frame_id, paging::JUMP_NEWER, paging::JUMP_OLDER),
            move |view, w, cx| {
                view.jumper.jumping = false;
                match view.paging.finish_jump(generation, frame_id, w) {
                    Ok(true) => {
                        view.jumper.open = false;
                        view.jumper.error = None;
                        view.new_frames(cx);
                    }
                    Ok(false) => {}
                    Err(e) if view.jumper.open => view.jumper.error = Some(e),
                    Err(e) => view.set_status(Some(e), None, true, cx),
                }
            },
        );
    }

    /// A new frame list: forget previews, pin the rail, fetch the side data.
    fn new_frames(&mut self, cx: &mut Context<Self>) {
        self.engine.reset();
        self.painted = None;
        self.sync_scroll();
        self.after_active(false, cx);
        self.refresh_side_data(cx);
    }

    fn sync_scroll(&mut self) {
        self.advance_px = self.paging.active as f32 * SLOT_W;
    }

    fn max_advance(&self) -> f32 {
        (self.paging.frames.len() as f32 * SLOT_W - SLOT_W).max(0.)
    }

    fn load_older(&mut self, cx: &mut Context<Self>) {
        let Some((generation, before)) = self.paging.begin_older() else {
            return;
        };
        self.fetch(
            cx,
            move |s| s.list_frames(before, PAGE_SIZE),
            move |view, page, cx| {
                view.paging.finish_older(generation, page);
                view.sync_scroll();
                view.refresh_side_data(cx);
                // Still inside the prefetch zone: chain the next page.
                if view.paging.needs_older() {
                    view.load_older(cx);
                }
            },
        );
    }

    fn load_newer(&mut self, cx: &mut Context<Self>) {
        let Some((generation, head)) = self.paging.begin_newer() else {
            return;
        };
        self.fetch(
            cx,
            move |s| s.window_around(head, PAGE_SIZE, 0),
            move |view, w, cx| {
                view.paging.finish_newer(generation, head, w);
                view.sync_scroll();
                view.refresh_side_data(cx);
                if view.paging.needs_newer() {
                    view.load_newer(cx);
                }
            },
        );
    }

    pub(super) fn poll(&mut self, cx: &mut Context<Self>) {
        if self.jumper.jumping {
            return;
        }
        let Some((generation, head)) = self.paging.begin_poll() else {
            return;
        };
        self.fetch(
            cx,
            move |s| paging::walk_head(head, |before| s.list_frames(before, POLL_PAGE)),
            move |view, outcome, cx| {
                let was_empty = view.paging.frames.is_empty();
                let follow = view.settings.follow_live;
                match view.paging.finish_poll(generation, outcome, follow) {
                    AfterPoll::Merged if was_empty => view.new_frames(cx),
                    AfterPoll::Merged => {
                        view.sync_scroll();
                        view.refresh_side_data(cx);
                    }
                    AfterPoll::Reset => view.reload(cx),
                    AfterPoll::Nothing => {}
                }
            },
        );
    }

    /// Move the playhead to `index`. `user` = a scrub (wheel, keys, rail click).
    pub fn scrub_to(&mut self, index: usize, user: bool, cx: &mut Context<Self>) {
        let prev = self.paging.active;
        if !self.paging.set_active(index) {
            return;
        }
        self.readout_dir = if self.paging.active > prev { 1. } else { -1. };
        self.after_active(user, cx);
        if self.ocr.visible {
            self.load_ocr(cx);
        }
        if self.paging.needs_newer() {
            self.load_newer(cx);
        } else if self.paging.needs_older() {
            self.load_older(cx);
        }
        cx.notify();
    }

    /// ←/→ (±1) and ⇧←/⇧→ (±10); positive is older.
    pub fn step(&mut self, delta: i64, cx: &mut Context<Self>) {
        if self.paging.frames.is_empty() {
            return;
        }
        let last = self.paging.frames.len() as i64 - 1;
        let target = (self.paging.active as i64 + delta).clamp(0, last) as usize;
        self.advance_px = target as f32 * SLOT_W;
        self.engine.velocity = 0.;
        self.scrub_to(target, true, cx);
    }

    /// Wheel anywhere scrubs: either axis, the larger wins; ⌘/⌃/⌥ pass through.
    pub(super) fn on_wheel(
        &mut self,
        e: &ScrollEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A wheel outside the drawer collapses its repair panel, else closes it.
        if let Some(d) = self.drawer.clone() {
            if !d.update(cx, |d, cx| d.collapse_repair(cx)) {
                if let Some(f) = d.read(cx).return_focus.clone() {
                    window.focus(&f, cx);
                }
                self.close_drawer(cx);
            }
        }
        let m = e.modifiers;
        if m.control || m.platform || m.alt || self.jumper.open || self.paging.frames.is_empty() {
            return;
        }
        let d = e.delta.pixel_delta(px(16.));
        let delta = if d.x.abs() > d.y.abs() { d.x } else { d.y };
        // GPUI's delta is the content's motion; a browser's deltaY is the
        // viewport's, so older (the browser's +delta) is our −delta.
        let delta = -f32::from(delta);
        if delta == 0. {
            return;
        }
        cx.stop_propagation();
        let before = self.advance_px;
        self.advance_px = (before + delta).clamp(0., self.max_advance());
        let moved = self.advance_px - before;
        if moved == 0. {
            return;
        }
        let now = cx.background_executor().now();
        let velocity = self.last_wheel.and_then(|(at, v)| {
            let dt = now.duration_since(at).as_secs_f32() * 1000.;
            (dt > 0.).then(|| moved.abs() / dt).or(Some(v))
        });
        let velocity = velocity.unwrap_or(0.);
        self.last_wheel = Some((now, velocity));
        self.readout_dir = moved.signum();
        let index = (self.advance_px / SLOT_W).round() as usize;
        if index != self.paging.active {
            self.engine.velocity = velocity;
            let dir = self.readout_dir;
            self.scrub_to(index, true, cx);
            self.readout_dir = dir;
        }
        cx.notify();
    }

    fn after_active(&mut self, user: bool, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let fx = self
            .engine
            .on_active_changed(now, &self.paging.frames, self.paging.active, user);
        self.run(fx, cx);
        self.load_match_boxes(cx);
    }

    pub(super) fn run(&mut self, effects: Vec<Effect>, cx: &mut Context<Self>) {
        for effect in effects {
            match effect {
                Effect::FetchExact(id) => self.fetch(
                    cx,
                    move |s| s.frame_preview(id),
                    move |view, res, cx| {
                        let now = cx.background_executor().now();
                        if let (Err(e), Some(f)) = (&res, view.paging.active_frame()) {
                            if f.id == id && !e.to_lowercase().contains("cancelled") {
                                let (msg, detail) = (prettify_preview_error(e), e.clone());
                                view.set_status(Some(msg), Some(detail), true, cx);
                            }
                        }
                        let fx = view.engine.exact_loaded(
                            now,
                            id,
                            res,
                            &view.paging.frames,
                            view.paging.active,
                        );
                        view.run(fx, cx);
                    },
                ),
                Effect::FetchScrub {
                    generation,
                    start_ms,
                    end_ms,
                    ids,
                } => self.fetch(
                    cx,
                    move |s| (s.scrub_availability(start_ms, end_ms, true), ids),
                    move |view, (res, ids), cx| {
                        let now = cx.background_executor().now();
                        let (frames, active) = (&view.paging.frames, view.paging.active);
                        view.engine
                            .scrub_loaded(now, generation, &ids, res, frames, active);
                        view.arm_timer(cx);
                    },
                ),
                Effect::Warm {
                    generation,
                    start_ms,
                    end_ms,
                } => self.fetch(
                    cx,
                    move |s| s.scrub_availability(start_ms, end_ms, false),
                    move |view, res, _| {
                        let (frames, active) = (&view.paging.frames, view.paging.active);
                        view.engine.warm_loaded(generation, res, frames, active);
                    },
                ),
                Effect::CancelVideo => self.fetch(cx, |s| s.cancel_video(), |_, _, _| {}),
            }
        }
        self.arm_timer(cx);
    }

    /// One task sleeps until the engine's next deadline.
    fn arm_timer(&mut self, cx: &mut Context<Self>) {
        let Some(at) = self.engine.next_deadline() else {
            self.engine_timer = None;
            return;
        };
        let wait = at.saturating_duration_since(cx.background_executor().now());
        self.engine_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |view, cx| {
                let now = cx.background_executor().now();
                let fx = view
                    .engine
                    .tick(now, &view.paging.frames, view.paging.active);
                view.run(fx, cx);
                cx.notify();
            })
            .ok();
        }));
    }

    /// Audio bars, band colours and app icons for the loaded window: one
    /// background fetch and one view update per frame-list change (a fast scrub
    /// pages every ~200 frames; three separate answers meant three re-renders).
    fn refresh_side_data(&mut self, cx: &mut Context<Self>) {
        let now = crate::ui::clock::now_ms(cx);
        let frames = &self.paging.frames;
        let (oldest, newest) = match (frames.last(), frames.first()) {
            (Some(o), Some(n)) => (o.captured_at_ms, n.captured_at_ms),
            // No frames (screen off / audio only): the last day of audio still lists.
            _ => (now - 86_400_000, now),
        };
        let empty = frames.is_empty();
        let mut missing: Vec<String> = frames
            .iter()
            .filter_map(|f| f.app_bundle_id.as_ref())
            .filter(|b| !self.icons.contains_key(*b))
            .cloned()
            .collect();
        missing.sort();
        missing.dedup();
        self.audio_loading = true;
        self.fetch(
            cx,
            move |s| {
                let audio = s.audio_segments(oldest - AUDIO_PADDING_MS, newest + AUDIO_PADDING_MS);
                let episodes = (!empty).then(|| s.episodes(oldest, newest + EPISODE_EDGE_PAD_MS));
                let icons = if missing.is_empty() {
                    Vec::new()
                } else {
                    s.app_icons(&missing)
                };
                (audio, episodes, icons)
            },
            |view, (audio, episodes, icons), _| {
                view.audio_loading = false;
                match audio {
                    Ok(mut segs) => {
                        segs.sort_by_key(|s| s.start_ms);
                        view.audio = segs;
                        view.audio_error = None;
                    }
                    Err(e) => view.audio_error = Some(e),
                }
                // A failure keeps the bands neutral (never a guessed colour).
                if let Some(Ok(e)) = episodes {
                    view.episodes = e;
                }
                view.icons.extend(icons);
            },
        );
    }

    fn load_match_boxes(&mut self, cx: &mut Context<Self>) {
        let Some(l) = self.landing.clone() else {
            return;
        };
        let active = self.paging.active_frame().map(|f| f.id);
        if !l.screen || active != Some(l.frame_id) || !self.match_boxes.is_empty() {
            return;
        }
        self.fetch(
            cx,
            move |s| s.match_boxes(l.frame_id, &l.query, l.by_meaning),
            // Best-effort: no outline beats an error over the frame.
            |view, res, _| view.match_boxes = res.unwrap_or_default(),
        );
    }

    /// Promote the engine's display to `painted` once its image has decoded, so a
    /// scrub never flashes an empty stage; a decode failure goes back to the engine.
    pub(super) fn settle_painted(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = self.engine.display().cloned() else {
            self.painted = None;
            return;
        };
        if self.painted.as_ref() == Some(&d) {
            return;
        }
        let res = Resource::Path(d.path.clone());
        let loaded = self
            .images
            .update(cx, |c, cx| gpui_kit::ImageCache::load(c, &res, window, cx));
        match loaded {
            Some(Ok(_)) => {
                self.engine.image_painted(d.frame_id);
                self.painted = Some(d);
            }
            Some(Err(_)) => {
                let now = cx.background_executor().now();
                let (frames, active) = (&self.paging.frames, self.paging.active);
                match self.engine.image_failed(now, d.frame_id, frames, active) {
                    Ok(fx) => self.run(fx, cx),
                    Err(msg) => {
                        self.painted = None;
                        self.status = Some(super::Status {
                            message: msg.into(),
                            detail: Some(
                                "This frame's image failed to decode after several attempts."
                                    .into(),
                            ),
                            error: true,
                        });
                    }
                }
            }
            None => {}
        }
    }

    /// P: open the AudioDrawer (S9) on the segment under the active frame.
    pub fn play_moment(&mut self, cx: &mut Context<Self>) {
        let Some(at) = self.paging.active_frame().map(|f| f.captured_at_ms) else {
            return;
        };
        match audio_moment(&self.audio, at) {
            Some((seg, offset)) => self.selected_audio = Some((seg.id, offset)),
            None => self.set_status(
                Some("No audio captured at this moment.".into()),
                Some("This frame isn't covered by a microphone or system-audio segment.".into()),
                false,
                cx,
            ),
        }
        cx.notify();
    }
}

/// `prettifyFramePreviewError`.
pub fn prettify_preview_error(raw: &str) -> String {
    let m = raw.to_lowercase();
    let pick = |s: &str| s.to_string();
    if m.contains("no decodable video sample found") {
        pick("Couldn't load the frame preview from this recording.")
    } else if m.contains("sample did not contain an image buffer") {
        pick("Couldn't decode an image from the frame preview.")
    } else if m.contains("failed to start reading video samples") {
        pick("Couldn't read the recording while generating the frame preview.")
    } else if m.contains("failed to read video sample") {
        pick("Couldn't read a video sample for the frame preview.")
    } else if m.contains("failed to join video preview extraction task") {
        pick("Frame preview generation stopped unexpectedly.")
    } else if m.contains("not found") {
        pick("This frame preview is no longer available.")
    } else {
        pick("Couldn't load the frame preview.")
    }
}
