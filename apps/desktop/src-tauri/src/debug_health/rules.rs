//! The 12 health rows as pure rules over [`DebugHealthInputs`]: no state, no
//! I/O, so every row is table-testable.
//!
//! Severity stays conservative: transient liveness (low disk, display asleep,
//! a provider that can't be reached) is `Warn` or `Info`, never `Error`
//! ([ADR 0021], [ADR 0048]). `Error` is kept for "gave up": a privacy filter
//! that needs a restart, jobs that failed, a model that won't load, a rejected
//! sign-in, a failed derivation run.
//!
//! [ADR 0021]: ../../../../../docs/adr/0021-recover-from-display-unavailable-as-transient-liveness.md
//! [ADR 0048]: ../../../../../docs/adr/0048-cloud-transcription-errors-are-transient-liveness-not-job-failures.md

use crate::native_capture::capture_health::{
    CaptureSourceHealth, CaptureSourceState, SuspensionKindDto, SystemAudioTapDto,
    TapRebuildReasonDto,
};

use super::{
    CliInputs, DebugFeature as F, DebugHealthInputs, DebugSeverity as S, FeatureHealthDto,
};

fn row(
    feature: F,
    severity: S,
    reason: impl Into<String>,
    metric: Option<String>,
) -> FeatureHealthDto {
    FeatureHealthDto {
        feature,
        severity,
        reason: reason.into(),
        metric,
        metric_at_ms: None,
    }
}

fn at(mut dto: FeatureHealthDto, at_ms: Option<i64>) -> FeatureHealthDto {
    dto.metric_at_ms = at_ms;
    dto
}

fn plural(n: impl Into<i64>, one: &str, many: &str) -> String {
    let n = n.into();
    format!("{} {}", thousands(n), if n == 1 { one } else { many })
}

fn thousands(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    if n < 0 {
        out.insert(0, '-');
    }
    out
}

/// "182 GB", "4.2 GB". Binary units, so the 1 GiB reserve floor reads "1 GB".
fn gb(bytes: u64) -> String {
    let gb = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    if gb >= 10.0 {
        format!("{gb:.0} GB")
    } else {
        format!("{gb:.1} GB").replace(".0 GB", " GB")
    }
}

/// "−34 dB", with a real minus sign.
fn db(level: f64) -> String {
    let rounded = level.round() as i64;
    if rounded < 0 {
        format!("\u{2212}{} dB", -rounded)
    } else {
        format!("{rounded} dB")
    }
}

/// "2h 14m" / "14m" (the verdict line's `formatRunning`).
fn running(ms: i64) -> String {
    let minutes = ms.max(0) / 60_000;
    if minutes >= 60 {
        format!("{}h {}m", minutes / 60, minutes % 60)
    } else {
        format!("{minutes}m")
    }
}

/// "just now" / "6 min ago" / "3 h ago" / "2 d ago".
fn ago(ms: i64) -> String {
    let minutes = ms.max(0) / 60_000;
    match minutes {
        0 => "just now".to_string(),
        1..=59 => format!("{minutes} min ago"),
        60..=1439 => format!("{} h ago", minutes / 60),
        _ => format!("{} d ago", minutes / 1440),
    }
}

fn ordinal(n: u64) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or("").trim()
}

fn low_disk(inputs: &DebugHealthInputs) -> bool {
    inputs
        .capture
        .suspension
        .as_ref()
        .is_some_and(|s| s.kind == SuspensionKindDto::LowDisk)
}

// ── Capture ────────────────────────────────────────────────────────────────

fn capture_off(
    feature: F,
    recording: bool,
    source: &CaptureSourceHealth,
) -> Option<FeatureHealthDto> {
    let why = if !recording {
        "Not recording"
    } else if !source.requested {
        "Turned off"
    } else {
        return None;
    };
    Some(row(feature, S::Off, why, Some("off".into())))
}

fn paused(feature: F, source: &CaptureSourceHealth) -> FeatureHealthDto {
    let why = if source.reason.as_deref() == Some("user_paused") {
        "Paused by you"
    } else {
        "Paused while you\u{2019}re away"
    };
    row(feature, S::Info, why, None)
}

