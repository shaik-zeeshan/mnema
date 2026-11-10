//! The scrub engine (`+page.svelte:195-211`, the active-preview `$effect`,
//! `ensureLatestScrubPreviews`, `warmLatestScrubPreviews`,
//! `markTimelinePreviewScrubbing`) as a pure state machine over explicit
//! `Instant`s. The view feeds it scrubs, loads and clock ticks and runs the
//! [`Effect`]s it returns; its timers are deadlines the view arms one task for.
//!
//! Two tiers: fast scrubbing shows cached low-res stills (scrub intervals) at
//! most every 80 ms; the exact frame is fetched only once the playhead has
//! rested 500 ms, one request at a time, latest wins.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::model::{Frame, Preview, ScrubInterval, SourceKind};

/// `ACTIVE_PREVIEW_SCRUB_REQUEST_INTERVAL_MS`: scrub-tier fetch throttle.
pub const THROTTLE: Duration = Duration::from_millis(160);
/// `..._FAST_REQUEST_INTERVAL_MS`: the throttle at ≥ 2 px/ms.
pub const THROTTLE_FAST: Duration = Duration::from_millis(240);
/// `ACTIVE_PREVIEW_DISPLAY_SCRUB_THROTTLE_MS`: the stage swaps at most this often.
pub const DISPLAY_THROTTLE: Duration = Duration::from_millis(80);
/// `ACTIVE_PREVIEW_DISPLAY_SETTLE_MS`: back to parked after this long still.
pub const DISPLAY_SETTLE: Duration = Duration::from_millis(180);
/// `ACTIVE_PREVIEW_SCRUB_WARM_SETTLE_MS`: read-only availability warm-up.
pub const WARM_SETTLE: Duration = Duration::from_millis(1200);
/// `ACTIVE_PREVIEW_EXACT_SETTLE_MS`: the exact frame after a scrub rests.
pub const EXACT_SETTLE: Duration = Duration::from_millis(500);
/// `ACTIVE_PREVIEW_*_SCRUB_PX_PER_MS`: the velocity steps.
pub const FAST_PX_PER_MS: f32 = 2.;
pub const VERY_FAST_PX_PER_MS: f32 = 4.;
pub const EXTREME_PX_PER_MS: f32 = 8.;
/// `PREVIEW_CACHE_MAX_ENTRIES` (exact paths here; decoded images in `image_cache`).
pub const CACHE_MAX: usize = 192;
/// `PREVIEW_FAILURE_CACHE_TTL_MS`.
pub const FAILURE_TTL: Duration = Duration::from_secs(5);
/// The scrub interval cache's cap (`touchScrubPreviewIntervalCache`).
const INTERVAL_MAX: usize = 1500;
/// `MAX_ACTIVE_PREVIEW_DECODE_RETRIES`.
const DECODE_RETRIES: u32 = 2;

/// `(radius, max uncached)` of the scrub prefetch at `velocity` px/ms: 4/9 →
/// 3/7 at 2 → 2/5 at 4 → 1/3 at 8.
pub fn window_for_velocity(velocity: f32) -> (usize, usize) {
    if velocity >= EXTREME_PX_PER_MS {
        (1, 3)
    } else if velocity >= VERY_FAST_PX_PER_MS {
        (2, 5)
    } else if velocity >= FAST_PX_PER_MS {
        (3, 7)
    } else {
        (4, 9)
    }
}

/// `scrubPreviewScheduleDelayMs`.
pub fn throttle_for_velocity(velocity: f32) -> Duration {
    if velocity >= FAST_PX_PER_MS {
        THROTTLE_FAST
    } else {
        THROTTLE
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Parked,
    Scrubbing,
}

/// What the stage paints.
#[derive(Clone, Debug, PartialEq)]
pub struct Display {
    pub frame_id: i64,
    pub path: Arc<Path>,
    pub exact: bool,
    pub source_kind: SourceKind,
}

/// Work for the view to run off the main thread.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// `get_frame_preview(videoScope: active_frame)`.
    FetchExact(i64),
    /// `get_scrub_preview_availability(enqueueMissing: true)`.
    FetchScrub {
        generation: u64,
        start_ms: i64,
        end_ms: i64,
        ids: Vec<i64>,
    },
    /// `get_scrub_preview_availability(enqueueMissing: false)`.
    Warm {
        generation: u64,
        start_ms: i64,
        end_ms: i64,
    },
    /// `cancel_active_frame_preview_video_requests`.
    CancelVideo,
}

