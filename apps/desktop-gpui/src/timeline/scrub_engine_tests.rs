//! The engine on explicit instants. The same constants are asserted again
//! through the view with `advance_clock` in `timeline/tests.rs`.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::*;

fn frames(n: i64) -> Vec<Frame> {
    (0..n)
        .map(|i| Frame {
            id: 1000 - i,
            session_id: String::new(),
            file_path: Arc::from(Path::new("/f.jpg")),
            captured_at_ms: 1_000_000 - i * 2000,
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

fn preview(name: &str) -> Preview {
    Preview {
        file_path: Arc::from(Path::new(name)),
        mime_type: "image/jpeg".into(),
        source_kind: SourceKind::OriginalFrame,
        redactions: 0,
    }
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn velocity_steps_shrink_the_prefetch() {
    assert_eq!(window_for_velocity(1.99), (4, 9));
    assert_eq!(window_for_velocity(2.), (3, 7));
    assert_eq!(window_for_velocity(3.99), (3, 7));
    assert_eq!(window_for_velocity(4.), (2, 5));
    assert_eq!(window_for_velocity(7.99), (2, 5));
    assert_eq!(window_for_velocity(8.), (1, 3));
    assert_eq!(throttle_for_velocity(1.99), THROTTLE);
    assert_eq!(throttle_for_velocity(2.), THROTTLE_FAST);
}

#[test]
fn a_scrub_fetches_the_exact_frame_only_after_it_rests_500ms() {
    let f = frames(20);
    let t0 = Instant::now();
    let mut e = ScrubEngine::default();
    // Parked load: the exact frame goes at once.
    assert_eq!(
        e.on_active_changed(t0, &f, 0, false),
        [Effect::FetchExact(1000)]
    );
    e.exact_loaded(t0, 1000, Ok(Some(preview("/a"))), &f, 0);
    let t1 = t0 + ms(1000);
    let fx = e.on_active_changed(t1, &f, 3, true);
    assert!(!fx.iter().any(|x| matches!(x, Effect::FetchExact(_))));
    assert!(e
        .tick(t1 + EXACT_SETTLE - ms(1), &f, 3)
        .iter()
        .all(|x| !matches!(x, Effect::FetchExact(_))));
    assert!(e
        .tick(t1 + EXACT_SETTLE, &f, 3)
        .contains(&Effect::FetchExact(997)));
}

#[test]
fn the_scrub_tier_is_a_160ms_throttle_with_the_velocity_radius() {
    let f = frames(40);
    let t0 = Instant::now();
    let mut e = ScrubEngine::default();
    e.on_active_changed(t0, &f, 10, true);
    // More scrubs inside the window move the target, not the deadline.
    e.on_active_changed(t0 + ms(100), &f, 12, true);
    assert!(e.tick(t0 + THROTTLE - ms(1), &f, 12).is_empty());
    let fx = e.tick(t0 + THROTTLE, &f, 12);
    let Some(Effect::FetchScrub { ids, .. }) = fx.first() else {
        panic!("{fx:?}")
    };
    assert_eq!(ids.len(), 9, "radius 4 → 9 frames at rest speed");
    assert_eq!(ids[0], f[12].id, "the active frame first");
}

#[test]
fn the_display_swaps_at_most_every_80ms_and_parks_after_180ms() {
    let f = frames(10);
    let t0 = Instant::now();
    let mut e = ScrubEngine::default();
    for (i, id) in f.iter().map(|x| x.id).enumerate() {
        e.touch_exact(id, preview(&format!("/{i}")));
    }
    e.on_active_changed(t0, &f, 1, true);
    assert_eq!(e.display().unwrap().frame_id, f[1].id);
    e.on_active_changed(t0 + DISPLAY_THROTTLE - ms(1), &f, 2, true);
    assert_eq!(e.display().unwrap().frame_id, f[1].id, "79 ms: no swap");
    e.on_active_changed(t0 + DISPLAY_THROTTLE, &f, 3, true);
    assert_eq!(e.display().unwrap().frame_id, f[3].id, "80 ms: swap");
    let last = t0 + DISPLAY_THROTTLE;
    e.tick(last + DISPLAY_SETTLE - ms(1), &f, 3);
    assert_eq!(e.mode(), Mode::Scrubbing);
    e.tick(last + DISPLAY_SETTLE, &f, 3);
    assert_eq!(e.mode(), Mode::Parked);
}

#[test]
fn a_failed_exact_fetch_is_not_retried_for_5s() {
    let f = frames(5);
    let t0 = Instant::now();
    let mut e = ScrubEngine::default();
    assert_eq!(
        e.on_active_changed(t0, &f, 0, false),
        [Effect::FetchExact(1000)]
    );
    e.exact_loaded(t0, 1000, Err("not found".into()), &f, 0);
    let has = |fx: Vec<Effect>| fx.contains(&Effect::FetchExact(1000));
    // Back on the frame so its 500 ms settle ends at 4999 ms: still a cached failure.
    let back = t0 + FAILURE_TTL - EXACT_SETTLE - ms(1);
    e.on_active_changed(back, &f, 1, false);
    e.on_active_changed(back, &f, 0, false);
    assert!(!has(e.tick(back + EXACT_SETTLE, &f, 0)), "4999 ms");
    // Ending at 5000 ms: retried.
    let back = t0 + FAILURE_TTL - EXACT_SETTLE;
    e.on_active_changed(back, &f, 1, false);
    e.on_active_changed(back, &f, 0, false);
    assert!(has(e.tick(back + EXACT_SETTLE, &f, 0)), "5000 ms");
}

#[test]
fn the_warm_up_fires_1200ms_after_the_last_scrub() {
    let f = frames(20);
    let t0 = Instant::now();
    let mut e = ScrubEngine::default();
    e.on_active_changed(t0, &f, 5, true);
    let warm = |fx: Vec<Effect>| fx.iter().any(|x| matches!(x, Effect::Warm { .. }));
    // The scrub batch at 160 ms is in flight; answer it so only the warm remains.
    let fx = e.tick(t0 + THROTTLE, &f, 5);
    let Some(Effect::FetchScrub {
        generation, ids, ..
    }) = fx.first().cloned()
    else {
        panic!("{fx:?}")
    };
    e.scrub_loaded(t0 + THROTTLE, generation, &ids, Ok(vec![]), &f, 5);
    assert!(!warm(e.tick(t0 + WARM_SETTLE - ms(1), &f, 5)));
    assert!(warm(e.tick(t0 + WARM_SETTLE, &f, 5)));
}

#[test]
fn the_exact_cache_holds_192() {
    let mut e = ScrubEngine::default();
    for id in 0..(CACHE_MAX as i64 + 1) {
        e.touch_exact(id, preview("/p"));
    }
    assert!(e.exact(0).is_none(), "the oldest went");
    assert!(e.exact(1).is_some() && e.exact(CACHE_MAX as i64).is_some());
}
