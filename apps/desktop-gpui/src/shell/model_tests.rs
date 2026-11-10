use super::*;
use crate::shell::health::{Disk, Health, SourceHealth, Tap};

fn live(requested: bool, paused: bool) -> RuntimeSource {
    RuntimeSource {
        requested,
        paused,
        live: requested && !paused,
        privacy_failed: false,
    }
}

#[test]
fn rec_view_precedence() {
    let mut m = StatusBarModel::default();
    assert_eq!(rec_view(&m).label, "NOT RECORDING");
    m.stopped_reason = Some("disk full".into());
    assert_eq!(rec_view(&m).label, "STOPPED · disk full");
    m.loading_start = true;
    assert_eq!(rec_view(&m).kind, RecMod::Starting);

    let mut m = StatusBarModel {
        running: true,
        runtime: Some([live(true, false), live(true, true), live(false, false)]),
        ..Default::default()
    };
    assert_eq!(
        rec_view(&m),
        RecView {
            kind: RecMod::Running,
            label: "REC".into(),
            clock: Some(Clock::Elapsed)
        }
    );
    // Any idle source sets the flag, but only all-requested-idle is "PAUSED · idle".
    m.inactivity_paused = true;
    assert_eq!(rec_view(&m).kind, RecMod::Running);
    m.runtime = Some([live(true, true), live(true, true), live(false, false)]);
    assert_eq!(rec_view(&m).label, "PAUSED · idle");
    assert_eq!(rec_view(&m).clock, None, "idle shows no clock");
    m.user_paused = true;
    assert_eq!(rec_view(&m).label, "PAUSED");
    assert_eq!(rec_view(&m).clock, Some(Clock::Paused));
    m.low_disk = true;
    assert_eq!(
        rec_view(&m).label,
        "PAUSED",
        "a user pause outranks low disk"
    );
    m.user_paused = false;
    assert_eq!(rec_view(&m).label, "PAUSED · low disk");
}

#[test]
fn lanes_follow_runtime_while_running_and_selection_while_stopped() {
    let mut m = StatusBarModel {
        selected: [true, false, true],
        ..Default::default()
    };
    let mods = live_lanes(&m).map(|l| l.0);
    assert_eq!(mods, [LaneMod::Starting, LaneMod::Off, LaneMod::Starting]);
    m.running = true;
    let mut failed = live(true, false);
    failed.privacy_failed = true;
    m.runtime = Some([failed, live(true, true), live(false, false)]);
    assert_eq!(
        live_lanes(&m),
        [
            (LaneMod::Error, Some("stopped")),
            (LaneMod::Paused, Some("idle")),
            (LaneMod::Off, None)
        ]
    );
}

#[test]
fn one_flag_with_documented_precedence() {
    let mut m = StatusBarModel::default();
    assert_eq!(status_flag(&m), None);
    m.selected[0] = true;
    m.screen_permission = Some(CapturePermissionState::Denied);
    assert_eq!(status_flag(&m).unwrap().action, "Open System Settings");
    m.read_only = true;
    assert_eq!(status_flag(&m).unwrap().action, "buy a license");
    // Auto-start found no display at launch: say why it isn't recording yet.
    m.waiting_for_display = true;
    assert_eq!(status_flag(&m).unwrap().action, "starts when it wakes");
    m.start_error = Some("boom".into());
    assert_eq!(status_flag(&m).unwrap().action, "Retry");
    let m = StatusBarModel {
        running: true,
        low_disk: true,
        ..Default::default()
    };
    assert_eq!(status_flag(&m).unwrap().text, "free up space ·");
}

