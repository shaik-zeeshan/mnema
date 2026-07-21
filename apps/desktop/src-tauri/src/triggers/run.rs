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
/// `pub(crate)` for the Context Assembly's past-run labels (issue #183).
pub(crate) fn format_short_local_date(utc_ms: i64, offset_minutes: i32) -> String {
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
fn schedule_label(cadence: ScheduleCadence, time: &str, weekdays: &[ScheduleWeekday]) -> String {
    match cadence {
        ScheduleCadence::Daily => format!("daily at {time}"),
        ScheduleCadence::Weekly if weekdays.is_empty() => format!("weekly at {time}"),
        ScheduleCadence::Weekly => {
            let days = weekdays
                .iter()
                .map(|day| format!("{day:?}"))
                .collect::<Vec<_>>()
                .join(", ");
            format!("weekly on {days} at {time}")
        }
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
    let TriggerCondition::Schedule { cadence, time, .. } = &trigger.condition else {
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
    let weekdays = trigger.condition.schedule_weekdays();
    let (period, window_start_ms) = match cadence {
        ScheduleCadence::Daily => {
            let local = local_datetime(occurrence_ms, offset_minutes);
            let midnight = local.replace_time(time::Time::MIDNIGHT);
            let start = (midnight.unix_timestamp_nanos() / 1_000_000) as i64
                - i64::from(offset_minutes) * 60_000;
            ("the day so far", start)
        }
        ScheduleCadence::Weekly => {
            let local = local_datetime(occurrence_ms, offset_minutes);
            let occurrence_index = i64::from(local.weekday().number_days_from_monday());
            // With a multi-day set, the natural window runs from the PREVIOUS
            // selected occurrence this week (e.g. weekdays at 18:00: Tuesday's
            // run covers Monday 18:00 → now, not the whole week again).
            let previous_index = weekdays
                .iter()
                .map(|day| day.week_index())
                .filter(|index| *index < occurrence_index)
                .max();
            match previous_index {
                Some(previous) => (
                    "since this trigger's previous scheduled run",
                    occurrence_ms - (occurrence_index - previous) * 86_400_000,
                ),
                None => {
                    let monday = local.replace_time(time::Time::MIDNIGHT)
                        - time::Duration::days(occurrence_index);
                    let start = (monday.unix_timestamp_nanos() / 1_000_000) as i64
                        - i64::from(offset_minutes) * 60_000;
                    ("the week so far (Monday-start)", start)
                }
            }
        }
    };
    format!(
        "[Automated Trigger Run] The user's trigger \"{name}\" ({schedule}, local time) fired. \
Covering window: {period} — {start} to {now} local time. Apply the standing instruction below \
to that window unless it says otherwise, and write the report document now.\n\n\
Standing instruction:\n{prompt}",
        name = trigger.name,
        schedule = schedule_label(*cadence, time, weekdays),
        period = period,
        start = format_local_ymd_hm(window_start_ms, offset_minutes),
        now = format_local_ymd_hm(now_ms, offset_minutes),
        prompt = trigger.prompt.trim(),
    )
}

// ── Running / Readiness-Wait registry (the sixth lifecycle state) ────────────

/// In-flight firings: trigger id → the UTC-ms instant the firing started
/// (entering the Readiness Wait or the run itself). In-memory only — the
/// ledger's post-wait semantics are unchanged; this exists so
/// `list_triggers_status` can surface "running — waiting for the transcript"
/// while a firing is between claim and ledger row. Marked at every firing
/// entry point, cleared in [`record_ledger`] — the one seam every outcome
/// (completed/skipped/failed) passes through.
/// ponytail: one slot per trigger id; a rare overlapping Run-Again + scheduled
/// fire shares it and clears on the first ledger row — the status self-heals
/// on the next poll.
static RUNNING_FIRINGS: OnceLock<Mutex<std::collections::HashMap<String, i64>>> = OnceLock::new();

fn running_firings() -> &'static Mutex<std::collections::HashMap<String, i64>> {
    RUNNING_FIRINGS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// Mark a firing in flight. Keeps the earliest start when already marked (a
/// Readiness Wait that proceeds into the run keeps its wait start).
pub(crate) fn mark_trigger_running(trigger_id: &str) {
    if let Ok(mut running) = running_firings().lock() {
        running.entry(trigger_id.to_string()).or_insert_with(now_ms);
    }
}

/// When the trigger's in-flight firing started, if one is running right now.
pub(crate) fn trigger_running_since_ms(trigger_id: &str) -> Option<i64> {
    running_firings()
        .lock()
        .ok()
        .and_then(|running| running.get(trigger_id).copied())
}

fn clear_trigger_running(trigger_id: &str) {
    if let Ok(mut running) = running_firings().lock() {
        running.remove(trigger_id);
    }
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
    // The ledger row landing ends the in-flight "running" state, whatever the
    // outcome (and even if the write itself fails — never wedge the status).
    clear_trigger_running(trigger_id);
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
    mark_trigger_running(&trigger.id);
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
    run_attempts_and_record(
        app_handle,
        infra,
        trigger,
        &conversation_id,
        &question,
        &title,
        offset_minutes,
        fired_at,
    )
    .await;
}

/// The attempt loop + outcome ledger row + good-news notification for one
/// firing's existing conversation — shared by [`run_trigger_fire`] and Run
/// Again ([`run_trigger_again`]).
#[allow(clippy::too_many_arguments)]
async fn run_attempts_and_record(
    app_handle: &tauri::AppHandle,
    infra: &AppInfraState,
    trigger: &TriggerDefinition,
    conversation_id: &str,
    question: &str,
    title: &str,
    offset_minutes: i32,
    fired_at: i64,
) {
    // Context Assembly (issue #183): the personalization block for this firing
    // — non-sensitive User-Context conclusions + past-run excerpts. Gathered
    // ONCE (best-effort, never blocks the run) and appended to the ephemeral
    // sealed preamble, so it stays out of the persisted question and history.
    let personalization =
        super::context_assembly::gather_personalization(app_handle, infra, trigger, offset_minutes)
            .await;
    // Each attempt is a fresh sealed turn in the SAME conversation (an errored
    // attempt persists as an errored turn; only a completed one becomes the
    // report and history).
    let attempt = || {
        let app_handle = app_handle.clone();
        let conversation_id = conversation_id.to_string();
        let question = question.to_string();
        let title = title.to_string();
        let personalization = personalization.clone();
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
                personalization,
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
                Some(conversation_id),
            )
            .await;
            deliver_run_notification(app_handle, title, conversation_id);
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
                Some(conversation_id),
            )
            .await;
        }
        AttemptOutcome::Failed => {
            tauri_plugin_log::log::warn!(
                "triggers: run for trigger '{}' failed after {attempts} attempts; no notification delivered",
                trigger.id
            );
            // The conversation (with its errored turns) IS linked: it's what
            // Run Again retries, and what a curious user can open.
            record_ledger(
                infra,
                &trigger.id,
                fired_at,
                TriggerFiringOutcome::Failed,
                Some(&format!("AI run did not complete after {attempts} attempts")),
                Some(conversation_id),
            )
            .await;
        }
    }
}

