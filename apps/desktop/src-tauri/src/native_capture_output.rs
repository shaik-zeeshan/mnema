use capture_types::{CaptureErrorResponse, CaptureOutputFiles, CaptureSources};
#[cfg(target_os = "macos")]
use std::collections::BTreeSet;

pub(crate) fn set_current_microphone_output_file(
    output_files: &mut CaptureOutputFiles,
    file: String,
) {
    output_files.microphone_file = Some(file.clone());
    output_files.microphone_files.push(file);
}

pub(crate) fn set_current_screen_output_file(output_files: &mut CaptureOutputFiles, file: String) {
    output_files.screen_file = Some(file.clone());
    output_files.screen_files.push(file);
}

pub(crate) fn set_current_system_audio_output_file(
    output_files: &mut CaptureOutputFiles,
    file: String,
) {
    output_files.system_audio_file = Some(file.clone());
    output_files.system_audio_files.push(file);
}

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
pub(crate) fn finalize_capture_outputs(
    output_files: Option<&CaptureOutputFiles>,
    recording_file: Option<&str>,
    microphone_recording_file: Option<&str>,
    system_audio_recording_file: Option<&str>,
    requested_sources: Option<&CaptureSources>,
) -> Result<(), CaptureErrorResponse> {
    let Some(output_files) = output_files else {
        return Ok(());
    };

    let mut failures: Vec<String> = Vec::new();
    let microphone_files = microphone_output_files(output_files);

    if output_files.microphone_file.is_some() && output_files.microphone_files.is_empty() {
        let microphone_file = output_files
            .microphone_file
            .as_deref()
            .expect("checked microphone_file is present");
        let source_recording = microphone_recording_file.or(recording_file);

        if let Some(source_recording) = source_recording {
            if source_recording != microphone_file {
                if let Err(error) = capture_writers::convert_recording_audio_to_m4a(
                    source_recording,
                    microphone_file,
                ) {
                    failures.push(format!(
                        "microphone output conversion failed: {}",
                        error.message
                    ));
                }
            }
        } else {
            failures
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
                    failures.push(format!(
                        "system audio output conversion failed: {}",
                        error.message
                    ));
                }
            }
        } else {
            failures.push(
                "system audio output conversion failed: missing source recording".to_string(),
            );
        }
    }

    if requested_sources.is_some_and(|sources| sources.system_audio) {
        if let Some(recording_file) = recording_file {
            if let Err(error) = capture_screen::strip_audio_from_recording_file(recording_file) {
                failures.push(format!(
                    "screen output video-only conversion failed: {}",
                    error.message
                ));
            }
        }
    }

    if let Some(microphone_recording_file) = microphone_recording_file {
        if !microphone_files.contains(&microphone_recording_file) {
            maybe_remove_intermediate_file(microphone_recording_file, "microphone", &mut failures);
        }
    }

    if let Some(system_audio_recording_file) = system_audio_recording_file {
        if output_files.system_audio_file.as_deref() != Some(system_audio_recording_file) {
            maybe_remove_intermediate_file(
                system_audio_recording_file,
                "system audio",
                &mut failures,
            );
        }
    }

    capture_writers::aggregate_output_processing_failures(failures)
}

#[cfg(target_os = "macos")]
pub(crate) fn append_committed_segment_output_files(
    committed: &mut CaptureOutputFiles,
    segment: &CaptureOutputFiles,
) {
    if let Some(file) = segment.screen_file.as_ref() {
        set_current_screen_output_file(committed, file.clone());
    }
    if let Some(file) = segment.microphone_file.as_ref() {
        set_current_microphone_output_file(committed, file.clone());
    }
    if let Some(file) = segment.system_audio_file.as_ref() {
        set_current_system_audio_output_file(committed, file.clone());
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
                eprintln!("failed removing unusable segment artifact {file}: {error}");
            }
        }
    }
}
