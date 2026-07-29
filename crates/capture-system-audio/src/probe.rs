//! A listening tap that reports the level it is hearing *right now*.
//!
//! System audio has no readable grant (ADR 0052), so onboarding's "play
//! anything" moment can only prove itself with sound arriving. The permission
//! evidence row cannot do that job: it is sticky and cross-session, so a Mac
//! that heard sound last week reads as "arriving" in silence — and a Mac playing
//! music right now reads as nothing until a real recording session runs. This is
//! the live signal both of those need.
//!
//! Deliberately separate from [`crate::activity`]: that signal drives inactivity
//! and the denial heuristic and belongs to the recording session. A probe run
//! from a settings screen must not be able to teach it anything.
//!
//! Self-stopping. The tap lives on its own thread and goes away a few seconds
//! after the last poll, so there is no start/stop pair to get wrong and no way
//! to leak a tap when the window that was polling disappears.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use capture_types::CaptureErrorResponse;
use capture_writers::{peak_audio_activity_level_from_audio_buffers, AudioSampleFormat};

use crate::tap::SystemAudioTapSession;
use crate::LOG_PREFIX;

static RUNNING: AtomicBool = AtomicBool::new(false);
static PEAK_BITS: AtomicU32 = AtomicU32::new(0);
static POLLED: AtomicBool = AtomicBool::new(false);

const TICK: Duration = Duration::from_millis(200);
/// ~3 s of silence from the poller ends the probe. Long enough to survive a
/// stalled webview frame, short enough that nobody notices the tap outliving
/// the screen that asked for it.
const IDLE_TICKS_BEFORE_STOP: u32 = 15;

/// Starts the probe unless one is already running.
///
/// Building the tap is what raises the "Screen & System Audio Recording" prompt,
/// so the first call on a machine that has never been asked prompts exactly like
/// [`crate::prompt_for_system_audio_permission`] — with the difference that this
/// one stays to listen. `Ok` means the tap runs, never that it was granted: a
/// denied tap runs perfectly and delivers zeros.
pub fn start_system_audio_level_probe() -> Result<(), CaptureErrorResponse> {
    if !crate::supports_system_audio_capture() {
        return Ok(());
    }
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Ok(());
    }

    let (tx, rx) = std::sync::mpsc::channel();
    // The tap is created and dropped on this thread and never crosses another,
    // which is also what keeps the delicate teardown order in one place.
    std::thread::spawn(move || {
        // The delivery format is only knowable once the tap exists, so the
        // callback reads it through a cell filled immediately after start. The
        // handful of deliveries that may beat it simply score nothing.
        let format: Arc<OnceLock<AudioSampleFormat>> = Arc::new(OnceLock::new());
        let callback_format = Arc::clone(&format);
        let session = SystemAudioTapSession::start(&[], move |_, buffers| {
            let Some(format) = callback_format.get() else {
                return;
            };
            if let Some(level) = peak_audio_activity_level_from_audio_buffers(buffers, *format) {
                record_peak(level);
            }
        });

        let session = match session {
            Ok(session) => {
                let _ = tx.send(Ok(()));
                session
            }
            Err(error) => {
                RUNNING.store(false, Ordering::SeqCst);
                let _ = tx.send(Err(error));
                return;
            }
        };
        let _ = format.set(sample_format_of(&session));

        let mut idle_ticks = 0;
        while idle_ticks < IDLE_TICKS_BEFORE_STOP {
            std::thread::sleep(TICK);
            idle_ticks = if POLLED.swap(false, Ordering::Relaxed) {
                0
            } else {
                idle_ticks + 1
            };
        }

        drop(session);
        PEAK_BITS.store(0, Ordering::Relaxed);
        POLLED.store(false, Ordering::Relaxed);
        RUNNING.store(false, Ordering::SeqCst);
        capture_runtime::debug_log!("{LOG_PREFIX} level probe stopped (no longer polled)");
    });

    rx.recv().unwrap_or_else(|_| {
        RUNNING.store(false, Ordering::SeqCst);
        Err(CaptureErrorResponse {
            code: "system_audio_tap_start_failed".to_string(),
            message: "system audio level probe thread died before starting".to_string(),
        })
    })
}

/// Peak level (0.0–1.0) delivered since the previous poll, or `None` when no
/// probe is running. Polling is also the keepalive — stop calling this and the
/// tap tears itself down.
pub fn take_system_audio_probe_level() -> Option<f32> {
    if !RUNNING.load(Ordering::SeqCst) {
        return None;
    }
    POLLED.store(true, Ordering::Relaxed);
    Some(take_peak())
}

fn sample_format_of(session: &SystemAudioTapSession) -> AudioSampleFormat {
    let asbd = session.asbd();
    AudioSampleFormat {
        sample_rate_hz: asbd.sample_rate,
        format_id: asbd.format.0,
        format_flags: asbd.format_flags.0,
        bytes_per_packet: asbd.bytes_per_packet,
        frames_per_packet: asbd.frames_per_packet,
        bytes_per_frame: asbd.bytes_per_frame,
        channels_per_frame: asbd.channels_per_frame,
        bits_per_channel: asbd.bits_per_channel,
    }
}

/// Peak rather than latest: deliveries arrive far faster than the meter is
/// polled, so keeping the loudest one is what makes a short blip visible.
fn record_peak(level: f32) {
    let level = level.clamp(0.0, 1.0);
    let level_bits = level.to_bits();
    let mut observed_bits = PEAK_BITS.load(Ordering::Relaxed);
    while f32::from_bits(observed_bits) < level {
        match PEAK_BITS.compare_exchange_weak(
            observed_bits,
            level_bits,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(next_bits) => observed_bits = next_bits,
        }
    }
}

fn take_peak() -> f32 {
    f32::from_bits(PEAK_BITS.swap(0, Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_returns_the_window_peak_and_resets_it() {
        PEAK_BITS.store(0, Ordering::Relaxed);

        record_peak(0.10);
        record_peak(0.70);
        record_peak(0.20);

        assert_eq!(take_peak(), 0.70);
        assert_eq!(take_peak(), 0.0);
    }

    #[test]
    fn no_probe_running_reads_as_no_signal() {
        assert!(!RUNNING.load(Ordering::SeqCst));
        assert_eq!(take_system_audio_probe_level(), None);
    }
}
