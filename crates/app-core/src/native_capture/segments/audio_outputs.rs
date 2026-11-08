//! Microphone output finalization and the reanchored audio files a segment
//! boundary opens, plus the system-audio family's start.

use super::*;

#[cfg(target_os = "macos")]
pub(crate) fn apply_microphone_output_finalization(
    output_files: Option<&mut CaptureOutputFiles>,
    finalization: &microphone_capture::MicrophoneOutputFinalization,
    source_sessions: Option<&SourceSessions>,
    schedule: Option<&SegmentSchedule>,
    segment_index: u64,
) {
    let Some(output_files) = output_files else {
        return;
    };

    let Some(original_output_file) = finalization.output_file.as_deref() else {
        let current_file = output_files.microphone_file.clone();
        output_files.microphone_files.retain(|file| {
            Some(file.as_str()) != current_file.as_deref()
                && Some(file.as_str()) != finalization.source_file.as_deref()
        });
        output_files.microphone_file = output_files.microphone_files.last().cloned();
        return;
    };
    let mut output_file = original_output_file.to_string();

    if finalization.start_offset_ms != 0 {
        if let (Some(source_session), Some(schedule)) = (
            source_sessions.and_then(|sessions| sessions.microphone.as_ref()),
            schedule,
        ) {
            let base_started_at = audio_segment_started_at_unix_ms_for_file(
                source_session,
                segment_index,
                schedule,
                &output_file,
            );
            let shifted_started_at =
                base_started_at.saturating_add_signed(finalization.start_offset_ms);
            if let Some(shifted) =
                timestamped_microphone_output_file(&output_file, shifted_started_at)
            {
                if shifted != output_file {
                    if let Err(error) = std::fs::rename(&output_file, &shifted) {
                        crate::debug_log::log_warn(format!(
                            "failed to rename trimmed microphone output {} to {}: {}",
                            output_file, shifted, error
                        ));
                    } else {
                        output_file = shifted;
                    }
                }
            }
        }
    }

    output_files.microphone_file = Some(output_file.clone());
    if let Some(existing) = output_files.microphone_files.iter_mut().rfind(|file| {
        file.as_str() == original_output_file
            || Some(file.as_str()) == finalization.source_file.as_deref()
    }) {
        *existing = output_file;
    } else if !output_files
        .microphone_files
        .iter()
        .any(|file| file == &output_file)
    {
        output_files.microphone_files.push(output_file);
    }
}

/// Stops the microphone session outright and books its finalization against the
/// live segment, as a rotation or pause does. A bare `session.stop()` drops it,
/// so the file later committed keeps its uncorrected stamp (trimmed silence, or
/// audio queued before a resume). Leaves `active_microphone_session` to the
/// caller, which decides what a failed stop means; a stop that succeeded leaves
/// nothing recording ([`apply_stopped_microphone_finalization`]).
#[cfg(target_os = "macos")]
pub(crate) fn stop_microphone_session_for_runtime(
    runtime: &mut NativeCaptureRuntime,
) -> Result<(), CaptureErrorResponse> {
    if runtime.active_microphone_session.is_none() {
        return Ok(());
    }
    crate::native_capture::activity::process_pending_microphone_vad_frames(runtime);
    let Some(session) = runtime.active_microphone_session.as_mut() else {
        return Ok(());
    };
    let finalization = session.stop_returning_finalization()?;
    apply_stopped_microphone_finalization(runtime, &finalization);
    Ok(())
}

/// Books a stopped session's finalization: the closed file, under its corrected
/// name, stays in the live segment's outputs for the next commit, and nothing
/// is recording any more. The recording file must not outlive the stop — the
/// finalization may have renamed it, so a suspension would carry the stale name
/// instead of the real file, and a rotation carries it forward until a later
/// commit deletes it as an intermediate.
#[cfg(target_os = "macos")]
pub(crate) fn apply_stopped_microphone_finalization(
    runtime: &mut NativeCaptureRuntime,
    finalization: &microphone_capture::MicrophoneOutputFinalization,
) {
    apply_microphone_output_finalization(
        runtime.current_segment_output_files.as_mut(),
        finalization,
        runtime.source_sessions.as_ref(),
        runtime.segment_schedule.as_ref(),
        runtime.current_segment_index,
    );
    runtime.microphone_recording_file = None;
}

