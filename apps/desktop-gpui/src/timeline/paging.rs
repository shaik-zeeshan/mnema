//! The rail's frame window as a pure state machine (`+page.svelte:156-248`,
//! `loadTimelinePage`, `loadTimelineNewerPage`, `pollTimelineHead`). The view
//! asks it what to fetch, runs the fetch off the main thread, and hands the
//! answer back with the generation it was issued under; a newer reset turns a
//! late answer into a no-op. No GPUI in here.

use std::time::Duration;

use super::model::{FocusedWindow, Frame};

/// `TIMELINE_PAGE_SIZE`.
pub const PAGE_SIZE: usize = 200;
/// `TIMELINE_PREFETCH_AHEAD`: load the next page this many frames before the end.
pub const PREFETCH_AHEAD: usize = 120;
/// `TIMELINE_MAX_LOADED_FRAMES`: trim around the active frame past this.
pub const MAX_LOADED: usize = 5_000;
/// `TIMELINE_POLL_INTERVAL_MS`.
pub const HEAD_POLL: Duration = Duration::from_millis(1500);
/// `TIMELINE_POLL_PAGE_SIZE`.
pub const POLL_PAGE: usize = 50;
/// `TIMELINE_POLL_PAGE_BUDGET`: pages walked per poll before falling back to a reset.
pub const POLL_PAGE_BUDGET: usize = 20;
/// `TIMELINE_VIEWPORT_BUFFER`: slots drawn past each edge of the rail.
pub const VIEWPORT_BUFFER: usize = 80;
/// The focused window around a jumped-to frame: one page, centred.
pub const JUMP_NEWER: usize = (PAGE_SIZE - 1) / 2;
pub const JUMP_OLDER: usize = PAGE_SIZE - JUMP_NEWER - 1;

#[derive(Debug, Default)]
pub struct Paging {
    /// Newest first: slot 0 sits at the rail's right edge.
    pub frames: Vec<Frame>,
    pub active: usize,
    pub loading: bool,
    pub loading_more: bool,
    pub exhausted: bool,
    pub has_newer: bool,
    /// A jumped-to window that isn't the live head.
    pub historical: bool,
    pub error: Option<String>,
    generation: u64,
    polling: bool,
}

#[derive(Debug, PartialEq)]
pub struct Trimmed {
    pub frames: Vec<Frame>,
    pub active: usize,
    pub trimmed_head: bool,
    pub trimmed_tail: bool,
}

/// `trimTimelineFramesAroundActive`: keep `MAX_LOADED` frames centred on the active one.
pub fn trim(frames: Vec<Frame>, active: usize) -> Trimmed {
    if frames.len() <= MAX_LOADED {
        return Trimmed {
            frames,
            active,
            trimmed_head: false,
            trimmed_tail: false,
        };
    }
    let active = active.min(frames.len() - 1);
    let max_start = frames.len() - MAX_LOADED;
    let start = active.saturating_sub(MAX_LOADED / 2).min(max_start);
    let len = frames.len();
    Trimmed {
        frames: frames.into_iter().skip(start).take(MAX_LOADED).collect(),
        active: active - start,
        trimmed_head: start > 0,
        trimmed_tail: start + MAX_LOADED < len,
    }
}

/// The slots the rail draws: half a viewport each side of the active frame, plus
/// `VIEWPORT_BUFFER`, clamped to what's loaded.
pub fn render_window(active: usize, len: usize, viewport_w: f32) -> std::ops::Range<usize> {
    let half = (viewport_w / 2. / super::model::SLOT_W).ceil() as usize;
    let start = active.saturating_sub(half + VIEWPORT_BUFFER);
    let end = (active + 1 + half + VIEWPORT_BUFFER).min(len);
    start..end.max(start)
}

/// What one head poll found.
#[derive(Debug, PartialEq)]
pub enum PollOutcome {
    /// The backend has no frames.
    Empty,
    /// The rail was empty: seed it with the newest page.
    Seed(Vec<Frame>),
    /// Frames newer than the head, newest first. `reached_head` is false when the
    /// page budget ran out first (splicing would leave a hole).
    Fresh {
        fresh: Vec<Frame>,
        reached_head: bool,
    },
}

