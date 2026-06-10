use capture_types::{CaptureErrorResponse, CaptureOutputFiles, CaptureSources};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::collections::BTreeSet;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::fs::File;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::io::{Read, Seek, SeekFrom};
#[cfg(any(target_os = "macos", target_os = "windows"))]
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

const MISSING_REQUESTED_SCREEN_OUTPUT_FAILURE_PREFIX: &str =
    "screen output missing: expected screen recording file";
#[cfg(any(test, target_os = "macos"))]
const MISSING_REQUESTED_SCREEN_OUTPUT_AT_PATH_PREFIX: &str =
    "screen output missing: expected screen recording file at ";

#[cfg(any(target_os = "macos", target_os = "windows"))]
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

/// Byte-level openability probe for a finalized visible screen segment.
///
/// Both the macOS `.mov` (QuickTime) and Windows `.mp4` (Media Foundation)
/// containers are ISO-BMFF and carry a `moov` atom once finalized; an
/// in-flight, truncated, or otherwise unopenable file is missing it. The check
/// is purely on bytes, so it validates `.mp4` exactly as it validates `.mov` —
/// the only per-platform difference is the extension used to *find* the file,
/// resolved by `capture_runtime::screen_segment_extension()` upstream.
#[cfg(any(target_os = "macos", target_os = "windows"))]
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

#[cfg(any(target_os = "macos", target_os = "windows"))]
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

#[cfg(any(target_os = "macos", target_os = "windows"))]
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

