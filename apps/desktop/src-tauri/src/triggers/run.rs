//! The Firing → sealed Ask AI run path (issues #175/#176, ADR 0058).
//!
//! [`run_trigger_fire`] owns everything after the evaluator claims an
//! occurrence: the origin-tagged conversation, the sealed-toolbox turn through
//! the shared Ask AI driver with **retries** (backoff [`RUN_RETRY_BACKOFF`]),
//! exactly one `trigger_firings` **ledger row** per firing
//! (completed/skipped/failed with an honest reason), and the good-news-only
//! **Delivery** (macOS notification on `completed` only — skips and failures
//! never notify, they surface as last-run status read from the ledger).

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use ::app_infra::trigger_firings::TriggerFiringOutcome;
use tauri_plugin_notification::NotificationExt;

use super::schedule::{ScheduleCadence, ScheduleWeekday};
use super::{TriggerCondition, TriggerDefinition};
use crate::app_infra::AppInfraState;
use crate::user_context::worker::now_ms;

/// How long a delivered notification's pending-open slot stays valid. An
/// activation later than this opens the app normally instead of routing onto a
/// stale trigger conversation.
const NOTIFICATION_OPEN_TTL_MS: i64 = 15 * 60 * 1000;

/// Backoff between AI-run attempts within ONE firing: a transient failure
/// retries twice before the ledger records `failed`. The occurrence was claimed
/// up front, so retries never re-fire the schedule — they happen inside the one
/// firing.
const RUN_RETRY_BACKOFF: [Duration; 2] = [Duration::from_secs(30), Duration::from_secs(60)];

// ── Local-time display helpers ───────────────────────────────────────────────

/// The instant as the user's local wall clock.
fn local_datetime(utc_ms: i64, offset_minutes: i32) -> time::OffsetDateTime {
    let local_ms = utc_ms + i64::from(offset_minutes) * 60_000;
    time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(local_ms) * 1_000_000)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
}

/// Short local date for the self-generated run title, e.g. "Fri Jul 24".
fn format_short_local_date(utc_ms: i64, offset_minutes: i32) -> String {
    let local = local_datetime(utc_ms, offset_minutes);
    let weekday = local.weekday().to_string();
    let month = local.date().month().to_string();
    format!("{} {} {}", &weekday[..3], &month[..3], local.day())
}

/// `YYYY-MM-DD HH:MM` local, for the firing-context window bounds.
fn format_local_ymd_hm(utc_ms: i64, offset_minutes: i32) -> String {
    let local = local_datetime(utc_ms, offset_minutes);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        local.year(),
        u8::from(local.month()),
        local.day(),
        local.hour(),
        local.minute(),
    )
}

/// The run's conversation title: `"<Trigger Name> — <abbrev date>"`.
fn run_title(trigger_name: &str, occurrence_ms: i64, offset_minutes: i32) -> String {
    format!(
        "{trigger_name} — {}",
        format_short_local_date(occurrence_ms, offset_minutes)
    )
}

/// One-line human description of the schedule, for the firing context.
fn schedule_label(cadence: ScheduleCadence, time: &str, weekday: Option<ScheduleWeekday>) -> String {
    match cadence {
        ScheduleCadence::Daily => format!("daily at {time}"),
        ScheduleCadence::Weekly => match weekday {
            Some(day) => format!("weekly on {day:?} at {time}"),
            None => format!("weekly at {time}"),
        },
    }
}

/// The Meeting half of the firing context (issue #177): the detected mic-hold
/// window and app, plus the Readiness Wait's honesty note when the recording
/// only partially covers the meeting or the catch-up was cut at the cap.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MeetingFiringContext {
    /// "Zoom" for a conferencing app; "Google Meet (Google Chrome)" for an
    /// evidence-backed browser meeting (issue #180).
    pub app_display_name: String,
    pub start_ms: i64,
    pub end_ms: i64,
    /// The sighted meeting URL for a browser meeting; `None` for app holds.
    pub meeting_url: Option<String>,
    pub coverage_note: Option<String>,
}

