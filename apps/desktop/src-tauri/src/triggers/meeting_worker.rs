//! The meeting detector worker + firing wiring (issue #177, ADR 0057).
//!
//! Drives the pure [`MeetingDetector`](super::meeting::MeetingDetector) from
//! Core Audio mic snapshots on its own poll loop, and turns its Ended events
//! into firings: per-trigger floor → the shared cooldown/provider-gate
//! decision ([`super::firing_decision`]) → claim → Readiness Wait
//! ([`super::readiness`]) → the shared run path ([`super::run`]) or a Skipped
//! Run ledger row.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use ::app_infra::trigger_firings::TriggerFiringOutcome;
use ::app_infra::{
    ProcessingJobStatus, ProcessingSubject, AUDIO_TRANSCRIPTION_PROCESSOR,
    SPEAKER_ANALYSIS_PROCESSOR,
};

use super::meeting::{
    conferencing_display_name, conferencing_holders, trigger_floor_ms, EndedHold, MeetingDetector,
    MeetingEvent, MEETING_LOG_PREFIX,
};
use super::meeting_browser::{BrowserMeetingTracker, MeetingUrlEvidence};
use super::readiness::{self, ReadinessOutcome, ReadinessSnapshot};
use super::run::MeetingFiringContext;
use super::{FiringDecision, TriggerCondition, TriggerDefinition};
use crate::app_infra::{shutdown_aware_sleep, AppInfraState, BackgroundWorkersState};
use crate::user_context::worker::now_ms;

/// Poll cadence while a conferencing hold is being tracked: fine enough that
/// the 2-minute grace and 5-minute floor are judged within seconds. The 30s
/// triggers tick is too coarse for that, so the detector runs its own loop.
const TRACKING_POLL: Duration = Duration::from_secs(5);
/// Poll cadence with nothing tracked (or no meeting_ends trigger defined):
/// hold-start timing only feeds the ≥5-minute floor, so ±15s costs nothing,
/// and it keeps the synchronous coreaudiod round-trip off a 5s treadmill.
const IDLE_POLL: Duration = Duration::from_secs(15);

/// Default global release grace (docs/triggers/CONTEXT.md). Also the default
/// the #182 Settings knob (`store::get_meeting_release_grace_minutes`) reports.
pub(crate) const DEFAULT_RELEASE_GRACE_MINUTES: i64 = 2;

/// A tick gap far beyond both poll cadences means the machine slept (or the
/// process was suspended): mic state was last actually known at the previous
/// tick, so knowledge must end there rather than letting a meeting window
/// absorb the sleep.
const GAP_THRESHOLD: Duration = Duration::from_secs(60);

/// When the gap since the last successful observation exceeds
/// [`GAP_THRESHOLD`], returns the timestamp for a synthetic empty observation
/// (the last tick — the last instant state was known). `None` on the
/// first-ever tick or a normal-cadence tick. Never returns a stamp past `now`.
fn gap_observation_ms(last_tick_ms: Option<i64>, now_ms: i64) -> Option<i64> {
    let last = last_tick_ms?;
    (now_ms.saturating_sub(last) > GAP_THRESHOLD.as_millis() as i64).then_some(last.min(now_ms))
}

/// Spawn the meeting detector worker: its own poll loop (the triggers tick is
/// too coarse for grace precision), same shutdown pattern as
/// [`super::spawn_triggers_worker`]. macOS-only in effect — elsewhere every
/// snapshot read fails and the loop just idles.
pub fn spawn_meeting_detector_worker(
    infra: AppInfraState,
    app_handle: tauri::AppHandle,
    background_workers: BackgroundWorkersState,
) {
    let mut shutdown_rx = background_workers.subscribe();
    crate::native_capture::debug_log::log_info("starting meeting detector worker");
    let handle = tauri::async_runtime::spawn(async move {
        let mut detector = MeetingDetector::new(DEFAULT_RELEASE_GRACE_MINUTES * 60_000);
        let mut browser_tracker = BrowserMeetingTracker::new();
        let mut last_tick_ms: Option<i64> = None;
        loop {
            if *shutdown_rx.borrow() {
                break;
            }
            let sleep_for = detector_tick(
                &mut detector,
                &mut browser_tracker,
                &mut last_tick_ms,
                &infra,
                &app_handle,
            )
            .await;
            if shutdown_aware_sleep(&mut shutdown_rx, sleep_for).await {
                break;
            }
        }
        crate::native_capture::debug_log::log_info("stopped meeting detector worker");
    });
    background_workers.track(handle);
}