/// Walk `list_frames` pages newest → older until the local head shows up.
/// `fetch(before_id)` returns one `POLL_PAGE` page.
pub fn walk_head(
    head_id: Option<i64>,
    mut fetch: impl FnMut(Option<i64>) -> Result<Vec<Frame>, String>,
) -> Result<PollOutcome, String> {
    let first = fetch(None)?;
    if first.is_empty() {
        return Ok(PollOutcome::Empty);
    }
    let Some(head) = head_id else {
        return Ok(PollOutcome::Seed(first));
    };
    let mut fresh = Vec::new();
    let mut page = first;
    let mut walked = 1;
    loop {
        if let Some(cut) = page.iter().position(|f| f.id <= head) {
            fresh.extend(page.into_iter().take(cut));
            return Ok(PollOutcome::Fresh {
                fresh,
                reached_head: true,
            });
        }
        let short = page.len() < POLL_PAGE;
        let tail = page.last().map(|f| f.id);
        fresh.extend(page);
        if short {
            return Ok(PollOutcome::Fresh {
                fresh,
                reached_head: true,
            });
        }
        if walked >= POLL_PAGE_BUDGET {
            return Ok(PollOutcome::Fresh {
                fresh,
                reached_head: false,
            });
        }
        page = fetch(tail)?;
        walked += 1;
        if page.is_empty() {
            return Ok(PollOutcome::Fresh {
                fresh,
                reached_head: true,
            });
        }
    }
}

/// What the view does after a poll merge.
#[derive(Debug, PartialEq, Eq)]
pub enum AfterPoll {
    Nothing,
    /// Fresh frames were merged (refresh audio, invalidate the jumper's months).
    Merged,
    /// Budget exhausted on the live head: reload from scratch.
    Reset,
}

impl Paging {
    pub fn active_frame(&self) -> Option<&Frame> {
        self.frames.get(self.active)
    }

    /// `timelineHasMore`.
    pub fn has_more(&self) -> bool {
        self.has_newer || !self.exhausted
    }

    /// Move the playhead; false when nothing changed.
    pub fn set_active(&mut self, index: usize) -> bool {
        let index = index.min(self.frames.len().saturating_sub(1));
        let changed = index != self.active;
        self.active = index;
        changed
    }

    /// Start a reset (refresh, Latest, first load); returns its generation.
    pub fn begin_reset(&mut self) -> u64 {
        self.generation += 1;
        self.loading = true;
        self.loading_more = false;
        self.exhausted = false;
        self.has_newer = false;
        self.historical = false;
        self.generation
    }

    /// True when the reset was current and replaced the frames.
    pub fn finish_reset(&mut self, generation: u64, page: Result<Vec<Frame>, String>) -> bool {
        if generation != self.generation {
            return false;
        }
        self.loading = false;
        self.loading_more = false;
        match page {
            Ok(page) => {
                self.exhausted = page.len() < PAGE_SIZE;
                self.frames = page;
                self.active = 0;
                self.error = None;
                true
            }
            Err(e) => {
                self.error = Some(e);
                false
            }
        }
    }

    /// The scrub reached within `PREFETCH_AHEAD` of the loaded tail.
    pub fn needs_older(&self) -> bool {
        !self.exhausted
            && !self.loading
            && !self.loading_more
            && self.frames.len().saturating_sub(self.active) <= PREFETCH_AHEAD
    }

    /// Start a `beforeId` page: `(generation, before_id)`.
    pub fn begin_older(&mut self) -> Option<(u64, Option<i64>)> {
        if self.loading || self.loading_more || self.exhausted {
            return None;
        }
        self.loading_more = true;
        Some((self.generation, self.frames.last().map(|f| f.id)))
    }

