//! The one "now". Views read the wall clock through [`now`], so tests and goldens
//! freeze it (at the fixture instant) and render the same text every run.

use gpui_kit::{App, Global};
use time::OffsetDateTime;

#[derive(Default)]
struct Clock {
    frozen: Option<OffsetDateTime>,
}

impl Global for Clock {}

/// Wall-clock now in the Mac's local offset, or the frozen instant.
pub fn now(cx: &App) -> OffsetDateTime {
    if let Some(frozen) = cx.try_global::<Clock>().and_then(|c| c.frozen) {
        return frozen;
    }
    // Read natively: `time`'s `current_local_offset` refuses in a multi-threaded process.
    OffsetDateTime::now_utc().to_offset(crate::native::sys::local_offset())
}

pub fn now_ms(cx: &App) -> i64 {
    (now(cx).unix_timestamp_nanos() / 1_000_000) as i64
}

/// Pin `now` (tests, goldens). `None` returns to the wall clock.
pub fn freeze(at: Option<OffsetDateTime>, cx: &mut App) {
    cx.set_global(Clock { frozen: at });
}

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;
    use time::macros::datetime;

    use super::*;

    #[gpui_kit::test]
    fn frozen_clock_wins_until_released(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let at = datetime!(2026-11-04 14:32 +05:30);
            freeze(Some(at), cx);
            assert_eq!(now(cx), at);
            assert_eq!(now_ms(cx), 1_793_782_920_000);
            freeze(None, cx);
            assert_ne!(now(cx), at);
        });
    }
}
