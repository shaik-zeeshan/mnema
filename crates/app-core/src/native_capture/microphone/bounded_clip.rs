//! The microphone and system-audio level meters, and the bounded microphone
//! clip voice enrollment records (lending the live session's microphone out
//! for the take).

#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
#[cfg(target_os = "macos")]
use std::time::Duration;

use capture_microphone as microphone_capture;
use capture_types::CaptureErrorResponse;

#[cfg(target_os = "macos")]
use super::{resolve_capture_microphone_device_id, MicrophoneControllerPreferencesState};
#[cfg(target_os = "macos")]
use crate::debug_log;
#[cfg(target_os = "macos")]
use crate::native_capture::lifecycle::NativeCaptureState;
#[cfg(target_os = "macos")]
use crate::native_capture::runtime;
use crate::AppCore;
#[cfg(target_os = "macos")]
use crate::WindowSignal;

/// Longest clip the bounded recorder will take. The caller picks the length
/// (the Voice screen asks for ~15 s); this only stops a runaway request from
/// holding the microphone open indefinitely. No other judgment lives here —
/// whether a clip is usable is the enrollment embedder's call, not this
/// adapter's.
#[cfg(target_os = "macos")]
const MAX_BOUNDED_MICROPHONE_CLIP_MS: u64 = 60_000;
#[cfg(target_os = "macos")]
const MIN_BOUNDED_MICROPHONE_CLIP_MS: u64 = 500;

// ponytail: one process-wide flag instead of a queue — the microphone is a
// single device and there is exactly one enrollment surface. If a second caller
// ever needs to wait rather than be refused, make it a mutex.
#[cfg(target_os = "macos")]
static BOUNDED_MICROPHONE_CLIP_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// Latest normalized microphone level (0.0–1.0), or `None` when no microphone
/// session has produced a sample. Read off the one activity probe every
/// microphone session already feeds — the level meter costs nothing beyond this
/// poll and never opens a second audio tap.
pub fn get_microphone_activity_level() -> Option<f32> {
    microphone_capture::microphone_activity_level()
}

/// Start listening for system audio so a level meter has something true to show.
///
/// The microphone's meter rides the recording session's own probe; system audio
/// has no session to ride outside of capture, and its permission state is sticky
/// evidence rather than a live reading (ADR 0052) — so proving "sound is
/// arriving" needs a tap of its own. Building it raises the TCC prompt, the same
/// way `request_capture_permission("systemAudio")` does. Self-stopping a few
/// seconds after the last poll.
#[cfg(target_os = "macos")]
pub async fn start_system_audio_level_probe(core: &AppCore) -> Result<(), String> {
    core.spawn_blocking(capture_system_audio::start_system_audio_level_probe)
        .await
        .map_err(|error| format!("system audio level probe failed to start: {error}"))?
        .map_err(|error| {
            format!(
                "system audio level probe failed to start: {}",
                error.message
            )
        })
}

#[cfg(not(target_os = "macos"))]
pub async fn start_system_audio_level_probe(_core: &AppCore) -> Result<(), String> {
    Ok(())
}

/// Peak system-audio level (0.0–1.0) since the previous poll, or `None` when no
/// probe is running. Polling is the probe's keepalive.
#[cfg(target_os = "macos")]
pub fn get_system_audio_probe_level() -> Option<f32> {
    capture_system_audio::take_system_audio_probe_level()
}

#[cfg(not(target_os = "macos"))]
pub fn get_system_audio_probe_level() -> Option<f32> {
    None
}

#[cfg(target_os = "macos")]
fn microphone_device_id_for_bounded_clip(
    preferences: &MicrophoneControllerPreferencesState,
) -> Option<String> {
    let runtime = preferences.lock().ok()?;
    let controller_state = microphone_capture::microphone_controller_state(
        runtime.preference.clone(),
        runtime.disconnect_policy.clone(),
    )
    .ok()?;
    resolve_capture_microphone_device_id(&controller_state)
}

/// Record a fixed-length microphone clip to a temp file and return its path.
///
/// A hardware adapter with no judgment: it does not inspect, score, or reject
/// what it recorded. The live-capture guard (pause the microphone, record,
/// resume) is owned by `native_capture::lifecycle`, so this runs anywhere —
/// onboarding, or Settings re-enroll during a live session.
pub async fn record_bounded_microphone_clip(
    duration_ms: u64,
    core: &Arc<AppCore>,
) -> Result<String, CaptureErrorResponse> {
    #[cfg(target_os = "macos")]
    {
        // The take blocks for its whole duration; keep it off the UI thread and,
        // more importantly, off the `NativeCaptureState` mutex the segment loop
        // ticks on.
        let task_core = core.clone();
        core.spawn_blocking(move || {
            record_bounded_microphone_clip_blocking(&task_core, duration_ms)
        })
        .await
        .map_err(|error| CaptureErrorResponse {
            code: "microphone_clip_task_failed".to_string(),
            message: format!("Bounded microphone recording task failed: {error}"),
        })?
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (duration_ms, core);
        Err(CaptureErrorResponse {
            code: "unsupported_platform".to_string(),
            message: "Bounded microphone recording is only implemented on macOS".to_string(),
        })
    }
}