    pub fn finish_older(&mut self, generation: u64, page: Result<Vec<Frame>, String>) {
        if generation != self.generation {
            return;
        }
        self.loading_more = false;
        let page = match page {
            Ok(page) => page,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        let short = page.len() < PAGE_SIZE;
        let anchor = self.active_frame().map(|f| f.id);
        let mut merged = std::mem::take(&mut self.frames);
        merged.extend(page);
        let at = anchor
            .and_then(|id| merged.iter().position(|f| f.id == id))
            .unwrap_or(self.active);
        self.apply_trim(trim(merged, at));
        if short {
            self.exhausted = true;
        }
        self.error = None;
    }

    /// A historical window scrubbed near its newest edge.
    pub fn needs_newer(&self) -> bool {
        self.historical
            && self.has_newer
            && !self.loading
            && !self.loading_more
            && self.active <= PREFETCH_AHEAD
    }

    /// Start a newer page above the head: `(generation, head_id)`.
    pub fn begin_newer(&mut self) -> Option<(u64, i64)> {
        if !self.historical || !self.has_newer || self.loading || self.loading_more {
            return None;
        }
        let head = self.frames.first()?.id;
        self.loading_more = true;
        Some((self.generation, head))
    }

    pub fn finish_newer(&mut self, generation: u64, head: i64, w: Result<FocusedWindow, String>) {
        if generation != self.generation {
            return;
        }
        self.loading_more = false;
        let w = match w {
            Ok(w) if w.frames.get(w.target_index).map(|f| f.id) == Some(head) => w,
            Ok(_) => {
                self.error = Some("failed to page newer timeline frames".into());
                return;
            }
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        let page: Vec<Frame> = w.frames.into_iter().take(w.target_index).collect();
        self.has_newer = w.has_newer;
        self.historical = w.has_newer;
        self.error = None;
        if page.is_empty() {
            return;
        }
        let at = self.active + page.len();
        let mut merged = page;
        merged.append(&mut self.frames);
        self.apply_trim(trim(merged, at));
        self.has_newer = w.has_newer || self.has_newer;
        self.historical = self.has_newer;
    }

    /// Start a head poll unless a reset / page / poll is already running.
    pub fn begin_poll(&mut self) -> Option<(u64, Option<i64>)> {
        if self.polling || self.loading || self.loading_more {
            return None;
        }
        self.polling = true;
        Some((self.generation, self.frames.first().map(|f| f.id)))
    }

    /// Merge a poll. `follow_live` reselects the newest frame instead of keeping
    /// the parked one under the caret.
    pub fn finish_poll(
        &mut self,
        generation: u64,
        outcome: Result<PollOutcome, String>,
        follow_live: bool,
    ) -> AfterPoll {
        self.polling = false;
        // Poll errors are swallowed: the refresh button owns the error path.
        let Ok(outcome) = outcome else {
            return AfterPoll::Nothing;
        };
        if generation != self.generation {
            return AfterPoll::Nothing;
        }
        match outcome {
            PollOutcome::Empty => AfterPoll::Nothing,
            PollOutcome::Seed(page) => {
                if !self.frames.is_empty() {
                    return AfterPoll::Nothing;
                }
                self.frames = page;
                self.active = 0;
                AfterPoll::Merged
            }
            PollOutcome::Fresh {
                reached_head: false,
                ..
            } if !self.historical => AfterPoll::Reset,
            PollOutcome::Fresh { fresh, .. } => {
                if self.historical || fresh.is_empty() {
                    return AfterPoll::Nothing;
                }
                let at = if follow_live {
                    0
                } else {
                    self.active + fresh.len()
                };
                let mut merged = fresh;
                merged.append(&mut self.frames);
                self.apply_trim(trim(merged, at));
                AfterPoll::Merged
            }
        }
    }

    /// Start a jump to a frame (jumper commit, Recall landing).
    pub fn begin_jump(&mut self) -> u64 {
        self.generation += 1;
        self.loading = true;
        self.loading_more = false;
        self.generation
    }

    /// `Ok(true)` when the focused window replaced the rail.
    pub fn finish_jump(
        &mut self,
        generation: u64,
        target: i64,
        w: Result<FocusedWindow, String>,
    ) -> Result<bool, String> {
        if generation != self.generation {
            return Ok(false);
        }
        self.loading = false;
        self.loading_more = false;
        let w = w?;
        if w.frames.get(w.target_index).map(|f| f.id) != Some(target) {
            return Err("failed to focus selected frame".into());
        }
        self.exhausted = !w.has_older;
        self.has_newer = w.has_newer;
        self.historical = w.has_newer;
        self.error = None;
        self.frames = w.frames;
        self.active = w.target_index;
        Ok(true)
    }

    fn apply_trim(&mut self, t: Trimmed) {
        self.frames = t.frames;
        self.active = t.active;
        if t.trimmed_head {
            self.has_newer = true;
            self.historical = true;
        }
        if t.trimmed_tail {
            self.exhausted = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use super::*;

    fn frames(ids: impl Iterator<Item = i64>) -> Vec<Frame> {
        ids.map(|id| Frame {
            id,
            session_id: String::new(),
            file_path: Arc::from(Path::new("/x")),
            captured_at_ms: id * 1000,
            width: None,
            height: None,
            app_bundle_id: None,
            app_name: None,
            window_title: None,
            url: None,
            equivalence_hint: None,
        })
        .collect()
    }

    #[test]
    fn prefetch_starts_120_frames_from_the_tail() {
        let mut p = Paging::default();
        let g = p.begin_reset();
        assert!(p.finish_reset(g, Ok(frames((1..=PAGE_SIZE as i64).rev()))));
        assert!(!p.exhausted, "a full page means more history");
        p.set_active(PAGE_SIZE - PREFETCH_AHEAD - 1);
        assert!(!p.needs_older(), "121 frames from the tail: not yet");
        p.set_active(PAGE_SIZE - PREFETCH_AHEAD);
        assert!(p.needs_older(), "120 frames from the tail: load");
        let (g, before) = p.begin_older().unwrap();
        assert_eq!(before, Some(1), "pages by the smallest loaded id");
        assert!(p.begin_older().is_none(), "one page at a time");
        p.finish_older(g, Ok(frames((-50..=0).rev())));
        assert_eq!(p.frames.len(), PAGE_SIZE + 51);
        assert!(p.exhausted, "a short page is the start of history");
    }

    #[test]
    fn the_cap_trims_around_the_active_frame() {
        let t = trim(frames((0..MAX_LOADED as i64).rev()), 10);
        assert_eq!(
            (t.frames.len(), t.trimmed_head, t.trimmed_tail),
            (MAX_LOADED, false, false)
        );
        let t = trim(frames((0..=MAX_LOADED as i64).rev()), 4000);
        assert_eq!(t.frames.len(), MAX_LOADED);
        assert_eq!(
            t.active,
            3999,
            "the cut is clamped to the tail, and the active frame moves with it"
        );
        assert!(t.trimmed_head && !t.trimmed_tail);
    }

    #[test]
    fn a_stale_reset_answer_is_dropped() {
        let mut p = Paging::default();
        let old = p.begin_reset();
        let new = p.begin_reset();
        assert!(!p.finish_reset(old, Ok(frames(1..3))));
        assert!(p.frames.is_empty() && p.loading);
        assert!(p.finish_reset(new, Err("boom".into())) == false);
        assert_eq!(p.error.as_deref(), Some("boom"));
    }

    #[test]
    fn the_poll_walks_pages_until_the_head_and_respects_the_budget() {
        // Head 1000; 120 new frames arrive: three 50-frame pages reach it.
        let mut calls = 0;
        let out = walk_head(Some(1000), |before| {
            calls += 1;
            let top = before.map_or(1120, |b| b - 1);
            Ok(frames((top - POLL_PAGE as i64 + 1..=top).rev()))
        })
        .unwrap();
        assert_eq!(calls, 3);
        match out {
            PollOutcome::Fresh {
                fresh,
                reached_head,
            } => {
                assert!(reached_head);
                assert_eq!(fresh.len(), 120);
            }
            o => panic!("{o:?}"),
        }
        // A burst bigger than the budget never reaches the head.
        let mut calls = 0;
        let out = walk_head(Some(0), |before| {
            calls += 1;
            let top = before.map_or(100_000, |b| b - 1);
            Ok(frames((top - POLL_PAGE as i64 + 1..=top).rev()))
        })
        .unwrap();
        assert_eq!(calls, POLL_PAGE_BUDGET);
        assert!(matches!(
            out,
            PollOutcome::Fresh {
                reached_head: false,
                ..
            }
        ));
        let mut p = Paging::default();
        let g = p.begin_reset();
        p.finish_reset(g, Ok(frames(1..3)));
        let (g, _) = p.begin_poll().unwrap();
        assert_eq!(p.finish_poll(g, Ok(out), false), AfterPoll::Reset);
    }

    #[test]
    fn a_poll_keeps_the_parked_frame_under_the_caret() {
        let mut p = Paging::default();
        let g = p.begin_reset();
        p.finish_reset(g, Ok(frames((1..=10).rev())));
        p.set_active(3);
        let parked = p.active_frame().unwrap().id;
        let (g, head) = p.begin_poll().unwrap();
        assert_eq!(head, Some(10));
        assert!(p.begin_poll().is_none(), "polls never overlap");
        let fresh = PollOutcome::Fresh {
            fresh: frames((11..=12).rev()),
            reached_head: true,
        };
        assert_eq!(p.finish_poll(g, Ok(fresh), false), AfterPoll::Merged);
        assert_eq!(p.active_frame().unwrap().id, parked);
        assert_eq!(p.active, 5);
    }

    #[test]
    fn the_render_window_is_half_a_viewport_plus_the_buffer() {
        // 800 px rail: 50 slots each side, +80 buffer.
        assert_eq!(render_window(500, 2000, 800.), 370..631);
        assert_eq!(render_window(0, 2000, 800.), 0..131);
        assert_eq!(render_window(10, 20, 800.), 0..20);
        assert_eq!((JUMP_NEWER, JUMP_OLDER), (99, 100));
    }
}