// `engineFlag` (status-bar.test.ts).
#[test]
fn engine_pill_only_when_unhealthy() {
    let status = |f: fn(&mut AiStatus)| {
        let mut s = AiStatus {
            enabled: true,
            has_providers: true,
            configured: true,
            available: true,
            ..Default::default()
        };
        f(&mut s);
        s
    };
    // Healthy and off show nothing.
    assert_eq!(engine_flag(&status(|_| {}), None), None);
    let off = status(|s| {
        s.enabled = false;
        s.available = false;
        s.reason = Some("ai_runtime_disabled".into());
    });
    assert_eq!(engine_flag(&off, None), None);
    // Unreachable reads offline, never reconnect (ADR 0058).
    let offline = status(|s| {
        s.available = false;
        s.reason = Some("provider_unreachable:chatgpt".into());
    });
    assert_eq!(
        engine_flag(&offline, None),
        Some(EngineFlag {
            label: "offline",
            error: true,
            tip: "Can't reach ChatGPT right now — your sign-in is fine. Resumes on its own.".into()
        })
    );
    let reconnect = status(|s| {
        s.available = false;
        s.reason = Some("needs_reconnect:chatgpt".into());
    });
    let flag = engine_flag(&reconnect, None).unwrap();
    assert_eq!((flag.label, flag.error), ("needs reconnect", true));
    assert_eq!(
        flag.tip,
        "ChatGPT signed you out. Sign in again to keep going."
    );
    let none = status(|s| {
        s.has_providers = false;
        s.available = false;
        s.reason = Some("no_providers".into());
    });
    assert_eq!(
        engine_flag(&none, None),
        Some(EngineFlag {
            label: "not set up",
            error: false,
            tip: "No AI provider set up — Insights shows capture-only views".into()
        })
    );
    let vault = status(|s| {
        s.available = false;
        s.reason = Some("vault_denied".into());
    });
    let flag = engine_flag(&vault, None).unwrap();
    assert_eq!(flag.label, "keychain locked");
    assert!(!flag.tip.ends_with("Resumes on its own."));
    let fix = status(|s| {
        s.available = false;
        s.reason = Some("no_default_model".into());
    });
    assert_eq!(
        engine_flag(&fix, None),
        Some(EngineFlag {
            label: "not set up",
            error: false,
            tip: "Choose a global default model.".into()
        })
    );
}

// ── live health lanes (status-bar.test.ts) ──────────────────────────────────

const GB: u64 = 1 << 30;

fn src(state: &str, reason: Option<&str>) -> SourceHealth {
    SourceHealth {
        requested: true,
        state: state.into(),
        reason: reason.map(str::to_string),
    }
}

/// pause 5 GB, resume 5.5 GB → one segment ≈ 0.5 GB, early warning below 11 GB.
fn health() -> Health {
    Health {
        screen: src("recording", None),
        microphone: src("recording", None),
        system_audio: src("recording", None),
        system_audio_tap: Some(Tap {
            last_reason: None,
            sample_rate_hz: Some(48_000),
        }),
        disk: Disk {
            free_bytes: Some(40 * GB),
            pause_threshold_bytes: Some(5 * GB),
            resume_threshold_bytes: Some(5 * GB + GB / 2),
        },
    }
}

fn recording(runtime: RuntimeSources, h: Health) -> StatusBarModel {
    StatusBarModel {
        running: true,
        runtime: Some(runtime),
        health: Some(h),
        ..Default::default()
    }
}

const ALL_ON: RuntimeSources = [RuntimeSource {
    requested: true,
    paused: false,
    live: true,
    privacy_failed: false,
}; 3];

#[test]
fn display_asleep_waits_the_screen_while_mic_and_system_stay_on() {
    let h = Health {
        screen: src("waiting", Some("capture_display_unavailable")),
        ..health()
    };
    assert_eq!(
        live_lanes(&recording(ALL_ON, h)),
        [
            (LaneMod::Waiting, Some("display asleep")),
            (LaneMod::On, None),
            (LaneMod::On, None)
        ]
    );
}

#[test]
fn privacy_and_idle_keep_precedence_over_waiting() {
    let h = Health {
        screen: src("waiting", Some("capture_display_unavailable")),
        ..health()
    };
    let mut privacy = ALL_ON;
    privacy[0].privacy_failed = true;
    assert_eq!(
        live_lanes(&recording(privacy, h.clone()))[0].0,
        LaneMod::Error
    );
    let mut idle = ALL_ON;
    idle[0].paused = true;
    assert_eq!(live_lanes(&recording(idle, h))[0].0, LaneMod::Paused);
}

