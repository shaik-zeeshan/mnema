//! The per-source output planners a running session keeps, and the live
//! source/backend projections read from them.

#[cfg(target_os = "macos")]
use crate::disk_space;
use capture_runtime::{current_date_prefix, SegmentPlanner};
use capture_types::{CaptureErrorResponse, CaptureSources, SourceSessionMeta, SourceSessions};
#[cfg(target_os = "macos")]
use std::path::PathBuf;

#[cfg(target_os = "macos")]
use super::privacy_suspended_sources_for_runtime_state;
use super::{
    active_sources_for_inactivity_paused_state, has_any_capture_sources, now_unix_ms,
    prefixed_capture_id, NativeCaptureRuntime,
};

pub fn screen_planner_for_runtime(runtime: &NativeCaptureRuntime) -> Option<&SegmentPlanner> {
    runtime.segment_planner.as_ref()
}

pub fn microphone_planner_for_runtime(runtime: &NativeCaptureRuntime) -> Option<&SegmentPlanner> {
    runtime.microphone_planner.as_ref()
}

pub fn system_audio_planner_for_runtime(runtime: &NativeCaptureRuntime) -> Option<&SegmentPlanner> {
    runtime.system_audio_planner.as_ref()
}

pub fn refresh_runtime_planner_dates(runtime: &mut NativeCaptureRuntime) -> String {
    let date_prefix = current_date_prefix();

    if let Some(planner) = runtime.segment_planner.as_mut() {
        planner.set_date_prefix(date_prefix.clone());
    }
    if let Some(planner) = runtime.microphone_planner.as_mut() {
        planner.set_date_prefix(date_prefix.clone());
    }
    if let Some(planner) = runtime.system_audio_planner.as_mut() {
        planner.set_date_prefix(date_prefix.clone());
    }

    date_prefix
}

fn seed_source_planner_from_runtime(
    screen_planner: &SegmentPlanner,
    source_session_id: &str,
) -> SegmentPlanner {
    SegmentPlanner::with_date_prefix(
        screen_planner.save_root_dir(),
        source_session_id,
        screen_planner.date_prefix(),
    )
}

fn empty_source_sessions() -> SourceSessions {
    SourceSessions {
        screen: None,
        microphone: None,
        system_audio: None,
    }
}

fn source_session_started_at_seed(runtime: &NativeCaptureRuntime) -> u64 {
    runtime
        .source_sessions
        .as_ref()
        .and_then(|sessions| {
            sessions
                .screen
                .as_ref()
                .or(sessions.microphone.as_ref())
                .or(sessions.system_audio.as_ref())
                .map(|session| session.started_at_unix_ms)
        })
        .unwrap_or_else(now_unix_ms)
}

fn persist_microphone_source_session(
    runtime: &mut NativeCaptureRuntime,
    session_id: String,
) -> SourceSessionMeta {
    let started_at_unix_ms = runtime
        .source_sessions
        .as_ref()
        .and_then(|sessions| sessions.microphone.as_ref())
        .map(|session| session.started_at_unix_ms)
        .unwrap_or_else(|| source_session_started_at_seed(runtime));
    let source_session = SourceSessionMeta {
        session_id,
        started_at_unix_ms,
    };

    runtime
        .source_sessions
        .get_or_insert_with(empty_source_sessions)
        .microphone = Some(source_session.clone());

    source_session
}

fn persist_system_audio_source_session(
    runtime: &mut NativeCaptureRuntime,
    session_id: String,
) -> SourceSessionMeta {
    let started_at_unix_ms = runtime
        .source_sessions
        .as_ref()
        .and_then(|sessions| sessions.system_audio.as_ref())
        .map(|session| session.started_at_unix_ms)
        .unwrap_or_else(|| source_session_started_at_seed(runtime));
    let source_session = SourceSessionMeta {
        session_id,
        started_at_unix_ms,
    };

    runtime
        .source_sessions
        .get_or_insert_with(empty_source_sessions)
        .system_audio = Some(source_session.clone());

    source_session
}

pub fn ensure_microphone_planner_for_runtime(
    runtime: &mut NativeCaptureRuntime,
    _context: &str,
) -> Result<Option<SegmentPlanner>, CaptureErrorResponse> {
    if let Some(planner) = runtime.microphone_planner.clone() {
        persist_microphone_source_session(runtime, planner.session_id().to_string());
        return Ok(Some(planner));
    }

    if !runtime
        .requested_sources
        .as_ref()
        .is_some_and(|sources| sources.microphone)
    {
        return Ok(None);
    }

    let Some(screen_planner) = runtime.segment_planner.clone() else {
        // Screen planner not yet available — cannot seed microphone planner yet.
        return Ok(None);
    };
    let source_session = runtime
        .source_sessions
        .as_ref()
        .and_then(|sessions| sessions.microphone.as_ref())
        .cloned()
        .unwrap_or_else(|| {
            persist_microphone_source_session(
                runtime,
                prefixed_capture_id("mic").unwrap_or_else(|_| format!("mic_{}", now_unix_ms())),
            )
        });

    let planner = seed_source_planner_from_runtime(&screen_planner, &source_session.session_id);
    runtime.microphone_planner = Some(planner.clone());

    Ok(Some(planner))
}

