//! Windows system-audio loopback -> .m4a smoke test (issue #56).
//!
//! Drives the real default-render-endpoint WASAPI loopback backend + Media
//! Foundation AAC/M4A sink and validates each produced segment through the same
//! MF positive-duration probe the runtime finalization seam uses. Records two
//! manually rotated segments so the runtime's 5-minute boundary rotate arm uses
//! the same backend operation without waiting for the full boundary.
//!
//! Play desktop audio while this runs, or start it from a shell that also emits
//! audio (for example, Windows speech synthesis).
//!
//! Run from a PowerShell with the MSVC + Perl/NASM env imported:
//!   cargo run -p capture-microphone --example smoke_system_audio_m4a

#[cfg(target_os = "windows")]
fn main() {
    use std::time::{Duration, Instant};

    use capture_microphone::{
        last_system_audio_activity_unix_ms, peek_system_audio_activity_window_peak_level,
        start_wasapi_system_audio_capture_session_for_file, system_audio_activity_idle_ms,
        system_audio_activity_level, system_audio_loopback_capture_supported, AudioCaptureSession,
    };
    use capture_writers::windows_audio_file_has_positive_duration;

    fn record_for(label: &str, secs: u64) {
        println!("  recording {label} for {secs}s ...");
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(secs) {
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn report(label: &str, path: &str) -> bool {
        let exists = std::path::Path::new(path).exists();
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let openable = exists && windows_audio_file_has_positive_duration(path);
        println!(
            "  [{label}] exists={exists} size={size}B mf_positive_duration={openable}\n         path={path}"
        );
        exists && size > 0 && openable
    }

    fn report_system_audio_activity_sample() -> bool {
        let last_unix_ms = last_system_audio_activity_unix_ms();
        let level = system_audio_activity_level();
        let idle_ms = system_audio_activity_idle_ms();
        let window_peak_level = peek_system_audio_activity_window_peak_level();

        println!("\nvalidating issue #59 debug-input system-audio activity sample:");
        println!("  last_system_audio_activity_unix_ms={last_unix_ms:?}");
        println!("  system_audio_activity_level={level:?}");
        println!("  system_audio_activity_idle_ms={idle_ms:?}");
        println!("  peek_system_audio_activity_window_peak_level={window_peak_level:?}");

        last_unix_ms.is_some()
            && level.is_some()
            && idle_ms.is_some()
            && window_peak_level.is_some()
    }

    println!("== Windows system audio loopback -> .m4a smoke test ==");

    let supported = system_audio_loopback_capture_supported();
    println!("system_audio_loopback_capture_supported = {supported}");
    if !supported {
        eprintln!("FAIL: no default render endpoint available for WASAPI loopback");
        std::process::exit(2);
    }

    let dir = std::env::temp_dir().join("mnema_system_audio_smoke");
    let _ = std::fs::create_dir_all(&dir);
    let seg1 = dir.join("sysaudio_session-segment-0000.m4a");
    let seg2 = dir.join("sysaudio_session-segment-0001.m4a");
    let seg1_s = seg1.to_string_lossy().to_string();
    let seg2_s = seg2.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&seg1);
    let _ = std::fs::remove_file(&seg2);

    println!("output dir: {}", dir.display());
    println!("\nstarting capture session (default render endpoint loopback) ...");
    let mut session = match start_wasapi_system_audio_capture_session_for_file(&seg1_s) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("FAIL: start session: {} / {}", e.code, e.message);
            std::process::exit(3);
        }
    };
    println!("session started; is_live={}", session.is_live());

    record_for("segment 0", 3);

    println!("\nrotating to segment 1 ...");
    let fin1 = match session.rotate_output_file_returning_finalization(&seg2_s) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("FAIL: rotate: {} / {}", e.code, e.message);
            std::process::exit(4);
        }
    };
    println!("  rotate finalization: {fin1:?}");

    record_for("segment 1", 3);

    println!("\nstopping session ...");
    let fin2 = match session.stop_returning_finalization() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("FAIL: stop: {} / {}", e.code, e.message);
            std::process::exit(5);
        }
    };
    println!("  stop finalization: {fin2:?}");
    if let Some(err) = session.take_stop_error() {
        eprintln!("  WARN async stop error: {} / {}", err.code, err.message);
    }
    println!("  is_live after stop = {}", session.is_live());

    println!("\nvalidating produced segments via MF Source Reader duration probe:");
    let ok1 = report("segment 0", &seg1_s);
    let ok2 = report("segment 1", &seg2_s);

    if ok1 && ok2 {
        let activity_sample_observed = report_system_audio_activity_sample();
        if !activity_sample_observed {
            eprintln!("\nFAIL: no complete issue #59 system-audio activity sample was observed. Ensure desktop audio is playing while the smoke test runs.");
            std::process::exit(6);
        }

        println!("\nPASS: both rotated system-audio segments are openable .m4a files with positive duration, and the system-audio activity sample was observed.");
    } else {
        eprintln!("\nFAIL: at least one segment was missing/empty/unopenable. Ensure desktop audio is playing while the smoke test runs.");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("smoke_system_audio_m4a is a Windows-only smoke test");
}
