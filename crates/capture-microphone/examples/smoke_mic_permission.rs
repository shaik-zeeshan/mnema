//! Windows microphone permission UX smoke test (issue #54, Phase A).
//!
//! Drives the *real* WASAPI capture backend and reports how a Windows
//! microphone-privacy denial surfaces. This is a manual on-device harness:
//!
//! Manual procedure:
//!   1. Run once with mic privacy ENABLED:
//!        cargo run -p capture-microphone --example smoke_mic_permission
//!      Expect: "capture started (access granted)".
//!   2. Toggle Windows Settings -> Privacy & security -> Microphone ->
//!      "Let desktop apps access your microphone" OFF.
//!   3. Run again. Expect: `microphone_access_denied` and a PASS line.
//!
//! Run from a PowerShell with the MSVC + Perl/NASM env imported.

#[cfg(target_os = "windows")]
fn main() {
    use capture_microphone::{
        microphone_permission_state, start_wasapi_microphone_capture_session_for_file,
        AudioCaptureSession,
    };

    println!("== Windows microphone permission smoke test ==");
    println!("microphone_permission_state = {:?}", microphone_permission_state());

    let dir = std::env::temp_dir().join("mnema_mic_permission_smoke");
    let _ = std::fs::create_dir_all(&dir);
    let seg = dir.join("mic_permission-segment-0000.m4a");
    let seg_s = seg.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&seg);

    println!("output path: {seg_s}");
    println!("\nattempting capture session (default capture endpoint) ...");

    match start_wasapi_microphone_capture_session_for_file(&seg_s, None) {
        Ok(mut session) => {
            println!("capture started (access granted); is_live={}", session.is_live());
            // Stop and clean up; we only care that activation succeeded.
            if let Err(e) = session.stop_returning_finalization() {
                eprintln!("  WARN stop: {} / {}", e.code, e.message);
            }
            let _ = std::fs::remove_file(&seg);
            std::process::exit(0);
        }
        Err(e) => {
            println!("capture start failed:");
            println!("  code    = {}", e.code);
            println!("  message = {}", e.message);
            if e.code == "microphone_access_denied" {
                println!("PASS: access-denied correctly surfaced as recoverable error");
            }
            // Manual harness: a failure is an expected outcome (privacy OFF), so
            // exit 0 either way.
            std::process::exit(0);
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("smoke_mic_permission is a Windows-only smoke test");
}