pub fn ensure_system_audio_planner_for_runtime(
    runtime: &mut NativeCaptureRuntime,
    _context: &str,
) -> Result<Option<SegmentPlanner>, CaptureErrorResponse> {
    if let Some(planner) = runtime.system_audio_planner.clone() {
        persist_system_audio_source_session(runtime, planner.session_id().to_string());
        return Ok(Some(planner));
    }

    if !runtime
        .requested_sources
        .as_ref()
        .is_some_and(|sources| sources.system_audio)
    {
        return Ok(None);
    }

    let Some(screen_planner) = runtime.segment_planner.clone() else {
        // Screen planner not yet available — cannot seed system-audio planner yet.
        return Ok(None);
    };
    let source_session = runtime
        .source_sessions
        .as_ref()
        .and_then(|sessions| sessions.system_audio.as_ref())
        .cloned()
        .unwrap_or_else(|| {
            persist_system_audio_source_session(
                runtime,
                prefixed_capture_id("sysaudio")
                    .unwrap_or_else(|_| format!("sysaudio_{}", now_unix_ms())),
            )
        });

    let planner = seed_source_planner_from_runtime(&screen_planner, &source_session.session_id);
    runtime.system_audio_planner = Some(planner.clone());

    Ok(Some(planner))
}

pub fn current_segment_sources_for_runtime(
    runtime: &NativeCaptureRuntime,
) -> Option<CaptureSources> {
    #[cfg(target_os = "macos")]
    if runtime.capture_suspension.is_some() {
        return privacy_suspended_sources_for_runtime_state(
            runtime,
            runtime.inactivity.is_microphone_paused(),
        );
    }

    if let Some(sources) = runtime.current_segment_sources.clone() {
        return has_any_capture_sources(&sources).then_some(sources);
    }

    #[cfg(target_os = "macos")]
    if runtime.current_segment_output_files.is_some()
        || capture_screen::screen_capture_session_is_live(runtime.active_screen_session.as_ref())
        || runtime.active_microphone_session.is_some()
        || runtime.active_system_audio_session.is_some()
    {
        return runtime.requested_sources.as_ref().and_then(|sources| {
            active_sources_for_inactivity_paused_state(
                sources,
                runtime.inactivity.screen_paused,
                runtime.inactivity.microphone_paused,
                runtime.inactivity.system_audio_paused,
            )
        });
    }

    None
}

#[cfg(target_os = "macos")]
pub fn microphone_backend_active_for_runtime(runtime: &NativeCaptureRuntime) -> bool {
    !runtime.inactivity.is_microphone_paused()
        && runtime.active_microphone_session.is_some()
        && runtime.microphone_recording_file.is_some()
        && current_segment_sources_for_runtime(runtime).is_some_and(|sources| sources.microphone)
}

#[cfg(target_os = "macos")]
pub fn microphone_probe_active_for_runtime(runtime: &NativeCaptureRuntime) -> bool {
    runtime.active_microphone_session.is_some()
}

#[cfg(target_os = "macos")]
pub fn system_audio_probe_active_for_runtime(runtime: &NativeCaptureRuntime) -> bool {
    runtime.active_system_audio_session.is_some()
}

#[cfg(target_os = "macos")]
pub fn system_audio_writer_active_for_runtime(runtime: &NativeCaptureRuntime) -> bool {
    !runtime.inactivity.is_system_audio_paused()
        && runtime.active_system_audio_session.is_some()
        && runtime.system_audio_recording_file.is_some()
        && current_segment_sources_for_runtime(runtime).is_some_and(|sources| sources.system_audio)
}

/// The next-segment size estimate derived from a running runtime's stored screen
/// bitrate, segment duration, and requested audio sources. Mirrors the preflight
/// estimate so the rotation boundary reserves the same amount the preflight did.
#[cfg(target_os = "macos")]
pub fn next_segment_estimate_for_runtime(runtime: &NativeCaptureRuntime) -> u64 {
    let bitrate_bps = runtime.effective_screen_bitrate_bps.unwrap_or(0) as u64;
    let (microphone, system_audio) = runtime
        .requested_sources
        .as_ref()
        .map(|sources| (sources.microphone, sources.system_audio))
        .unwrap_or((false, false));
    let audio_bytes_per_sec = disk_space::audio_bytes_per_sec_for_sources(microphone, system_audio);
    let segment_duration_seconds = runtime
        .segment_schedule
        .as_ref()
        .map(|schedule| schedule.segment_duration().as_secs())
        .unwrap_or(0);
    disk_space::next_segment_estimate_bytes(
        bitrate_bps,
        audio_bytes_per_sec,
        segment_duration_seconds,
    )
}

/// The recordings root the runtime is writing into, recovered from any source
/// planner (screen/mic/system-audio all share the same recordings root). Used to
/// probe free space at rotation/recovery time.
#[cfg(target_os = "macos")]
pub fn recordings_root_for_runtime(runtime: &NativeCaptureRuntime) -> Option<PathBuf> {
    screen_planner_for_runtime(runtime)
        .or(runtime.microphone_planner.as_ref())
        .or(runtime.system_audio_planner.as_ref())
        .map(|planner| PathBuf::from(planner.save_root_dir()))
}