// ── Run Again (retry a failed firing) ────────────────────────────────────────

/// Conversation ids with a Run Again retry currently in flight — a second
/// click while one is running is refused instead of stacking turns.
static RETRIES_INFLIGHT: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();

fn retries_inflight() -> &'static Mutex<std::collections::HashSet<String>> {
    RETRIES_INFLIGHT.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
}

/// Run Again (docs/triggers/CONTEXT.md): retry a FAILED firing as a fresh
/// sealed turn re-running the persisted question in the same conversation —
/// never a synthetic new firing. Bypasses Cooldown (a deliberate click isn't
/// flapping); the Provider Gate still applies. Appends a new ledger row and
/// notifies on completion like any run. Returns as soon as the retry is
/// started — the outcome lands in the ledger.
#[tauri::command]
pub async fn run_trigger_again(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppInfraState>,
    trigger_id: String,
    conversation_id: String,
    offset_minutes: i32,
) -> Result<(), String> {
    let trigger = super::load_triggers(&app_handle)
        .into_iter()
        .find(|trigger| trigger.id == trigger_id)
        .ok_or_else(|| "this trigger no longer exists".to_string())?;
    crate::ask_ai::ensure_ask_ai_access_ready(&app_handle)
        .await
        .map_err(|_| "an AI provider needs to be configured first".to_string())?;
    let conversation = state
        .conversation()
        .get_conversation(&conversation_id)
        .await
        .map_err(|error| format!("could not read the failed run: {error}"))?
        .ok_or_else(|| "the failed run's conversation no longer exists".to_string())?;
    if conversation.trigger_id.as_deref() != Some(trigger_id.as_str()) {
        return Err("that run does not belong to this trigger".to_string());
    }
    let question = conversation
        .turns
        .first()
        .map(|turn| turn.question.clone())
        .ok_or_else(|| "the failed run has nothing to re-run".to_string())?;
    {
        let mut inflight = retries_inflight()
            .lock()
            .map_err(|_| "retry state poisoned".to_string())?;
        if !inflight.insert(conversation_id.clone()) {
            return Err("this run is already being retried".to_string());
        }
    }
    let infra = std::sync::Arc::clone(&*state);
    let title = conversation.title.clone();
    mark_trigger_running(&trigger_id);
    tauri::async_runtime::spawn(async move {
        run_attempts_and_record(
            &app_handle,
            &infra,
            &trigger,
            &conversation_id,
            &question,
            &title,
            offset_minutes,
            now_ms(),
        )
        .await;
        if let Ok(mut inflight) = retries_inflight().lock() {
            inflight.remove(&conversation_id);
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests;
