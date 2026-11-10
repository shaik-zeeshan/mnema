//! The shell's live feed (`status-live.svelte.ts`): `get_capture_health` and
//! the processing backlog, read as the commands' wire JSON reduced to what the
//! status bar and popover show, plus the pure rules over them and the paused
//! clock. `data.rs` reads them from app-core; `fixture.rs` from the parity map.

use std::time::{Duration, Instant};

use app_infra::ProcessorPipelineStatus;
use serde::Deserialize;

pub use crate::quick_recall::model::IndexStatus;

/// `CaptureHealthDto` (app-core's type only serializes; its serde shape is the contract).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Health {
    pub screen: SourceHealth,
    pub microphone: SourceHealth,
    pub system_audio: SourceHealth,
    pub system_audio_tap: Option<Tap>,
    pub disk: Disk,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct SourceHealth {
    pub requested: bool,
    /// `off` · `recording` · `paused` · `waiting` · `reconnecting` · `failed`.
    pub state: String,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Tap {
    /// `deviceChanged` · `deviceDied` · `sampleRateChanged` · `excludeListMoved` · `silenceWatchdog`.
    pub last_reason: Option<String>,
    pub sample_rate_hz: Option<u32>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Disk {
    pub free_bytes: Option<u64>,
    /// `None` while stopped: they follow the live session.
    pub pause_threshold_bytes: Option<u64>,
    pub resume_threshold_bytes: Option<u64>,
}

/// The popover's "Catching up" reads (`get_quick_recall_index_status`,
/// `get_processing_pipeline_status`, `transcription_deepgram_auth_status`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Backlog {
    pub index: Option<IndexStatus>,
    pub pipeline: Option<Vec<ProcessorPipelineStatus>>,
    pub deepgram_auth_error: Option<String>,
}

/// System audio is rebuilding its tap, except a silence-watchdog rebuild whose
/// tap is already back up: that is still recording (founder decision 1).
pub fn sys_audio_reconnecting(health: Option<&Health>) -> bool {
    let Some(h) = health.filter(|h| h.system_audio.state == "reconnecting") else {
        return false;
    };
    !h.system_audio_tap.as_ref().is_some_and(|tap| {
        tap.last_reason.as_deref() == Some("silenceWatchdog") && tap.sample_rate_hz.is_some()
    })
}

// ponytail: ≈1 h of recording at 5-min segments; tune here.
pub const EARLY_WARNING_SEGMENTS: u64 = 12;

/// Free space is within ~an hour of the low-disk pause. `resume − pause` is one
/// segment's estimate (`disk_space.rs`); thresholds are `None` while stopped.
pub fn disk_soon(health: Option<&Health>, running: bool, low_disk: bool) -> bool {
    let Some(d) = health.map(|h| &h.disk) else {
        return false;
    };
    match (
        d.free_bytes,
        d.pause_threshold_bytes,
        d.resume_threshold_bytes,
    ) {
        (Some(free), Some(pause), Some(resume)) if running && !low_disk => {
            free < pause + EARLY_WARNING_SEGMENTS * resume.saturating_sub(pause)
        }
        _ => false,
    }
}

/// "PAUSED 00:12": the session carries no pause instant, so it counts from when
/// this window saw the pause begin, and shows no clock for a pause already on
/// at first sight (`pausedSinceMs` in StatusBar.svelte).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PauseClock {
    saw_unpaused: bool,
    since: Option<Instant>,
}

impl PauseClock {
    pub fn observe(&mut self, user_paused: bool, now: Instant) {
        if !user_paused {
            self.saw_unpaused = true;
            self.since = None;
        } else if self.since.is_none() && self.saw_unpaused {
            self.since = Some(now);
        }
    }

    pub fn elapsed(&self, now: Instant) -> Option<Duration> {
        self.since.map(|s| now.saturating_duration_since(s))
    }
}

/// `isTransientError` (debug/detail/jobs.ts): a provider that comes back on its own.
pub fn is_transient_error(error: Option<&str>) -> bool {
    use std::sync::OnceLock;
    static TRANSIENT: OnceLock<regex::Regex> = OnceLock::new();
    let re = TRANSIENT.get_or_init(|| {
        regex::Regex::new(
            r"(?i)timed? ?out|timeout|unreachable|could not connect|connection|network|dns|offline|temporar|(http|status)\s*(429|5\d\d)\b",
        )
        .expect("static regex")
    });
    error.is_some_and(|e| re.is_match(e))
}