/// One detector pass; returns how long to sleep before the next.
/// `last_tick_ms` is the last successful observation instant — the sleep-gap
/// boundary ([`gap_observation_ms`]); a failed snapshot read leaves it alone.
async fn detector_tick(
    detector: &mut MeetingDetector,
    browser_tracker: &mut BrowserMeetingTracker,
    last_tick_ms: &mut Option<i64>,
    infra: &AppInfraState,
    app_handle: &tauri::AppHandle,
) -> Duration {
    let meeting_triggers: Vec<TriggerDefinition> = super::load_triggers(app_handle)
        .into_iter()
        .filter(|trigger| {
            trigger.enabled && matches!(trigger.condition, TriggerCondition::MeetingEnds { .. })
        })
        .collect();
    if meeting_triggers.is_empty() {
        // No Core Audio read (and no browser probes) at all with nobody to fire.
        detector.reset();
        browser_tracker.reset();
        return IDLE_POLL;
    }

    let grace_minutes = infra
        .trigger_state()
        .meeting_release_grace_minutes()
        .await
        .ok()
        .flatten()
        .filter(|minutes| *minutes > 0)
        .unwrap_or(DEFAULT_RELEASE_GRACE_MINUTES);
    detector.set_grace_ms(grace_minutes * 60_000);

    let holders = match capture_system_audio::snapshot_mic_holding_bundle_ids() {
        Ok(holders) => holders,
        Err(error) => {
            // A failed read is a skipped tick, never a "everyone released".
            tauri_plugin_log::log::debug!(
                "{MEETING_LOG_PREFIX} mic snapshot failed, skipping tick: {}",
                error.message
            );
            return if detector.is_tracking() || browser_tracker.is_tracking() {
                TRACKING_POLL
            } else {
                IDLE_POLL
            };
        }
    };

    let now = now_ms();
    if let Some(gap_stamp_ms) = gap_observation_ms(*last_tick_ms, now) {
        // Sleep/suspend gap: end knowledge at the last tick BEFORE the real
        // observation, so a meeting window never absorbs the sleep. The state
        // machine handles the rest (rejoin within grace stays one meeting; an
        // ongoing hold at wake re-holds fresh). Pre-evidence browser hold
        // starts are stale across the gap and cleared; evidence-backed ones
        // follow the detector's gap semantics like any conferencing hold.
        tauri_plugin_log::log::info!(
            "{MEETING_LOG_PREFIX} tick gap of {}s (sleep?); injecting empty observation at last tick {gap_stamp_ms}",
            now.saturating_sub(gap_stamp_ms) / 1000
        );
        browser_tracker.clear_pre_evidence_holds();
        let events = detector.observe(&BTreeSet::new(), gap_stamp_ms);
        process_meeting_events(events, browser_tracker, infra, app_handle, &meeting_triggers)
            .await;
    }
    *last_tick_ms = Some(now);

    // Browser holds: probe the front tab of every known browser currently
    // holding the mic without evidence yet (issue #180). One sighting of a
    // meeting URL marks the whole hold — evidence is sticky, so a marked hold
    // is never probed again. The probe obeys the capture privacy gates (ADR
    // 0057 amendment 2026-07-21): browser-URL `Off` disables it, privacy-
    // excluded browsers are never probed, stored evidence is sanitized per the
    // user's mode.
    let recording_settings =
        crate::native_capture::current_recording_settings_from_app_handle(app_handle);
    let policy = probe_policy(&recording_settings.metadata, &recording_settings.privacy);
    for bundle_id in browser_tracker.observe_raw_holders(&holders, now) {
        if !policy.enabled || policy.excluded.contains(&bundle_id) {
            continue;
        }
        let Some(raw_url) = probe_front_tab_url(&bundle_id).await else {
            // A failed probe (browser busy, no AppleScript/AX access, no front
            // window) is no evidence this tick, not an error.
            continue;
        };
        // Match on host + path (query never matters to the allowlist), store
        // sanitized: in the default Sanitized mode this strips e.g. Zoom's
        // `?pwd=` before the URL can reach the firing context.
        let Some(url) = capture_metadata::sanitize_url(&raw_url, policy.url_mode) else {
            continue;
        };
        if let Some(service) = browser_tracker.record_sighting(&bundle_id, &url) {
            tauri_plugin_log::log::info!(
                "{MEETING_LOG_PREFIX} meeting URL sighted app={bundle_id} service={service}; \
hold marked as meeting (sticky)"
            );
        }
    }

    // Evidence-backed browsers join the allowlist-filtered set, seeded with
    // their TRUE hold start so the meeting window is the whole mic hold, not
    // evidence-onward.
    let mut detector_set = conferencing_holders(&holders);
    for bundle_id in browser_tracker.evidence_backed_holders(&holders) {
        if let Some(start_ms) = browser_tracker.hold_start_ms(&bundle_id) {
            detector.seed_hold(&bundle_id, start_ms);
        }
        detector_set.insert(bundle_id);
    }

    let events = detector.observe(&detector_set, now);
    process_meeting_events(events, browser_tracker, infra, app_handle, &meeting_triggers).await;

    // With probing disabled a pre-evidence browser hold can never become a
    // meeting — no point ticking at probe cadence for it.
    if detector.is_tracking() || (policy.enabled && browser_tracker.is_tracking()) {
        TRACKING_POLL
    } else {
        IDLE_POLL
    }
}

