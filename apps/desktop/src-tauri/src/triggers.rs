//! Triggers — walking skeleton (issue #175, ADRs 0057/0058).
//!
//! A **Trigger** is Condition + Prompt + Delivery (docs/triggers/CONTEXT.md).
//! This slice ships the thinnest end-to-end path with the simplest Condition,
//! **Schedule**:
//!
//! - Definitions are CONFIG, not DB (ADR 0058): `triggers.json` in the app
//!   config dir, hand-edited in this slice (no management UI). It is re-read on
//!   every evaluator tick, so edits hot-reload within one tick.
//! - A background evaluator worker (the user-context worker's poll-loop
//!   pattern) fires due Schedule occurrences: daily/weekly at a chosen local
//!   time, catch-up within the natural period (day/week) — including across a
//!   restart, via the per-trigger last-fired row in the encrypted DB
//!   (`app_infra::trigger_state`) — expired occurrences quietly missed. The
//!   existing `system_did_wake` notifier nudges the loop so a wake catches up
//!   immediately instead of waiting out the tick.
//! - A **Firing** runs one **sealed-toolbox** Ask AI turn (ADR 0058:
//!   `search`/`timeline`/`recall_context` only, global default model) through
//!   the SAME `run_ask_ai_turn` driver interactive Ask AI uses, persisting a
//!   self-titled conversation with `origin = 'trigger'` + trigger id/name.
//!   Follow-ups continue the conversation through the normal chat path.
//! - **Delivery** is a macOS notification on a COMPLETED run only; clicking it
//!   activates the app, and the pending-open slot below routes an activation
//!   with no open window onto that conversation.
//!
//! ponytail: firings run inline on the evaluator loop (one at a time) and the
//! firing ledger/last-run status is issue #176 — this slice persists only the
//! last-fired cursor.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{Listener, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::app_infra::{shutdown_aware_sleep, AppInfraState, BackgroundWorkersState};
use crate::user_context::worker::now_ms;

pub(crate) mod schedule;

use schedule::{ScheduleCadence, ScheduleWeekday};

/// The trigger definitions file inside the app config dir (ADR 0058).
pub const TRIGGERS_FILE_NAME: &str = "triggers.json";

/// Evaluator poll interval. Coarse on purpose: a Schedule's resolution is one
/// minute, and catch-up correctness comes from the due-occurrence math, not the
/// tick rate.
const TRIGGERS_TICK: Duration = Duration::from_secs(30);

/// How long a delivered notification's pending-open slot stays valid. An
/// activation later than this opens the app normally instead of routing onto a
/// stale trigger conversation.
const NOTIFICATION_OPEN_TTL_MS: i64 = 15 * 60 * 1000;

// ── Trigger JSON (the shareable definition shape) ────────────────────────────

fn default_enabled() -> bool {
    true
}

fn default_version() -> u32 {
    1
}

/// One user-authored Trigger from `triggers.json` (docs/triggers/CONTEXT.md
/// "Trigger JSON": name, condition + params, prompt, `version: 1`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerDefinition {
    /// Stable id string — the cross-boundary reference the DB rows use (no FK).
    pub id: String,
    /// Display name, e.g. "Evening Review".
    pub name: String,
    pub condition: TriggerCondition,
    /// The user's own free-text instruction (plain prose, no template vars).
    pub prompt: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_version")]
    pub version: u32,
}

/// The Condition menu, tagged on `type`. v1 walking skeleton: Schedule only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TriggerCondition {
    /// `{"type":"schedule","cadence":"daily","time":"18:30"}` — weekly adds
    /// `"weekday":"friday"`.
    Schedule {
        cadence: ScheduleCadence,
        /// Local time-of-day, `"HH:MM"`.
        time: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        weekday: Option<ScheduleWeekday>,
    },
}

/// Read + parse `triggers.json` from the app config dir. A missing file is the
/// normal no-triggers state; a malformed file logs a warning and evaluates as
/// empty (never wedges the worker).
fn load_triggers(app_handle: &tauri::AppHandle) -> Vec<TriggerDefinition> {
    let Ok(config_dir) = app_handle.path().app_config_dir() else {
        return Vec::new();
    };
    let path = config_dir.join(TRIGGERS_FILE_NAME);
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            tauri_plugin_log::log::warn!("triggers: failed to read {path:?}: {error}");
            return Vec::new();
        }
    };
    match serde_json::from_str::<Vec<TriggerDefinition>>(&contents) {
        Ok(triggers) => triggers,
        Err(error) => {
            tauri_plugin_log::log::warn!(
                "triggers: {path:?} is not a valid trigger definition array: {error}"
            );
            Vec::new()
        }
    }
}

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

