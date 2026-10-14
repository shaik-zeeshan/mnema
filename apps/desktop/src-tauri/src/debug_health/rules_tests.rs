use super::*;
use crate::app_infra::StartupReport;
use crate::debug_health::{AiEngineInputs, UserContextInputs};
use crate::debug_status::SemanticIndexStatusDto;
use crate::native_capture::capture_health::{
    CaptureHealthDto, CaptureSuspensionDto, DiskHealthDto,
};

const NOW: i64 = 1_800_000_000_000;
const GIB: u64 = 1024 * 1024 * 1024;

fn source(state: CaptureSourceState, level_db: Option<f64>) -> CaptureSourceHealth {
    CaptureSourceHealth {
        requested: true,
        state,
        reason: None,
        level_db,
        idle_ms: None,
        device_name: None,
    }
}

fn lane_status(processor: &str, queued: i64) -> ::app_infra::ProcessorPipelineStatus {
    ::app_infra::ProcessorPipelineStatus {
        processor: processor.to_string(),
        queued,
        retrying: 0,
        running: 0,
        completed: 5,
        failed: 0,
        failed_last_24h: 0,
        average_completed_seconds_last_24h: Some(1.0),
        last_error: None,
    }
}

/// The mockup's `ok` scenario: recording, everything healthy.
fn ok() -> DebugHealthInputs {
    DebugHealthInputs {
        now_ms: NOW,
        capture: CaptureHealthDto {
            recording: true,
            started_at_ms: Some((NOW - (2 * 60 + 14) * 60_000) as u64),
            screen: source(CaptureSourceState::Recording, None),
            microphone: source(CaptureSourceState::Recording, Some(-34.2)),
            system_audio: source(CaptureSourceState::Recording, Some(-22.0)),
            suspension: None,
            system_audio_tap: Some(SystemAudioTapDto {
                rebuilds_today: 0,
                last_reason: None,
                last_rebuild_at_ms: None,
                last_detail: None,
                next_silence_check_in_ms: None,
                watchdog_delay_ms: Some(30_000),
                sample_rate_hz: Some(48_000),
            }),
            disk: DiskHealthDto {
                free_bytes: Some(182 * GIB),
                reserve_floor_bytes: GIB,
                pause_threshold_bytes: Some(5 * GIB),
                resume_threshold_bytes: Some(9 * GIB),
            },
        },
        privacy_unhidden_app_count: 0,
        pipeline: vec![
            lane_status(::app_infra::OCR_PROCESSOR, 4),
            lane_status(::app_infra::AUDIO_TRANSCRIPTION_PROCESSOR, 1),
            lane_status(::app_infra::SPEAKER_ANALYSIS_PROCESSOR, 0),
        ],
        transcription_provider: "Deepgram",
        ocr_enabled: true,
        transcription_enabled: true,
        speakers_enabled: true,
        semantic_enabled: true,
        semantic_model_installed: true,
        semantic: SemanticIndexStatusDto {
            vector_count: 12_480,
            backlog_count: 0,
            live_dimension: Some(768),
            model_loaded: None,
            consecutive_load_failures: None,
            quarantined_count: None,
            last_load_error: None,
        },
        ai: AiEngineInputs {
            enabled: true,
            provider: Some("Anthropic"),
            status: Some(Ok(())),
            last_turn_at_ms: Some(NOW - 60_000),
        },
        user_context: UserContextInputs {
            enabled: true,
            last_run_failed: false,
            last_derived_at_ms: Some(NOW - 6 * 60_000),
            digest_fresh: Some(true),
        },
        cli: CliInputs::Ready {
            allowed: 2,
            blocked: 0,
            today: 14,
        },
        startup: Some(StartupReport {
            duration_ms: 1_800,
            vault_unlocked: true,
            maintenance: Some(Default::default()),
            workspaces_removed: Some(0),
            ..Default::default()
        }),
        startup_panicked: false,
    }
}

fn get(report: &[FeatureHealthDto], feature: F) -> &FeatureHealthDto {
    report
        .iter()
        .find(|row| row.feature == feature)
        .unwrap_or_else(|| panic!("{feature:?} should be in the rollup"))
}

fn sev(report: &[FeatureHealthDto], feature: F) -> S {
    get(report, feature).severity
}

#[test]
fn twelve_rows_in_the_mockup_order() {
    let order: Vec<F> = roll_up(&ok()).iter().map(|row| row.feature).collect();
    assert_eq!(
        order,
        [
            F::Screen,
            F::Microphone,
            F::SystemAudio,
            F::Reading,
            F::Transcription,
            F::Speakers,
            F::MeaningIndex,
            F::AiEngine,
            F::UserContext,
            F::Disk,
            F::CliAccess,
            F::Startup,
        ]
    );
}