/// The tick's probe policy, derived from the SAME gates as the capture-
/// metadata prober (ADR 0057 amendment 2026-07-21): `enabled` mirrors
/// `collect_browser_url_for_metadata` (metadata on AND mode != Off), and
/// `excluded` is the enabled privacy exclude list.
struct ProbePolicy {
    enabled: bool,
    url_mode: capture_metadata::BrowserUrlMode,
    excluded: BTreeSet<String>,
}

fn probe_policy(
    metadata: &capture_metadata::MetadataSettings,
    privacy: &capture_metadata::PrivacySettings,
) -> ProbePolicy {
    ProbePolicy {
        enabled: capture_metadata::metadata_collection_plan(metadata)
            .collect_browser_url_for_metadata,
        url_mode: metadata.browser_url_mode,
        excluded: capture_metadata::evaluate_privacy(
            privacy,
            &capture_metadata::MetadataContext::default(),
        )
        .excluded_bundle_ids
        .into_iter()
        .collect(),
    }
}

/// Front-tab URL probe for one browser, off the async runtime (osascript can
/// block up to ~1s, a Gecko AX read up to ~1.4s). Reuses the metadata layer's
/// strategy dispatch (Chromium/WebKit AppleScript, Gecko Accessibility) but
/// never fires the Accessibility prompt.
async fn probe_front_tab_url(bundle_id: &str) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let bundle_id = bundle_id.to_string();
        tauri::async_runtime::spawn_blocking(move || {
            crate::native_capture::metadata::probe_browser_front_tab_url(&bundle_id)
        })
        .await
        .ok()
        .flatten()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = bundle_id;
        None
    }
}

/// Log every detector event and route `Ended` into the firing path — shared by
/// the real observation and the synthetic sleep-gap observation, so gap-ended
/// meetings fire exactly like tick-ended ones.
async fn process_meeting_events(
    events: Vec<MeetingEvent>,
    browser_tracker: &mut BrowserMeetingTracker,
    infra: &AppInfraState,
    app_handle: &tauri::AppHandle,
    triggers: &[TriggerDefinition],
) {
    for event in events {
        match event {
            MeetingEvent::HoldStarted { bundle_id, at_ms } => {
                tauri_plugin_log::log::info!(
                    "{MEETING_LOG_PREFIX} mic hold started app={bundle_id} at={at_ms}"
                );
            }
            MeetingEvent::HoldReleased { bundle_id, at_ms } => {
                tauri_plugin_log::log::info!(
                    "{MEETING_LOG_PREFIX} mic released app={bundle_id} at={at_ms}; grace running"
                );
            }
            MeetingEvent::HoldRejoined { bundle_id } => {
                tauri_plugin_log::log::info!(
                    "{MEETING_LOG_PREFIX} rejoined within grace app={bundle_id}; same meeting"
                );
            }
            MeetingEvent::Ended(ended) => {
                // A browser hold's sticky evidence ends with the hold — taken
                // here so the NEXT hold needs a fresh sighting. `None` for
                // conferencing apps (the tracker never sees them).
                let evidence = browser_tracker.take_ended(&ended.bundle_id);
                tauri_plugin_log::log::info!(
                    "{MEETING_LOG_PREFIX} meeting ended app={} window={}..{} ({}s){}",
                    ended.bundle_id,
                    ended.start_ms,
                    ended.end_ms,
                    ended.duration_ms() / 1000,
                    evidence
                        .as_ref()
                        .map(|evidence| format!(" service={}", evidence.service))
                        .unwrap_or_default()
                );
                handle_meeting_ended(infra, app_handle, triggers, ended, evidence).await;
            }
        }
    }
}

