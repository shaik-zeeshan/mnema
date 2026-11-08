//! Feel instrumentation (feature `feel-metrics`): per-frame work and dropped
//! frames, keystroke → paint, summon latency and startup milestones on the Debug
//! target `mnema_gpui::feel`. `scripts/gpui-feel-report.sh` turns those lines
//! into the §5 budget table. Never logs content: only names and milliseconds.
//! Without the feature every call compiles to nothing.
//!
//! Frame numbers come from GPUI's own frame profiler (`gpui-kit/profiler`), one
//! sample per drawn-and-presented frame, however many views call
//! [`frame_rendered`]:
//! - `work_ms`: `Window::draw` start (render, layout, prepaint, paint) until the
//!   scene is handed to Metal (end of present);
//! - `dropped=1`: the gap since the window's previous frame was over 1.5× the
//!   display's refresh period (`Window::refresh_interval`) *while the window
//!   wanted a frame*: it reached Metal more than a refresh after the first vsync
//!   it was owed. It's owed the first vsync after the later of its first
//!   invalidation and the previous frame's present, or its own vsync signal when
//!   the main thread sat on that signal for over a refresh (a stall that also
//!   held the input back). Idle gaps and slow-arriving updates don't count.
//!
//! Benches wrap the measured stretch in [`section`]; the report then counts only
//! the frames inside it (no startup frames).

use gpui_kit::Window;

#[cfg(feature = "feel-metrics")]
mod imp {
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    use gpui_kit::profiler::{FrameEvent, FrameTiming, FrameTimingCollector, PresentTiming};
    use gpui_kit::{App, Window, WindowId};

    pub const TARGET: &str = "mnema_gpui::feel";
    static START: OnceLock<Instant> = OnceLock::new();

    #[derive(Default)]
    struct Frames {
        collector: Option<FrameTimingCollector>,
        drawn: HashMap<WindowId, FrameTiming>,
        last_present: HashMap<WindowId, Instant>,
    }

    thread_local! {
        static KEY_AT: Cell<Option<Instant>> = const { Cell::new(None) };
        static SUMMON_AT: Cell<Option<Instant>> = const { Cell::new(None) };
        static FIRST_PAINT: Cell<bool> = const { Cell::new(false) };
        static FRAMES: RefCell<Frames> = RefCell::default();
    }

    fn ms(d: Duration) -> f64 {
        d.as_secs_f64() * 1000.
    }

    pub fn mark_start() {
        START.get_or_init(Instant::now);
        start_collecting();
    }

    fn start_collecting() {
        FRAMES.with(|f| {
            let mut f = f.borrow_mut();
            if f.collector.is_none() {
                gpui_kit::profiler::set_trace_enabled(true);
                f.collector = Some(FrameTimingCollector::new());
            }
        });
    }

    pub fn milestone(name: &str) {
        let at = START.get_or_init(Instant::now).elapsed();
        log::debug!(target: TARGET, "milestone {name} ms={:.1}", ms(at));
    }

    pub fn observe_keystrokes(cx: &mut App) {
        cx.observe_keystrokes(|_, _, _| KEY_AT.with(|k| k.set(Some(Instant::now()))))
            .detach();
    }

    pub fn summon_started() {
        SUMMON_AT.with(|s| s.set(Some(Instant::now())));
    }

    /// Whether a frame dropped. `needed` (when it was wanted) and `presented`
    /// (when it reached Metal) are ms relative to its vsync signal; `interval`
    /// is the gap since the previous frame.
    pub(super) fn dropped(refresh: f64, needed: f64, presented: f64, interval: f64) -> bool {
        // Wanted over a refresh after its own signal: the main thread wasn't
        // servicing vsyncs (a stall), so it was wanted from the signal on.
        let needed = if needed > refresh { 0. } else { needed };
        // An update landing in the last eighth of a refresh before a vsync races
        // that vsync's callback; it's owed the next one, not that one.
        let owed = ((needed + refresh / 8.) / refresh).ceil() * refresh;
        interval > refresh * 1.5 && presented - owed > refresh
    }

    pub fn section(start: bool) {
        let edge = if start { "start" } else { "end" };
        log::debug!(target: TARGET, "section {edge}");
    }