/// The dev overlay's counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub exact_hits: u32,
    pub exact_misses: u32,
    pub exact_failures: u32,
    pub scrub_batches: u32,
    pub scrub_ready: u32,
    pub warm: u32,
}

#[derive(Default)]
pub struct ScrubEngine {
    /// Velocity committed with the last active-frame change (px/ms).
    pub velocity: f32,
    pub stats: Stats,
    mode: Option<Mode>,
    display: Option<Display>,
    last_swap: Option<Instant>,
    settle: Option<(Instant, i64)>,
    exact_gen: u64,
    exact_timer: Option<(Instant, i64, u64)>,
    exact_in_flight: bool,
    exact_pending: Option<i64>,
    scrub_gen: u64,
    scrub_timer: Option<Instant>,
    scheduled: Option<(usize, u64)>,
    batch_in_flight: bool,
    pending_index: Option<usize>,
    warm_timer: Option<(Instant, usize, u64)>,
    warm_in_flight: bool,
    prev_index: usize,
    prev_frame: Option<i64>,
    /// LRU, oldest first.
    exact: Vec<(i64, Preview)>,
    exact_failed: HashMap<i64, Instant>,
    exact_loading: HashSet<i64>,
    intervals: Vec<ScrubInterval>,
    scrub_failed: HashMap<i64, Instant>,
    scrub_loading: HashSet<i64>,
    decode_retries: HashMap<i64, u32>,
}

impl ScrubEngine {
    pub fn mode(&self) -> Mode {
        self.mode.unwrap_or(Mode::Parked)
    }

    pub fn display(&self) -> Option<&Display> {
        self.display.as_ref()
    }

    pub fn exact(&self, frame_id: i64) -> Option<&Preview> {
        self.exact
            .iter()
            .find(|(id, _)| *id == frame_id)
            .map(|(_, p)| p)
    }

    /// Forget everything (reset, jump): a new frame list.
    pub fn reset(&mut self) {
        *self = Self {
            exact_gen: self.exact_gen + 1,
            scrub_gen: self.scrub_gen + 1,
            stats: self.stats,
            ..Default::default()
        };
    }