fn screen(inputs: &DebugHealthInputs) -> FeatureHealthDto {
    let capture = &inputs.capture;
    let source = &capture.screen;
    if let Some(off) = capture_off(F::Screen, capture.recording, source) {
        return off;
    }
    if let Some(suspension) = capture.suspension.as_ref() {
        return match (suspension.kind, suspension.restart_required) {
            (SuspensionKindDto::LowDisk, _) => {
                row(F::Screen, S::Warn, "Paused, disk almost full", None)
            }
            (SuspensionKindDto::DisplayUnavailable, _) => {
                let since = suspension
                    .since_ms
                    .map(|since| format!("{} min", (inputs.now_ms - since as i64).max(0) / 60_000));
                row(F::Screen, S::Info, "Waiting, display asleep", since)
            }
            (SuspensionKindDto::PrivacyFilter, true) => row(
                F::Screen,
                S::Error,
                "Privacy filter failed, restart recording",
                None,
            ),
            (SuspensionKindDto::PrivacyFilter, false) => row(
                F::Screen,
                S::Warn,
                "Privacy filter couldn\u{2019}t be applied, retrying",
                None,
            ),
        };
    }
    if source.state == CaptureSourceState::Paused {
        return paused(F::Screen, source);
    }
    // Privacy folds into Screen: apps that should be hidden but aren't.
    if inputs.privacy_unhidden_app_count > 0 {
        return row(
            F::Screen,
            S::Warn,
            format!(
                "{} not hidden right now",
                plural(
                    inputs.privacy_unhidden_app_count as i64,
                    "excluded app is",
                    "excluded apps are"
                )
            ),
            None,
        );
    }
    if source.state == CaptureSourceState::Failed {
        return row(F::Screen, S::Warn, "Not capturing right now", None);
    }
    let duration = capture
        .started_at_ms
        .map(|started| running(inputs.now_ms - started as i64));
    let why = match source.device_name.as_deref() {
        Some(device) => format!("Recording, {device}"),
        None => "Recording".to_string(),
    };
    row(F::Screen, S::Ok, why, duration)
}

/// Bluetooth call (HFP) audio runs at 16 kHz or less; music (A2DP) above it.
const CALL_RATE_HZ: u32 = 16_000;

/// The rates out of a sample-rate rebuild's detail ("48000 → 16000 Hz").
fn rate_change(detail: Option<&str>) -> Option<(u32, u32)> {
    let (before, after) = detail?.strip_suffix(" Hz")?.split_once(" → ")?;
    Some((before.parse().ok()?, after.parse().ok()?))
}

fn rebuild_reason(reason: TapRebuildReasonDto, detail: Option<&str>) -> &'static str {
    match reason {
        TapRebuildReasonDto::SampleRateChanged => match rate_change(detail) {
            Some((before, after)) if after <= CALL_RATE_HZ && before > CALL_RATE_HZ => {
                "headset switched to call quality"
            }
            Some((before, after)) if before <= CALL_RATE_HZ && after > CALL_RATE_HZ => {
                "headset went back to full quality"
            }
            _ => "a sample rate change",
        },
        TapRebuildReasonDto::DeviceChanged => "output device changed",
        TapRebuildReasonDto::DeviceDied => "output device disappeared",
        TapRebuildReasonDto::ExcludeListMoved => "an excluded app started or quit",
        TapRebuildReasonDto::SilenceWatchdog => "a long silence",
    }
}

fn audio(
    inputs: &DebugHealthInputs,
    feature: F,
    source: &CaptureSourceHealth,
    tap: Option<&SystemAudioTapDto>,
) -> FeatureHealthDto {
    if let Some(off) = capture_off(feature, inputs.capture.recording, source) {
        return off;
    }
    if low_disk(inputs) {
        return row(feature, S::Warn, "Paused, disk almost full", None);
    }
    match source.state {
        CaptureSourceState::Paused => return paused(feature, source),
        CaptureSourceState::Reconnecting => {
            let why = match tap.and_then(|tap| Some((tap.last_reason?, tap.last_detail.as_deref())))
            {
                Some((reason, detail)) => {
                    format!("Rebuilding after {}", rebuild_reason(reason, detail))
                }
                None => "Reconnecting".to_string(),
            };
            let nth = tap
                .map(|tap| tap.rebuilds_today)
                .filter(|n| *n > 0)
                .map(|n| format!("{} today", ordinal(n)));
            return row(feature, S::Warn, why, nth);
        }
        CaptureSourceState::Failed => {
            return row(feature, S::Warn, "Not capturing right now", None)
        }
        _ => {}
    }
    let rate = tap
        .and_then(|tap| tap.sample_rate_hz)
        .map(|hz| format!("{} kHz", (hz as f64 / 1000.0).round()));
    let why = match (source.device_name.as_deref(), rate) {
        (Some(device), Some(rate)) => format!("{device} \u{b7} {rate}"),
        (Some(device), None) => device.to_string(),
        (None, Some(rate)) => format!("Recording \u{b7} {rate}"),
        (None, None) => "Recording".to_string(),
    };
    row(feature, S::Ok, why, source.level_db.map(db))
}