#[cfg(target_os = "macos")]
fn record_bounded_microphone_clip_blocking(
    core: &AppCore,
    duration_ms: u64,
) -> Result<String, CaptureErrorResponse> {
    if BOUNDED_MICROPHONE_CLIP_IN_FLIGHT.swap(true, Ordering::SeqCst) {
        return Err(CaptureErrorResponse {
            code: "microphone_clip_already_recording".to_string(),
            message: "A bounded microphone recording is already in progress".to_string(),
        });
    }
    let _in_flight = BoundedMicrophoneClipGuard;

    let device_id = microphone_device_id_for_bounded_clip(
        &core.state::<MicrophoneControllerPreferencesState>(),
    );

    let capture_state = core.state::<NativeCaptureState>();
    let released = {
        let mut lifecycle = capture_state.lock().expect("native capture state poisoned");
        lifecycle.release_microphone_for_out_of_band_recording()?
    };
    // Hands the microphone back when the take ends, however it ends.
    let lend = released.then(|| MicrophoneLend(&capture_state));

    let clip = take_bounded_microphone_clip(
        duration_ms.clamp(
            MIN_BOUNDED_MICROPHONE_CLIP_MS,
            MAX_BOUNDED_MICROPHONE_CLIP_MS,
        ),
        device_id.as_deref(),
    );
    drop(lend);

    // The enrollment surface plays the take back before committing to it, and
    // the asset protocol serves only explicitly-allowed paths (empty default
    // scope). Grant this one clip; without it `convertFileSrc` 403s and the
    // playback confirmation is a dead button.
    if let Ok(path) = &clip {
        core.host().window_signal(WindowSignal::AllowAssetPath {
            path: std::path::PathBuf::from(path),
            recursive: false,
        });
    }

    clip
}

/// A microphone lent to a bounded recording, handed back on drop — a take that
/// panics included, or `microphone_lent_out` would keep the live session's
/// microphone down for good.
#[cfg(target_os = "macos")]
pub(crate) struct MicrophoneLend<'a>(pub(crate) &'a NativeCaptureState);

#[cfg(target_os = "macos")]
impl Drop for MicrophoneLend<'_> {
    fn drop(&mut self) {
        let mut lifecycle = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Err(error) = lifecycle.restore_microphone_after_out_of_band_recording() {
            // The clip itself is fine, so this only logs. The tick reopens a
            // microphone left with no session once nothing holds it
            // (`microphone_away::reopen_unwakeable_microphone`).
            debug_log::log_error(format!(
                "failed to restart microphone capture after a bounded enrollment recording: [{}] {}",
                error.code, error.message
            ));
        }
    }
}

#[cfg(target_os = "macos")]
struct BoundedMicrophoneClipGuard;

#[cfg(target_os = "macos")]
impl Drop for BoundedMicrophoneClipGuard {
    fn drop(&mut self) {
        BOUNDED_MICROPHONE_CLIP_IN_FLIGHT.store(false, Ordering::SeqCst);
    }
}

/// Filename prefix every bounded enrollment take carries. It is what makes the
/// clips this recorder wrote identifiable in a shared OS temp dir, and what the
/// enrollment door checks before it opens (and destroys) a caller-named path.
#[cfg(target_os = "macos")]
pub const ENROLLMENT_CLIP_PREFIX: &str = "mnema-voice-enrollment-";

/// Delete enrollment clips left behind in `dir` by an earlier take.
///
/// `voice_enrollment` destroys a clip when it judges it — but only then. A
/// retake, a discarded review, a closed window, or a crash abandons the take,
/// and a raw recording of the user's voice then sits in the OS temp dir
/// indefinitely: outside the encrypted capture store, outside retention, and
/// outside Delete Recent Capture. Sweeping before the next take is the one place
/// every one of those paths passes through.
///
/// ponytail: swept at the start of the next take rather than tracked per clip.
/// A discarded take survives until the user records again; closing that fully
/// would need a delete-on-discard command, i.e. a new arbitrary-path-delete
/// surface — worse than the gap.
#[cfg(target_os = "macos")]
pub fn remove_abandoned_bounded_microphone_clips(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        if file_name.starts_with(ENROLLMENT_CLIP_PREFIX) && file_name.ends_with(".m4a") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(target_os = "macos")]
fn take_bounded_microphone_clip(
    duration_ms: u64,
    device_id: Option<&str>,
) -> Result<String, CaptureErrorResponse> {
    let temp_dir = std::env::temp_dir();
    remove_abandoned_bounded_microphone_clips(&temp_dir);
    let output_file = temp_dir
        .join(format!(
            "{ENROLLMENT_CLIP_PREFIX}{}.m4a",
            runtime::now_unix_ms()
        ))
        .to_string_lossy()
        .to_string();

    let mut session =
        microphone_capture::start_avfoundation_microphone_capture_session_for_file_with_device_id(
            &output_file,
            device_id,
        )?;
    std::thread::sleep(Duration::from_millis(duration_ms));
    session.stop()?;

    Ok(output_file)
}
