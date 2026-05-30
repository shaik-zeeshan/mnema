//! On-hardware smoke harness for the Windows Graphics Capture backend.
//!
//! Drives the real `start_capture_session_with_options` seam on this machine,
//! records the primary monitor for a few seconds, finalizes the segment, and
//! reports the resulting `.mp4` path, encoded dimensions, frame artifact
//! dimensions, and size. It also enables the ~1 fps frame export path and prints
//! artifact cadence.
//!
//! Run with: `cargo run -p capture-screen --example win_smoke -- --resolution 720p --bitrate low`

use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::Duration;

use capture_screen::{
    start_capture_session_with_options, ScreenCaptureSession, ScreenCaptureSessionOptions,
    ScreenCaptureSources, ScreenFrameArtifact, ScreenFrameExportConfig,
    DEFAULT_SCREEN_FRAME_EXPORT_INTERVAL,
};
use capture_types::{ScreenResolution, ScreenResolutionPreset};
use image::GenericImageView;

#[cfg(target_os = "windows")]
use std::os::windows::ffi::OsStrExt;
#[cfg(target_os = "windows")]
use windows::core::PCWSTR;
#[cfg(target_os = "windows")]
use windows::Win32::Media::MediaFoundation::{
    MFCreateSourceReaderFromURL, MFShutdown, MFStartup, MFSTARTUP_FULL, MF_MT_FRAME_SIZE,
    MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_VERSION,
};

#[derive(Debug)]
struct SmokeConfig {
    label: String,
    resolution_label: String,
    resolution: ScreenResolution,
    bitrate_label: String,
    bitrate_bps: Option<u32>,
    seconds: u64,
    frame_rate: u32,
    expected_dimensions: Option<(u32, u32)>,
}

fn main() {
    let config = parse_args();
    let dir = std::env::temp_dir()
        .join("mnema_win_smoke")
        .join(safe_path_component(&config.label));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    let out = dir.join(format!(
        "screen_smoke_{}.mp4",
        safe_path_component(&config.label)
    ));
    let _ = std::fs::remove_file(&out);

    let sources = ScreenCaptureSources {
        screen: true,
        system_audio: false,
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

    println!(
        "[win_smoke] starting label={} resolution={} bitrate={} bps={:?} -> {}",
        config.label,
        config.resolution_label,
        config.bitrate_label,
        config.bitrate_bps,
        out.display()
    );
    let mut started = match start_capture_session_with_options(
        &dir,
        Some(out.as_path()),
        None,
        &sources,
        config.frame_rate,
        &config.resolution,
        config.bitrate_bps,
        options,
    ) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[win_smoke] start failed: {} / {}", e.code, e.message);
            std::process::exit(2);
        }
    };

    println!(
        "[win_smoke] recording ~{}s (console output is animated to create screen changes)...",
        config.seconds
    );
    animate_console_activity(config.seconds);

    if let Err(e) = started.session.stop(0) {
        eprintln!("[win_smoke] stop failed: {} / {}", e.code, e.message);
        std::process::exit(3);
    }

    let bytes = match std::fs::metadata(&out) {
        Ok(m) if m.len() > 0 => m.len(),
        Ok(_) => {
            eprintln!("[win_smoke] output file is empty");
            std::process::exit(4);
        }
        Err(e) => {
            eprintln!("[win_smoke] no output file: {e}");
            std::process::exit(4);
        }
    };

    let artifacts = artifacts.lock().expect("frame artifact list poisoned");
    let artifact_dimensions = report_frame_artifact_cadence(&artifacts);
    let jpeg_dimensions = inspect_first_frame_dimensions(&artifacts);
    let video_dimensions = inspect_mp4_dimensions(&out).unwrap_or_else(|error| {
        eprintln!("[win_smoke] failed to inspect mp4 dimensions: {error}");
        std::process::exit(6);
    });

    if video_dimensions != artifact_dimensions {
        eprintln!(
            "[win_smoke] video dimensions {:?} did not match artifact metadata {:?}",
            video_dimensions, artifact_dimensions
        );
        std::process::exit(7);
    }
    if jpeg_dimensions != artifact_dimensions {
        eprintln!(
            "[win_smoke] decoded JPEG dimensions {:?} did not match artifact metadata {:?}",
            jpeg_dimensions, artifact_dimensions
        );
        std::process::exit(8);
    }
    if let Some(expected) = config.expected_dimensions {
        if video_dimensions != expected {
            eprintln!(
                "[win_smoke] expected dimensions {:?}, got {:?}",
                expected, video_dimensions
            );
            std::process::exit(9);
        }
    }

    println!("[win_smoke] OK: wrote {} bytes -> {}", bytes, out.display());
    println!(
        "[win_smoke] recording_file reported by session: {}",
        started.recording_file
    );
    println!(
        "[win_smoke] RESULT label={} resolution={} bitrate={} bps={:?} video={}x{} frame={}x{} bytes={} file={}",
        config.label,
        config.resolution_label,
        config.bitrate_label,
        config.bitrate_bps,
        video_dimensions.0,
        video_dimensions.1,
        artifact_dimensions.0,
        artifact_dimensions.1,
        bytes,
        out.display()
    );
}

