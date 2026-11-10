//! `status-pop.test.ts`, case for case: same inputs, same words, tones and order.

use app_infra::ProcessorPipelineStatus;

use super::*;
use crate::shell::health::{Disk, IndexStatus, Tap};
use crate::shell::model::{EngineFlag, StatusBarModel};

const GB: u64 = 1 << 30;

fn src(state: &str, reason: Option<&str>) -> SourceHealth {
    SourceHealth {
        requested: true,
        state: state.into(),
        reason: reason.map(str::to_string),
    }
}

fn rec() -> SourceHealth {
    src("recording", None)
}

fn disk(free: u64) -> Disk {
    Disk {
        free_bytes: Some(free),
        pause_threshold_bytes: Some(5 * GB),
        resume_threshold_bytes: Some(5 * GB + GB / 2),
    }
}

fn health() -> Health {
    Health {
        screen: rec(),
        microphone: rec(),
        system_audio: rec(),
        system_audio_tap: None,
        disk: disk(182 * GB),
    }
}

fn tap(reason: Option<&str>, rate: Option<u32>) -> Option<Tap> {
    Some(Tap {
        last_reason: reason.map(str::to_string),
        sample_rate_hz: rate,
    })
}

fn lane(processor: &str) -> ProcessorPipelineStatus {
    ProcessorPipelineStatus {
        processor: processor.into(),
        queued: 0,
        retrying: 0,
        running: 0,
        completed: 0,
        failed: 0,
        failed_last_24h: 0,
        average_completed_seconds_last_24h: None,
        last_error: None,
    }
}

fn index(frames: i64) -> IndexStatus {
    IndexStatus {
        queued_frames: frames,
        oldest_queued_ms: None,
        semantic_building: false,
        semantic_indexed_percent: Some(100),
    }
}

fn backlog() -> Backlog {
    Backlog {
        index: Some(index(0)),
        pipeline: Some(vec![lane("ocr"), lane("audio_transcription")]),
        deepgram_auth_error: None,
    }
}

fn model(h: Option<Health>) -> StatusBarModel {
    StatusBarModel {
        running: true,
        health: h,
        ..Default::default()
    }
}

fn values(m: &StatusBarModel) -> Vec<(String, bool, Option<RowTone>)> {
    match capture_group(m) {
        CaptureGroup::Rows(rows) => rows
            .into_iter()
            .map(|r| (r.value, r.ring, r.tone))
            .collect(),
        other => panic!("{other:?}"),
    }
}

fn recording_row() -> (String, bool, Option<RowTone>) {
    ("Recording".into(), false, Some(RowTone::Ok))
}

fn busy_rows(c: Option<CatchingUp>) -> Vec<PopRow> {
    match c {
        Some(CatchingUp::Busy { rows, .. }) => rows,
        other => panic!("{other:?}"),
    }
}

// ── mockup states ───────────────────────────────────────────────────────────

#[test]
fn ok_three_recording_rows_caught_up_plain_free_space() {
    let m = model(Some(health()));
    assert_eq!(values(&m), vec![recording_row(); 3]);
    assert_eq!(
        catching_up(&m, Some(&backlog())),
        Some(CatchingUp::CaughtUp)
    );
    assert_eq!(disk_row(&m), Some(fill("Free space", None, "182 GB")));
    assert_eq!(ai_row(&m), None);
}

#[test]
fn catching_up_reading_with_eta_transcribing_clips_meaning_bar() {
    let m = model(Some(health()));
    let b = Backlog {
        index: Some(IndexStatus {
            semantic_building: true,
            semantic_indexed_percent: Some(92),
            ..index(38)
        }),
        pipeline: Some(vec![
            ProcessorPipelineStatus {
                average_completed_seconds_last_24h: Some(3.2),
                ..lane("ocr")
            },
            ProcessorPipelineStatus {
                queued: 2,
                running: 1,
                ..lane("audio_transcription")
            },
        ]),
        deepgram_auth_error: None,
    };
    assert_eq!(
        catching_up(&m, Some(&b)),
        Some(CatchingUp::Busy {
            meaning_percent: Some(92.),
            rows: vec![
                ring("Reading", "38 frames · about 2 min", None),
                ring("Transcribing", "3 clips", None),
            ],
        })
    );
}