#[test]
fn scenario_ok_is_ok_everywhere_with_the_mockup_copy() {
    let report = roll_up(&ok());
    assert!(
        report.iter().all(|row| row.severity == S::Ok),
        "{report:#?}"
    );
    let check = |feature, why: &str, metric: Option<&str>| {
        let row = get(&report, feature);
        assert_eq!(row.reason, why, "{feature:?}");
        assert_eq!(row.metric.as_deref(), metric, "{feature:?}");
    };
    check(F::Screen, "Recording", Some("2h 14m"));
    check(F::Microphone, "Recording", Some("\u{2212}34 dB"));
    check(
        F::SystemAudio,
        "Recording \u{b7} 48 kHz",
        Some("\u{2212}22 dB"),
    );
    check(F::Reading, "Keeping up", Some("4 waiting"));
    check(F::Speakers, "Up to date", Some("0 waiting"));
    check(
        F::MeaningIndex,
        "12,480 passages searchable",
        Some("0 behind"),
    );
    check(F::AiEngine, "Anthropic, ready", None);
    check(F::UserContext, "Derived 6 min ago, digest fresh", None);
    check(F::Disk, "Plenty of room, pauses below 5 GB", Some("182 GB"));
    check(
        F::CliAccess,
        "2 tools allowed, none blocked",
        Some("14 today"),
    );
    check(F::Startup, "Nothing needed repair", Some("1.8 s"));
    assert_eq!(get(&report, F::AiEngine).metric_at_ms, Some(NOW - 60_000));
    assert_eq!(
        get(&report, F::UserContext).metric_at_ms,
        Some(NOW - 6 * 60_000)
    );
}

#[test]
fn scenario_issues() {
    let mut inputs = ok();
    inputs.capture.system_audio.state = CaptureSourceState::Reconnecting;
    let tap = inputs.capture.system_audio_tap.as_mut().unwrap();
    tap.rebuilds_today = 3;
    tap.last_reason = Some(TapRebuildReasonDto::SampleRateChanged);
    tap.last_detail = Some("48000 → 16000 Hz".into());
    inputs.pipeline[1].queued = 14;
    inputs.pipeline[1].retrying = 12;
    inputs.pipeline[1].last_error = Some(
        "audio transcription provider is temporarily unavailable: Deepgram request failed: dns\ntrace"
            .into(),
    );
    inputs.pipeline[0].failed = 1;
    inputs.pipeline[0].failed_last_24h = 1;

    let report = roll_up(&inputs);
    let sys = get(&report, F::SystemAudio);
    assert_eq!(sys.severity, S::Warn);
    assert_eq!(
        sys.reason,
        "Rebuilding after headset switched to call quality"
    );
    assert_eq!(sys.metric.as_deref(), Some("3rd today"));

    let transcription = get(&report, F::Transcription);
    assert_eq!(
        transcription.severity,
        S::Warn,
        "transient is never an error"
    );
    assert_eq!(
        transcription.reason,
        "Deepgram can\u{2019}t be reached, retrying on its own"
    );
    assert_eq!(transcription.metric.as_deref(), Some("14 waiting"));

    let reading = get(&report, F::Reading);
    assert_eq!(reading.severity, S::Error);
    assert_eq!(
        reading.reason,
        "1 frame couldn\u{2019}t be read in the last 24 h"
    );
    assert_eq!(reading.metric.as_deref(), Some("1 failed"));
}

#[test]
fn scenario_asleep() {
    let mut inputs = ok();
    inputs.capture.screen.state = CaptureSourceState::Waiting;
    inputs.capture.suspension = Some(CaptureSuspensionDto {
        kind: SuspensionKindDto::DisplayUnavailable,
        reason: "capture_display_unavailable".into(),
        recovery_attempts: 0,
        restart_required: false,
        since_ms: Some((NOW - 6 * 60_000) as u64),
        last_error_message: None,
    });

    let report = roll_up(&inputs);
    let screen = get(&report, F::Screen);
    assert_eq!(screen.severity, S::Info);
    assert_eq!(screen.reason, "Waiting, display asleep");
    assert_eq!(screen.metric.as_deref(), Some("6 min"));
    assert_eq!(sev(&report, F::Microphone), S::Ok);
    assert_eq!(sev(&report, F::SystemAudio), S::Ok);
}

#[test]
fn scenario_off() {
    let mut inputs = ok();
    inputs.capture.recording = false;
    inputs.capture.system_audio_tap = None;
    inputs.pipeline[0].queued = 38;

    let report = roll_up(&inputs);
    for feature in [F::Screen, F::Microphone, F::SystemAudio] {
        let row = get(&report, feature);
        assert_eq!(row.severity, S::Off);
        assert_eq!(row.reason, "Not recording");
    }
    let reading = get(&report, F::Reading);
    assert_eq!(reading.severity, S::Info);
    assert_eq!(reading.reason, "Finishing the last recording");
    assert_eq!(reading.metric.as_deref(), Some("38 waiting"));
}