#[cfg(target_os = "macos")]
pub(crate) fn timestamped_microphone_output_file(
    file_path: &str,
    started_at_unix_ms: u64,
) -> Option<String> {
    let path = Path::new(file_path);
    let parent = path.parent();
    let stem = path.file_stem()?.to_str()?;
    let extension = path.extension().and_then(|extension| extension.to_str());
    let base_stem = microphone_output_timestamp_base_stem(stem);
    let file_name = match extension {
        Some(extension) => format!("{base_stem}-{started_at_unix_ms}.{extension}"),
        None => format!("{base_stem}-{started_at_unix_ms}"),
    };
    Some(
        parent
            .map(|parent| parent.join(&file_name))
            .unwrap_or_else(|| PathBuf::from(file_name))
            .to_string_lossy()
            .to_string(),
    )
}

#[cfg(target_os = "macos")]
pub(crate) fn microphone_output_timestamp_base_stem(stem: &str) -> &str {
    let marker = "-segment-";
    let Some(marker_start) = stem.rfind(marker) else {
        return stem;
    };
    let after_marker_start = marker_start + marker.len();
    let after_marker = &stem[after_marker_start..];
    if after_marker.len() < 4 {
        return stem;
    }

    let (segment_index, remainder) = after_marker.split_at(4);
    if !segment_index.bytes().all(|byte| byte.is_ascii_digit()) {
        return stem;
    }
    let Some(timestamp_with_suffix) = remainder.strip_prefix('-') else {
        return stem;
    };
    let (timestamp, suffix) = timestamp_with_suffix
        .split_once('-')
        .map_or((timestamp_with_suffix, None), |(timestamp, suffix)| {
            (timestamp, Some(suffix))
        });

    if timestamp.is_empty() || !timestamp.bytes().all(|byte| byte.is_ascii_digit()) {
        return stem;
    }
    if suffix.is_some_and(|suffix| {
        suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit())
    }) {
        return stem;
    }

    &stem[..after_marker_start + segment_index.len()]
}

#[cfg(target_os = "macos")]
pub(crate) fn audio_only_output_files(
    output_files: Option<&CaptureOutputFiles>,
    include_microphone: bool,
    include_system_audio: bool,
) -> Option<CaptureOutputFiles> {
    let output_files = output_files?;
    let mut audio_only = empty_output_files();

    if include_microphone {
        audio_only.microphone_file = output_files.microphone_file.clone();
        audio_only.microphone_files = output_files.microphone_files.clone();
    }

    if include_system_audio {
        audio_only.system_audio_file = output_files.system_audio_file.clone();
        audio_only.system_audio_files = output_files.system_audio_files.clone();
    }

    let has_audio = audio_only.microphone_file.is_some()
        || !audio_only.microphone_files.is_empty()
        || audio_only.system_audio_file.is_some()
        || !audio_only.system_audio_files.is_empty();

    has_audio.then_some(audio_only)
}

#[cfg(target_os = "macos")]
pub(crate) fn append_and_persist_committed_audio_outputs(
    runtime: &mut NativeCaptureRuntime,
    core: Option<&Arc<AppCore>>,
    output_files: Option<&CaptureOutputFiles>,
) {
    let Some(output_files) = output_files else {
        return;
    };

    if let Some(committed) = runtime.output_files.as_mut() {
        append_committed_segment_output_files(committed, output_files);
    }

    persist_committed_audio_segments(
        core,
        runtime.source_sessions.as_ref(),
        runtime.segment_schedule.as_ref(),
        runtime.current_segment_index,
        Some(output_files),
    );
}

