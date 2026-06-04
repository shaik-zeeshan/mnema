//! Windows microphone Audio Activity Samples + VAD PCM feed smoke test (issue #55).
//!
//! Drives the *real* WASAPI capture backend and exercises the debug-visible
//! sample emission seam (`get_idle_debug` surface): peak-since-last-poll Audio
//! Activity Samples plus the VAD PCM feed. This is the on-device sanity check
//! for the acceptance criterion "Emission adds no measurable regression to the
//! capture hot path"; it confirms emission is actually wired without claiming
//! pause/resume or audio processing (those arrive in a later inactivity slice).
//!
//! Run from a PowerShell with the MSVC + Perl/NASM env imported:
//!   cargo run -p capture-microphone --example smoke_mic_activity

#[cfg(target_os = "windows")]
fn main() {
    use std::time::{Duration, Instant};

    use capture_microphone::{
        last_microphone_activity_unix_ms, microphone_activity_level,
        microphone_permission_state, microphone_vad_pcm_frame_count,
        peek_microphone_activity_window_peak_level,
        start_wasapi_microphone_capture_session_for_file, take_microphone_vad_pcm_frames,
        AudioCaptureSession, MICROPHONE_VAD_PCM_FRAME_SAMPLE_COUNT,
        MICROPHONE_VAD_PCM_SAMPLE_RATE_HZ,
    };
    use capture_types::CapturePermissionState;

    println!("== Windows microphone Audio Activity Samples + VAD PCM feed smoke test ==");

    let state = microphone_permission_state();
    println!("microphone_permission_state = {state:?}");
    // On Windows the per-app privacy setting cannot be queried synchronously, so a
    // usable default endpoint reports as `Unknown` (never `Granted`); capture start
    // is the real arbiter. Treat `Unknown` as runnable here and bail only when no
    // endpoint exists (`Unsupported`) or access is known-denied.
    if !matches!(
        state,
        CapturePermissionState::Granted | CapturePermissionState::Unknown
    ) {
        eprintln!("FAIL: no usable microphone endpoint on this machine ({state:?})");
        std::process::exit(2);
    }

    let dir = std::env::temp_dir().join("mnema_mic_activity_smoke");
    let _ = std::fs::create_dir_all(&dir);
    let seg = dir.join("mic_session-segment-0000.m4a");
    let seg_s = seg.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&seg);

    println!("output dir: {}", dir.display());
    println!("VAD PCM feed expectation: sample_rate_hz={MICROPHONE_VAD_PCM_SAMPLE_RATE_HZ}, samples.len()={MICROPHONE_VAD_PCM_FRAME_SAMPLE_COUNT} per frame");
    println!("\nstarting capture session (default capture endpoint) ...");
    let mut session = match start_wasapi_microphone_capture_session_for_file(&seg_s, None) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("FAIL: start session: {} / {}", e.code, e.message);
            std::process::exit(3);
        }
    };
    println!("session started; is_live={}", session.is_live());

    let mut max_peak: f32 = 0.0;
    let mut saw_activity_timestamp = false;
    let mut max_vad_frame_count: usize = 0;

    println!("\npolling debug-visible samples for ~4s in ~250ms steps:");
    let start = Instant::now();
    let mut step = 0u32;
    while start.elapsed() < Duration::from_secs(4) {
        std::thread::sleep(Duration::from_millis(250));
        step += 1;

        let peak = peek_microphone_activity_window_peak_level();
        let last_ms = last_microphone_activity_unix_ms();
        let level = microphone_activity_level();
        let vad_frames = microphone_vad_pcm_frame_count();

        if let Some(p) = peak {
            if p > max_peak {
                max_peak = p;
            }
        }
        if last_ms.is_some() {
            saw_activity_timestamp = true;
        }
        if vad_frames > max_vad_frame_count {
            max_vad_frame_count = vad_frames;
        }

        println!(
            "  [poll {step:02}] peak_since_last={peak:?} last_activity_unix_ms={last_ms:?} level={level:?} vad_pcm_frames={vad_frames}"
        );
    }

    println!("\ndraining VAD PCM frames (take up to 96):");
    let frames = take_microphone_vad_pcm_frames(96);
    println!("  drained frames.len()={}", frames.len());
    if let Some(frame) = frames.first() {
        println!(
            "  first frame: sample_rate_hz={} samples.len()={} normalized_peak_level={} media_start_secs={:?} media_end_secs={:?}",
            frame.sample_rate_hz,
            frame.samples.len(),
            frame.normalized_peak_level,
            frame.media_start_secs,
            frame.media_end_secs,
        );
    }

    println!("\nstopping session ...");
    match session.stop_returning_finalization() {
        Ok(f) => println!("  stop finalization: {f:?}"),
        Err(e) => eprintln!("  stop error: {} / {}", e.code, e.message),
    }
    println!("  is_live after stop = {}", session.is_live());

    println!(
        "\nsummary: max_peak={max_peak} saw_activity_timestamp={saw_activity_timestamp} max_vad_frame_count={max_vad_frame_count} drained_frames={}",
        frames.len()
    );

    let produced_vad = max_vad_frame_count > 0 || !frames.is_empty();
    if saw_activity_timestamp && produced_vad {
        println!(
            "\nPASS: observed an Audio Activity Sample timestamp and at least one VAD PCM frame."
        );
    } else {
        eprintln!(
            "\nFAIL: did not observe an activity timestamp ({saw_activity_timestamp}) and/or a VAD PCM frame (produced={produced_vad})."
        );
        eprintln!(
            "  Note: total silence on the mic can legitimately still produce frames (silence is fed"
        );
        eprintln!(
            "  as level-0 samples), so a FAIL most likely means emission is not wired, not that the"
        );
        eprintln!("  room was quiet.");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("smoke_mic_activity is a Windows-only smoke test");
}
