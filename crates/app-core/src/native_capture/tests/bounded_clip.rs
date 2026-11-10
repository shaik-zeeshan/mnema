//! The bounded voice-enrollment take ends early on a cancel: the session stops
//! at once, the partial clip is deleted, and the live microphone is handed back
//! exactly as after a full take. A fake session stands in for AVFoundation.

use super::*;
use crate::native_capture::lifecycle::NativeCaptureState;
use crate::native_capture::microphone::bounded_clip::{
    run_take, BoundedClip, MicrophoneLend, TakeCancel, TakeSlot,
};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A fake session: `open` writes the clip file, its stop records when it ran.
struct FakeSession {
    opened: Mutex<u32>,
    stopped_at: Mutex<Vec<Instant>>,
}

impl FakeSession {
    fn new() -> Self {
        Self {
            opened: Mutex::new(0),
            stopped_at: Mutex::new(Vec::new()),
        }
    }

    fn take(
        &self,
        file: &Path,
        duration: Duration,
        cancel: &TakeCancel,
        lend: Option<MicrophoneLend<'_>>,
    ) -> Result<BoundedClip, CaptureErrorResponse> {
        run_take(
            file.to_string_lossy().into_owned(),
            duration,
            cancel,
            lend,
            |path| {
                *self.opened.lock().unwrap() += 1;
                fs::write(path, b"partial").unwrap();
                Ok(move || {
                    self.stopped_at.lock().unwrap().push(Instant::now());
                    Ok(())
                })
            },
        )
    }
}

/// A live session whose microphone the take borrowed; the hand-back's reopen
/// fails before any audio device is touched (see the stranded fixture).
fn lent_state(away: bool) -> NativeCaptureState {
    let mut runtime = stranded_microphone_runtime_fixture();
    runtime.microphone_lent_out = true;
    let probe: fn() -> bool = if away { || true } else { || false };
    runtime.away_probe = Some(probe);
    let mut lifecycle = RecordingLifecycle::default();
    *lifecycle.runtime_mut() = runtime;
    Arc::new(std::sync::Mutex::new(lifecycle))
}

#[test]
fn a_take_cancelled_at_one_second_stops_at_once_and_hands_the_microphone_back() {
    let dir = TestDir::new("bounded-clip-cancel");
    let file = dir.path().join("take.m4a");
    let state = lent_state(false);
    let slot = TakeSlot::new();
    let session = FakeSession::new();

    let (result, cancel_took) = std::thread::scope(|scope| {
        let ticket = slot.begin().expect("the slot is free");
        let take = scope.spawn(|| {
            let ticket = ticket;
            session.take(
                &file,
                Duration::from_secs(15),
                &ticket.cancel,
                Some(MicrophoneLend(&state)),
            )
        });
        std::thread::sleep(Duration::from_secs(1));
        let asked = Instant::now();
        // Returns once the take has closed the microphone and freed the slot.
        slot.cancel_and_wait(Duration::from_secs(2));
        let took = asked.elapsed();
        assert_eq!(
            session.stopped_at.lock().unwrap().len(),
            1,
            "stopped by now"
        );
        (take.join().unwrap(), took)
    });

    assert_eq!(result.unwrap(), BoundedClip::Cancelled);
    assert!(
        cancel_took < Duration::from_millis(500),
        "the session stops at once, not at 15 s ({cancel_took:?})"
    );
    assert!(!file.exists(), "the partial clip is deleted");
    let lifecycle = state.lock().unwrap();
    assert!(!lifecycle.runtime().microphone_lent_out);
    assert!(
        microphone_open_was_attempted(lifecycle.runtime()),
        "the live microphone is reopened as after a full take"
    );
    assert!(slot.begin().is_some(), "the slot is free again");
}

#[test]
fn a_take_cancelled_while_away_hands_back_under_the_away_hold() {
    let dir = TestDir::new("bounded-clip-cancel-away");
    let file = dir.path().join("take.m4a");
    let state = lent_state(true);
    let cancel = TakeCancel::default();
    let session = FakeSession::new();

    let result = std::thread::scope(|scope| {
        let take = scope.spawn(|| {
            session.take(
                &file,
                Duration::from_secs(15),
                &cancel,
                Some(MicrophoneLend(&state)),
            )
        });
        std::thread::sleep(Duration::from_millis(100));
        cancel.cancel();
        take.join().unwrap()
    });

    assert_eq!(result.unwrap(), BoundedClip::Cancelled);
    let lifecycle = state.lock().unwrap();
    assert!(!lifecycle.runtime().microphone_lent_out);
    assert!(lifecycle.runtime().microphone_away_hold, "held while away");
    assert!(!microphone_open_was_attempted(lifecycle.runtime()));
}

#[test]
fn an_uncancelled_take_runs_its_full_length_and_keeps_the_clip() {
    let dir = TestDir::new("bounded-clip-full");
    let file = dir.path().join("take.m4a");
    let cancel = TakeCancel::default();
    let session = FakeSession::new();
    let started = Instant::now();

    let result = session.take(&file, Duration::from_millis(300), &cancel, None);

    let stopped = session.stopped_at.lock().unwrap()[0];
    assert!(stopped - started >= Duration::from_millis(300));
    assert_eq!(
        result.unwrap(),
        BoundedClip::Recorded(file.to_string_lossy().into_owned())
    );
    assert!(file.exists());
}

#[test]
fn a_stale_cancel_never_reaches_the_next_take() {
    let dir = TestDir::new("bounded-clip-stale");
    let file = dir.path().join("take.m4a");
    let slot = TakeSlot::new();
    let session = FakeSession::new();

    // No take running: a no-op that returns at once.
    let asked = Instant::now();
    slot.cancel_and_wait(Duration::from_secs(2));
    assert!(asked.elapsed() < Duration::from_millis(100));

    // One take at a time; a cancelled take's signal dies with it.
    let first = slot.begin().expect("free");
    assert!(slot.begin().is_none(), "a second take is refused");
    slot.cancel();
    assert!(
        first.cancel.wait(Duration::ZERO),
        "the take in progress got it"
    );
    drop(first);
    slot.cancel();
    slot.cancel_and_wait(Duration::from_secs(2));

    let second = slot.begin().expect("free again");
    let result = session.take(&file, Duration::from_millis(200), &second.cancel, None);
    assert!(matches!(result.unwrap(), BoundedClip::Recorded(_)));
}

#[test]
fn a_take_cancelled_before_it_starts_never_opens_the_microphone() {
    let dir = TestDir::new("bounded-clip-early");
    let file = dir.path().join("take.m4a");
    let state = lent_state(true);
    let cancel = TakeCancel::default();
    cancel.cancel();
    let session = FakeSession::new();

    let result = session.take(
        &file,
        Duration::from_secs(15),
        &cancel,
        Some(MicrophoneLend(&state)),
    );

    assert_eq!(result.unwrap(), BoundedClip::Cancelled);
    assert_eq!(*session.opened.lock().unwrap(), 0);
    assert!(!state.lock().unwrap().runtime().microphone_lent_out);
}