#[cfg(target_os = "macos")]
pub(crate) fn ensure_audio_dir_exists(audio_dir: &Path) -> Result<(), CaptureErrorResponse> {
    std::fs::create_dir_all(audio_dir).map_err(|error| CaptureErrorResponse {
        code: "io_error".to_string(),
        message: format!("Failed to create capture audio directory: {error}"),
    })
}

#[cfg(target_os = "macos")]
pub(crate) fn next_reanchored_microphone_output_file(
    runtime: &mut NativeCaptureRuntime,
    next_index: u64,
    context: &str,
) -> Result<Option<String>, CaptureErrorResponse> {
    if runtime.inactivity.is_microphone_paused()
        || runtime.active_microphone_session.is_none()
        || !runtime
            .requested_sources
            .as_ref()
            .is_some_and(|sources| sources.microphone)
    {
        return Ok(None);
    }

    let planner = ensure_microphone_planner_for_runtime(runtime, context)?.ok_or_else(|| {
        CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: format!("Capture microphone planner missing while {context}"),
        }
    })?;
    ensure_audio_dir_exists(&planner.audio_dir())?;

    Ok(Some(
        planner
            .microphone_reconnect_file(next_index, now_unix_ms())
            .to_string_lossy()
            .to_string(),
    ))
}

/// Brings the system-audio tap up for a session that requests it, seeding its
/// planner and opening the first segment at `segment_index`.
///
/// Callable with no screen session and no display: that is the whole point of the
/// family (ADR 0052). A start failure is returned to the caller, which decides
/// whether the session can go on without system audio.
#[cfg(target_os = "macos")]
pub(crate) fn start_system_audio_family_for_runtime(
    runtime: &mut NativeCaptureRuntime,
    excluded_bundle_ids: Vec<String>,
    segment_index: u64,
    context: &str,
) -> Result<Option<String>, CaptureErrorResponse> {
    if !runtime
        .requested_sources
        .as_ref()
        .is_some_and(|sources| sources.system_audio)
        || runtime.inactivity.is_system_audio_paused()
        || runtime.active_system_audio_session.is_some()
    {
        return Ok(None);
    }

    let planner = ensure_system_audio_planner_for_runtime(runtime, context)?.ok_or_else(|| {
        CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: format!("Capture system-audio planner missing while {context}"),
        }
    })?;
    ensure_audio_dir_exists(&planner.audio_dir())?;
    let first_output_file = planner.system_audio_file(segment_index);

    let family = crate::native_capture::system_audio::SystemAudioFamily::start(
        planner,
        segment_index,
        first_output_file,
        excluded_bundle_ids,
    )?;
    let output_file = family.live_output_file();
    runtime.active_system_audio_session = Some(family);

    Ok(output_file)
}

/// Rotates the live system-audio segment onto a fresh file for `next_index` and
/// returns it, or `None` when there is nothing recording to rotate.
///
/// Nothing here asks about the screen. The tap has no display dependency, so the
/// only questions worth asking are whether system audio is requested, unpaused,
/// and has a live tap (ADR 0052).
#[cfg(target_os = "macos")]
pub(crate) fn next_reanchored_system_audio_output_file(
    runtime: &mut NativeCaptureRuntime,
    next_index: u64,
    context: &str,
) -> Result<Option<String>, CaptureErrorResponse> {
    if runtime.inactivity.is_system_audio_paused()
        || runtime.system_audio_recording_file.is_none()
        || runtime.active_system_audio_session.is_none()
        || !runtime
            .requested_sources
            .as_ref()
            .is_some_and(|sources| sources.system_audio)
    {
        return Ok(None);
    }

    let planner = ensure_system_audio_planner_for_runtime(runtime, context)?.ok_or_else(|| {
        CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: format!("Capture system-audio planner missing while {context}"),
        }
    })?;
    ensure_audio_dir_exists(&planner.audio_dir())?;
    let next_file = planner.system_audio_resume_file(next_index, now_unix_ms());

    Ok(runtime
        .active_system_audio_session
        .as_mut()
        .expect("system audio session checked above")
        .advance_segment(&planner, next_index, Some(next_file)))
}
