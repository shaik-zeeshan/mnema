use std::path::Path;

use crate::{hidden_segment_workspace::HiddenSegmentWorkspacePaths, processing::Frame};

pub(crate) fn cleanup_frame_artifacts(frames: &[Frame]) -> Vec<(String, std::io::Error)> {
    let mut errors = Vec::new();

    for frame in frames {
        let path = Path::new(&frame.file_path);
        if !is_safe_frame_artifact_path(path) {
            capture_runtime::debug_log!(
                "[app-infra][frame-batches] skipping cleanup of frame artifact with unsafe path: {}",
                frame.file_path
            );
            continue;
        }
        let hidden_segment_workspace_dir = hidden_segment_workspace_dir_for_frame(path);
        if let Some(segment_dir) = hidden_segment_workspace_dir {
            if !is_managed_hidden_segment_workspace_dir(segment_dir) {
                capture_runtime::debug_log!(
                    "[app-infra][frame-batches] skipping cleanup of hidden segment workspace frame outside managed recordings subtree: {}",
                    frame.file_path
                );
                continue;
            }
        }
        if should_preserve_hidden_workspace_frame(path) {
            continue;
        }
        if path.exists() {
            if let Err(error) = std::fs::remove_file(path) {
                errors.push((frame.file_path.clone(), error));
                continue;
            }
        }
        if hidden_segment_workspace_dir.is_some() {
            // Leave hidden segment workspace directories in place. The active
            // segment may still export new Screen Frame Artifacts into the same
            // frames/ directory after earlier persisted frames are finalized.
            continue;
        }
        if let Some(frames_dir) = path.parent() {
            try_remove_empty_dir(frames_dir, &mut errors);
            if let Some(segment_dir) = frames_dir.parent() {
                try_remove_empty_dir(segment_dir, &mut errors);
            }
        }
    }

    errors
}

pub(crate) fn is_safe_frame_artifact_path(path: &Path) -> bool {
    if !path.is_absolute() {
        return false;
    }
    if path
        .components()
        .any(|c| c == std::path::Component::ParentDir)
    {
        return false;
    }
    let frames_dir = match path.parent() {
        Some(p) => p,
        None => return false,
    };
    let parent_is_frames = frames_dir.file_name().is_some_and(|name| name == "frames");
    if !parent_is_frames {
        return false;
    }
    let segment_dir_name = frames_dir
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("");
    if !segment_dir_name.contains("-segment-") {
        return false;
    }
    let file_name = match path.file_name().and_then(|n| n.to_str()) {
        Some(n) => n,
        None => return false,
    };
    file_name.starts_with("frame-")
        && [".png", ".jpg", ".jpeg"]
            .into_iter()
            .any(|ext| file_name.ends_with(ext))
}

fn should_preserve_hidden_workspace_frame(path: &Path) -> bool {
    let Some(paths) = HiddenSegmentWorkspacePaths::from_frame_artifact_path(path) else {
        return false;
    };
    !Path::new(&paths.visible_segment_path).exists()
}

fn is_numeric_path_component(path: &Path, expected_len: usize) -> bool {
    let Some(component) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };

    component.len() == expected_len && component.chars().all(|ch| ch.is_ascii_digit())
}

fn is_managed_hidden_segment_workspace_dir(segment_dir: &Path) -> bool {
    let Some(segment_dir_name) = segment_dir.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if !segment_dir_name.starts_with('.') || !segment_dir_name.contains("-segment-") {
        return false;
    }

    let Some(day_dir) = segment_dir.parent() else {
        return false;
    };
    let Some(month_dir) = day_dir.parent() else {
        return false;
    };
    let Some(year_dir) = month_dir.parent() else {
        return false;
    };
    let Some(recordings_dir) = year_dir.parent() else {
        return false;
    };
    let Some(dot_z_dir) = recordings_dir.parent() else {
        return false;
    };

    is_numeric_path_component(year_dir, 4)
        && is_numeric_path_component(month_dir, 2)
        && is_numeric_path_component(day_dir, 2)
        && recordings_dir
            .file_name()
            .is_some_and(|name| name == "recordings")
        && dot_z_dir.file_name().is_some_and(|name| name == ".z")
}

fn hidden_segment_workspace_dir_for_frame(path: &Path) -> Option<&Path> {
    let frames_dir = path.parent()?;
    let segment_dir = frames_dir.parent()?;
    Some(segment_dir)
}

fn try_remove_empty_dir(dir: &Path, errors: &mut Vec<(String, std::io::Error)>) {
    match std::fs::remove_dir(dir) {
        Ok(()) => {}
        Err(e) if is_benign_dir_remove_error(&e) => {}
        Err(e) => {
            errors.push((dir.to_string_lossy().to_string(), e));
        }
    }
}