#[test]
fn reconnecting_after_a_device_change_waits_a_quiet_watchdog_stays_on() {
    let tap = |reason: &str, rate| {
        Some(Tap {
            last_reason: Some(reason.into()),
            sample_rate_hz: rate,
        })
    };
    let device = Health {
        system_audio: src("reconnecting", Some("default_output_device_changed")),
        system_audio_tap: tap("deviceChanged", Some(48_000)),
        ..health()
    };
    assert_eq!(
        live_lanes(&recording(ALL_ON, device))[2],
        (LaneMod::Waiting, Some("reconnecting"))
    );
    let quiet = Health {
        system_audio: src("reconnecting", None),
        system_audio_tap: tap("silenceWatchdog", Some(48_000)),
        ..health()
    };
    assert_eq!(live_lanes(&recording(ALL_ON, quiet))[2].0, LaneMod::On);
    // A watchdog rebuild whose tap is still down is a real reconnect.
    let down = Health {
        system_audio: src("reconnecting", None),
        system_audio_tap: tap("silenceWatchdog", None),
        ..health()
    };
    assert_eq!(live_lanes(&recording(ALL_ON, down))[2].0, LaneMod::Waiting);
    // Unread health: the runtime sources alone.
    let mut unread = recording(ALL_ON, health());
    unread.health = None;
    assert_eq!(live_lanes(&unread).map(|l| l.0), [LaneMod::On; 3]);
}

#[test]
fn flag_privacy_beats_low_disk_beats_disk_soon() {
    let soon = Health {
        disk: Disk {
            free_bytes: Some((6.1 * GB as f64) as u64),
            ..health().disk
        },
        ..health()
    };
    let m = recording(ALL_ON, soon);
    assert_eq!(
        status_flag(&m),
        Some(StatusFlag {
            tone: Tone::Info,
            text: "disk almost full ·",
            action: "6.1 GB left".into()
        })
    );
    // Already suspended → the low-disk flag, never both.
    let low = StatusBarModel {
        low_disk: true,
        ..m.clone()
    };
    assert_eq!(status_flag(&low).unwrap().text, "free up space ·");
    let mut privacy = ALL_ON;
    privacy[0].privacy_failed = true;
    let retry = StatusBarModel {
        runtime: Some(privacy),
        ..m.clone()
    };
    assert_eq!(status_flag(&retry).unwrap().text, "privacy filter failed ·");
    // Plenty free: no flag.
    assert_eq!(status_flag(&recording(ALL_ON, health())), None);
}

#[test]
fn formats_match_the_svelte_helpers() {
    assert_eq!(format_tracked(16_860_000), "4h 41m");
    assert_eq!(format_tracked(12 * 60_000 + 59_999), "12m");
    assert_eq!(format_clock(5_947_000, true), "01:39:07");
    assert_eq!(format_clock(12_000, false), "00:12");
    assert_eq!(format_clock(3_600_000, false), "01:00:00");
    assert_eq!(format_bytes(2_360_000_000), "2.4 GB");
    assert_eq!(format_bytes(999), "999 B");
    assert_eq!(format_bytes(1_100_000_000), "1.1 GB");
}

#[test]
fn stopped_reason_is_history_after_a_newer_session() {
    let row = NotificationRow {
        id: "capture_disk_full_stopped".into(),
        severity: Severity::Error,
        title: String::new(),
        message: String::new(),
        created_at_ms: 100,
        action: None,
        kind: Default::default(),
    };
    assert_eq!(
        stopped_reason_from(std::slice::from_ref(&row), None).as_deref(),
        Some("disk full")
    );
    assert_eq!(
        stopped_reason_from(std::slice::from_ref(&row), Some(99)).as_deref(),
        Some("disk full")
    );
    assert_eq!(stopped_reason_from(&[row], Some(100)), None);
}