/// Decide + fire every meeting trigger for one ended hold: floor, then the
/// shared cooldown/provider-gate decision, then claim and spawn the firing.
async fn handle_meeting_ended(
    infra: &AppInfraState,
    app_handle: &tauri::AppHandle,
    triggers: &[TriggerDefinition],
    ended: EndedHold,
    evidence: Option<MeetingUrlEvidence>,
) {
    let now = now_ms();
    for trigger in triggers {
        let Some(floor_ms) = trigger_floor_ms(trigger) else {
            continue;
        };
        if ended.duration_ms() < floor_ms {
            tauri_plugin_log::log::info!(
                "{MEETING_LOG_PREFIX} hold below trigger '{}' floor ({}s < {}s); not a meeting",
                trigger.id,
                ended.duration_ms() / 1000,
                floor_ms / 1000
            );
            continue;
        }
        let ledger_ms = infra
            .trigger_firings()
            .last_firing(&trigger.id)
            .await
            .ok()
            .flatten()
            .map(|firing| firing.fired_at_ms);
        let claim_cursor_ms = infra
            .trigger_state()
            .last_fired_ms(&trigger.id)
            .await
            .ok()
            .flatten();
        let last_firing_ms = super::event_cooldown_anchor_ms(ledger_ms, claim_cursor_ms);
        let provider_ready = crate::ask_ai::ensure_ask_ai_access_ready(app_handle)
            .await
            .is_ok();
        match super::firing_decision(
            Some(ended.end_ms),
            last_firing_ms,
            trigger.cooldown_ms(),
            provider_ready,
            now,
        ) {
            FiringDecision::NotDue => continue,
            FiringDecision::CooldownSuppressed => {
                // Unlike a Schedule occurrence, a meeting event is one-shot:
                // suppressed means dropped, exactly what Cooldown is for
                // (back-to-back mic churn re-firing the same recap).
                tauri_plugin_log::log::info!(
                    "{MEETING_LOG_PREFIX} trigger '{}' cooling down; meeting event dropped",
                    trigger.id
                );
            }
            FiringDecision::NeedsProvider => {
                tauri_plugin_log::log::info!(
                    "{MEETING_LOG_PREFIX} trigger '{}' needs an AI provider; meeting event dropped",
                    trigger.id
                );
            }
            FiringDecision::Fire { .. } => {
                if let Err(error) = infra.trigger_state().set_last_fired_ms(&trigger.id, now).await
                {
                    tauri_plugin_log::log::warn!(
                        "{MEETING_LOG_PREFIX} failed to record firing for trigger '{}': {error}; not running",
                        trigger.id
                    );
                    continue;
                }
                tauri_plugin_log::log::info!(
                    "{MEETING_LOG_PREFIX} firing trigger '{}' for meeting {}..{}",
                    trigger.id,
                    ended.start_ms,
                    ended.end_ms
                );
                spawn_meeting_firing(
                    Arc::clone(infra),
                    app_handle.clone(),
                    trigger.clone(),
                    ended.clone(),
                    evidence.clone(),
                    now,
                );
            }
        }
    }
}

