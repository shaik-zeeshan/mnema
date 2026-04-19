use capture_types::{CaptureErrorResponse, CaptureOutputFiles, CaptureSources};
#[cfg(target_os = "macos")]
use std::collections::BTreeSet;
#[cfg(target_os = "macos")]
use std::path::Path;

pub(crate) fn set_current_microphone_output_file(
    output_files: &mut CaptureOutputFiles,
    file: String,
) {
    output_files.microphone_file = Some(file.clone());
    output_files.microphone_files.push(file);
}

pub(crate) fn clear_current_microphone_output_file(output_files: &mut CaptureOutputFiles) {
    output_files.microphone_file = None;
    output_files.microphone_files.clear();
}

pub(crate) fn set_current_screen_output_file(output_files: &mut CaptureOutputFiles, file: String) {
    output_files.screen_file = Some(file.clone());
    output_files.screen_files.push(file);
}

pub(crate) fn clear_current_screen_output_file(output_files: &mut CaptureOutputFiles) {
    output_files.screen_file = None;
    output_files.screen_files.clear();
}

pub(crate) fn set_current_system_audio_output_file(
    output_files: &mut CaptureOutputFiles,
    file: String,
) {
    output_files.system_audio_file = Some(file.clone());
    output_files.system_audio_files.push(file);
}

pub(crate) fn clear_current_system_audio_output_file(output_files: &mut CaptureOutputFiles) {
    output_files.system_audio_file = None;
    output_files.system_audio_files.clear();
}

#[cfg(target_os = "macos")]
const MISSING_REQUESTED_SCREEN_OUTPUT_FAILURE_PREFIX: &str =
    "screen output missing: expected screen recording file";
#[cfg(target_os = "macos")]
const MISSING_REQUESTED_SCREEN_OUTPUT_AT_PATH_PREFIX: &str =
    "screen output missing: expected screen recording file at ";

#[cfg(target_os = "macos")]
fn maybe_remove_intermediate_file(file: &str, label: &str, failures: &mut Vec<String>) {
    match std::fs::remove_file(file) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            failures.push(format!(
                "failed to remove intermediate {label} recording file: {error}"
            ));
        }
    }
}

#[cfg(target_os = "macos")]
fn microphone_output_files(output_files: &CaptureOutputFiles) -> Vec<&str> {
    if !output_files.microphone_files.is_empty() {
        output_files
            .microphone_files
            .iter()
            .map(String::as_str)
            .collect()
    } else {
        output_files
            .microphone_file
            .as_deref()
            .into_iter()
            .collect()
    }
}

#[cfg(target_os = "macos")]
fn sync_finalized_screen_output_file(
    output_files: &mut CaptureOutputFiles,
    recording_file: Option<&str>,
) -> bool {
    let Some(recording_file) = recording_file.filter(|path| Path::new(path).is_file()) else {
        clear_current_screen_output_file(output_files);
        return false;
    };

    clear_current_screen_output_file(output_files);
    set_current_screen_output_file(output_files, recording_file.to_string());
    true
}