/// The App Opened firing context (issue #178): which app started a fresh
/// session and when it was last frontmost (`None` = first session observed).
/// The away window is `last_frontmost_end_ms → occurrence_ms`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AppOpenedFiringContext {
    pub app_display_name: String,
    pub last_frontmost_end_ms: Option<i64>,
}

/// What an event condition hands to [`run_trigger_fire`]; Schedule firings
/// pass `None` and describe their window from the cadence instead.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum EventFiringContext {
    Meeting(MeetingFiringContext),
    AppOpened(AppOpenedFiringContext),
}

/// Human away span for the app-opened firing context.
fn format_away_duration(away_ms: i64) -> String {
    let minutes = (away_ms / 60_000).max(0);
    if minutes < 120 {
        format!("about {minutes} minutes")
    } else {
        format!("about {} hours", minutes / 60)
    }
}

/// The **Context Assembly** for this slice: the firing context prepended to the
/// user's Prompt. Names the trigger and what fired it — the schedule and its
/// natural-period window, or the detected meeting's window and app — then the
/// standing instruction verbatim. This whole string is the sealed turn's
/// "question", so it persists on the turn row and stays in follow-up history.
fn build_firing_question(
    trigger: &TriggerDefinition,
    occurrence_ms: i64,
    now_ms: i64,
    offset_minutes: i32,
    event: Option<&EventFiringContext>,
) -> String {
    if let Some(EventFiringContext::Meeting(meeting)) = event {
        let url_note = meeting
            .meeting_url
            .as_deref()
            .map(|url| format!(" Meeting URL: {url}."))
            .unwrap_or_default();
        let coverage = meeting
            .coverage_note
            .as_deref()
            .map(|note| format!(" Note: {note}"))
            .unwrap_or_default();
        return format!(
            "[Automated Trigger Run] The user's trigger \"{name}\" (meeting ends) fired: a meeting \
in {app} just ended. Meeting window: {start} to {end} local time (about {minutes} minutes).\
{url_note}{coverage} Apply the standing instruction below to that meeting unless it says \
otherwise, and write the report document now.\n\n\
Standing instruction:\n{prompt}",
            name = trigger.name,
            app = meeting.app_display_name,
            start = format_local_ymd_hm(meeting.start_ms, offset_minutes),
            end = format_local_ymd_hm(meeting.end_ms, offset_minutes),
            minutes = (meeting.end_ms - meeting.start_ms).max(0) / 60_000,
            url_note = url_note,
            coverage = coverage,
            prompt = trigger.prompt.trim(),
        );
    }
    if let Some(EventFiringContext::AppOpened(opened)) = event {
        let session = match opened.last_frontmost_end_ms {
            Some(since_ms) => format!(
                "the user just opened {app} after {away} away. Window since the last {app} \
session: {since} to {now} local time",
                app = opened.app_display_name,
                away = format_away_duration(occurrence_ms.saturating_sub(since_ms)),
                since = format_local_ymd_hm(since_ms, offset_minutes),
                now = format_local_ymd_hm(occurrence_ms, offset_minutes),
            ),
            None => format!(
                "the user just opened {app} at {now} local time (the first {app} session \
observed; no previous session on record)",
                app = opened.app_display_name,
                now = format_local_ymd_hm(occurrence_ms, offset_minutes),
            ),
        };
        return format!(
            "[Automated Trigger Run] The user's trigger \"{name}\" (app opened) fired: {session}. \
Apply the standing instruction below — the time since the user last used the app is the natural \
window to consider — and write the report document now.\n\n\
Standing instruction:\n{prompt}",
            name = trigger.name,
            session = session,
            prompt = trigger.prompt.trim(),
        );
    }
    let TriggerCondition::Schedule {
        cadence,
        time,
        weekday,
    } = &trigger.condition
    else {
        // Only the event workers fire event conditions, and they always pass
        // the context; keep an honest fallback rather than a panic.
        return format!(
            "[Automated Trigger Run] The user's trigger \"{name}\" fired. Apply the standing \
instruction below and write the report document now.\n\n\
Standing instruction:\n{prompt}",
            name = trigger.name,
            prompt = trigger.prompt.trim(),
        );
    };
    let period = match cadence {
        ScheduleCadence::Daily => "the day so far",
        ScheduleCadence::Weekly => "the week so far (Monday-start)",
    };
    let window_start_ms = match cadence {
        ScheduleCadence::Daily => {
            let local = local_datetime(occurrence_ms, offset_minutes);
            let midnight = local.replace_time(time::Time::MIDNIGHT);
            (midnight.unix_timestamp_nanos() / 1_000_000) as i64
                - i64::from(offset_minutes) * 60_000
        }
        ScheduleCadence::Weekly => {
            let local = local_datetime(occurrence_ms, offset_minutes);
            let days_back = local.weekday().number_days_from_monday();
            let monday = local.replace_time(time::Time::MIDNIGHT)
                - time::Duration::days(i64::from(days_back));
            (monday.unix_timestamp_nanos() / 1_000_000) as i64
                - i64::from(offset_minutes) * 60_000
        }
    };
    format!(
        "[Automated Trigger Run] The user's trigger \"{name}\" ({schedule}, local time) fired. \
Covering window: {period} — {start} to {now} local time. Apply the standing instruction below \
to that window unless it says otherwise, and write the report document now.\n\n\
Standing instruction:\n{prompt}",
        name = trigger.name,
        schedule = schedule_label(*cadence, time, *weekday),
        period = period,
        start = format_local_ymd_hm(window_start_ms, offset_minutes),
        now = format_local_ymd_hm(now_ms, offset_minutes),
        prompt = trigger.prompt.trim(),
    )
}

