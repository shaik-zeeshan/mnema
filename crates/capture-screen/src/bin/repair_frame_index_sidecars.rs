use std::{
    fs,
    path::{Path, PathBuf},
};

/// Visible-segment container extensions across platforms (`.mp4` on Windows,
/// `.mov` on macOS). This offline tool may run against recordings produced on
/// either OS, so it probes the current platform's container first
/// (`capture_runtime::screen_segment_extension`) and then the rest.
const SIBLING_VIDEO_EXTENSIONS: [&str; 2] = ["mp4", "mov"];

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
            "usage: cargo run -p capture-screen --bin repair_frame_index_sidecars -- <recordings-root>"
                .to_string(),
        );
    };
    if args.next().is_some() {
        return Err(
            "usage: cargo run -p capture-screen --bin repair_frame_index_sidecars -- <recordings-root>"
                .to_string(),
        );
    }

    let recordings_root = PathBuf::from(recordings_root_arg);
    let mut scanned = 0_u64;
    let mut repaired = 0_u64;
    let mut skipped = 0_u64;
    let mut stack = vec![recordings_root.clone()];

    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir)
            .map_err(|error| format!("failed to read {}: {error}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                format!(
                    "failed to read directory entry under {}: {error}",
                    dir.display()
                )
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
                .is_some_and(|name| name.ends_with(".frame-index.bin"))
            {
                continue;
            }

            scanned = scanned.saturating_add(1);
            let bytes = fs::read(&path)
                .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
            let index = capture_screen::decode_screen_segment_frame_index(&bytes)
                .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
            if capture_screen::screen_segment_frame_index_offsets_are_monotonic(&index.entries) {
                skipped = skipped.saturating_add(1);
                continue;
            }

            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| format!("invalid UTF-8 file name for {}", path.display()))?;
            let segment_stem = file_name
                .strip_suffix(".frame-index.bin")
                .unwrap_or(file_name);
            let Some(video_path) = resolve_sibling_video(&path, segment_stem) else {
                return Err(format!(
                    "missing sibling video for sidecar {} (tried stem {} with extensions {})",
                    path.display(),
                    segment_stem,
                    SIBLING_VIDEO_EXTENSIONS.join(", ")
                ));
            };

            let rebuilt = capture_screen::rebuild_screen_segment_frame_index_from_video(
                &video_path,
                &index.entries,
            )?;
            if !capture_screen::screen_segment_frame_index_offsets_are_monotonic(&rebuilt.entries) {
                return Err(format!(
                    "rebuilt sidecar remained non-monotonic for {}",
                    path.display()
                ));
            }

            fs::write(
                &path,
                capture_screen::encode_screen_segment_frame_index(&rebuilt),
            )
            .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
            repaired = repaired.saturating_add(1);
        }
    }

    println!(
        "scanned={} repaired={} skipped={}",
        scanned, repaired, skipped
    );
    Ok(())
}

/// Resolve the visible-segment video that sits beside a `<stem>.frame-index.bin`
/// sidecar. Tries the current platform's container extension first, then the
/// other known extensions, returning the first candidate that exists on disk.
fn resolve_sibling_video(sidecar_path: &Path, segment_stem: &str) -> Option<PathBuf> {
    let platform_ext = capture_runtime::screen_segment_extension();
    std::iter::once(platform_ext)
        .chain(
            SIBLING_VIDEO_EXTENSIONS
                .into_iter()
                .filter(|ext| *ext != platform_ext),
        )
        .map(|ext| sidecar_path.with_file_name(format!("{segment_stem}.{ext}")))
        .find(|candidate| candidate.is_file())
}