/// Readiness Wait → run, as its own task so a (up to 15-minute) wait never
/// blocks the detector loop or stacks behind another firing. Deliberately
/// untracked: joining it at shutdown would hold the app open for the wait;
/// dropping it mid-flight is the documented crash-mid-run semantics (the
/// occurrence was claimed, the run is quietly missed).
fn spawn_meeting_firing(
    infra: AppInfraState,
    app_handle: tauri::AppHandle,
    trigger: TriggerDefinition,
    ended: EndedHold,
    evidence: Option<MeetingUrlEvidence>,
    fired_at_ms: i64,
) {
    tauri::async_runtime::spawn(async move {
        // The Readiness Wait IS the running state's start (DESIGN.md: "running
        // — waiting for the transcript"); the ledger row landing clears it.
        super::run::mark_trigger_running(&trigger.id);
        let window = (ended.start_ms, ended.end_ms);
        let outcome = readiness::wait_for_readiness(
            window,
            fired_at_ms,
            || readiness_snapshot(&infra, window.0, window.1),
            tokio::time::sleep,
            now_ms,
        )
        .await;
        match outcome {
            ReadinessOutcome::Skip { reason } => {
                // A Skipped Run: one honest ledger row, no notification.
                tauri_plugin_log::log::info!(
                    "{MEETING_LOG_PREFIX} trigger '{}' skipped: {reason}",
                    trigger.id
                );
                super::run::record_ledger(
                    &infra,
                    &trigger.id,
                    fired_at_ms,
                    TriggerFiringOutcome::Skipped,
                    Some(reason),
                    None,
                )
                .await;
            }
            ReadinessOutcome::Proceed { coverage_note } => {
                let offset_minutes = infra
                    .user_context()
                    .local_offset_minutes()
                    .await
                    .ok()
                    .flatten()
                    .map(|minutes| minutes as i32)
                    .unwrap_or(0);
                let (app_display_name, meeting_url) = meeting_display(&ended.bundle_id, evidence);
                let context = super::run::EventFiringContext::Meeting(MeetingFiringContext {
                    app_display_name,
                    start_ms: ended.start_ms,
                    end_ms: ended.end_ms,
                    meeting_url,
                    coverage_note,
                });
                super::run::run_trigger_fire(
                    &app_handle,
                    &infra,
                    &trigger,
                    ended.end_ms,
                    offset_minutes,
                    Some(&context),
                )
                .await;
            }
        }
    });
}

/// The firing display for an ended hold: "Zoom" for an allowlisted app;
/// "Google Meet (Google Chrome)" plus the sighted URL for an evidence-backed
/// browser meeting (issue #180).
fn meeting_display(
    bundle_id: &str,
    evidence: Option<MeetingUrlEvidence>,
) -> (String, Option<String>) {
    if let Some(evidence) = evidence {
        let browser = capture_metadata::known_browser_app(bundle_id)
            .map(|browser| browser.display_name)
            .unwrap_or(bundle_id);
        return (
            format!("{} ({browser})", evidence.service),
            Some(evidence.url),
        );
    }
    (
        conferencing_display_name(bundle_id)
            .unwrap_or(bundle_id)
            .to_string(),
        None,
    )
}

/// The real readiness probe: audio segments (mic + system audio) overlapping
/// the meeting window, and how many transcription/diarization jobs over them
/// are still queued/running.
async fn readiness_snapshot(
    infra: &AppInfraState,
    start_ms: i64,
    end_ms: i64,
) -> Result<ReadinessSnapshot, String> {
    let segments = infra
        .list_audio_segments_overlapping_range(
            &rfc3339_from_ms(start_ms),
            &rfc3339_from_ms(end_ms),
            None,
            None,
        )
        .await
        .map_err(|error| format!("list audio segments: {error}"))?;

    let mut snapshot = ReadinessSnapshot::default();
    for segment in &segments {
        snapshot.segment_spans_ms.push((
            ms_from_rfc3339(&segment.started_at).unwrap_or(start_ms),
            ms_from_rfc3339(&segment.ended_at).unwrap_or(end_ms),
        ));
        let jobs = infra
            .list_processing_jobs_for_subject(&ProcessingSubject::audio_segment(segment.id))
            .await
            .map_err(|error| format!("list processing jobs: {error}"))?;
        snapshot.pending_jobs += jobs
            .iter()
            .filter(|job| {
                matches!(
                    job.status,
                    ProcessingJobStatus::Queued | ProcessingJobStatus::Running
                ) && matches!(
                    job.processor.as_str(),
                    AUDIO_TRANSCRIPTION_PROCESSOR | SPEAKER_ANALYSIS_PROCESSOR
                )
            })
            .count();
    }
    Ok(snapshot)
}