#[cfg(target_os = "macos")]
fn is_usable_audio_output_file(path: &str, unusable_files: &BTreeSet<String>) -> bool {
    !unusable_files.contains(path)
        && std::fs::metadata(path)
            .map(|metadata| metadata.is_file() && metadata.len() > 0)
            .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn maybe_remove_unusable_audio_output_file(
    file: &str,
    label: &str,
    unusable_files: &BTreeSet<String>,
    failures: &mut Vec<String>,
) {
    if is_usable_audio_output_file(file, unusable_files) {
        return;
    }

    let cleanup_label = format!("unusable {label}");
    maybe_remove_intermediate_file(file, cleanup_label.as_str(), failures);
}

#[cfg(target_os = "macos")]
fn sync_finalized_audio_output_file(
    current_file: &mut Option<String>,
    files: &mut Vec<String>,
    label: &str,
    unusable_files: &BTreeSet<String>,
    failures: &mut Vec<String>,
) {
    let mut removed_paths = BTreeSet::new();

    files.retain(|path| {
        if is_usable_audio_output_file(path, unusable_files) {
            true
        } else {
            if removed_paths.insert(path.clone()) {
                maybe_remove_unusable_audio_output_file(path, label, unusable_files, failures);
            }
            false
        }
    });

    let current = current_file
        .as_deref()
        .filter(|path| is_usable_audio_output_file(path, unusable_files))
        .map(str::to_owned);

    if current.is_none() {
        if let Some(path) = current_file.as_ref() {
            if removed_paths.insert(path.clone()) {
                maybe_remove_unusable_audio_output_file(path, label, unusable_files, failures);
            }
        }
    }

    *current_file = match current {
        Some(current) => {
            if !files.iter().any(|path| path == &current) {
                files.push(current.clone());
            }
            Some(current)
        }
        None => files.last().cloned(),
    };
}

#[cfg(target_os = "macos")]
fn sync_finalized_microphone_output_files(
    output_files: &mut CaptureOutputFiles,
    unusable_files: &BTreeSet<String>,
    failures: &mut Vec<String>,
) {
    sync_finalized_audio_output_file(
        &mut output_files.microphone_file,
        &mut output_files.microphone_files,
        "microphone",
        unusable_files,
        failures,
    );

    if output_files.microphone_file.is_none() {
        clear_current_microphone_output_file(output_files);
    }
}

#[cfg(target_os = "macos")]
fn sync_finalized_system_audio_output_files(
    output_files: &mut CaptureOutputFiles,
    unusable_files: &BTreeSet<String>,
    failures: &mut Vec<String>,
) {
    sync_finalized_audio_output_file(
        &mut output_files.system_audio_file,
        &mut output_files.system_audio_files,
        "system audio",
        unusable_files,
        failures,
    );

    if output_files.system_audio_file.is_none() {
        clear_current_system_audio_output_file(output_files);
    }
}

#[cfg(target_os = "macos")]
fn missing_requested_screen_output_failure(recording_file: Option<&str>) -> String {
    let path_detail = recording_file
        .map(|path| format!(" at {path}"))
        .unwrap_or_default();
    format!("{MISSING_REQUESTED_SCREEN_OUTPUT_FAILURE_PREFIX}{path_detail}")
}

#[cfg(target_os = "macos")]
pub(crate) fn is_missing_requested_screen_output_failure_detail(detail: &str) -> bool {
    detail == MISSING_REQUESTED_SCREEN_OUTPUT_FAILURE_PREFIX
        || detail.starts_with(MISSING_REQUESTED_SCREEN_OUTPUT_AT_PATH_PREFIX)
}

#[cfg(target_os = "macos")]
pub(crate) fn finalize_capture_outputs(
    output_files: Option<&mut CaptureOutputFiles>,
    recording_file: Option<&str>,
    microphone_recording_file: Option<&str>,
    system_audio_recording_file: Option<&str>,
    requested_sources: Option<&CaptureSources>,
) -> Result<(), CaptureErrorResponse> {
    let Some(output_files) = output_files else {
        return Ok(());
    };

    let mut failures: Vec<String> = Vec::new();
    let mut audio_failures: Vec<String> = Vec::new();
    let mut unusable_audio_files: BTreeSet<String> = BTreeSet::new();
    let has_screen_output = sync_finalized_screen_output_file(output_files, recording_file);

    if requested_sources.is_some_and(|sources| sources.screen) && !has_screen_output {
        failures.push(missing_requested_screen_output_failure(recording_file));
    }

    if output_files.microphone_file.is_some() && output_files.microphone_files.is_empty() {
        let microphone_file = output_files
            .microphone_file
            .as_deref()
            .expect("checked microphone_file is present");
        let source_recording = microphone_recording_file;

        if let Some(source_recording) = source_recording {
            if source_recording != microphone_file {
                if let Err(error) = capture_writers::convert_recording_audio_to_m4a(
                    source_recording,
                    microphone_file,
                ) {
                    let _ = unusable_audio_files.insert(microphone_file.to_string());
                    audio_failures.push(format!(
                        "microphone output conversion failed: {}",
                        error.message
                    ));
                }
            }
        } else {
            let _ = unusable_audio_files.insert(microphone_file.to_string());
            audio_failures
                .push("microphone output conversion failed: missing source recording".to_string());
        }
    }

    if let Some(system_audio_file) = output_files.system_audio_file.as_deref() {
        if let Some(source_recording) = system_audio_recording_file {
            if source_recording != system_audio_file {
                if let Err(error) = capture_writers::convert_recording_audio_to_m4a(
                    source_recording,
                    system_audio_file,
                ) {
                    let _ = unusable_audio_files.insert(system_audio_file.to_string());
                    audio_failures.push(format!(
                        "system audio output conversion failed: {}",
                        error.message
                    ));
                }
            }
        } else {
            let _ = unusable_audio_files.insert(system_audio_file.to_string());
            audio_failures.push(
                "system audio output conversion failed: missing source recording".to_string(),
            );
        }
    }

    if has_screen_output && requested_sources.is_some_and(|sources| sources.system_audio) {
        if let Some(recording_file) = recording_file {
            if let Err(error) = capture_screen::strip_audio_from_recording_file(recording_file) {
                failures.push(format!(
                    "screen output video-only conversion failed: {}",
                    error.message
                ));
            }
        }
    }

    sync_finalized_microphone_output_files(
        output_files,
        &unusable_audio_files,
        &mut audio_failures,
    );
    sync_finalized_system_audio_output_files(
        output_files,
        &unusable_audio_files,
        &mut audio_failures,
    );

    let microphone_files = microphone_output_files(output_files);

    if let Some(microphone_recording_file) = microphone_recording_file {
        if !microphone_files.contains(&microphone_recording_file) {
            maybe_remove_intermediate_file(
                microphone_recording_file,
                "microphone",
                &mut audio_failures,
            );
        }
    }

    if let Some(system_audio_recording_file) = system_audio_recording_file {
        if output_files.system_audio_file.as_deref() != Some(system_audio_recording_file) {
            maybe_remove_intermediate_file(
                system_audio_recording_file,
                "system audio",
                &mut audio_failures,
            );
        }
    }

    if !has_screen_output || !failures.is_empty() {
        failures.extend(audio_failures);
    }

    capture_writers::aggregate_output_processing_failures(failures)
}

#[cfg(target_os = "macos")]
fn append_output_file(current_file: &mut Option<String>, files: &mut Vec<String>, file: &str) {
    let file = file.to_string();
    *current_file = Some(file.clone());
    if !files.iter().any(|existing| existing == &file) {
        files.push(file);
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn append_committed_segment_output_files(
    committed: &mut CaptureOutputFiles,
    segment: &CaptureOutputFiles,
) {
    let screen_files = if segment.screen_files.is_empty() {
        segment.screen_file.iter().collect::<Vec<_>>()
    } else {
        segment.screen_files.iter().collect::<Vec<_>>()
    };
    for file in screen_files {
        append_output_file(
            &mut committed.screen_file,
            &mut committed.screen_files,
            file,
        );
    }

    let microphone_files = if segment.microphone_files.is_empty() {
        segment.microphone_file.iter().collect::<Vec<_>>()
    } else {
        segment.microphone_files.iter().collect::<Vec<_>>()
    };
    for file in microphone_files {
        append_output_file(
            &mut committed.microphone_file,
            &mut committed.microphone_files,
            file,
        );
    }

    let system_audio_files = if segment.system_audio_files.is_empty() {
        segment.system_audio_file.iter().collect::<Vec<_>>()
    } else {
        segment.system_audio_files.iter().collect::<Vec<_>>()
    };
    for file in system_audio_files {
        append_output_file(
            &mut committed.system_audio_file,
            &mut committed.system_audio_files,
            file,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    #[cfg(target_os = "macos")]
    struct TestDir {
        path: PathBuf,
    }

    #[cfg(target_os = "macos")]
    impl TestDir {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("native-capture-output-{label}-{unique}"));
            fs::create_dir_all(&path).expect("test directory should exist");
            Self { path }
        }
    }

    #[cfg(target_os = "macos")]
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finalize_capture_outputs_drops_missing_screen_output_and_skips_video_only_conversion() {
        let dir = TestDir::new("missing-screen-output");
        let recording_file = dir.path.join("screen.mov");
        let recording_file = recording_file.to_string_lossy().to_string();
        let requested_sources = CaptureSources {
            screen: true,
            microphone: false,
            system_audio: true,
        };
        let mut output_files = CaptureOutputFiles {
            screen_file: Some(recording_file.clone()),
            screen_files: vec![recording_file.clone()],
            microphone_file: None,
            microphone_files: Vec::new(),
            system_audio_file: None,
            system_audio_files: Vec::new(),
        };

        let error = finalize_capture_outputs(
            Some(&mut output_files),
            Some(&recording_file),
            None,
            None,
            Some(&requested_sources),
        )
        .expect_err("missing requested screen recording must fail finalization");

        assert_eq!(error.code, "capture_output_processing_failed");
        assert!(error
            .message
            .contains("screen output missing: expected screen recording file"));

        assert_eq!(output_files.screen_file, None);
        assert!(output_files.screen_files.is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finalize_capture_outputs_requires_dedicated_microphone_source_recording() {
        let mut output_files = CaptureOutputFiles {
            screen_file: None,
            screen_files: Vec::new(),
            microphone_file: Some("/tmp/final-microphone.m4a".to_string()),
            microphone_files: Vec::new(),
            system_audio_file: None,
            system_audio_files: Vec::new(),
        };

        let error = finalize_capture_outputs(
            Some(&mut output_files),
            Some("/tmp/screen-recording.mov"),
            None,
            None,
            Some(&CaptureSources {
                screen: false,
                microphone: true,
                system_audio: false,
            }),
        )
        .expect_err("microphone finalization should not fall back to the screen recording");

        assert_eq!(error.code, "capture_output_processing_failed");
        assert!(error
            .message
            .contains("microphone output conversion failed: missing source recording"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn sync_finalized_audio_output_files_clears_removed_audio_artifacts_from_bookkeeping() {
        let dir = TestDir::new("sync-audio-bookkeeping");
        let screen_file = dir.path.join("screen.mov");
        fs::write(&screen_file, b"screen").expect("screen artifact should exist");
        let screen_file = screen_file.to_string_lossy().to_string();
        let microphone_kept_file = dir
            .path
            .join("microphone-kept.m4a")
            .to_string_lossy()
            .to_string();
        fs::write(&microphone_kept_file, b"microphone").expect("microphone artifact should exist");
        let missing_microphone_file = dir
            .path
            .join("microphone-missing.m4a")
            .to_string_lossy()
            .to_string();
        let microphone_file = dir
            .path
            .join("microphone-legacy.m4a")
            .to_string_lossy()
            .to_string();
        fs::write(&microphone_file, b"microphone")
            .expect("legacy microphone artifact should exist");
        let empty_microphone_file = dir
            .path
            .join("microphone-empty.m4a")
            .to_string_lossy()
            .to_string();
        fs::File::create(&empty_microphone_file)
            .expect("empty microphone artifact placeholder should exist");
        let empty_system_audio_file = dir
            .path
            .join("system-audio.m4a")
            .to_string_lossy()
            .to_string();
        fs::File::create(&empty_system_audio_file)
            .expect("empty system audio artifact placeholder should exist");
        let mut output_files = CaptureOutputFiles {
            screen_file: Some(screen_file.clone()),
            screen_files: vec![screen_file.clone()],
            microphone_file: Some(missing_microphone_file),
            microphone_files: vec![microphone_kept_file.clone(), empty_microphone_file.clone()],
            system_audio_file: Some(empty_system_audio_file.clone()),
            system_audio_files: Vec::new(),
        };
        let mut failures = Vec::new();
        let unusable_files = BTreeSet::new();

        sync_finalized_microphone_output_files(&mut output_files, &unusable_files, &mut failures);
        sync_finalized_system_audio_output_files(&mut output_files, &unusable_files, &mut failures);

        let mut legacy_output_files = CaptureOutputFiles {
            screen_file: None,
            screen_files: Vec::new(),
            microphone_file: Some(microphone_file.clone()),
            microphone_files: Vec::new(),
            system_audio_file: None,
            system_audio_files: Vec::new(),
        };
        sync_finalized_microphone_output_files(
            &mut legacy_output_files,
            &unusable_files,
            &mut failures,
        );

        assert_eq!(output_files.screen_file, Some(screen_file.clone()));
        assert_eq!(output_files.screen_files, vec![screen_file]);
        assert_eq!(
            output_files.microphone_file,
            Some(microphone_kept_file.clone())
        );
        assert_eq!(output_files.microphone_files, vec![microphone_kept_file]);
        assert_eq!(output_files.system_audio_file, None);
        assert!(output_files.system_audio_files.is_empty());

        assert_eq!(
            legacy_output_files.microphone_file,
            Some(microphone_file.clone())
        );
        assert_eq!(legacy_output_files.microphone_files, vec![microphone_file]);
        assert!(failures.is_empty());
        assert!(!Path::new(&empty_microphone_file).exists());
        assert!(!Path::new(&empty_system_audio_file).exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finalize_capture_outputs_preserves_screen_output_while_dropping_audio_conversion_failures() {
        let dir = TestDir::new("preserve-screen-drop-audio-conversion-failures");
        let screen_file = dir.path.join("screen.mov");
        fs::write(&screen_file, b"screen").expect("screen artifact should exist");
        let screen_file = screen_file.to_string_lossy().to_string();
        let microphone_file = dir
            .path
            .join("microphone.m4a")
            .to_string_lossy()
            .to_string();
        let system_audio_file = dir
            .path
            .join("system-audio.m4a")
            .to_string_lossy()
            .to_string();
        let mut output_files = CaptureOutputFiles {
            screen_file: Some(screen_file.clone()),
            screen_files: vec![screen_file.clone()],
            microphone_file: Some(microphone_file),
            microphone_files: Vec::new(),
            system_audio_file: Some(system_audio_file),
            system_audio_files: Vec::new(),
        };

        finalize_capture_outputs(
            Some(&mut output_files),
            Some(&screen_file),
            None,
            None,
            None,
        )
        .expect("audio-only conversion failures should not block valid screen output");

        assert_eq!(output_files.screen_file, Some(screen_file.clone()));
        assert_eq!(output_files.screen_files, vec![screen_file]);
        assert_eq!(output_files.microphone_file, None);
        assert!(output_files.microphone_files.is_empty());
        assert_eq!(output_files.system_audio_file, None);
        assert!(output_files.system_audio_files.is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finalize_capture_outputs_preserves_screen_output_while_dropping_missing_audio_outputs() {
        let dir = TestDir::new("preserve-screen-drop-audio");
        let screen_file = dir.path.join("screen.mov");
        fs::write(&screen_file, b"screen").expect("screen artifact should exist");
        let screen_file = screen_file.to_string_lossy().to_string();
        let microphone_file = dir
            .path
            .join("microphone.m4a")
            .to_string_lossy()
            .to_string();
        let system_audio_file = dir
            .path
            .join("system-audio.m4a")
            .to_string_lossy()
            .to_string();
        let mut output_files = CaptureOutputFiles {
            screen_file: Some(screen_file.clone()),
            screen_files: vec![screen_file.clone()],
            microphone_file: Some(microphone_file.clone()),
            microphone_files: vec![microphone_file.clone()],
            system_audio_file: Some(system_audio_file.clone()),
            system_audio_files: vec![system_audio_file.clone()],
        };

        finalize_capture_outputs(
            Some(&mut output_files),
            Some(&screen_file),
            Some(&microphone_file),
            Some(&system_audio_file),
            None,
        )
        .expect("missing audio artifacts should not block finalized screen output");

        assert_eq!(output_files.screen_file, Some(screen_file.clone()));
        assert_eq!(output_files.screen_files, vec![screen_file]);
        assert_eq!(output_files.microphone_file, None);
        assert!(output_files.microphone_files.is_empty());
        assert_eq!(output_files.system_audio_file, None);
        assert!(output_files.system_audio_files.is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn cleanup_unusable_segment_artifacts_removes_audio_files_outside_screen_segment_directory() {
        let dir = TestDir::new("cleanup-separate-audio-dirs");
        let screen_dir = dir.path.join("session-1-segment-0001");
        let audio_dir = dir.path.join("session-1-audio").join("segment-0001");
        fs::create_dir_all(&screen_dir).expect("screen dir should exist");
        fs::create_dir_all(&audio_dir).expect("audio dir should exist");

        let screen_file = screen_dir.join("screen.mov");
        let microphone_file = audio_dir.join("microphone.m4a");
        let system_audio_file = audio_dir.join("system-audio.m4a");
        fs::write(&screen_file, b"screen").expect("screen artifact should exist");
        fs::write(&microphone_file, b"microphone").expect("microphone artifact should exist");
        fs::write(&system_audio_file, b"system-audio").expect("system audio artifact should exist");

        let output_files = CaptureOutputFiles {
            screen_file: Some(screen_file.to_string_lossy().to_string()),
            screen_files: vec![screen_file.to_string_lossy().to_string()],
            microphone_file: Some(microphone_file.to_string_lossy().to_string()),
            microphone_files: vec![microphone_file.to_string_lossy().to_string()],
            system_audio_file: Some(system_audio_file.to_string_lossy().to_string()),
            system_audio_files: vec![system_audio_file.to_string_lossy().to_string()],
        };

        cleanup_unusable_segment_artifacts(
            Some(&output_files),
            output_files.screen_file.as_deref(),
            output_files.microphone_file.as_deref(),
            output_files.system_audio_file.as_deref(),
        );

        assert!(!screen_file.exists());
        assert!(!microphone_file.exists());
        assert!(!system_audio_file.exists());
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn cleanup_unusable_segment_artifacts(
    output_files: Option<&CaptureOutputFiles>,
    recording_file: Option<&str>,
    microphone_recording_file: Option<&str>,
    system_audio_recording_file: Option<&str>,
) {
    let mut files_to_remove: BTreeSet<String> = BTreeSet::new();

    if let Some(output_files) = output_files {
        for file in &output_files.screen_files {
            let _ = files_to_remove.insert(file.clone());
        }
        for file in &output_files.microphone_files {
            let _ = files_to_remove.insert(file.clone());
        }
        for file in &output_files.system_audio_files {
            let _ = files_to_remove.insert(file.clone());
        }

        if let Some(file) = output_files.screen_file.as_ref() {
            let _ = files_to_remove.insert(file.clone());
        }
        if let Some(file) = output_files.microphone_file.as_ref() {
            let _ = files_to_remove.insert(file.clone());
        }
        if let Some(file) = output_files.system_audio_file.as_ref() {
            let _ = files_to_remove.insert(file.clone());
        }
    }

    if let Some(file) = recording_file {
        let _ = files_to_remove.insert(file.to_string());
    }
    if let Some(file) = microphone_recording_file {
        let _ = files_to_remove.insert(file.to_string());
    }
    if let Some(file) = system_audio_recording_file {
        let _ = files_to_remove.insert(file.to_string());
    }

    for file in files_to_remove {
        if let Err(error) = std::fs::remove_file(&file) {
            if error.kind() != std::io::ErrorKind::NotFound {
                crate::native_capture_debug_log::log(format!(
                    "failed removing unusable segment artifact {file}: {error}"
                ));
            }
        }
    }
}