fn parse_args() -> SmokeConfig {
    let mut resolution_label = "original".to_string();
    let mut bitrate_label = "medium".to_string();
    let mut seconds = 4_u64;
    let mut frame_rate = 30_u32;
    let mut label: Option<String> = None;
    let mut expected_dimensions: Option<(u32, u32)> = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let Some(value) = args.next() else {
            eprintln!("[win_smoke] missing value for {arg}");
            std::process::exit(10);
        };

        match arg.as_str() {
            "--resolution" => resolution_label = value,
            "--bitrate" => bitrate_label = value,
            "--seconds" => {
                seconds = value.parse().unwrap_or_else(|_| {
                    eprintln!("[win_smoke] invalid seconds: {value}");
                    std::process::exit(10);
                });
            }
            "--frame-rate" => {
                frame_rate = value.parse().unwrap_or_else(|_| {
                    eprintln!("[win_smoke] invalid frame rate: {value}");
                    std::process::exit(10);
                });
            }
            "--label" => label = Some(value),
            "--expect" => expected_dimensions = Some(parse_dimensions(&value)),
            _ => {
                eprintln!("[win_smoke] unknown argument: {arg}");
                std::process::exit(10);
            }
        }
    }

    let resolution = parse_resolution(&resolution_label);
    let bitrate_bps = bitrate_bps_for_preset(&bitrate_label, &resolution, frame_rate);
    let label = label.unwrap_or_else(|| format!("{resolution_label}-{bitrate_label}"));

    SmokeConfig {
        label,
        resolution_label,
        resolution,
        bitrate_label,
        bitrate_bps,
        seconds,
        frame_rate,
        expected_dimensions,
    }
}

fn parse_resolution(value: &str) -> ScreenResolution {
    let preset = match value {
        "original" => ScreenResolutionPreset::Original,
        "1080p" => ScreenResolutionPreset::P1080,
        "720p" => ScreenResolutionPreset::P720,
        "540p" => ScreenResolutionPreset::P540,
        other => {
            eprintln!("[win_smoke] unsupported resolution preset: {other}");
            std::process::exit(10);
        }
    };

    ScreenResolution::Preset { preset }
}

fn bitrate_bps_for_preset(
    bitrate_label: &str,
    resolution: &ScreenResolution,
    frame_rate: u32,
) -> Option<u32> {
    let factor = match bitrate_label {
        "none" => return None,
        "low" => 0.07,
        "medium" => 0.10,
        "high" => 0.14,
        other => {
            eprintln!("[win_smoke] unsupported bitrate preset: {other}");
            std::process::exit(10);
        }
    };

    let (width, height) = match resolution {
        ScreenResolution::Preset { preset } => match preset {
            ScreenResolutionPreset::Original => (1920, 1080),
            ScreenResolutionPreset::P1080 => (1920, 1080),
            ScreenResolutionPreset::P720 => (1280, 720),
            ScreenResolutionPreset::P540 => (960, 540),
        },
        ScreenResolution::Custom { width, height } => (*width, *height),
    };
    let raw = (width as f64) * (height as f64) * (frame_rate as f64) * factor;
    Some(clamp_and_round_bitrate_bits_per_second(raw))
}

fn clamp_and_round_bitrate_bits_per_second(raw_bps: f64) -> u32 {
    let clamped = raw_bps.clamp(500_000.0, 120_000_000.0).round() as u64;
    let step = 250_000_u64;
    (((clamped + (step / 2)) / step) * step) as u32
}