#[test]
fn a_chatgpt_provider_that_cant_be_reached_warns_never_errors() {
    let mut inputs = ok();
    inputs.ai.provider = Some("ChatGPT");
    inputs.ai.status = Some(Err("provider_unreachable:chatgpt".into()));
    let report = roll_up(&inputs);
    let ai = get(&report, F::AiEngine);
    assert_eq!(ai.severity, S::Warn);
    assert_eq!(ai.reason, "ChatGPT can\u{2019}t be reached");
    assert!(!ai.reason.to_lowercase().contains("sign in"));

    inputs.ai.status = Some(Err("needs_reconnect:chatgpt".into()));
    assert_eq!(sev(&roll_up(&inputs), F::AiEngine), S::Error);
}

#[test]
fn an_absent_lane_is_idle_not_broken() {
    let mut inputs = ok();
    inputs.pipeline.clear();
    let report = roll_up(&inputs);
    for feature in [F::Reading, F::Transcription, F::Speakers] {
        assert_eq!(sev(&report, feature), S::Ok);
    }
}

#[test]
fn turned_off_processing_lanes_read_off_not_failed() {
    let mut inputs = ok();
    // Turning OCR off fails every queued OCR job; that's the user's choice.
    let mut ocr = lane_status(::app_infra::OCR_PROCESSOR, 0);
    ocr.failed_last_24h = 1_234;
    inputs.pipeline = vec![ocr];
    inputs.ocr_enabled = false;
    inputs.speakers_enabled = false;
    let report = roll_up(&inputs);
    assert_eq!(sev(&report, F::Reading), S::Off);
    assert_eq!(sev(&report, F::Speakers), S::Off);
    assert_eq!(sev(&report, F::Transcription), S::Ok);
}

#[test]
fn unhidden_apps_are_decided_minus_applied() {
    use crate::debug_health::unhidden_app_count;
    let ids = |list: &[&str]| list.iter().map(|id| id.to_string()).collect::<Vec<_>>();
    // A filter that failed to apply leaves the applied list behind the decision.
    assert_eq!(unhidden_app_count(&ids(&["a", "b"]), &ids(&[])), 2);
    assert_eq!(unhidden_app_count(&ids(&["a", "b"]), &ids(&["a"])), 1);
    assert_eq!(unhidden_app_count(&ids(&["a"]), &ids(&["a", "b"])), 0);
}

#[test]
fn privacy_folds_into_screen() {
    let mut inputs = ok();
    inputs.privacy_unhidden_app_count = 2;
    assert_eq!(sev(&roll_up(&inputs), F::Screen), S::Warn);
    inputs.privacy_unhidden_app_count = 0;
    assert_eq!(sev(&roll_up(&inputs), F::Screen), S::Ok);

    inputs.capture.suspension = Some(CaptureSuspensionDto {
        kind: SuspensionKindDto::PrivacyFilter,
        reason: "privacy_filter".into(),
        recovery_attempts: 3,
        restart_required: true,
        since_ms: None,
        last_error_message: None,
    });
    assert_eq!(sev(&roll_up(&inputs), F::Screen), S::Error);
}

#[test]
fn low_disk_warns_capture_and_disk() {
    let mut inputs = ok();
    inputs.capture.suspension = Some(CaptureSuspensionDto {
        kind: SuspensionKindDto::LowDisk,
        reason: "low_disk".into(),
        recovery_attempts: 0,
        restart_required: false,
        since_ms: None,
        last_error_message: None,
    });
    let report = roll_up(&inputs);
    for feature in [F::Screen, F::Microphone, F::SystemAudio, F::Disk] {
        assert_eq!(sev(&report, feature), S::Warn, "{feature:?}");
    }

    let mut inputs = ok();
    inputs.capture.disk.free_bytes = Some(8 * GIB);
    assert_eq!(sev(&roll_up(&inputs), F::Disk), S::Warn);
}

#[test]
fn disk_while_stopped_warns_at_the_popovers_5_gb() {
    // Not recording: no live thresholds. 1.1 GB free used to read
    // "Plenty of room, pauses below 1 GB" though a start would be refused.
    let mut inputs = ok();
    inputs.capture.recording = false;
    inputs.capture.disk.pause_threshold_bytes = None;
    inputs.capture.disk.resume_threshold_bytes = None;
    inputs.capture.disk.free_bytes = Some(GIB + GIB / 10);
    let report = roll_up(&inputs);
    assert_eq!(sev(&report, F::Disk), S::Warn);
    assert!(!get(&report, F::Disk).reason.contains("1 GB"));
    inputs.capture.disk.free_bytes = Some(6 * GIB);
    assert_eq!(sev(&roll_up(&inputs), F::Disk), S::Ok);
}