/// Windows definition of an openable `.m4a`: the Media Foundation Source Reader
/// opens it and reports `MF_PD_DURATION > 0`. This is the only new validator
/// leaf on Windows; the structural finalization helpers above are shared.
#[cfg(target_os = "windows")]
fn audio_file_has_positive_duration(path: &str) -> bool {
    capture_writers::windows_audio_file_has_positive_duration(path)
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

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn maybe_remove_unusable_audio_output_file_with_duration_validator(
    file: &str,
    label: &str,
    unusable_files: &BTreeSet<String>,
    failures: &mut Vec<String>,
    has_positive_duration: impl Fn(&str) -> bool,
) {
    if is_usable_audio_output_file_with_duration_validator(
        file,
        unusable_files,
        has_positive_duration,
    ) {
        return;
    }

    let cleanup_label = format!("unusable {label}");
    maybe_remove_intermediate_file(file, cleanup_label.as_str(), failures);
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
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
                maybe_remove_unusable_audio_output_file_with_duration_validator(
                    path,
                    label,
                    unusable_files,
                    failures,
                    has_positive_duration,
                );
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
                maybe_remove_unusable_audio_output_file_with_duration_validator(
                    path,
                    label,
                    unusable_files,
                    failures,
                    has_positive_duration,
                );
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

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn sync_finalized_microphone_output_files_with_duration_validator(
    output_files: &mut CaptureOutputFiles,
    unusable_files: &BTreeSet<String>,
    failures: &mut Vec<String>,
    has_positive_duration: impl Fn(&str) -> bool + Copy,
) {
    sync_finalized_audio_output_file_with_duration_validator(
        &mut output_files.microphone_file,
        &mut output_files.microphone_files,
        "microphone",
        unusable_files,
        failures,
        has_positive_duration,
    );

    if output_files.microphone_file.is_none() {
        clear_current_microphone_output_file(output_files);
    }
}

#[cfg(target_os = "macos")]
fn sync_finalized_microphone_output_files(
    output_files: &mut CaptureOutputFiles,
    unusable_files: &BTreeSet<String>,
    failures: &mut Vec<String>,
) {
    sync_finalized_microphone_output_files_with_duration_validator(
        output_files,
        unusable_files,
        failures,
        audio_file_has_positive_duration,
    );
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn sync_finalized_system_audio_output_files_with_duration_validator(
    output_files: &mut CaptureOutputFiles,
    unusable_files: &BTreeSet<String>,
    failures: &mut Vec<String>,
    has_positive_duration: impl Fn(&str) -> bool + Copy,
) {
    sync_finalized_audio_output_file_with_duration_validator(
        &mut output_files.system_audio_file,
        &mut output_files.system_audio_files,
        "system audio",
        unusable_files,
        failures,
        has_positive_duration,
    );

    if output_files.system_audio_file.is_none() {
        clear_current_system_audio_output_file(output_files);
    }
}

#[cfg(target_os = "macos")]
fn sync_finalized_system_audio_output_files(
    output_files: &mut CaptureOutputFiles,
    unusable_files: &BTreeSet<String>,
    failures: &mut Vec<String>,
) {
    sync_finalized_system_audio_output_files_with_duration_validator(
        output_files,
        unusable_files,
        failures,
        audio_file_has_positive_duration,
    );
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn audio_output_files_are_empty(output_files: &CaptureOutputFiles) -> bool {
    output_files.microphone_file.is_none()
        && output_files.microphone_files.is_empty()
        && output_files.system_audio_file.is_none()
        && output_files.system_audio_files.is_empty()
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn missing_requested_screen_output_failure(recording_file: Option<&str>) -> String {
    let path_detail = recording_file
        .map(|path| format!(" at {path}"))
        .unwrap_or_default();
    format!("{MISSING_REQUESTED_SCREEN_OUTPUT_FAILURE_PREFIX}{path_detail}")
}

#[cfg(any(test, target_os = "macos"))]
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

    if has_screen_output && requested_sources.is_some_and(|sources| sources.system_audio) {
        if let Some(recording_file) = recording_file {
            if let Err(error) = capture_screen::strip_audio_from_recording_file(recording_file) {
                audio_failures.push(format!(
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

#[cfg(target_os = "windows")]
fn finalize_windows_audio_outputs_with_duration_validator(
    output_files: &mut CaptureOutputFiles,
    has_positive_duration: impl Fn(&str) -> bool + Copy,
) -> Result<(), CaptureErrorResponse> {
    let mut failures: Vec<String> = Vec::new();
    let unusable_audio_files: BTreeSet<String> = BTreeSet::new();

    sync_finalized_microphone_output_files_with_duration_validator(
        output_files,
        &unusable_audio_files,
        &mut failures,
        has_positive_duration,
    );
    sync_finalized_system_audio_output_files_with_duration_validator(
        output_files,
        &unusable_audio_files,
        &mut failures,
        has_positive_duration,
    );

    capture_writers::aggregate_output_processing_failures(failures)
}

/// Windows capture-output finalization.
///
/// Windows writes the final `.m4a` directly via Media Foundation, so there is no
/// source→`.m4a` conversion or video-only strip step (those are macOS-only). The
/// finalization work is (1) validating the produced audio outputs through the
/// shared injectable validator seam — `MF_PD_DURATION > 0` via the MF Source
/// Reader probe — and dropping any unusable files, and (2) validating the
/// finalized screen `.mp4` is openable.
///
/// The screen-output validation mirrors macOS: a requested screen segment whose
/// `.mp4` is missing or unopenable (no `moov` atom — e.g. a sink writer that
/// crashed mid-segment) is rejected exactly as an unopenable macOS `.mov` is.
/// `.mp4` and `.mov` are both ISO-BMFF, so the byte-level `moov` probe in
/// [`screen_output_appears_openable`] is container-agnostic; the only thing that
/// made this Windows-blind before was that the screen path was never validated
/// here at all. When valid audio was captured, the audio-only segment is
/// preserved instead of discarding the whole segment, matching macOS.
#[cfg(target_os = "windows")]
pub(crate) fn finalize_capture_outputs(
    output_files: Option<&mut CaptureOutputFiles>,
    recording_file: Option<&str>,
    microphone_recording_file: Option<&str>,
    system_audio_recording_file: Option<&str>,
    requested_sources: Option<&CaptureSources>,
) -> Result<(), CaptureErrorResponse> {
    let _ = (microphone_recording_file, system_audio_recording_file);
    let Some(output_files) = output_files else {
        return Ok(());
    };

    let mut failures: Vec<String> = Vec::new();
    let has_screen_output = sync_finalized_screen_output_file(output_files, recording_file);
    if requested_sources.is_some_and(|sources| sources.screen) && !has_screen_output {
        failures.push(missing_requested_screen_output_failure(recording_file));
    }

    // Validate/drop unusable audio outputs. `failures` only carries the
    // missing-screen failure (audio problems route through `audio_failures`),
    // so `failures.len() == 1` with empty `audio_failures` means the screen
    // output is the lone problem — the same invariant the macOS path relies on.
    let audio_failures = match finalize_windows_audio_outputs_with_duration_validator(
        output_files,
        audio_file_has_positive_duration,
    ) {
        Ok(()) => Vec::new(),
        Err(error) => vec![error.message],
    };

    // When the only failure is a missing/unopenable screen segment but usable
    // audio was captured, drop the unusable screen recording and commit the
    // audio-only segment rather than discarding everything. Mirrors the macOS
    // `preserve_audio_despite_missing_screen` path.
    let preserve_audio_despite_missing_screen = requested_sources
        .is_some_and(|sources| sources.screen && (sources.microphone || sources.system_audio))
        && !has_screen_output
        && audio_failures.is_empty()
        && failures.len() == 1
        && !audio_output_files_are_empty(output_files);

    if preserve_audio_despite_missing_screen {
        if let Some(recording_file) = recording_file {
            let mut discarded_failures = Vec::new();
            maybe_remove_intermediate_file(recording_file, "screen", &mut discarded_failures);
        }
        return capture_writers::aggregate_output_processing_failures(Vec::new());
    }

    failures.extend(audio_failures);
    capture_writers::aggregate_output_processing_failures(failures)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn append_output_file(current_file: &mut Option<String>, files: &mut Vec<String>, file: &str) {
    let file = file.to_string();
    *current_file = Some(file.clone());
    if !files.iter().any(|existing| existing == &file) {
        files.push(file);
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
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

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    struct TestDir {
        path: PathBuf,
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
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
    fn write_openable_screen_file(path: &Path) {
        fs::write(path, b"\0\0\0\x14ftypqt  \0\0\0\0qt  \0\0\0\x10moovtrak")
            .expect("screen artifact should exist");
    }

    // Produces a real, positive-duration AAC/m4a file so that the AVFoundation
    // duration validation inside `finalize_capture_outputs` accepts it as usable
    // captured audio. The AAC fourcc is written directly to avoid depending on
    // cidre's optional `cat` module from this crate.
    #[cfg(target_os = "macos")]
    fn write_valid_m4a_audio_file(path: &Path) {
        use cidre::{av, ns};

        let _autorelease_pool = cidre::objc::autorelease_pool::AutoreleasePoolPage::push();
        let path_str = path.to_string_lossy().to_string();
        let url = ns::Url::with_fs_path_str(&path_str, false);
        let sample_rate = 48_000.0_f64;
        let format_id = ns::Number::with_u32(u32::from_be_bytes(*b"aac "));
        let sample_rate_value = ns::Number::with_f64(sample_rate);
        let channel_count_value = ns::Number::with_i64(1);
        let settings: cidre::arc::R<ns::Dictionary<ns::String, ns::Id>> =
            ns::Dictionary::with_keys_values(
                &[
                    av::audio::all_formats_keys::id(),
                    av::audio::all_formats_keys::sample_rate(),
                    av::audio::all_formats_keys::number_of_channels(),
                ],
                &[
                    format_id.as_id_ref(),
                    sample_rate_value.as_id_ref(),
                    channel_count_value.as_id_ref(),
                ],
            );
        let mut file = av::AudioFile::open_write_common_format(
            &url,
            &settings,
            av::AudioCommonFormat::PcmF32,
            false,
        )
        .expect("writable test audio file should open");
        let processing_format = file.processing_format();
        let frames: u32 = 24_000; // 0.5s @ 48kHz — comfortably positive duration.
        let mut buffer = av::AudioPcmBuf::with_format(&processing_format, frames)
            .expect("test audio buffer should allocate");
        buffer
            .set_frame_len(frames)
            .expect("frame length should set");
        if let Some(samples) = buffer.data_f32_mut_at(0) {
            for (index, sample) in samples.iter_mut().enumerate() {
                *sample = ((index as f32) * 0.05).sin() * 0.1;
            }
        }
        file.write(&buffer).expect("test audio should write");
        file.close();
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_finalization_validates_system_audio_files_with_duration_seam() {
        let dir = TestDir::new("windows-system-audio-finalization");
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
        let system_audio_rotated_file = dir
            .path
            .join("system-audio-rotated.m4a")
            .to_string_lossy()
            .to_string();
        fs::write(&microphone_file, b"microphone").expect("microphone artifact should exist");
        fs::write(&system_audio_file, b"system-audio").expect("system audio artifact should exist");
        fs::write(&system_audio_rotated_file, b"system-audio-rotated")
            .expect("rotated system audio artifact should exist");
        let mut output_files = CaptureOutputFiles {
            screen_file: None,
            screen_files: Vec::new(),
            microphone_file: Some(microphone_file.clone()),
            microphone_files: vec![microphone_file.clone()],
            system_audio_file: Some(system_audio_file.clone()),
            system_audio_files: vec![system_audio_file.clone(), system_audio_rotated_file.clone()],
        };

        finalize_windows_audio_outputs_with_duration_validator(&mut output_files, |path| {
            path == microphone_file || path == system_audio_rotated_file
        })
        .expect("valid rotated system audio output should be retained");

        assert_eq!(output_files.microphone_file, Some(microphone_file.clone()));
        assert_eq!(output_files.microphone_files, vec![microphone_file]);
        assert_eq!(
            output_files.system_audio_file,
            Some(system_audio_rotated_file.clone())
        );
        assert_eq!(
            output_files.system_audio_files,
            vec![system_audio_rotated_file]
        );
        assert!(!Path::new(&system_audio_file).exists());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn committed_output_bookkeeping_appends_system_audio_files() {
        let mut committed = CaptureOutputFiles {
            screen_file: None,
            screen_files: Vec::new(),
            microphone_file: None,
            microphone_files: Vec::new(),
            system_audio_file: Some("system-audio-old.m4a".to_string()),
            system_audio_files: vec!["system-audio-old.m4a".to_string()],
        };
        let segment = CaptureOutputFiles {
            screen_file: None,
            screen_files: Vec::new(),
            microphone_file: None,
            microphone_files: Vec::new(),
            system_audio_file: Some("system-audio-new.m4a".to_string()),
            system_audio_files: vec![
                "system-audio-rotated-1.m4a".to_string(),
                "system-audio-rotated-2.m4a".to_string(),
            ],
        };

        append_committed_segment_output_files(&mut committed, &segment);

        assert_eq!(
            committed.system_audio_file,
            Some("system-audio-rotated-2.m4a".to_string())
        );
        assert_eq!(
            committed.system_audio_files,
            vec![
                "system-audio-old.m4a".to_string(),
                "system-audio-rotated-1.m4a".to_string(),
                "system-audio-rotated-2.m4a".to_string(),
            ]
        );
    }

    // A finalized `.mp4` carries a `moov` atom, exactly like a finalized `.mov`.
    #[cfg(target_os = "windows")]
    fn write_openable_screen_mp4(path: &Path) {
        fs::write(path, b"\0\0\0\x14ftypisom\0\0\0\0isom\0\0\0\x10moovtrak")
            .expect("screen artifact should exist");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_finalization_commits_openable_mp4_screen_segment() {
        let dir = TestDir::new("windows-openable-mp4");
        let recording_file = dir.path.join("screen.mp4");
        write_openable_screen_mp4(&recording_file);
        let recording_file = recording_file.to_string_lossy().to_string();
        let requested_sources = CaptureSources {
            screen: true,
            microphone: false,
            system_audio: false,
        };
        let mut output_files = CaptureOutputFiles {
            screen_file: Some(recording_file.clone()),
            screen_files: vec![recording_file.clone()],
            microphone_file: None,
            microphone_files: Vec::new(),
            system_audio_file: None,
            system_audio_files: Vec::new(),
        };

        finalize_capture_outputs(
            Some(&mut output_files),
            Some(&recording_file),
            None,
            None,
            Some(&requested_sources),
        )
        .expect("openable mp4 screen segment should finalize");

        assert_eq!(output_files.screen_file, Some(recording_file.clone()));
        assert_eq!(output_files.screen_files, vec![recording_file.clone()]);
        assert!(Path::new(&recording_file).exists());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_finalization_rejects_existing_but_unopenable_mp4_screen_segment() {
        let dir = TestDir::new("windows-unopenable-mp4");
        let recording_file = dir.path.join("screen.mp4");
        // Present on disk but missing the `moov` atom — e.g. the MF sink writer
        // crashed mid-segment. Must be rejected exactly like an unopenable `.mov`.
        fs::write(&recording_file, b"\0\0\0\x14ftypisom\0\0\0\0isom\0\0\0\x10mdatjunk")
            .expect("unopenable screen artifact should exist");
        let recording_file = recording_file.to_string_lossy().to_string();
        let requested_sources = CaptureSources {
            screen: true,
            microphone: false,
            system_audio: false,
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
        .expect_err("unopenable mp4 screen segment should be rejected");

        assert_eq!(error.code, "capture_output_processing_failed");
        assert!(error
            .message
            .contains("screen output missing: expected screen recording file"));
        assert_eq!(output_files.screen_file, None);
        assert!(output_files.screen_files.is_empty());
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
    fn finalize_capture_outputs_preserves_audio_when_requested_screen_output_missing() {
        let dir = TestDir::new("preserve-audio-missing-screen");
        // A screen .mov that exists on disk but is not openable (no `moov`),
        // exactly the failure mode that was discarding captured audio.
        let recording_file = dir.path.join("screen.mov");
        fs::write(
            &recording_file,
            b"\0\0\0\x14ftypqt  \0\0\0\0qt  \0\0\0\x10mdatjunk",
        )
        .expect("broken screen artifact should exist");
        let recording_file = recording_file.to_string_lossy().to_string();

        let microphone_file = dir.path.join("microphone.m4a");
        write_valid_m4a_audio_file(&microphone_file);
        let microphone_file = microphone_file.to_string_lossy().to_string();

        let requested_sources = CaptureSources {
            screen: true,
            microphone: true,
            system_audio: false,
        };
        let mut output_files = CaptureOutputFiles {
            screen_file: Some(recording_file.clone()),
            screen_files: vec![recording_file.clone()],
            microphone_file: Some(microphone_file.clone()),
            microphone_files: vec![microphone_file.clone()],
            system_audio_file: None,
            system_audio_files: Vec::new(),
        };

        finalize_capture_outputs(
            Some(&mut output_files),
            Some(&recording_file),
            None,
            None,
            Some(&requested_sources),
        )
        .expect("captured audio must survive a missing screen segment");

        // The unusable screen output is dropped from bookkeeping and disk...
        assert_eq!(output_files.screen_file, None);
        assert!(output_files.screen_files.is_empty());
        assert!(!Path::new(&recording_file).exists());
        // ...but the captured microphone audio is preserved for commit.
        assert_eq!(output_files.microphone_file, Some(microphone_file.clone()));
        assert_eq!(output_files.microphone_files, vec![microphone_file.clone()]);
        assert!(Path::new(&microphone_file).exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finalize_capture_outputs_rejects_invalid_existing_screen_output() {
        let dir = TestDir::new("invalid-screen-output");
        let recording_file = dir.path.join("screen.mov");
        fs::write(
            &recording_file,
            b"\0\0\0\x14ftypqt  \0\0\0\0qt  \0\0\0\x10mdatjunk",
        )
        .expect("invalid screen artifact should exist");
        let recording_file = recording_file.to_string_lossy().to_string();
        let requested_sources = CaptureSources {
            screen: true,
            microphone: false,
            system_audio: false,
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
        .expect_err("invalid existing screen recording must fail finalization");

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

        sync_finalized_audio_output_file_with_duration_validator(
            &mut output_files.microphone_file,
            &mut output_files.microphone_files,
            "microphone",
            &unusable_files,
            &mut failures,
            |path| path == microphone_kept_file,
        );
        sync_finalized_audio_output_file_with_duration_validator(
            &mut output_files.system_audio_file,
            &mut output_files.system_audio_files,
            "system audio",
            &unusable_files,
            &mut failures,
            |_| false,
        );

        let mut legacy_output_files = CaptureOutputFiles {
            screen_file: None,
            screen_files: Vec::new(),
            microphone_file: Some(microphone_file.clone()),
            microphone_files: Vec::new(),
            system_audio_file: None,
            system_audio_files: Vec::new(),
        };
        sync_finalized_audio_output_file_with_duration_validator(
            &mut legacy_output_files.microphone_file,
            &mut legacy_output_files.microphone_files,
            "microphone",
            &unusable_files,
            &mut failures,
            |path| path == microphone_file,
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
    fn zero_duration_non_empty_audio_output_is_unusable() {
        let dir = TestDir::new("zero-duration-audio-output");
        let microphone_file = dir
            .path
            .join("microphone-zero-duration.m4a")
            .to_string_lossy()
            .to_string();
        fs::write(&microphone_file, b"non-empty").expect("audio artifact should exist");
        let mut output_files = CaptureOutputFiles {
            screen_file: None,
            screen_files: Vec::new(),
            microphone_file: Some(microphone_file.clone()),
            microphone_files: vec![microphone_file.clone()],
            system_audio_file: None,
            system_audio_files: Vec::new(),
        };
        let mut failures = Vec::new();
        let unusable_files = BTreeSet::new();

        sync_finalized_audio_output_file_with_duration_validator(
            &mut output_files.microphone_file,
            &mut output_files.microphone_files,
            "microphone",
            &unusable_files,
            &mut failures,
            |_| false,
        );

        assert_eq!(output_files.microphone_file, None);
        assert!(output_files.microphone_files.is_empty());
        assert!(failures.is_empty());
        assert!(!Path::new(&microphone_file).exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn positive_duration_short_system_audio_output_is_usable() {
        let dir = TestDir::new("short-system-audio-output");
        let system_audio_file = dir
            .path
            .join("system-audio-short.m4a")
            .to_string_lossy()
            .to_string();
        fs::write(&system_audio_file, b"short-active").expect("audio artifact should exist");
        let mut output_files = CaptureOutputFiles {
            screen_file: None,
            screen_files: Vec::new(),
            microphone_file: None,
            microphone_files: Vec::new(),
            system_audio_file: Some(system_audio_file.clone()),
            system_audio_files: vec![system_audio_file.clone()],
        };
        let mut failures = Vec::new();
        let unusable_files = BTreeSet::new();

        sync_finalized_audio_output_file_with_duration_validator(
            &mut output_files.system_audio_file,
            &mut output_files.system_audio_files,
            "system audio",
            &unusable_files,
            &mut failures,
            |_| true,
        );

        assert_eq!(
            output_files.system_audio_file,
            Some(system_audio_file.clone())
        );
        assert_eq!(output_files.system_audio_files, vec![system_audio_file]);
        assert!(failures.is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn positive_duration_ignored_microphone_output_is_removed_from_payload() {
        let dir = TestDir::new("ignored-microphone-output");
        let microphone_file = dir
            .path
            .join("microphone-silent-startup.m4a")
            .to_string_lossy()
            .to_string();
        fs::write(&microphone_file, b"positive-duration-silence")
            .expect("microphone artifact should exist");
        let mut output_files = CaptureOutputFiles {
            screen_file: None,
            screen_files: Vec::new(),
            microphone_file: Some(microphone_file.clone()),
            microphone_files: vec![microphone_file.clone()],
            system_audio_file: None,
            system_audio_files: Vec::new(),
        };
        let mut failures = Vec::new();
        let unusable_files = BTreeSet::from([microphone_file.clone()]);

        sync_finalized_audio_output_file_with_duration_validator(
            &mut output_files.microphone_file,
            &mut output_files.microphone_files,
            "microphone",
            &unusable_files,
            &mut failures,
            |_| true,
        );

        assert_eq!(output_files.microphone_file, None);
        assert!(output_files.microphone_files.is_empty());
        assert!(failures.is_empty());
        assert!(!Path::new(&microphone_file).exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finalize_capture_outputs_treats_deleted_audio_only_outputs_as_nonfatal() {
        let dir = TestDir::new("deleted-audio-only-output");
        let microphone_file = dir
            .path
            .join("microphone-empty.m4a")
            .to_string_lossy()
            .to_string();
        let system_audio_file = dir
            .path
            .join("system-audio-empty.m4a")
            .to_string_lossy()
            .to_string();
        fs::File::create(&microphone_file).expect("empty microphone placeholder should exist");
        fs::File::create(&system_audio_file).expect("empty system-audio placeholder should exist");
        let mut output_files = CaptureOutputFiles {
            screen_file: None,
            screen_files: Vec::new(),
            microphone_file: Some(microphone_file.clone()),
            microphone_files: vec![microphone_file.clone()],
            system_audio_file: Some(system_audio_file.clone()),
            system_audio_files: vec![system_audio_file.clone()],
        };

        finalize_capture_outputs(
            Some(&mut output_files),
            None,
            Some(&microphone_file),
            Some(&system_audio_file),
            Some(&CaptureSources {
                screen: false,
                microphone: true,
                system_audio: true,
            }),
        )
        .expect("empty audio-only inactivity outputs should be ignored");

        assert_eq!(output_files.microphone_file, None);
        assert!(output_files.microphone_files.is_empty());
        assert_eq!(output_files.system_audio_file, None);
        assert!(output_files.system_audio_files.is_empty());
        assert!(!Path::new(&microphone_file).exists());
        assert!(!Path::new(&system_audio_file).exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finalize_capture_outputs_treats_ignored_system_audio_only_output_as_nonfatal() {
        let dir = TestDir::new("ignored-system-audio-only-output");
        let system_audio_file = dir
            .path
            .join("system-audio-empty.m4a")
            .to_string_lossy()
            .to_string();
        fs::File::create(&system_audio_file).expect("empty system-audio placeholder should exist");
        let mut output_files = CaptureOutputFiles {
            screen_file: None,
            screen_files: Vec::new(),
            microphone_file: None,
            microphone_files: Vec::new(),
            system_audio_file: Some(system_audio_file.clone()),
            system_audio_files: vec![system_audio_file.clone()],
        };

        finalize_capture_outputs(
            Some(&mut output_files),
            None,
            None,
            None,
            Some(&CaptureSources {
                screen: false,
                microphone: false,
                system_audio: true,
            }),
        )
        .expect("ignored audio-only inactivity output should be nonfatal");

        assert_eq!(output_files.system_audio_file, None);
        assert!(output_files.system_audio_files.is_empty());
        assert!(!Path::new(&system_audio_file).exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finalize_capture_outputs_preserves_screen_output_while_dropping_audio_conversion_failures() {
        let dir = TestDir::new("preserve-screen-drop-audio-conversion-failures");
        let screen_file = dir.path.join("screen.mov");
        write_openable_screen_file(&screen_file);
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
    fn finalize_capture_outputs_rejects_invalid_screen_output_when_system_audio_is_requested() {
        let dir = TestDir::new("preserve-screen-strip-audio-failure");
        let screen_file = dir.path.join("screen.mov");
        fs::write(&screen_file, b"not a real mov").expect("screen artifact should exist");
        let screen_file = screen_file.to_string_lossy().to_string();
        let requested_sources = CaptureSources {
            screen: true,
            microphone: false,
            system_audio: true,
        };
        let mut output_files = CaptureOutputFiles {
            screen_file: Some(screen_file.clone()),
            screen_files: vec![screen_file.clone()],
            microphone_file: None,
            microphone_files: Vec::new(),
            system_audio_file: None,
            system_audio_files: Vec::new(),
        };

        let error = finalize_capture_outputs(
            Some(&mut output_files),
            Some(&screen_file),
            None,
            None,
            Some(&requested_sources),
        )
        .expect_err("invalid screen output should not be committed");

        assert_eq!(error.code, "capture_output_processing_failed");
        assert!(error
            .message
            .contains("screen output missing: expected screen recording file"));
        assert_eq!(output_files.screen_file, None);
        assert!(output_files.screen_files.is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finalize_capture_outputs_preserves_screen_output_while_dropping_missing_audio_outputs() {
        let dir = TestDir::new("preserve-screen-drop-audio");
        let screen_file = dir.path.join("screen.mov");
        write_openable_screen_file(&screen_file);
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
                super::debug_log::log(format!(
                    "failed removing unusable segment artifact {file}: {error}"
                ));
            }
        }
    }
}