#[test]
fn display_asleep_neutral_ring_audio_continues() {
    let screen = src("waiting", Some("capture_display_unavailable"));
    let h = Health {
        screen: screen.clone(),
        ..health()
    };
    assert_eq!(
        capture_group(&model(Some(h))),
        CaptureGroup::Rows(vec![
            ring(
                "Screen",
                "Waiting for the display. Mic and system audio continue.",
                None
            ),
            fill("Microphone", Some(RowTone::Ok), "Recording"),
            fill("System audio", Some(RowTone::Ok), "Recording"),
        ])
    );
    let first = |h: Health| values(&model(Some(h)))[0].0.clone();
    let off = SourceHealth {
        requested: false,
        ..src("off", Some("not_requested"))
    };
    let mic_only = Health {
        screen: screen.clone(),
        system_audio: off.clone(),
        ..health()
    };
    assert_eq!(first(mic_only), "Waiting for the display. Mic continues.");
    let sys_only = Health {
        screen: screen.clone(),
        microphone: off,
        ..health()
    };
    assert_eq!(
        first(sys_only),
        "Waiting for the display. System audio continues."
    );
    let alone = Health {
        screen,
        microphone: src("off", None),
        system_audio: src("off", None),
        ..health()
    };
    assert_eq!(first(alone), "Waiting for the display.");
}

#[test]
fn sysaudio_rebuild_reconnecting_after_the_reason_only_while_the_tap_is_down() {
    let row = |reason: Option<&str>| {
        let h = Health {
            system_audio: src("reconnecting", None),
            system_audio_tap: tap(reason, None),
            ..health()
        };
        values(&model(Some(h)))[2].clone()
    };
    assert_eq!(
        row(Some("deviceChanged")),
        (
            "Reconnecting after the output device changed".into(),
            true,
            None
        )
    );
    assert_eq!(
        row(Some("deviceDied")).0,
        "Reconnecting after the output device went away"
    );
    assert_eq!(
        row(Some("sampleRateChanged")).0,
        "Reconnecting after the headset changed mode"
    );
    assert_eq!(
        row(Some("excludeListMoved")).0,
        "Reconnecting after an excluded app started or quit"
    );
    assert_eq!(
        row(Some("silenceWatchdog")).0,
        "Reconnecting after a long silence"
    );
    assert_eq!(row(None).0, "Reconnecting");
    // Founder ruling 1: a watchdog rebuild with the tap up reads as Recording.
    let up = Health {
        system_audio: src("reconnecting", None),
        system_audio_tap: tap(Some("silenceWatchdog"), Some(48_000)),
        ..health()
    };
    assert_eq!(values(&model(Some(up)))[2], recording_row());
}

#[test]
fn low_disk_soon_warn_row_with_the_pause_threshold_and_open_storage() {
    let free = (6.1 * GB as f64) as u64;
    let m = model(Some(Health {
        disk: disk(free),
        ..health()
    }));
    assert_eq!(
        disk_row(&m),
        Some(PopRow {
            note: Some("Recording pauses below 5 GB and resumes when you free up space.".into()),
            action: Some(("Open Storage", "storage")),
            ..fill("Free space", Some(RowTone::Warn), "6.1 GB left")
        })
    );
}

#[test]
fn engine_offline_deepgram_unreachable_and_the_ai_row_reuses_the_pill_tip() {
    let tip = "Can't reach ChatGPT right now — your sign-in is fine. Resumes on its own.";
    let m = StatusBarModel {
        deepgram_transcription: true,
        engine: Some(EngineFlag {
            label: "offline",
            error: true,
            tip: tip.into(),
        }),
        ..model(Some(health()))
    };
    let b = Backlog {
        pipeline: Some(vec![
            lane("ocr"),
            ProcessorPipelineStatus {
                queued: 3,
                retrying: 3,
                last_error: Some("connection refused".into()),
                ..lane("audio_transcription")
            },
        ]),
        ..backlog()
    };
    assert_eq!(
        catching_up(&m, Some(&b)),
        Some(CatchingUp::Busy {
            meaning_percent: None,
            rows: vec![PopRow {
                note: Some("Can’t reach Deepgram. Retries on its own.".into()),
                ..fill("Transcribing", Some(RowTone::Warn), "3 clips waiting")
            }],
        })
    );
    assert_eq!(
        ai_row(&m),
        Some(PopRow {
            action: Some(("AI settings", "intelligence")),
            ..fill("Ask AI", Some(RowTone::Warn), tip)
        })
    );
}