// ── Delivery (notification + pending open) ───────────────────────────────────

/// The conversation a just-delivered notification should open, with its
/// delivery instant. One slot, latest wins — only the newest completed run
/// deserves the next activation.
static PENDING_NOTIFICATION_OPEN: OnceLock<Mutex<Option<(String, i64)>>> = OnceLock::new();

fn pending_notification_open() -> &'static Mutex<Option<(String, i64)>> {
    PENDING_NOTIFICATION_OPEN.get_or_init(|| Mutex::new(None))
}

fn set_pending_notification_open(conversation_id: &str) {
    if let Ok(mut slot) = pending_notification_open().lock() {
        *slot = Some((conversation_id.to_string(), now_ms()));
    }
}

/// Take the pending notification conversation if one was delivered recently
/// (within [`NOTIFICATION_OPEN_TTL_MS`]). The desktop notification plugin has
/// no click callback on macOS, so app ACTIVATION with no open window is the
/// click signal we route on; the TTL keeps a long-ignored notification from
/// hijacking an unrelated later activation.
pub(crate) fn take_recent_notification_conversation() -> Option<String> {
    let mut slot = pending_notification_open().lock().ok()?;
    let (conversation_id, delivered_at_ms) = slot.take()?;
    if now_ms().saturating_sub(delivered_at_ms) > NOTIFICATION_OPEN_TTL_MS {
        return None;
    }
    Some(conversation_id)
}

/// Deliver the good-news notification for a COMPLETED run (ADR 0058: skips and
/// failures never notify) and arm the pending-open slot. Best-effort — a
/// denied/unavailable notification never fails the run.
fn deliver_run_notification(app_handle: &tauri::AppHandle, title: &str, conversation_id: &str) {
    let notifications = app_handle.notification();
    // macOS prompts on first use; ask explicitly when not yet granted so the
    // prompt appears at the first delivery rather than never.
    match notifications.permission_state() {
        Ok(tauri_plugin_notification::PermissionState::Granted) => {}
        _ => {
            let _ = notifications.request_permission();
        }
    }
    let shown = notifications
        .builder()
        .title(title)
        .body("Your trigger finished — open Mnema to read the report.")
        .show();
    match shown {
        Ok(()) => set_pending_notification_open(conversation_id),
        Err(error) => {
            tauri_plugin_log::log::warn!(
                "triggers: failed to deliver run notification for {conversation_id}: {error}"
            );
        }
    }
}

// ── Retries ──────────────────────────────────────────────────────────────────

/// One attempt's verdict: only transient failures are worth retrying — a user
/// cancel is a deliberate stop, never re-run against their explicit intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttemptOutcome {
    Completed,
    Failed,
    Aborted,
}