fn disk(inputs: &DebugHealthInputs) -> FeatureHealthDto {
    let disk = &inputs.capture.disk;
    let free = disk.free_bytes.map(gb);
    if low_disk(inputs) {
        return row(F::Disk, S::Warn, "Recording paused, disk almost full", free);
    }
    // Thresholds follow the live session. While not recording they're unknown
    // (the pause point is the floor plus the next segment's estimate), so warn
    // at the status popover's 5 GB instead of naming a number that's wrong.
    match (
        disk.free_bytes,
        disk.pause_threshold_bytes,
        disk.resume_threshold_bytes,
    ) {
        (None, ..) => row(F::Disk, S::Info, "Couldn\u{2019}t read free space", None),
        (Some(bytes), Some(pause), Some(resume)) => {
            let (severity, why) = if bytes < resume {
                (S::Warn, "Getting low")
            } else {
                (S::Ok, "Plenty of room")
            };
            row(
                F::Disk,
                severity,
                format!("{why}, pauses below {}", gb(pause)),
                free,
            )
        }
        (Some(bytes), ..) if bytes < LOW_DISK_WARN_BYTES => row(
            F::Disk,
            S::Warn,
            format!("Getting low, under {} free", gb(LOW_DISK_WARN_BYTES)),
            free,
        ),
        (Some(_), ..) => row(F::Disk, S::Ok, "Plenty of room", free),
    }
}

/// Where the not-recording Disk row warns; matches the status popover's
/// `LOW_DISK_WARN_BYTES` (`lib/shell/status-pop.ts`).
const LOW_DISK_WARN_BYTES: u64 = 5 * 1024 * 1024 * 1024;

// ── Pipeline lanes ─────────────────────────────────────────────────────────

/// A lane's retry reason in plain words. Transient provider errors (ADR 0048)
/// read as "can't be reached"; anything else stays generic (the raw error is
/// the Pipeline tab's job).
fn retry_reason(provider: Option<&str>, last_error: Option<&str>) -> String {
    let line = first_line(last_error.unwrap_or("")).to_lowercase();
    match provider {
        Some(provider) if line.contains("rejected the api key") => {
            format!("{provider} rejected the key, retrying on its own")
        }
        Some(provider)
            if line.contains("temporarily unavailable")
                || line.contains("request failed")
                || line.contains("timed out")
                || line.contains("connect") =>
        {
            format!("{provider} can\u{2019}t be reached, retrying on its own")
        }
        _ => "Retrying on its own".to_string(),
    }
}

/// `noun` is what one job is about ("frame" / "recording"), `verb` what
/// failing it means ("read" / "transcribed").
fn lane(
    inputs: &DebugHealthInputs,
    feature: F,
    enabled: bool,
    processor: &str,
    noun: &str,
    verb: &str,
    provider: Option<&str>,
) -> FeatureHealthDto {
    if !enabled {
        return row(feature, S::Off, "Turned off", Some("off".into()));
    }
    // An absent lane is idle: the query is a `GROUP BY`, so zero jobs = no row.
    let lane = inputs
        .pipeline
        .iter()
        .find(|lane| lane.processor == processor);
    // `retrying` is a subset of `queued`, so `queued` alone is what waits.
    let waiting = lane.map_or(0, |lane| lane.queued);
    let retrying = lane.map_or(0, |lane| lane.retrying);
    let failed = lane.map_or(0, |lane| lane.failed_last_24h);
    let waiting_metric = Some(format!("{} waiting", thousands(waiting)));

    if retrying > 0 {
        let error = lane.and_then(|lane| lane.last_error.as_deref());
        return row(
            feature,
            S::Warn,
            retry_reason(provider, error),
            waiting_metric,
        );
    }
    if failed > 0 {
        return row(
            feature,
            S::Error,
            format!(
                "{} couldn\u{2019}t be {verb} in the last 24 h",
                plural(failed, noun, &format!("{noun}s"))
            ),
            Some(format!("{} failed", thousands(failed))),
        );
    }
    if !inputs.capture.recording && waiting > 0 {
        return row(
            feature,
            S::Info,
            "Finishing the last recording",
            waiting_metric,
        );
    }
    let why = if waiting > 0 {
        "Keeping up"
    } else {
        "Up to date"
    };
    row(feature, S::Ok, why, waiting_metric)
}