    /// The earliest timer, for the view's one wake-up task.
    pub fn next_deadline(&self) -> Option<Instant> {
        [
            self.settle.map(|s| s.0),
            self.exact_timer.map(|t| t.0),
            self.scrub_timer,
            self.warm_timer.map(|t| t.0),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// The playhead moved to `active`. `scrub` = by the user (wheel / keys /
    /// rail), which flips the stage to scrubbing and throttles its swaps.
    pub fn on_active_changed(
        &mut self,
        now: Instant,
        frames: &[Frame],
        active: usize,
        scrub: bool,
    ) -> Vec<Effect> {
        let mut out = Vec::new();
        let frame = frames.get(active);
        if scrub {
            if let Some(f) = frame {
                self.mark_scrubbing(now, f);
            }
        }
        let id = frame.map(|f| f.id);
        let frame_changed = id != self.prev_frame;
        let delta = active.abs_diff(self.prev_index);
        self.prev_index = active;
        self.prev_frame = id;
        let should_scrub = frame_changed && delta > 0;
        self.exact_gen += 1;
        self.scrub_gen += 1;
        self.exact_timer = None;
        self.warm_timer = None;
        if frame_changed && self.exact_in_flight {
            out.push(Effect::CancelVideo);
        }
        if self.mode() == Mode::Parked {
            self.refresh_display(frames, active);
        }
        let Some(f) = frame else { return out };
        if self.exact(f.id).is_some() || self.exact_loading.contains(&f.id) {
            return out;
        }
        if should_scrub {
            if self.batch_in_flight {
                self.pending_index = Some(active);
            } else {
                self.schedule_scrub(now, active, self.scrub_gen);
            }
            self.warm_timer = Some((now + WARM_SETTLE, active, self.scrub_gen));
        }
        if self.exact_in_flight {
            self.exact_pending = Some(f.id);
            return out;
        }
        if should_scrub {
            self.exact_timer = Some((now + EXACT_SETTLE, f.id, self.exact_gen));
        } else {
            out.extend(self.ensure_exact(now, f.id));
        }
        out
    }

    /// `scrub_preview_cache_changed`: new stills exist, so re-ask for the ones
    /// around the playhead (nothing to do once its exact image is in).
    pub fn cache_changed(&mut self, now: Instant, frames: &[Frame], active: usize) {
        let Some(f) = frames.get(active) else { return };
        if self.exact(f.id).is_some() {
            return;
        }
        self.scrub_gen += 1;
        if self.batch_in_flight {
            self.pending_index = Some(active);
        } else {
            self.schedule_scrub(now, active, self.scrub_gen);
        }
    }

    /// Fire every timer due at `now`.
    pub fn tick(&mut self, now: Instant, frames: &[Frame], active: usize) -> Vec<Effect> {
        let mut out = Vec::new();
        let active_id = frames.get(active).map(|f| f.id);
        if let Some((at, id)) = self.settle {
            if at <= now {
                self.settle = None;
                if active_id == Some(id) {
                    self.mode = Some(Mode::Parked);
                    self.refresh_display(frames, active);
                }
            }
        }
        if let Some((at, id, gen)) = self.exact_timer {
            if at <= now {
                self.exact_timer = None;
                if gen == self.exact_gen && active_id == Some(id) {
                    out.extend(self.ensure_exact(now, id));
                }
            }
        }
        if let Some(at) = self.scrub_timer {
            if at <= now {
                self.scrub_timer = None;
                if let Some((index, gen)) = self.scheduled.take() {
                    if gen == self.scrub_gen && index == active {
                        out.extend(self.ensure_scrub(frames, index));
                    }
                }
            }
        }
        if let Some((at, index, gen)) = self.warm_timer {
            if at <= now {
                self.warm_timer = None;
                if gen == self.scrub_gen && index == active && !self.warm_in_flight {
                    if let Some((start_ms, end_ms)) = self.time_window(frames, index) {
                        self.warm_in_flight = true;
                        self.stats.warm += 1;
                        out.push(Effect::Warm {
                            generation: gen,
                            start_ms,
                            end_ms,
                        });
                    }
                }
            }
        }
        out
    }

    /// `get_frame_preview` answered. `Ok(None)` = the frame is gone.
    pub fn exact_loaded(
        &mut self,
        now: Instant,
        frame_id: i64,
        result: Result<Option<Preview>, String>,
        frames: &[Frame],
        active: usize,
    ) -> Vec<Effect> {
        self.exact_loading.remove(&frame_id);
        self.exact_in_flight = false;
        match result {
            Ok(Some(p)) => {
                self.exact_failed.remove(&frame_id);
                self.touch_exact(frame_id, p);
            }
            // A cancelled video extraction is the playhead moving on, not a failure.
            Err(e) if e.to_lowercase().contains("preview generation cancelled") => {}
            Ok(None) | Err(_) => {
                self.stats.exact_failures += 1;
                self.exact_failed.insert(frame_id, now);
            }
        }
        self.refresh_display(frames, active);
        let pending = self.exact_pending.take();
        match frames.get(active) {
            Some(f)
                if pending == Some(f.id)
                    && self.exact(f.id).is_none()
                    && !self.exact_loading.contains(&f.id) =>
            {
                self.exact_gen += 1;
                self.exact_timer = Some((now + EXACT_SETTLE, f.id, self.exact_gen));
            }
            _ => {}
        }
        Vec::new()
    }

    /// A scrub batch (`FetchScrub`) answered.
    pub fn scrub_loaded(
        &mut self,
        now: Instant,
        generation: u64,
        ids: &[i64],
        result: Result<Vec<ScrubInterval>, String>,
        frames: &[Frame],
        active: usize,
    ) {
        for id in ids {
            self.scrub_loading.remove(id);
        }
        self.batch_in_flight = false;
        if generation == self.scrub_gen {
            if let Ok(intervals) = result {
                self.take_intervals(intervals, frames, active);
            }
        }
        if let Some(index) = self.pending_index.take() {
            if index == active {
                self.scrub_gen += 1;
                self.schedule_scrub(now, index, self.scrub_gen);
            }
        }
    }

    /// A warm-up (`Warm`) answered.
    pub fn warm_loaded(
        &mut self,
        generation: u64,
        result: Result<Vec<ScrubInterval>, String>,
        frames: &[Frame],
        active: usize,
    ) {
        self.warm_in_flight = false;
        if generation == self.scrub_gen {
            if let Ok(intervals) = result {
                self.take_intervals(intervals, frames, active);
            }
        }
    }

    /// The stage couldn't decode what it was shown (`handleActivePreviewLoadError`):
    /// a scrub still is dropped and refetched; an exact one is retried twice, then
    /// `Err` carries the terminal message for the stage status.
    pub fn image_failed(
        &mut self,
        now: Instant,
        frame_id: i64,
        frames: &[Frame],
        active: usize,
    ) -> Result<Vec<Effect>, &'static str> {
        let shown = self.display.as_ref().filter(|d| d.frame_id == frame_id);
        if shown.is_some_and(|d| !d.exact) {
            let path = shown.map(|d| d.path.clone());
            self.intervals
                .retain(|i| i.preview.as_ref().map(|p| &p.file_path) != path.as_ref());
            self.scrub_failed.insert(frame_id, now);
            self.scrub_gen += 1;
            return Ok(self.ensure_scrub(frames, active));
        }
        let tries = self.decode_retries.entry(frame_id).or_default();
        if *tries >= DECODE_RETRIES {
            return Err("Preview couldn't be displayed");
        }
        *tries += 1;
        self.exact.retain(|(id, _)| *id != frame_id);
        self.exact_failed.remove(&frame_id);
        Ok(self.ensure_exact(now, frame_id))
    }

    /// The image painted: its retry budget resets.
    pub fn image_painted(&mut self, frame_id: i64) {
        self.decode_retries.remove(&frame_id);
    }

    fn mark_scrubbing(&mut self, now: Instant, frame: &Frame) {
        let swap = self.display.is_none()
            || self
                .last_swap
                .is_none_or(|t| now.duration_since(t) >= DISPLAY_THROTTLE);
        self.mode = Some(Mode::Scrubbing);
        if swap {
            self.last_swap = Some(now);
            let next = self.candidate(frame, true);
            self.display = next;
        }
        self.settle = Some((now + DISPLAY_SETTLE, frame.id));
    }

    /// Parked: show the active frame's best image (or nothing). Scrubbing: only
    /// upgrade to a better image of the frame under the caret, never clear.
    fn refresh_display(&mut self, frames: &[Frame], active: usize) {
        let Some(frame) = frames.get(active) else {
            self.display = None;
            return;
        };
        let next = self.candidate(frame, true);
        match self.mode() {
            Mode::Parked => self.display = next,
            Mode::Scrubbing => {
                if next.is_some() {
                    self.display = next;
                }
            }
        }
    }

    fn candidate(&self, frame: &Frame, allow_exact: bool) -> Option<Display> {
        if allow_exact {
            if let Some(p) = self.exact(frame.id) {
                return Some(Display {
                    frame_id: frame.id,
                    path: p.file_path.clone(),
                    exact: true,
                    source_kind: p.source_kind,
                });
            }
        }
        self.interval_for(frame).and_then(|i| {
            i.preview.as_ref().map(|p| Display {
                frame_id: frame.id,
                path: p.file_path.clone(),
                exact: false,
                source_kind: p.source_kind,
            })
        })
    }

    /// `scrubPreviewIntervalForFrame`: the latest-starting ready interval covering it.
    fn interval_for(&self, frame: &Frame) -> Option<&ScrubInterval> {
        let at = frame.captured_at_ms;
        self.intervals
            .iter()
            .filter(|i| i.preview.is_some() && at >= i.start_ms && at < i.end_ms)
            .max_by_key(|i| i.start_ms)
    }

    fn ensure_exact(&mut self, now: Instant, frame_id: i64) -> Vec<Effect> {
        if self.exact(frame_id).is_some() {
            self.stats.exact_hits += 1;
            return Vec::new();
        }
        if self
            .exact_failed
            .get(&frame_id)
            .is_some_and(|at| now.duration_since(*at) < FAILURE_TTL)
        {
            return Vec::new();
        }
        if self.exact_loading.contains(&frame_id) {
            return Vec::new();
        }
        if self.exact_in_flight {
            self.exact_pending = Some(frame_id);
            return Vec::new();
        }
        self.stats.exact_misses += 1;
        self.exact_in_flight = true;
        self.exact_loading.insert(frame_id);
        vec![Effect::FetchExact(frame_id)]
    }

    fn schedule_scrub(&mut self, now: Instant, index: usize, generation: u64) {
        // A throttle, not a debounce: the first scrub arms it, later ones only
        // move what it fetches.
        self.scheduled = Some((index, generation));
        if self.scrub_timer.is_none() {
            self.scrub_timer = Some(now + throttle_for_velocity(self.velocity));
        }
    }

    fn ensure_scrub(&mut self, frames: &[Frame], index: usize) -> Vec<Effect> {
        if self.batch_in_flight {
            self.pending_index = Some(index);
            return Vec::new();
        }
        let Some((start_ms, end_ms)) = self.time_window(frames, index) else {
            return Vec::new();
        };
        let ids = self.uncached_around(frames, index);
        if ids.is_empty()
            && frames
                .get(index)
                .and_then(|f| self.interval_for(f))
                .is_some()
        {
            return Vec::new();
        }
        self.batch_in_flight = true;
        self.stats.scrub_batches += 1;
        self.scrub_loading.extend(ids.iter().copied());
        vec![Effect::FetchScrub {
            generation: self.scrub_gen,
            start_ms,
            end_ms,
            ids,
        }]
    }

    /// `scrubPreviewFrameIdsAround`: the active frame, then outward, skipping
    /// covered, in-flight and recently failed frames, up to the velocity's cap.
    fn uncached_around(&self, frames: &[Frame], active: usize) -> Vec<i64> {
        let (radius, max) = window_for_velocity(self.velocity);
        let lo = active.saturating_sub(radius);
        let hi = (active + radius).min(frames.len().saturating_sub(1));
        let mut order = vec![active];
        for d in 1..=radius {
            if active >= d && active - d >= lo {
                order.push(active - d);
            }
            if active + d <= hi {
                order.push(active + d);
            }
        }
        order
            .into_iter()
            .filter_map(|i| frames.get(i))
            .filter(|f| {
                self.interval_for(f).is_none()
                    && !self.scrub_loading.contains(&f.id)
                    && !self.scrub_failed.contains_key(&f.id)
            })
            .map(|f| f.id)
            .take(max)
            .collect()
    }

    /// `scrubPreviewTimeWindowAround`: the radius's capture span, ±1 s.
    fn time_window(&self, frames: &[Frame], active: usize) -> Option<(i64, i64)> {
        let (radius, _) = window_for_velocity(self.velocity);
        let lo = active.saturating_sub(radius);
        let hi = (active + radius).min(frames.len().checked_sub(1)?);
        let span = frames.get(lo..=hi)?;
        let min = span.iter().map(|f| f.captured_at_ms).min()?;
        let max = span.iter().map(|f| f.captured_at_ms).max()?;
        Some((min - 1000, max + 1000))
    }

    fn touch_exact(&mut self, frame_id: i64, preview: Preview) {
        self.exact.retain(|(id, _)| *id != frame_id);
        self.exact.push((frame_id, preview));
        if self.exact.len() > CACHE_MAX {
            let over = self.exact.len() - CACHE_MAX;
            self.exact.drain(..over);
        }
    }

    fn take_intervals(&mut self, intervals: Vec<ScrubInterval>, frames: &[Frame], active: usize) {
        let mut ready = 0;
        for i in intervals.into_iter().filter(|i| i.preview.is_some()) {
            ready += 1;
            self.intervals.retain(|o| {
                (&o.segment_key, o.start_offset_ms) != (&i.segment_key, i.start_offset_ms)
            });
            self.intervals.push(i);
        }
        if self.intervals.len() > INTERVAL_MAX {
            let over = self.intervals.len() - INTERVAL_MAX;
            self.intervals.drain(..over);
        }
        self.stats.scrub_ready += ready;
        if ready > 0 {
            self.refresh_display(frames, active);
        }
    }
}

#[cfg(test)]
#[path = "scrub_engine_tests.rs"]
mod tests;