/// Run `attempt` until it completes or aborts, sleeping `backoff[i]` between
/// transient failures (so at most `backoff.len() + 1` attempts). Returns
/// `(outcome, attempts)`. Generic over the sleeper so retry-then-failed
/// semantics unit-test without wall-clock time.
async fn attempt_with_retries<A, AFut, S, SFut>(
    backoff: &[Duration],
    mut attempt: A,
    mut sleep: S,
) -> (AttemptOutcome, usize)
where
    A: FnMut() -> AFut,
    AFut: std::future::Future<Output = AttemptOutcome>,
    S: FnMut(Duration) -> SFut,
    SFut: std::future::Future<Output = ()>,
{
    let mut attempts = 1usize;
    match attempt().await {
        AttemptOutcome::Failed => {}
        outcome => return (outcome, attempts),
    }
    for &delay in backoff {
        sleep(delay).await;
        attempts += 1;
        match attempt().await {
            AttemptOutcome::Failed => {}
            outcome => return (outcome, attempts),
        }
    }
    (AttemptOutcome::Failed, attempts)
}

// ── Firing → ledger row ──────────────────────────────────────────────────────

/// Write the firing's ledger row. Best-effort at this seam: the firing already
/// happened, so a failed ledger write logs loudly but never un-notifies or
/// re-runs anything. `pub(crate)` because the meeting path records its
/// Skipped Runs (Readiness Wait: not recording) without entering a run.
pub(crate) async fn record_ledger(
    infra: &AppInfraState,
    trigger_id: &str,
    fired_at_ms: i64,
    outcome: TriggerFiringOutcome,
    reason: Option<&str>,
    conversation_id: Option<&str>,
) {
    if let Err(error) = infra
        .trigger_firings()
        .record_firing(trigger_id, fired_at_ms, outcome, reason, conversation_id)
        .await
    {
        tauri_plugin_log::log::warn!(
            "triggers: failed to record {} ledger row for trigger '{trigger_id}': {error}",
            outcome.as_str()
        );
    }
}

