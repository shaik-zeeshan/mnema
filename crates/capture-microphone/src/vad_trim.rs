//! Where a finished microphone file starts: VAD boundary trimming of its
//! silent lead-in and tail, and the resume-stamp correction for audio a resume
//! carried in from before its stamp.

use capture_types::CaptureErrorResponse;
use capture_writers::{
    audio_writer_session_start_secs, audio_writer_timeline_gap_secs, trim_audio_file_to_m4a,
    AUDIO_TRIM_RANGE_PAST_END_CODE,
};

use super::{
    current_microphone_vad_boundary_trim_disabled_sequence, maybe_remove_microphone_output_file,
    take_microphone_vad_speech_boundaries, MicrophoneOutputContext, MicrophoneOutputFinalization,
    MicrophoneVadSpeechBoundaryState,
};

/// Delivery jitter allowed before the carried span: a buffer's PTS can sit a
/// little before the carried audio's own start.
const RESUME_STAMP_EARLY_SLACK_SECS: f64 = 1.0;

/// How long after the resume instant the first sample may land. A resume that
/// carried nothing (a device just restarted, an empty ring) waits for the
/// device to start, and a Bluetooth headset switching into its call profile
/// takes a few seconds; anything further out is not host-clock PTS.
const RESUME_STAMP_LATE_SLACK_SECS: f64 = 10.0;

/// A resumed file opens with the audio queued while paused, so its first sample
/// predates the resume instant its path is stamped with by the span the resume
/// carried (up to `MAX_PENDING_MIC_SAMPLES` buffers). Both are host-clock seconds.
pub(super) fn microphone_resume_stamp_correction_ms(context: &MicrophoneOutputContext) -> i64 {
    let (Some(resumed_at_secs), Some(first_sample_secs)) = (
        context.resumed_at_host_secs,
        context
            .writer
            .as_ref()
            .and_then(audio_writer_session_start_secs),
    ) else {
        return 0;
    };
    resume_stamp_correction_ms(
        resumed_at_secs,
        context.resume_carried_secs,
        first_sample_secs,
    )
}

pub(super) fn resume_stamp_correction_ms(
    resumed_at_secs: f64,
    carried_secs: f64,
    first_sample_secs: f64,
) -> i64 {
    let correction_secs = first_sample_secs - resumed_at_secs;
    // A gap the carried audio cannot explain means the PTS were not host-clock;
    // keep the resume stamp rather than move the file by garbage.
    if !correction_secs.is_finite()
        || correction_secs < -(carried_secs + RESUME_STAMP_EARLY_SLACK_SECS)
        || correction_secs > RESUME_STAMP_LATE_SLACK_SECS
    {
        return 0;
    }
    (correction_secs * 1000.0).round() as i64
}

const MICROPHONE_VAD_BOUNDARY_PADDING_SECS: f64 = 1.0;

/// VAD boundaries are host-clock PTS; the file starts at the writer's session
/// start, so rebase onto it before padding into a file-relative trim range.
/// `None` keeps the whole file: no usable timing, or a file with a hole in it,
/// where a file offset no longer maps back to host time. Speech that all ended
/// before this file's first sample never gets here: the caller discards it.
pub(super) fn microphone_vad_trim_range_secs(
    boundaries: &MicrophoneVadSpeechBoundaryState,
    session_start_secs: Option<f64>,
    timeline_gap_secs: f64,
) -> Option<(f64, f64)> {
    let (false, false, Some(first_start_secs), Some(last_end_secs), Some(session_start_secs)) = (
        timeline_gap_secs > 0.0,
        boundaries.invalid_timing,
        boundaries.first_start_secs,
        boundaries.last_end_secs,
        session_start_secs,
    ) else {
        return None;
    };
    if last_end_secs <= session_start_secs {
        return None;
    }
    let start =
        (first_start_secs - session_start_secs - MICROPHONE_VAD_BOUNDARY_PADDING_SECS).max(0.0);
    let end =
        (last_end_secs - session_start_secs + MICROPHONE_VAD_BOUNDARY_PADDING_SECS).max(start);
    Some((start, end))
}

pub(super) fn microphone_output_finalization_for_context(
    context: &MicrophoneOutputContext,
    speech_detected: bool,
    start_offset_ms: i64,
    discard_reason: Option<String>,
) -> MicrophoneOutputFinalization {
    MicrophoneOutputFinalization {
        source_file: context.source_output_file.clone(),
        output_file: context.output_file.clone(),
        speech_detected,
        start_offset_ms,
        discard_reason,
    }
}

pub(super) fn take_microphone_vad_speech_boundaries_for_context(
    context: &mut MicrophoneOutputContext,
) -> MicrophoneVadSpeechBoundaryState {
    let boundaries = take_microphone_vad_speech_boundaries();
    context.detected_vad_speech = boundaries.detected_speech;
    context.first_speech_start_secs = boundaries.first_start_secs;
    context.last_speech_end_secs = boundaries.last_end_secs;
    boundaries
}