fn meaning_index(inputs: &DebugHealthInputs) -> FeatureHealthDto {
    if !inputs.semantic_enabled {
        return row(F::MeaningIndex, S::Off, "Turned off", Some("off".into()));
    }
    if !inputs.semantic_model_installed {
        return row(
            F::MeaningIndex,
            S::Off,
            "No model installed, search is keyword-only",
            Some("off".into()),
        );
    }
    let semantic = &inputs.semantic;
    let behind = Some(format!("{} behind", thousands(semantic.backlog_count)));
    let load_failures = semantic.consecutive_load_failures.unwrap_or(0);
    // The worker gives up at its cap; below it, the next pass retries the load.
    if load_failures >= crate::semantic_search_worker::MAX_CONSECUTIVE_LOAD_FAILURES {
        return row(
            F::MeaningIndex,
            S::Error,
            "Model won\u{2019}t load, search is keyword-only",
            behind,
        );
    }
    if load_failures > 0 || semantic.last_load_error.is_some() {
        return row(
            F::MeaningIndex,
            S::Warn,
            "Model didn\u{2019}t load, retrying",
            behind,
        );
    }
    let quarantined = semantic.quarantined_count.unwrap_or(0);
    if quarantined > 0 {
        return row(
            F::MeaningIndex,
            S::Warn,
            format!(
                "{} couldn\u{2019}t be indexed since the app started",
                plural(quarantined as i64, "passage", "passages")
            ),
            behind,
        );
    }
    if !inputs.capture.recording && semantic.backlog_count > 0 {
        return row(
            F::MeaningIndex,
            S::Info,
            "Indexing what was just read",
            behind,
        );
    }
    row(
        F::MeaningIndex,
        S::Ok,
        format!(
            "{} searchable",
            plural(semantic.vector_count, "passage", "passages")
        ),
        behind,
    )
}

// ── AI ─────────────────────────────────────────────────────────────────────

/// One engine reason code in plain words. A provider that didn't answer is
/// `Warn` and never says "sign in again" (ADR 0058); only a rejected grant is
/// `Error`.
fn ai_reason(reason: &str, provider: &str) -> (S, String) {
    let not_set_up = |what: &str| (S::Warn, format!("Not set up, {what}"));
    if reason.starts_with("needs_reconnect:") {
        (S::Error, "Sign-in expired, reconnect in Settings".into())
    } else if reason.starts_with("provider_unreachable:") || reason == "local_endpoint_unreachable"
    {
        (S::Warn, format!("{provider} can\u{2019}t be reached"))
    } else if reason == "vault_denied" {
        (S::Warn, "Saved keys are locked, see Startup".into())
    } else if reason == "no_providers" {
        not_set_up("no provider connected")
    } else if reason == "no_default_model" {
        not_set_up("no default model chosen")
    } else if reason.starts_with("provider_not_connected:") {
        not_set_up("the default model\u{2019}s provider is gone")
    } else if reason.starts_with("no_provider_key:") {
        not_set_up("no API key saved")
    } else if reason == "no_base_url" {
        not_set_up("no server address")
    } else {
        (S::Warn, "Not ready, check AI settings".into())
    }
}

fn ai_engine(inputs: &DebugHealthInputs) -> FeatureHealthDto {
    let ai = &inputs.ai;
    if !ai.enabled {
        return row(F::AiEngine, S::Off, "Turned off", Some("off".into()));
    }
    let provider = ai.provider.unwrap_or("The provider");
    let (severity, why) = match ai.status.as_ref() {
        None if inputs.startup_panicked => (S::Error, STARTUP_CRASHED.to_string()),
        None => (S::Info, "Waiting for startup checks".to_string()),
        Some(Ok(())) => (S::Ok, format!("{provider}, ready")),
        Some(Err(reason)) => ai_reason(reason, provider),
    };
    at(row(F::AiEngine, severity, why, None), ai.last_turn_at_ms)
}

fn user_context(inputs: &DebugHealthInputs) -> FeatureHealthDto {
    let uc = &inputs.user_context;
    if !uc.enabled {
        return row(F::UserContext, S::Off, "Turned off", Some("off".into()));
    }
    let engine_down = !inputs.ai.enabled || matches!(inputs.ai.status, Some(Err(_)));
    let dto = if engine_down {
        row(
            F::UserContext,
            S::Warn,
            "On, but the AI engine isn\u{2019}t ready",
            None,
        )
    } else if uc.last_run_failed {
        row(
            F::UserContext,
            S::Error,
            "The last run failed, see the AI tab",
            None,
        )
    } else {
        let why = match uc.last_derived_at_ms {
            None => "Nothing derived yet".to_string(),
            Some(derived) => {
                let digest = match uc.digest_fresh {
                    Some(true) => ", digest fresh",
                    Some(false) => ", digest stale",
                    None => "",
                };
                format!("Derived {}{digest}", ago(inputs.now_ms - derived))
            }
        };
        row(F::UserContext, S::Ok, why, None)
    };
    at(dto, uc.last_derived_at_ms)
}

