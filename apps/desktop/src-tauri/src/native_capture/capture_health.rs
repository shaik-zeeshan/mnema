//! `get_capture_health`: one read-only snapshot of every capture source, the
//! screen suspension, the system-audio tap and the disk thresholds, for the
//! developer window. `capture_health_snapshot` is the one seam — the health
//! rollup reuses it rather than reading the runtime itself.
//!
//! Never logs: it is polled every second.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use capture_microphone as microphone_capture;
use serde::Serialize;
use tauri::Manager;

use super::disk_space::{self, RESERVE_FLOOR_BYTES};
use super::runtime::NativeCaptureRuntime;
use super::NativeCaptureState;

/// A system-audio rebuild this recent reads as "reconnecting". A tunable
/// ceiling: a rebuild finishes inside one tick, so this is how long the lane
/// keeps saying so afterwards.
const RECONNECTING_WINDOW: Duration = Duration::from_secs(60);

/// Quietest level the meters show; anything below (and exact silence) clamps here.
const LEVEL_FLOOR_DB: f64 = -60.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CaptureSourceState {
    Off,
    Recording,
    Paused,
    Waiting,
    Reconnecting,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaptureSourceHealth {
    pub requested: bool,
    pub state: CaptureSourceState,
    pub reason: Option<String>,
    pub level_db: Option<f64>,
    pub idle_ms: Option<u64>,
    pub device_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SuspensionKindDto {
    PrivacyFilter,
    DisplayUnavailable,
    LowDisk,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaptureSuspensionDto {
    pub kind: SuspensionKindDto,
    pub reason: String,
    pub recovery_attempts: u8,
    pub restart_required: bool,
    pub since_ms: Option<u64>,
    pub last_error_message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum TapRebuildReasonDto {
    DeviceChanged,
    DeviceDied,
    SampleRateChanged,
    ExcludeListMoved,
    SilenceWatchdog,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SystemAudioTapDto {
    /// Since local midnight, within this app run (not persisted).
    pub rebuilds_today: u64,
    pub last_reason: Option<TapRebuildReasonDto>,
    pub last_rebuild_at_ms: Option<u64>,
    pub last_detail: Option<String>,
    pub next_silence_check_in_ms: Option<u64>,
    pub watchdog_delay_ms: Option<u64>,
    pub sample_rate_hz: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiskHealthDto {
    pub free_bytes: Option<u64>,
    pub reserve_floor_bytes: u64,
    /// `None` while not recording: the thresholds follow the live session's
    /// bitrate and sources.
    pub pause_threshold_bytes: Option<u64>,
    pub resume_threshold_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaptureHealthDto {
    pub recording: bool,
    pub started_at_ms: Option<u64>,
    pub screen: CaptureSourceHealth,
    pub microphone: CaptureSourceHealth,
    pub system_audio: CaptureSourceHealth,
    pub suspension: Option<CaptureSuspensionDto>,
    pub system_audio_tap: Option<SystemAudioTapDto>,
    pub disk: DiskHealthDto,
}

// ── Pure mapping ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Screen,
    Microphone,
    SystemAudio,
}

/// The suspension as the state mapping sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Suspended {
    PrivacyFilter { restart_required: bool },
    DisplayUnavailable,
    LowDisk,
}

#[derive(Debug, Clone, Copy, Default)]
struct SourceFacts {
    recording: bool,
    requested: bool,
    session_active: bool,
    /// Inactivity pause for this source, or the user's pause.
    paused: bool,
    /// System audio only: a rebuild inside [`RECONNECTING_WINDOW`], or a
    /// session whose last rebuild left it without a tap.
    reconnecting: bool,
}

fn source_state(
    source: Source,
    facts: SourceFacts,
    suspension: Option<Suspended>,
) -> CaptureSourceState {
    if !facts.recording || !facts.requested {
        return CaptureSourceState::Off;
    }
    match (suspension, source) {
        (Some(Suspended::LowDisk), _) => return CaptureSourceState::Paused,
        // Display-unavailable and privacy-filter suspensions stop the screen
        // alone (ADR 0021, amended); the audio families record through them.
        (Some(Suspended::DisplayUnavailable), Source::Screen) => {
            return CaptureSourceState::Waiting
        }
        (Some(Suspended::PrivacyFilter { restart_required }), Source::Screen) => {
            return if restart_required {
                CaptureSourceState::Failed
            } else {
                CaptureSourceState::Waiting
            };
        }
        _ => {}
    }
    if facts.paused {
        CaptureSourceState::Paused
    } else if facts.reconnecting {
        CaptureSourceState::Reconnecting
    } else if facts.session_active {
        CaptureSourceState::Recording
    } else {
        CaptureSourceState::Failed
    }
}

/// dBFS from a normalized (0..=1) peak, clamped to [`LEVEL_FLOOR_DB`].
fn level_db(normalized_peak: Option<f32>) -> Option<f64> {
    let peak = f64::from(normalized_peak?);
    if !peak.is_finite() {
        return None;
    }
    Some((20.0 * peak.log10()).clamp(LEVEL_FLOOR_DB, 0.0))
}

fn unix_ms(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|elapsed| elapsed.as_millis() as u64)
}

#[cfg(target_os = "macos")]
fn tap_reason(reason: capture_system_audio::RebuildReason) -> TapRebuildReasonDto {
    use capture_system_audio::RebuildReason as R;
    match reason {
        R::DefaultOutputDeviceChanged => TapRebuildReasonDto::DeviceChanged,
        R::DeviceSampleRateChanged => TapRebuildReasonDto::SampleRateChanged,
        R::DeviceDied => TapRebuildReasonDto::DeviceDied,
        R::ExcludeListMoved => TapRebuildReasonDto::ExcludeListMoved,
        R::ZeroWatchdog => TapRebuildReasonDto::SilenceWatchdog,
    }
}

// ── Snapshot ───────────────────────────────────────────────────────────────

/// Everything copied out under the capture lock; the disk read happens after
/// it is released (a stat on a network volume can block).
struct Locked {
    dto: CaptureHealthDto,
    recordings_root: Option<PathBuf>,
}

fn read_locked(runtime: &NativeCaptureRuntime) -> Locked {
    let recording = runtime.is_running;
    let sources = runtime.requested_sources.as_ref();
    let requested = |pick: fn(&capture_types::CaptureSources) -> bool| sources.is_some_and(pick);
    let user_paused = runtime.user_capture_paused;

    let started_at_ms = runtime.source_sessions.as_ref().and_then(|sessions| {
        [
            &sessions.screen,
            &sessions.microphone,
            &sessions.system_audio,
        ]
        .into_iter()
        .flatten()
        .map(|session| session.started_at_unix_ms)
        .min()
    });

    #[cfg(target_os = "macos")]
    let (suspended, suspension) = match runtime.capture_suspension.as_ref() {
        Some(suspension) => {
            use super::runtime::{CaptureSuspensionKind as K, CaptureSuspensionStatus};
            let restart_required = suspension.status == CaptureSuspensionStatus::RestartRequired;
            let (suspended, kind) = match suspension.kind {
                K::PrivacyFilter => (
                    Suspended::PrivacyFilter { restart_required },
                    SuspensionKindDto::PrivacyFilter,
                ),
                K::DisplayUnavailable => (
                    Suspended::DisplayUnavailable,
                    SuspensionKindDto::DisplayUnavailable,
                ),
                K::LowDisk => (Suspended::LowDisk, SuspensionKindDto::LowDisk),
            };
            (
                Some(suspended),
                Some(CaptureSuspensionDto {
                    kind,
                    reason: suspension.reason.clone(),
                    recovery_attempts: suspension.recovery_attempts,
                    restart_required,
                    since_ms: suspension.since.and_then(unix_ms),
                    last_error_message: Some(suspension.last_error_message.clone())
                        .filter(|message| !message.is_empty()),
                }),
            )
        }
        None => (None, None),
    };
    #[cfg(not(target_os = "macos"))]
    let (suspended, suspension): (Option<Suspended>, Option<CaptureSuspensionDto>) = (None, None);

    #[cfg(target_os = "macos")]
    let tap = runtime
        .active_system_audio_session
        .as_ref()
        .map(|family| family.health());
    #[cfg(target_os = "macos")]
    let system_audio_tap = tap.as_ref().map(|tap| SystemAudioTapDto {
        rebuilds_today: super::system_audio::rebuilds_today(),
        last_reason: tap.last_reason.map(tap_reason),
        last_rebuild_at_ms: tap.last_rebuild_at.and_then(unix_ms),
        last_detail: tap.last_detail.clone(),
        next_silence_check_in_ms: tap
            .next_silence_check_in
            .map(|left| left.as_millis() as u64),
        watchdog_delay_ms: Some(tap.watchdog_delay.as_millis() as u64),
        sample_rate_hz: tap.sample_rate_hz,
    });
    #[cfg(target_os = "macos")]
    let system_audio_reconnecting = tap.as_ref().is_some_and(|tap| {
        tap.sample_rate_hz.is_none()
            || tap.last_rebuild_at.is_some_and(|at| {
                SystemTime::now()
                    .duration_since(at)
                    .is_ok_and(|ago| ago < RECONNECTING_WINDOW)
            })
    });
    #[cfg(target_os = "macos")]
    let last_rebuild_reason = tap
        .as_ref()
        .and_then(|tap| tap.last_reason)
        .map(|reason| reason.as_str().to_string());
    #[cfg(not(target_os = "macos"))]
    let (system_audio_tap, system_audio_reconnecting, last_rebuild_reason) =
        (None::<SystemAudioTapDto>, false, None::<String>);

    #[cfg(target_os = "macos")]
    let (screen_live, mic_live, sys_live) = (
        capture_screen::screen_capture_session_is_live(runtime.active_screen_session.as_ref())
            || runtime.recording_file.is_some(),
        runtime.active_microphone_session.is_some(),
        runtime.active_system_audio_session.is_some(),
    );
    #[cfg(not(target_os = "macos"))]
    let (screen_live, mic_live, sys_live) = (false, false, false);

    let suspension_reason = suspension.as_ref().map(|s| s.reason.clone());
    let lane = |source: Source,
                requested: bool,
                session_active: bool,
                inactivity_paused: bool,
                reconnecting: bool,
                level: Option<f32>,
                idle_ms: Option<u64>| {
        let facts = SourceFacts {
            recording,
            requested,
            session_active,
            paused: inactivity_paused || user_paused,
            reconnecting,
        };
        let state = source_state(source, facts, suspended);
        // The screen always carries the suspension reason — the bug this
        // command exists for: the frontend never saw "display asleep" on macOS.
        let reason = if source == Source::Screen || suspended == Some(Suspended::LowDisk) {
            suspension_reason.clone()
        } else {
            None
        }
        .or_else(|| match state {
            CaptureSourceState::Off if recording && !requested => Some("not_requested".into()),
            CaptureSourceState::Paused if user_paused => Some("user_paused".into()),
            CaptureSourceState::Paused => Some("inactivity".into()),
            CaptureSourceState::Reconnecting => last_rebuild_reason.clone(),
            _ => None,
        });
        CaptureSourceHealth {
            requested,
            state,
            reason,
            level_db: level_db(level),
            idle_ms,
            device_name: None,
        }
    };

    // Peek, never take: a debug poll must not consume the inactivity
    // detector's peak window.
    let dto = CaptureHealthDto {
        recording,
        started_at_ms,
        screen: lane(
            Source::Screen,
            requested(|s| s.screen),
            screen_live,
            runtime.inactivity.is_screen_paused(),
            false,
            None,
            capture_screen::screen_activity_idle_ms(),
        ),
        microphone: lane(
            Source::Microphone,
            requested(|s| s.microphone),
            mic_live,
            runtime.inactivity.is_microphone_paused(),
            false,
            microphone_capture::peek_microphone_activity_window_peak_level(),
            microphone_capture::microphone_activity_idle_ms(),
        ),
        system_audio: lane(
            Source::SystemAudio,
            requested(|s| s.system_audio),
            sys_live,
            runtime.inactivity.is_system_audio_paused(),
            system_audio_reconnecting,
            capture_system_audio::peek_system_audio_activity_window_peak_level(),
            capture_system_audio::system_audio_activity_idle_ms(),
        ),
        suspension,
        system_audio_tap,
        disk: disk_thresholds(runtime),
    };

    #[cfg(target_os = "macos")]
    let recordings_root = super::segments::recordings_root_for_runtime(runtime);
    #[cfg(not(target_os = "macos"))]
    let recordings_root = None;

    Locked {
        dto,
        recordings_root,
    }
}

#[cfg(target_os = "macos")]
fn disk_thresholds(runtime: &NativeCaptureRuntime) -> DiskHealthDto {
    let estimate = runtime
        .is_running
        .then(|| super::segments::next_segment_estimate_for_runtime(runtime));
    DiskHealthDto {
        free_bytes: None,
        reserve_floor_bytes: RESERVE_FLOOR_BYTES,
        pause_threshold_bytes: estimate.map(disk_space::pause_threshold_bytes),
        resume_threshold_bytes: estimate.map(disk_space::resume_threshold_bytes),
    }
}

#[cfg(not(target_os = "macos"))]
fn disk_thresholds(_runtime: &NativeCaptureRuntime) -> DiskHealthDto {
    DiskHealthDto {
        free_bytes: None,
        reserve_floor_bytes: RESERVE_FLOOR_BYTES,
        pause_threshold_bytes: None,
        resume_threshold_bytes: None,
    }
}

/// The one capture-health seam. Takes the capture lock once, copies out what it
/// needs, then measures free space (a single statfs, no directory walk) after
/// releasing it — on the live recordings root, else the storage root.
pub(crate) fn capture_health_snapshot(
    state: &NativeCaptureState,
    app_handle: &tauri::AppHandle,
) -> CaptureHealthDto {
    let Locked {
        mut dto,
        recordings_root,
    } = {
        let lifecycle = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        read_locked(lifecycle.runtime())
    };

    let root = recordings_root.or_else(|| {
        app_handle
            .try_state::<crate::app_infra::AppInfraState>()
            .map(|infra| infra.base_dir().to_path_buf())
    });
    dto.disk.free_bytes = root.and_then(|root| {
        disk_space::measure_free_space(&root, disk_space::default_free_space_probe)
    });
    dto
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live() -> SourceFacts {
        SourceFacts {
            recording: true,
            requested: true,
            session_active: true,
            ..SourceFacts::default()
        }
    }

    #[test]
    fn display_unavailable_waits_the_screen_while_audio_keeps_recording() {
        let asleep = Some(Suspended::DisplayUnavailable);
        assert_eq!(
            source_state(Source::Screen, live(), asleep),
            CaptureSourceState::Waiting
        );
        assert_eq!(
            source_state(Source::Microphone, live(), asleep),
            CaptureSourceState::Recording
        );
        assert_eq!(
            source_state(Source::SystemAudio, live(), asleep),
            CaptureSourceState::Recording
        );
    }

    #[test]
    fn low_disk_pauses_every_source() {
        for source in [Source::Screen, Source::Microphone, Source::SystemAudio] {
            assert_eq!(
                source_state(source, live(), Some(Suspended::LowDisk)),
                CaptureSourceState::Paused
            );
        }
    }

    #[test]
    fn privacy_filter_waits_until_it_needs_a_restart() {
        let retrying = Some(Suspended::PrivacyFilter {
            restart_required: false,
        });
        let stuck = Some(Suspended::PrivacyFilter {
            restart_required: true,
        });
        assert_eq!(
            source_state(Source::Screen, live(), retrying),
            CaptureSourceState::Waiting
        );
        assert_eq!(
            source_state(Source::Screen, live(), stuck),
            CaptureSourceState::Failed
        );
        assert_eq!(
            source_state(Source::Microphone, live(), stuck),
            CaptureSourceState::Recording
        );
    }

    #[test]
    fn other_states() {
        let off = SourceFacts {
            requested: false,
            ..live()
        };
        let paused = SourceFacts {
            paused: true,
            ..live()
        };
        let reconnecting = SourceFacts {
            reconnecting: true,
            ..live()
        };
        let lost = SourceFacts {
            session_active: false,
            ..live()
        };
        assert_eq!(
            source_state(Source::Screen, off, None),
            CaptureSourceState::Off
        );
        assert_eq!(
            source_state(Source::Microphone, paused, None),
            CaptureSourceState::Paused
        );
        assert_eq!(
            source_state(Source::SystemAudio, reconnecting, None),
            CaptureSourceState::Reconnecting
        );
        assert_eq!(
            source_state(Source::Microphone, lost, None),
            CaptureSourceState::Failed
        );
    }

    #[test]
    fn level_db_is_dbfs_clamped_to_the_floor() {
        assert_eq!(level_db(None), None);
        assert_eq!(level_db(Some(1.0)), Some(0.0));
        assert!((level_db(Some(0.5)).unwrap() + 6.0206).abs() < 1e-3);
        assert_eq!(level_db(Some(0.0)), Some(LEVEL_FLOOR_DB));
        assert_eq!(level_db(Some(1e-9)), Some(LEVEL_FLOOR_DB));
        assert_eq!(level_db(Some(f32::NAN)), None);
    }

    #[test]
    fn wire_shape_is_camel_case() {
        let json = serde_json::to_value(SystemAudioTapDto {
            rebuilds_today: 3,
            last_reason: Some(TapRebuildReasonDto::SampleRateChanged),
            last_rebuild_at_ms: None,
            last_detail: None,
            next_silence_check_in_ms: Some(1),
            watchdog_delay_ms: Some(30_000),
            sample_rate_hz: Some(16_000),
        })
        .unwrap();
        assert_eq!(json["rebuildsToday"], 3);
        assert_eq!(json["lastReason"], "sampleRateChanged");
        assert_eq!(json["nextSilenceCheckInMs"], 1);
        assert_eq!(
            serde_json::to_value(CaptureSourceState::Reconnecting).unwrap(),
            "reconnecting"
        );
        assert_eq!(
            serde_json::to_value(SuspensionKindDto::DisplayUnavailable).unwrap(),
            "displayUnavailable"
        );
    }
}
