use capture_types::{CaptureErrorResponse, CaptureOutputFiles, CaptureSources};
#[cfg(target_os = "macos")]
use std::collections::BTreeSet;
#[cfg(target_os = "macos")]
use std::fs::File;
#[cfg(target_os = "macos")]
use std::io::{Read, Seek, SeekFrom};
#[cfg(target_os = "macos")]
use std::path::Path;

pub fn set_current_microphone_output_file(output_files: &mut CaptureOutputFiles, file: String) {
    output_files.microphone_file = Some(file.clone());
    output_files.microphone_files.push(file);
}

pub fn clear_current_microphone_output_file(output_files: &mut CaptureOutputFiles) {
    output_files.microphone_file = None;
    output_files.microphone_files.clear();
}

pub fn set_current_screen_output_file(output_files: &mut CaptureOutputFiles, file: String) {
    output_files.screen_file = Some(file.clone());
    output_files.screen_files.push(file);
}

pub fn clear_current_screen_output_file(output_files: &mut CaptureOutputFiles) {
    output_files.screen_file = None;
    output_files.screen_files.clear();
}

pub fn set_current_system_audio_output_file(output_files: &mut CaptureOutputFiles, file: String) {
    output_files.system_audio_file = Some(file.clone());
    output_files.system_audio_files.push(file);
}

pub fn clear_current_system_audio_output_file(output_files: &mut CaptureOutputFiles) {
    output_files.system_audio_file = None;
    output_files.system_audio_files.clear();
}