fn parse_dimensions(value: &str) -> (u32, u32) {
    let Some((width, height)) = value.split_once('x') else {
        eprintln!("[win_smoke] expected dimensions as WIDTHxHEIGHT, got {value}");
        std::process::exit(10);
    };
    let width = width.parse().unwrap_or_else(|_| {
        eprintln!("[win_smoke] invalid expected width: {width}");
        std::process::exit(10);
    });
    let height = height.parse().unwrap_or_else(|_| {
        eprintln!("[win_smoke] invalid expected height: {height}");
        std::process::exit(10);
    });
    (width, height)
}

fn safe_path_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn animate_console_activity(seconds: u64) {
    let ticks = seconds.saturating_mul(10).max(1);
    for tick in 0..ticks {
        let bar_len = ((tick % 40) + 1) as usize;
        print!(
            "\r[win_smoke] motion {:03}/{} {}",
            tick + 1,
            ticks,
            "#".repeat(bar_len)
        );
        let _ = std::io::stdout().flush();
        sleep(Duration::from_millis(100));
    }
    println!();
}

fn inspect_first_frame_dimensions(artifacts: &[ScreenFrameArtifact]) -> (u32, u32) {
    let first = artifacts.first().unwrap_or_else(|| {
        eprintln!("[win_smoke] no frame artifacts were exported");
        std::process::exit(5);
    });

    let image = image::ImageReader::open(&first.file_path)
        .unwrap_or_else(|error| {
            eprintln!(
                "[win_smoke] failed to open frame artifact {}: {error}",
                first.file_path
            );
            std::process::exit(5);
        })
        .decode()
        .unwrap_or_else(|error| {
            eprintln!(
                "[win_smoke] failed to decode frame artifact {}: {error}",
                first.file_path
            );
            std::process::exit(5);
        });

    image.dimensions()
}

#[cfg(target_os = "windows")]
fn inspect_mp4_dimensions(path: &Path) -> Result<(u32, u32), String> {
    struct MfGuard;

    impl Drop for MfGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = MFShutdown();
            }
        }
    }

    unsafe {
        MFStartup(MF_VERSION, MFSTARTUP_FULL)
            .map_err(|error| format!("MFStartup failed: {error}"))?;
    }
    let _guard = MfGuard;

    let url: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let reader = unsafe { MFCreateSourceReaderFromURL(PCWSTR(url.as_ptr()), None) }
        .map_err(|error| format!("MFCreateSourceReaderFromURL failed: {error}"))?;
    let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
    let media_type = unsafe {
        reader
            .GetNativeMediaType(stream, 0)
            .or_else(|_| reader.GetCurrentMediaType(stream))
            .or_else(|_| reader.GetNativeMediaType(0, 0))
    }
    .map_err(|error| format!("failed to read video media type: {error}"))?;
    let packed = unsafe { media_type.GetUINT64(&MF_MT_FRAME_SIZE) }
        .map_err(|error| format!("failed to read MF_MT_FRAME_SIZE: {error}"))?;
    Ok(((packed >> 32) as u32, (packed & 0xffff_ffff) as u32))
}

#[cfg(not(target_os = "windows"))]
fn inspect_mp4_dimensions(_path: &Path) -> Result<(u32, u32), String> {
    Err("Windows Media Foundation inspection is only available on Windows".to_string())
}

fn report_frame_artifact_cadence(artifacts: &[ScreenFrameArtifact]) -> (u32, u32) {
    if artifacts.is_empty() {
        eprintln!("[win_smoke] no frame artifacts were exported");
        std::process::exit(5);
    }

    let dimensions = artifacts
        .first()
        .and_then(|artifact| Some((artifact.width?, artifact.height?)))
        .unwrap_or_else(|| {
            eprintln!("[win_smoke] first frame artifact did not report dimensions");
            std::process::exit(5);
        });

    if let Some(mismatched) = artifacts.iter().find(|artifact| {
        artifact.width != Some(dimensions.0) || artifact.height != Some(dimensions.1)
    }) {
        eprintln!(
            "[win_smoke] mismatched frame artifact dimensions: {} {:?}x{:?}, expected {}x{}",
            mismatched.file_path, mismatched.width, mismatched.height, dimensions.0, dimensions.1
        );
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
        "[win_smoke] OK: exported {} frame artifact(s); dimensions={}x{} intervals_ms={:?}",
        artifacts.len(),
        dimensions.0,
        dimensions.1,
        intervals
    );
    dimensions
}
