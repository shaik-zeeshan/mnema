//! On-hardware smoke harness for the Windows Graphics Capture backend.
//!
//! Drives the real `start_capture_session_with_options` seam on this machine,
//! records the primary monitor for a few seconds, finalizes the segment, and
//! reports the resulting `.mp4` path + size so a human can open it and confirm
//! the two MVP acceptance criteria: the capture border is absent
//! (`SetIsBorderRequired(false)`) and the cursor is visible
//! (`SetIsCursorCaptureEnabled(true)`).
//!
//! Run with: `cargo run -p capture-screen --example win_smoke`

use std::thread::sleep;
use std::time::Duration;

use capture_screen::{
    start_capture_session_with_options, ScreenCaptureSession, ScreenCaptureSessionOptions,
    ScreenCaptureSources,
};
use capture_types::{ScreenResolution, ScreenResolutionPreset};

fn main() {
    let dir = std::env::temp_dir().join("mnema_win_smoke");
    let _ = std::fs::create_dir_all(&dir);
    let out = dir.join("screen_smoke.mp4");
    let _ = std::fs::remove_file(&out);

    let sources = ScreenCaptureSources {
        screen: true,
        system_audio: false,
    };
    let resolution = ScreenResolution::Preset {
        preset: ScreenResolutionPreset::Original,
    };
    let options = ScreenCaptureSessionOptions::default();

    println!("[win_smoke] starting capture -> {}", out.display());
    let mut started = match start_capture_session_with_options(
        &dir,
        Some(out.as_path()),
        None,
        &sources,
        30,
        &resolution,
        None,
        options,
    ) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[win_smoke] start failed: {} / {}", e.code, e.message);
            std::process::exit(2);
        }
    };

    let record_secs = 4;
    println!("[win_smoke] recording ~{record_secs}s (move the mouse to exercise the cursor)...");
    sleep(Duration::from_secs(record_secs));

    if let Err(e) = started.session.stop(0) {
        eprintln!("[win_smoke] stop failed: {} / {}", e.code, e.message);
        std::process::exit(3);
    }

    match std::fs::metadata(&out) {
        Ok(m) if m.len() > 0 => {
            println!(
                "[win_smoke] OK: wrote {} bytes -> {}",
                m.len(),
                out.display()
            );
            println!(
                "[win_smoke] recording_file reported by session: {}",
                started.recording_file
            );
        }
        Ok(_) => {
            eprintln!("[win_smoke] output file is empty");
            std::process::exit(4);
        }
        Err(e) => {
            eprintln!("[win_smoke] no output file: {e}");
            std::process::exit(4);
        }
    }
}