#[test]
fn meaning_index_rules() {
    let mut inputs = ok();
    inputs.semantic.quarantined_count = Some(3);
    assert_eq!(sev(&roll_up(&inputs), F::MeaningIndex), S::Warn);
    // Below the worker's cap the load is retried on its own.
    inputs.semantic.consecutive_load_failures = Some(1);
    assert_eq!(sev(&roll_up(&inputs), F::MeaningIndex), S::Warn);
    inputs.semantic.consecutive_load_failures =
        Some(crate::semantic_search_worker::MAX_CONSECUTIVE_LOAD_FAILURES);
    assert_eq!(sev(&roll_up(&inputs), F::MeaningIndex), S::Error);
    inputs.semantic_enabled = false;
    assert_eq!(sev(&roll_up(&inputs), F::MeaningIndex), S::Off);
}

#[test]
fn meaning_index_without_a_model_is_off_not_healthy() {
    // The default install: on, no model, the backlog only grows.
    let mut inputs = ok();
    inputs.semantic_model_installed = false;
    inputs.semantic.backlog_count = 5_000;
    assert_eq!(sev(&roll_up(&inputs), F::MeaningIndex), S::Off);
    inputs.capture.recording = false;
    assert_eq!(sev(&roll_up(&inputs), F::MeaningIndex), S::Off);
}

#[test]
fn startup_that_crashed_is_an_error_not_a_wait() {
    let mut inputs = ok();
    inputs.startup = None;
    inputs.ai.status = None;
    inputs.startup_panicked = true;
    let report = roll_up(&inputs);
    assert_eq!(sev(&report, F::Startup), S::Error);
    assert_eq!(sev(&report, F::AiEngine), S::Error);
}

#[test]
fn user_context_and_startup_rules() {
    let mut inputs = ok();
    inputs.user_context.last_run_failed = true;
    assert_eq!(sev(&roll_up(&inputs), F::UserContext), S::Error);
    inputs.ai.enabled = false;
    let report = roll_up(&inputs);
    assert_eq!(sev(&report, F::AiEngine), S::Off);
    assert_eq!(sev(&report, F::UserContext), S::Warn);

    let mut inputs = ok();
    inputs.startup = None;
    assert_eq!(sev(&roll_up(&inputs), F::Startup), S::Info);
    inputs.startup = Some(StartupReport {
        errors: vec!["vault_unlock"],
        ..Default::default()
    });
    let report = roll_up(&inputs);
    assert_eq!(sev(&report, F::Startup), S::Warn);
    assert_eq!(
        get(&report, F::Startup).reason,
        "Unlocking saved keys failed, see Logs"
    );

    let mut inputs = ok();
    inputs.cli = CliInputs::NotInstalled;
    assert_eq!(sev(&roll_up(&inputs), F::CliAccess), S::Off);
}

#[test]
fn small_formatters() {
    assert_eq!(thousands(12_480), "12,480");
    assert_eq!(thousands(999), "999");
    assert_eq!(ordinal(1), "1st");
    assert_eq!(ordinal(12), "12th");
    assert_eq!(ordinal(23), "23rd");
    assert_eq!(gb(GIB), "1 GB");
    assert_eq!(gb(GIB * 3 / 2), "1.5 GB");
}

#[test]
fn the_wire_shape_is_camel_case() {
    let json = serde_json::to_value(get(&roll_up(&ok()), F::AiEngine)).unwrap();
    assert_eq!(json["feature"], "aiEngine");
    assert_eq!(json["severity"], "ok");
    assert!(json["metricAtMs"].is_i64());
    assert!(json["metric"].is_null());
}

#[test]
fn a_sample_rate_rebuild_reads_the_direction_of_the_switch() {
    let mut inputs = ok();
    inputs.capture.system_audio.state = CaptureSourceState::Reconnecting;
    let tap = inputs.capture.system_audio_tap.as_mut().unwrap();
    tap.last_reason = Some(TapRebuildReasonDto::SampleRateChanged);
    for (detail, why) in [
        (
            Some("16000 → 48000 Hz"),
            "Rebuilding after headset went back to full quality",
        ),
        (
            Some("48000 → 44100 Hz"),
            "Rebuilding after a sample rate change",
        ),
        (None, "Rebuilding after a sample rate change"),
    ] {
        inputs
            .capture
            .system_audio_tap
            .as_mut()
            .unwrap()
            .last_detail = detail.map(Into::into);
        assert_eq!(get(&roll_up(&inputs), F::SystemAudio).reason, why);
    }
}