fn rfc3339_from_ms(unix_ms: i64) -> String {
    time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(unix_ms) * 1_000_000)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn ms_from_rfc3339(value: &str) -> Option<i64> {
    time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
        .ok()
        .map(|dt| (dt.unix_timestamp_nanos() / 1_000_000) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_helpers_round_trip_segment_timestamps() {
        let ms = 1_784_505_600_000_i64; // 2026-07-20T00:00:00Z
        assert_eq!(ms_from_rfc3339(&rfc3339_from_ms(ms)), Some(ms));
        assert_eq!(ms_from_rfc3339("not a timestamp"), None);
    }

    #[test]
    fn claim_cursor_suppresses_second_ended_event_before_ledger_row_lands() {
        // First meeting claimed at `claim`; its ledger row is still pending
        // behind the Readiness Wait, so the ledger read yields None. A second
        // Ended event 7 min later (inside the 10-min cooldown) must be
        // suppressed via the claim cursor.
        const MIN_MS: i64 = 60_000;
        let claim = 1_000_000_i64;
        let now = claim + 7 * MIN_MS;
        assert_eq!(
            super::super::firing_decision(
                Some(now),
                super::super::event_cooldown_anchor_ms(None, Some(claim)),
                10 * MIN_MS,
                true,
                now,
            ),
            super::super::FiringDecision::CooldownSuppressed
        );
        // Once the cooldown passes, the same anchor no longer suppresses.
        let later = claim + 11 * MIN_MS;
        assert_eq!(
            super::super::firing_decision(
                Some(later),
                super::super::event_cooldown_anchor_ms(None, Some(claim)),
                10 * MIN_MS,
                true,
                later,
            ),
            super::super::FiringDecision::Fire {
                occurrence_ms: later
            }
        );
    }

    #[test]
    fn meeting_display_names_service_with_browser_and_falls_back_to_the_allowlist() {
        // Evidence-backed browser hold: service (browser) + URL.
        let evidence = MeetingUrlEvidence {
            url: "https://meet.google.com/abc-defg-hij".to_string(),
            service: "Google Meet",
        };
        assert_eq!(
            meeting_display("com.google.Chrome", Some(evidence)),
            (
                "Google Meet (Google Chrome)".to_string(),
                Some("https://meet.google.com/abc-defg-hij".to_string())
            )
        );
        // Conferencing app: allowlist display name, no URL.
        assert_eq!(
            meeting_display("us.zoom.xos", None),
            ("Zoom".to_string(), None)
        );
        // Unknown bundle id degrades to the raw id.
        assert_eq!(
            meeting_display("com.unknown.app", None),
            ("com.unknown.app".to_string(), None)
        );
    }

    #[test]
    fn gap_observation_fires_only_on_large_gaps_and_never_on_first_tick() {
        // First-ever tick: nothing to compare against.
        assert_eq!(gap_observation_ms(None, 1_000_000), None);
        // Normal 5s/15s cadences stay far under the threshold.
        assert_eq!(gap_observation_ms(Some(1_000_000), 1_015_000), None);
        // Exactly the threshold is still a normal tick (> semantics).
        assert_eq!(gap_observation_ms(Some(1_000_000), 1_060_000), None);
        // A sleep gap stamps the last instant state was actually known.
        assert_eq!(
            gap_observation_ms(Some(1_000_000), 1_061_000),
            Some(1_000_000)
        );
        // Clock stepped backwards: no gap, and never a stamp past `now`.
        assert_eq!(gap_observation_ms(Some(2_000_000), 1_000_000), None);
    }

    #[test]
    fn probe_policy_mirrors_the_capture_gates() {
        use capture_metadata::{
            BrowserUrlMode, ExcludedAppEntry, MetadataSettings, PrivacySettings,
        };
        let metadata = MetadataSettings::default(); // enabled, Sanitized
        let privacy = PrivacySettings {
            excluded_apps: vec![
                ExcludedAppEntry {
                    id: "1".into(),
                    enabled: true,
                    bundle_id: "com.google.Chrome".into(),
                    display_name: "Chrome".into(),
                },
                ExcludedAppEntry {
                    id: "2".into(),
                    enabled: false, // disabled entries don't exclude
                    bundle_id: "com.apple.Safari".into(),
                    display_name: "Safari".into(),
                },
            ],
            ..PrivacySettings::default()
        };
        let policy = probe_policy(&metadata, &privacy);
        assert!(policy.enabled);
        assert_eq!(policy.url_mode, BrowserUrlMode::Sanitized);
        assert!(policy.excluded.contains("com.google.Chrome"));
        assert!(!policy.excluded.contains("com.apple.Safari"));

        // Browser-URL mode Off disables probing entirely.
        let off = MetadataSettings {
            browser_url_mode: BrowserUrlMode::Off,
            ..MetadataSettings::default()
        };
        assert!(!probe_policy(&off, &privacy).enabled);
        // Metadata collection disabled does too (same plan gate as capture).
        let disabled = MetadataSettings {
            enabled: false,
            ..MetadataSettings::default()
        };
        assert!(!probe_policy(&disabled, &privacy).enabled);
    }
}