fn cli_access(inputs: &DebugHealthInputs) -> FeatureHealthDto {
    match &inputs.cli {
        CliInputs::NotInstalled => row(F::CliAccess, S::Off, "Not set up", Some("off".into())),
        CliInputs::Unreadable => row(
            F::CliAccess,
            S::Warn,
            "Couldn\u{2019}t read the permission file",
            None,
        ),
        // A refusal is the user's choice, never a warning.
        CliInputs::Ready {
            allowed,
            blocked,
            today,
        } => {
            let blocked = if *blocked == 0 {
                "none blocked".to_string()
            } else {
                format!("{blocked} blocked")
            };
            row(
                F::CliAccess,
                S::Ok,
                format!(
                    "{} allowed, {blocked}",
                    plural(*allowed as i64, "tool", "tools")
                ),
                Some(format!("{today} today")),
            )
        }
    }
}

fn startup_pass_label(code: &str) -> &'static str {
    match code {
        "vault_unlock" => "Unlocking saved keys",
        "startup_maintenance" => "Database maintenance",
        "workspace_repair" => "Workspace cleanup",
        "ocr_disabled_reconcile" => "Clearing queued reading",
        _ => "A startup check",
    }
}

const STARTUP_CRASHED: &str = "Startup checks crashed, restart Mnema";

fn startup(inputs: &DebugHealthInputs) -> FeatureHealthDto {
    let Some(report) = inputs.startup.as_ref() else {
        if inputs.startup_panicked {
            return row(F::Startup, S::Error, STARTUP_CRASHED, None);
        }
        return row(F::Startup, S::Info, "Running startup checks", None);
    };
    let duration = Some(format!("{:.1} s", report.duration_ms as f64 / 1000.0));
    if let Some(code) = report.errors.first() {
        return row(
            F::Startup,
            S::Warn,
            format!("{} failed, see Logs", startup_pass_label(code)),
            duration,
        );
    }
    let mut repaired = Vec::new();
    if let Some(m) = report.maintenance.as_ref() {
        let jobs =
            m.orphaned_app_jobs + m.orphaned_processing_requeued + m.orphaned_processing_failed;
        if jobs > 0 {
            repaired.push(format!("{} reclaimed", plural(jobs as i64, "job", "jobs")));
        }
        let batches = m.closed_batches_reconciled + m.open_batches_reconciled;
        if batches > 0 {
            repaired.push(format!(
                "{} closed",
                plural(batches as i64, "batch", "batches")
            ));
        }
    }
    if let Some(workspaces) = report.workspaces_removed.filter(|n| *n > 0) {
        repaired.push(format!(
            "{} cleaned",
            plural(workspaces as i64, "workspace", "workspaces")
        ));
    }
    let why = if repaired.is_empty() {
        "Nothing needed repair".to_string()
    } else {
        format!("Repaired: {}", repaired.join(", "))
    };
    row(F::Startup, S::Ok, why, duration)
}

/// The whole rollup, always 12 rows in the mockup's order.
pub fn roll_up(inputs: &DebugHealthInputs) -> Vec<FeatureHealthDto> {
    let capture = &inputs.capture;
    vec![
        screen(inputs),
        audio(inputs, F::Microphone, &capture.microphone, None),
        audio(
            inputs,
            F::SystemAudio,
            &capture.system_audio,
            capture.system_audio_tap.as_ref(),
        ),
        lane(
            inputs,
            F::Reading,
            inputs.ocr_enabled,
            ::app_infra::OCR_PROCESSOR,
            "frame",
            "read",
            None,
        ),
        lane(
            inputs,
            F::Transcription,
            inputs.transcription_enabled,
            ::app_infra::AUDIO_TRANSCRIPTION_PROCESSOR,
            "recording",
            "transcribed",
            Some(inputs.transcription_provider),
        ),
        lane(
            inputs,
            F::Speakers,
            inputs.speakers_enabled,
            ::app_infra::SPEAKER_ANALYSIS_PROCESSOR,
            "recording",
            "split by speaker",
            None,
        ),
        meaning_index(inputs),
        ai_engine(inputs),
        user_context(inputs),
        disk(inputs),
        cli_access(inputs),
        startup(inputs),
    ]
}

#[cfg(test)]
#[path = "rules_tests.rs"]
mod tests;
