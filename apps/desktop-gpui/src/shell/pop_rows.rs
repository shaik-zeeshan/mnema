//! Every word the status popover ("What Mnema is doing") says, ported from
//! `lib/shell/status-pop.ts` over the status bar's model and the 2 s feed
//! (`health.rs`). `status_pop.rs` only renders these.

use super::health::{self, Backlog, Health, SourceHealth};
use super::model::StatusBarModel;
use crate::small::debug::readouts::gb;

/// Free space below this always warns. Not the backend's pause point (~1 GiB
/// reserve + the next segment); only where we start warning.
pub const LOW_DISK_WARN_BYTES: u64 = 5 * 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowTone {
    Ok,
    Warn,
    Danger,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopRow {
    pub name: &'static str,
    /// The neutral ring (in progress, not a problem); else a filled dot.
    pub ring: bool,
    /// A filled dot without a tone is the faint one.
    pub tone: Option<RowTone>,
    // shortcut: `Part[]`'s `{ num }` spans flatten to plain text (no tabular
    // figures inline); split into styled runs if the numbers ever jitter.
    pub value: String,
    /// The `<small>` second line.
    pub note: Option<String>,
    /// (button label, Settings tab).
    pub action: Option<(&'static str, &'static str)>,
}

fn ring(name: &'static str, value: impl Into<String>, note: Option<&str>) -> PopRow {
    PopRow {
        name,
        ring: true,
        tone: None,
        value: value.into(),
        note: note.map(str::to_string),
        action: None,
    }
}

fn fill(name: &'static str, tone: Option<RowTone>, value: impl Into<String>) -> PopRow {
    PopRow {
        name,
        ring: false,
        tone,
        value: value.into(),
        note: None,
        action: None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureGroup {
    Rows(Vec<PopRow>),
    Stopped,
    Starting,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CatchingUp {
    CaughtUp,
    Busy {
        rows: Vec<PopRow>,
        meaning_percent: Option<f64>,
    },
}

fn rebuild(reason: &str) -> Option<&'static str> {
    Some(match reason {
        "deviceChanged" => "the output device changed",
        "deviceDied" => "the output device went away",
        "sampleRateChanged" => "the headset changed mode",
        "excludeListMoved" => "an excluded app started or quit",
        "silenceWatchdog" => "a long silence",
        _ => return None,
    })
}

const STORAGE: (&str, &str) = ("Open Storage", "storage");

/// A reconnecting tap that is actually up (founder ruling 1) still records.
fn recording(src: &SourceHealth, sys_audio: bool, h: &Health) -> bool {
    src.state == "recording"
        || (sys_audio && src.state == "reconnecting" && !health::sys_audio_reconnecting(Some(h)))
}

fn source_row(
    name: &'static str,
    src: &SourceHealth,
    sys_audio: bool,
    h: &Health,
    m: &StatusBarModel,
) -> PopRow {
    if recording(src, sys_audio, h) {
        return fill(name, Some(RowTone::Ok), "Recording");
    }
    let reason = src.reason.as_deref();
    match src.state.as_str() {
        "reconnecting" => {
            let why = h
                .system_audio_tap
                .as_ref()
                .and_then(|t| t.last_reason.as_deref());
            match why.and_then(rebuild) {
                Some(why) => ring(name, format!("Reconnecting after {why}"), None),
                None => ring(name, "Reconnecting", None),
            }
        }
        "paused" if reason == Some("capture_low_disk") || m.low_disk => {
            fill(name, Some(RowTone::Warn), "Paused until you free up space")
        }
        "paused" if reason == Some("user_paused") => ring(name, "Paused by you", None),
        "paused" => ring(name, "Paused while you’re away", None),
        "waiting" if reason == Some("privacy_filter_apply_failed") => fill(
            name,
            Some(RowTone::Warn),
            "Privacy filter couldn’t be applied, retrying",
        ),
        "waiting" => {
            let mic = recording(&h.microphone, false, h);
            let sys = recording(&h.system_audio, true, h);
            let rest = match (mic, sys) {
                (true, true) => " Mic and system audio continue.",
                (true, false) => " Mic continues.",
                (false, true) => " System audio continues.",
                (false, false) => "",
            };
            ring(name, format!("Waiting for the display.{rest}"), None)
        }
        "failed" if reason == Some("privacy_recovery_restart_required") => fill(
            name,
            Some(RowTone::Danger),
            "Stopped. Restart recording to resume",
        ),
        "failed" => fill(name, Some(RowTone::Warn), "Not capturing right now"),
        _ => ring(name, "Off for this recording", None),
    }
}

pub fn capture_group(m: &StatusBarModel) -> CaptureGroup {
    if !m.running {
        return if m.loading_start {
            CaptureGroup::Starting
        } else {
            CaptureGroup::Stopped
        };
    }
    // ponytail: health unread (first poll pending or the read failed) hides the rows.
    let Some(h) = &m.health else {
        return CaptureGroup::Rows(Vec::new());
    };
    CaptureGroup::Rows(vec![
        source_row("Screen", &h.screen, false, h, m),
        source_row("Microphone", &h.microphone, false, h, m),
        source_row("System audio", &h.system_audio, true, h, m),
    ])
}

fn plural(n: i64, one: &str) -> String {
    if n == 1 {
        one.into()
    } else {
        format!("{one}s")
    }
}

/// "under a minute" · "about 2 min" · "about 2h 14m".
fn eta(seconds: f64) -> String {
    if seconds < 60. {
        "under a minute".into()
    } else {
        format!("about {}", health::format_wait((seconds * 1000.) as i64))
    }
}

/// `None` = neither backlog could be read: the group is hidden.
pub fn catching_up(m: &StatusBarModel, b: Option<&Backlog>) -> Option<CatchingUp> {
    let index = b.and_then(|b| b.index.as_ref());
    let pipeline = b.and_then(|b| b.pipeline.as_deref());
    if index.is_none() && pipeline.is_none() {
        return None;
    }
    let lane = |processor: &str| pipeline?.iter().find(|p| p.processor == processor);
    let mut rows = Vec::new();

    let frames = index.map_or(0, |i| i.queued_frames);
    if frames > 0 {
        // ponytail: one job = one frame, serial throughput; ignores worker parallelism.
        let avg = lane("ocr").and_then(|l| l.average_completed_seconds_last_24h);
        let mut value = format!("{frames} {}", plural(frames, "frame"));
        if let Some(avg) = avg {
            value.push_str(&format!(" · {}", eta(frames as f64 * avg)));
        }
        rows.push(ring("Reading", value, None));
    }

    if let Some(audio) = lane("audio_transcription") {
        let clips = audio.queued + audio.running; // queued already includes retrying
        if clips > 0 {
            let value = format!("{clips} {}", plural(clips, "clip"));
            let auth = b
                .and_then(|b| b.deepgram_auth_error.as_deref())
                .filter(|e| !e.is_empty());
            let down = auth.is_some() || health::is_transient_error(audio.last_error.as_deref());
            if audio.retrying > 0 && m.deepgram_transcription && down {
                let mut row = fill(
                    "Transcribing",
                    Some(RowTone::Warn),
                    format!("{value} waiting"),
                );
                if auth.is_some() {
                    row.note = Some("Deepgram rejected your key.".into());
                    row.action = Some(("Transcription settings", "transcription"));
                } else {
                    row.note = Some("Can’t reach Deepgram. Retries on its own.".into());
                }
                rows.push(row);
            } else {
                let note = (audio.retrying > 0).then_some("Retrying on its own.");
                rows.push(ring("Transcribing", value, note));
            }
        }
    }

    let meaning_percent = index
        .filter(|i| i.semantic_building)
        .and_then(|i| i.semantic_indexed_percent)
        .map(f64::from);
    Some(if rows.is_empty() && meaning_percent.is_none() {
        CatchingUp::CaughtUp
    } else {
        CatchingUp::Busy {
            rows,
            meaning_percent,
        }
    })
}

pub fn disk_row(m: &StatusBarModel) -> Option<PopRow> {
    // Health unread: the shell's own free-space read, thresholds unknown (as stopped).
    let fallback = health::Disk {
        free_bytes: m.free_bytes,
        ..Default::default()
    };
    let disk = m.health.as_ref().map_or(&fallback, |h| &h.disk);
    let free_bytes = disk.free_bytes?;
    let free = format!("{} GB", gb(free_bytes as f64));
    let warn = |note: String| PopRow {
        note: Some(note),
        action: Some(STORAGE),
        ..fill("Free space", Some(RowTone::Warn), format!("{free} left"))
    };
    if m.low_disk {
        return Some(warn(
            "Recording is paused. It resumes on its own once you free up space.".into(),
        ));
    }
    let soon = health::disk_soon(m.health.as_ref(), m.running, m.low_disk);
    // Thresholds are None while stopped (e.g. after a disk-full stop), so a flat floor still warns.
    if (soon && disk.pause_threshold_bytes.is_some()) || free_bytes < LOW_DISK_WARN_BYTES {
        // Unknown threshold: 5 GB is only where we warn; the real pause is ~1 GB + the next segment.
        return Some(warn(match disk.pause_threshold_bytes {
            None => format!(
                "Less than {} GB free. Recording pauses when the disk is nearly full.",
                gb(LOW_DISK_WARN_BYTES as f64)
            ),
            Some(pause) => format!(
                "Recording pauses below {} GB and resumes when you free up space.",
                gb(pause as f64)
            ),
        }));
    }
    Some(fill("Free space", None, free))
}

/// Only while the engine is unhealthy (the same rule as its pill); reuses its tip.
pub fn ai_row(m: &StatusBarModel) -> Option<PopRow> {
    let e = m.engine.as_ref()?;
    let tone = if e.label == "needs reconnect" {
        Some(RowTone::Danger)
    } else if e.error {
        Some(RowTone::Warn)
    } else {
        None
    };
    Some(PopRow {
        action: Some(("AI settings", "intelligence")),
        ..fill("Ask AI", tone, e.tip.clone())
    })
}

#[cfg(test)]
#[path = "pop_rows_tests.rs"]
mod tests;