/// Run one Firing end-to-end: persist the origin-tagged conversation, run the
/// sealed-toolbox turn through the shared Ask AI driver with retries, write
/// exactly ONE ledger row for the outcome, and deliver the notification on a
/// completed run only.
pub(crate) async fn run_trigger_fire(
    app_handle: &tauri::AppHandle,
    infra: &AppInfraState,
    trigger: &TriggerDefinition,
    occurrence_ms: i64,
    offset_minutes: i32,
    event: Option<&EventFiringContext>,
) {
    let fired_at = now_ms();
    // The firing's stable conversation id: trigger id + occurrence instant, so
    // one occurrence maps to one conversation even across a re-fire attempt.
    let conversation_id = format!("trigger-{}-{}", trigger.id, occurrence_ms);
    let title = run_title(&trigger.name, occurrence_ms, offset_minutes);

    if let Err(error) = infra
        .conversation()
        .create_trigger_conversation(&conversation_id, &title, &trigger.id, &trigger.name, fired_at)
        .await
    {
        tauri_plugin_log::log::warn!(
            "triggers: failed to create run conversation for trigger '{}': {error}",
            trigger.id
        );
        record_ledger(
            infra,
            &trigger.id,
            fired_at,
            TriggerFiringOutcome::Failed,
            Some(&format!("failed to create run conversation: {error}")),
            None,
        )
        .await;
        return;
    }
    let _ = tauri::Emitter::emit(
        app_handle,
        crate::conversation::commands::CONVERSATION_CHANGED_EVENT,
        (),
    );

    let question = build_firing_question(trigger, occurrence_ms, fired_at, offset_minutes, event);
    // Each attempt is a fresh sealed turn in the SAME conversation (an errored
    // attempt persists as an errored turn; only a completed one becomes the
    // report and history).
    let attempt = || {
        let app_handle = app_handle.clone();
        let conversation_id = conversation_id.clone();
        let question = question.clone();
        let title = title.clone();
        async move {
            let clock = crate::ask_ai::ClientClock {
                utc_offset_minutes: Some(offset_minutes),
                time_zone: None,
            };
            let cancel = crate::ask_ai::register_inflight(&conversation_id);
            let completed = crate::ask_ai::run_ask_ai_turn(
                app_handle,
                conversation_id.clone(),
                question,
                None,
                "trigger".to_string(),
                title,
                clock,
                cancel.clone(),
                /* sealed = */ true,
            )
            .await;
            if completed {
                AttemptOutcome::Completed
            } else if cancel.load(std::sync::atomic::Ordering::SeqCst) {
                // The user hit stop on the visible run: honor it, don't retry.
                AttemptOutcome::Aborted
            } else {
                AttemptOutcome::Failed
            }
        }
    };
    let trigger_id = trigger.id.clone();
    let (outcome, attempts) = attempt_with_retries(&RUN_RETRY_BACKOFF, attempt, |delay| {
        tauri_plugin_log::log::warn!(
            "triggers: run attempt for trigger '{trigger_id}' did not complete; retrying in {}s",
            delay.as_secs()
        );
        tokio::time::sleep(delay)
    })
    .await;

    match outcome {
        AttemptOutcome::Completed => {
            record_ledger(
                infra,
                &trigger.id,
                fired_at,
                TriggerFiringOutcome::Completed,
                None,
                Some(&conversation_id),
            )
            .await;
            deliver_run_notification(app_handle, &title, &conversation_id);
        }
        AttemptOutcome::Aborted => {
            tauri_plugin_log::log::warn!(
                "triggers: run for trigger '{}' cancelled by the user; no notification delivered",
                trigger.id
            );
            record_ledger(
                infra,
                &trigger.id,
                fired_at,
                TriggerFiringOutcome::Failed,
                Some("run cancelled by the user"),
                Some(&conversation_id),
            )
            .await;
        }
        AttemptOutcome::Failed => {
            tauri_plugin_log::log::warn!(
                "triggers: run for trigger '{}' failed after {attempts} attempts; no notification delivered",
                trigger.id
            );
            record_ledger(
                infra,
                &trigger.id,
                fired_at,
                TriggerFiringOutcome::Failed,
                Some(&format!("AI run did not complete after {attempts} attempts")),
                None,
            )
            .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::sample_daily;
    use super::*;
    use std::cell::{Cell, RefCell};

    const IST: i32 = 330;

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(future)
    }

    #[test]
    fn run_title_is_name_dash_abbreviated_local_date() {
        // 2026-07-24 09:00 UTC is a Friday; IST stays the same date.
        let friday_utc_ms = 1_784_505_600_000 + 4 * 86_400_000 + 9 * 3_600_000;
        assert_eq!(
            run_title("Evening Review", friday_utc_ms, IST),
            "Evening Review — Fri Jul 24"
        );
        // A negative offset that crosses local midnight shifts the date.
        let just_past_utc_midnight = 1_784_505_600_000 + 30 * 60_000; // Mon 00:30 UTC
        assert_eq!(
            run_title("Recap", just_past_utc_midnight, -480),
            "Recap — Sun Jul 19"
        );
    }

    #[test]
    fn firing_question_carries_context_window_and_prompt() {
        let trigger = sample_daily();
        // Fired at 18:30 IST on 2026-07-20 (13:00 UTC), evaluated at 13:05 UTC.
        let occurrence_ms = 1_784_505_600_000 + 13 * 3_600_000;
        let now_ms = occurrence_ms + 5 * 60_000;
        let question = build_firing_question(&trigger, occurrence_ms, now_ms, IST, None);
        assert!(question.starts_with("[Automated Trigger Run]"));
        assert!(question.contains("\"Evening Review\""));
        assert!(question.contains("daily at 18:30"));
        // Day-so-far window: local midnight → now, in local time.
        assert!(question.contains("2026-07-20 00:00"));
        assert!(question.contains("2026-07-20 18:35"));
        // The user's standing instruction rides verbatim at the end.
        assert!(question.ends_with("Summarize what I worked on today."));
    }

    #[test]
    fn meeting_firing_question_carries_window_app_and_coverage_note() {
        let trigger = TriggerDefinition {
            id: "meeting-recap".to_string(),
            name: "Meeting Recap".to_string(),
            condition: super::super::TriggerCondition::MeetingEnds {
                min_meeting_minutes: None,
            },
            prompt: "Recap the meeting.".to_string(),
            ..sample_daily()
        };
        // A 42-minute Zoom call ending 13:00 UTC on 2026-07-20 (18:30 IST).
        let end_ms = 1_784_505_600_000 + 13 * 3_600_000;
        let meeting = MeetingFiringContext {
            app_display_name: "Zoom".to_string(),
            start_ms: end_ms - 42 * 60_000,
            end_ms,
            meeting_url: None,
            coverage_note: Some("The recording covers only part of the meeting window.".to_string()),
        };
        let question = build_firing_question(
            &trigger,
            end_ms,
            end_ms + 5 * 60_000,
            IST,
            Some(&EventFiringContext::Meeting(meeting.clone())),
        );
        assert!(question.starts_with("[Automated Trigger Run]"));
        assert!(question.contains("\"Meeting Recap\""));
        assert!(question.contains("a meeting in Zoom just ended"));
        // The meeting window in LOCAL time, with its duration.
        assert!(question.contains("2026-07-20 17:48 to 2026-07-20 18:30"));
        assert!(question.contains("about 42 minutes"));
        // The Readiness Wait's honesty note rides along...
        assert!(question.contains("Note: The recording covers only part of the meeting window."));
        // ...and the standing instruction still closes the question verbatim.
        assert!(question.ends_with("Recap the meeting."));

        // App holds carry no URL — no dangling "Meeting URL:".
        assert!(!question.contains("Meeting URL:"));

        // Without a note there is no dangling "Note:".
        let quiet = MeetingFiringContext {
            coverage_note: None,
            ..meeting.clone()
        };
        let question = build_firing_question(
            &trigger,
            end_ms,
            end_ms + 5 * 60_000,
            IST,
            Some(&EventFiringContext::Meeting(quiet)),
        );
        assert!(!question.contains("Note:"));

        // A browser meeting (issue #180) names the service + browser and rides
        // the sighted meeting URL along.
        let browser_meeting = MeetingFiringContext {
            app_display_name: "Google Meet (Google Chrome)".to_string(),
            meeting_url: Some("https://meet.google.com/abc-defg-hij".to_string()),
            ..meeting
        };
        let question = build_firing_question(
            &trigger,
            end_ms,
            end_ms + 5 * 60_000,
            IST,
            Some(&EventFiringContext::Meeting(browser_meeting)),
        );
        assert!(question.contains("a meeting in Google Meet (Google Chrome) just ended"));
        assert!(question.contains("Meeting URL: https://meet.google.com/abc-defg-hij."));
        assert!(question.ends_with("Recap the meeting."));
    }

    #[test]
    fn app_opened_firing_question_carries_app_away_window_and_prompt() {
        let trigger = TriggerDefinition {
            id: "figma-session".to_string(),
            name: "Figma Session".to_string(),
            condition: super::super::TriggerCondition::AppOpened {
                bundle_id: "com.figma.Desktop".to_string(),
                app_name: "Figma".to_string(),
                away_gap_minutes: None,
            },
            prompt: "Recap where I left off.".to_string(),
            ..sample_daily()
        };
        // Opened at 13:00 UTC on 2026-07-20 (18:30 IST) after 2h away.
        let opened_ms = 1_784_505_600_000 + 13 * 3_600_000;
        let context = EventFiringContext::AppOpened(AppOpenedFiringContext {
            app_display_name: "Figma".to_string(),
            last_frontmost_end_ms: Some(opened_ms - 2 * 3_600_000),
        });
        let question = build_firing_question(&trigger, opened_ms, opened_ms, IST, Some(&context));
        assert!(question.starts_with("[Automated Trigger Run]"));
        assert!(question.contains("\"Figma Session\""));
        assert!(question.contains("(app opened)"));
        assert!(question.contains("just opened Figma after about 2 hours away"));
        // The away window (last-frontmost-end → now) in LOCAL time.
        assert!(question.contains("2026-07-20 16:30 to 2026-07-20 18:30"));
        assert!(question.ends_with("Recap where I left off."));

        // First observed session: no away window, an honest first-session note.
        let first = EventFiringContext::AppOpened(AppOpenedFiringContext {
            app_display_name: "Figma".to_string(),
            last_frontmost_end_ms: None,
        });
        let question = build_firing_question(&trigger, opened_ms, opened_ms, IST, Some(&first));
        assert!(question.contains("the first Figma session observed"));
        assert!(question.contains("2026-07-20 18:30"));
        assert!(!question.contains(" away"));
        assert!(question.ends_with("Recap where I left off."));
    }

    #[test]
    fn away_duration_formats_minutes_then_whole_hours() {
        assert_eq!(format_away_duration(35 * 60_000), "about 35 minutes");
        assert_eq!(format_away_duration(119 * 60_000), "about 119 minutes");
        assert_eq!(format_away_duration(125 * 60_000), "about 2 hours");
        assert_eq!(format_away_duration(-5), "about 0 minutes");
    }

    #[test]
    fn pending_notification_open_expires_after_ttl_and_is_take_once() {
        set_pending_notification_open("conv-fresh");
        assert_eq!(
            take_recent_notification_conversation().as_deref(),
            Some("conv-fresh")
        );
        // Take-once: the slot is consumed.
        assert_eq!(take_recent_notification_conversation(), None);

        // An expired delivery is dropped, not returned.
        if let Ok(mut slot) = pending_notification_open().lock() {
            *slot = Some((
                "conv-stale".to_string(),
                now_ms() - NOTIFICATION_OPEN_TTL_MS - 1,
            ));
        }
        assert_eq!(take_recent_notification_conversation(), None);
    }

    #[test]
    fn retry_exhausts_the_backoff_schedule_then_reports_failed() {
        block_on(async {
            let calls = Cell::new(0usize);
            let sleeps = RefCell::new(Vec::new());
            let (outcome, attempts) = attempt_with_retries(
                &RUN_RETRY_BACKOFF,
                || {
                    calls.set(calls.get() + 1);
                    std::future::ready(AttemptOutcome::Failed)
                },
                |delay| {
                    sleeps.borrow_mut().push(delay);
                    std::future::ready(())
                },
            )
            .await;
            // Transient failure retried twice with backoff, then gave up: the
            // caller records exactly one `failed` ledger row from this.
            assert_eq!(outcome, AttemptOutcome::Failed);
            assert_eq!(attempts, 3);
            assert_eq!(calls.get(), 3);
            assert_eq!(
                *sleeps.borrow(),
                vec![Duration::from_secs(30), Duration::from_secs(60)]
            );
        });
    }

    #[test]
    fn retry_stops_at_the_first_success() {
        block_on(async {
            let calls = Cell::new(0usize);
            let sleeps = RefCell::new(Vec::new());
            let (outcome, attempts) = attempt_with_retries(
                &RUN_RETRY_BACKOFF,
                || {
                    calls.set(calls.get() + 1);
                    std::future::ready(if calls.get() == 2 {
                        AttemptOutcome::Completed
                    } else {
                        AttemptOutcome::Failed
                    })
                },
                |delay| {
                    sleeps.borrow_mut().push(delay);
                    std::future::ready(())
                },
            )
            .await;
            assert_eq!(outcome, AttemptOutcome::Completed);
            assert_eq!(attempts, 2);
            assert_eq!(*sleeps.borrow(), vec![Duration::from_secs(30)]);
        });
    }

    #[test]
    fn retry_never_reruns_a_user_cancelled_attempt() {
        block_on(async {
            let calls = Cell::new(0usize);
            let sleeps = RefCell::new(Vec::new());
            let (outcome, attempts) = attempt_with_retries(
                &RUN_RETRY_BACKOFF,
                || {
                    calls.set(calls.get() + 1);
                    std::future::ready(AttemptOutcome::Aborted)
                },
                |delay| {
                    sleeps.borrow_mut().push(delay);
                    std::future::ready(())
                },
            )
            .await;
            // A deliberate cancel stops the firing cold: one attempt, no sleeps.
            assert_eq!(outcome, AttemptOutcome::Aborted);
            assert_eq!(attempts, 1);
            assert_eq!(calls.get(), 1);
            assert!(sleeps.borrow().is_empty());
        });
    }
}