/// The **Context Assembly** for this slice: the firing context prepended to the
/// user's Prompt. Names the trigger and its schedule, states the natural-period
/// window the run covers (day so far / week so far, in local time), then the
/// standing instruction verbatim. This whole string is the sealed turn's
/// "question", so it persists on the turn row and stays in follow-up history.
fn build_firing_question(
    trigger: &TriggerDefinition,
    occurrence_ms: i64,
    now_ms: i64,
    offset_minutes: i32,
) -> String {
    let TriggerCondition::Schedule {
        cadence,
        time,
        weekday,
    } = &trigger.condition;
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

// ── Firing → sealed Ask AI run ───────────────────────────────────────────────

/// Run one Firing end-to-end: persist the origin-tagged conversation, run the
/// sealed-toolbox turn through the shared Ask AI driver, and deliver the
/// notification on a completed run. Returns whether the run completed.
async fn run_trigger_fire(
    app_handle: &tauri::AppHandle,
    infra: &AppInfraState,
    trigger: &TriggerDefinition,
    occurrence_ms: i64,
    offset_minutes: i32,
) -> bool {
    let now = now_ms();
    // The firing's stable conversation id: trigger id + occurrence instant, so
    // one occurrence maps to one conversation even across a re-fire attempt.
    let conversation_id = format!("trigger-{}-{}", trigger.id, occurrence_ms);
    let title = run_title(&trigger.name, occurrence_ms, offset_minutes);

    if let Err(error) = infra
        .conversation()
        .create_trigger_conversation(&conversation_id, &title, &trigger.id, &trigger.name, now)
        .await
    {
        tauri_plugin_log::log::warn!(
            "triggers: failed to create run conversation for trigger '{}': {error}",
            trigger.id
        );
        return false;
    }
    let _ = tauri::Emitter::emit(
        app_handle,
        crate::conversation::commands::CONVERSATION_CHANGED_EVENT,
        (),
    );

    let question = build_firing_question(trigger, occurrence_ms, now, offset_minutes);
    let clock = crate::ask_ai::ClientClock {
        utc_offset_minutes: Some(offset_minutes),
        time_zone: None,
    };
    let cancel = crate::ask_ai::register_inflight(&conversation_id);
    let completed = crate::ask_ai::run_ask_ai_turn(
        app_handle.clone(),
        conversation_id.clone(),
        question,
        None,
        "trigger".to_string(),
        title.clone(),
        clock,
        cancel,
        /* sealed = */ true,
    )
    .await;

    if completed {
        deliver_run_notification(app_handle, &title, &conversation_id);
    } else {
        tauri_plugin_log::log::warn!(
            "triggers: run for trigger '{}' did not complete; no notification delivered",
            trigger.id
        );
    }
    completed
}

// ── Evaluator worker ─────────────────────────────────────────────────────────

/// One evaluator pass: hot-reload `triggers.json`, then fire every enabled
/// Schedule trigger whose current-period occurrence is due and unfired.
async fn evaluator_tick(infra: &AppInfraState, app_handle: &tauri::AppHandle) {
    let triggers = load_triggers(app_handle);
    if triggers.is_empty() {
        return;
    }

    // Local wall clock: the frontend-stamped UTC offset User Context already
    // maintains (`user_context.local_offset_minutes`); UTC when never stamped.
    let offset_minutes = infra
        .user_context()
        .local_offset_minutes()
        .await
        .ok()
        .flatten()
        .map(|minutes| minutes as i32)
        .unwrap_or(0);

    for trigger in triggers.into_iter().filter(|trigger| trigger.enabled) {
        let TriggerCondition::Schedule {
            cadence,
            ref time,
            weekday,
        } = trigger.condition;
        let Some(time_minutes) = schedule::parse_time_minutes(time) else {
            tauri_plugin_log::log::warn!(
                "triggers: trigger '{}' has an invalid time {time:?}; skipping",
                trigger.id
            );
            continue;
        };
        let last_fired = infra
            .trigger_state()
            .last_fired_ms(&trigger.id)
            .await
            .ok()
            .flatten();
        let now = now_ms();
        let Some(occurrence_ms) = schedule::due_occurrence_ms(
            cadence,
            time_minutes,
            weekday,
            now,
            offset_minutes,
            last_fired,
        ) else {
            continue;
        };

        // Provider Gate (docs/triggers/CONTEXT.md): a run never starts
        // unconfigured. NOT marking it fired keeps the occurrence retrying each
        // tick, so configuring the engine later within the same period still
        // catches up; the period rolling over quietly misses it.
        if let Err(reason) = crate::ask_ai::ensure_ask_ai_access_ready(app_handle).await {
            tauri_plugin_log::log::debug!(
                "triggers: trigger '{}' is due but the engine is not ready ({reason}); will retry",
                trigger.id
            );
            continue;
        }

        // Durably claim the occurrence BEFORE running: a crash/error mid-run
        // quietly misses it (the ledger with retry semantics is issue #176)
        // rather than re-billing the model every tick.
        if let Err(error) = infra.trigger_state().set_last_fired_ms(&trigger.id, now).await {
            tauri_plugin_log::log::warn!(
                "triggers: failed to record firing for trigger '{}': {error}; not running",
                trigger.id
            );
            continue;
        }

        tauri_plugin_log::log::info!(
            "triggers: firing trigger '{}' for occurrence {occurrence_ms}",
            trigger.id
        );
        // ponytail: firings run inline, one at a time — parallel firings need
        // the #176 ledger to attribute outcomes first.
        run_trigger_fire(app_handle, infra, &trigger, occurrence_ms, offset_minutes).await;
    }
}

/// Spawn the Triggers evaluator worker. Mirrors `spawn_user_context_worker`:
/// one tracked `tauri::async_runtime::spawn` loop selecting between the tick
/// sleep and the shutdown watch — plus a `system_did_wake` nudge (the existing
/// wake notifier, reused) so a sleep-through-schedule-time wake evaluates
/// immediately instead of waiting out the tick.
pub fn spawn_triggers_worker(
    infra: AppInfraState,
    app_handle: tauri::AppHandle,
    background_workers: BackgroundWorkersState,
) {
    let mut shutdown_rx = background_workers.subscribe();

    let wake_nudge = Arc::new(tokio::sync::Notify::new());
    {
        let wake_nudge = Arc::clone(&wake_nudge);
        app_handle.listen(crate::native_capture::SYSTEM_DID_WAKE_EVENT, move |_| {
            wake_nudge.notify_one();
        });
    }

    crate::native_capture::debug_log::log_info("starting triggers evaluator worker");
    let handle = tauri::async_runtime::spawn(async move {
        loop {
            if *shutdown_rx.borrow() {
                break;
            }
            evaluator_tick(&infra, &app_handle).await;
            tokio::select! {
                stopped = shutdown_aware_sleep(&mut shutdown_rx, TRIGGERS_TICK) => {
                    if stopped {
                        break;
                    }
                }
                _ = wake_nudge.notified() => {}
            }
        }
        crate::native_capture::debug_log::log_info("stopped triggers evaluator worker");
    });
    background_workers.track(handle);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const IST: i32 = 330;

    fn sample_daily() -> TriggerDefinition {
        TriggerDefinition {
            id: "evening-review".to_string(),
            name: "Evening Review".to_string(),
            condition: TriggerCondition::Schedule {
                cadence: ScheduleCadence::Daily,
                time: "18:30".to_string(),
                weekday: None,
            },
            prompt: "Summarize what I worked on today.".to_string(),
            enabled: true,
            version: 1,
        }
    }

    #[test]
    fn trigger_definition_serde_round_trips_and_pins_the_wire_shape() {
        // The exact Trigger JSON shape (docs/triggers/CONTEXT.md): this is the
        // shareable form, so the wire shape is pinned, not just round-tripped.
        let trigger = sample_daily();
        let value = serde_json::to_value(&trigger).unwrap();
        assert_eq!(
            value,
            json!({
                "id": "evening-review",
                "name": "Evening Review",
                "condition": { "type": "schedule", "cadence": "daily", "time": "18:30" },
                "prompt": "Summarize what I worked on today.",
                "enabled": true,
                "version": 1
            })
        );
        let round_tripped: TriggerDefinition = serde_json::from_value(value).unwrap();
        assert_eq!(round_tripped, trigger);

        // Weekly carries its weekday tag.
        let weekly = TriggerDefinition {
            condition: TriggerCondition::Schedule {
                cadence: ScheduleCadence::Weekly,
                time: "09:00".to_string(),
                weekday: Some(ScheduleWeekday::Friday),
            },
            ..sample_daily()
        };
        let value = serde_json::to_value(&weekly).unwrap();
        assert_eq!(
            value["condition"],
            json!({ "type": "schedule", "cadence": "weekly", "time": "09:00", "weekday": "friday" })
        );
        let round_tripped: TriggerDefinition = serde_json::from_value(value).unwrap();
        assert_eq!(round_tripped, weekly);
    }

    #[test]
    fn trigger_definition_defaults_enabled_and_version() {
        // A minimal hand-authored file omits `enabled`/`version`.
        let parsed: TriggerDefinition = serde_json::from_value(json!({
            "id": "t",
            "name": "T",
            "condition": { "type": "schedule", "cadence": "daily", "time": "08:00" },
            "prompt": "p"
        }))
        .unwrap();
        assert!(parsed.enabled);
        assert_eq!(parsed.version, 1);
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
        let question = build_firing_question(&trigger, occurrence_ms, now_ms, IST);
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
}