/// `formatWait`: "2 min" under an hour, else "2h 14m".
pub fn format_wait(ms: i64) -> String {
    if ms < 3_600_000 {
        format!("{} min", (ms / 60_000).max(0))
    } else {
        crate::small::debug::model::format_running(ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1 << 30;

    fn health(state: &str, tap: Option<Tap>) -> Health {
        Health {
            system_audio: SourceHealth {
                requested: true,
                state: state.into(),
                reason: None,
            },
            system_audio_tap: tap,
            disk: Disk {
                free_bytes: Some(40 * GB),
                pause_threshold_bytes: Some(5 * GB),
                resume_threshold_bytes: Some(5 * GB + GB / 2),
            },
            ..Default::default()
        }
    }

    fn tap(reason: &str, rate: Option<u32>) -> Option<Tap> {
        Some(Tap {
            last_reason: Some(reason.into()),
            sample_rate_hz: rate,
        })
    }

    #[test]
    fn reconnecting_unless_a_watchdog_rebuild_already_has_its_tap() {
        let device = health("reconnecting", tap("deviceChanged", Some(48_000)));
        assert!(sys_audio_reconnecting(Some(&device)));
        let quiet = health("reconnecting", tap("silenceWatchdog", Some(48_000)));
        assert!(!sys_audio_reconnecting(Some(&quiet)));
        let down = health("reconnecting", tap("silenceWatchdog", None));
        assert!(sys_audio_reconnecting(Some(&down)));
        assert!(!sys_audio_reconnecting(Some(&health("recording", None))));
        assert!(!sys_audio_reconnecting(None));
    }

    #[test]
    fn disk_soon_boundary_and_stop_conditions() {
        // pause 5 GB, one segment ≈ 0.5 GB → early warning below 11 GB.
        let edge = 5 * GB + EARLY_WARNING_SEGMENTS * (GB / 2);
        let at = |free| {
            let mut h = health("recording", None);
            h.disk.free_bytes = Some(free);
            h
        };
        assert!(disk_soon(Some(&at(edge - 1)), true, false));
        assert!(!disk_soon(Some(&at(edge)), true, false));
        assert!(!disk_soon(Some(&at(edge + GB)), true, false));
        assert!(!disk_soon(Some(&at(6 * GB)), false, false), "stopped");
        assert!(!disk_soon(Some(&at(4 * GB)), true, true), "suspended");
        assert!(!disk_soon(None, true, false), "unread");
        let mut stopped = at(6 * GB);
        stopped.disk.pause_threshold_bytes = None;
        stopped.disk.resume_threshold_bytes = None;
        assert!(!disk_soon(Some(&stopped), true, false));
    }

    #[test]
    fn paused_clock_counts_from_the_pause_it_saw_begin() {
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);

        // Already paused at first sight: no clock, and none while it stays paused.
        let mut c = PauseClock::default();
        c.observe(true, at(0));
        c.observe(true, at(5_000));
        assert_eq!(c.elapsed(at(6_000)), None);

        // Unpaused, then paused at 10 s: counts from 10 s.
        c.observe(false, at(9_000));
        assert_eq!(c.elapsed(at(9_500)), None);
        c.observe(true, at(10_000));
        let shown = |c: &PauseClock, ms| {
            crate::shell::model::format_clock(c.elapsed(at(ms)).unwrap().as_millis() as i64, false)
        };
        assert_eq!(shown(&c, 10_999), "00:00");
        assert_eq!(shown(&c, 11_000), "00:01");
        // A second observation of the same pause keeps the first instant.
        c.observe(true, at(20_000));
        assert_eq!(shown(&c, 3_609_999), "59:59");
        assert_eq!(shown(&c, 3_610_000), "01:00:00", "hours appear at an hour");
        // Resume clears it; the next pause starts over.
        c.observe(false, at(3_700_000));
        assert_eq!(c.elapsed(at(3_700_001)), None);
        c.observe(true, at(3_800_000));
        assert_eq!(shown(&c, 3_800_000), "00:00");
    }

    #[test]
    fn transient_errors_and_waits() {
        assert!(is_transient_error(Some("connection refused")));
        assert!(is_transient_error(Some("HTTP 503 from upstream")));
        assert!(is_transient_error(Some("request timed out")));
        assert!(!is_transient_error(Some(
            "invalid data found when processing input (moov atom not found)"
        )));
        assert!(!is_transient_error(None));
        assert_eq!(format_wait(59_999), "0 min");
        assert_eq!(format_wait(120_000), "2 min");
        assert_eq!(format_wait(3_599_999), "59 min");
        assert_eq!(format_wait(3_600_000), "1h 0m");
        assert_eq!(format_wait(8_040_000), "2h 14m");
    }
}