#[test]
fn not_recording_stopped_capture_reading_keeps_going_free_space_only() {
    let h = Health {
        disk: Disk {
            pause_threshold_bytes: None,
            resume_threshold_bytes: None,
            ..disk(182 * GB)
        },
        ..health()
    };
    let m = StatusBarModel {
        running: false,
        ..model(Some(h))
    };
    let b = Backlog {
        index: Some(index(12)),
        pipeline: Some(vec![ProcessorPipelineStatus {
            average_completed_seconds_last_24h: Some(2.),
            ..lane("ocr")
        }]),
        deepgram_auth_error: None,
    };
    assert_eq!(capture_group(&m), CaptureGroup::Stopped);
    assert_eq!(
        busy_rows(catching_up(&m, Some(&b)))[0].value,
        "12 frames · under a minute"
    );
    assert_eq!(disk_row(&m).unwrap().value, "182 GB");
}

// ── undrawn capture rows ────────────────────────────────────────────────────

fn screen_row(s: SourceHealth, low_disk: bool) -> (String, bool, Option<RowTone>) {
    let m = StatusBarModel {
        low_disk,
        ..model(Some(Health {
            screen: s,
            ..health()
        }))
    };
    values(&m)[0].clone()
}

#[test]
fn starting_while_the_session_spins_up() {
    let m = StatusBarModel {
        running: false,
        loading_start: true,
        ..model(Some(health()))
    };
    assert_eq!(capture_group(&m), CaptureGroup::Starting);
}

#[test]
fn unread_health_hides_the_rows() {
    assert_eq!(capture_group(&model(None)), CaptureGroup::Rows(vec![]));
}

#[test]
fn paused_rows() {
    let paused = |r| screen_row(src("paused", Some(r)), false);
    assert_eq!(paused("user_paused"), ("Paused by you".into(), true, None));
    assert_eq!(
        paused("inactivity"),
        ("Paused while you’re away".into(), true, None)
    );
    assert_eq!(
        paused("capture_low_disk"),
        (
            "Paused until you free up space".into(),
            false,
            Some(RowTone::Warn)
        )
    );
}

#[test]
fn privacy_and_failures() {
    assert_eq!(
        screen_row(src("waiting", Some("privacy_filter_apply_failed")), false),
        (
            "Privacy filter couldn’t be applied, retrying".into(),
            false,
            Some(RowTone::Warn)
        )
    );
    assert_eq!(
        screen_row(
            src("failed", Some("privacy_recovery_restart_required")),
            false
        ),
        (
            "Stopped. Restart recording to resume".into(),
            false,
            Some(RowTone::Danger)
        )
    );
    assert_eq!(
        screen_row(src("failed", None), false),
        ("Not capturing right now".into(), false, Some(RowTone::Warn))
    );
}

#[test]
fn off_for_this_recording() {
    let off = SourceHealth {
        requested: false,
        ..src("off", Some("not_requested"))
    };
    assert_eq!(
        screen_row(off, false),
        ("Off for this recording".into(), true, None)
    );
}

// ── catching up ─────────────────────────────────────────────────────────────

fn reading(frames: i64, avg: Option<f64>) -> String {
    let b = Backlog {
        index: Some(index(frames)),
        pipeline: Some(vec![ProcessorPipelineStatus {
            average_completed_seconds_last_24h: avg,
            ..lane("ocr")
        }]),
        deepgram_auth_error: None,
    };
    busy_rows(catching_up(&model(Some(health())), Some(&b)))[0]
        .value
        .clone()
}

#[test]
fn eta_boundaries() {
    assert_eq!(reading(1, Some(59.)), "1 frame · under a minute");
    assert_eq!(reading(1, Some(60.)), "1 frame · about 1 min");
    assert_eq!(reading(2, Some(4020.)), "2 frames · about 2h 14m");
    assert_eq!(reading(38, None), "38 frames");
}

#[test]
fn both_reads_failed_hides_the_group() {
    let m = model(Some(health()));
    let failed = Backlog::default();
    assert_eq!(catching_up(&m, Some(&failed)), None);
    assert_eq!(catching_up(&m, None), None, "not polled yet");
}

fn transcribing(deepgram: bool, auth: Option<&str>, audio: ProcessorPipelineStatus) -> PopRow {
    let m = StatusBarModel {
        deepgram_transcription: deepgram,
        ..model(Some(health()))
    };
    let b = Backlog {
        pipeline: Some(vec![audio]),
        deepgram_auth_error: auth.map(str::to_string),
        ..backlog()
    };
    busy_rows(catching_up(&m, Some(&b))).remove(0)
}

