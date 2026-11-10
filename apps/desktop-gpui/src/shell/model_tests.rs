use super::*;

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
            elapsed_clock: true
        }
    );
    // Any idle source sets the flag, but only all-requested-idle is "PAUSED · idle".
    m.inactivity_paused = true;
    assert_eq!(rec_view(&m).kind, RecMod::Running);
    m.runtime = Some([live(true, true), live(true, true), live(false, false)]);
    assert_eq!(rec_view(&m).label, "PAUSED · idle");
    m.user_paused = true;
    assert_eq!(rec_view(&m).label, "PAUSED");
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
    m.start_error = Some("boom".into());
    assert_eq!(status_flag(&m).unwrap().action, "Retry");
    let m = StatusBarModel {
        running: true,
        low_disk: true,
        ..Default::default()
    };
    assert_eq!(status_flag(&m).unwrap().text, "free up space ·");
}

#[test]
fn engine_pill_only_when_unhealthy() {
    use serde_json::json;
    let healthy =
        json!({"enabled": true, "hasProviders": true, "configured": true, "reason": null});
    assert_eq!(engine_flag(&healthy), None);
    let never =
        json!({"enabled": false, "hasProviders": false, "configured": false, "reason": "disabled"});
    assert_eq!(engine_flag(&never).unwrap().label, "not set up");
    let offline =
        json!({"hasProviders": true, "configured": true, "reason": "provider_unreachable:openai"});
    assert_eq!(
        engine_flag(&offline),
        Some(EngineFlag {
            label: "offline",
            error: true
        })
    );
    let reconnect = json!({"hasProviders": true, "reason": "needs_reconnect:chatgpt"});
    assert_eq!(engine_flag(&reconnect).unwrap().label, "needs reconnect");
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
