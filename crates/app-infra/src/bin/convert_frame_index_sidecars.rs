use std::{fs, path::PathBuf};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct LegacyScreenSegmentFrameIndexEntry {
    captured_at_unix_ms: u64,
    frame_index: u64,
    video_offset_ms: u64,
}

#[derive(Debug, Deserialize)]
struct LegacyScreenSegmentFrameIndex {
    version: u32,
    entries: Vec<LegacyScreenSegmentFrameIndexEntry>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let Some(recordings_root_arg) = args.next() else {
        return Err(
            "usage: cargo run -p app-infra --bin convert_frame_index_sidecars -- <recordings-root>"
                .to_string(),
        );
    };
    if args.next().is_some() {
        return Err(
            "usage: cargo run -p app-infra --bin convert_frame_index_sidecars -- <recordings-root>"
                .to_string(),
        );
    }

    let recordings_root = PathBuf::from(recordings_root_arg);
    let mut converted = 0_u64;
    let mut skipped = 0_u64;
    let mut stack = vec![recordings_root.clone()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir)
            .map_err(|error| format!("failed to read {}: {error}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                format!("failed to read directory entry under {}: {error}", dir.display())
            })?;
            let path = entry.path();
            let file_type = entry.file_type().map_err(|error| {
                format!("failed to read file type for {}: {error}", path.display())
            })?;
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            if !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".frame-index.json"))
            {
                continue;
            }

            let binary_path = path.with_file_name(
                path.file_name()
                    .and_then(|name| name.to_str())
                    .expect("json sidecar file name should be valid utf-8")
                    .replace(".frame-index.json", ".frame-index.bin"),
            );
            if binary_path.exists() {
                skipped = skipped.saturating_add(1);
                continue;
            }

            let bytes = fs::read(&path)
                .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
            let legacy: LegacyScreenSegmentFrameIndex = serde_json::from_slice(&bytes)
                .map_err(|error| format!("failed to parse legacy sidecar {}: {error}", path.display()))?;
            let binary = capture_screen::encode_screen_segment_frame_index(
                &capture_screen::ScreenSegmentFrameIndex {
                    version: legacy.version,
                    entries: legacy
                        .entries
                        .into_iter()
                        .map(|entry| capture_screen::ScreenSegmentFrameIndexEntry {
                            captured_at_unix_ms: entry.captured_at_unix_ms,
                            frame_index: entry.frame_index,
                            video_offset_ms: entry.video_offset_ms,
                        })
                        .collect(),
                },
            );
            fs::write(&binary_path, binary)
                .map_err(|error| format!("failed to write {}: {error}", binary_path.display()))?;
            converted = converted.saturating_add(1);
        }
    }

    println!("converted={} skipped={}", converted, skipped);
    Ok(())
}