#[test]
fn deepgram_rejected_the_key() {
    let t = transcribing(
        true,
        Some("401"),
        ProcessorPipelineStatus {
            queued: 3,
            retrying: 3,
            ..lane("audio_transcription")
        },
    );
    assert_eq!(t.note.as_deref(), Some("Deepgram rejected your key."));
    assert_eq!(t.action, Some(("Transcription settings", "transcription")));
}

#[test]
fn deepgram_retrying_a_non_network_error_stays_neutral() {
    let last_error = "invalid data found when processing input (moov atom not found)";
    let t = transcribing(
        true,
        None,
        ProcessorPipelineStatus {
            queued: 1,
            retrying: 1,
            last_error: Some(last_error.into()),
            ..lane("audio_transcription")
        },
    );
    assert_eq!(
        t,
        ring("Transcribing", "1 clip", Some("Retrying on its own."))
    );
}

#[test]
fn other_provider_retrying_stays_neutral() {
    let t = transcribing(
        false,
        None,
        ProcessorPipelineStatus {
            queued: 1,
            retrying: 1,
            ..lane("audio_transcription")
        },
    );
    assert_eq!(
        t,
        ring("Transcribing", "1 clip", Some("Retrying on its own."))
    );
}

// ── disk and AI ─────────────────────────────────────────────────────────────

#[test]
fn paused_for_low_disk() {
    let m = StatusBarModel {
        low_disk: true,
        ..model(Some(Health {
            disk: disk((4.2 * GB as f64) as u64),
            ..health()
        }))
    };
    let r = disk_row(&m).unwrap();
    assert_eq!(r.value, "4.2 GB left");
    assert_eq!(
        r.note.as_deref(),
        Some("Recording is paused. It resumes on its own once you free up space.")
    );
}

fn stopped(free: u64) -> StatusBarModel {
    StatusBarModel {
        running: false,
        ..model(Some(Health {
            disk: Disk {
                free_bytes: Some(free),
                ..Default::default()
            },
            ..health()
        }))
    }
}

// Founder ruling: below 5 GB always warns, even with the thresholds unknown (stopped).
#[test]
fn stopped_at_0_4_gb_warns_without_claiming_a_5_gb_pause() {
    let r = disk_row(&stopped((0.4 * GB as f64) as u64)).unwrap();
    assert_eq!(r.tone, Some(RowTone::Warn));
    assert_eq!(r.value, "0.4 GB left");
    assert_eq!(
        r.note.as_deref(),
        Some("Less than 5 GB free. Recording pauses when the disk is nearly full.")
    );
    assert_eq!(r.action, Some(("Open Storage", "storage")));
    // Both sides of the 5 GB floor.
    assert_eq!(
        disk_row(&stopped(LOW_DISK_WARN_BYTES - 1)).unwrap().tone,
        Some(RowTone::Warn)
    );
    assert_eq!(disk_row(&stopped(LOW_DISK_WARN_BYTES)).unwrap().tone, None);
}

#[test]
fn stopped_at_50_gb_stays_neutral() {
    assert_eq!(
        disk_row(&stopped(50 * GB)),
        Some(fill("Free space", None, "50 GB"))
    );
}

#[test]
fn recording_with_thresholds_and_plenty_free_stays_neutral() {
    let m = model(Some(Health {
        disk: disk(20 * GB),
        ..health()
    }));
    assert_eq!(disk_row(&m).unwrap().tone, None);
}

#[test]
fn unreadable_free_space_omits_the_row() {
    assert_eq!(disk_row(&model(None)), None);
    // Health unread: the shell's own free-space read stands in, as if stopped.
    let m = StatusBarModel {
        free_bytes: Some(50 * GB),
        ..model(None)
    };
    assert_eq!(disk_row(&m), Some(fill("Free space", None, "50 GB")));
}

#[test]
fn engine_tones() {
    let with = |label, error| StatusBarModel {
        engine: Some(EngineFlag {
            label,
            error,
            tip: "x".into(),
        }),
        ..model(Some(health()))
    };
    assert_eq!(
        ai_row(&with("needs reconnect", true)).unwrap().tone,
        Some(RowTone::Danger)
    );
    assert_eq!(ai_row(&with("not set up", false)).unwrap().tone, None);
}