fn is_benign_dir_remove_error(e: &std::io::Error) -> bool {
    match e.kind() {
        std::io::ErrorKind::NotFound => true,
        _ => {
            #[cfg(unix)]
            {
                matches!(e.raw_os_error(), Some(66) | Some(39))
            }
            #[cfg(not(unix))]
            {
                matches!(e.raw_os_error(), Some(145))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("frame-batch-cleanup-{label}-{unique}"));
            fs::create_dir_all(&path).expect("test directory should exist");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }

        fn managed_recordings_day_path(&self, year: &str, month: &str, day: &str) -> PathBuf {
            self.path.join(format!(".z/recordings/{year}/{month}/{day}"))
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn test_frame(file_path: PathBuf) -> Frame {
        Frame {
            id: 1,
            session_id: "s".to_string(),
            file_path: file_path.to_string_lossy().to_string(),
            captured_at: "2026-04-12T10:01:00Z".to_string(),
            width: None,
            height: None,
            equivalence: crate::processing::FrameEquivalence {
                hint: None,
                proof: None,
                version: None,
                status: None,
                error: None,
            },
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn is_safe_frame_artifact_path_accepts_valid_paths() {
        assert!(is_safe_frame_artifact_path(Path::new(
            "/data/session/session-a-segment-0001/frames/frame-1717000123456-000042.png"
        )));
        assert!(is_safe_frame_artifact_path(Path::new(
            "/tmp/my-session-segment-0001/frames/frame-1.png"
        )));
        assert!(is_safe_frame_artifact_path(Path::new(
            "/tmp/my-session-segment-0001/frames/frame-1.jpg"
        )));
        assert!(is_safe_frame_artifact_path(Path::new(
            "/tmp/my-session-segment-0001/frames/frame-1.jpeg"
        )));
    }

    #[test]
    fn is_safe_frame_artifact_path_rejects_missing_segment_grandparent() {
        assert!(!is_safe_frame_artifact_path(Path::new("/tmp/frames/frame-1.png")));
        assert!(!is_safe_frame_artifact_path(Path::new("/data/session/frames/frame-1.png")));
    }

    #[test]
    fn is_safe_frame_artifact_path_rejects_relative_paths() {
        assert!(!is_safe_frame_artifact_path(Path::new("frames/frame-1.png")));
    }

    #[test]
    fn is_safe_frame_artifact_path_rejects_path_traversal() {
        assert!(!is_safe_frame_artifact_path(Path::new(
            "/data/session/../../../etc/frames/frame-1.png"
        )));
    }

    #[test]
    fn is_safe_frame_artifact_path_rejects_non_png_extension() {
        assert!(!is_safe_frame_artifact_path(Path::new(
            "/data/session-a-segment-0001/frames/frame-1.txt"
        )));
        assert!(!is_safe_frame_artifact_path(Path::new(
            "/data/session-a-segment-0001/frames/frame-1"
        )));
    }

    #[test]
    fn is_safe_frame_artifact_path_rejects_wrong_parent_dir() {
        assert!(!is_safe_frame_artifact_path(Path::new(
            "/data/session-a-segment-0001/frame-1.png"
        )));
        assert!(!is_safe_frame_artifact_path(Path::new("/etc/passwd")));
    }

    #[test]
    fn is_safe_frame_artifact_path_rejects_wrong_filename_prefix() {
        assert!(!is_safe_frame_artifact_path(Path::new(
            "/data/session-a-segment-0001/frames/screenshot.png"
        )));
        assert!(!is_safe_frame_artifact_path(Path::new(
            "/data/session-a-segment-0001/frames/not-a-frame.png"
        )));
    }

    #[test]
    fn cleanup_skips_unsafe_paths() {
        let dir = TestDir::new("unsafe-cleanup");
        let bad_file = dir.path().join("important.txt");
        fs::write(&bad_file, b"important data").expect("file should be written");

        let errors = cleanup_frame_artifacts(&[test_frame(bad_file.clone())]);
        assert!(errors.is_empty(), "no errors expected for skipped paths");
        assert!(bad_file.exists(), "file with unsafe path must not be deleted");
    }

    #[test]
    fn cleanup_preserves_hidden_segment_dirs_after_frame_removal() {
        let dir = TestDir::new("empty-dir-cleanup");
        let recordings_day_dir = dir.managed_recordings_day_path("2026", "04", "12");
        let segment_dir = recordings_day_dir.join(".session-x-segment-0001");
        let frames_dir = segment_dir.join("frames");
        fs::create_dir_all(&frames_dir).expect("frames dir should be created");
        fs::write(recordings_day_dir.join("session-x-segment-0001.mov"), b"fake mov")
            .expect("visible segment should be written");
        let frame_path = frames_dir.join("frame-1.png");
        fs::write(&frame_path, b"fake").expect("frame file should be written");

        let errors = cleanup_frame_artifacts(&[test_frame(frame_path.clone())]);
        assert!(errors.is_empty(), "cleanup should succeed without errors");
        assert!(!frame_path.exists(), "frame file should be deleted");
        assert!(frames_dir.exists(), "hidden workspace frames/ dir should remain");
        assert!(segment_dir.exists(), "hidden workspace dir should remain");
    }

    #[test]
    fn cleanup_preserves_hidden_segment_dirs_when_frame_already_missing() {
        let dir = TestDir::new("already-missing-frame");
        let recordings_day_dir = dir.managed_recordings_day_path("2026", "04", "12");
        let segment_dir = recordings_day_dir.join(".session-z-segment-0001");
        let frames_dir = segment_dir.join("frames");
        fs::create_dir_all(&frames_dir).expect("frames dir should be created");
        fs::write(recordings_day_dir.join("session-z-segment-0001.mov"), b"fake mov")
            .expect("visible segment should be written");
        let frame_path = frames_dir.join("frame-1.png");

        let errors = cleanup_frame_artifacts(&[test_frame(frame_path)]);
        assert!(errors.is_empty(), "cleanup should succeed without errors");
        assert!(frames_dir.exists());
        assert!(segment_dir.exists());
    }

    #[test]
    fn cleanup_preserves_segment_workspace_and_separate_audio_dir() {
        let dir = TestDir::new("audio-separate-cleanup");
        let recordings_day_dir = dir.managed_recordings_day_path("2026", "04", "19");
        let segment_dir = recordings_day_dir.join(".session-audio-sep-segment-0001");
        let frames_dir = segment_dir.join("frames");
        fs::create_dir_all(&frames_dir).expect("frames dir should be created");
        fs::write(
            recordings_day_dir.join("session-audio-sep-segment-0001.mov"),
            b"fake mov",
        )
        .expect("visible segment should be written");
        let frame_path = frames_dir.join("frame-1.png");
        fs::write(&frame_path, b"fake png").expect("frame file should be written");

        let audio_dir = recordings_day_dir.join("audio");
        fs::create_dir_all(&audio_dir).expect("audio dir should be created");
        let audio_file = audio_dir.join("system-audio-session-audio-sep-segment-0001.m4a");
        fs::write(&audio_file, b"fake audio").expect("audio file should be written");

        let errors = cleanup_frame_artifacts(&[test_frame(frame_path.clone())]);
        assert!(errors.is_empty(), "cleanup should succeed without errors: {errors:?}");
        assert!(!frame_path.exists());
        assert!(frames_dir.exists());
        assert!(segment_dir.exists());
        assert!(audio_file.exists());
        assert!(audio_dir.exists());
    }

    #[test]
    fn cleanup_preserves_hidden_segment_workspace_structure() {
        let dir = TestDir::new("workspace-recursive-cleanup");
        let segment_dir = dir
            .path()
            .join(".z/recordings/2026/04/12/.session-y-segment-0001");
        let frames_dir = segment_dir.join("frames");
        fs::create_dir_all(&frames_dir).expect("frames dir should be created");
        fs::write(
            segment_dir
                .parent()
                .expect("segment dir should have a date parent")
                .join("session-y-segment-0001.mov"),
            b"fake mov",
        )
        .expect("visible segment should be written");
        let frame_path = frames_dir.join("frame-1.png");
        fs::write(&frame_path, b"fake").expect("frame file should be written");
        let other_file = segment_dir.join("metadata.json");
        fs::write(&other_file, b"{}").expect("other file should be written");

        let errors = cleanup_frame_artifacts(&[test_frame(frame_path.clone())]);
        assert!(errors.is_empty(), "cleanup should succeed without errors");
        assert!(!frame_path.exists());
        assert!(frames_dir.exists());
        assert!(other_file.exists());
        assert!(segment_dir.exists());
    }

    #[test]
    fn cleanup_preserves_hidden_workspace_for_follow_on_exports() {
        let dir = TestDir::new("shared-workspace-mixed-batches");
        let segment_dir = dir
            .path()
            .join(".z/recordings/2026/04/12/.session-shared-segment-0001");
        let frames_dir = segment_dir.join("frames");
        fs::create_dir_all(&frames_dir).expect("frames dir should be created");
        fs::write(
            segment_dir
                .parent()
                .expect("segment dir should have a date parent")
                .join("session-shared-segment-0001.mov"),
            b"fake mov",
        )
        .expect("visible segment should be written");

        let first_frame_path = frames_dir.join("frame-1.png");
        let second_frame_path = frames_dir.join("frame-2.png");
        fs::write(&first_frame_path, b"fake").expect("first frame file should be written");
        fs::write(&second_frame_path, b"fake").expect("second frame file should be written");

        let errors = cleanup_frame_artifacts(&[test_frame(first_frame_path.clone())]);
        assert!(errors.is_empty(), "cleanup should succeed without errors");
        assert!(!first_frame_path.exists());
        assert!(second_frame_path.exists());
        assert!(frames_dir.exists());
        assert!(segment_dir.exists());
    }

    #[test]
    fn cleanup_preserves_hidden_workspace_frame_when_visible_segment_is_missing() {
        let dir = TestDir::new("missing-visible-segment");
        let recordings_day_dir = dir.managed_recordings_day_path("2026", "04", "12");
        let segment_dir = recordings_day_dir.join(".session-preview-segment-0001");
        let frames_dir = segment_dir.join("frames");
        fs::create_dir_all(&frames_dir).expect("frames dir should be created");
        let frame_path = frames_dir.join("frame-1.png");
        fs::write(&frame_path, b"fake").expect("frame file should be written");

        let errors = cleanup_frame_artifacts(&[test_frame(frame_path.clone())]);
        assert!(errors.is_empty(), "cleanup should succeed without errors");
        assert!(frame_path.exists());
        assert!(frames_dir.exists());
        assert!(segment_dir.exists());
    }

    #[test]
    fn cleanup_removes_hidden_workspace_frame_but_preserves_parent_dirs_when_visible_segment_exists() {
        let dir = TestDir::new("visible-segment-present");
        let recordings_day_dir = dir.managed_recordings_day_path("2026", "04", "12");
        let segment_dir = recordings_day_dir.join(".session-preview-segment-0001");
        let frames_dir = segment_dir.join("frames");
        fs::create_dir_all(&frames_dir).expect("frames dir should be created");
        fs::write(
            recordings_day_dir.join("session-preview-segment-0001.mov"),
            b"fake mov",
        )
        .expect("visible segment should be written");
        let frame_path = frames_dir.join("frame-1.png");
        fs::write(&frame_path, b"fake").expect("frame file should be written");

        let errors = cleanup_frame_artifacts(&[test_frame(frame_path.clone())]);
        assert!(errors.is_empty(), "cleanup should succeed without errors");
        assert!(!frame_path.exists());
        assert!(frames_dir.exists());
        assert!(segment_dir.exists());
    }

    #[test]
    fn cleanup_removes_hidden_workspace_jpeg_but_preserves_parent_dirs_when_visible_segment_exists() {
        let dir = TestDir::new("visible-segment-present-jpeg");
        let recordings_day_dir = dir.managed_recordings_day_path("2026", "04", "12");
        let segment_dir = recordings_day_dir.join(".session-preview-segment-0001");
        let frames_dir = segment_dir.join("frames");
        fs::create_dir_all(&frames_dir).expect("frames dir should be created");
        fs::write(
            recordings_day_dir.join("session-preview-segment-0001.mov"),
            b"fake mov",
        )
        .expect("visible segment should be written");
        let frame_path = frames_dir.join("frame-1.jpg");
        fs::write(&frame_path, b"fake").expect("frame file should be written");

        let errors = cleanup_frame_artifacts(&[test_frame(frame_path.clone())]);
        assert!(errors.is_empty(), "cleanup should succeed without errors");
        assert!(!frame_path.exists());
        assert!(frames_dir.exists());
        assert!(segment_dir.exists());
    }

    #[test]
    fn cleanup_leaves_out_of_tree_hidden_segment_workspace_untouched() {
        let dir = TestDir::new("out-of-tree-workspace");
        let segment_dir = dir.path().join("2026/04/12/.session-out-of-tree-segment-0001");
        let frames_dir = segment_dir.join("frames");
        fs::create_dir_all(&frames_dir).expect("frames dir should be created");
        fs::write(
            segment_dir
                .parent()
                .expect("segment dir should have a date parent")
                .join("session-out-of-tree-segment-0001.mov"),
            b"fake mov",
        )
        .expect("visible segment should be written");
        let frame_path = frames_dir.join("frame-1.png");
        fs::write(&frame_path, b"fake").expect("frame file should be written");
        let other_file = segment_dir.join("metadata.json");
        fs::write(&other_file, b"{}").expect("other file should be written");

        let errors = cleanup_frame_artifacts(&[test_frame(frame_path.clone())]);
        assert!(errors.is_empty(), "cleanup should succeed without errors");
        assert!(frame_path.exists());
        assert!(frames_dir.exists());
        assert!(other_file.exists());
        assert!(segment_dir.exists());
    }
}