/// Drops only the file the tap is still writing, keeping earlier-generation
/// files a mid-segment rebuild already finalized so they commit with the
/// segment instead of being orphaned on disk.
pub fn strip_live_system_audio_output_file(output_files: &mut CaptureOutputFiles, live_file: &str) {
    output_files
        .system_audio_files
        .retain(|path| path != live_file);
    if output_files.system_audio_file.as_deref() == Some(live_file) {
        output_files.system_audio_file = None;
    }
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
fn screen_output_appears_openable(path: &str) -> bool {
    const SEARCH_WINDOW_BYTES: u64 = 256 * 1024;

    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let Ok(metadata) = file.metadata() else {
        return false;
    };
    let file_len = metadata.len();
    if file_len < 8 {
        return false;
    }

    let prefix_len = file_len.min(SEARCH_WINDOW_BYTES) as usize;
    let mut prefix = vec![0_u8; prefix_len];
    if file.read_exact(&mut prefix).is_err() {
        return false;
    }
    if prefix.windows(4).any(|window| window == b"moov") {
        return true;
    }

    if file_len <= SEARCH_WINDOW_BYTES {
        return false;
    }

    let suffix_len = file_len.min(SEARCH_WINDOW_BYTES) as usize;
    if file.seek(SeekFrom::End(-(suffix_len as i64))).is_err() {
        return false;
    }
    let mut suffix = vec![0_u8; suffix_len];
    if file.read_exact(&mut suffix).is_err() {
        return false;
    }

    suffix.windows(4).any(|window| window == b"moov")
}

#[cfg(target_os = "macos")]
fn sync_finalized_screen_output_file(
    output_files: &mut CaptureOutputFiles,
    recording_file: Option<&str>,
) -> bool {
    let Some(recording_file) = recording_file
        .filter(|path| Path::new(path).is_file() && screen_output_appears_openable(path))
    else {
        clear_current_screen_output_file(output_files);
        return false;
    };

    clear_current_screen_output_file(output_files);
    set_current_screen_output_file(output_files, recording_file.to_string());
    true
}

#[cfg(target_os = "macos")]
fn is_usable_audio_output_file(path: &str, unusable_files: &BTreeSet<String>) -> bool {
    is_usable_audio_output_file_with_duration_validator(
        path,
        unusable_files,
        audio_file_has_positive_duration,
    )
}

#[cfg(target_os = "macos")]
fn is_usable_audio_output_file_with_duration_validator(
    path: &str,
    unusable_files: &BTreeSet<String>,
    has_positive_duration: impl Fn(&str) -> bool,
) -> bool {
    if unusable_files.contains(path) {
        return false;
    }

    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() || metadata.len() == 0 {
        return false;
    }

    has_positive_duration(path)
}

#[cfg(target_os = "macos")]
fn audio_file_has_positive_duration(path: &str) -> bool {
    use cidre::{av, ns};

    let _autorelease_pool = cidre::objc::autorelease_pool::AutoreleasePoolPage::push();
    let result = {
        let url = ns::Url::with_fs_path_str(path, false);
        av::UrlAsset::with_url(&url, None)
            .map(|asset| asset.duration())
            .is_some_and(|duration| {
                duration.is_numeric() && duration.value > 0 && duration.scale > 0
            })
    };

    result
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
    sync_finalized_audio_output_file_with_duration_validator(
        current_file,
        files,
        label,
        unusable_files,
        failures,
        audio_file_has_positive_duration,
    );
}

#[cfg(target_os = "macos")]
fn sync_finalized_audio_output_file_with_duration_validator(
    current_file: &mut Option<String>,
    files: &mut Vec<String>,
    label: &str,
    unusable_files: &BTreeSet<String>,
    failures: &mut Vec<String>,
    has_positive_duration: impl Fn(&str) -> bool + Copy,
) {
    let mut removed_paths = BTreeSet::new();

    files.retain(|path| {
        if is_usable_audio_output_file_with_duration_validator(
            path,
            unusable_files,
            has_positive_duration,
        ) {
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
        .filter(|path| {
            is_usable_audio_output_file_with_duration_validator(
                path,
                unusable_files,
                has_positive_duration,
            )
        })
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
fn audio_output_files_are_empty(output_files: &CaptureOutputFiles) -> bool {
    output_files.microphone_file.is_none()
        && output_files.microphone_files.is_empty()
        && output_files.system_audio_file.is_none()
        && output_files.system_audio_files.is_empty()
}

#[cfg(target_os = "macos")]
fn missing_requested_screen_output_failure(recording_file: Option<&str>) -> String {
    let path_detail = recording_file
        .map(|path| format!(" at {path}"))
        .unwrap_or_default();
    format!("{MISSING_REQUESTED_SCREEN_OUTPUT_FAILURE_PREFIX}{path_detail}")
}

#[cfg(target_os = "macos")]
pub fn is_missing_requested_screen_output_failure_detail(detail: &str) -> bool {
    detail == MISSING_REQUESTED_SCREEN_OUTPUT_FAILURE_PREFIX
        || detail.starts_with(MISSING_REQUESTED_SCREEN_OUTPUT_AT_PATH_PREFIX)
}

#[cfg(target_os = "macos")]
pub fn finalize_capture_outputs(
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
    let had_audio_artifact_before_sync = output_files
        .microphone_file
        .iter()
        .chain(output_files.microphone_files.iter())
        .chain(output_files.system_audio_file.iter())
        .chain(output_files.system_audio_files.iter())
        .any(|path| Path::new(path).is_file());

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

    // A requested screen segment that produced no usable `.mov` normally fails
    // the whole segment. Callers treat that as a recoverable error and run
    // `cleanup_unusable_segment_artifacts`, which deletes the segment's audio
    // files and skips persistence — so a screen-capture failure silently throws
    // away microphone/system audio that was captured just fine. When valid
    // audio *was* produced and the missing screen output is the only failure,
    // preserve it: drop the unusable screen recording and commit the audio-only
    // segment instead of discarding everything.
    //
    // `failures` only ever carries the missing-screen failure (every other
    // failure routes through `audio_failures`), so `failures.len() == 1` plus an
    // empty `audio_failures` means the screen output is the lone problem.
    let preserve_audio_despite_missing_screen = requested_sources
        .is_some_and(|sources| sources.screen && (sources.microphone || sources.system_audio))
        && !has_screen_output
        && audio_failures.is_empty()
        && failures.len() == 1
        && !audio_output_files_are_empty(output_files);

    if preserve_audio_despite_missing_screen {
        // Best-effort: drop the unusable screen recording so it cannot later
        // masquerade as a preview source. A failure here must not block the
        // audio commit, so its errors are intentionally discarded.
        if let Some(recording_file) = recording_file {
            let mut discarded_failures = Vec::new();
            maybe_remove_intermediate_file(recording_file, "screen", &mut discarded_failures);
        }
        return capture_writers::aggregate_output_processing_failures(Vec::new());
    }

    if !has_screen_output || !failures.is_empty() {
        failures.extend(audio_failures);
    }

    if requested_sources.is_some_and(|sources| {
        !sources.screen
            && (sources.microphone || sources.system_audio)
            && audio_output_files_are_empty(output_files)
            && had_audio_artifact_before_sync
    }) {
        failures.clear();
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
pub fn append_committed_segment_output_files(
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
mod tests;

#[cfg(target_os = "macos")]
pub fn cleanup_unusable_segment_artifacts(
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
                crate::debug_log::log_warn(format!(
                    "failed removing unusable segment artifact {file}: {error}"
                ));
            }
        }
    }
}