    fn presented(f: &mut Frames, draw: FrameTiming, present: PresentTiming) {
        let work = present
            .present_end
            .saturating_duration_since(draw.draw_start);
        let prev = f.last_present.insert(draw.window_id, present.present_end);
        let interval = prev.map_or(-1., |p| {
            ms(present.present_end.saturating_duration_since(p))
        });
        let drop = match (draw.refresh_interval, draw.signal_at) {
            (Some(refresh), Some(signal)) => {
                let needed = draw
                    .dirty_at
                    .into_iter()
                    .chain(prev)
                    .max()
                    .unwrap_or(signal);
                let rel = |at: Instant| {
                    if at >= signal {
                        ms(at - signal)
                    } else {
                        -ms(signal - at)
                    }
                };
                let late = rel(present.present_end);
                dropped(ms(refresh), rel(needed), late, interval)
            }
            _ => false,
        };
        log::debug!(
            target: TARGET,
            "frame work_ms={:.2} draw_ms={:.2} layout_ms={:.2} prepaint_ms={:.2} paint_ms={:.2} present_ms={:.2} interval_ms={interval:.2} refresh_ms={:.2} dropped={}",
            ms(work),
            ms(draw.draw_duration()),
            ms(draw.phases.request_layout),
            ms(draw.phases.prepaint),
            ms(draw.phases.paint),
            ms(present.present_duration()),
            draw.refresh_interval.map_or(-1., ms),
            u8::from(drop),
        );
        if let Some(at) = KEY_AT.with(|k| k.get()).filter(|at| draw.draw_start >= *at) {
            KEY_AT.with(|k| k.set(None));
            log::debug!(target: TARGET, "key_to_paint_ms={:.2}", ms(present.present_end - at));
        }
        if let Some(at) = SUMMON_AT
            .with(|s| s.get())
            .filter(|at| draw.draw_start >= *at)
        {
            SUMMON_AT.with(|s| s.set(None));
            log::debug!(target: TARGET, "summon_ms={:.2}", ms(present.present_end - at));
        }
        if !FIRST_PAINT.with(|p| p.replace(true)) {
            milestone("first_paint");
        }
    }

    fn collect() {
        FRAMES.with(|f| {
            let mut f = f.borrow_mut();
            let Some(events) = f.collector.as_mut().map(|c| c.collect_unseen()) else {
                return;
            };
            for event in events {
                match event {
                    FrameEvent::Draw(t) => {
                        f.drawn.insert(t.window_id, t);
                    }
                    FrameEvent::Present(p) => {
                        if let Some(draw) = f.drawn.remove(&p.window_id) {
                            presented(&mut f, draw, p);
                        }
                    }
                }
            }
        });
    }

    pub fn frame_rendered(window: &mut Window) {
        start_collecting();
        // Frames drawn so far (this one lands on the next render).
        collect();
        let waiting = KEY_AT.with(|k| k.get()).is_some()
            || SUMMON_AT.with(|s| s.get()).is_some()
            || !FIRST_PAINT.with(|p| p.get());
        if waiting {
            // One callback, not a loop: it runs once this frame has been presented.
            window.on_next_frame(|_, _| collect());
        }
    }
}

/// Call first thing in `main`: startup milestones count from here.
pub fn mark_start() {
    #[cfg(feature = "feel-metrics")]
    imp::mark_start();
}

pub fn milestone(_name: &str) {
    #[cfg(feature = "feel-metrics")]
    imp::milestone(_name);
}

pub fn observe_keystrokes(_cx: &mut gpui_kit::App) {
    #[cfg(feature = "feel-metrics")]
    imp::observe_keystrokes(_cx);
}

/// Quick Recall's hotkey handler calls this; the panel's next paint logs `summon_ms`.
pub fn summon_started() {
    #[cfg(feature = "feel-metrics")]
    imp::summon_started();
}

/// Benches: bracket the measured stretch (`true` before, `false` after).
pub fn section(_start: bool) {
    #[cfg(feature = "feel-metrics")]
    imp::section(_start);
}

/// Call from a root view's `render`. Any number of views may call it: samples
/// are drained from GPUI's frame log, so each frame is logged once.
pub fn frame_rendered(_window: &mut Window) {
    #[cfg(feature = "feel-metrics")]
    imp::frame_rendered(_window);
}

#[cfg(all(test, feature = "feel-metrics"))]
mod tests {
    #[test]
    fn a_frame_drops_when_it_misses_the_first_vsync_it_was_wanted_for() {
        let d =
            |needed, presented, interval| super::imp::dropped(16.667, needed, presented, interval);
        // Wanted 5 ms before its vsync, handed over 10 ms after it: on time.
        assert!(!d(-5., 10., 16.7));
        // An update that just missed the previous vsync waits for this one: on time.
        assert!(!d(-16.4, 10.6, 33.3));
        // Wanted a vsync earlier than it was drawn: one refresh lost.
        assert!(d(-20., 1., 33.3));
        // ...but wanted 1 ms before that vsync: a race with its callback, not a drop.
        assert!(!d(-17.6, 1., 33.3));
        // Late on the vsync grid but under 1.5 refreshes since the last frame: kept.
        assert!(!d(-20., 1., 24.));
        // Draw and present past the next vsync: dropped.
        assert!(d(-5., 17., 30.));
        // Wanted long after its signal (main thread stalled on the vsync): dropped.
        assert!(d(150., 170., 170.));
        // An idle gap: wanted just now, on time, however long since the last frame.
        assert!(!d(-3., 4., 900.));
    }
}
