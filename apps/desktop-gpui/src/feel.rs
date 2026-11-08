//! Feel instrumentation (feature `feel-metrics`): frame time, keystroke → paint,
//! summon latency and startup milestones on the Debug target `mnema_gpui::feel`.
//! `scripts/gpui-feel-report.sh` turns those lines into the §5 budget table.
//! Never logs content: only names and milliseconds. Without the feature every
//! call compiles to nothing.

use gpui_kit::Window;

#[cfg(feature = "feel-metrics")]
mod imp {
    use std::cell::Cell;
    use std::sync::OnceLock;
    use std::time::Instant;

    use gpui_kit::{App, Window};

    pub const TARGET: &str = "mnema_gpui::feel";
    static START: OnceLock<Instant> = OnceLock::new();

    thread_local! {
        static KEY_AT: Cell<Option<Instant>> = const { Cell::new(None) };
        static SUMMON_AT: Cell<Option<Instant>> = const { Cell::new(None) };
        static FIRST_PAINT: Cell<bool> = const { Cell::new(false) };
    }

    pub fn mark_start() {
        START.get_or_init(Instant::now);
    }

    fn since_start_ms() -> f64 {
        START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.
    }

    pub fn milestone(name: &str) {
        log::debug!(target: TARGET, "milestone {name} ms={:.1}", since_start_ms());
    }

    pub fn observe_keystrokes(cx: &mut App) {
        cx.observe_keystrokes(|_, _, _| KEY_AT.with(|k| k.set(Some(Instant::now()))))
            .detach();
    }

    pub fn summon_started() {
        SUMMON_AT.with(|s| s.set(Some(Instant::now())));
    }

    pub fn frame_rendered(window: &mut Window) {
        let started = Instant::now();
        let key = KEY_AT.with(|k| k.take());
        let summon = SUMMON_AT.with(|s| s.take());
        let first = !FIRST_PAINT.with(|f| f.replace(true));
        window.on_next_frame(move |_, _| {
            let ms = |at: Instant| at.elapsed().as_secs_f64() * 1000.;
            log::debug!(target: TARGET, "frame_ms={:.2}", ms(started));
            if let Some(at) = key {
                log::debug!(target: TARGET, "key_to_paint_ms={:.2}", ms(at));
            }
            if let Some(at) = summon {
                log::debug!(target: TARGET, "summon_ms={:.2}", ms(at));
            }
            if first {
                milestone("first_paint");
            }
        });
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

/// Call from a root view's `render`.
pub fn frame_rendered(_window: &mut Window) {
    #[cfg(feature = "feel-metrics")]
    imp::frame_rendered(_window);
}