pub(super) fn finalize_microphone_vad_boundary_trim(
    context: &mut MicrophoneOutputContext,
) -> Result<MicrophoneOutputFinalization, CaptureErrorResponse> {
    let disabled_sequence = current_microphone_vad_boundary_trim_disabled_sequence();
    if disabled_sequence > context.observed_boundary_trim_disabled_sequence {
        context.boundary_trim_disabled = true;
    }

    if !context.boundary_trim_enabled || context.boundary_trim_disabled {
        if context.boundary_trim_enabled {
            take_microphone_vad_speech_boundaries_for_context(context);
        }
        if let (Some(source), Some(output)) = (
            context.source_output_file.as_deref(),
            context.output_file.as_deref(),
        ) {
            if source != output {
                move_microphone_source_to_output(source, output)?;
            }
        }
        return Ok(microphone_output_finalization_for_context(
            context, false, 0, None,
        ));
    }

    let boundaries = take_microphone_vad_speech_boundaries_for_context(context);

    let Some(source_file) = context.source_output_file.clone() else {
        return Ok(microphone_output_finalization_for_context(
            context,
            boundaries.detected_speech,
            0,
            Some("missing_source_file".to_string()),
        ));
    };
    let Some(output_file) = context.output_file.clone() else {
        return Ok(microphone_output_finalization_for_context(
            context,
            boundaries.detected_speech,
            0,
            Some("missing_output_file".to_string()),
        ));
    };

    let session_start_secs = context
        .writer
        .as_ref()
        .and_then(audio_writer_session_start_secs);
    // Speech that all ended before this file's first sample was booked for the
    // previous file just before the swap; this file heard none of its own.
    let speech_ended_before_file = !boundaries.invalid_timing
        && boundaries
            .last_end_secs
            .zip(session_start_secs)
            .is_some_and(|(last_end, session_start)| last_end <= session_start);
    if !boundaries.detected_speech || speech_ended_before_file {
        maybe_remove_microphone_output_file(&source_file);
        maybe_remove_microphone_output_file(&output_file);
        capture_runtime::debug_log!(
            "[capture-microphone] discarded microphone output without VAD speech source={source_file}"
        );
        return Ok(MicrophoneOutputFinalization {
            source_file: Some(source_file),
            output_file: None,
            speech_detected: false,
            start_offset_ms: 0,
            discard_reason: Some("no_vad_speech".to_string()),
        });
    }

    let timeline_gap_secs = context
        .writer
        .as_ref()
        .map_or(0.0, audio_writer_timeline_gap_secs);
    if timeline_gap_secs > 0.0 {
        capture_runtime::debug_log!(
            "[capture-microphone] microphone output has a {timeline_gap_secs:.2}s gap; keeping full source"
        );
    }
    let trimmed_from_secs = microphone_vad_trim_range_secs(
        &boundaries,
        session_start_secs,
        timeline_gap_secs,
    )
        .and_then(|(trim_start_secs, trim_end_secs)| {
            match trim_audio_file_to_m4a(&source_file, &output_file, trim_start_secs, trim_end_secs)
            {
                Ok(trimmed) => trimmed.then_some(trim_start_secs),
                // Speech timed past the file's last sample (VAD frames that
                // outran the writer) leaves nothing to trim: routine.
                Err(error) if error.code == AUDIO_TRIM_RANGE_PAST_END_CODE => {
                    capture_runtime::debug_log!(
                        "[capture-microphone] microphone speech starts past the file end; keeping full source"
                    );
                    None
                }
                Err(error) => {
                    capture_runtime::warn_log!(
                        "[capture-microphone] failed to trim microphone output; keeping full source: [{}] {}",
                        error.code,
                        error.message
                    );
                    None
                }
            }
        });
    let start_offset_ms = match trimmed_from_secs {
        Some(trim_start_secs) => {
            maybe_remove_microphone_output_file(&source_file);
            (trim_start_secs * 1000.0).round().max(0.0) as i64
        }
        None => {
            move_microphone_source_to_output(&source_file, &output_file)?;
            0
        }
    };
    Ok(MicrophoneOutputFinalization {
        source_file: Some(source_file),
        output_file: Some(output_file),
        speech_detected: true,
        start_offset_ms,
        discard_reason: None,
    })
}

pub(super) fn move_microphone_source_to_output(
    source: &str,
    output: &str,
) -> Result<(), CaptureErrorResponse> {
    if source == output {
        return Ok(());
    }
    maybe_remove_microphone_output_file(output);
    std::fs::rename(source, output)
        .or_else(|_| {
            std::fs::copy(source, output).map(|_| {
                maybe_remove_microphone_output_file(source);
            })
        })
        .map_err(|error| CaptureErrorResponse {
            code: "io_error".to_string(),
            message: format!("Failed to move microphone source output into place: {error}"),
        })
}
