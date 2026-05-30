//! On-hardware smoke harness for the Windows Graphics Capture backend.
//!
//! Drives the real `start_capture_session_with_options` seam on this machine,
//! records the primary monitor for a few seconds, finalizes the segment, and
//! reports the resulting `.mp4` path + size so a human can open it and confirm
//! the two MVP acceptance criteria: the capture border is absent
//! (`SetIsBorderRequired(false)`) and the cursor is visible
//! (`SetIsCursorCaptureEnabled(true)`). It also enables the ~1 fps frame export
//! path and prints artifact cadence.
//!
//! Run with: `cargo run -p capture-screen --example win_smoke`

use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::Duration;

use capture_screen::{
    start_capture_session_with_options, ScreenCaptureSession, ScreenCaptureSessionOptions,
    ScreenCaptureSources, ScreenFrameArtifact, ScreenFrameExportConfig,
    DEFAULT_SCREEN_FRAME_EXPORT_INTERVAL,
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
    let artifacts = Arc::new(Mutex::new(Vec::<ScreenFrameArtifact>::new()));
    let captured_artifacts = Arc::clone(&artifacts);
    let options = ScreenCaptureSessionOptions {
        frame_export: Some(ScreenFrameExportConfig {
            minimum_interval: DEFAULT_SCREEN_FRAME_EXPORT_INTERVAL,
            on_frame_exported: Arc::new(move |artifact| {
                println!(
                    "[win_smoke] frame artifact {} {}x{} at {}",
                    artifact.file_path,
                    artifact.width.unwrap_or_default(),
                    artifact.height.unwrap_or_default(),
                    artifact.captured_at_unix_ms
                );
                captured_artifacts
                    .lock()
                    .expect("frame artifact list poisoned")
                    .push(artifact);
            }),
        }),
        ..Default::default()
    };

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
            report_frame_artifact_cadence(&artifacts.lock().expect("frame artifact list poisoned"));
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

fn report_frame_artifact_cadence(artifacts: &[ScreenFrameArtifact]) {
    if artifacts.is_empty() {
        eprintln!("[win_smoke] no frame artifacts were exported");
        std::process::exit(5);
    }

    let intervals: Vec<u64> = artifacts
        .windows(2)
        .map(|pair| {
            pair[1]
                .captured_at_unix_ms
                .saturating_sub(pair[0].captured_at_unix_ms)
        })
        .collect();
    println!(
        "[win_smoke] OK: exported {} frame artifact(s); intervals_ms={:?}",
        artifacts.len(),
        intervals
    );
}
